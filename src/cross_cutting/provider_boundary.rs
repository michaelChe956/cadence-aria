//! Provider 写边界(write boundary)的纯 DTO owner(Task 1a,REQ-LCG-03/07;
//! T10 旧物理 launcher 退役)。
//!
//! `ProviderBoundaryPlan` 是 validated launch 持有的不可伪造边界意图:
//! 进程 cwd(canonical LC root)、唯一可写 target(Coding;read-only action
//! 为 `None`)与受保护根(`.git`/`.aria` 等元数据位置)。`ProviderBoundary
//! Evidence` 是真实 probe(6c)产出并经三方一致性校验(2d)后导入 durable
//! capability 的证据记录。本文件只拥有 DTO 形状与 6c 宿主全开探针的
//! 通道原语;provider 进程统一受信 Host 启动(T10 起
//! `ProcessManager::spawn`,env_clear 白名单环境),bwrap launcher/
//! probe_bwrap/mount helper 已随旧物理隔离档退役;canonical cwd/target/
//! gitdir/worktree 复验由 gateway `revalidate_before_spawn` 与
//! `canonical_target_git_identity` 持有。
//!
//! 字段私有+`pub(crate)` 构造:plan/evidence 只能由 gateway/probe 装配,
//! 调用方不能自造;读取经 pub getter(只读投影)。

use std::path::{Path, PathBuf};

use crate::product::models::ProviderName;

/// 边界模式:read-only action 无任何可写 target;Coding 恰一个 canonical
/// target 可写(target-only)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderBoundaryMode {
    /// Planning/Review:root、成员与元数据均不可写。
    ReadOnly,
    /// Coding:唯一 writable root 恰为 target worktree。
    TargetWriteOnly,
}

/// validated launch 持有的不可伪造边界计划(Task 1a 冻结形状)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBoundaryPlan {
    mode: ProviderBoundaryMode,
    /// 进程 cwd(canonical `provider_context_root`)。target 独立解析,
    /// 不得替代 cwd(Global Constraints)。
    working_directory: PathBuf,
    /// 唯一可写 target(Coding);read-only action 为 `None`。
    target_root: Option<PathBuf>,
    /// 受保护根/路径(`.git`、`.aria` 等元数据位置;root 与非 target 的
    /// 保护由 launcher/负向探针执行)。
    protected_roots: Vec<PathBuf>,
}

impl ProviderBoundaryPlan {
    /// crate 内构造(gateway/probe 装配;调用方不能自造)。
    pub(crate) fn new(
        mode: ProviderBoundaryMode,
        working_directory: PathBuf,
        target_root: Option<PathBuf>,
        protected_roots: Vec<PathBuf>,
    ) -> Self {
        Self {
            mode,
            working_directory,
            target_root,
            protected_roots,
        }
    }

    pub fn mode(&self) -> ProviderBoundaryMode {
        self.mode
    }

    pub fn working_directory(&self) -> &Path {
        &self.working_directory
    }

    pub fn target_root(&self) -> Option<&Path> {
        self.target_root.as_deref()
    }

    pub fn protected_roots(&self) -> &[PathBuf] {
        &self.protected_roots
    }
}

/// 批2 Task 9 冻结:capability 证据的**执行语义标识**(单一来源;capability
/// store 行、probe 证据与 2d 导入三方共用,T10/T12 交接 digest)。
///
/// `host-open-v1` = 宿主全开执行语义:provider 以宿主身份直接执行
/// (`ProviderExecutionProfile::HostCoding`/HostLocal 部署),受控宿主写成功
/// 即「全开」事实,不再要求 root/非 target/`.git`/`.aria` 物理拒写。旧
/// 物理拒写语义(v1/v2 记录)没有本标识,读取时当前行 Unknown,必须真实
/// 重探针铸新证。
pub const PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN: &str = "host-open-v1";

/// 真实 boundary probe 的证据记录(Task 1a 冻结形状;6c 产出、2d 导入;
/// 批2 Task 9 增补执行语义/岗位/窗口维度)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderBoundaryEvidence {
    provider: ProviderName,
    /// 被探测 provider 的 exact CLI version(与 capability record 比对)。
    exact_version: String,
    boundary_mode: ProviderBoundaryMode,
    /// 当次 probe 比对的 projection digest(与 session projection、
    /// capability action row 三方一致才可导入 Confirmed)。
    projection_digest: String,
    /// probe 工件引用(快照/日志的 durable 引用)。
    artifact_ref: String,
    probed_at: String,
    /// 执行语义标识(批2 Task 9;必须等于
    /// `PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN`,缺语义证据不可导入)。
    execution_semantics: String,
    /// 受信岗位(author/reviewer;受信注入,probe 不采信模型自报)。
    duty: String,
    /// Reviewer 窗口引用(reviewer 岗位必带;author 为 None)。
    review_window_id: Option<String>,
}

