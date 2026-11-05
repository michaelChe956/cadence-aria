//! LC provider capability probe 与证据签发(Task 6c,REQ-LCG-03/07;
//! 批2 Task 9 语义换代:宿主全开执行 host-open-v1)。
//!
//! `ProviderBoundaryProbe::run` 是受控 fixture 的**探测通道**(冻结签名):
//! 对某个真实 provider 的 projection,以宿主全开执行语义
//! (`PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN`,消费 T2 Host profile/HostLocal
//! 部署,不调用 bwrap launcher)执行**真实正向写探针**(builtin/terminal/
//! MCP 三通道 + Coding 受控 commit)+ 真实 resume 面(launch→同 id resume→
//! 错 id 负探针),并且:
//! - 受控宿主写成功是「全开」事实——不再要求 root/非 target/`.git`/
//!   `.aria` 物理拒写;不在真实平台目录做破坏探针(一切写探针落在受控
//!   fixture 内);
//! - 岗位/窗口(HostOpenProbeScope,受信注入)缺失一律拒签;probe 不靠
//!   模型自述、`git status` 或「MCP 可用」单条件代证;
//! - 审计/资源身份(duty/review_window_id/resource_scope_id)随证据工件
//!   落盘,可追溯。
//!
//! 语义红线(批2 Task 9):
//! - `write_boundary=Confirmed` 只能由本 probe 的真实正向探针+resume 面
//!   签发;CLI 版本漂移、观测缺失(哨兵)、观测超预算、证据工件未落盘、
//!   岗位/窗口缺失——一律**不签 Confirmed**,状态保持 Unknown;正向探针
//!   真实失败是防护失效 → Denied。超限**不截断冒充通过**。
//! - evidence 工件写盘不入 git;`artifact_ref` 必须指向真实存在的工件
//!   文件;工件不含任何 secret 值(env 只记键约定,不记值)。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::cross_cutting::provider_boundary::{
    BoundaryWriteAttempt, BoundaryWriteChannel, PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN,
    PlannedBoundaryWrite, ProviderBoundaryError, ProviderBoundaryEvidence, ProviderBoundaryMode,
    ProviderBoundaryPlan, probe_write_command, shell_quote,
};
use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use crate::cross_cutting::provider_execution::TrustedProviderRole;
use crate::product::logical_codebase::aggregate_index::freshness::COMPACT_INVENTORY_HARD_BUDGET_BYTES;
use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;
use crate::product::models::ProviderName;

/// evidence 工件的 schema 标记(字段序变化必须换 schema 版本;批2 Task 9
/// 语义换代 → v2:增 execution_semantics/duty/review_window_id/
/// resource_scope_id,去物理拒写面 negatives/D4/bwrap/mounts)。
pub const BOUNDARY_PROBE_EVIDENCE_SCHEMA: &str = "lc-boundary-probe-evidence-v2";

/// capability 视角保持 Unknown 的稳定码前缀(缺防护/版本漂移/观测缺失/
/// 超预算/工件未落盘);其余 `ProbeFailed`(负向落盘、正向失败、受保护面
/// 漂移)是防护失效 → Denied。
const UNKNOWN_STATE_CODES: [&str; 5] = [
    "boundary_unavailable",
    "boundary_version_drift",
    "boundary_unobserved",
    "boundary_observation_over_budget",
    "boundary_artifact_unwritten",
];
/// provider 的自然 session/runtime 可写窄面目录(host-open fixture 与
/// 正向探针共用同一窄面;原 `PROVIDER_RUNTIME_WRITABLE_DIRS` 常量已随
/// T10 launcher mount overlay 退役,目录事实由此处与 fixture 持有)。
pub fn provider_runtime_dir_name(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => ".claude",
        ProviderName::Codex => ".codex",
        ProviderName::Pi => ".pi",
        ProviderName::KimiCode => ".kimi-code",
        // Fake 不经真实 probe 通道(evidence 由 2c fail-closed 拒绝);无窄面
        // 目录时正向探针缺位,永不签 Confirmed。
        ProviderName::Fake => "",
    }
}

/// provider 的报告目录族名(evidence 目录布局:
/// `cadence/reports/lc-gateway-multi-provider/<族名>/boundary/`)。
pub fn provider_family_text(provider: &ProviderName) -> &'static str {
    match provider {
        ProviderName::ClaudeCode => "claude-code",
        ProviderName::Codex => "codex",
        ProviderName::Pi => "pi",
        ProviderName::KimiCode => "kimi-code",
        ProviderName::Fake => "fake",
    }
}

/// resume 面 probe 通道:四家真实 CLI 的 launch/resume 原生 id 语义
/// (2026-10-05 现场实测;argv/错 id 行为均为真实 CLI 实测口径):
/// - claude:`-p --output-format json` 应答 JSON `session_id`;`--resume <id>`
///   续开同 id;错 id exit 1「No conversation found with session ID」。
/// - codex:`exec --json` JSONL `thread.started.thread_id`;`exec resume <id>
///   --json` 续开同 id;错 id exit 1「no rollout found for thread id」。
/// - pi:`--mode json -p` 首事件 `{"type":"session","id":…}`;resume 经
///   `--session-id <id>`(与产品 adapter 同通道);错 id 时新建**另一** id
///   的会话(stderr 警告),不回显原 id——负探针判据为「错 id 不得回显原
///   会话 id」(claude/codex/kimi 以真实报错满足,pi 以不同 id 满足)。
/// - kimi:`-p --output-format stream-json` 事件 `session_id`;`-S <id>`
///   续开同 id;错 id exit 1「Session … not found」。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResumeChannelKind {
    ClaudePrintJson,
    CodexExecJson,
    PiSessionId,
    KimiStreamJson,
}

impl ResumeChannelKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ClaudePrintJson => "claude_print_json",
            Self::CodexExecJson => "codex_exec_json",
            Self::PiSessionId => "pi_session_id",
            Self::KimiStreamJson => "kimi_stream_json",
        }
    }

    /// launch argv(不含程序名;prompt 注入)。
    pub(crate) fn launch_argv(self, prompt: &str) -> Vec<String> {
        match self {
            Self::ClaudePrintJson => {
                vec![
                    "-p".into(),
                    prompt.into(),
                    "--output-format".into(),
                    "json".into(),
                ]
            }
            Self::CodexExecJson => vec!["exec".into(), "--json".into(), prompt.into()],
            Self::PiSessionId => vec!["--mode".into(), "json".into(), "-p".into(), prompt.into()],
            Self::KimiStreamJson => vec![
                "-p".into(),
                prompt.into(),
                "--output-format".into(),
                "stream-json".into(),
            ],
        }
    }

    /// 同 id 续开 argv(native id 注入)。
    pub(crate) fn resume_argv(self, native_id: &str, prompt: &str) -> Vec<String> {
        match self {
            Self::ClaudePrintJson => vec![
                "-p".into(),
                "--resume".into(),
                native_id.into(),
                prompt.into(),
                "--output-format".into(),
                "json".into(),
            ],
            Self::CodexExecJson => vec![
                "exec".into(),
                "resume".into(),
                native_id.into(),
                "--json".into(),
                prompt.into(),
            ],
            Self::PiSessionId => vec![
                "--mode".into(),
                "json".into(),
                "--session-id".into(),
                native_id.into(),
                "-p".into(),
                prompt.into(),
            ],
            Self::KimiStreamJson => vec![
                "-S".into(),
                native_id.into(),
                "-p".into(),
                prompt.into(),
                "--output-format".into(),
                "stream-json".into(),
            ],
        }
    }

    /// 负探针使用的伪造 native id(格式与各家真实 id 同形)。
    pub(crate) fn bogus_native_id(self) -> String {
        match self {
            Self::KimiStreamJson => "session_00000000-dead-0000-0000-000000000000".to_string(),
            _ => "00000000-dead-0000-0000-000000000000".to_string(),
        }
    }

    /// 从真实输出提取 native id(launch 与 resume 共用;逐行 JSON 扫描,
    /// claude 单对象整体解析优先)。
    pub(crate) fn extract_native_id(self, output: &str) -> Option<String> {
        match self {
            Self::ClaudePrintJson => {
                if let Ok(value) = serde_json::from_str::<serde_json::Value>(output.trim())
                    && let Some(id) = value.get("session_id").and_then(|id| id.as_str())
                {
                    return Some(id.to_string());
                }
                scan_json_lines(output, "session_id", None)
            }
            Self::CodexExecJson => scan_json_lines(output, "thread_id", None),
            Self::PiSessionId => scan_json_lines(output, "id", Some("session")),
            Self::KimiStreamJson => scan_json_lines(output, "session_id", None),
        }
    }
}

/// 逐行扫描 JSON 输出,取首个含 `key` 的行的该字段(可选按 `type` 过滤)。
fn scan_json_lines(output: &str, key: &str, type_filter: Option<&str>) -> Option<String> {
    for line in output.lines() {
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
            continue;
        };
        if let Some(filter) = type_filter
            && value.get("type").and_then(|kind| kind.as_str()) != Some(filter)
        {
            continue;
        }
        if let Some(id) = value.get(key).and_then(|id| id.as_str()) {
            return Some(id.to_string());
        }
    }
    None
}

/// resume 面 probe 规格:受控 fixture 内「真实 launch(记录 native id)→
/// 同 id 真实 resume→native id 确认同 id→错 id 负探针」。规格缺省 =
/// resume 未探测(launch/write_boundary 面不受影响)。
#[derive(Debug, Clone)]
pub struct ResumeProbeSpec {
    kind: ResumeChannelKind,
    prompt: String,
    timeout_secs: u64,
}

impl ResumeProbeSpec {
    /// 默认单次调用 240s 超时(真机 LLM 轮次;fail-closed)。
    pub fn new(kind: ResumeChannelKind, prompt: impl Into<String>) -> Self {
        Self {
            kind,
            prompt: prompt.into(),
            timeout_secs: 240,
        }
    }

    pub fn with_timeout_secs(mut self, timeout_secs: u64) -> Self {
        self.timeout_secs = timeout_secs;
        self
    }

    pub fn kind(&self) -> ResumeChannelKind {
        self.kind
    }

    pub fn prompt(&self) -> &str {
        &self.prompt
    }

    pub(crate) fn timeout_secs(&self) -> u64 {
        self.timeout_secs
    }
}

/// 宿主全开执行探针的可信岗位/窗口/资源身份(批2 Task 9 冻结面)。
///
/// `duty`(`author`/`reviewer`,与 T2 `TrustedProviderRole::as_str` 同口径)、
/// `review_window_id`(Reviewer 窗口)与 `resource_scope_id`(审计/资源
/// 归属)由受信调用方(产品 revalidate 通道、矩阵 harness)注入;probe
/// **不从模型自报、prompt、env 或 `git status` 推导**,也不以「MCP 可用」
/// 单条件代证。缺岗位、Reviewer 缺窗口、缺资源身份一律拒签。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostOpenProbeScope {
    duty: String,
    review_window_id: Option<String>,
    resource_scope_id: String,
}

impl HostOpenProbeScope {
    /// Author 岗位(Coding/HostCoding;无窗口)。
    pub fn author(resource_scope_id: impl Into<String>) -> Self {
        Self {
            duty: "author".to_string(),
            review_window_id: None,
            resource_scope_id: resource_scope_id.into(),
        }
    }

    /// Reviewer 窗口(Planning/Review 只读 action;窗口必带非空)。
    pub fn reviewer(
        review_window_id: impl Into<String>,
        resource_scope_id: impl Into<String>,
    ) -> Self {
        Self {
            duty: "reviewer".to_string(),
            review_window_id: Some(review_window_id.into()),
            resource_scope_id: resource_scope_id.into(),
        }
    }

    pub fn duty(&self) -> &str {
        &self.duty
    }

    pub fn review_window_id(&self) -> Option<&str> {
        self.review_window_id.as_deref()
    }

    pub fn resource_scope_id(&self) -> &str {
        &self.resource_scope_id
    }
}

/// `SessionPolicyAction` 的稳定文本(serde snake_case 序列化同形)。
fn action_text(action: SessionPolicyAction) -> &'static str {
    match action {
        SessionPolicyAction::PlanningReadOnly => "planning_read_only",
        SessionPolicyAction::CodingTargetWrite => "coding_target_write",
        SessionPolicyAction::ReviewReadOnly => "review_read_only",
    }
}

/// 长度分隔的 canonical 字段(`len:value`,杜绝分隔符/注入歧义;与四家
/// projector 的 digest 输入形态同构)。
fn canonical_field(canonical: &mut String, value: &str) {
    canonical.push_str(&value.len().to_string());
    canonical.push(':');
    canonical.push_str(value);
}

