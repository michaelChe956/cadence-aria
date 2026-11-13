// 安装作业的可诊断性事实（纯数据 + 纯函数），由 `runner.rs` 以 `include!`
// 引入（同一模块作用域）。
//
// 拆出理由：`runner.rs` 触到 large_file_guard 的 1200 行硬线；这些类型与纯
// 函数只服务于「日志说清为什么这么判」，与执行主流程可分离。日志只是这些
// 事实的消费者，用例直接断言事实。

/// 前缀决策链的候选来源（日志字段 `candidates` 里的 `source=`，稳定可 grep）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixCandidateSource {
    /// ① PATH 上当前生效的 CLI 所在前缀。
    Visible,
    /// ② 已安装包（`<prefix>/lib/node_modules/<pkg>`）反推。
    InstalledLocation,
    /// ③ `npm prefix -g`（仅作新装候选）。
    NpmGlobal,
    /// ④ `NPM_PREFIX` / `$HOME/.npm-global` 兜底。
    Fallback,
}

impl PrefixCandidateSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Visible => "visible",
            Self::InstalledLocation => "installed_location",
            Self::NpmGlobal => "npm_global",
            Self::Fallback => "fallback",
        }
    }
}

/// 候选被选中 / 排除的原因（日志字段 `reason=`，稳定可 grep）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrefixCandidateReason {
    /// 命中并被选中。
    Hit,
    /// 前缀不可写。
    NotWritable,
    /// `<prefix>/bin` 不在 PATH 上（装到 PATH 之外 ⇒ 装完探测仍失败）。
    BinNotOnPath,
    /// 候选不存在（PATH 上没有该命令 / 没有已安装包 / 没有 npm 全局前缀）。
    NotFound,
    /// 候选本身可用，但前序候选已命中 ⇒ 本次不选。
    Superseded,
}

impl PrefixCandidateReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Hit => "hit",
            Self::NotWritable => "not_writable",
            Self::BinNotOnPath => "bin_not_on_path",
            Self::NotFound => "not_found",
            Self::Superseded => "superseded",
        }
    }
}

/// 决策链里的一个候选（日志消费者：`InstallJobExecution::run` 汇总成一条
/// info；用例断言这些事实，而不是日志文本）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixCandidate {
    pub source: PrefixCandidateSource,
    /// 候选前缀；`None` = 该来源没有候选。
    pub prefix: Option<PathBuf>,
    pub chosen: bool,
    pub reason: PrefixCandidateReason,
}

/// 前缀决策结果：四个候选 + 最终落点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixDecision {
    pub candidates: Vec<PrefixCandidate>,
    pub chosen_prefix: PathBuf,
}

impl PrefixDecision {
    /// 一条可 grep 的候选摘要（客户机日志里直接 `grep 'reason=not_writable'`）。
    pub fn summary(&self) -> String {
        self.candidates
            .iter()
            .map(|candidate| {
                format!(
                    "{}={}(chosen={},reason={})",
                    candidate.source.as_str(),
                    candidate
                        .prefix
                        .as_deref()
                        .map(|prefix| prefix.display().to_string())
                        .unwrap_or_else(|| "-".to_string()),
                    candidate.chosen,
                    candidate.reason.as_str()
                )
            })
            .collect::<Vec<_>>()
            .join(";")
    }
}

/// 决策链输入：可写 / `bin ∈ PATH` / 已安装反推等判定由调用方用既有 helper
/// 算好 ⇒ [`decide_effective_prefix`] 是纯函数、可单测。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrefixDecisionInput {
    pub visible: Option<PathBuf>,
    pub visible_writable: bool,
    pub installed: Option<PathBuf>,
    pub npm_global: Option<PathBuf>,
    pub npm_global_writable: bool,
    pub npm_global_bin_on_path: bool,
    pub fallback: PathBuf,
}

