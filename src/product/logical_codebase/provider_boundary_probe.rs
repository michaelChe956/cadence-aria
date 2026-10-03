//! LC provider 写边界 probe 与证据签发(Task 6c,REQ-LCG-03/07)。
//!
//! `ProviderBoundaryProbe::run` 是受控隔离 fixture 的**探测通道**(冻结签名,
//! 计划 Task 6 Interfaces):对某个真实 provider 的 projection,在产品拥有的
//! 写边界沙箱(6a launcher)内执行**真实正负写探针**,只有「完整防护在场 +
//! 正向探针真实成功落盘 + 负向探针全部被拒且带证据 + 受保护面 pre==post 零
//! 漂移 + 观测完整且未超预算」才签发 `ProviderBoundaryEvidence`(2c shape
//! 校验通过后由 2d 导入 Confirmed)。
//!
//! 语义红线(计划 Global Constraints/Task 6 Step 3/4):
//! - probe 不是用户 normal action 的 Unknown 旁路;`write_boundary=Confirmed`
//!   只能由本 probe 的完整防护+真实正负探针签发。
//! - 缺 bwrap/user namespace、CLI 版本漂移、观测缺失(哨兵/FIFO 缺失)、
//!   观测超预算(沿 HEAD 既有预算:snapshot `max_entries=20_000`/
//!   `max_bytes=64 MiB`、inventory soft4096/hard8192B,不新立预算)、证据
//!   工件未落盘——一律**不签 Confirmed**,状态保持 Unknown;负向写真实落盘
//!   或受保护面漂移才是 Denied。FIFO/观测缺失/超限**不截断冒充通过**。
//! - evidence 工件(快照/日志)写盘不入 git;`artifact_ref` 必须指向真实
//!   存在的工件文件。

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

use crate::cross_cutting::provider_boundary::{
    ProviderBoundaryError, ProviderBoundaryEvidence, ProviderBoundaryMode, ProviderBoundaryPlan,
};
use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::logical_codebase::provider_projection::ProviderPolicyProjection;
use crate::product::models::ProviderName;

/// evidence 工件的 schema 标记(字段序变化必须换 schema 版本)。
pub const BOUNDARY_PROBE_EVIDENCE_SCHEMA: &str = "lc-boundary-probe-evidence-v1";

/// 6a 写面探针通道对「哨兵缺失(写未被观测)」产出的证据标记(见
/// `provider_boundary.rs::run_probe_script` 的
/// `"probe sentinel missing (write unobserved)"`);6c 分类器据此把未观测
/// 拒绝与真实 errno 拒绝区分开——前者永不计入支持面。
const UNOBSERVED_MARKER: &str = "probe sentinel missing";

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