/// 受控隔离 fixture:6c probe 的物理探测材料。`create` 在 `base` 下搭建
/// canonical LC root(git 仓库 + `.aria`/AGENTS.md/.mcp.json)、两个成员、
/// provider runtime home 窄面目录,以及(Coding)`git worktree add` 出的
/// linked-worktree target;probe 在该材料上执行,不触碰任何真实 LC。
#[derive(Debug, Clone)]
pub struct BoundaryFixture {
    provider: ProviderName,
    cli_program: String,
    root: PathBuf,
    member: PathBuf,
    member_b: PathBuf,
    target: Option<PathBuf>,
    home: PathBuf,
    evidence_root: PathBuf,
    session_label: String,
    /// resume 面 probe 规格(None = resume 未探测,只验 launch/write 面)。
    resume: Option<ResumeProbeSpec>,
    /// r50:请求 action 原样保留——Planning/Review 两只读 action 探针
    /// 形态同构但导入行不同,不得折叠(boundary_action 由此取值)。
    action: SessionPolicyAction,
}

impl BoundaryFixture {
    /// 搭建受控隔离 fixture 材料(git init/worktree add/运行时窄面目录)。
    /// read-only action 不创建 target;Coding 创建 linked-worktree target
    /// (git identity 链冻结面与真实产品 launcher 同源)。
    #[allow(clippy::too_many_arguments)]
    pub fn create(
        provider: ProviderName,
        cli_program: &str,
        action: SessionPolicyAction,
        base: &Path,
        evidence_root: &Path,
        session_label: &str,
    ) -> Result<Self, ProviderBoundaryError> {
        let root = base.join("lc-root");
        // 成员 checkout 是 canonical root 的子目录(真实 LC 拓扑与 6a 先例
        // 同构):launcher 只 ro-bind root 即覆盖成员写面,负向探针得到真实
        // EROFS 拒绝,而非 tmpfs 遮蔽下的 ENOENT。
        let member = root.join("member-a");
        let member_b = root.join("member-b");
        let home = base.join("home");
        fs::create_dir_all(root.join(".aria"))
            .and_then(|()| fs::write(root.join(".aria").join("state.json"), "{}"))
            .map_err(|error| {
                ProviderBoundaryError::InvalidPlan(format!("fixture root metadata: {}", error))
            })?;
        fs::write(root.join("AGENTS.md"), "# lc root\n").map_err(|error| {
            ProviderBoundaryError::InvalidPlan(format!("fixture agents: {error}"))
        })?;
        fs::write(root.join(".mcp.json"), "{}\n").map_err(|error| {
            ProviderBoundaryError::InvalidPlan(format!("fixture mcp config: {error}"))
        })?;
        fs::create_dir_all(&member)
            .and_then(|()| fs::write(member.join("README.md"), "member\n"))
            .map_err(|error| {
                ProviderBoundaryError::InvalidPlan(format!("fixture member: {error}"))
            })?;
        fs::create_dir_all(&member_b)
            .and_then(|()| fs::write(member_b.join("note.txt"), "note\n"))
            .map_err(|error| {
                ProviderBoundaryError::InvalidPlan(format!("fixture member b: {error}"))
            })?;
        let runtime_dir = provider_runtime_dir_name(&provider);
        if runtime_dir.is_empty() {
            return Err(ProviderBoundaryError::InvalidPlan(format!(
                "provider {provider:?} has no runtime writable face for a real probe"
            )));
        }
        fs::create_dir_all(home.join(runtime_dir)).map_err(|error| {
            ProviderBoundaryError::InvalidPlan(format!("fixture runtime home: {error}"))
        })?;
        let target = match action {
            SessionPolicyAction::CodingTargetWrite => {
                let repo = base.join("repo-source");
                fs::create_dir_all(&repo).map_err(|error| {
                    ProviderBoundaryError::InvalidPlan(format!("fixture repo: {error}"))
                })?;
                git(&repo, &["init", "-q"])?;
                git(
                    &repo,
                    &[
                        "-c",
                        "user.name=aria",
                        "-c",
                        "user.email=aria@aria",
                        "commit",
                        "--allow-empty",
                        "-m",
                        "init",
                    ],
                )?;
                let target = base.join("target-wt");
                git(
                    &repo,
                    &[
                        "worktree",
                        "add",
                        "-q",
                        target.to_string_lossy().as_ref(),
                        "-b",
                        "lc-probe-target",
                    ],
                )?;
                fs::create_dir_all(target.join(".aria"))
                    .and_then(|()| fs::write(target.join(".aria").join("state.json"), "{}"))
                    .map_err(|error| {
                        ProviderBoundaryError::InvalidPlan(format!(
                            "fixture target metadata: {error}"
                        ))
                    })?;
                Some(target)
            }
            SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => None,
        };
        git(&root, &["init", "-q"])?;
        git(&member, &["init", "-q"])?;
        Ok(Self {
            provider,
            cli_program: cli_program.to_string(),
            root,
            member,
            member_b,
            target,
            home,
            evidence_root: evidence_root.to_path_buf(),
            session_label: session_label.to_string(),
            resume: None,
            action,
        })
    }

    pub fn provider(&self) -> ProviderName {
        self.provider.clone()
    }

    pub fn cli_program(&self) -> &str {
        &self.cli_program
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn member(&self) -> &Path {
        &self.member
    }

    pub fn member_b(&self) -> &Path {
        &self.member_b
    }

    pub fn target(&self) -> Option<&Path> {
        self.target.as_deref()
    }

    pub fn home(&self) -> &Path {
        &self.home
    }

    pub fn evidence_root(&self) -> &Path {
        &self.evidence_root
    }

    pub fn session_label(&self) -> &str {
        &self.session_label
    }

    /// 附加 resume 面 probe 规格(builder;材料不变,仅扩展探测面)。
    pub fn with_resume(mut self, spec: ResumeProbeSpec) -> Self {
        self.resume = Some(spec);
        self
    }

    pub fn resume_spec(&self) -> Option<&ResumeProbeSpec> {
        self.resume.as_ref()
    }

    /// fixture 材料对应的 canonical action(target 在场即 Coding)。
    pub fn boundary_action(&self) -> SessionPolicyAction {
        if self.target.is_some() {
            SessionPolicyAction::CodingTargetWrite
        } else {
            // r50:只读两 action 原样透传(请求 Review 即 Review——导入行
            // 与能力矩阵消费行必须同 action,折叠会让该行恒 Unknown)。
            self.action
        }
    }

    /// 证据会话目录(evidence.json/shape-validation.json 的落盘位置)。
    pub fn evidence_session_dir(&self) -> PathBuf {
        self.evidence_root.join(&self.session_label)
    }

    /// fixture digest:对 fixture 材料身份(provider/CLI/root/member/member_b/
    /// target/home/evidence/session)的长度分隔 SHA-256(schema 前缀钉定)。
    pub fn fixture_digest(&self) -> String {
        let mut canonical = String::new();
        canonical_field(&mut canonical, "lc-boundary-fixture-v1");
        for part in [
            format!("{:?}", self.provider),
            self.cli_program.clone(),
            self.root.to_string_lossy().into_owned(),
            self.member.to_string_lossy().into_owned(),
            self.member_b.to_string_lossy().into_owned(),
            self.target
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default(),
            self.home.to_string_lossy().into_owned(),
            self.evidence_root.to_string_lossy().into_owned(),
            self.session_label.clone(),
        ] {
            canonical.push('|');
            canonical_field(&mut canonical, &part);
        }
        format!("sha256:{:x}", Sha256::digest(canonical.as_bytes()))
    }

    /// probe 沙箱环境(6a launcher 的 env overlay):HOME 指向 fixture 运行时
    /// 窄面,PASS 沿宿主(保持既有配置发现),git identity 冻结,禁读宿主
    /// 全局 git 配置。pub(Task 11 组装层:harness 在自有 fixture 上追加
    /// 写探针时复用同一 overlay,不另立第二套 env 约定)。
    pub fn probe_env(&self) -> BTreeMap<String, String> {
        let mut env = BTreeMap::new();
        if let Ok(path) = std::env::var("PATH") {
            env.insert("PATH".to_string(), path);
        }
        env.insert("HOME".to_string(), self.home.to_string_lossy().into_owned());
        for (key, value) in [
            ("GIT_CONFIG_GLOBAL", "/dev/null"),
            ("GIT_CONFIG_NOSYSTEM", "1"),
            ("GIT_AUTHOR_NAME", "aria"),
            ("GIT_AUTHOR_EMAIL", "aria@aria"),
            ("GIT_COMMITTER_NAME", "aria"),
            ("GIT_COMMITTER_EMAIL", "aria@aria"),
        ] {
            env.insert(key.to_string(), value.to_string());
        }
        env
    }

    /// provider 自然运行时窄面内的正向探针文件路径(必须可写:探针通道
    /// 自身有效的对照,read-only action 也保留该会话写面)。
    pub(crate) fn runtime_probe_path(&self) -> PathBuf {
        self.home
            .join(provider_runtime_dir_name(&self.provider))
            .join("aria-boundary-runtime-probe.txt")
    }
}

/// fixture 搭建用的宿主侧 git(git identity 隔离,不读宿主全局配置)。
fn git(dir: &Path, args: &[&str]) -> Result<(), ProviderBoundaryError> {
    let status = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .map_err(|error| {
            ProviderBoundaryError::InvalidPlan(format!("fixture git spawn: {error}"))
        })?;
    if !status.success() {
        return Err(ProviderBoundaryError::InvalidPlan(format!(
            "fixture git {args:?} exited with {:?}",
            status.code()
        )));
    }
    Ok(())
}

/// LC 写边界 probe(冻结签名归本类型;调用方不得自造 evidence)。
#[derive(Debug, Clone, Copy, Default)]
pub struct ProviderBoundaryProbe;

impl ProviderBoundaryProbe {
    /// 冻结签名(批2 Task 9 语义换代):受控 fixture 的宿主全开探测通道。
    /// 真实正向写探针(builtin/terminal/MCP 三通道 + Coding 受控 commit)+
    /// 岗位/窗口受信在场 + 版本一致 + 观测完整,才签发 host-open evidence;
    /// 任一环节缺位返回稳定码错误(见 [`Self::evidence_state`] 三态映射)。
    pub async fn run(
        projection: &ProviderPolicyProjection,
        fixture: &BoundaryFixture,
        scope: &HostOpenProbeScope,
    ) -> Result<ProviderBoundaryEvidence, ProviderBoundaryError> {
        Self::run_with_observation_budget(
            COMPACT_INVENTORY_HARD_BUDGET_BYTES,
            projection,
            fixture,
            scope,
        )
        .await
    }

