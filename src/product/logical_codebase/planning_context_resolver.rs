//! 唯一 `PlanningContextResolver`：使 Story/Design/WorkItemPlan 的 context、
//! cwd、prompt、session audit 全部来自同一 `PlanningContextSnapshot`。
//!
//! 禁止任何 `issue.repo_id` / first Story fallback（REQ-PLN-07）：有效成员为空即
//! blocker，不塞 primary；`active_required` / 政策 artifact 缺失时 fail-closed，
//! 不回退任何单仓路径。`cwd` 来自 manifest 的 `provider_context_root`（聚合根），
//! 不硬编码 `/aggregate`。

use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::aggregate_index::{AggregateIndexError, AggregateIndexStore};
use crate::product::logical_codebase::planning_context::{
    MemberCheckoutFingerprint, PlanningContextSnapshot, PlanningContextSnapshotStore,
};
use crate::product::logical_codebase::planning_context_set::{
    InventoryInjection, PlanningContextSetResolver, RepositoryContextResolution,
    render_compact_inventory,
};
use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
use crate::product::logical_codebase::{LogicalCodebaseStore, LogicalRepositoryId};

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
pub struct PlanningContextResolver {
    paths: ProductAppPaths,
    logical: LogicalCodebaseStore,
    sets: PlanningContextSetResolver,
    snapshots: PlanningContextSnapshotStore,
}