impl ProviderBoundaryEvidence {
    /// crate 内构造(6c probe 产出;调用方不能自造)。批2 Task 9 增补
    /// 执行语义/岗位/窗口三参数(缺语义/缺岗位由 probe 侧拒签,构造器
    /// 只承载字段)。
    pub(crate) fn new(
        provider: ProviderName,
        exact_version: String,
        boundary_mode: ProviderBoundaryMode,
        projection_digest: String,
        artifact_ref: String,
        probed_at: String,
        execution_semantics: String,
        duty: String,
        review_window_id: Option<String>,
    ) -> Self {
        Self {
            provider,
            exact_version,
            boundary_mode,
            projection_digest,
            artifact_ref,
            probed_at,
            execution_semantics,
            duty,
            review_window_id,
        }
    }

    pub fn provider(&self) -> &ProviderName {
        &self.provider
    }

    pub fn exact_version(&self) -> &str {
        &self.exact_version
    }

    pub fn boundary_mode(&self) -> ProviderBoundaryMode {
        self.boundary_mode
    }

    pub fn projection_digest(&self) -> &str {
        &self.projection_digest
    }

    pub fn artifact_ref(&self) -> &str {
        &self.artifact_ref
    }

    pub fn probed_at(&self) -> &str {
        &self.probed_at
    }

    /// 执行语义标识(批2 Task 9;导入链与 `host-open-v1` 单一来源比对)。
    pub fn execution_semantics(&self) -> &str {
        &self.execution_semantics
    }

    /// 受信岗位(author/reviewer)。
    pub fn duty(&self) -> &str {
        &self.duty
    }

    /// Reviewer 窗口引用(reviewer 岗位必带)。
    pub fn review_window_id(&self) -> Option<&str> {
        self.review_window_id.as_deref()
    }
}

/// 边界执行/校验错误(稳定判别码 + 上下文)。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProviderBoundaryError {
    /// 当前 OS/运行时不支持可控写边界,不得退回无隔离 spawn。
    #[error("provider_boundary_unsupported: {0}")]
    Unsupported(String),
    /// plan 非法(cwd/target/protected roots 缺失或不一致)。
    #[error("provider_boundary_invalid_plan: {0}")]
    InvalidPlan(String),
    /// 真实 probe 失败(越界写成功/保护位置可写等)。
    #[error("provider_boundary_probe_failed: {0}")]
    ProbeFailed(String),
}

// ============================================================================
// Task 6c:宿主全开写面探针的通道原语(受控 fixture 的探测通道;probe 与
// 验收复用。T10:旧 bwrap 沙箱探针通道已退役,探针以宿主身份直接执行)
// ============================================================================

/// 写探针通道:覆盖 provider 进程自身与其后代写面(terminal/MCP/extension
/// 均为 provider 派生的子孙进程,天然落在同一 mount namespace 内)。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BoundaryWriteChannel {
    /// provider 进程自身直接写(内建写工具面)。
    Builtin,
    /// terminal 子进程写(provider 起 shell 的通道)。
    Terminal,
    /// MCP 后代写(MCP server 进程链;不可隔离的外部 MCP 写通道同样只
    /// 命中只读挂载面而被阻断)。
    Mcp,
    /// extension 子进程写(如 Pi extension)。
    Extension,
    /// child 子进程写(Task 11 第五通道:经 `setsid` 脱离会话组的再派生
    /// 子进程尝试写——验证重挂/脱组的后代仍共享同一 mount namespace,
    /// 越界写被只读挂载拒绝)。创建失败(setsid 不可用)由探针证据原文
    /// 记录,不算写拒绝。
    Child,
}

/// 计划内写探针(通道 + 目标文件)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedBoundaryWrite {
    channel: BoundaryWriteChannel,
    path: PathBuf,
}

impl PlannedBoundaryWrite {
    pub fn new(channel: BoundaryWriteChannel, path: PathBuf) -> Self {
        Self { channel, path }
    }

    pub fn channel(&self) -> BoundaryWriteChannel {
        self.channel
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// 单次写探针结果:拒绝必须带证据(errno/shell 报文),未观测到证据的
/// 「拒绝」不可计入支持面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundaryWriteAttempt {
    channel: BoundaryWriteChannel,
    path: PathBuf,
    refused: bool,
    evidence: String,
}

impl BoundaryWriteAttempt {
    pub(crate) fn allowed(channel: BoundaryWriteChannel, path: PathBuf) -> Self {
        Self {
            channel,
            path,
            refused: false,
            evidence: String::new(),
        }
    }

    pub(crate) fn refused_with(
        channel: BoundaryWriteChannel,
        path: PathBuf,
        evidence: String,
    ) -> Self {
        Self {
            channel,
            path,
            refused: true,
            evidence,
        }
    }