    /// 注入式 seam(观测预算可注入;测试与装配用)。生产 `run` 以沿 HEAD
    /// 的 inventory 硬预算(8192B)调用本函数。宿主全开语义:无 bwrap
    /// launcher 前置(消费 Host profile,不在沙箱内执行)。
    pub(crate) async fn run_with_observation_budget(
        observation_budget: usize,
        projection: &ProviderPolicyProjection,
        fixture: &BoundaryFixture,
        scope: &HostOpenProbeScope,
    ) -> Result<ProviderBoundaryEvidence, ProviderBoundaryError> {
        // 1) plan 派生与 projection/fixture 一致性(材料不匹配=未探测,
        //    InvalidPlan → Unknown)。
        let plan = derive_boundary_plan(projection, fixture)?;
        // 2) 岗位/窗口受信在场(批2 Task 9):缺岗位、Reviewer 缺窗口、
        //    缺资源身份一律拒签;岗位必须与 action 执行语义相符。
        validate_scope_for_action(scope, projection.action())?;
        // 3) 真实 CLI exact version(宿主侧只读探测);漂移即拒签。
        let cli_version = probe_cli_version(fixture.cli_program())?;
        if cli_version != projection.exact_version() {
            return Err(ProviderBoundaryError::ProbeFailed(format!(
                "boundary_version_drift: projection={:?} live `{} --version`={:?}",
                projection.exact_version(),
                fixture.cli_program(),
                cli_version
            )));
        }
        // 4) resume 面(规格在场时):宿主侧真实 launch→同 id resume→错
        //    id 负探针(保留真实 native id 与负探针记录;resume 结果与写
        //    面正交,只记录工件 resume 段,2d 导入消费)。
        let resume = match fixture.resume_spec() {
            Some(spec) => run_resume_segment(spec, fixture.cli_program(), fixture.root()).await,
            None => ResumeProbeRecord::not_probed(),
        };
        let env = fixture.probe_env();
        // 5) 宿主全开正向写探针:builtin(provider 运行时窄面 [+ Coding
        //    target])/terminal(root 普通文件)/MCP(member 普通文件)三
        //    通道真实执行——受控宿主写成功=「全开」事实,不再要求物理
        //    拒写;一切写探针只落在受控 fixture 内,不触真实平台目录。
        let coding = matches!(projection.action(), SessionPolicyAction::CodingTargetWrite);
        let runtime_probe = fixture.runtime_probe_path();
        let target_probe = fixture
            .target()
            .map(|target| target.join("aria-boundary-probe.txt"));
        let terminal_probe = fixture.root().join("aria-host-open-terminal-probe.txt");
        let mcp_probe = fixture.member().join("aria-host-open-mcp-probe.txt");
        let mut planned = vec![PlannedBoundaryWrite::new(
            BoundaryWriteChannel::Builtin,
            runtime_probe.clone(),
        )];
        if let Some(target_probe) = target_probe.clone() {
            planned.push(PlannedBoundaryWrite::new(
                BoundaryWriteChannel::Builtin,
                target_probe,
            ));
        }
        planned.push(PlannedBoundaryWrite::new(
            BoundaryWriteChannel::Terminal,
            terminal_probe.clone(),
        ));
        planned.push(PlannedBoundaryWrite::new(
            BoundaryWriteChannel::Mcp,
            mcp_probe.clone(),
        ));
        // 记录宿主执行 argv 形态(payload 以标记代替;逐 attempt 记录见
        // positives)。
        let probe_argv = argv_with_program(
            "sh",
            &[
                "-c".to_string(),
                "(aria host-open write-surface probe payload)".to_string(),
            ],
        );
        let attempts = run_host_write_surface_probe(&env, fixture.root(), &planned).await?;
        // 6) 观测预算:超限拒签,绝不截断观测面冒充通过。
        let observation_bytes: usize = attempts
            .iter()
            .map(|attempt| attempt.evidence().len() + attempt.path().as_os_str().len())
            .sum();
        if observation_bytes > observation_budget {
            return Err(ProviderBoundaryError::ProbeFailed(format!(
                "boundary_observation_over_budget: attempts observation {observation_bytes}B exceeds budget {observation_budget}B; refusing to truncate the observation surface"
            )));
        }
        // 7) 全开判定:每个正向探针必须写成功且宿主复核真实落盘 verbatim
        //    (写失败/未观测 = 防护失效 → Denied 维度,不签)。
        for attempt in &attempts {
            if attempt.result() != Ok(()) {
                return Err(ProviderBoundaryError::ProbeFailed(format!(
                    "positive probe refused: channel={:?} path={} evidence={}",
                    attempt.channel(),
                    attempt.path().display(),
                    attempt.evidence()
                )));
            }
        }
        let probe_content = b"aria-boundary-probe".to_vec();
        for path in std::iter::once(&runtime_probe)
            .chain(target_probe.as_ref())
            .chain([&terminal_probe, &mcp_probe])
        {
            let observed = fs::read(path).unwrap_or_default();
            if observed != probe_content {
                return Err(ProviderBoundaryError::ProbeFailed(format!(
                    "positive probe unverified on host: {} did not land verbatim",
                    path.display()
                )));
            }
        }
        // 8) 受控 commit(Coding):target 内 git commit 宿主直接执行成功
        //    (git identity 沿冻结授权链)。
        let controlled_commit = if coding {
            Some(run_controlled_commit(fixture, &env).await?)
        } else {
            None
        };
        // 9) 证据工件落盘(artifact_ref 必须指向真实存在的文件;schema v2
        //    含执行语义/岗位/窗口/资源身份,不含任何 secret 值)。
        let probed_at = chrono::Utc::now().to_rfc3339();
        let session_dir = fixture.evidence_session_dir();
        fs::create_dir_all(&session_dir).map_err(|error| {
            ProviderBoundaryError::ProbeFailed(format!(
                "boundary_artifact_unwritten: cannot create {}: {error}",
                session_dir.display()
            ))
        })?;
        let artifact_path = session_dir.join("evidence.json");
        let artifact = BoundaryProbeEvidenceArtifact {
            schema: BOUNDARY_PROBE_EVIDENCE_SCHEMA,
            provider: format!("{:?}", fixture.provider()),
            provider_family: provider_family_text(&fixture.provider()).to_string(),
            cli_program: fixture.cli_program().to_string(),
            cli_version_argv: vec![fixture.cli_program().to_string(), "--version".to_string()],
            cli_exact_version: cli_version.clone(),
            os: os_text(),
            execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
            duty: scope.duty().to_string(),
            review_window_id: scope.review_window_id().map(str::to_string),
            resource_scope_id: scope.resource_scope_id().to_string(),
            action: action_text(projection.action()).to_string(),
            boundary_mode: boundary_mode_text(plan.mode()).to_string(),
            role: format!("{:?}", projection.role()),
            fixture_digest: fixture.fixture_digest(),
            profile_digest: projection.capability_projection_digest().to_string(),
            session_projection_digest: projection.projection_digest().to_string(),
            host_probe_argv: probe_argv,
            positives: attempts.iter().map(attempt_record).collect(),
            controlled_commit,
            resume,
            observation: ObservationRecord {
                attempt_count: attempts.len(),
                evidence_bytes: observation_bytes,
                budget_bytes: observation_budget,
                complete: true,
            },
            write_boundary_state: "Confirmed".to_string(),
            probed_at: probed_at.clone(),
        };
        let artifact_json = serde_json::to_string_pretty(&artifact).map_err(|error| {
            ProviderBoundaryError::ProbeFailed(format!(
                "boundary_artifact_unwritten: cannot serialize evidence: {error}"
            ))
        })?;
        fs::write(&artifact_path, artifact_json).map_err(|error| {
            ProviderBoundaryError::ProbeFailed(format!(
                "boundary_artifact_unwritten: cannot write {}: {error}",
                artifact_path.display()
            ))
        })?;
        // 10) 签发:真实正向探针 + 岗位/窗口/资源身份齐备后构造 host-open
        //     evidence(语义/岗位/窗口随证据进入 2c/2d 导入链)。
        Ok(ProviderBoundaryEvidence::new(
            fixture.provider(),
            cli_version,
            plan.mode(),
            projection.projection_digest().to_string(),
            artifact_path.to_string_lossy().into_owned(),
            probed_at,
            PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
            scope.duty().to_string(),
            scope.review_window_id().map(str::to_string),
        ))
    }

    /// capability 三态视角(Confirmed 只能来自 `run` 的真实正向探针+
    /// 岗位/窗口受信在场签发):Ok=Confirmed;材料不一致/岗位窗口缺失/
    /// 版本漂移/观测缺失/超预算/工件未落盘=Unknown;防护失效(正向写
    /// 真实失败)=Denied。
    pub fn evidence_state(
        outcome: &Result<ProviderBoundaryEvidence, ProviderBoundaryError>,
    ) -> ProviderCapabilityEvidence {
        match outcome {
            Ok(_) => ProviderCapabilityEvidence::Confirmed,
            Err(ProviderBoundaryError::Unsupported(_))
            | Err(ProviderBoundaryError::InvalidPlan(_)) => ProviderCapabilityEvidence::Unknown,
            Err(ProviderBoundaryError::ProbeFailed(reason)) => {
                if UNKNOWN_STATE_CODES
                    .iter()
                    .any(|code| reason.starts_with(code))
                {
                    ProviderCapabilityEvidence::Unknown
                } else {
                    ProviderCapabilityEvidence::Denied {
                        reason: reason.clone(),
                    }
                }
            }
        }
    }

    /// 证据工件 resume 面的三态(capability `resume` 列的签发口径;2d
    /// 导入 resume 格消费):Confirmed=launch/resume/负探针全部真实通过;
    /// Denied=CLI 不支持/同 id 续开 id 不符/错 id 回显原 id(带真实错误);
    /// Unknown=未探测/超时/工件不可读。
    pub fn artifact_resume_state(artifact_ref: &str) -> ProviderCapabilityEvidence {
        let Ok(content) = fs::read_to_string(artifact_ref) else {
            return ProviderCapabilityEvidence::Unknown;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&content) else {
            return ProviderCapabilityEvidence::Unknown;
        };
        let resume = &value["resume"];
        if resume.get("probed").and_then(|probed| probed.as_bool()) != Some(true) {
            return ProviderCapabilityEvidence::Unknown;
        }
        match resume.get("state").and_then(|state| state.as_str()) {
            Some("Confirmed") => ProviderCapabilityEvidence::Confirmed,
            Some("Denied") => ProviderCapabilityEvidence::Denied {
                reason: resume
                    .get("reason")
                    .and_then(|reason| reason.as_str())
                    .unwrap_or_default()
                    .to_string(),
            },
            _ => ProviderCapabilityEvidence::Unknown,
        }
    }
}

/// 单次写探针记录(通道/路径/结果/errno 证据;evidence 工件元素)。
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct AttemptRecord {
    channel: String,
    path: String,
    outcome: String,
    evidence: String,
}

fn attempt_record(attempt: &BoundaryWriteAttempt) -> AttemptRecord {
    AttemptRecord {
        channel: format!("{:?}", attempt.channel()),
        path: attempt.path().to_string_lossy().into_owned(),
        outcome: if attempt.result() == Ok(()) {
            "allowed"
        } else {
            "refused"
        }
        .to_string(),
        evidence: attempt.evidence().to_string(),
    }
}

/// 受控 commit 记录(subject + 宿主复核 + actual argv)。
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct CommitRecord {
    subject: String,
    verified_on_host: bool,
    argv: Vec<String>,
}

/// 观测面记录(沿 HEAD 既有预算,不截断)。
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct ObservationRecord {
    attempt_count: usize,
    evidence_bytes: usize,
    budget_bytes: usize,
    complete: bool,
}

/// probe 证据工件(`evidence.json`,schema v2):观测记录本体,durable
/// capability 导入归 2d(经 2c shape 校验),本工件不入 git。执行语义/
/// 岗位/窗口/资源身份随工件落盘(批2 Task 9);不含任何 secret 值。
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct BoundaryProbeEvidenceArtifact {
    schema: &'static str,
    provider: String,
    provider_family: String,
    cli_program: String,
    cli_version_argv: Vec<String>,
    cli_exact_version: String,
    os: String,
    execution_semantics: String,
    duty: String,
    review_window_id: Option<String>,
    resource_scope_id: String,
    action: String,
    boundary_mode: String,
    role: String,
    fixture_digest: String,
    profile_digest: String,
    session_projection_digest: String,
    host_probe_argv: Vec<String>,
    positives: Vec<AttemptRecord>,
    controlled_commit: Option<CommitRecord>,
    resume: ResumeProbeRecord,
    observation: ObservationRecord,
    write_boundary_state: String,
    probed_at: String,
}

/// resume 面 probe 记录(launch/native id/同 id resume/错 id 负探针;
/// `probed=false` = 规格缺省,launch/write 面不受影响)。
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct ResumeProbeRecord {
    probed: bool,
    /// Confirmed | Denied | Unknown | not_probed。
    state: String,
    reason: String,
    channel_kind: String,
    launch_argv: Vec<String>,
    launch_exit: Option<i32>,
    launch_native_session_id: Option<String>,
    resume_argv: Vec<String>,
    resume_exit: Option<i32>,
    resume_native_session_id: Option<String>,
    resume_reply_excerpt: String,
    wrong_id_argv: Vec<String>,
    wrong_id_exit: Option<i32>,
    wrong_id_native_session_id: Option<String>,
    wrong_id_rejected: Option<bool>,
    id_confirmed_same: Option<bool>,
}

impl ResumeProbeRecord {
    fn not_probed() -> Self {
        Self {
            probed: false,
            state: "not_probed".to_string(),
            reason: String::new(),
            channel_kind: String::new(),
            launch_argv: Vec::new(),
            launch_exit: None,
            launch_native_session_id: None,
            resume_argv: Vec::new(),
            resume_exit: None,
            resume_native_session_id: None,
            resume_reply_excerpt: String::new(),
            wrong_id_argv: Vec::new(),
            wrong_id_exit: None,
            wrong_id_native_session_id: None,
            wrong_id_rejected: None,
            id_confirmed_same: None,
        }
    }

    fn probing(kind: ResumeChannelKind) -> Self {
        Self {
            probed: true,
            state: "Unknown".to_string(),
            channel_kind: kind.as_str().to_string(),
            ..Self::not_probed()
        }
    }

    fn deny(&mut self, reason: String) {
        self.state = "Denied".to_string();
        self.reason = reason;
    }