impl PlanningContextResolver {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self {
            logical: LogicalCodebaseStore::new(paths.clone()),
            sets: PlanningContextSetResolver::new(paths.clone()),
            snapshots: PlanningContextSnapshotStore::new(paths.clone()),
            paths,
        }
    }

    /// 构建 `ResolvedPlanningContext`。流程：解析参与仓库集合 → fail-closed 拒绝空有效
    /// 成员（REQ-PLN-07）→ 读 active 索引 + 政策 artifact（缺失即 blocker）→ 渲染紧凑
    /// inventory → 组装并持久化快照 → 返回唯一上下文。`cwd` 来自 manifest 的
    /// `provider_context_root`。
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
        let resolution = self.sets.resolve(project_id, issue_id)?;
        if resolution.set.is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "effective_member_empty",
                reason: format!(
                    "{project_id}/{issue_id}: no effective member; primary fallback forbidden"
                ),
            });
        }

        let index = AggregateIndexStore::new(self.paths.clone())
            .active_required(project_id)
            .map_err(map_index_error)?;
        let policy = AggregatePolicyArtifactStore::new(self.paths.clone())
            .get(project_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "aggregate_policy_artifact",
                id: project_id.to_string(),
            })?;

        // cwd 来自 manifest 的 provider_context_root（聚合根），不硬编码。
        let manifest =
            self.logical
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
            captured_at: chrono::Utc::now().to_rfc3339(),
        };
        // 返回的快照携带冻结指纹（与落盘一致），保证 `ResolvedPlanningContext.snapshot`
        // 是 context 的唯一事实来源（Task 11 resume 校验依赖该字段）。
        snapshot.access_fingerprint = snapshot.access_fingerprint_value();

        Ok(ResolvedPlanningContext {
            cwd,
            inventory_injection: injection,
            aggregate_index_id: index.aggregate_index_id,
            policy_digest: policy.digest,
            best_effort_readonly_status: BestEffortReadonlyStatus::BestEffortConfigured,
            context_resolution: resolution,
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
mod tests {
    use super::*;
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
    };
    use crate::product::logical_codebase::planning_context_set::InventoryInjectionBudget;
    use crate::product::logical_codebase::policy::{
        AggregatePolicyArtifactStore, PolicyTarget, SessionPolicyAction,
    };
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
        IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore, MemberStatus,
        RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use std::path::PathBuf;
    use tempfile::TempDir;
    use uuid::Uuid;

    /// 稳定 UUID：禁止运行时随机，保证测试可复现；ID 组成磁盘路径前受
    /// `validate_relative_id` 约束（本测试使用 `project_0001` / `issue_0001` 等稳定 id）。
    const API_MEMBER_UUID: Uuid = stable_uuid(0x0001);

    const fn stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        // version 7 + variant 10xx，满足 Uuid::from_bytes 的合法构造。
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    struct ResolverFixture {
        // 保留 temp 以持有临时目录生命周期；paths 派生自 temp.path()。
        #[allow(dead_code)]
        temp: TempDir,
        paths: ProductAppPaths,
        api_member_id: LogicalRepositoryId,
        cached_policy_digest: Option<String>,
    }

    impl ResolverFixture {
        fn resolver(&self) -> PlanningContextResolver {
            PlanningContextResolver::new(self.paths.clone())
        }

        fn aggregate_root(&self) -> PathBuf {
            self.temp.path().join("aggregate-root")
        }

        /// planning 只读启动使用的 provider ref。与 gateway `ProviderRef` 对齐,
        /// planning 走 ClaudeCode(Codex danger-full-access 在 gateway 路由级被阻断)。
        fn provider_ref(&self) -> crate::product::logical_codebase::provider_gateway::ProviderRef {
            crate::product::logical_codebase::provider_gateway::ProviderRef::claude_code(
                "cap_claude_code_1_4_0",
            )
        }

        /// planning 启动携带的托管配置 artifact 引用(envelope 冻结其 digest)。
        fn config_artifact_ref(&self) -> String {
            "sha256:managed-config-artifact".to_string()
        }

        fn membership_revision(&self) -> u64 {
            1
        }

        fn policy_digest(&self) -> String {
            self.cached_policy_digest
                .clone()
                .expect("write_active_manifest_index_and_policy must run first")
        }

        /// 写入单成员 manifest（api，active）+ 显式 selection(issue_0001 → api) +
        /// active aggregate index（membership_revision 与 manifest 对齐）+ 政策
        /// bootstrap artifact，覆盖 resolver 的所有必读依赖。
        fn write_active_manifest_index_and_policy(&mut self) {
            let store = LogicalCodebaseStore::new(self.paths.clone());
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.aggregate_root(),
                vec![self.api_member_id],
            );
            store.save_manifest("project_0001", &manifest).unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(self.api_member_id, "api", MemberStatus::Active),
                )
                .unwrap();
            store
                .save_checkout("project_0001", &self.api_checkout())
                .unwrap();

            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![self.api_member_id],
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();

            // active aggregate index：成员快照与 api member 对齐。
            let index = active_index_record("project_0001", self.api_member_id);
            AggregateIndexStore::new(self.paths.clone())
                .create("project_0001", index.clone())
                .unwrap();
            let mut activated = index.clone();
            activated.status = AggregateIndexStatus::Active;
            AggregateIndexStore::new(self.paths.clone())
                .replace_active("project_0001", activated)
                .unwrap();

            // 政策 bootstrap artifact。
            let policy = AggregatePolicyArtifactStore::new(self.paths.clone())
                .ensure_bootstrap(&manifest)
                .unwrap();
            self.cached_policy_digest = Some(policy.digest);
        }

        /// 写入一个 issue（issue_empty）的显式空 selection，使 resolver 对该 issue
        /// 解析出空有效成员集合，触发 fail-closed blocker。
        fn write_selection_with_no_effective_members(&self) {
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_empty",
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();
        }

        /// 模拟成员变更：manifest membership_revision 1 → 2 并同步推进 active aggregate
        /// index 的 membership_revision 与成员 checkout revision，使 planning snapshot
        /// 指纹漂移（membership/index/checkout 任一变化都会改变 access_fingerprint）。
        /// 与 `write_active_manifest_index_and_policy` 保持同一项目数据，供 resume 测试使用。
        fn change_membership_revision(&self) {
            let store = LogicalCodebaseStore::new(self.paths.clone());
            let mut manifest = store.load_manifest("project_0001").unwrap().unwrap();
            manifest.membership_revision = 2;
            store.save_manifest("project_0001", &manifest).unwrap();

            let index_store = AggregateIndexStore::new(self.paths.clone());
            let mut index = index_store.active("project_0001").unwrap().unwrap();
            index.membership_revision = 2;
            for snapshot in &mut index.member_snapshots {
                snapshot.revision = "def456".to_string();
            }
            index.updated_at = "2026-08-10T01:00:00Z".to_string();
            index_store.replace_active("project_0001", index).unwrap();
        }

        /// 模拟 checkout identity 更换：仅变更 active aggregate index 成员的
        /// checkout_id（revision/dirty/availability/membership 均不变）。B2 验证
        /// checkout_id 参与指纹哈希，避免 checkout 更换被漂移检测绕过。
        fn change_checkout_id(&self) {
            let index_store = AggregateIndexStore::new(self.paths.clone());
            let mut index = index_store.active("project_0001").unwrap().unwrap();
            for snapshot in &mut index.member_snapshots {
                snapshot.checkout_id = RepositoryCheckoutId(stable_uuid(0x0002));
            }
            index.updated_at = "2026-08-10T02:00:00Z".to_string();
            index_store.replace_active("project_0001", index).unwrap();
        }

        fn member_record(
            &self,
            id: LogicalRepositoryId,
            alias: &str,
            status: MemberStatus,
        ) -> CodebaseMemberRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            let checkout_path = self.aggregate_root().join(alias);
            CodebaseMemberRecord {
                logical_repository_id: id,
                physical_repository_id: format!("repository_{alias}"),
                alias: alias.to_string(),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &checkout_path,
                    checkout_path.join(".git"),
                    Some(format!("ssh://git@example.test/acme/{alias}.git")),
                ),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".to_string()],
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![RepositoryCheckoutId(Uuid::nil())],
                status,
                created_at: now.clone(),
                updated_at: now,
            }
        }

        fn api_checkout(&self) -> RepositoryCheckoutRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            RepositoryCheckoutRecord {
                checkout_id: RepositoryCheckoutId(Uuid::nil()),
                logical_repository_id: self.api_member_id,
                physical_repository_id: "repository_api".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: self.aggregate_root().join("api"),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: "sha256:git-dir".to_string(),
                revision: Some("abc123".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            }
        }
    }

    fn active_index_record(
        project_id: &str,
        member_id: LogicalRepositoryId,
    ) -> AggregateIndexRecord {
        let now = "2026-08-10T00:00:00Z".to_string();
        AggregateIndexRecord::building(
            "aggregate_index_0001".to_string(),
            project_id.to_string(),
            1,
            vec![AggregateIndexMemberSnapshot::indexed(
                member_id,
                RepositoryCheckoutId(Uuid::nil()),
                "abc123".to_string(),
                false,
                now,
            )],
            "2026-08-10T00:00:00Z".to_string(),
        )
    }

    fn resolver_fixture() -> ResolverFixture {
        let temp = tempfile::tempdir().unwrap();
        ResolverFixture {
            paths: ProductAppPaths::new(temp.path()),
            temp,
            api_member_id: LogicalRepositoryId(API_MEMBER_UUID),
            cached_policy_digest: None,
        }
    }

    #[test]
    fn resolver_produces_single_snapshot_cwd_and_inventory_for_all_artifacts() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();

        let resolved = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();
        assert_eq!(resolved.cwd, fixture.aggregate_root());
        assert_eq!(
            resolved.snapshot.membership_revision,
            fixture.membership_revision()
        );
        assert_eq!(resolved.snapshot.policy_digest, fixture.policy_digest());
        assert!(
            resolved.inventory_injection.rendered.len()
                <= InventoryInjectionBudget::DEFAULT.hard_bytes
        );
        assert_eq!(
            resolved.best_effort_readonly_status,
            BestEffortReadonlyStatus::BestEffortConfigured
        );
    }

    #[test]
    fn planning_launch_has_no_write_roots_and_cwd_is_aggregate_root() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        let resolved = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();

        let request =
            resolved.launch_request(fixture.provider_ref(), fixture.config_artifact_ref());
        assert_eq!(request.action, SessionPolicyAction::PlanningReadOnly);
        assert!(request.writable_roots.is_empty());
        assert_eq!(request.readable_roots, vec![fixture.aggregate_root()]);
        assert_eq!(
            request.target,
            PolicyTarget::aggregate_root(fixture.aggregate_root())
        );
    }

    #[test]
    fn resolver_rejects_primary_fallback_when_selection_empty() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        fixture.write_selection_with_no_effective_members();

        let error = fixture
            .resolver()
            .build("project_0001", "issue_empty", &[])
            .unwrap_err();
        assert!(error.to_string().contains("effective_member_empty"));
    }

    #[test]
    fn resume_with_matching_fingerprint_reuses_context_and_mismatch_rebuilds() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        let first = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();
        let persisted_fingerprint = first.snapshot.access_fingerprint.clone();
        // build 返回的 snapshot 携带冻结指纹，与落盘一致（Task 11 一致性修正）。
        assert!(!persisted_fingerprint.is_empty());

        let same = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(same, ResumeDecision::SameContext(_)));

        fixture.change_membership_revision(); // 模拟成员变更，指纹漂移
        let stale = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(
            stale,
            ResumeDecision::StaleContext { reason, .. } if reason != persisted_fingerprint
        ));
    }

    #[test]
    fn resume_is_readonly_and_rejected_reconnect_stays_stale() {
        // B1/TOCTOU 修复：resume 先加载既有 snapshot（load 只读、不写）再比较，禁止
        // 先写后比。因此漂移首次拒绝后，同一会话重连（未启动新会话）仍是 StaleContext，
        // 绝不因 snapshot 被 build 提前更新而误判 SameContext。
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        let first = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();
        let persisted_fingerprint = first.snapshot.access_fingerprint.clone();

        fixture.change_membership_revision(); // 指纹漂移

        let stale = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(stale, ResumeDecision::StaleContext { .. }));

        // resume 不落盘：拒绝后落盘快照仍为旧指纹（未被 build 更新）。
        let store = PlanningContextSnapshotStore::new(fixture.paths.clone());
        let persisted = store.load("project_0001", "issue_0001").unwrap().unwrap();
        assert_eq!(persisted.access_fingerprint, persisted_fingerprint);

        // 重连（未启动新会话）仍是 StaleContext。
        let again = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(again, ResumeDecision::StaleContext { .. }));
    }

    #[test]
    fn checkout_identity_change_triggers_fingerprint_drift() {
        // B2 修复：access_fingerprint_value 哈希 checkout_id。checkout identity 更换而
        // revision/dirty/availability 不变时，也必须触发漂移（StaleContext）。
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();

        let same = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(same, ResumeDecision::SameContext(_)));

        fixture.change_checkout_id(); // 仅更换 checkout identity
        let stale = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(stale, ResumeDecision::StaleContext { .. }));
    }
}