/// 判定链（写死，语义见 [`InstallJobExecution::resolve_effective_prefix`]）：
/// ① visible（可写）＞ ② 已安装位置反推 ＞ ③ npm 全局（可写且 bin ∈ PATH）
/// ＞ ④ 兜底。返回**每个候选**的 chosen / reason（诊断用）。
pub fn decide_effective_prefix(input: &PrefixDecisionInput) -> PrefixDecision {
    let mut candidates: Vec<PrefixCandidate> = Vec::with_capacity(4);
    let mut chosen: Option<PathBuf> = None;

    // ① visible
    let visible_reason = match input.visible.as_ref() {
        None => PrefixCandidateReason::NotFound,
        Some(_) if !input.visible_writable => PrefixCandidateReason::NotWritable,
        Some(_) if chosen.is_some() => PrefixCandidateReason::Superseded,
        Some(_) => PrefixCandidateReason::Hit,
    };
    let visible_chosen = visible_reason == PrefixCandidateReason::Hit;
    if visible_chosen {
        chosen = input.visible.clone();
    }
    candidates.push(PrefixCandidate {
        source: PrefixCandidateSource::Visible,
        prefix: input.visible.clone(),
        chosen: visible_chosen,
        reason: visible_reason,
    });

    // ② 已安装位置反推
    let installed_reason = match input.installed.as_ref() {
        None => PrefixCandidateReason::NotFound,
        Some(_) if chosen.is_some() => PrefixCandidateReason::Superseded,
        Some(_) => PrefixCandidateReason::Hit,
    };
    let installed_chosen = installed_reason == PrefixCandidateReason::Hit;
    if installed_chosen {
        chosen = input.installed.clone();
    }
    candidates.push(PrefixCandidate {
        source: PrefixCandidateSource::InstalledLocation,
        prefix: input.installed.clone(),
        chosen: installed_chosen,
        reason: installed_reason,
    });

    // ③ npm 全局前缀（仅新装候选：可写 **且** bin ∈ PATH）
    let npm_global_reason = match input.npm_global.as_ref() {
        None => PrefixCandidateReason::NotFound,
        Some(_) if !input.npm_global_writable => PrefixCandidateReason::NotWritable,
        Some(_) if !input.npm_global_bin_on_path => PrefixCandidateReason::BinNotOnPath,
        Some(_) if chosen.is_some() => PrefixCandidateReason::Superseded,
        Some(_) => PrefixCandidateReason::Hit,
    };
    let npm_global_chosen = npm_global_reason == PrefixCandidateReason::Hit;
    if npm_global_chosen {
        chosen = input.npm_global.clone();
    }
    candidates.push(PrefixCandidate {
        source: PrefixCandidateSource::NpmGlobal,
        prefix: input.npm_global.clone(),
        chosen: npm_global_chosen,
        reason: npm_global_reason,
    });

    // ④ 兜底
    let fallback_reason = if chosen.is_some() {
        PrefixCandidateReason::Superseded
    } else {
        PrefixCandidateReason::Hit
    };
    let fallback_chosen = fallback_reason == PrefixCandidateReason::Hit;
    if fallback_chosen {
        chosen = Some(input.fallback.clone());
    }
    candidates.push(PrefixCandidate {
        source: PrefixCandidateSource::Fallback,
        prefix: Some(input.fallback.clone()),
        chosen: fallback_chosen,
        reason: fallback_reason,
    });

    PrefixDecision {
        chosen_prefix: chosen.unwrap_or_else(|| input.fallback.clone()),
        candidates,
    }
}

/// 装后探测的版本来源（日志字段 `version_source`，稳定可 grep）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VersionProbeSource {
    /// npm 包的 `package.json`（不执行进程）。
    PackageJson,
    /// provider health 快照（PATH ∪ overlay 口径）。
    HealthSnapshot,
    /// PATH 口径 `--version`。
    VersionFlag,
    /// `<bin>/<cmd> --version` 绝对路径兜底。
    AbsolutePathProbe,
    /// 都没读到。
    None,
}

impl VersionProbeSource {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::PackageJson => "from_package_json",
            Self::HealthSnapshot => "from_health_snapshot",
            Self::VersionFlag => "from_version_flag",
            Self::AbsolutePathProbe => "absolute_path_probe",
            Self::None => "none",
        }
    }
}

/// 装后探测的**结构化事实**（日志消费者：`refresh_after_install` 汇总成一条
/// info；用例断言这些事实，而不是日志文本）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PostInstallProbeFacts {
    pub version_source: VersionProbeSource,
    /// PATH 口径 `--version` 是否成功。
    pub path_version_ok: bool,
    /// 真实命令的 argv（`codegraph status --json` / provider `--help`）。
    pub real_command_argv: Vec<String>,
    pub real_command_ok: bool,
    /// PATH 口径（裸命令）真实命令是否成功 ⇒ false = 装到了 PATH 之外。
    pub path_probe_ok: bool,
}

/// 装后失败三语义（日志字段 `failure_class`，稳定 code：2.8③）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PostInstallFailureClass {
    /// 没装上（产物里没有该命令）。
    NotInstalled,
    /// 装上了但跑不起来（真实命令失败，可能缺平台包）。
    MissingPlatformPackage,
    /// 装上了、能跑，但 PATH 上找不到它。
    NotOnPath,
}

impl PostInstallFailureClass {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotInstalled => "not_installed",
            Self::MissingPlatformPackage => "missing_platform_package",
            Self::NotOnPath => "not_on_path",
        }
    }
}