    fn unknown(&mut self, reason: String) {
        self.state = "Unknown".to_string();
        self.reason = reason;
    }
}

/// 由 projection/fixture 派生探测用 boundary plan,并校验一致性(cwd/
/// target/action 语义;材料不匹配=未探测,InvalidPlan → Unknown)。
fn derive_boundary_plan(
    projection: &ProviderPolicyProjection,
    fixture: &BoundaryFixture,
) -> Result<ProviderBoundaryPlan, ProviderBoundaryError> {
    if projection.working_directory() != fixture.root() {
        return Err(ProviderBoundaryError::InvalidPlan(format!(
            "projection working directory {} != fixture root {}",
            projection.working_directory().display(),
            fixture.root().display()
        )));
    }
    match projection.action() {
        SessionPolicyAction::CodingTargetWrite => {
            let Some(target) = fixture.target() else {
                return Err(ProviderBoundaryError::InvalidPlan(
                    "coding probe fixture must carry a target worktree".to_string(),
                ));
            };
            if projection.target().worktree != target {
                return Err(ProviderBoundaryError::InvalidPlan(format!(
                    "projection target {} != fixture target {}",
                    projection.target().worktree.display(),
                    target.display()
                )));
            }
            Ok(ProviderBoundaryPlan::new(
                ProviderBoundaryMode::TargetWriteOnly,
                fixture.root().to_path_buf(),
                Some(target.to_path_buf()),
                Vec::new(),
            ))
        }
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            if fixture.target().is_some() {
                return Err(ProviderBoundaryError::InvalidPlan(
                    "read-only probe fixture must not carry a target worktree".to_string(),
                ));
            }
            Ok(ProviderBoundaryPlan::new(
                ProviderBoundaryMode::ReadOnly,
                fixture.root().to_path_buf(),
                None,
                Vec::new(),
            ))
        }
    }
}

/// 岗位/窗口/资源身份受信校验(批2 Task 9):岗位必须与 action 执行语义
/// 相符(Coding=author,只读=reviewer);Reviewer 缺窗口、缺资源身份、
/// 岗位不符一律拒签(InvalidPlan → Unknown;probe 不采信模型自报)。
fn validate_scope_for_action(
    scope: &HostOpenProbeScope,
    action: SessionPolicyAction,
) -> Result<(), ProviderBoundaryError> {
    let expected_duty = match action {
        SessionPolicyAction::CodingTargetWrite => TrustedProviderRole::Author.as_str(),
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            TrustedProviderRole::Reviewer.as_str()
        }
    };
    if scope.duty() != expected_duty {
        return Err(ProviderBoundaryError::InvalidPlan(format!(
            "boundary_scope_missing: action {action:?} 要求岗位 {expected_duty:?},scope 携带 {:?}(岗位缺失/不符不签)",
            scope.duty()
        )));
    }
    if expected_duty == TrustedProviderRole::Reviewer.as_str()
        && scope.review_window_id().map(str::is_empty).unwrap_or(true)
    {
        return Err(ProviderBoundaryError::InvalidPlan(
            "boundary_scope_missing: reviewer 探针缺 review 窗口引用(窗口缺失不签)".to_string(),
        ));
    }
    if scope.resource_scope_id().trim().is_empty() {
        return Err(ProviderBoundaryError::InvalidPlan(
            "boundary_scope_missing: resource scope id 缺失(审计/资源身份不可追溯)".to_string(),
        ));
    }
    Ok(())
}

/// 宿主全开写面探针(批2 Task 9):受控 fixture 内以宿主身份直接执行
/// `sh` 探针脚本(无 bwrap),按通道逐路径尝试写入并解析
/// `A <i> ok|refused <evidence>` 哨兵。宿主普通写成功=全开事实;进程
/// 失败/超时=未观测(boundary_unobserved),不产生 attempt 冒充。
async fn run_host_write_surface_probe(
    env: &BTreeMap<String, String>,
    cwd: &Path,
    planned: &[PlannedBoundaryWrite],
) -> Result<Vec<BoundaryWriteAttempt>, ProviderBoundaryError> {
    let mut script = String::new();
    for (index, planned) in planned.iter().enumerate() {
        let quoted = shell_quote(&planned.path().to_string_lossy());
        let write = probe_write_command(planned.channel(), &quoted);
        // 错误采集文件落在受控 fixture 内(cwd 相对),不触宿主其它目录。
        let error_file = format!(".aria-host-open-probe-{index}.err");
        script.push_str(&format!(
            "if ( {write} ) 2>{error_file}; then printf 'A {index} ok\\n'; \
             else printf 'A {index} refused %s\\n' \"$(tr '\\n' ' ' <{error_file} | cut -c1-160)\"; fi\n"
        ));
    }
    let invocation = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&script)
        .current_dir(cwd)
        .envs(env)
        .output();
    let output = tokio::time::timeout(std::time::Duration::from_secs(60), invocation)
        .await
        .map_err(|_| {
            ProviderBoundaryError::ProbeFailed(
                "boundary_unobserved: host write-surface probe timed out (60s)".to_string(),
            )
        })?
        .map_err(|error| {
            ProviderBoundaryError::ProbeFailed(format!(
                "boundary_unobserved: host write-surface probe spawn failed: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(ProviderBoundaryError::ProbeFailed(format!(
            "boundary_unobserved: host write-surface probe channel failed: exit {:?}: {}",
            output.status.code(),
            text_excerpt(&String::from_utf8_lossy(&output.stderr), 200)
        )));
    }
    let stdout_text = bounded_text(&output.stdout);
    let mut attempts = Vec::with_capacity(planned.len());
    for (index, planned) in planned.iter().enumerate() {
        let sentinel = stdout_text
            .lines()
            .find_map(|line| line.strip_prefix(&format!("A {index} ")));
        let attempt = match sentinel {
            Some(rest) if rest.starts_with("ok") => {
                BoundaryWriteAttempt::allowed(planned.channel(), planned.path().to_path_buf())
            }
            Some(rest) => BoundaryWriteAttempt::refused_with(
                planned.channel(),
                planned.path().to_path_buf(),
                rest.trim_start_matches("refused").trim().to_string(),
            ),
            None => BoundaryWriteAttempt::refused_with(
                planned.channel(),
                planned.path().to_path_buf(),
                "probe sentinel missing (write unobserved)".to_string(),
            ),
        };
        attempts.push(attempt);
    }
    Ok(attempts)
}

/// Coding 受控 commit(批2 Task 9):target 内 `git add + commit` 在宿主
/// 直接执行(git identity 沿冻结授权链),宿主复核提交 subject。
async fn run_controlled_commit(
    fixture: &BoundaryFixture,
    env: &BTreeMap<String, String>,
) -> Result<CommitRecord, ProviderBoundaryError> {
    let Some(target) = fixture.target() else {
        return Err(ProviderBoundaryError::InvalidPlan(
            "controlled commit requires a coding target worktree".to_string(),
        ));
    };
    let target_text = target.to_string_lossy().into_owned();
    let script = format!(
        "git -C '{target_text}' add aria-boundary-probe.txt && \
         git -C '{target_text}' -c commit.gpgsign=false commit -q -m aria-boundary-probe-commit"
    );
    let argv = argv_with_program("sh", &["-c".to_string(), script.clone()]);
    let commit = run_env_capture("sh", &["-c".to_string(), script], fixture.root(), env, 120).await;
    if commit.timed_out {
        return Err(ProviderBoundaryError::ProbeFailed(
            "controlled commit timed out (120s)".to_string(),
        ));
    }
    if !commit.succeeded() {
        return Err(ProviderBoundaryError::ProbeFailed(format!(
            "controlled commit failed: {}{}",
            commit.exit_summary(),
            text_excerpt(&commit.stderr, 200)
        )));
    }
    let log = Command::new("git")
        .arg("-C")
        .arg(target)
        .args(["log", "-1", "--pretty=%s"])
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .map_err(|error| {
            ProviderBoundaryError::ProbeFailed(format!("controlled commit verify spawn: {error}"))
        })?;
    let subject = String::from_utf8_lossy(&log.stdout).trim().to_string();
    if !log.status.success() || subject != "aria-boundary-probe-commit" {
        return Err(ProviderBoundaryError::ProbeFailed(format!(
            "controlled commit subject mismatch on host: {subject:?}"
        )));
    }
    Ok(CommitRecord {
        subject: "aria-boundary-probe-commit".to_string(),
        verified_on_host: true,
        argv,
    })
}

/// resume 面 probe 段:宿主侧(LLM 会话需要真实 HOME/网络/登录态)在
/// fixture root 内执行「真实 launch→同 id resume→错 id 负探针」。native
/// id 语义与写边界正交:本段结果只写入工件 resume 段,不影响写面签发。
async fn run_resume_segment(
    spec: &ResumeProbeSpec,
    cli_program: &str,
    cwd: &Path,
) -> ResumeProbeRecord {
    let kind = spec.kind();
    let prompt = spec.prompt();
    let timeout = spec.timeout_secs();
    let mut record = ResumeProbeRecord::probing(kind);

    // 1) 真实 launch:记录 native session id。
    let launch_args = kind.launch_argv(prompt);
    record.launch_argv = argv_with_program(cli_program, &launch_args);
    let launch = run_cli_capture(cli_program, &launch_args, cwd, timeout).await;
    record.launch_exit = launch.exit_code();
    if launch.timed_out {
        record.unknown(format!("resume probe timed out after {timeout}s (launch)"));
        return record;
    }
    if !launch.succeeded() {
        record.deny(format!(
            "resume launch failed: {}{}",
            launch.exit_summary(),
            text_excerpt(&launch.stderr, 200)
        ));
        return record;
    }
    let Some(native_id) = kind.extract_native_id(&launch.stdout) else {
        record.deny(format!(
            "resume launch produced no native session id{}",
            text_excerpt(&launch.stdout, 200)
        ));
        return record;
    };
    record.launch_native_session_id = Some(native_id.clone());

    // 2) 同 id 真实 resume:native id 确认同 id。
    let resume_args = kind.resume_argv(&native_id, prompt);
    record.resume_argv = argv_with_program(cli_program, &resume_args);
    let resume = run_cli_capture(cli_program, &resume_args, cwd, timeout).await;
    record.resume_exit = resume.exit_code();
    if resume.timed_out {
        record.unknown(format!("resume probe timed out after {timeout}s (resume)"));
        return record;
    }
    if !resume.succeeded() {
        record.deny(format!(
            "resume invocation failed: {}{}",
            resume.exit_summary(),
            text_excerpt(&resume.stderr, 200)
        ));
        return record;
    }
    record.resume_reply_excerpt = text_excerpt(&resume.stdout, 200);
    let resumed_id = kind.extract_native_id(&resume.stdout);
    let confirmed_same = resumed_id.as_deref() == Some(native_id.as_str());
    record.resume_native_session_id = resumed_id.clone();
    record.id_confirmed_same = Some(confirmed_same);
    if !confirmed_same {
        record.deny(format!(
            "resume native id mismatch: requested {native_id} got {:?}",
            resumed_id.as_deref().unwrap_or("<none>")
        ));
        return record;
    }

    // 3) 错 id 负探针:错 id 不得回显**原** native id(claude/codex/kimi
    //    以真实报错满足;pi 以新建另一 id 满足)。
    let bogus = kind.bogus_native_id();
    let wrong_args = kind.resume_argv(&bogus, prompt);
    record.wrong_id_argv = argv_with_program(cli_program, &wrong_args);
    let wrong = run_cli_capture(cli_program, &wrong_args, cwd, timeout).await;
    record.wrong_id_exit = wrong.exit_code();
    if wrong.timed_out {
        record.unknown(format!(
            "resume probe timed out after {timeout}s (bogus id)"
        ));
        return record;
    }
    let wrong_id = if wrong.succeeded() {
        kind.extract_native_id(&wrong.stdout)
    } else {
        None
    };
    record.wrong_id_native_session_id = wrong_id.clone();
    let wrong_rejected = wrong_id.as_deref() != Some(native_id.as_str());
    record.wrong_id_rejected = Some(wrong_rejected);
    if !wrong_rejected {
        record.deny(format!(
            "resume confirmed the original native id {native_id} for a bogus native id"
        ));
        return record;
    }

    record.state = "Confirmed".to_string();
    record
}

/// 宿主侧 CLI 捕获(有界输出 + 超时 fail-closed)。
struct CliCapture {
    timed_out: bool,
    spawn_error: Option<String>,
    status_success: bool,
    exit_code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl CliCapture {
    fn exit_code(&self) -> Option<i32> {
        self.exit_code
    }

    fn succeeded(&self) -> bool {
        self.spawn_error.is_none() && self.status_success
    }

    fn exit_summary(&self) -> String {
        if let Some(error) = &self.spawn_error {
            return format!("cannot execute: {error}");
        }
        format!("exit {:?}: ", self.exit_code)
    }
}

/// 单次输出捕获上限(超限截断记录;LLM 轮次输出远小于此)。
const CLI_CAPTURE_TEXT_LIMIT: usize = 64 * 1024;

async fn run_cli_capture(
    program: &str,
    args: &[String],
    cwd: &Path,
    timeout_secs: u64,
) -> CliCapture {
    let invocation = tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .output();
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), invocation).await;
    match output {
        Err(_) => CliCapture {
            timed_out: true,
            spawn_error: None,
            status_success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
        },
        Ok(Err(error)) => CliCapture {
            timed_out: false,
            spawn_error: Some(error.to_string()),
            status_success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
        },
        Ok(Ok(output)) => CliCapture {
            timed_out: false,
            spawn_error: None,
            status_success: output.status.success(),
            exit_code: output.status.code(),
            stdout: bounded_text(&output.stdout),
            stderr: bounded_text(&output.stderr),
        },
    }
}