/// provider 的自然 session/runtime 可写窄面目录(`PROVIDER_RUNTIME_WRITABLE_DIRS`
/// 中归属该 provider 的第一项;fixture 与正向探针共用同一窄面)。
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
        let member = base.join("member-a");
        let member_b = base.join("member-b");
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

    /// fixture 材料对应的 canonical action(target 在场即 Coding)。
    pub fn boundary_action(&self) -> SessionPolicyAction {
        if self.target.is_some() {
            SessionPolicyAction::CodingTargetWrite
        } else {
            SessionPolicyAction::PlanningReadOnly
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
    /// 全局 git 配置。
    pub(crate) fn probe_env(&self) -> BTreeMap<String, String> {
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
    /// 冻结签名:受控隔离 fixture 的探测通道。完整防护(6a launcher 实测
    /// 可用)+ 真实正负探针全部通过才签发 evidence;任一环节缺位返回
    /// 稳定码错误(见 [`Self::evidence_state`] 的三态映射)。
    pub async fn run(
        projection: &ProviderPolicyProjection,
        fixture: &BoundaryFixture,
    ) -> Result<ProviderBoundaryEvidence, ProviderBoundaryError> {
        // Task 6c 阶段 1 RED 桩:阶段 2 实现真实探测与签发。
        let _ = (projection, fixture);
        Err(ProviderBoundaryError::Unsupported(
            "task 6c red stub: boundary probe run is not implemented yet".to_string(),
        ))
    }

    /// 注入式 seam(launcher/观测预算可注入;测试与装配用)。生产 `run`
    /// 以 `ProviderBoundaryLauncher::probe_environment()` 与沿 HEAD 的
    /// inventory 硬预算(8192B)调用本函数。
    pub(crate) async fn run_with_observation_budget(
        _launcher: &crate::cross_cutting::provider_boundary::ProviderBoundaryLauncher,
        _observation_budget: usize,
        _projection: &ProviderPolicyProjection,
        _fixture: &BoundaryFixture,
    ) -> Result<ProviderBoundaryEvidence, ProviderBoundaryError> {
        // Task 6c 阶段 1 RED 桩:阶段 2 实现真实探测与签发。
        Err(ProviderBoundaryError::Unsupported(
            "task 6c red stub: boundary probe run is not implemented yet".to_string(),
        ))
    }

    /// 单次写探针是否**被观测**:被拒且带真实证据(errno/shell 报文)。
    /// 哨兵缺失(6a 通道以 `probe sentinel missing` 标记)与空证据的「拒绝」
    /// 都是未观测——不得计入支持面,更不得据此签 Confirmed。
    pub(crate) fn attempt_observed(_refused: bool, _evidence: &str) -> bool {
        // Task 6c 阶段 1 RED 桩:阶段 2 实现真实观测判定。
        true
    }

    /// capability 三态视角(Confirmed 只能来自 `run` 的完整防护+真实正负
    /// 探针签发):Ok=Confirmed;缺防护/材料不一致/版本漂移/观测缺失/超
    /// 预算/工件未落盘=Unknown;防护失效(负向落盘/正向失败/漂移)=Denied。
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
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    use tempfile::tempdir;

    use super::{BoundaryFixture, ProviderBoundaryProbe};
    use crate::cross_cutting::claude_code_provider::ClaudePolicyProjector;
    use crate::cross_cutting::codex_provider::CodexPolicyProjector;
    use crate::cross_cutting::kimi_code_provider::KimiPolicyProjector;
    use crate::cross_cutting::pi_provider::PiPolicyProjector;
    use crate::cross_cutting::provider_boundary::{
        ProviderBoundaryError, ProviderBoundaryLauncher, ProviderBoundaryMode, ProviderBoundaryPlan,
    };
    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
    use crate::cross_cutting::streaming_provider::ProviderPermissionMode;
    use crate::product::logical_codebase::policy::{
        PolicyTarget, ProviderDialect, SessionPolicyAction, SessionPolicyEnvelope,
    };
    use crate::product::logical_codebase::provider_capability_probe::ProviderCapabilityProbeService;
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence, PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION, ProviderActionCapability,
        ProviderActionMatrix, ProviderCapabilityRecord,
    };
    use crate::product::logical_codebase::provider_gateway::{ProviderRef, ResumeEvidenceState};
    use crate::product::logical_codebase::provider_projection::{
        ProviderPolicyProjection, ProviderPolicyProjector, ProviderProjectionInput,
    };
    use crate::product::models::ProviderName;
    use crate::protocol::contracts::AdapterRole;

    use super::action_text;

    /// 以被测 provider 的真实 projector 构造 probe 用 projection(fixture
    /// 材料与 projection 的 cwd/target/action 完全一致;envelope 直构)。
    fn probe_projection(
        provider: ProviderName,
        fixture: &BoundaryFixture,
        exact_version: &str,
        role: AdapterRole,
    ) -> ProviderPolicyProjection {
        let action = fixture.boundary_action();
        let (dialect, mcp_source) = match &provider {
            ProviderName::ClaudeCode => (ProviderDialect::ClaudeCodeCliV1, ""),
            ProviderName::Codex => (ProviderDialect::CodexCliV1, ""),
            ProviderName::Pi => (ProviderDialect::PiRpcV1, ""),
            // kimi 投影要求 MCP 来源非空:无 Aria 注入时用 native 标记
            // (与 `KIMI_NATIVE_MCP_SOURCE` 冻结同值)。
            ProviderName::KimiCode => (ProviderDialect::KimiAcpV1, "native-project-config"),
            ProviderName::Fake => unreachable!("fake has no real projector"),
        };
        let target_worktree = fixture
            .target()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| fixture.root().to_path_buf());
        let writable_roots = match action {
            SessionPolicyAction::CodingTargetWrite => vec![target_worktree.clone()],
            SessionPolicyAction::PlanningReadOnly | SessionPolicyAction::ReviewReadOnly => {
                Vec::new()
            }
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

    /// Task 6c Step 1(断言 355 逐字):未观测/超预算的 evidence 永不
    /// Confirmed——哨兵缺失的「拒绝」、空证据拒绝、以及观测面超出沿 HEAD
    /// 预算的真实 probe 运行都必须保持 Unknown(不截断、不冒充)。
    #[tokio::test]
    async fn lcg_t06_unobserved_or_over_budget_evidence_never_confirms() {
        // 1) 分类器:6a 通道的哨兵缺失标记与空证据都是未观测。
        assert!(
            !ProviderBoundaryProbe::attempt_observed(
                true,
                "sh: 1: probe sentinel missing (write unobserved)"
            ),
            "sentinel-missing refusal is unobserved"
        );
        assert!(
            !ProviderBoundaryProbe::attempt_observed(true, "  "),
            "empty-evidence refusal is unobserved"
        );
        assert!(
            ProviderBoundaryProbe::attempt_observed(
                true,
                "sh: 1: cannot create /lc/root/rogue: Read-only file system"
            ),
            "refusal with real errno evidence is observed"
        );

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

        // 3) 真实沙箱运行 + 观测面超出预算(注入 16B 预算):拒签且保持
        //    Unknown,错误带稳定码,不截断冒充。
        let launcher = ProviderBoundaryLauncher::probe_environment();
        assert!(
            launcher.is_available(),
            "environment blocked: over-budget probe case needs bwrap + user namespace"
        );
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
            &launcher,
            16,
            &projection,
            &fixture,
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

    /// Task 6c:完整防护 + 真实正负探针才签 Confirmed——Ok evidence 的每个
    /// 字段锚定 projection/provider/版本/模式/工件,Coding 与 read-only 两种
    /// action 各签一份;证据工件含正负探针、D4 零漂移与 actual argv/mount。
    #[tokio::test]
    async fn lcg_t06_probe_signs_confirmed_only_after_full_protection_and_real_probes() {
        let launcher = ProviderBoundaryLauncher::probe_environment();
        assert!(
            launcher.is_available(),
            "environment blocked: full signing case needs bwrap + user namespace"
        );
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
        let outcome = ProviderBoundaryProbe::run(&projection, &fixture).await;
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
        assert!(!evidence.artifact_ref().is_empty());
        assert!(!evidence.probed_at().is_empty());
        let artifact = PathBuf::from(evidence.artifact_ref());
        assert!(artifact.is_file(), "artifact must exist on disk");
        let content = fs::read_to_string(&artifact).expect("read artifact");
        assert!(content.contains("\"positives\""));
        assert!(content.contains("\"negatives\""));
        assert!(content.contains("\"d4\""));
        assert!(content.contains("\"launcher_argv\""));
        assert!(content.contains("\"cli_exact_version\""));

        // read-only action 同样签发(全负向 + runtime 窄面正向)。
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
        let ro_outcome = ProviderBoundaryProbe::run(&ro_projection, &ro_fixture).await;
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
    }

    /// Task 6c:缺 sandbox(Coding 口径 Unknown 非 skip→pass 非 Denied)与
    /// CLI 版本漂移(projection 与真实 CLI 不一致)都不签 evidence,状态
    /// 保持 Unknown 且错误带稳定码。
    #[tokio::test]
    async fn lcg_t06_probe_keeps_unknown_without_sandbox_or_version_drift() {
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

        // 缺 bwrap/user namespace:失败关闭,Unknown。
        let unavailable = ProviderBoundaryProbe::run_with_observation_budget(
            &ProviderBoundaryLauncher::from_bwrap(None),
            8192,
            &probe_projection(
                ProviderName::ClaudeCode,
                &fixture,
                "git version 0.0.0-probe",
                AdapterRole::Executor,
            ),
            &fixture,
        )
        .await;
        assert!(matches!(
            &unavailable,
            Err(ProviderBoundaryError::Unsupported(reason))
                if reason.starts_with("boundary_unavailable")
        ));
        assert_eq!(
            ProviderBoundaryProbe::evidence_state(&unavailable),
            ProviderCapabilityEvidence::Unknown
        );

        // 版本漂移:projection 冻结版本 ≠ 真实 CLI 版本 → 拒签,Unknown。
        let launcher = ProviderBoundaryLauncher::probe_environment();
        assert!(
            launcher.is_available(),
            "environment blocked: version drift case needs bwrap + user namespace"
        );
        let drifted = ProviderBoundaryProbe::run_with_observation_budget(
            &launcher,
            8192,
            &probe_projection(
                ProviderName::ClaudeCode,
                &fixture,
                "git version 0.0.0-red-drift",
                AdapterRole::Executor,
            ),
            &fixture,
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

    /// 三态映射:防护失效(负向落盘/正向失败/受保护面漂移)→ Denied;
    /// 稳定码前缀(缺防护/漂移/未观测/超预算/工件未落盘)→ Unknown。
    #[test]
    fn boundary_probe_state_mapping_denies_only_on_protection_failure() {
        let denied =
            ProviderBoundaryProbe::evidence_state(&Err(ProviderBoundaryError::ProbeFailed(
                "negative probe landed: channel=Builtin path=/lc/root/.aria/rogue".to_string(),
            )));
        assert_eq!(
            denied,
            ProviderCapabilityEvidence::Denied {
                reason: "negative probe landed: channel=Builtin path=/lc/root/.aria/rogue"
                    .to_string()
            }
        );
        let drift_denied =
            ProviderBoundaryProbe::evidence_state(&Err(ProviderBoundaryError::ProbeFailed(
                "protected-face drift after probes (pre != post)".to_string(),
            )));
        assert!(matches!(
            drift_denied,
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

    // ==== Task 6c 四家现场:真实 CLI boundary probe(LC_GATEWAY_E2E=1) ====

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

    /// 2c shape 校验用的对齐 record(只签 write_boundary=Confirmed;launch/
    /// resume 未探测保持 Unknown;artifact/probed_at/version 三方一致)。
    fn shape_validation_record(
        evidence: &crate::cross_cutting::provider_boundary::ProviderBoundaryEvidence,
        projection: &ProviderPolicyProjection,
    ) -> ProviderCapabilityRecord {
        let action = projection.action();
        let row = ProviderActionCapability {
            action,
            launch: ProviderCapabilityEvidence::Unknown,
            resume: ProviderCapabilityEvidence::Unknown,
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
            root_recipe_evidence:
                crate::product::logical_codebase::provider_capability_store::RootRecipeEvidence::None,
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
            .expect("live fixture");
            let role = if action == SessionPolicyAction::CodingTargetWrite {
                AdapterRole::Executor
            } else {
                AdapterRole::Reviewer
            };
            let projection = probe_projection(provider.clone(), &fixture, &version, role);
            let outcome = ProviderBoundaryProbe::run(&projection, &fixture).await;
            let state = ProviderBoundaryProbe::evidence_state(&outcome);
            assert_eq!(
                state,
                ProviderCapabilityEvidence::Confirmed,
                "live {family_dir} {label} probe must sign Confirmed: {outcome:?}"
            );
            let evidence = outcome.expect("confirmed live evidence");
            let record = shape_validation_record(&evidence, &projection);
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
            let negatives = artifact["negatives"].as_array().expect("negatives");
            println!(
                "EVIDENCE live-boundary-probe family={family_dir} action={} version={version} \
                 state=Confirmed positives={} negatives={} artifact={}",
                action_text(action),
                positives.len(),
                negatives.len(),
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