/// 装后失败语义判定（纯函数，日志消费者）：`installed` = 安装产物 bin 目录
/// 里确实有该命令；`real_command_ok` = 真实命令跑通。
pub fn post_install_failure_class(
    installed: bool,
    real_command_ok: bool,
) -> PostInstallFailureClass {
    if !installed {
        PostInstallFailureClass::NotInstalled
    } else if !real_command_ok {
        PostInstallFailureClass::MissingPlatformPackage
    } else {
        PostInstallFailureClass::NotOnPath
    }
}

/// 更新前后版本的**结构化事实**（日志 `before_version` / `after_version` /
/// `still_old`；「仍是旧版」判定与日志同源）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateVersionFacts {
    pub before: Option<String>,
    pub after: Option<String>,
    /// 前后都能读到且相同 ⇒ 命令仍是旧版（更新没生效）。
    pub still_old: bool,
}

/// 比较更新前后版本（纯函数）。
pub fn update_version_facts(before: Option<&str>, after: Option<&str>) -> UpdateVersionFacts {
    UpdateVersionFacts {
        before: before.map(str::to_string),
        after: after.map(str::to_string),
        still_old: matches!((before, after), (Some(before), Some(after)) if before == after),
    }
}

/// `InstallFailureKind` → 日志用的稳定 code（与 wire 序列化的 snake_case 同形）。
fn install_failure_kind_code(kind: InstallFailureKind) -> &'static str {
    match kind {
        InstallFailureKind::Permission => "permission",
        InstallFailureKind::Network => "network",
        InstallFailureKind::PackageNotFound => "package_not_found",
        InstallFailureKind::Other => "other",
    }
}

impl HealthRefreshProber {
    /// 装后探测 + **结构化事实**（版本来源 / 真实命令 argv / PATH 口径结果）。
    ///
    /// 日志与用例都消费这里的事实：`refresh_after_install` 只负责把它们写成
    /// 一条 info，用例直接断言事实（探测来源、真实命令、PATH 口径）。
    pub async fn probe_with_facts(
        &self,
        target: InstallTarget,
        install_bin_dir: Option<&Path>,
    ) -> (
        Result<PostInstallProbeOutcome, String>,
        PostInstallProbeFacts,
    ) {
        let health = self.health.refresh(CancellationToken::new()).await;
        let row = row_for_wire(target.wire_label());
        let mut version_source = VersionProbeSource::None;
        // ① 版本**优先读 npm 包的 `package.json`**（不执行进程、更快更稳；
        //    magpie `proc.Version` 同款）——读不到才回落 `--version`。
        let mut version = row.and_then(|row| {
            let package = row.npm_package?;
            let prefix = install_bin_dir?.parent()?;
            npm_package_json_version(prefix, package)
        });
        if version.is_some() {
            version_source = VersionProbeSource::PackageJson;
        }
        // ② provider 版本走 health 快照（PATH ∪ overlay 口径）。
        if version.is_none()
            && let InstallTarget::Provider(provider) = &target
            && let Some(snapshot) = self
                .health
                .latest_diagnostic()
                .entry(provider)
                .and_then(|entry| entry.version.clone())
        {
            version = Some(snapshot);
            version_source = VersionProbeSource::HealthSnapshot;
        }
        // 读不到版本 ⇒ 跑 `--version`：先 PATH 口径，再**绝对路径**兜底
        // （装到 PATH 之外时 PATH 口径必然 127）。
        let mut path_version_ok = false;
        if version.is_none() {
            let command = row.map(|row| row.command).unwrap_or("codegraph");
            if let Some(probed) = self.probe_version(command, install_bin_dir, false).await {
                version = Some(probed);
                version_source = VersionProbeSource::VersionFlag;
                path_version_ok = true;
            } else if let Some(probed) = self.probe_version(command, install_bin_dir, true).await {
                version = Some(probed);
                version_source = VersionProbeSource::AbsolutePathProbe;
            }
        }
        // 真实命令验证（独立于版本读取：版本可来自 package.json）。
        let (real_command_ok, path_probe_ok, real_command_argv) =
            self.probe_real_command(target, install_bin_dir).await;
        let facts = PostInstallProbeFacts {
            version_source,
            path_version_ok,
            real_command_argv,
            real_command_ok,
            path_probe_ok,
        };
        let result = match (version, health) {
            (Some(version), _) => Ok(PostInstallProbeOutcome {
                installed_version: Some(version),
                real_command_ok,
            }),
            (None, Err(error)) => Err(error.to_string()),
            (None, Ok(_)) => Ok(PostInstallProbeOutcome {
                installed_version: None,
                real_command_ok,
            }),
        };
        (result, facts)
    }
}
