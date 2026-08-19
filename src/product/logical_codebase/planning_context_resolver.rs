//! 唯一 `PlanningContextResolver`：使 Story/Design/WorkItemPlan 的 context、
//! cwd、prompt、session audit 全部来自同一 `PlanningContextSnapshot`。
//!
//! 禁止任何 `issue.repo_id` / first Story fallback（REQ-PLN-07）：有效成员为空即
//! blocker，不塞 primary；`active_required` / 政策 artifact 缺失时 fail-closed，
//! 不回退任何单仓路径。`cwd` 来自 manifest 的 `provider_context_root`（聚合根），
//! 不硬编码 `/aggregate`。

use std::path::PathBuf;
use std::sync::Arc;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexError, AggregateIndexFreshness, AggregateIndexFreshnessService,
    AggregateIndexOperation, AggregateIndexStore, CodeGraphCli, CodeGraphExcludeGenerator,
};
use crate::product::logical_codebase::planning_context::{
    MemberCheckoutFingerprint, PlanningContextSnapshot, PlanningContextSnapshotStore,
};
use crate::product::logical_codebase::planning_context_set::{
    InventoryInjection, PlanningContextSetResolver, RepositoryContextResolution,
    render_compact_inventory,
};
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::{
    InvalidationRecord, LogicalCodebaseStore, LogicalRepositoryId,
};

/// 规划只读 best-effort 状态。当前唯一取值为 `BestEffortConfigured`：已配置目标 +
/// cwd + pre/post 检测，但未达 `production_verified_readonly`，因此不宣称「物理上
/// 无法写入」。
///
/// 后续 task（PreToolUse deny / 生产级只读）可在本枚举新增 `ProductionVerifiedReadonly`，
/// 但本 task 仅产出 `BestEffortConfigured`，调用方必须据此上报 best-effort 语义。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BestEffortReadonlyStatus {
    /// best_effort_configured：配置目标 + cwd + pre/post 检测，未达 production_verified_readonly。
    BestEffortConfigured,
}

/// `PlanningContextResolver::build` 的返回值。Story/Design/WorkItemPlan 的
/// context/cwd/prompt/session audit 全部由该返回值派生，禁止旁路重建 context。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlanningContext {
    /// 冻结后的唯一上下文快照；context/prompt/session audit 的唯一事实来源。
    pub snapshot: PlanningContextSnapshot,
    /// 聚合根 cwd，等于 manifest 的 `provider_context_root`。
    pub cwd: PathBuf,
    /// 紧凑成员 inventory 注入（已按预算截断），供 prompt 注入。
    pub inventory_injection: InventoryInjection,
    /// 读到的 active aggregate index id（冗余于 snapshot，便于 gateway 直接消费）。
    pub aggregate_index_id: String,
    /// 读到的 policy digest（冗余于 snapshot，便于 gateway 直接消费）。
    pub policy_digest: String,
    /// best-effort 只读状态；本 task 恒为 `BestEffortConfigured`。
    pub best_effort_readonly_status: BestEffortReadonlyStatus,
    /// 参与仓库集合解析结果（含 invalid 成员与 manifest 成员修订号）。
    pub context_resolution: RepositoryContextResolution,
    /// 失效标记（REQ-PLN-02）：规划后成员删除/停用后为 Some；消费方应展示失效警告。
    pub invalidation: Option<InvalidationRecord>,
}