/// 环境感知的宿主侧捕获(受控 commit 等需要 env overlay 的宿主执行;
/// 与 `run_cli_capture` 同构,叠加显式 env)。
async fn run_env_capture(
    program: &str,
    args: &[String],
    cwd: &Path,
    env: &BTreeMap<String, String>,
    timeout_secs: u64,
) -> CliCapture {
    let invocation = tokio::process::Command::new(program)
        .args(args)
        .current_dir(cwd)
        .envs(env)
        .output();
    let output =
        tokio::time::timeout(std::time::Duration::from_secs(timeout_secs), invocation).await;
    match output {
        Err(_) => CliCapture {
            timed_out: true,
            spawn_error: None,
            status_success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
        },
        Ok(Err(error)) => CliCapture {
            timed_out: false,
            spawn_error: Some(error.to_string()),
            status_success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: String::new(),
        },
        Ok(Ok(output)) => CliCapture {
            timed_out: false,
            spawn_error: None,
            status_success: output.status.success(),
            exit_code: output.status.code(),
            stdout: bounded_text(&output.stdout),
            stderr: bounded_text(&output.stderr),
        },
    }
}

/// 有界文本(Lossy + 截断到捕获上限)。
fn bounded_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    text.chars().take(CLI_CAPTURE_TEXT_LIMIT).collect()
}

/// 记录摘录:压缩空白并截断到 `limit` 字符。
fn text_excerpt(text: &str, limit: usize) -> String {
    let flattened: String = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    flattened.chars().take(limit).collect()
}

/// 完整 argv 记录(程序名 + 参数)。
fn argv_with_program(program: &str, args: &[String]) -> Vec<String> {
    let mut argv = vec![program.to_string()];
    argv.extend(args.iter().cloned());
    argv
}

/// 宿主侧真实 CLI 版本探测(`<cli> --version`,trimmed 完整 stdout;与
/// capability probe 的版本口径一致)。CLI 缺失/失败 → boundary_unavailable
/// (Unknown),不冒充探测。
pub(crate) fn probe_cli_version(cli_program: &str) -> Result<String, ProviderBoundaryError> {
    let output = Command::new(cli_program)
        .arg("--version")
        .output()
        .map_err(|error| {
            ProviderBoundaryError::Unsupported(format!(
                "boundary_unavailable: cannot execute `{cli_program} --version`: {error}"
            ))
        })?;
    if !output.status.success() {
        return Err(ProviderBoundaryError::Unsupported(format!(
            "boundary_unavailable: `{cli_program} --version` exited with {:?}: {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if version.is_empty() {
        return Err(ProviderBoundaryError::Unsupported(format!(
            "boundary_unavailable: `{cli_program} --version` produced no version text"
        )));
    }
    Ok(version)
}

/// 以被测 provider 的真实 projector 构造 probe 用 projection(fixture
/// 材料与 projection 的 cwd/target/action 完全一致;envelope 直构)。
/// `pub(crate)`:`ProviderProjectionInput` 构造冻结为 crate 内(1a),本
/// 构造与 [`run_cli_boundary_probe`] 是外部调用方(矩阵 harness 等)经
/// 由的受控 seam。
pub(crate) fn boundary_probe_projection(
    provider: ProviderName,
    fixture: &BoundaryFixture,
    exact_version: &str,
    role: crate::protocol::contracts::AdapterRole,
) -> ProviderPolicyProjection {
    use crate::cross_cutting::claude_code_provider::ClaudePolicyProjector;
    use crate::cross_cutting::codex_provider::CodexPolicyProjector;
    use crate::cross_cutting::kimi_code_provider::KimiPolicyProjector;
    use crate::cross_cutting::pi_provider::PiPolicyProjector;
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::logical_codebase::policy::{
        PolicyTarget, ProviderDialect, SessionPolicyEnvelope,
    };
    use crate::product::logical_codebase::provider_gateway::ProviderRef;
    use crate::product::logical_codebase::provider_projection::ProviderPolicyProjector;
    use crate::product::logical_codebase::provider_projection::ProviderProjectionInput;

    let action = fixture.boundary_action();
    let (dialect, mcp_source) = match &provider {
        ProviderName::ClaudeCode => (ProviderDialect::ClaudeCodeCliV1, ""),
        ProviderName::Codex => (ProviderDialect::CodexCliV1, ""),
        ProviderName::Pi => (ProviderDialect::PiRpcV1, ""),
        // kimi 投影要求 MCP 来源非空:无 Aria 注入时用 native 标记
        //(与 `KIMI_NATIVE_MCP_SOURCE` 冻结同值)。
        ProviderName::KimiCode => (ProviderDialect::KimiAcpV1, "native-project-config"),
        ProviderName::Fake => unreachable!("fake has no real projector"),
    };
    let target_worktree = fixture
        .target()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| fixture.root().to_path_buf());
    let writable_roots = match action {
        SessionPolicyAction::CodingTargetWrite => vec![target_worktree.clone()],
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => Vec::new(),
    };
    let envelope = SessionPolicyEnvelope {
        policy_id: "lc-boundary-probe-policy".to_string(),
        policy_revision: 1,
        policy_digest: format!("sha256:{}", "0".repeat(64)),
        action,
        target: PolicyTarget::checkout("lc_probe_repo", "lc_probe_checkout", target_worktree),
        working_directory: fixture.root().to_path_buf(),
        readable_roots: vec![
            fixture.root().to_path_buf(),
            fixture.member().to_path_buf(),
            fixture.member_b().to_path_buf(),
        ],
        writable_roots,
        provider_dialect: dialect,
        config_artifact_ref: "lc-boundary-probe-config".to_string(),
        config_digest: format!("sha256:{}", "1".repeat(64)),
        created_at: "2026-10-04T00:00:00Z".to_string(),
        authority_root: fixture.root().to_path_buf(),
    };
    let boundary_mode = match action {
        SessionPolicyAction::CodingTargetWrite => ProviderBoundaryMode::TargetWriteOnly,
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            ProviderBoundaryMode::ReadOnly
        }
    };
    let boundary = ProviderBoundaryPlan::new(
        boundary_mode,
        fixture.root().to_path_buf(),
        fixture.target().map(Path::to_path_buf),
        Vec::new(),
    );
    let provider_ref = match provider {
        ProviderName::ClaudeCode => ProviderRef::claude_code("lc_boundary_probe"),
        ProviderName::Codex => ProviderRef::codex("lc_boundary_probe"),
        ProviderName::Pi => ProviderRef::pi("lc_boundary_probe"),
        ProviderName::KimiCode => ProviderRef::kimi_code("lc_boundary_probe"),
        ProviderName::Fake => unreachable!("fake has no gateway ref"),
    };
    let input = ProviderProjectionInput::new(
        envelope,
        provider_ref,
        action,
        role,
        ProviderPermissionMode::Auto,
        None,
        "lc-boundary-probe".to_string(),
        mcp_source.to_string(),
        "lc-boundary-probe-config".to_string(),
        String::new(),
        Some(boundary),
    );
    match provider {
        ProviderName::ClaudeCode => ClaudePolicyProjector::new(exact_version)
            .project(&input)
            .expect("claude probe projection"),
        ProviderName::Codex => CodexPolicyProjector::new(exact_version)
            .project(&input)
            .expect("codex probe projection"),
        ProviderName::Pi => PiPolicyProjector::new(exact_version)
            .project(&input)
            .expect("pi probe projection"),
        ProviderName::KimiCode => KimiPolicyProjector::new(exact_version)
            .project(&input)
            .expect("kimi probe projection"),
        ProviderName::Fake => unreachable!("fake has no real projector"),
    }
}
/// 6c 现场探针结果(外部调用方的 2d 导入材料包):evidence + projection +
/// 导入用 record(record.resume 格取工件签发的 `artifact_resume_state`,
/// write_boundary=Confirmed 由 probe 签发;launch 未探测保持 Unknown,由
/// 2b 过渡桥继续覆盖)。projection 的身份 getter 冻结为 crate 内,外部
/// 调用方经本材料包完成 2d 导入,不自造 record 身份字段。
#[derive(Debug, Clone)]
pub struct CliBoundaryProbeOutcome {
    pub evidence: ProviderBoundaryEvidence,
    pub projection: ProviderPolicyProjection,
    pub record:
        crate::product::logical_codebase::provider_capability_store::ProviderCapabilityRecord,
}

/// probe 签发证据对应的 2d 导入 record(与 6c 测试的 shape_validation_
/// record 同构造:三方一致字段对齐,write_boundary=Confirmed、resume=工件
/// 签发、launch=Unknown)。
pub(crate) fn probe_import_record(
    evidence: &ProviderBoundaryEvidence,
    projection: &ProviderPolicyProjection,
    resume: ProviderCapabilityEvidence,
) -> crate::product::logical_codebase::provider_capability_store::ProviderCapabilityRecord {
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, MANAGED_CAPABILITY_SNAPSHOT_REF,
        PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability, ProviderActionMatrix,
        ProviderCapabilityRecord,
    };
    use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;

    let action = projection.action();
    let row = ProviderActionCapability {
        action,
        // r48(三家 r2):launch 改签 Confirmed——探针本身真实启动了 CLI
        //(launch+正负写探针+resume 全真实验证,工件含真实 argv 与启动
        // 成功),launch=Unknown 是 d8b9ea37 的设计保守而非无证据;无
        // bootstrap 记录的三家(Codex/Pi/Kimi)探针直建记录后靠 2b
        // 过渡桥(需 bootstrap 旧 allow 列表)放行——三家无 bootstrap
        // 记录→桥不放行→launch cell Unknown 拒收(r2 现场共通首错)。
        launch: ProviderCapabilityEvidence::Confirmed,
        resume,
        write_boundary: ProviderCapabilityEvidence::Confirmed,
        projection_digest: evidence.projection_digest().to_string(),
        evidence_ref: evidence.artifact_ref().to_string(),
    };
    ProviderCapabilityRecord {
        provider_type: projection.provider_type(),
        schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
        version: evidence.exact_version().to_string(),
        adapter_dialect: projection.provider_dialect(),
        wire_dialect: projection.wire_dialect(),
        // r47(pi/kimi 首轮):capability_snapshot_ref 必须与运行时 ProviderRef
        // 的约定值一致(cap_managed_snapshot——launch 链 from_provider_name
        // 统一传入)。此前写 "lc_boundary_probe":bootstrap(Claude recipe)
        // 先建过记录的家(Claude)经 import 合并保留既有 ref 不受影响;
        // 无 bootstrap 记录的 Pi/Kimi 探针直建记录 → gateway load_record
        // 比对 mismatch。
        capability_snapshot_ref: MANAGED_CAPABILITY_SNAPSHOT_REF.to_string(),
        evidence: CapabilityEvidence::ProductionVerified,
        resume_evidence: ResumeEvidenceState::Unsupported,
        supported_actions: vec![action],
        action_matrix: ProviderActionMatrix::from_rows(vec![row]).expect("probe action rows"),
        trust: ProviderCapabilityEvidence::Unknown,
        probed_at: Some(evidence.probed_at().to_string()),
        probe_artifact_ref: Some(evidence.artifact_ref().to_string()),
        execution_semantics: evidence.execution_semantics().to_string(),
    }
}

