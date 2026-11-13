use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

use chrono::Utc;

use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::entity_deletion_store::{DeletionEntityKind, EntityDeletionStore, EntityScope};
use crate::product::entity_operation_store::EntityOperationStore;
use crate::product::id::{next_sequential_id, next_sequential_id_in_directory};
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    ChildBindingIdentity, HumanGateTakeoverEvent, LcFreshOperationResult, LcFreshOperationState,
    LcResumeRecoveryRecord, LcResumeRecoveryState, LifecycleWorkItemRecord,
    PlanRepairSessionSnapshotDto, ProviderConversationRef, WorkItemRuntimeBinding,
    WorkspaceMessageRecord, WorkspaceRolePermissionModes, WorkspaceSessionLink,
    WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceSessionSummaryRecord, WorkspaceType,
};
use crate::web::workspace_ws_types::{ArtifactVersion, TimelineNode};

use super::{
    CreateWorkItemChildSessionInput, CreateWorkspaceSessionInput, LifecycleStore,
    child_directories, json_file_paths, list_workspace_session_records, path_exists,
    path_is_regular_file, read_workspace_session_record, remove_dir_all_if_exists,
    remove_file_if_exists, workspace_session_file_paths,
};

impl LifecycleStore {
    pub fn compare_and_save_plan_repair_session_state(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        expected: &PlanRepairSessionSnapshotDto,
        next: &PlanRepairSessionSnapshotDto,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(session_id)?;
        if expected.link.child_session_id != session_id || next.link.child_session_id != session_id
        {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "plan_repair_session_state",
                id: session_id.to_string(),
            });
        }
        let scope = self.resolve_session_scope_lenient(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let path = self
                .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
                .join("plan_repair_session_state.json");
            with_exclusive_lock(&path, || {
                let stored: PlanRepairSessionSnapshotDto = read_json(&path)?;
                if stored != *expected {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "plan_repair_session_state",
                        id: session_id.to_string(),
                    });
                }
                if stored == *next {
                    return Ok(());
                }
                write_json(&path, next)
            })
        })
    }

    pub fn save_plan_repair_session_state(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        value: &PlanRepairSessionSnapshotDto,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(session_id)?;
        if value.link.child_session_id != session_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "plan_repair_session_state",
                id: session_id.to_string(),
            });
        }
        let scope = self.resolve_session_scope_lenient(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let path = self
                .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
                .join("plan_repair_session_state.json");
            write_json(&path, value)
        })
    }

    pub fn load_plan_repair_session_state(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<Option<PlanRepairSessionSnapshotDto>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
            .join("plan_repair_session_state.json");
        if !path_exists(&path)? {
            return Ok(None);
        }
        let value: PlanRepairSessionSnapshotDto = read_json(&path)?;
        if value.link.child_session_id != session_id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "plan_repair_session_state",
                id: session_id.to_string(),
            });
        }
        Ok(Some(value))
    }

    pub fn put_session_link(
        &self,
        project_id: &str,
        issue_id: &str,
        value: &WorkspaceSessionLink,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.parent_session_id)?;
        validate_relative_id(&value.child_session_id)?;
        let scope = self.resolve_session_scope_lenient(&value.child_session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let path = self
                .paths
                .issue_lifecycle_root(project_id, issue_id)
                .join("workspace-session-links")
                .join(format!("{}.json", value.id));
            if path_exists(&path)? {
                let existing: WorkspaceSessionLink = read_json(&path)?;
                if existing == *value {
                    return Ok(());
                }
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "workspace_session_link",
                    id: value.id.clone(),
                });
            }
            write_json(&path, value)
        })
    }

    pub fn list_session_links(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<WorkspaceSessionLink>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let root = self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-session-links");
        let mut links = Vec::new();
        for path in json_file_paths(&root)? {
            let link: WorkspaceSessionLink = read_json(&path)?;
            let file_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    ProductStoreError::Io(format!(
                        "invalid workspace session link path: {}",
                        path.display()
                    ))
                })?;
            if link.id != file_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "workspace_session_link",
                    id: file_id.to_string(),
                });
            }
            links.push(link);
        }
        links.sort_by(|left, right| {
            (left.created_at.as_str(), left.id.as_str())
                .cmp(&(right.created_at.as_str(), right.id.as_str()))
        });
        Ok(links)
    }

    pub fn get_session_link(
        &self,
        child_session_id: &str,
    ) -> Result<WorkspaceSessionLink, ProductStoreError> {
        let session = self.get_workspace_session(child_session_id)?;
        let mut matches = self
            .list_session_links(&session.project_id, &session.issue_id)?
            .into_iter()
            .filter(|link| link.child_session_id == child_session_id);
        let link = matches.next().ok_or_else(|| ProductStoreError::NotFound {
            kind: "workspace_session_link",
            id: child_session_id.to_string(),
        })?;
        if matches.next().is_some() {
            return Err(ProductStoreError::Ambiguous {
                kind: "workspace_session_link",
                id: child_session_id.to_string(),
            });
        }
        Ok(link)
    }

    pub fn create_workspace_session(
        &self,
        input: CreateWorkspaceSessionInput,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let id = self.next_workspace_session_id()?;
        // auto 分配：防复用由耐久高水位保证（D1.0 allocate），不落同名的
        // 身份绑定文件（state/entity-identity/sessions/{id}.json 与 session
        // 记录同名，全树文件名唯一性探测会撞名）；绑定在显式 id 路径落盘。
        self.create_workspace_session_with_child_binding(input, id, None, false)
    }

    pub(crate) fn create_workspace_session_with_id(
        &self,
        input: CreateWorkspaceSessionInput,
        id: String,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        self.create_workspace_session_with_child_binding(input, id, None, true)
    }

    /// C1 Task 8（REQ-C1-CHILD-01）：WorkItem compile child 专用创建入口
    /// ——child binding 身份随 session 的同一原子写入持久化；既有同 id 文件
    /// 按基础身份字段＋child binding 幂等重放，身份漂移 fail-closed。
    pub fn create_workspace_child_session(
        &self,
        input: CreateWorkItemChildSessionInput,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        if input.session.workspace_type != WorkspaceType::WorkItem {
            return Err(ProductStoreError::InvalidRecord {
                kind: "workspace_session",
                reason: "work item child binding requires workspace_type work_item".to_string(),
            });
        }
        if input.session.entity_id != input.child_binding.logical_work_item_id {
            return Err(ProductStoreError::InvalidRecord {
                kind: "workspace_session",
                reason: "work item child binding logical id must match session entity id"
                    .to_string(),
            });
        }
        let id = self.next_workspace_session_id()?;
        self.create_workspace_session_with_child_binding(
            input.session,
            id,
            Some(input.child_binding),
            false,
        )
    }

    fn create_workspace_session_with_child_binding(
        &self,
        input: CreateWorkspaceSessionInput,
        id: String,
        child_binding: Option<crate::product::models::ChildBindingIdentity>,
        explicit_identity: bool,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.entity_id)?;
        validate_relative_id(&id)?;
        if input.workspace_type != WorkspaceType::WorkItemPlan
            && input.work_item_plan_options.is_some()
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "workspace_session",
                reason: "work_item_plan_options require workspace_type work_item_plan".to_string(),
            });
        }

        let scope = self.entity_scope_for_new_session(&input, child_binding.as_ref())?;
        let closure_scope = scope.clone();
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            // 会话身份绑定（D1.0 §0.2）：只 ID/scope；同身份幂等。显式 id 在
            // 创建时绑定（跨实体复用拒）；auto id 由耐久高水位保证不复用，
            // 旧 auto 会话的索引迁移按 §0.2「删除前迁移」归 D1.1 收集。
            if explicit_identity && let Some(scope) = &closure_scope {
                self.entity_operation_store()
                    .bind_workspace_session_identity(&id, scope)?;
            }
            let root = self.workspace_sessions_root(&input.project_id, &input.issue_id);
            let target_path = root.join(format!("{id}.json"));
            if path_exists(&target_path)? {
                let existing: WorkspaceSessionRecord = read_json(&target_path)?;
                if existing.id == id
                    && existing.project_id == input.project_id
                    && existing.issue_id == input.issue_id
                    && existing.entity_id == input.entity_id
                    && existing.workspace_type == input.workspace_type
                    && existing.work_item_child_binding == child_binding
                {
                    return Ok(existing);
                }
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "workspace_session",
                    id,
                });
            }
            let now = Utc::now().to_rfc3339();
            let work_item_plan_options = input.work_item_plan_options.unwrap_or_default();
            let session = WorkspaceSessionRecord {
                id: id.clone(),
                project_id: input.project_id,
                issue_id: input.issue_id,
                entity_id: input.entity_id,
                workspace_type: input.workspace_type,
                status: WorkspaceSessionStatus::Open,
                author_provider: input.author_provider,
                reviewer_provider: input.reviewer_provider,
                review_rounds: input.review_rounds,
                permission_modes: WorkspaceRolePermissionModes::default(),
                provisional_reviewer_provider: None,
                reviewer_enabled_at_start: None,
                superpowers_enabled: input.superpowers_enabled,
                openspec_enabled: input.openspec_enabled,
                flow_kind: work_item_plan_options.flow_kind,
                run_policy: work_item_plan_options.run_policy,
                run_history: Default::default(),
                review_invocation_scope: None,
                human_gate_snapshot: None,
                repair_reservation: None,
                human_gate_reservation: None,
                author_rewrite_budget: None,
                policy_diagnostics: Vec::new(),
                provider_start_ledger: Vec::new(),
                sc_generation_teaching_windows: Vec::new(),
                single_candidate_phase: (work_item_plan_options.flow_kind
                    == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate)
                    .then_some(crate::product::models::SingleCandidatePhase::Prepare),
                work_item_plan_source_revision_ref: None,
                plan_candidate_ir_ref: None,
                mechanical_report_ref: None,
                publication_provenance_ref: None,
                approval_attempt_id: None,
                approved_at: None,
                compile_reservation: None,
                sc_recovery_claim: None,
                work_item_runtime_binding: None,
                work_item_child_binding: child_binding,
                provider_conversations: Vec::new(),
                messages: Vec::new(),
                created_at: now.clone(),
                updated_at: now,
                logical_launch_fingerprints: Vec::new(),
                superseded_native_sessions: Vec::new(),
                lc_resume_recovery: None,
                structural_governance_state: None,
                // REQ-ENV-13（设计 A 条目 17）：会话创建即绑定唯一评审
                // 身份（UUID simple）；与记录同文件持久化，会话删除即解绑。
                review_identity_id: uuid::Uuid::new_v4().simple().to_string(),
            };
            super::ensure_target_absent(&target_path)?;
            write_json(&target_path, &session)?;
            Ok(session)
        })
    }

    /// P1 WIGA Task 3：绑定创建——既存 session 必须与完整创建快照一致
    /// （基础 `create_workspace_session_with_id` 只核对五个身份字段，不足以
    /// 作为授权校验）。批 2 P2（REQ-WIGA-02 修订）：run policy 的授权口径
    /// 移至 enrollment 冻结策略——创建/binding/准入/确认投影各接缝互证，
    /// 本入口接受明确的 AutoIfValid（auto 冻结意图），保留完整身份比较
    /// （含 run_policy，漂移即 IdentityMismatch）。
    pub fn create_workspace_session_bound(
        &self,
        input: CreateWorkspaceSessionInput,
        id: String,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let root = self.workspace_sessions_root(&input.project_id, &input.issue_id);
        let target_path = root.join(format!("{id}.json"));
        if path_exists(&target_path)? {
            let existing: WorkspaceSessionRecord = read_json(&target_path)?;
            let expected_options = input.work_item_plan_options.clone().unwrap_or_default();
            if existing.id == id
                && existing.project_id == input.project_id
                && existing.issue_id == input.issue_id
                && existing.entity_id == input.entity_id
                && existing.workspace_type == input.workspace_type
                && existing.author_provider == input.author_provider
                && existing.reviewer_provider == input.reviewer_provider
                && existing.review_rounds == input.review_rounds
                && existing.superpowers_enabled == input.superpowers_enabled
                && existing.openspec_enabled == input.openspec_enabled
                && existing.flow_kind == expected_options.flow_kind
                && existing.run_policy == expected_options.run_policy
            {
                return Ok(existing);
            }
            return Err(ProductStoreError::IdentityMismatch {
                kind: "workspace_session",
                id,
            });
        }
        self.create_workspace_session_with_id(input, id)
    }

    /// Creates the interactive successor for a resumable stopped WorkItemPlan
    /// session while leaving the terminal parent record untouched. The durable
    /// event makes a repeated explicit takeover idempotent.
    pub fn takeover_stopped_needs_human(
        &self,
        parent_session_id: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(parent_session_id)?;
        let parent = self.get_workspace_session(parent_session_id)?;
        if parent.workspace_type != WorkspaceType::WorkItemPlan
            || parent.status != WorkspaceSessionStatus::StoppedNeedsHuman
            || parent.run_policy != crate::product::work_item_plan_policy::RunPolicy::AutoIfValid
            || !parent
                .human_gate_snapshot
                .as_ref()
                .is_some_and(|snapshot| snapshot.resumable)
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "human_gate_takeover",
                reason: "parent must be a resumable stopped work_item_plan session".to_string(),
            });
        }
        if parent
            .policy_diagnostics
            .iter()
            .any(|diagnostic| Self::is_fatal_takeover_diagnostic(&diagnostic.code))
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "human_gate_takeover",
                reason: "parent contains a fatal or persistence diagnostic".to_string(),
            });
        }

        let event_id = format!("human_gate_takeover_{}", parent.id);
        let event_path = self
            .paths
            .issue_lifecycle_root(&parent.project_id, &parent.issue_id)
            .join("human-gate-takeovers")
            .join(format!("{event_id}.json"));
        let parent_scope = self.entity_scope_for_session(&parent)?;
        self.with_optional_entity_live_write(parent_scope.as_ref(), || {
            with_exclusive_lock(&event_path, || {
                if path_exists(&event_path)? {
                    let event: HumanGateTakeoverEvent = read_json(&event_path)?;
                    if event.id != event_id || event.parent_session_id != parent.id {
                        return Err(ProductStoreError::IdentityMismatch {
                            kind: "human_gate_takeover",
                            id: event_id.clone(),
                        });
                    }
                    return self.get_workspace_session(&event.child_session_id);
                }

                let child_id = format!("workspace_session_takeover_{}", parent.id);
                let mut child = self.create_workspace_session_with_id(
                    CreateWorkspaceSessionInput {
                        project_id: parent.project_id.clone(),
                        issue_id: parent.issue_id.clone(),
                        entity_id: parent.entity_id.clone(),
                        workspace_type: WorkspaceType::WorkItemPlan,
                        author_provider: parent.author_provider.clone(),
                        reviewer_provider: parent.reviewer_provider.clone(),
                        review_rounds: parent.review_rounds,
                        superpowers_enabled: parent.superpowers_enabled,
                        openspec_enabled: parent.openspec_enabled,
                        work_item_plan_options: Some(super::WorkItemPlanSessionOptions {
                            flow_kind: parent.flow_kind,
                            run_policy:
                                crate::product::work_item_plan_policy::RunPolicy::Interactive,
                            rollout_snapshot: false,
                        }),
                    },
                    child_id,
                )?;

                // A stopped auto session is the immutable source of truth for the
                // interactive successor. Copy only resumable context and remaining
                // budget; never copy a provider-start or in-flight reservation.
                child.status = WorkspaceSessionStatus::WaitingForHuman;
                child.permission_modes = parent.permission_modes.clone();
                child.provisional_reviewer_provider = parent.provisional_reviewer_provider.clone();
                child.reviewer_enabled_at_start = parent.reviewer_enabled_at_start;
                child.run_history = parent.run_history.clone();
                child.review_invocation_scope = parent.review_invocation_scope.clone();
                child.human_gate_snapshot = parent.human_gate_snapshot.clone();
                child.policy_diagnostics = parent.policy_diagnostics.clone();
                child.single_candidate_phase = parent.single_candidate_phase;
                child.work_item_plan_source_revision_ref =
                    parent.work_item_plan_source_revision_ref.clone();
                child.plan_candidate_ir_ref = parent.plan_candidate_ir_ref.clone();
                child.mechanical_report_ref = parent.mechanical_report_ref.clone();
                child.publication_provenance_ref = parent.publication_provenance_ref.clone();
                child.messages = parent.messages.clone();
                child.provider_conversations = parent.provider_conversations.clone();
                child.repair_reservation = None;
                child.human_gate_reservation = None;
                child.provider_start_ledger = Vec::new();
                child.updated_at = Utc::now().to_rfc3339();
                let child_path = self
                    .workspace_sessions_root(&parent.project_id, &parent.issue_id)
                    .join(format!("{}.json", child.id));
                write_json(&child_path, &child)?;

                let event = HumanGateTakeoverEvent {
                    id: event_id,
                    parent_session_id: parent.id.clone(),
                    child_session_id: child.id.clone(),
                    created_at: Utc::now().to_rfc3339(),
                };
                write_json(&event_path, &event)?;
                Ok(child)
            })
        })
    }

    pub fn get_human_gate_takeover_event(
        &self,
        parent_session_id: &str,
    ) -> Result<Option<HumanGateTakeoverEvent>, ProductStoreError> {
        let parent = self.get_workspace_session(parent_session_id)?;
        let event_id = format!("human_gate_takeover_{}", parent.id);
        let event_path = self
            .paths
            .issue_lifecycle_root(&parent.project_id, &parent.issue_id)
            .join("human-gate-takeovers")
            .join(format!("{event_id}.json"));
        if !path_exists(&event_path)? {
            return Ok(None);
        }
        let event: HumanGateTakeoverEvent = read_json(&event_path)?;
        if event.id != event_id || event.parent_session_id != parent.id {
            return Err(ProductStoreError::IdentityMismatch {
                kind: "human_gate_takeover",
                id: event_id,
            });
        }
        Ok(Some(event))
    }

    fn is_fatal_takeover_diagnostic(code: &str) -> bool {
        matches!(
            code,
            "transition_budget_exhausted"
                | "unknown_category"
                | "unknown_class_hint"
                | "invalid_structured_output"
                | "state_corruption"
                | "protocol_violation"
                | "persistence_failure"
                | "safety_invariant_violation"
        )
    }

    pub fn list_workspace_sessions(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<WorkspaceSessionRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_workspace_session_records(&self.workspace_sessions_root(project_id, issue_id))
    }

    pub fn list_workspace_session_summaries(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<WorkspaceSessionSummaryRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        let entries =
            workspace_session_file_paths(&self.workspace_sessions_root(project_id, issue_id))?;
        let mut records = Vec::with_capacity(entries.len());
        for entry in entries {
            records.push(read_json(&entry)?);
        }
        Ok(records)
    }

    pub fn get_workspace_session(
        &self,
        session_id: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        #[cfg(test)]
        {
            super::workspace_session_read_spy::record_workspace_session_read();
        }
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let record: WorkspaceSessionRecord = read_json(&session_path)?;
        // REQ-ENV-13（设计 A 条目 17）+ P2-4：存量会话（旧 JSON 无身份字段）
        // 首读补生成一个身份并持久化一次（set 幂等不覆盖已有值）；后续
        // 读取复用持久值，MUST NOT 每次重新生成。
        //
        // P2-4 两条约束：
        // ① 补写门禁到多仓（lc）形态——单仓不消费该身份
        //    （`reviewer_verify_policy` 单仓恒 `None`），会话读取必须保持只读：
        //    不写盘、不刷新 `updated_at`（避免会话列表按 updated_at 重排）。
        // ② 补写是读路径的附带动作——写失败（只读盘 / 非 live scope）只降级为
        //    「本次返回记录」，绝不上浮成读错误，也不返回未持久化的随机身份。
        if record.review_identity_id.is_empty() && self.is_logical_codebase_session(&record) {
            let identity_id = uuid::Uuid::new_v4().simple().to_string();
            match self.set_workspace_session_review_identity(session_id, &identity_id) {
                Ok(()) => return read_json(&session_path),
                Err(error) => {
                    tracing::warn!(
                        session_id,
                        error = %error,
                        "workspace session review identity backfill failed; serving record as read"
                    );
                }
            }
        }
        Ok(record)
    }

    /// P2-4：会话所属 issue 是否为多仓（lc）形态——判据是 issue 记录存在
    /// （归属字段已必填，读得到即归属）。issue 记录缺失或不可判定
    /// 一律按单仓处理（单仓不需该身份，读路径保持只读）。
    fn is_logical_codebase_session(&self, session: &WorkspaceSessionRecord) -> bool {
        crate::product::logical_codebase::resolve_issue_logical_codebase_id(
            &self.paths,
            &session.project_id,
            &session.issue_id,
        )
        .is_ok()
    }

    pub fn ensure_work_item_runtime_binding(
        &self,
        session_id: &str,
        binding: &WorkItemRuntimeBinding,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            with_exclusive_lock(&session_path, || {
                let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
                if session.workspace_type != WorkspaceType::WorkItem {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "workspace_session_type",
                        id: session.id.clone(),
                    });
                }
                match &session.work_item_runtime_binding {
                    Some(existing) if existing == binding => Ok(session),
                    Some(_) => Err(ProductStoreError::IdentityMismatch {
                        kind: "work_item_runtime_binding",
                        id: session.id.clone(),
                    }),
                    None => {
                        session.work_item_runtime_binding = Some(binding.clone());
                        session.updated_at = Utc::now().to_rfc3339();
                        write_json(&session_path, &session)?;
                        Ok(session)
                    }
                }
            })
        })
    }

    pub fn append_workspace_message(
        &self,
        session_id: &str,
        role: String,
        content: String,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            let now = Utc::now().to_rfc3339();
            session.messages.push(WorkspaceMessageRecord {
                role,
                content,
                created_at: now.clone(),
            });
            session.updated_at = now;
            write_json(&session_path, &session)?;
            Ok(session)
        })
    }

    pub fn replace_workspace_messages(
        &self,
        session_id: &str,
        messages: Vec<WorkspaceMessageRecord>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            session.messages = messages;
            session.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &session)?;
            Ok(session)
        })
    }

    pub fn replace_workspace_provider_conversations(
        &self,
        session_id: &str,
        provider_conversations: Vec<ProviderConversationRef>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            session.provider_conversations = provider_conversations;
            session.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &session)?;
            Ok(session)
        })
    }

    pub fn update_workspace_session_status(
        &self,
        session_id: &str,
        status: WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            // 批 2 T2（REQ-WIGA-09）：Terminated 是终止终态——run 取消后的
            // 迟到 Failed/Open 收口写不得覆盖；再次写 Terminated 幂等保留。
            if session.status == WorkspaceSessionStatus::Terminated
                && status != WorkspaceSessionStatus::Terminated
            {
                return Ok(session);
            }
            session.status = status;
            if matches!(
                session.status,
                WorkspaceSessionStatus::Confirmed
                    | WorkspaceSessionStatus::Terminated
                    | WorkspaceSessionStatus::Failed
            ) {
                session.human_gate_snapshot = None;
            }
            session.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &session)?;
            Ok(session)
        })
    }

    /// Atomically claims the human-gate close operation.  The expected record
    /// check is the single-flight boundary between a stale websocket and the
    /// worker that already closed or advanced this gate.
    ///
    /// 人工权威升级（3.6 矩阵族⑤，oracle 裁决 A）：门以 Evaluate 相位开门时
    /// （request-change 修订后 repeated_fingerprint 门经 EnterHumanGate 保持
    /// Evaluate），confirm 是人工批准权威，在锁内判等通过后原子提升
    /// phase→Approval 再走既有 compile 链——compile 链三处 Approval 前置靠满足
    ///
    /// F-21 fix round（controller 裁定）：terminate（Terminated）对 plan 会话
    ///（SC 流）非终审门形态放行——context blocker 门（prepare 相位，plan_outline/
    /// authoring.rs enter_work_item_plan_context_blocker）与 author 连续 validate
    /// 失败门（generate 相位，decisions.rs）只置 stage+WaitingForHuman，不写快照
    /// 不提相位，WaitingForHuman 本身即这两门的 durable 开态证据；引擎
    /// close_human_gate 已在内存校验 stage==HumanConfirm 后才抵达本 CAS，矩阵
    /// HumanConfirm SC 臂放行 AbandonHumanGate。confirm（Running）前置一字不动。
    ///
    /// F-54 fix round：`Failed` 相位加入 terminate 放行面——compile 失败残留
    /// 相位 × 门重开 WaitingForHuman（0009 形态）此前 abandon 也被拒＝全通路
    /// 死锁。fail-closed 只锁 confirm（防未授权推进），永不锁 terminate（用户
    /// 脱困权）；confirm 授权面 {Approval, Evaluate} 一字不动。
    pub fn compare_and_save_human_gate_close(
        &self,
        expected: &WorkspaceSessionRecord,
        status: WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let gate_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(gate_scope.as_ref(), move || {
            if expected.workspace_type != WorkspaceType::WorkItemPlan
                || expected.flow_kind
                    != crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
            {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "human_gate_close",
                    reason: "session is not a SingleCandidate WorkItemPlan".to_string(),
                });
            }
            let relaxed_terminate = status == WorkspaceSessionStatus::Terminated
                && matches!(
                    expected.single_candidate_phase,
                    None | Some(crate::product::models::SingleCandidatePhase::Prepare)
                        | Some(crate::product::models::SingleCandidatePhase::Generate)
                        // F-54 B3：失败残留相位的开门也必须可终止（用户脱困权）。
                        | Some(crate::product::models::SingleCandidatePhase::Failed)
                );
            if expected.status != WorkspaceSessionStatus::WaitingForHuman
                || !(relaxed_terminate
                    || (matches!(
                        expected.single_candidate_phase,
                        Some(crate::product::models::SingleCandidatePhase::Approval)
                            | Some(crate::product::models::SingleCandidatePhase::Evaluate)
                    ) && expected.human_gate_snapshot.is_some()))
            {
                return Err(ProductStoreError::Conflict {
                    kind: "human_gate_close",
                    id: expected.id.clone(),
                });
            }
            let session_path = self.find_workspace_session_path(&expected.id)?;
            let locked_session_path = session_path.clone();
            with_exclusive_lock(&session_path, move || {
                let mut stored: WorkspaceSessionRecord = read_json(&locked_session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::Conflict {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                stored.status = status.clone();
                if status == WorkspaceSessionStatus::Running {
                    // confirm（Running）＝人工批准权威：锁内原子提升相位，
                    // Evaluate 进门 × Approval 关门死锁在此解开。
                    stored.single_candidate_phase =
                        Some(crate::product::models::SingleCandidatePhase::Approval);
                }
                if matches!(
                    status,
                    WorkspaceSessionStatus::Confirmed | WorkspaceSessionStatus::Terminated
                ) {
                    stored.human_gate_snapshot = None;
                    stored.human_gate_reservation = None;
                }
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&locked_session_path, &stored)?;
                Ok(stored)
            })
        })
    }

    /// C1 Task 4（REQ-C1-GATE-01）：候选恢复评估 label 的原子合并写。
    /// CAS 判据与 `compare_and_save_policy_route` 一致（整记录相等，
    /// single-flight）；只改写 `human_gate_snapshot.candidate_recovery`，
    /// 不触碰 status/phase/预算/ledger/turn。门快照缺席时 fail-closed。
    pub fn compare_and_save_candidate_recovery(
        &self,
        expected: &WorkspaceSessionRecord,
        recovery: crate::product::work_item_plan_policy::CandidateSnapshotRecovery,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let recovery_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(recovery_scope.as_ref(), move || {
            if expected.human_gate_snapshot.is_none() {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "candidate_recovery",
                    reason: "human gate snapshot is missing".to_string(),
                });
            }
            let session_path = self.find_workspace_session_path(&expected.id)?;
            let locked_session_path = session_path.clone();
            with_exclusive_lock(&session_path, move || {
                let mut stored: WorkspaceSessionRecord = read_json(&locked_session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::Conflict {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                let Some(snapshot) = stored.human_gate_snapshot.as_mut() else {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "candidate_recovery",
                        reason: "human gate snapshot is missing".to_string(),
                    });
                };
                snapshot.candidate_recovery = Some(recovery);
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&locked_session_path, &stored)?;
                Ok(stored)
            })
        })
    }

    /// Atomically claims a provider-start idempotency key. A key can be claimed
    /// only once, and the durable ledger is the source of truth during recovery.
    pub fn claim_provider_start(
        &self,
        session_id: &str,
        provider_start_idempotency_key: &str,
    ) -> Result<bool, ProductStoreError> {
        self.claim_provider_start_with_details(
            session_id,
            provider_start_idempotency_key,
            None,
            None,
        )
    }

    /// F-19：带诊断详情的 provider start 登记（story/design 等 legacy 流专用，
    /// 对照 SC 面 `reserve_single_candidate_provider_start` 先例）。与
    /// `claim_provider_start` 同语义：同 key 幂等（重复 claim 返回 false）、
    /// 排它锁下原子落盘；条目额外携带 provider 名与登记时间，供监控区分
    /// 「start 未发生」（ledger 空）与「start 后楔死」（有登记无后续事件）。
    pub fn claim_provider_start_with_details(
        &self,
        session_id: &str,
        provider_start_idempotency_key: &str,
        provider: Option<&str>,
        started_at: Option<String>,
    ) -> Result<bool, ProductStoreError> {
        validate_relative_id(session_id)?;
        if provider_start_idempotency_key.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "provider_start_ledger",
                reason: "idempotency key must not be empty".to_string(),
            });
        }
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            with_exclusive_lock(&session_path, || {
                let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
                if session.provider_start_ledger.iter().any(|entry| {
                    entry.provider_start_idempotency_key == provider_start_idempotency_key
                }) {
                    return Ok(false);
                }
                session.provider_start_ledger.push(
                    crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                        provider_start_idempotency_key: provider_start_idempotency_key.to_string(),
                        started: true,
                        provider: provider.map(ToString::to_string),
                        started_at,
                        provider_attempt_key: None,
                    },
                );
                session.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &session)?;
                Ok(true)
            })
        })
    }

    /// A2a（Task 2.5）：LC resume 状态迁移的整记录 expected CAS。锁内重读
    /// 最新 record，仅替换 `lc_resume_recovery` 字段——非 LC 字段（人审
    /// 快照/预算/ledger/tombstone 等）以锁内 stored 为准保留，绝不用
    /// caller 侧过期副本覆盖；任何并发写都令整记录比较冲突
    /// （single-flight）。后续新漂移替换 recovery 时传新 recovery_id 的
    /// 完整记录。
    pub fn compare_and_save_lc_resume_state(
        &self,
        expected: &WorkspaceSessionRecord,
        next_lc_fields: LcResumeRecoveryRecord,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let resume_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(resume_scope.as_ref(), move || {
            let session_path = self.find_workspace_session_path(&expected.id)?;
            let locked_session_path = session_path.clone();
            with_exclusive_lock(&session_path, move || {
                let mut stored: WorkspaceSessionRecord = read_json(&locked_session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::Conflict {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                stored.lc_resume_recovery = Some(next_lc_fields);
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&locked_session_path, &stored)?;
                Ok(stored)
            })
        })
    }

    /// A2a（Task 2.5，设计 §5.1 固定判定顺序）：显式 fresh 的 durable
    /// claim。store 层落点：① recovery 身份（record 缺失 recovery 或
    /// recovery_id 已被新漂移替换 → Stale）② 同 ID 已受理先 Replay——
    /// waiting/prepare 失败、reserved/running、resolved/completed 一律回放
    /// 原受理结果＋当前操作状态，禁止 spawn，且该查询先于一切
    /// waiting/busy/resolved 判定 ③ 仅新 operation_id 才按状态机分类：
    /// FreshReserved/FreshRunning → Busy、Resolved → Stale（不造第二会
    /// 话）、Waiting → 排它锁下原子 CAS 受理（recovery → FreshReserved，
    /// fresh_operations 追加 Reserved 结果）。已受理 operation 的 Replay
    /// 只经本 store 文件锁读 durable 事实，不等待 engine mutex；write
    /// 失败整体 Err，不残留半受理状态（跨文件不承诺事务）。
    pub fn claim_lc_fresh(
        &self,
        session_id: &str,
        recovery_id: &str,
        operation_id: &str,
    ) -> Result<LcFreshClaimOutcome, ProductStoreError> {
        validate_relative_id(session_id)?;
        if recovery_id.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "lc_resume_recovery_claim",
                reason: "recovery id must not be empty".to_string(),
            });
        }
        if operation_id.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "lc_resume_recovery_claim",
                reason: "operation id must not be empty".to_string(),
            });
        }
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            with_exclusive_lock(&session_path, || {
                let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
                // ① recovery 身份：缺失/被替换 → Stale。
                let Some(recovery) = session.lc_resume_recovery.as_ref() else {
                    return Ok(LcFreshClaimOutcome::Stale {
                        reason_code: LcFreshClaimOutcome::STALE_NO_RECOVERY.to_string(),
                    });
                };
                if recovery.recovery_id != recovery_id {
                    return Ok(LcFreshClaimOutcome::Stale {
                        reason_code: LcFreshClaimOutcome::STALE_SUPERSEDED.to_string(),
                    });
                }
                // ② 同 ID 已受理先 Replay：先于一切 waiting/busy/resolved 判定，
                //    只读 durable 事实，不 spawn、不等待 engine mutex。
                if let Some(operation) = recovery
                    .fresh_operations
                    .iter()
                    .find(|operation| operation.operation_id == operation_id)
                {
                    return Ok(LcFreshClaimOutcome::Replay {
                        operation: operation.clone(),
                        recovery_state: recovery.state,
                    });
                }
                // ③ 仅新 operation_id 才按 recovery 状态机分类。
                match recovery.state {
                    LcResumeRecoveryState::FreshReserved | LcResumeRecoveryState::FreshRunning => {
                        return Ok(LcFreshClaimOutcome::Busy {
                            active_operation_id: recovery.operation_id.clone(),
                        });
                    }
                    LcResumeRecoveryState::Resolved => {
                        return Ok(LcFreshClaimOutcome::Stale {
                            reason_code: LcFreshClaimOutcome::STALE_RESOLVED.to_string(),
                        });
                    }
                    LcResumeRecoveryState::Waiting => {}
                }
                let now = Utc::now().to_rfc3339();
                let operation = LcFreshOperationResult {
                    operation_id: operation_id.to_string(),
                    state: LcFreshOperationState::Reserved,
                    fresh_node_id: None,
                    reason_code: None,
                    role_run_seq: None,
                    accepted_at: now.clone(),
                    updated_at: now.clone(),
                };
                let recovery = session
                    .lc_resume_recovery
                    .as_mut()
                    .expect("lc resume recovery presence checked above");
                recovery.state = LcResumeRecoveryState::FreshReserved;
                recovery.operation_id = Some(operation_id.to_string());
                recovery.fresh_operations.push(operation.clone());
                recovery.updated_at = now.clone();
                session.updated_at = now;
                write_json(&session_path, &session)?;
                Ok(LcFreshClaimOutcome::Claimed {
                    record: session,
                    operation,
                })
            })
        })
    }

    /// A2a（Task 2.5）：已受理 fresh operation 的只读查询（§5.1：Replay
    /// 对账不等待 engine mutex）。recovery 身份不匹配或 operation 未受理
    /// 返回 None；不写任何字段，不与 claim 事务耦合。
    pub fn query_lc_fresh_operation(
        &self,
        session_id: &str,
        recovery_id: &str,
        operation_id: &str,
    ) -> Result<Option<LcFreshOperationResult>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let session: WorkspaceSessionRecord = read_json(&session_path)?;
        let Some(recovery) = session.lc_resume_recovery else {
            return Ok(None);
        };
        if recovery.recovery_id != recovery_id {
            return Ok(None);
        }
        Ok(recovery
            .fresh_operations
            .into_iter()
            .find(|operation| operation.operation_id == operation_id))
    }

    pub fn compare_and_update_workspace_session_status(
        &self,
        expected: &WorkspaceSessionRecord,
        status: WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(&expected.id)?;
        let cas_scope = self.entity_scope_for_session(expected)?;
        self.with_optional_entity_live_write(cas_scope.as_ref(), move || {
            let session_path = self.find_workspace_session_path(&expected.id)?;
            with_exclusive_lock(&session_path, || {
                let mut stored: WorkspaceSessionRecord = read_json(&session_path)?;
                if stored != *expected {
                    return Err(ProductStoreError::IdentityMismatch {
                        kind: "workspace_session",
                        id: expected.id.clone(),
                    });
                }
                if stored.status == status {
                    return Ok(stored);
                }
                // 批 2 T2（REQ-WIGA-09）：CAS Terminated 不覆盖已 Confirmed——
                // 人工确认是比终止 CAS 更晚的最终事实，幂等保留 Confirmed。
                if status == WorkspaceSessionStatus::Terminated
                    && stored.status == WorkspaceSessionStatus::Confirmed
                {
                    return Ok(stored);
                }
                stored.status = status;
                stored.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &stored)?;
                Ok(stored)
            })
        })
    }

    pub fn update_workspace_session_providers(
        &self,
        session_id: &str,
        author_provider: crate::product::models::ProviderName,
        reviewer_provider: Option<crate::product::models::ProviderName>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            session.author_provider = author_provider;
            session.reviewer_provider = reviewer_provider;
            session.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &session)?;
            Ok(session)
        })
    }

    pub fn update_workspace_session_permission_modes(
        &self,
        session_id: &str,
        permission_modes: WorkspaceRolePermissionModes,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            session.permission_modes = permission_modes;
            session.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &session)?;
            Ok(session)
        })
    }

    pub fn update_workspace_session_provisional_reviewer(
        &self,
        session_id: &str,
        provisional_reviewer_provider: Option<crate::product::models::ProviderName>,
        reviewer_enabled_at_start: Option<bool>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            session.provisional_reviewer_provider = provisional_reviewer_provider;
            session.reviewer_enabled_at_start = reviewer_enabled_at_start;
            session.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &session)?;
            Ok(session)
        })
    }

    /// REQ-ENV-13（设计 A 条目 17）：写入会话评审身份——幂等：已有值
    /// 不覆盖（并发首读竞态下只落第一个）；仅在字段为空时写盘。身份
    /// 与会话记录同文件，会话删除即解绑，无独立映射存储需清理。
    pub fn set_workspace_session_review_identity(
        &self,
        session_id: &str,
        identity_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            if session.review_identity_id.is_empty() {
                session.review_identity_id = identity_id.to_string();
                session.updated_at = Utc::now().to_rfc3339();
                write_json(&session_path, &session)?;
            }
            Ok(())
        })
    }

    pub fn truncate_workspace_session_messages(
        &self,
        session_id: &str,
        keep_count: usize,
        status: WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let (session_path, scope) = self.resolve_session_scope(session_id)?;
        self.with_optional_entity_live_write(scope.as_ref(), move || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            session.messages.truncate(keep_count);
            session.status = status;
            session.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &session)?;
            Ok(session)
        })
    }
}