impl ResolvedPlanningContext {
    /// 构造 planning 只读启动请求。action 恒为 `PlanningReadOnly`,writable_roots
    /// 为空,readable_roots 与 target.worktree 均为聚合根 cwd
    /// (`provider_context_root`)。target 用 `PolicyTarget::aggregate_root`,
    /// 不绑定具体 logical member(logical_repository_id/checkout_id 为空串)。
    ///
    /// 返回的 `SessionLaunchRequest` 需经 `LogicalCodebaseProviderGateway::validate`
    /// 冻结为 validated policy 后才能进入真实 provider 启动;本方法只构造请求,
    /// 不越权校验政策。
    pub fn launch_request(
        &self,
        provider: crate::product::logical_codebase::provider_gateway::ProviderRef,
        config_artifact_ref: impl Into<String>,
    ) -> crate::product::logical_codebase::provider_gateway::SessionLaunchRequest {
        crate::product::logical_codebase::provider_gateway::SessionLaunchRequest::planning(
            self.snapshot.project_id.clone(),
            provider,
            crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                self.cwd.clone(),
            ),
            vec![self.cwd.clone()],
            config_artifact_ref,
        )
    }

    /// 经 gateway `validate` 产出 validated policy,再组装为
    /// `ValidatedStreamingProviderInput`。本方法是 planning 启动经 gateway 的唯一
    /// 组装点:cwd/policy/action 全部来自本 context 的快照与 envelope,不旁路重建。
    ///
    /// `provider_type` 由 validated envelope 冻结的 `provider_dialect` 派生
    /// (ClaudeCodeCliV1 → ClaudeCode,CodexCliV1 → Codex),与 gateway 路由级
    /// 解析的 dialect 一致。本 task 不实现硬只读 PreToolUse deny,只读语义由
    /// envelope 的 `PlanningReadOnly` action(空 writable_roots)+ prompt 层
    /// 「只读」指令表达(best_effort_configured,design §5.2),permission_mode
    /// 取 `Supervised`。
    pub fn validated_planning_input(
        &self,
        gateway: &crate::product::logical_codebase::provider_gateway::LogicalCodebaseProviderGateway,
        provider: crate::product::logical_codebase::provider_gateway::ProviderRef,
        config_artifact_ref: impl Into<String>,
        prompt: String,
    ) -> Result<
        crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
        crate::product::logical_codebase::provider_gateway::ProviderGatewayError,
    > {
        let request = self.launch_request(provider, config_artifact_ref);
        let validated = gateway.validate(request)?;
        let provider_type = provider_type_for_dialect(validated.envelope().provider_dialect);
        let input = crate::cross_cutting::streaming_provider::StreamingProviderInput {
            provider_type,
            role: crate::protocol::contracts::AdapterRole::Orchestrator,
            prompt,
            working_dir: self.cwd.clone(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode:
                crate::cross_cutting::streaming_provider::ProviderPermissionMode::Supervised,
            structured_output_contract: None,
            env_vars: std::collections::BTreeMap::new(),
            timeout_secs: 0,
        };
        Ok(
            crate::cross_cutting::session_launch::ValidatedStreamingProviderInput::new(
                input, validated,
            ),
        )
    }
}

/// 唯一规划上下文 resolver。组合 `PlanningContextSetResolver`（参与仓库集合）、
/// `AggregateIndexStore::active_required`（索引快照）、
/// `AggregatePolicyArtifactStore`（政策 digest）与
/// `PlanningContextSnapshotStore`（快照持久化），产出单一 `ResolvedPlanningContext`。
pub trait PlanningIndexFreshness: Send + Sync {
    fn assess(&self, project_id: &str) -> Result<AggregateIndexFreshness, AggregateIndexError>;
    fn sync_if_stale(
        &self,
        project_id: &str,
    ) -> Result<
        crate::product::logical_codebase::aggregate_index::AggregateIndexRecord,
        AggregateIndexError,
    >;
}

impl PlanningIndexFreshness for AggregateIndexFreshnessService {
    fn assess(&self, project_id: &str) -> Result<AggregateIndexFreshness, AggregateIndexError> {
        AggregateIndexFreshnessService::assess(self, project_id)
    }

    fn sync_if_stale(
        &self,
        project_id: &str,
    ) -> Result<
        crate::product::logical_codebase::aggregate_index::AggregateIndexRecord,
        AggregateIndexError,
    > {
        AggregateIndexFreshnessService::sync_if_stale(self, project_id)
    }
}

pub struct PlanningContextResolver {
    paths: ProductAppPaths,
    sets: PlanningContextSetResolver,
    snapshots: PlanningContextSnapshotStore,
    freshness: Arc<dyn PlanningIndexFreshness>,
}

impl PlanningContextResolver {
    pub fn new(paths: ProductAppPaths) -> Self {
        let freshness = AggregateIndexFreshnessService::new(AggregateIndexOperation::new(
            paths.clone(),
            CodeGraphCli::new(
                Arc::new(crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner),
                "codegraph".to_string(),
            ),
            CodeGraphExcludeGenerator,
        ));
        Self::with_freshness_service(paths, Arc::new(freshness))
    }