/// 6c 现场探针的外部入口(矩阵 harness 等 crate 外调用方):受控 fixture +
/// 真实 projector 组装 probe 投影并执行完整真实 probe(`resume` 规格在场时
/// 覆盖 resume 面:真实 launch→同 id resume→错 id 负探针;批2 Task 9 起
/// `scope` 携带受信岗位/窗口/资源身份,缺失即拒签)。返回 evidence/
/// projection/record 材料包,record 经 2d `record_verified_probe` 导入
/// durable;任何环节失败返回稳定码错误——调用方 BLOCKED 真实报告,不伪造。
#[allow(clippy::too_many_arguments)] // crate 外矩阵 harness 的稳定入口,参数逐项对应探针输入契约。
pub async fn run_cli_boundary_probe(
    provider: ProviderName,
    cli_program: &str,
    action: SessionPolicyAction,
    base: &Path,
    evidence_root: &Path,
    session_label: &str,
    resume: Option<ResumeProbeSpec>,
    scope: HostOpenProbeScope,
) -> Result<CliBoundaryProbeOutcome, ProviderBoundaryError> {
    let mut fixture = BoundaryFixture::create(
        provider.clone(),
        cli_program,
        action,
        base,
        evidence_root,
        session_label,
    )?;
    if let Some(resume) = resume {
        fixture = fixture.with_resume(resume);
    }
    // 宿主侧当前版本:与 probe 内部版本门同一口径(漂移即拒)。
    let version = probe_cli_version(cli_program)?;
    let role = match action {
        SessionPolicyAction::CodingTargetWrite => crate::protocol::contracts::AdapterRole::Executor,
        SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
            crate::protocol::contracts::AdapterRole::Reviewer
        }
    };
    let projection = boundary_probe_projection(provider, &fixture, &version, role);
    let evidence = ProviderBoundaryProbe::run(&projection, &fixture, &scope).await?;
    // record.resume 格取工件签发结果(6c worker 提醒:不自报)。
    let resume = ProviderBoundaryProbe::artifact_resume_state(evidence.artifact_ref());
    let record = probe_import_record(&evidence, &projection, resume);
    Ok(CliBoundaryProbeOutcome {
        evidence,
        projection,
        record,
    })
}