include!("workspace_timeline_artifacts.inc.rs");

impl LifecycleStore {
    fn next_workspace_session_id(&self) -> Result<String, ProductStoreError> {
        // D1.3a：全项目 session 最大序号与耐久高水位取 max 再增（D1.0
        // `allocate_workspace_session_id`）；已分配（含已删实体）ID 不复用。
        self.entity_operation_store()
            .allocate_workspace_session_id()
    }

    pub(super) fn find_workspace_session_path(
        &self,
        session_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_file_name = format!("{session_id}.json");
        let mut matched_path = None;
        for project_path in child_directories(&self.paths.projects_root())? {
            let issues_root = project_path.join("issues");
            for issue_path in child_directories(&issues_root)? {
                let session_path = issue_path
                    .join("workspace-sessions")
                    .join(&session_file_name);
                if !path_is_regular_file(&session_path)? {
                    continue;
                }
                let Some(session) = read_workspace_session_record(&session_path)? else {
                    continue;
                };
                if session.id != session_id {
                    continue;
                }
                if matched_path.is_some() {
                    return Err(ProductStoreError::Io(
                        "workspace_session_ambiguous".to_string(),
                    ));
                }
                matched_path = Some(session_path);
            }
        }

        matched_path.ok_or_else(|| ProductStoreError::NotFound {
            kind: "workspace_session",
            id: session_id.to_string(),
        })
    }