    #[cfg(test)]
    pub fn new_without_freshness(paths: ProductAppPaths) -> Self {
        struct TestFreshness {
            store: AggregateIndexStore,
        }
        impl PlanningIndexFreshness for TestFreshness {
            fn assess(
                &self,
                project_id: &str,
            ) -> Result<AggregateIndexFreshness, AggregateIndexError> {
                let record = self.store.active_required(project_id)?;
                Ok(AggregateIndexFreshness::active(record))
            }
            fn sync_if_stale(
                &self,
                project_id: &str,
            ) -> Result<
                crate::product::logical_codebase::aggregate_index::AggregateIndexRecord,
                AggregateIndexError,
            > {
                self.store.active_required(project_id)
            }
        }
        Self::with_freshness_service(
            paths.clone(),
            Arc::new(TestFreshness {
                store: AggregateIndexStore::new(paths),
            }),
        )
    }

    pub fn with_freshness_service(
        paths: ProductAppPaths,
        freshness: Arc<dyn PlanningIndexFreshness>,
    ) -> Self {
        Self {
            sets: PlanningContextSetResolver::new(paths.clone()),
            snapshots: PlanningContextSnapshotStore::new(paths.clone()),
            paths,
            freshness,
        }
    }

    /// 构建 `ResolvedPlanningContext`。流程：解析参与仓库集合 → fail-closed 拒绝空有效
    /// 成员（REQ-PLN-07）→ 读 active 索引 + 政策 artifact（缺失即 blocker）→ 渲染紧凑
    /// inventory → 组装并持久化快照 → 返回唯一上下文。`cwd` 来自 manifest 的
    /// `provider_context_root`。v1.3：manifest/selection/index/policy 均按 issue 所属
    /// codebase 的 lc_id 子树解析。
    pub async fn build_with_fresh_index(
        &self,
        project_id: &str,
        issue_id: &str,
        targets: &[LogicalRepositoryId],
    ) -> Result<ResolvedPlanningContext, ProductStoreError> {
        let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
            &self.paths,
            project_id,
            issue_id,
        )?;
        let warning = self
            .refresh_index_for_read(project_id, lc_id.as_deref())
            .await?;
        let mut resolved = self.resolve_context(project_id, issue_id, targets)?;
        if let Some(warning) = warning {
            resolved.inventory_injection.rendered.push('\n');
            resolved.inventory_injection.rendered.push_str(&warning);
        }
        self.snapshots.save(&resolved.snapshot)?;
        Ok(resolved)
    }

    /// Assess and, only for a stale index, synchronize the index away from the
    /// Tokio worker pool. A degraded record remains last-known-good and is
    /// surfaced as an auditable warning in the planning inventory.
    async fn refresh_index_for_read(
        &self,
        project_id: &str,
        lc_id: Option<&str>,
    ) -> Result<Option<String>, ProductStoreError> {
        let freshness = match lc_id {
            Some(lc_id) => {
                let operation = AggregateIndexOperation::new(
                    self.paths.clone(),
                    CodeGraphCli::new(
                        Arc::new(
                            crate::cross_cutting::bounded_command_runner::TokioBoundedCommandRunner,
                        ),
                        "codegraph".to_string(),
                    ),
                    CodeGraphExcludeGenerator,
                )
                .for_lc(lc_id);
                Arc::new(AggregateIndexFreshnessService::new(operation))
                    as Arc<dyn PlanningIndexFreshness>
            }
            None => Arc::clone(&self.freshness),
        };
        let project_id = project_id.to_string();
        tokio::task::spawn_blocking(move || {
            let assessment = freshness.assess(&project_id).map_err(map_index_error)?;
            // A degraded record is explicitly last-known-good: never trigger a
            // rebuild merely because its live evidence also differs.
            if assessment.record.status
                == crate::product::logical_codebase::aggregate_index::AggregateIndexStatus::Degraded
            {
                let reason = if assessment.reason.is_empty() {
                    assessment
                        .record
                        .warning
                        .unwrap_or_else(|| "unknown reason".to_string())
                } else {
                    assessment.reason
                };
                return Ok(Some(format!("aggregate index warning: {reason}")));
            }
            if assessment.status
                == crate::product::logical_codebase::aggregate_index::AggregateIndexStatus::Stale
            {
                freshness
                    .sync_if_stale(&project_id)
                    .map_err(map_index_error)?;
            }
            Ok(None)
        })
        .await
        .map_err(|error| {
            ProductStoreError::Io(format!("planning index freshness task failed: {error}"))
        })?
    }

    pub async fn resume_with_fresh_index(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<ResumeDecision, ProductStoreError> {
        let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
            &self.paths,
            project_id,
            issue_id,
        )?;
        let warning = self
            .refresh_index_for_read(project_id, lc_id.as_deref())
            .await?;
        self.resume_after_freshness(project_id, issue_id, warning)
    }

    fn resume_after_freshness(
        &self,
        project_id: &str,
        issue_id: &str,
        warning: Option<String>,
    ) -> Result<ResumeDecision, ProductStoreError> {
        let previous = match self.snapshots.load(project_id, issue_id)? {
            Some(previous) => previous,
            None => {
                let mut current = self.resolve_context(project_id, issue_id, &[])?;
                if let Some(warning) = warning {
                    current.inventory_injection.rendered.push('\n');
                    current.inventory_injection.rendered.push_str(&warning);
                }
                self.snapshots.save(&current.snapshot)?;
                return Ok(ResumeDecision::SameContext(current));
            }
        };
        if previous.invalidation.is_some() {
            let mut current = self.resolve_context(project_id, issue_id, &[])?;
            if let Some(warning) = warning {
                current.inventory_injection.rendered.push('\n');
                current.inventory_injection.rendered.push_str(&warning);
            }
            return Ok(ResumeDecision::StaleContext {
                reason: format!(
                    "invalidated:{}",
                    previous
                        .invalidation
                        .as_ref()
                        .map(|record| record.reason.as_str())
                        .unwrap_or("member_removed")
                ),
                rebuilt: current,
            });
        }
        let mut current = self.resolve_context(project_id, issue_id, &[])?;
        if let Some(warning) = warning {
            current.inventory_injection.rendered.push('\n');
            current.inventory_injection.rendered.push_str(&warning);
        }
        if current.snapshot.access_fingerprint_value() == previous.access_fingerprint {
            Ok(ResumeDecision::SameContext(current))
        } else {
            Ok(ResumeDecision::StaleContext {
                reason: format!("fingerprint_changed:{}", previous.access_fingerprint),
                rebuilt: current,
            })
        }
    }

    pub fn build(
        &self,
        project_id: &str,
        issue_id: &str,
        targets: &[LogicalRepositoryId],
    ) -> Result<ResolvedPlanningContext, ProductStoreError> {
        let resolved = self.resolve_context(project_id, issue_id, targets)?;
        self.snapshots.save(&resolved.snapshot)?;
        Ok(resolved)
    }

    /// 计算当前规划上下文但**不落盘**。`resume` 指纹比对专用：先加载既有 snapshot
    /// （`load` 只读、不写）并复算当前指纹（不写盘）再比较，禁止先写后比（B1/TOCTOU：
    /// 若先 `build` 写新 snapshot，漂移首次拒绝后重连会因 snapshot 已被更新而误判
    /// `SameContext`，继续沿用旧会话 prompt）。`build` 在其基础上落盘快照。
    fn resolve_context(
        &self,
        project_id: &str,
        issue_id: &str,
        targets: &[LogicalRepositoryId],
    ) -> Result<ResolvedPlanningContext, ProductStoreError> {
        // v1.3：按 issue 唯一归属的代码库把 index/policy/manifest 全部解析到 lc_id 子树。
        let lc_id = crate::product::logical_codebase::resolve_issue_logical_codebase_id(
            &self.paths,
            project_id,
            issue_id,
        )?;
        let (logical, index_store, policy_store) = match lc_id.as_deref() {
            Some(lc_id) => (
                LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id),
                AggregateIndexStore::for_lc(self.paths.clone(), lc_id),
                AggregatePolicyArtifactStore::for_lc(self.paths.clone(), lc_id),
            ),
            None => (
                LogicalCodebaseStore::new(self.paths.clone()),
                AggregateIndexStore::new(self.paths.clone()),
                AggregatePolicyArtifactStore::new(self.paths.clone()),
            ),
        };
        let resolution = self.sets.resolve(project_id, issue_id)?;
        if resolution.set.is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "effective_member_empty",
                reason: format!(
                    "{project_id}/{issue_id}: no effective member; primary fallback forbidden"
                ),
            });
        }

        let index = index_store
            .active_required(project_id)
            .map_err(map_index_error)?;
        let policy = policy_store
            .get(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "aggregate_policy_artifact",
                id: project_id.to_string(),
            })?;

        // cwd 来自 manifest 的 provider_context_root（聚合根），不硬编码。
        let manifest =
            logical
                .load_manifest(project_id)?
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "logical_codebase_manifest",
                    id: project_id.to_string(),
                })?;
        let cwd = manifest.provider_context_root.clone();

        let injection = render_compact_inventory(&resolution, targets)?;

        let member_fingerprints = build_member_fingerprints(&index.member_snapshots, &resolution);

        let mut snapshot = PlanningContextSnapshot {
            schema_version: 1,
            project_id: project_id.into(),
            issue_id: issue_id.into(),
            membership_revision: resolution.membership_revision,
            effective_member_ids: resolution
                .set
                .iter()
                .map(|member| member.member_id)
                .collect(),
            member_fingerprints,
            aggregate_index_id: index.aggregate_index_id.clone(),
            index_revision: index.membership_revision,
            policy_digest: policy.digest.clone(),
            access_fingerprint: String::new(),
            invalidation: None,
            captured_at: chrono::Utc::now().to_rfc3339(),
        };
        // 返回的快照携带冻结指纹（与落盘一致），保证 `ResolvedPlanningContext.snapshot`
        // 是 context 的唯一事实来源（Task 11 resume 校验依赖该字段）。
        snapshot.access_fingerprint = snapshot.access_fingerprint_value();
        // REQ-PLN-02：存在失效成员时标记 snapshot 失效并在结果中附失效警告；
        // 失效是显式标记，不删除既有 JSON，resume 据此强制 StaleContext 重建。
        let invalidation = if resolution.invalid_member_ids.is_empty() {
            None
        } else {
            Some(InvalidationRecord {
                reason: "member_removed".to_string(),
                invalidated_at: snapshot.captured_at.clone(),
            })
        };
        snapshot.invalidation = invalidation.clone();

        Ok(ResolvedPlanningContext {
            cwd,
            inventory_injection: injection,
            aggregate_index_id: index.aggregate_index_id,
            policy_digest: policy.digest,
            best_effort_readonly_status: BestEffortReadonlyStatus::BestEffortConfigured,
            context_resolution: resolution,
            invalidation,
            snapshot,
        })
    }
}