/// 成功且非空才返回的命令输出采集(best-effort 记录用)。
fn capture_trim(program: &Path, args: &[&str]) -> Option<String> {
    let output = Command::new(program).args(args).output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// OS 记录(uname -sr;不可得时退化为 consts 的 OS/arch 描述)。
fn os_text() -> String {
    capture_trim(Path::new("uname"), &["-sr"])
        .unwrap_or_else(|| format!("{} {}", std::env::consts::OS, std::env::consts::ARCH))
}

fn boundary_mode_text(mode: ProviderBoundaryMode) -> &'static str {
    match mode {
        ProviderBoundaryMode::ReadOnly => "read-only",
        ProviderBoundaryMode::TargetWriteOnly => "target-write-only",
    }
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{
        BoundaryFixture, HostOpenProbeScope, ProviderBoundaryProbe, ResumeChannelKind,
        ResumeProbeSpec,
    };

    use crate::cross_cutting::provider_boundary::{
        PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN, ProviderBoundaryError, ProviderBoundaryMode,
    };
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;

    use crate::product::logical_codebase::policy::SessionPolicyAction;
    use crate::product::logical_codebase::provider_capability_probe::ProviderCapabilityProbeService;
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord,
    };
    use crate::product::logical_codebase::provider_gateway::ResumeEvidenceState;
    use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::AdapterRole;

    use super::action_text;
    use super::boundary_probe_projection as probe_projection;

    /// 宿主侧 CLI 版本采集(与 probe 内部同一口径:trimmed 完整 stdout)。
    fn host_cli_version(cli: &str) -> String {
        let output = std::process::Command::new(cli)
            .arg("--version")
            .output()
            .unwrap_or_else(|error| panic!("run `{cli} --version`: {error}"));
        assert!(
            output.status.success(),
            "`{cli} --version` exited with {:?}",
            output.status.code()
        );
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    }

    /// Coding/author 岗位的受信 scope(测试注入可信 fixture)。
    fn author_scope() -> HostOpenProbeScope {
        HostOpenProbeScope::author("aria-unit-probe-scope")
    }

    /// 只读/reviewer 窗口的受信 scope。
    fn reviewer_scope() -> HostOpenProbeScope {
        HostOpenProbeScope::reviewer("aria-unit-review-window", "aria-unit-probe-scope")
    }

    /// Task 6c Step 1(断言 355 逐字):未观测/超预算的 evidence 永不
    /// Confirmed——观测缺失(含旧 6a 哨兵语义)与观测面超出沿 HEAD 预算
    /// 的真实 probe 运行都必须保持 Unknown(不截断、不冒充)。
    #[tokio::test]
    async fn lcg_t06_unobserved_or_over_budget_evidence_never_confirms() {
        // 1) 分类器(`attempt_observed`)已随 T10 旧 6a bwrap 探针通道
        //    退役删除;观测缺失面改由 evidence_state 的 Unknown 分类
        //    (第 2 段)与真实探针错误码持有。
        // 2) 观测缺失 → 状态非 Confirmed(计划断言 355 逐字)。
        let unobserved_outcome: Result<
            crate::cross_cutting::provider_boundary::ProviderBoundaryEvidence,
            ProviderBoundaryError,
        > = Err(ProviderBoundaryError::ProbeFailed(
            "boundary_unobserved: channel=Mcp path=/lc/root/rogue evidence=probe sentinel missing"
                .to_string(),
        ));
        let missing_observation_state = ProviderBoundaryProbe::evidence_state(&unobserved_outcome);
        assert_ne!(
            missing_observation_state,
            ProviderCapabilityEvidence::Confirmed
        );

        // 3) 真实宿主运行 + 观测面超出预算(注入 16B 预算):拒签且保持
        //    Unknown,错误带稳定码,不截断冒充。
        let base = tempdir().expect("base dir");
        let fixture = BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "git",
            SessionPolicyAction::PlanningReadOnly,
            base.path(),
            &base.path().join("evidence"),
            "unit-over-budget",
        )
        .expect("fixture");
        let version = host_cli_version("git");
        let projection = probe_projection(
            ProviderName::ClaudeCode,
            &fixture,
            &version,
            AdapterRole::Reviewer,
        );
        let over_budget_outcome = ProviderBoundaryProbe::run_with_observation_budget(
            16,
            &projection,
            &fixture,
            &reviewer_scope(),
        )
        .await;
        assert!(
            matches!(
                &over_budget_outcome,
                Err(ProviderBoundaryError::ProbeFailed(reason))
                    if reason.starts_with("boundary_observation_over_budget")
            ),
            "over-budget observation must refuse to sign: {over_budget_outcome:?}"
        );
        let over_budget_state = ProviderBoundaryProbe::evidence_state(&over_budget_outcome);
        assert_ne!(over_budget_state, ProviderCapabilityEvidence::Confirmed);
    }

    /// Task 9(承接 6c):真实正向宿主探针才签 Confirmed——Ok evidence 的
    /// 每个字段锚定 projection/provider/版本/模式/工件/执行语义/岗位/窗口,
    /// Coding 与 read-only 两种 action 各签一份;宿主普通写(terminal 写
    /// root、MCP 写 member)成功是「全开」事实,不再判失败;证据工件含
    /// 正向探针、受控 commit 与审计/资源身份,不含物理拒写面。
    #[tokio::test]
    async fn lcg_t06_probe_signs_confirmed_only_after_real_host_open_probes() {
        let base = tempdir().expect("base dir");
        // CLI 以 git 充当(版本通道确定性;真实四家 CLI 由 live probe 覆盖)。
        let version = host_cli_version("git");
        let fixture = BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "git",
            SessionPolicyAction::CodingTargetWrite,
            base.path(),
            &base.path().join("evidence"),
            "unit-coding",
        )
        .expect("coding fixture");
        let projection = probe_projection(
            ProviderName::ClaudeCode,
            &fixture,
            &version,
            AdapterRole::Executor,
        );
        let scope = author_scope();
        let outcome = ProviderBoundaryProbe::run(&projection, &fixture, &scope).await;
        let state = ProviderBoundaryProbe::evidence_state(&outcome);
        assert_eq!(state, ProviderCapabilityEvidence::Confirmed, "{outcome:?}");
        let evidence = outcome.expect("confirmed coding evidence");
        assert_eq!(evidence.provider(), &ProviderName::ClaudeCode);
        assert_eq!(evidence.exact_version(), version);
        assert_eq!(
            evidence.boundary_mode(),
            ProviderBoundaryMode::TargetWriteOnly
        );
        assert_eq!(evidence.projection_digest(), projection.projection_digest());
        assert_eq!(
            evidence.execution_semantics(),
            PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN
        );
        assert_eq!(evidence.duty(), "author");
        assert_eq!(evidence.review_window_id(), None);
        assert!(!evidence.artifact_ref().is_empty());
        assert!(!evidence.probed_at().is_empty());
        let artifact = PathBuf::from(evidence.artifact_ref());
        assert!(artifact.is_file(), "artifact must exist on disk");
        let content = fs::read_to_string(&artifact).expect("read artifact");
        assert!(content.contains("\"positives\""));
        assert!(content.contains("\"execution_semantics\""));
        assert!(content.contains("host-open-v1"));
        assert!(content.contains("\"duty\""));
        assert!(content.contains("\"resource_scope_id\""));
        assert!(content.contains("\"host_probe_argv\""));
        assert!(content.contains("\"cli_exact_version\""));
        // 物理拒写语义面已退役:无 negatives/D4/bwrap/mounts 字段。
        assert!(!content.contains("\"negatives\""));
        assert!(!content.contains("\"d4\""));
        assert!(!content.contains("\"bwrap_path\""));
        assert!(!content.contains("\"mounts\""));
        // 宿主普通写成功=全开事实:terminal 写 root、MCP 写 member 的
        // 正向 attempt 均 allowed 且工件可见(旧语义会判 negative landed)。
        let parsed: serde_json::Value = serde_json::from_str(&content).expect("parse artifact");
        let positives = parsed["positives"].as_array().expect("positives");
        let terminal_ok = positives.iter().any(|attempt| {
            attempt["channel"] == serde_json::json!("Terminal")
                && attempt["outcome"] == serde_json::json!("allowed")
                && attempt["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("aria-host-open-terminal-probe.txt"))
        });
        let mcp_ok = positives.iter().any(|attempt| {
            attempt["channel"] == serde_json::json!("Mcp")
                && attempt["outcome"] == serde_json::json!("allowed")
                && attempt["path"]
                    .as_str()
                    .is_some_and(|path| path.ends_with("aria-host-open-mcp-probe.txt"))
        });
        assert!(
            terminal_ok,
            "terminal 宿主普通写成功必须是全开事实: {positives:?}"
        );
        assert!(mcp_ok, "MCP 宿主普通写成功必须是全开事实: {positives:?}");
        assert!(
            fixture
                .root()
                .join("aria-host-open-terminal-probe.txt")
                .is_file()
        );
        assert!(
            fixture
                .member()
                .join("aria-host-open-mcp-probe.txt")
                .is_file()
        );

        // read-only action 同样签发(reviewer 岗位 + 窗口引用)。
        let ro_base = tempdir().expect("ro base dir");
        let ro_fixture = BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "git",
            SessionPolicyAction::PlanningReadOnly,
            ro_base.path(),
            &ro_base.path().join("evidence"),
            "unit-planning",
        )
        .expect("read-only fixture");
        let ro_projection = probe_projection(
            ProviderName::ClaudeCode,
            &ro_fixture,
            &version,
            AdapterRole::Reviewer,
        );
        let ro_outcome =
            ProviderBoundaryProbe::run(&ro_projection, &ro_fixture, &reviewer_scope()).await;
        assert_eq!(
            ProviderBoundaryProbe::evidence_state(&ro_outcome),
            ProviderCapabilityEvidence::Confirmed,
            "{ro_outcome:?}"
        );
        let ro_evidence = ro_outcome.expect("confirmed read-only evidence");
        assert_eq!(ro_evidence.boundary_mode(), ProviderBoundaryMode::ReadOnly);
        assert_eq!(
            ro_evidence.projection_digest(),
            ro_projection.projection_digest()
        );
        assert_eq!(
            ro_evidence.execution_semantics(),
            PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN
        );
        assert_eq!(ro_evidence.duty(), "reviewer");
        assert_eq!(
            ro_evidence.review_window_id(),
            Some("aria-unit-review-window")
        );
    }

    /// Task 9:岗位/窗口缺失(拒签,Unknown)与 CLI 版本漂移(projection
    /// 与真实 CLI 不一致)都不签 evidence,状态保持 Unknown 且错误带稳定码。
    #[tokio::test]
    async fn lcg_t06_probe_keeps_unknown_on_scope_missing_or_version_drift() {
        let base = tempdir().expect("base dir");
        let fixture = BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "git",
            SessionPolicyAction::CodingTargetWrite,
            base.path(),
            &base.path().join("evidence"),
            "unit-unknown",
        )
        .expect("fixture");

        // 岗位与 action 不符(Coding 证据携带 reviewer 岗位):拒签,Unknown。
        let wrong_duty = ProviderBoundaryProbe::run_with_observation_budget(
            8192,
            &probe_projection(
                ProviderName::ClaudeCode,
                &fixture,
                "git version 0.0.0-probe",
                AdapterRole::Executor,
            ),
            &fixture,
            &reviewer_scope(),
        )
        .await;
        assert!(
            matches!(
                &wrong_duty,
                Err(ProviderBoundaryError::InvalidPlan(reason))
                    if reason.starts_with("boundary_scope_missing")
            ),
            "duty/action mismatch must refuse to sign: {wrong_duty:?}"
        );
        assert_eq!(
            ProviderBoundaryProbe::evidence_state(&wrong_duty),
            ProviderCapabilityEvidence::Unknown
        );

        // Reviewer 窗口缺失:只读 action + 空窗口 → 拒签,Unknown。
        let ro_fixture = BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "git",
            SessionPolicyAction::PlanningReadOnly,
            base.path(),
            &base.path().join("evidence"),
            "unit-unknown-ro",
        )
        .expect("ro fixture");
        let no_window = ProviderBoundaryProbe::run_with_observation_budget(
            8192,
            &probe_projection(
                ProviderName::ClaudeCode,
                &ro_fixture,
                "git version 0.0.0-probe",
                AdapterRole::Reviewer,
            ),
            &ro_fixture,
            &HostOpenProbeScope::reviewer("", "aria-unit-probe-scope"),
        )
        .await;
        assert!(
            matches!(
                &no_window,
                Err(ProviderBoundaryError::InvalidPlan(reason))
                    if reason.starts_with("boundary_scope_missing")
            ),
            "missing review window must refuse to sign: {no_window:?}"
        );
        assert_eq!(
            ProviderBoundaryProbe::evidence_state(&no_window),
            ProviderCapabilityEvidence::Unknown
        );

        // 资源身份缺失:拒签,Unknown(审计/资源身份不可追溯不签)。
        let no_resource = ProviderBoundaryProbe::run_with_observation_budget(
            8192,
            &probe_projection(
                ProviderName::ClaudeCode,
                &ro_fixture,
                "git version 0.0.0-probe",
                AdapterRole::Reviewer,
            ),
            &ro_fixture,
            &HostOpenProbeScope::reviewer("aria-unit-review-window", " "),
        )
        .await;
        assert!(
            matches!(
                &no_resource,
                Err(ProviderBoundaryError::InvalidPlan(reason))
                    if reason.starts_with("boundary_scope_missing")
            ),
            "missing resource scope must refuse to sign: {no_resource:?}"
        );

        // 版本漂移:projection 冻结版本 ≠ 真实 CLI 版本 → 拒签,Unknown。
        let drifted = ProviderBoundaryProbe::run_with_observation_budget(
            8192,
            &probe_projection(
                ProviderName::ClaudeCode,
                &fixture,
                "git version 0.0.0-red-drift",
                AdapterRole::Executor,
            ),
            &fixture,
            &author_scope(),
        )
        .await;
        assert!(
            matches!(
                &drifted,
                Err(ProviderBoundaryError::ProbeFailed(reason))
                    if reason.starts_with("boundary_version_drift")
            ),
            "version drift must refuse to sign: {drifted:?}"
        );
        assert_eq!(
            ProviderBoundaryProbe::evidence_state(&drifted),
            ProviderCapabilityEvidence::Unknown
        );
    }

    /// fixture digest 稳定且覆盖全部材料维度(同材料同 digest,任一维度
    /// 变化即漂移)。
    #[test]
    fn boundary_fixture_digest_is_stable_and_covers_material() {
        let base = tempdir().expect("base dir");
        let fixture = BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "git",
            SessionPolicyAction::PlanningReadOnly,
            base.path(),
            &base.path().join("evidence"),
            "digest-stability",
        )
        .expect("fixture");
        assert_eq!(fixture.fixture_digest(), fixture.fixture_digest());
        let other = BoundaryFixture::create(
            ProviderName::Codex,
            "git",
            SessionPolicyAction::PlanningReadOnly,
            base.path(),
            &base.path().join("evidence"),
            "digest-stability",
        )
        .expect("other fixture");
        assert_ne!(fixture.fixture_digest(), other.fixture_digest());
        assert!(fixture.fixture_digest().starts_with("sha256:"));
    }

    /// 三态映射:防护失效(正向写真实失败/受控 commit 失败)→ Denied;
    /// 稳定码前缀(CLI 缺失/漂移/未观测/超预算/工件未落盘)→ Unknown。
    #[test]
    fn boundary_probe_state_mapping_denies_only_on_protection_failure() {
        let denied =
            ProviderBoundaryProbe::evidence_state(&Err(ProviderBoundaryError::ProbeFailed(
                "positive probe refused: channel=Mcp path=/lc/root/rogue".to_string(),
            )));
        assert_eq!(
            denied,
            ProviderCapabilityEvidence::Denied {
                reason: "positive probe refused: channel=Mcp path=/lc/root/rogue".to_string()
            }
        );
        let commit_denied =
            ProviderBoundaryProbe::evidence_state(&Err(ProviderBoundaryError::ProbeFailed(
                "controlled commit failed: exit Some(1)".to_string(),
            )));
        assert!(matches!(
            commit_denied,
            ProviderCapabilityEvidence::Denied { .. }
        ));
        for code in [
            "boundary_unavailable",
            "boundary_version_drift",
            "boundary_unobserved",
            "boundary_observation_over_budget",
            "boundary_artifact_unwritten",
        ] {
            let state = ProviderBoundaryProbe::evidence_state(&Err(
                ProviderBoundaryError::ProbeFailed(format!("{code}: detail")),
            ));
            assert_eq!(state, ProviderCapabilityEvidence::Unknown, "{code}");
        }
        let invalid_plan = ProviderBoundaryProbe::evidence_state(&Err(
            ProviderBoundaryError::InvalidPlan("fixture root mismatch".to_string()),
        ));
        assert_eq!(invalid_plan, ProviderCapabilityEvidence::Unknown);
    }

    // ==== Task 6c resume 面:真实 launch→同 id resume→错 id 负探针 ====

    /// 可编程假 CLI(宿主真实进程,claude print-json 形态):launch 打印
    /// 固定 session_id 的 JSON;`--resume <id>` 回显请求 id;错 id
    /// (00000000-*)exit 1 带真实报文。`broken` 变体的 resume 回显**另一个**
    /// id(模拟 id 绑定失效)。真实四家 CLI 由 live probe 覆盖。
    fn write_fake_resume_cli(dir: &Path, variant: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;

        let resume_body = if variant == "broken" {
            // id 绑定失效:resume 回显另一个 id。
            "printf '{\"session_id\":\"99999999-8888-7777-6666-555555555555\",\"result\":\"resumed\"}\\n'"
        } else {
            "printf '{\"session_id\":\"%s\",\"result\":\"resumed\"}\\n' \"$rid\""
        };
        let script = format!(
            "#!/bin/sh\n\
             if [ \"$1\" = \"--version\" ]; then echo \"fake-resume-cli 1.0.0\"; exit 0; fi\n\
             prev=\"\"; rid=\"\"\n\
             for a in \"$@\"; do\n\
             \x20 if [ \"$prev\" = \"--resume\" ]; then rid=\"$a\"; fi\n\
             \x20 prev=\"$a\"\n\
             done\n\
             if [ -n \"$rid\" ]; then\n\
             \x20 case \"$rid\" in\n\
             \x20 \x20 00000000-*) echo \"No conversation found with session ID: $rid\" >&2; exit 1;;\n\
             \x20 esac\n\
             \x20 {resume_body}\n\
             \x20 exit 0\n\
             fi\n\
             printf '{{\"session_id\":\"11111111-2222-3333-4444-555555555555\",\"result\":\"ok\"}}\\n'\n"
        );
        let path = dir.join(format!("fake-resume-cli-{variant}"));
        fs::write(&path, script).expect("write fake resume cli");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755))
            .expect("chmod fake resume cli");
        path
    }

    /// Task 6c resume 面:launch 记录 native id→同 id resume 确认同 id→
    /// 错 id 负探针被拒;工件 resume 段完整可审计,`artifact_resume_state`
    /// 签发 Confirmed,且 2c shape 对 resume=Confirmed 的行兼容。
    #[tokio::test]
    async fn lcg_t06_resume_probe_round_trips_native_session_id() {
        let base = tempdir().expect("base dir");
        let cli = write_fake_resume_cli(base.path(), "ok");
        let cli_text = cli.to_string_lossy().into_owned();
        let version = host_cli_version(&cli_text);
        let fixture = BoundaryFixture::create(
            ProviderName::ClaudeCode,
            &cli_text,
            SessionPolicyAction::CodingTargetWrite,
            base.path(),
            &base.path().join("evidence"),
            "unit-resume-ok",
        )
        .expect("fixture")
        .with_resume(ResumeProbeSpec::new(
            ResumeChannelKind::ClaudePrintJson,
            "Reply with exactly: resume-ok",
        ));
        let projection = probe_projection(
            ProviderName::ClaudeCode,
            &fixture,
            &version,
            AdapterRole::Executor,
        );
        let outcome = ProviderBoundaryProbe::run(&projection, &fixture, &author_scope()).await;
        assert_eq!(
            ProviderBoundaryProbe::evidence_state(&outcome),
            ProviderCapabilityEvidence::Confirmed,
            "write face must stay Confirmed independently of the resume face: {outcome:?}"
        );
        let evidence = outcome.expect("confirmed evidence");
        let artifact: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(evidence.artifact_ref()).expect("read artifact"),
        )
        .expect("parse artifact");
        let resume = &artifact["resume"];
        assert_eq!(resume["probed"], serde_json::json!(true));
        assert_eq!(resume["state"], serde_json::json!("Confirmed"));
        assert_eq!(
            resume["launch_native_session_id"],
            serde_json::json!("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(
            resume["resume_native_session_id"],
            serde_json::json!("11111111-2222-3333-4444-555555555555")
        );
        assert_eq!(resume["id_confirmed_same"], serde_json::json!(true));
        assert_eq!(resume["wrong_id_rejected"], serde_json::json!(true));
        assert_eq!(
            ProviderBoundaryProbe::artifact_resume_state(evidence.artifact_ref()),
            ProviderCapabilityEvidence::Confirmed
        );
        // 2c shape 校验对 resume=Confirmed 的行兼容(2d 导入 resume 格)。
        let record = shape_validation_record(
            &evidence,
            &projection,
            ProviderCapabilityEvidence::Confirmed,
        );
        ProviderCapabilityProbeService::new()
            .validate_probe_shape(&record, &evidence, &projection)
            .expect("2c shape validation accepts a resume-confirmed row");
    }

    // r25 接线回归:外部探针入口 `run_cli_boundary_probe`(harness seam)
    // 以原生参数执行完整 probe(含 resume 面)并返回 (evidence, projection),
    // 结果可经 2d `record_verified_probe` 导入 durable——矩阵 LC setup 段
    // capability 播种链端到端(probe→2c shape→2d import)。

    /// r50(#1 残根,coding fresh 现场):BoundaryFixture::boundary_action
    /// 把 ReviewReadOnly 折叠为 PlanningReadOnly(无 target 即 Planning),
    /// projection/evidence/2d 导入全落在 planning 行——review_read_only
    /// 行恒 Unknown,code review reviewer 的原生 resume 被
    /// require_resume_supported 拒(provider_gateway_resume_not_supported,
    /// r50 实测探针 evidence.action=planning_read_only)。修:fixture 记住
    /// 请求 action,只读两 action 原样透传。
    #[test]
    fn boundary_fixture_preserves_requested_review_read_only_action() {
        let base = tempdir().expect("base dir");
        let fixture = super::BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "claude",
            SessionPolicyAction::ReviewReadOnly,
            base.path(),
            &base.path().join("evidence"),
            "unit-review-action-fixture",
        )
        .expect("fixture");
        assert_eq!(
            fixture.boundary_action(),
            SessionPolicyAction::ReviewReadOnly,
            "ReviewReadOnly 探针请求不得折叠为 PlanningReadOnly(r50 现场:reviewer resume 行恒 Unknown)"
        );
        let planning = super::BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "claude",
            SessionPolicyAction::PlanningReadOnly,
            base.path(),
            &base.path().join("evidence"),
            "unit-planning-action-fixture",
        )
        .expect("fixture");
        assert_eq!(
            planning.boundary_action(),
            SessionPolicyAction::PlanningReadOnly
        );
        let coding = super::BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "claude",
            SessionPolicyAction::CodingTargetWrite,
            base.path(),
            &base.path().join("evidence"),
            "unit-coding-action-fixture",
        )
        .expect("fixture");
        assert_eq!(
            coding.boundary_action(),
            SessionPolicyAction::CodingTargetWrite
        );
    }
    #[tokio::test]
    async fn lcg_t06_external_cli_probe_entry_feeds_2d_import() {
        let base = tempdir().expect("base dir");
        let cli = write_fake_resume_cli(base.path(), "ok");
        let cli_text = cli.to_string_lossy().into_owned();
        let outcome = super::run_cli_boundary_probe(
            ProviderName::ClaudeCode,
            &cli_text,
            SessionPolicyAction::PlanningReadOnly,
            base.path(),
            &base.path().join("evidence"),
            "unit-external-entry",
            Some(ResumeProbeSpec::new(
                ResumeChannelKind::ClaudePrintJson,
                "Reply with exactly: resume-ok",
            )),
            reviewer_scope(),
        )
        .await
        .expect("external probe entry must run the full probe");
        let evidence = outcome.evidence;
        let projection = outcome.projection;
        assert_eq!(
            ProviderBoundaryProbe::evidence_state(&Ok(evidence.clone())),
            ProviderCapabilityEvidence::Confirmed
        );
        let resume_state = ProviderBoundaryProbe::artifact_resume_state(evidence.artifact_ref());
        assert_eq!(resume_state, ProviderCapabilityEvidence::Confirmed);
        // record.resume 格必须取工件签发结果(材料包构造同口径)。
        assert_eq!(outcome.record.action_matrix.rows()[0].resume, resume_state);
        // r48:launch 也签 Confirmed(探针真实启动 CLI;三家无 bootstrap
        // 记录时 2b 过渡桥不可达,launch=Unknown 会拒收——r2 现场首错)。
        assert_eq!(
            outcome.record.action_matrix.rows()[0].launch,
            ProviderCapabilityEvidence::Confirmed,
            "探针真实启动过 CLI,launch 必须签 Confirmed(r2 三家首错锚点)"
        );
        let record = outcome.record;
        let store = crate::product::logical_codebase::provider_capability_store::ProviderCapabilityStore::new(
            crate::product::app_paths::ProductAppPaths::new(base.path().join(".aria")),
        );
        ProviderCapabilityProbeService::with_durable_writer(store)
            .record_verified_probe(&record, &evidence, &projection)
            .expect("2d import from external entry evidence");
    }

    /// Task 6c resume 面:同 id resume 回显**另一个** native id(id 绑定
    /// 失效)→ resume 段 Denied(带真实应答摘录);写面证据不受影响。
    #[tokio::test]
    async fn lcg_t06_resume_probe_denies_when_cli_breaks_id_binding() {
        let base = tempdir().expect("base dir");
        let cli = write_fake_resume_cli(base.path(), "broken");
        let cli_text = cli.to_string_lossy().into_owned();
        let version = host_cli_version(&cli_text);
        let fixture = BoundaryFixture::create(
            ProviderName::ClaudeCode,
            &cli_text,
            SessionPolicyAction::CodingTargetWrite,
            base.path(),
            &base.path().join("evidence"),
            "unit-resume-broken",
        )
        .expect("fixture")
        .with_resume(ResumeProbeSpec::new(
            ResumeChannelKind::ClaudePrintJson,
            "Reply with exactly: resume-ok",
        ));
        let projection = probe_projection(
            ProviderName::ClaudeCode,
            &fixture,
            &version,
            AdapterRole::Executor,
        );
        let outcome = ProviderBoundaryProbe::run(&projection, &fixture, &author_scope()).await;
        assert_eq!(
            ProviderBoundaryProbe::evidence_state(&outcome),
            ProviderCapabilityEvidence::Confirmed,
            "write face is independent of the resume face: {outcome:?}"
        );
        let evidence = outcome.expect("confirmed evidence");
        let artifact: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(evidence.artifact_ref()).expect("read artifact"),
        )
        .expect("parse artifact");
        assert_eq!(artifact["resume"]["state"], serde_json::json!("Denied"));
        let denied = ProviderBoundaryProbe::artifact_resume_state(evidence.artifact_ref());
        match denied {
            ProviderCapabilityEvidence::Denied { reason } => {
                assert!(
                    reason.contains("resume native id mismatch"),
                    "deny reason must carry the real mismatch: {reason}"
                );
            }
            other => panic!("resume face must be Denied, got {other:?}"),
        }
    }

    /// Task 6c resume 面:规格缺省 = 未探测——工件 resume 段 `probed=false`,
    /// 三态 Unknown(launch/write 面不受影响,legacy 行为零变化)。
    #[tokio::test]
    async fn lcg_t06_resume_face_not_probed_without_spec() {
        let base = tempdir().expect("base dir");
        let version = host_cli_version("git");
        let fixture = BoundaryFixture::create(
            ProviderName::ClaudeCode,
            "git",
            SessionPolicyAction::PlanningReadOnly,
            base.path(),
            &base.path().join("evidence"),
            "unit-resume-absent",
        )
        .expect("fixture");
        let projection = probe_projection(
            ProviderName::ClaudeCode,
            &fixture,
            &version,
            AdapterRole::Reviewer,
        );
        let outcome = ProviderBoundaryProbe::run(&projection, &fixture, &reviewer_scope()).await;
        assert_eq!(
            ProviderBoundaryProbe::evidence_state(&outcome),
            ProviderCapabilityEvidence::Confirmed,
            "{outcome:?}"
        );
        let evidence = outcome.expect("confirmed evidence");
        let artifact: serde_json::Value = serde_json::from_str(
            &fs::read_to_string(evidence.artifact_ref()).expect("read artifact"),
        )
        .expect("parse artifact");
        assert_eq!(artifact["resume"]["probed"], serde_json::json!(false));
        assert_eq!(
            ProviderBoundaryProbe::artifact_resume_state(evidence.artifact_ref()),
            ProviderCapabilityEvidence::Unknown
        );
    }

    // ==== Task 6c 四家现场:真实 CLI boundary probe(LC_GATEWAY_E2E=1) ====

    /// 各家真实 CLI 的 resume 通道(现场实测口径,见 `ResumeChannelKind`)。
    fn live_resume_kind(provider: &ProviderName) -> ResumeChannelKind {
        match provider {
            ProviderName::ClaudeCode => ResumeChannelKind::ClaudePrintJson,
            ProviderName::Codex => ResumeChannelKind::CodexExecJson,
            ProviderName::Pi => ResumeChannelKind::PiSessionId,
            ProviderName::KimiCode => ResumeChannelKind::KimiStreamJson,
            ProviderName::Fake => unreachable!("fake has no real resume channel"),
        }
    }

    fn live_gate() -> bool {
        std::env::var("LC_GATEWAY_E2E").ok().as_deref() == Some("1")
    }

    fn live_evidence_root(family_dir: &str) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("cadence")
            .join("reports")
            .join("lc-gateway-multi-provider")
            .join(family_dir)
            .join("boundary")
    }

    /// 2c shape 校验用的对齐 record(write_boundary=Confirmed;resume 格由
    /// probe 工件签发,launch 未探测保持 Unknown;三方一致字段对齐)。
    /// r48:与 probe_import_record 同步——launch 签 Confirmed(探针真实
    /// 启动 CLI;三家无 bootstrap 记录时 2b 过渡桥不可达)。
    fn shape_validation_record(
        evidence: &crate::cross_cutting::provider_boundary::ProviderBoundaryEvidence,
        projection: &ProviderPolicyProjection,
        resume: ProviderCapabilityEvidence,
    ) -> ProviderCapabilityRecord {
        let action = projection.action();
        let row = ProviderActionCapability {
            action,
            launch: ProviderCapabilityEvidence::Confirmed,
            resume,
            write_boundary: ProviderCapabilityEvidence::Confirmed,
            projection_digest: evidence.projection_digest().to_string(),
            evidence_ref: evidence.artifact_ref().to_string(),
        };
        ProviderCapabilityRecord {
            provider_type: projection.provider_type(),
            schema_version: PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION,
            version: evidence.exact_version().to_string(),
            adapter_dialect: projection.provider_dialect(),
            wire_dialect: projection.wire_dialect(),
            capability_snapshot_ref: "lc_boundary_probe".to_string(),
            evidence: CapabilityEvidence::ProductionVerified,
            resume_evidence: ResumeEvidenceState::Unsupported,
            supported_actions: vec![action],
            action_matrix: ProviderActionMatrix::from_rows(vec![row])
                .expect("probe action rows"),
            trust: ProviderCapabilityEvidence::Unknown,
            probed_at: Some(evidence.probed_at().to_string()),
            probe_artifact_ref: Some(evidence.artifact_ref().to_string()),
            execution_semantics: PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
        }
    }

    /// 单家现场切片:Coding 与 read-only 两个 action 各一次真实 probe,证据
    /// 落该家 boundary/ 目录;2c shape 校验通过才标记可导入(2d 消费)。
    async fn run_live_probe(provider: ProviderName, cli: &str, family_dir: &str) {
        if !live_gate() {
            return;
        }
        let version = host_cli_version(cli);
        let evidence_root = live_evidence_root(family_dir);
        for action in [
            SessionPolicyAction::CodingTargetWrite,
            SessionPolicyAction::PlanningReadOnly,
        ] {
            let base = tempdir().expect("live base dir");
            let label = format!(
                "{}-{}",
                action_text(action),
                chrono::Utc::now().format("%Y%m%dT%H%M%SZ")
            );
            let fixture = BoundaryFixture::create(
                provider.clone(),
                cli,
                action,
                base.path(),
                &evidence_root,
                &label,
            )
            .expect("live fixture")
            .with_resume(ResumeProbeSpec::new(
                live_resume_kind(&provider),
                "Reply with exactly: boundary-resume-probe",
            ));
            let role = if action == SessionPolicyAction::CodingTargetWrite {
                AdapterRole::Executor
            } else {
                AdapterRole::Reviewer
            };
            let projection = probe_projection(provider.clone(), &fixture, &version, role);
            let scope = if action == SessionPolicyAction::CodingTargetWrite {
                author_scope()
            } else {
                reviewer_scope()
            };
            let outcome = ProviderBoundaryProbe::run(&projection, &fixture, &scope).await;
            let state = ProviderBoundaryProbe::evidence_state(&outcome);
            assert_eq!(
                state,
                ProviderCapabilityEvidence::Confirmed,
                "live {family_dir} {label} probe must sign Confirmed: {outcome:?}"
            );
            let evidence = outcome.expect("confirmed live evidence");
            // resume 面:真实 launch→同 id resume→错 id 负探针(工件签发)。
            let resume_state =
                ProviderBoundaryProbe::artifact_resume_state(evidence.artifact_ref());
            assert_eq!(
                resume_state,
                ProviderCapabilityEvidence::Confirmed,
                "live {family_dir} {label} resume face must be Confirmed"
            );
            let record = shape_validation_record(&evidence, &projection, resume_state);
            ProviderCapabilityProbeService::new()
                .validate_probe_shape(&record, &evidence, &projection)
                .unwrap_or_else(|error| panic!("2c shape validation: {error:?}"));
            let session_dir = fixture.evidence_session_dir();
            fs::write(
                session_dir.join("shape-validation.json"),
                serde_json::json!({
                    "schema": "lc-boundary-probe-shape-validation-v1",
                    "provider": family_dir,
                    "action": action_text(action),
                    "shape_validation": "passed",
                    "importable_by": "task-2d record_verified_probe",
                    "checked_at": chrono::Utc::now().to_rfc3339(),
                    "evidence_ref": evidence.artifact_ref(),
                })
                .to_string(),
            )
            .expect("write shape validation");
            let artifact: serde_json::Value = serde_json::from_str(
                &fs::read_to_string(evidence.artifact_ref()).expect("read live artifact"),
            )
            .expect("parse live artifact");
            let positives = artifact["positives"].as_array().expect("positives");
            println!(
                "EVIDENCE live-boundary-probe family={family_dir} action={} version={version} \
                 state=Confirmed positives={} artifact={}",
                action_text(action),
                positives.len(),
                evidence.artifact_ref(),
            );
        }
    }

    #[tokio::test]
    #[ignore = "set LC_GATEWAY_E2E=1 to run the real CLI write-boundary probe"]
    async fn lcg_t06_live_boundary_probe_claude_code() {
        run_live_probe(ProviderName::ClaudeCode, "claude", "claude-code").await;
    }

    #[tokio::test]
    #[ignore = "set LC_GATEWAY_E2E=1 to run the real CLI write-boundary probe"]
    async fn lcg_t06_live_boundary_probe_codex() {
        run_live_probe(ProviderName::Codex, "codex", "codex").await;
    }

    #[tokio::test]
    #[ignore = "set LC_GATEWAY_E2E=1 to run the real CLI write-boundary probe"]
    async fn lcg_t06_live_boundary_probe_pi() {
        run_live_probe(ProviderName::Pi, "pi", "pi").await;
    }

    #[tokio::test]
    #[ignore = "set LC_GATEWAY_E2E=1 to run the real CLI write-boundary probe"]
    async fn lcg_t06_live_boundary_probe_kimi_code() {
        run_live_probe(ProviderName::KimiCode, "kimi", "kimi-code").await;
    }
}