    pub(crate) fn workspace_timeline_root_for_session(
        &self,
        session_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        let session_path = self.find_workspace_session_path(session_id)?;
        let sessions_root = session_path.parent().ok_or_else(|| {
            ProductStoreError::Io(format!(
                "workspace session path has no parent: {}",
                session_path.display()
            ))
        })?;
        let issue_root = sessions_root.parent().ok_or_else(|| {
            ProductStoreError::Io(format!(
                "workspace sessions path has no issue parent: {}",
                sessions_root.display()
            ))
        })?;
        Ok(issue_root.join("workspace-timelines").join(session_id))
    }

    pub(crate) fn workspace_timeline_root_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(session_id)?;
        Ok(self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-timelines")
            .join(session_id))
    }
}

// ==== 实体 scope 准入（fix-cascade-deletion Task D1.3a；plan §0.2/§0.4）====
//
// 生命周期/session 写入口的迟到写防御：先按真实归属归约出 `EntityScope`，
// 在任何 session/对象锁之前取得 scope 共享准入（try-shared＋锁内墓碑复核，
// 由 `EntityOperationStore::with_live_write` 提供），purging/deleted 一律稳定
// 拒绝 `Conflict{entity_deleting}`；无法归属任何存活实体的旧档案（无 owner
// refs）保持原行为，不引入第二套授权。锁序遵守 §0.2 固定序：scope →
// resource → 关系短锁 → session/对象锁。