/// resume/续接规划会话的校验决策（REQ-PLN-03）。指纹一致沿用现有 session 审计与
/// prompt 上下文；指纹漂移启动新会话重建上下文，不沿用可能过时/越权的内容。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResumeDecision {
    /// 快照指纹与当前 membership/index/checkout 一致：沿用现有上下文继续。
    SameContext(ResolvedPlanningContext),
    /// 快照指纹漂移：不得沿用旧上下文，必须启动新会话并重建。`reason` 携带旧的
    /// access_fingerprint 供审计，`rebuilt` 为重新 `build` 的权威上下文。
    StaleContext {
        reason: String,
        rebuilt: ResolvedPlanningContext,
    },
}

impl PlanningContextResolver {
    /// resume/续接校验：**先加载**既有 planning snapshot（`load` 只读、不写），**再**复算
    /// 当前指纹（不落盘）比较。指纹一致返回 `SameContext`（沿用现有 session 审计与
    /// prompt 上下文）；指纹漂移返回 `StaleContext`（拒绝沿用可能过时/越权的
    /// prompt/cwd/policy，携带重建上下文）。无既有快照时视为首次构建，直接
    /// `SameContext`（无旧上下文可沿用）。
    ///
    /// 本方法只读判定、不落盘（B1/TOCTOU 修复：禁止先写后比）；「重建后写新 snapshot」
    /// 属于消费方启动新会话时的动作（Web 层 StaleContext 分支）。因此同一会话重连
    /// （未启动新会话）持续返回 `StaleContext`，不会因 snapshot 被提前更新而误判。
    ///
    /// resume 校验在 provider 启动前完成；envelope 的 resume fingerprint（Plan 2）与
    /// planning snapshot 指纹互补——envelope 校验 policy/target/version 一致，snapshot
    /// 校验 membership/index/checkout 一致。两者均须通过才允许 resume。
    pub fn resume(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<ResumeDecision, ProductStoreError> {
        let previous = match self.snapshots.load(project_id, issue_id)? {
            Some(previous) => previous,
            None => {
                return Ok(ResumeDecision::SameContext(self.build(
                    project_id,
                    issue_id,
                    &[],
                )?));
            }
        };
        // REQ-PLN-02：规划后成员删除/停用 → snapshot 已标记失效 → 强制 StaleContext 重建，
        // 绝不沿用可能已失效的旧上下文（即使指纹未漂移）。
        if previous.invalidation.is_some() {
            let current = self.resolve_context(project_id, issue_id, &[])?;
            return Ok(ResumeDecision::StaleContext {
                reason: format!(
                    "invalidated:{}",
                    previous
                        .invalidation
                        .as_ref()
                        .map(|record| record.reason.as_str())
                        .unwrap_or("member_removed")
                ),
                rebuilt: current,
            });
        }
        // 先加载既有 snapshot（`load` 只读、不写），再复算当前指纹（`resolve_context`
        // 不落盘）比较。禁止先写后比（B1/TOCTOU）：否则漂移首次拒绝后，重连会因
        // snapshot 已被 build 更新而误判 `SameContext`，继续沿用旧会话 prompt。
        // 落盘 rebuilt 快照属于消费方「启动新会话并重建」动作（Web 层 B3），本方法
        // 保持只读判定，重连（未启动新会话）持续返回 `StaleContext`。
        let current = self.resolve_context(project_id, issue_id, &[])?;
        if current.snapshot.access_fingerprint_value() == previous.access_fingerprint {
            Ok(ResumeDecision::SameContext(current))
        } else {
            Ok(ResumeDecision::StaleContext {
                reason: format!("fingerprint_changed:{}", previous.access_fingerprint),
                rebuilt: current,
            })
        }
    }
}

/// 将 active aggregate-index 成员快照投影为快照所需 `MemberCheckoutFingerprint`，
/// 只保留参与成员集合内的成员；可用性直接取 index 的 `included` 标志。
fn build_member_fingerprints(
    member_snapshots: &[crate::product::logical_codebase::aggregate_index::AggregateIndexMemberSnapshot],
    resolution: &RepositoryContextResolution,
) -> Vec<MemberCheckoutFingerprint> {
    member_snapshots
        .iter()
        .filter(|snapshot| {
            resolution
                .set
                .iter()
                .any(|member| member.member_id == snapshot.logical_repository_id)
        })
        .map(|snapshot| MemberCheckoutFingerprint {
            logical_repository_id: snapshot.logical_repository_id,
            checkout_id: snapshot.checkout_id,
            revision: snapshot.revision.clone(),
            dirty: snapshot.dirty,
            available: snapshot.included,
        })
        .collect()
}

/// `AggregateIndexError` → `ProductStoreError`：聚合索引读取失败一律 fail-closed，
/// 不回退单仓路径。
fn map_index_error(error: AggregateIndexError) -> ProductStoreError {
    ProductStoreError::Io(format!("aggregate_index_unavailable: {error}"))
}

/// 据 validated envelope 冻结的 provider dialect 映射到 streaming input 的
/// `ProviderType`。dialect 与 provider 类型一一对应,与 gateway 路由级解析一致。
/// Fake/测试 provider 不经 gateway,故此处只映射两种真实 dialect。
fn provider_type_for_dialect(
    dialect: crate::product::logical_codebase::policy::ProviderDialect,
) -> crate::protocol::contracts::ProviderType {
    use crate::product::logical_codebase::policy::ProviderDialect;
    match dialect {
        ProviderDialect::ClaudeCodeCliV1 => crate::protocol::contracts::ProviderType::ClaudeCode,
        ProviderDialect::CodexCliV1 => crate::protocol::contracts::ProviderType::Codex,
    }
}

#[cfg(test)]
include!("planning_context_resolver_tests.inc.rs");