    /// 拒绝且带证据(可审计):`was_refused_with_evidence` 是写边界验收的
    /// 唯一「拒绝」口径。
    pub fn was_refused_with_evidence(&self) -> bool {
        self.refused && !self.evidence.trim().is_empty()
    }

    /// 写结果:Ok(())= 写成功并真实落盘;Err(evidence)= 被拒并带证据。
    pub fn result(&self) -> Result<(), String> {
        if self.refused {
            Err(self.evidence.clone())
        } else {
            Ok(())
        }
    }

    pub fn channel(&self) -> BoundaryWriteChannel {
        self.channel
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn evidence(&self) -> &str {
        &self.evidence
    }
}

/// shell 单引号转义(探针路径/脚本片段嵌入用)。
pub(crate) fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

/// 生成指定通道的一次写命令(嵌入探针脚本;路径/脚本经单引号转义)。
pub(crate) fn probe_write_command(channel: BoundaryWriteChannel, quoted_path: &str) -> String {
    let builtin = r#"printf aria-boundary-probe >"$1""#;
    match channel {
        BoundaryWriteChannel::Builtin => {
            format!("printf aria-boundary-probe >{quoted_path}")
        }
        BoundaryWriteChannel::Terminal => {
            format!("sh -c {} aria-terminal {quoted_path}", shell_quote(builtin))
        }
        BoundaryWriteChannel::Extension => format!(
            "sh -c {} aria-extension {quoted_path}",
            shell_quote(builtin)
        ),
        // MCP 通道 = 两层后代:外层 shell 再起内层 MCP server 代写。
        BoundaryWriteChannel::Mcp => {
            let inner = format!("sh -c {} aria-mcp-inner \"$1\"", shell_quote(builtin));
            format!("sh -c {} aria-mcp {quoted_path}", shell_quote(&inner))
        }
        // child 通道:setsid 在非进程组长时直接 exec(退出码透传),内层
        // shell 成为新会话(脱离终端/进程组)的再派生子进程。
        BoundaryWriteChannel::Child => format!(
            "setsid sh -c {} aria-child {quoted_path}",
            shell_quote(builtin)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// DTO 形状锁:构造字段与 getter 往返一致(read-only 无 target)。
    #[test]
    fn provider_boundary_plan_round_trips_read_only_shape() {
        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::ReadOnly,
            PathBuf::from("/lc-root"),
            None,
            vec![
                PathBuf::from("/lc-root/.git"),
                PathBuf::from("/lc-root/.aria"),
            ],
        );
        assert_eq!(plan.mode(), ProviderBoundaryMode::ReadOnly);
        assert_eq!(plan.working_directory(), std::path::Path::new("/lc-root"));
        assert_eq!(plan.target_root(), None);
        assert_eq!(plan.protected_roots().len(), 2);
    }

    /// target-only 形状:唯一 target 可写,cwd 与 target 分离。
    #[test]
    fn provider_boundary_plan_round_trips_target_write_only_shape() {
        let plan = ProviderBoundaryPlan::new(
            ProviderBoundaryMode::TargetWriteOnly,
            PathBuf::from("/lc-root"),
            Some(PathBuf::from("/work/api/.worktrees/issue_1")),
            vec![],
        );
        assert_eq!(plan.mode(), ProviderBoundaryMode::TargetWriteOnly);
        assert_eq!(
            plan.target_root(),
            Some(std::path::Path::new("/work/api/.worktrees/issue_1"))
        );
    }

    /// evidence getter 往返(2d 三方一致性校验消费的固定字段)。
    #[test]
    fn provider_boundary_evidence_exposes_probe_fields() {
        let evidence = ProviderBoundaryEvidence::new(
            ProviderName::ClaudeCode,
            "2.1.0".to_string(),
            ProviderBoundaryMode::TargetWriteOnly,
            "sha256:abc".to_string(),
            "probe://run/1".to_string(),
            "2026-10-03T00:00:00Z".to_string(),
            PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN.to_string(),
            "author".to_string(),
            None,
        );
        assert_eq!(evidence.provider(), &ProviderName::ClaudeCode);
        assert_eq!(evidence.exact_version(), "2.1.0");
        assert_eq!(
            evidence.boundary_mode(),
            ProviderBoundaryMode::TargetWriteOnly
        );
        assert_eq!(evidence.projection_digest(), "sha256:abc");
        assert_eq!(evidence.artifact_ref(), "probe://run/1");
        assert_eq!(evidence.probed_at(), "2026-10-03T00:00:00Z");
        assert_eq!(
            evidence.execution_semantics(),
            PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN
        );
        assert_eq!(evidence.duty(), "author");
        assert_eq!(evidence.review_window_id(), None);
    }
}