thread_local! {
    /// 本线程已取得 scope 共享准入的实体集合：同一外层准入内的组合流程
    /// （confirm→WI 状态、SC 门修订→artifact/回滚等）内层写不再重复取
    /// scope 锁（plan §0.4：内层只消费已持 lease，嵌套取 scope 不当合法设计）。
    static ADMITTED_ENTITY_SCOPES: RefCell<HashSet<EntityScope>> =
        RefCell::new(HashSet::new());
}

/// 单个已准入 scope 的线程级登记守卫（正常返回/Err/panic 均撤销登记）。
struct AdmittedEntityScope(EntityScope);

impl Drop for AdmittedEntityScope {
    fn drop(&mut self) {
        ADMITTED_ENTITY_SCOPES.with(|admitted| {
            admitted.borrow_mut().remove(&self.0);
        });
    }
}

impl LifecycleStore {
    pub(super) fn entity_operation_store(&self) -> EntityOperationStore {
        EntityOperationStore::new(self.paths.clone())
    }

    /// scope 共享准入（try-shared＋锁内墓碑复核）；本线程已在同一外层准入
    /// 内持有该 scope 时不重复获取，直接执行（消费已持 lease）。
    pub(super) fn with_entity_live_write<T>(
        &self,
        scope: &EntityScope,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let already_admitted =
            ADMITTED_ENTITY_SCOPES.with(|admitted| admitted.borrow().contains(scope));
        if already_admitted {
            return operation();
        }
        ADMITTED_ENTITY_SCOPES.with(|admitted| admitted.borrow_mut().insert(scope.clone()));
        let _guard = AdmittedEntityScope(scope.clone());
        self.entity_operation_store()
            .with_live_write(scope, operation)
    }

