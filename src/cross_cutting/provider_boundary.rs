//! Provider 写边界(write boundary)的纯 DTO owner(Task 1a,REQ-LCG-03/07)。
//!
//! `ProviderBoundaryPlan` 是 validated launch 持有的不可伪造边界意图:
//! 进程 cwd(canonical LC root)、唯一可写 target(Coding;read-only action
//! 为 `None`)与受保护根(`.git`/`.aria` 等元数据位置)。`ProviderBoundary
//! Evidence` 是真实 probe(6c)产出并经三方一致性校验(2d)后导入 durable
//! capability 的证据记录。本文件只拥有 DTO 形状;`ProcessManager::
//! spawn_with_boundary` 等 launcher 行为归 Task 6a,probe 归 Task 6c。
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

/// 真实 boundary probe 的证据记录(Task 1a 冻结形状;6c 产出、2d 导入)。
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
}

impl ProviderBoundaryEvidence {
    /// crate 内构造(6c probe 产出;调用方不能自造)。
    pub(crate) fn new(
        provider: ProviderName,
        exact_version: String,
        boundary_mode: ProviderBoundaryMode,
        projection_digest: String,
        artifact_ref: String,
        probed_at: String,
    ) -> Self {
        Self {
            provider,
            exact_version,
            boundary_mode,
            projection_digest,
            artifact_ref,
            probed_at,
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
    }
}