    /// 无法归属实体的旧档案保持原行为（None → 不准入，非授权放宽）。
    pub(super) fn with_optional_entity_live_write<T>(
        &self,
        scope: Option<&EntityScope>,
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        match scope {
            Some(scope) => self.with_entity_live_write(scope, operation),
            None => operation(),
        }
    }

    /// 多 scope 准入（外层按 kind/project/issue/entity 固定排序、去重后
    /// 嵌套获取；关联创建同时核自身与上游 scope，plan §0.2）。
    pub(super) fn with_entity_live_writes<T>(
        &self,
        scopes: &[EntityScope],
        operation: impl FnOnce() -> Result<T, ProductStoreError>,
    ) -> Result<T, ProductStoreError> {
        let mut ordered: Vec<EntityScope> = scopes.to_vec();
        ordered.sort_by(|left, right| {
            (
                left.kind.path_segment(),
                left.project_id.as_str(),
                left.issue_id.as_str(),
                left.entity_id.as_str(),
            )
                .cmp(&(
                    right.kind.path_segment(),
                    right.project_id.as_str(),
                    right.issue_id.as_str(),
                    right.entity_id.as_str(),
                ))
        });
        ordered.dedup();

        fn nested<T>(
            store: &LifecycleStore,
            scopes: &[EntityScope],
            operation: &mut dyn FnMut() -> Result<T, ProductStoreError>,
        ) -> Result<T, ProductStoreError> {
            let Some((first, rest)) = scopes.split_first() else {
                return operation();
            };
            store.with_entity_live_write(first, || nested(store, rest, operation))
        }
        let mut operation = Some(operation);
        nested(self, &ordered, &mut || {
            (operation
                .take()
                .expect("entity admission operation invoked once"))()
        })
    }

    /// session 记录 → 拥有实体 scope 归约（真实 parent/Plan binding，
    /// plan §0.5 WI/session 行）：Story/Design/WorkItemPlan 直接映射；
    /// WorkItem session 先 child_binding.plan_id，再 WI 记录
    /// source_work_item_plan_id，最后按 issue 计划 work_item_ids 反查；
    /// 全部来源耗尽（旧无 owner 档案）→ None（不误伤，也不当 live 授权）。
    pub(super) fn entity_scope_for_session(
        &self,
        session: &WorkspaceSessionRecord,
    ) -> Result<Option<EntityScope>, ProductStoreError> {
        match &session.workspace_type {
            WorkspaceType::Story => Ok(Some(EntityScope::new(
                &session.project_id,
                &session.issue_id,
                DeletionEntityKind::Story,
                &session.entity_id,
            ))),
            WorkspaceType::Design => Ok(Some(EntityScope::new(
                &session.project_id,
                &session.issue_id,
                DeletionEntityKind::Design,
                &session.entity_id,
            ))),
            WorkspaceType::WorkItemPlan => Ok(Some(EntityScope::new(
                &session.project_id,
                &session.issue_id,
                DeletionEntityKind::Plan,
                &session.entity_id,
            ))),
            WorkspaceType::WorkItem => {
                if let Some(binding) = &session.work_item_child_binding {
                    return Ok(Some(EntityScope::new(
                        &session.project_id,
                        &session.issue_id,
                        DeletionEntityKind::Plan,
                        &binding.plan_id,
                    )));
                }
                self.owning_plan_scope(&session.project_id, &session.issue_id, &session.entity_id)
            }
        }
    }

    /// 新建 session 的目标 scope（创建入口无已存记录可读，按输入归约）。
    fn entity_scope_for_new_session(
        &self,
        input: &CreateWorkspaceSessionInput,
        child_binding: Option<&ChildBindingIdentity>,
    ) -> Result<Option<EntityScope>, ProductStoreError> {
        match (&input.workspace_type, child_binding) {
            (WorkspaceType::Story, _) => Ok(Some(EntityScope::new(
                &input.project_id,
                &input.issue_id,
                DeletionEntityKind::Story,
                &input.entity_id,
            ))),
            (WorkspaceType::Design, _) => Ok(Some(EntityScope::new(
                &input.project_id,
                &input.issue_id,
                DeletionEntityKind::Design,
                &input.entity_id,
            ))),
            (WorkspaceType::WorkItemPlan, _) => Ok(Some(EntityScope::new(
                &input.project_id,
                &input.issue_id,
                DeletionEntityKind::Plan,
                &input.entity_id,
            ))),
            (WorkspaceType::WorkItem, Some(binding)) => Ok(Some(EntityScope::new(
                &input.project_id,
                &input.issue_id,
                DeletionEntityKind::Plan,
                &binding.plan_id,
            ))),
            (WorkspaceType::WorkItem, None) => {
                self.owning_plan_scope(&input.project_id, &input.issue_id, &input.entity_id)
            }
        }
    }

    /// WorkItem → 拥有 Plan 归约：WI 记录 source_work_item_plan_id 优先，
    /// 缺失时按 issue 计划 work_item_ids refs 反查（旧无 owner 档案按 refs
    /// 核归属，plan D1.3a Interfaces）。
    pub(super) fn owning_plan_scope(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<Option<EntityScope>, ProductStoreError> {
        // 直接构造路径：work_item_path 对缺失记录报 NotFound，而归约必须
        // 允许「档案缺失 → 无归属」的旧形态。
        let work_item_path = self
            .work_items_root(project_id, issue_id)
            .join(format!("{work_item_id}.json"));
        if path_is_regular_file(&work_item_path)? {
            let record: LifecycleWorkItemRecord = read_json(&work_item_path)?;
            if let Some(plan_id) = &record.source_work_item_plan_id {
                return Ok(Some(EntityScope::new(
                    project_id,
                    issue_id,
                    DeletionEntityKind::Plan,
                    plan_id,
                )));
            }
        }
        for plan in self.list_issue_work_item_plans(project_id, issue_id)? {
            if plan.work_item_ids.iter().any(|id| id == work_item_id) {
                return Ok(Some(EntityScope::new(
                    project_id,
                    issue_id,
                    DeletionEntityKind::Plan,
                    &plan.id,
                )));
            }
        }
        Ok(None)
    }

    /// session_id → (session 路径, scope)。只读解析，不持锁；随后写体在
    /// 准入内重读最新记录（先 find 后删除再 write 的竞态由准入闭合）。
    pub(super) fn resolve_session_scope(
        &self,
        session_id: &str,
    ) -> Result<(PathBuf, Option<EntityScope>), ProductStoreError> {
        let session_path = self.find_workspace_session_path(session_id)?;
        let session: WorkspaceSessionRecord = read_json(&session_path)?;
        let scope = self.entity_scope_for_session(&session)?;
        Ok((session_path, scope))
    }

    /// 宽松版：session 档案缺失（NotFound）→ None（原行为本就不读 session 的
    /// 写入口：link/repair state/review round——不因准入引入新 NotFound）。
    pub(super) fn resolve_session_scope_lenient(
        &self,
        session_id: &str,
    ) -> Result<Option<EntityScope>, ProductStoreError> {
        match self.resolve_session_scope(session_id) {
            Ok((_, scope)) => Ok(scope),
            Err(ProductStoreError::NotFound { .. }) => Ok(None),
            Err(other) => Err(other),
        }
    }

    /// issue 级实体 ID 分配：目录序号扫描跳过已有删除墓碑的 ID（创建新
    /// entity 跳过 tombstone ID，plan D1.3a Interfaces；已删实体 ID 不复用）。
    pub(super) fn next_fresh_entity_id(
        &self,
        prefix: &str,
        directory: &std::path::Path,
        kind: DeletionEntityKind,
        project_id: &str,
        issue_id: &str,
    ) -> Result<String, ProductStoreError> {
        let deletions = EntityDeletionStore::new(self.paths.clone());
        let mut id = next_sequential_id_in_directory(prefix, directory).map_err(|error| {
            ProductStoreError::Io(format!("read {}: {error}", directory.display()))
        })?;
        loop {
            let scope = EntityScope::new(project_id, issue_id, kind, &id);
            if deletions.get(&scope)?.is_none() {
                return Ok(id);
            }
            let sequence = id
                .rsplit_once('_')
                .and_then(|(_, suffix)| suffix.parse::<usize>().ok())
                .unwrap_or(0);
            id = next_sequential_id(prefix, sequence);
        }
    }

    /// WI 记录 → 拥有 Plan scope（D1.3a work_item 写入口消费）：
    /// source_work_item_plan_id 优先，缺失按 refs 反查。
    pub(super) fn entity_scope_for_work_item_record(
        &self,
        record: &LifecycleWorkItemRecord,
    ) -> Result<Option<EntityScope>, ProductStoreError> {
        if let Some(plan_id) = &record.source_work_item_plan_id {
            return Ok(Some(EntityScope::new(
                &record.project_id,
                &record.issue_id,
                DeletionEntityKind::Plan,
                plan_id,
            )));
        }
        self.owning_plan_scope(&record.project_id, &record.issue_id, &record.id)
    }
}

/// A2a（Task 2.5）：显式 fresh claim 的 typed 四态结果（设计 §5.1 固定
/// 判定顺序）。Replay 为结构变体：同 ID 已受理一律回放原受理结果＋当前
/// 操作状态（waiting/prepare 失败、reserved/running、resolved/completed
/// 全覆盖），禁止 spawn，卡不复活。
// 四态结果按值遍历/匹配;Claimed/Replayed 携带完整 record,Box 化会波及全调用链。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LcFreshClaimOutcome {
    /// 新 operation_id 在 recovery=Waiting 下原子 CAS 受理成功：
    /// recovery → FreshReserved，fresh_operations 追加 Reserved 结果。
    /// provider start/spawn 属于 engine 后续职责，不在 store 内发生。
    Claimed {
        record: WorkspaceSessionRecord,
        operation: LcFreshOperationResult,
    },
    /// 同 ID 已受理：原受理结果＋当前操作状态；只读 durable 事实回放，
    /// 不等待 engine mutex；resolved 同 ID 也只 Replay。
    Replay {
        operation: LcFreshOperationResult,
        recovery_state: LcResumeRecoveryState,
    },
    /// 新 ID 撞上 FreshReserved/FreshRunning（已受理 operation 优先，
    /// 不能只靠 UI disabled）。
    Busy { active_operation_id: Option<String> },
    /// recovery 缺失/已被新漂移替换，或已 resolved（新 ID 拒绝第二会
    /// 话）；reason_code 见本类型常量。
    Stale { reason_code: String },
}

impl LcFreshClaimOutcome {
    /// record 上无 lc_resume_recovery（尚无等待恢复可认领）。
    pub const STALE_NO_RECOVERY: &'static str = "lc_resume_recovery_required";
    /// recovery_id 已被新漂移替换（旧 ID 非权威身份）。
    pub const STALE_SUPERSEDED: &'static str = "lc_resume_recovery_stale";
    /// recovery 已 resolved：新 operation 拒绝，不造第二会话。
    pub const STALE_RESOLVED: &'static str = "lc_resume_recovery_resolved";
}

#[cfg(test)]
#[path = "workspace_tests.rs"]
mod tests;

// ---------------------------------------------------------------------------
// 批 2 T2（REQ-WIGA-09）：Terminated 终态防迟到覆盖与 Confirmed 保留。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod termination_tests {
    use super::*;
    use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
    use crate::product::models::ProviderName;
    use crate::product::models::WorkspaceType;

    fn session_fixture() -> (tempfile::TempDir, LifecycleStore, WorkspaceSessionRecord) {
        let tmp = tempfile::tempdir().unwrap();
        let store = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
            tmp.path().join(".aria"),
        ));
        let record = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .unwrap();
        (tmp, store, record)
    }

    /// 红测①：Terminated 粘性——run 取消后的迟到 Failed/Open 收口写不覆盖
    /// Terminated（终态保留）；写 Terminated 幂等。
    #[test]
    fn workspace_session_terminated_is_sticky_against_late_failed_writes() {
        let (_tmp, store, record) = session_fixture();
        let terminated = store
            .update_workspace_session_status(&record.id, WorkspaceSessionStatus::Terminated)
            .unwrap();
        assert_eq!(terminated.status, WorkspaceSessionStatus::Terminated);

        let late_failed = store
            .update_workspace_session_status(&record.id, WorkspaceSessionStatus::Failed)
            .unwrap();
        assert_eq!(
            late_failed.status,
            WorkspaceSessionStatus::Terminated,
            "late failed finish must not overwrite terminated"
        );
        let late_open = store
            .update_workspace_session_status(&record.id, WorkspaceSessionStatus::Open)
            .unwrap();
        assert_eq!(late_open.status, WorkspaceSessionStatus::Terminated);
        // 幂等：重复写 Terminated 保持。
        let again = store
            .update_workspace_session_status(&record.id, WorkspaceSessionStatus::Terminated)
            .unwrap();
        assert_eq!(again.status, WorkspaceSessionStatus::Terminated);
    }

    /// 红测②：已 Confirmed 不改——CAS Terminated 不覆盖 Confirmed（人工
    /// 确认是更晚的最终事实）；普通状态 CAS 回归不变。
    #[test]
    fn workspace_session_terminated_cas_preserves_confirmed() {
        let (_tmp, store, record) = session_fixture();
        let confirmed = store
            .update_workspace_session_status(&record.id, WorkspaceSessionStatus::Confirmed)
            .unwrap();
        let preserved = store
            .compare_and_update_workspace_session_status(
                &confirmed,
                WorkspaceSessionStatus::Terminated,
            )
            .unwrap();
        assert_eq!(
            preserved.status,
            WorkspaceSessionStatus::Confirmed,
            "termination CAS must not overwrite a confirmed plan session"
        );

        // 非 Confirmed 会话 CAS Terminated 正常（终止主路径回归）。
        let (_tmp2, store2, record2) = session_fixture();
        let terminated = store2
            .compare_and_update_workspace_session_status(
                &record2,
                WorkspaceSessionStatus::Terminated,
            )
            .unwrap();
        assert_eq!(terminated.status, WorkspaceSessionStatus::Terminated);
    }
}

/// P08「当前 operation 归属」RED 契约：显式 fresh 的**新** operation 受理
/// 时，root 上只属于当前 operation 的物理镜像（node/seq/native）必须清空，
/// 避免新 operation 复用旧物理 run；旧 operation 的历史事实（fresh_operations
/// 逐字段、状态、会话引用、launch 指纹、SC ledger/phase/教学计数）原样保留
/// （旧 Unknown 不被改 Failed）。
#[cfg(test)]
mod fresh_operation_binding_tests {
    use super::*;
    use crate::product::lifecycle_store::CreateWorkspaceSessionInput;
    use crate::product::models::{
        LcRecoveryBusinessContext, LcRecoveryRunContext, ProviderConversationRole, ProviderName,
    };

    const RECOVERY_ID: &str = "recovery_fresh_binding_0001";
    const OLD_NODE_ID: &str = "timeline_node_old_0001";
    const OLD_NATIVE_ID: &str = "native_old_0001";

    fn session_fixture() -> (tempfile::TempDir, LifecycleStore, WorkspaceSessionRecord) {
        let tmp = tempfile::tempdir().unwrap();
        let store = LifecycleStore::new(crate::product::app_paths::ProductAppPaths::new(
            tmp.path().join(".aria"),
        ));
        let record = store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: Some(ProviderName::Fake),
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("create workspace session");
        (tmp, store, record)
    }

    /// 旧 operation（Unknown）+ root 当前镜像（node/seq/native）的等待态。
    fn waiting_recovery_with_old_operation(
        record: &WorkspaceSessionRecord,
    ) -> LcResumeRecoveryRecord {
        let mut old_operation = LcFreshOperationResult {
            operation_id: "op_old_0001".to_string(),
            state: LcFreshOperationState::Unknown,
            fresh_node_id: Some(OLD_NODE_ID.to_string()),
            reason_code: Some("lc_resume_round_launch_unknown".to_string()),
            role_run_seq: Some(7),
            accepted_at: "2026-11-13T00:00:00Z".to_string(),
            updated_at: "2026-11-13T00:00:00Z".to_string(),
        };
        old_operation.updated_at = "2026-11-13T00:00:00Z".to_string();
        LcResumeRecoveryRecord {
            schema_version: 1,
            recovery_id: RECOVERY_ID.to_string(),
            failed_provider_node_id: OLD_NODE_ID.to_string(),
            reason_code: "resume_fingerprint_mismatch".to_string(),
            role: ProviderConversationRole::Author,
            provider: record.author_provider.clone(),
            superseded_native_session_id: Some(OLD_NATIVE_ID.to_string()),
            previous_fingerprint_digest: Some("sha256:old".to_string()),
            previous_session_status: record.status.clone(),
            previous_stage: "revision".to_string(),
            run_context: LcRecoveryRunContext::ReviewerRevision {
                business: LcRecoveryBusinessContext {
                    entity_id: record.entity_id.clone(),
                    plan_id: None,
                    workspace_type: record.workspace_type.clone(),
                    business_repository_physical_id: None,
                    business_repository_logical_id: None,
                    business_checkout_id: None,
                    business_repository_path: None,
                    source_story_ids: Vec::new(),
                    source_design_ids: Vec::new(),
                    plan_options_ref: None,
                    artifact_refs: Vec::new(),
                    review_refs: Vec::new(),
                    author_provider: record.author_provider.clone(),
                    effective_reviewer_provider: record.reviewer_provider.clone(),
                    original_node_id: Some(OLD_NODE_ID.to_string()),
                    original_role_run_seq: None,
                },
                original_feedback: None,
            },
            state: LcResumeRecoveryState::Waiting,
            operation_id: None,
            fresh_node_id: Some(OLD_NODE_ID.to_string()),
            fresh_native_session_id: Some(OLD_NATIVE_ID.to_string()),
            fresh_role_run_seq: Some(7),
            fresh_operations: vec![old_operation],
            detected_at: "2026-11-13T00:00:00Z".to_string(),
            updated_at: "2026-11-13T00:00:00Z".to_string(),
        }
    }

    /// RED①：新 operation 受理后，root 当前镜像三字段清空；旧 operation
    /// 历史逐字段不变（Unknown 不被改 Failed）；指纹/会话引用/SC ledger/
    /// phase/教学计数零改动。
    #[test]
    fn fresh_operation_binding_new_claim_clears_current_operation_mirror() {
        let (_tmp, store, record) = session_fixture();
        let recovery = waiting_recovery_with_old_operation(&record);
        let old_operation = recovery.fresh_operations[0].clone();
        store
            .compare_and_save_lc_resume_state(&record, recovery)
            .expect("seed waiting recovery");

        let outcome = store
            .claim_lc_fresh(&record.id, RECOVERY_ID, "op_new_0001")
            .expect("claim new operation");
        assert!(
            matches!(outcome, LcFreshClaimOutcome::Claimed { .. }),
            "新 operation 必须被受理，实际: {outcome:?}"
        );

        let after = store
            .get_workspace_session(&record.id)
            .expect("record after claim");
        let current = after
            .lc_resume_recovery
            .as_ref()
            .expect("recovery kept after claim");
        assert_eq!(current.state, LcResumeRecoveryState::FreshReserved);
        assert_eq!(current.operation_id.as_deref(), Some("op_new_0001"));
        // 当前 operation 镜像：旧物理 node/seq/native 不得投影到新 operation。
        assert_eq!(
            current.fresh_node_id, None,
            "新 operation 不得继承旧 fresh node 镜像"
        );
        assert_eq!(
            current.fresh_role_run_seq, None,
            "新 operation 不得继承旧 fresh role_run_seq 镜像"
        );
        assert_eq!(
            current.fresh_native_session_id, None,
            "新 operation 不得继承旧 native 镜像"
        );
        // 历史 operation 逐字段原样（Unknown 不被改 Failed）。
        assert_eq!(current.fresh_operations.len(), 2);
        assert_eq!(
            current.fresh_operations[0], old_operation,
            "旧 operation 历史必须逐字段原样保留"
        );
        assert_eq!(
            current.fresh_operations[0].state,
            LcFreshOperationState::Unknown,
            "旧 Unknown operation 不得被改 Failed"
        );
        let new_operation = &current.fresh_operations[1];
        assert_eq!(new_operation.operation_id, "op_new_0001");
        assert_eq!(new_operation.state, LcFreshOperationState::Reserved);
        assert_eq!(new_operation.fresh_node_id, None);
        assert_eq!(new_operation.role_run_seq, None);
        // 非 LC 面零改动：launch 指纹/会话引用/SC ledger/phase/教学计数。
        assert!(after.logical_launch_fingerprints.is_empty());
        assert!(after.provider_conversations.is_empty());
        assert!(after.provider_start_ledger.is_empty());
        assert!(after.sc_generation_teaching_windows.is_empty());
        assert!(after.single_candidate_phase.is_none());
    }

    /// RED② 守卫：同 operation_id 的 Replay 不写盘（记录 bytes 不变），
    /// 不清当前镜像、不新增 operation/seq。
    #[test]
    fn fresh_operation_binding_same_operation_replay_keeps_record_bytes() {
        let (_tmp, store, record) = session_fixture();
        let recovery = waiting_recovery_with_old_operation(&record);
        store
            .compare_and_save_lc_resume_state(&record, recovery)
            .expect("seed waiting recovery");

        let path = store
            .workspace_sessions_root(&record.project_id, &record.issue_id)
            .join(format!("{}.json", record.id));
        let before = std::fs::read(&path).expect("read record bytes before replay");
        let outcome = store
            .claim_lc_fresh(&record.id, RECOVERY_ID, "op_old_0001")
            .expect("replay same operation");
        assert!(
            matches!(outcome, LcFreshClaimOutcome::Replay { .. }),
            "同 operation_id 必须回放原受理结果，实际: {outcome:?}"
        );
        let after = std::fs::read(&path).expect("read record bytes after replay");
        assert_eq!(before, after, "Replay 不得改写记录字节");
        let current = store
            .get_workspace_session(&record.id)
            .expect("record after replay")
            .lc_resume_recovery
            .expect("recovery kept after replay");
        assert_eq!(
            current.fresh_node_id.as_deref(),
            Some(OLD_NODE_ID),
            "Replay 不是新受理，当前镜像不得被清"
        );
        assert_eq!(current.fresh_role_run_seq, Some(7));
        assert_eq!(current.fresh_native_session_id.as_deref(), Some(OLD_NATIVE_ID));
        assert_eq!(
            current.fresh_operations.len(),
            1,
            "Replay 不得新增 operation"
        );
    }
}
