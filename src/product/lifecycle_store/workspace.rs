use std::path::PathBuf;

use chrono::Utc;

use crate::product::coding_attempt_store::locking::with_exclusive_lock;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    HumanGateTakeoverEvent, PlanRepairSessionSnapshotDto, ProviderConversationRef,
    WorkItemRuntimeBinding, WorkspaceMessageRecord, WorkspaceRolePermissionModes,
    WorkspaceSessionLink, WorkspaceSessionRecord, WorkspaceSessionStatus,
    WorkspaceSessionSummaryRecord, WorkspaceType,
};
use crate::web::workspace_ws_types::{ArtifactVersion, TimelineNode};

use super::{
    CreateWorkspaceSessionInput, LifecycleStore, child_directories, json_file_paths,
    list_workspace_session_records, path_exists, path_is_regular_file,
    read_workspace_session_record, remove_dir_all_if_exists, remove_file_if_exists,
    workspace_session_file_paths,
};

pub struct PolicyRoutePersist {
    pub status: WorkspaceSessionStatus,
    /// SingleCandidate 由同一 CAS 与策略结果一起推进；legacy 始终保持 `None`。
    pub single_candidate_phase: Option<crate::product::models::SingleCandidatePhase>,
    pub run_history: crate::product::work_item_plan_policy::RunHistory,
    pub scope: Option<crate::product::work_item_plan_policy::ReviewInvocationScope>,
    pub gate: Option<crate::product::work_item_plan_policy::HumanGateSnapshot>,
    pub diagnostics: Vec<crate::product::work_item_plan_policy::PolicyDiagnostic>,
    pub repair_reservation: Option<crate::product::work_item_plan_policy::RepairReservation>,
    pub provider_start_ledger: Vec<crate::product::work_item_plan_policy::ProviderStartLedgerEntry>,
}

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
        let path = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
            .join("plan_repair_session_state.json");
        write_json(&path, value)
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
        self.create_workspace_session_with_id(input, id)
    }

    pub(crate) fn create_workspace_session_with_id(
        &self,
        input: CreateWorkspaceSessionInput,
        id: String,
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

        let root = self.workspace_sessions_root(&input.project_id, &input.issue_id);
        let target_path = root.join(format!("{id}.json"));
        if path_exists(&target_path)? {
            let existing: WorkspaceSessionRecord = read_json(&target_path)?;
            if existing.id == id
                && existing.project_id == input.project_id
                && existing.issue_id == input.issue_id
                && existing.entity_id == input.entity_id
                && existing.workspace_type == input.workspace_type
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
            policy_diagnostics: Vec::new(),
            provider_start_ledger: Vec::new(),
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
            work_item_runtime_binding: None,
            provider_conversations: Vec::new(),
            messages: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        };

        super::ensure_target_absent(&target_path)?;
        write_json(&target_path, &session)?;
        Ok(session)
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

        let event_id = format!("human_gate_takeover_{}", parent.id);
        let event_path = self
            .paths
            .issue_lifecycle_root(&parent.project_id, &parent.issue_id)
            .join("human-gate-takeovers")
            .join(format!("{event_id}.json"));
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
            let child = self.create_workspace_session_with_id(
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
                        run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
                        rollout_snapshot: false,
                    }),
                },
                child_id,
            )?;
            let event = HumanGateTakeoverEvent {
                id: event_id,
                parent_session_id: parent.id.clone(),
                child_session_id: child.id.clone(),
                created_at: Utc::now().to_rfc3339(),
            };
            write_json(&event_path, &event)?;
            Ok(child)
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
        validate_relative_id(session_id)?;
        read_json(&self.find_workspace_session_path(session_id)?)
    }

    pub fn ensure_work_item_runtime_binding(
        &self,
        session_id: &str,
        binding: &WorkItemRuntimeBinding,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
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
    }

    pub fn append_workspace_message(
        &self,
        session_id: &str,
        role: String,
        content: String,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
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
    }

    pub fn replace_workspace_messages(
        &self,
        session_id: &str,
        messages: Vec<WorkspaceMessageRecord>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.messages = messages;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn replace_workspace_provider_conversations(
        &self,
        session_id: &str,
        provider_conversations: Vec<ProviderConversationRef>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.provider_conversations = provider_conversations;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn update_workspace_session_status(
        &self,
        session_id: &str,
        status: WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
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
    }

    /// Atomically projects a policy route into the durable session record.
    /// The expected record check prevents stale websocket workers from
    /// overwriting a newer route; callers must reload and re-evaluate on clash.
    pub fn compare_and_save_policy_route(
        &self,
        expected: &WorkspaceSessionRecord,
        persist: PolicyRoutePersist,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        let PolicyRoutePersist {
            status,
            single_candidate_phase,
            run_history,
            scope,
            gate,
            diagnostics,
            repair_reservation,
            provider_start_ledger,
        } = persist;
        validate_relative_id(&expected.id)?;
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
            stored.status = status;
            if stored.flow_kind
                == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
            {
                stored.single_candidate_phase = single_candidate_phase;
            }
            stored.run_history = run_history;
            stored.review_invocation_scope = scope;
            stored.human_gate_snapshot = gate;
            stored.policy_diagnostics = diagnostics;
            stored.repair_reservation = repair_reservation;
            stored.provider_start_ledger = provider_start_ledger;
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&locked_session_path, &stored)?;
            Ok(stored)
        })
    }

    /// Atomically claims a provider-start idempotency key. A key can be claimed
    /// only once, and the durable ledger is the source of truth during recovery.
    pub fn claim_provider_start(
        &self,
        session_id: &str,
        provider_start_idempotency_key: &str,
    ) -> Result<bool, ProductStoreError> {
        validate_relative_id(session_id)?;
        if provider_start_idempotency_key.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "provider_start_ledger",
                reason: "idempotency key must not be empty".to_string(),
            });
        }
        let session_path = self.find_workspace_session_path(session_id)?;
        with_exclusive_lock(&session_path, || {
            let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
            if session
                .provider_start_ledger
                .iter()
                .any(|entry| entry.provider_start_idempotency_key == provider_start_idempotency_key)
            {
                return Ok(false);
            }
            session.provider_start_ledger.push(
                crate::product::work_item_plan_policy::ProviderStartLedgerEntry {
                    provider_start_idempotency_key: provider_start_idempotency_key.to_string(),
                    started: true,
                },
            );
            session.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &session)?;
            Ok(true)
        })
    }

    pub fn compare_and_update_workspace_session_status(
        &self,
        expected: &WorkspaceSessionRecord,
        status: WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(&expected.id)?;
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
            stored.status = status;
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&session_path, &stored)?;
            Ok(stored)
        })
    }

    pub fn update_workspace_session_providers(
        &self,
        session_id: &str,
        author_provider: crate::product::models::ProviderName,
        reviewer_provider: crate::product::models::ProviderName,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.author_provider = author_provider;
        session.reviewer_provider = reviewer_provider;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn update_workspace_session_permission_modes(
        &self,
        session_id: &str,
        permission_modes: WorkspaceRolePermissionModes,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.permission_modes = permission_modes;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn update_workspace_session_provisional_reviewer(
        &self,
        session_id: &str,
        provisional_reviewer_provider: Option<crate::product::models::ProviderName>,
        reviewer_enabled_at_start: Option<bool>,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.provisional_reviewer_provider = provisional_reviewer_provider;
        session.reviewer_enabled_at_start = reviewer_enabled_at_start;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn truncate_workspace_session_messages(
        &self,
        session_id: &str,
        keep_count: usize,
        status: WorkspaceSessionStatus,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        let session_path = self.find_workspace_session_path(session_id)?;
        let mut session: WorkspaceSessionRecord = read_json(&session_path)?;
        session.messages.truncate(keep_count);
        session.status = status;
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn save_timeline_nodes(
        &self,
        session_id: &str,
        nodes: &[TimelineNode],
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_nodes.json");
        write_json(&path, &nodes)
    }

    pub fn load_timeline_nodes(
        &self,
        session_id: &str,
    ) -> Result<Vec<TimelineNode>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_nodes.json");
        if !path_exists(&path)? {
            return Ok(Vec::new());
        }
        read_json(&path)
    }

    pub fn load_timeline_nodes_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<Vec<TimelineNode>, ProductStoreError> {
        let path = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
            .join("timeline_nodes.json");
        if !path_exists(&path)? {
            return Ok(Vec::new());
        }
        read_json(&path)
    }

    pub fn save_node_detail(
        &self,
        session_id: &str,
        node_id: &str,
        detail: &crate::product::models::NodeDetail,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_node_details")
            .join(format!("{node_id}.json"));
        write_json(&path, detail)
    }

    pub fn load_node_detail(
        &self,
        session_id: &str,
        node_id: &str,
    ) -> Result<crate::product::models::NodeDetail, ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_node_details")
            .join(format!("{node_id}.json"));
        if !path_exists(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "node_detail",
                id: format!("{session_id}/{node_id}"),
            });
        }
        read_json(&path)
    }

    pub(crate) fn delete_node_detail(
        &self,
        session_id: &str,
        node_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_node_details")
            .join(format!("{node_id}.json"));
        remove_file_if_exists(&path)
    }

    pub fn load_node_detail_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
        node_id: &str,
    ) -> Result<crate::product::models::NodeDetail, ProductStoreError> {
        validate_relative_id(node_id)?;
        let path = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
            .join("timeline_node_details")
            .join(format!("{node_id}.json"));
        if !path_exists(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "node_detail",
                id: format!("{session_id}/{node_id}"),
            });
        }
        read_json(&path)
    }

    pub fn list_node_detail_ids(&self, session_id: &str) -> Result<Vec<String>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let dir = self
            .workspace_timeline_root_for_session(session_id)?
            .join("timeline_node_details");
        let entries = json_file_paths(&dir)?;
        let mut ids = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Some(stem) = entry.file_stem() {
                ids.push(stem.to_string_lossy().to_string());
            }
        }
        Ok(ids)
    }

    pub fn list_node_detail_ids_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<Vec<String>, ProductStoreError> {
        let dir = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
            .join("timeline_node_details");
        let entries = json_file_paths(&dir)?;
        let mut ids = Vec::with_capacity(entries.len());
        for entry in entries {
            if let Some(stem) = entry.file_stem() {
                ids.push(stem.to_string_lossy().to_string());
            }
        }
        Ok(ids)
    }

    pub fn append_artifact_version(
        &self,
        session_id: &str,
        version: ArtifactVersion,
    ) -> Result<(), ProductStoreError> {
        let mut versions = self.list_artifact_versions(session_id)?;
        versions.push(version);
        self.save_artifact_versions(session_id, &versions)
    }

    pub fn list_artifact_versions(
        &self,
        session_id: &str,
    ) -> Result<Vec<ArtifactVersion>, ProductStoreError> {
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("artifact_versions.json");
        if !path_exists(&path)? {
            return Ok(Vec::new());
        }
        read_json(&path)
    }

    pub fn list_artifact_versions_for_issue_session(
        &self,
        project_id: &str,
        issue_id: &str,
        session_id: &str,
    ) -> Result<Vec<ArtifactVersion>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_issue_session(project_id, issue_id, session_id)?
            .join("artifact_versions.json");
        if !path_exists(&path)? {
            return Ok(Vec::new());
        }
        read_json(&path)
    }

    pub fn save_artifact_versions(
        &self,
        session_id: &str,
        versions: &[ArtifactVersion],
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(session_id)?;
        let path = self
            .workspace_timeline_root_for_session(session_id)?
            .join("artifact_versions.json");
        write_json(&path, &versions)
    }

    pub(crate) fn delete_workspace_sessions_for_entity(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
        workspace_type: WorkspaceType,
    ) -> Result<(), ProductStoreError> {
        let sessions_root = self.workspace_sessions_root(project_id, issue_id);
        let timeline_root = self
            .paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-timelines");
        for session in self
            .list_workspace_sessions(project_id, issue_id)?
            .into_iter()
            .filter(|session| {
                session.entity_id == entity_id && session.workspace_type == workspace_type
            })
        {
            remove_dir_all_if_exists(&timeline_root.join(&session.id))?;
            remove_file_if_exists(&sessions_root.join(format!("{}.json", session.id)))?;
        }
        Ok(())
    }

    fn next_workspace_session_id(&self) -> Result<String, ProductStoreError> {
        let max_sequence = super::workspace_single_candidate::max_workspace_session_sequence(
            &self.paths.projects_root(),
        )?;
        Ok(next_sequential_id("workspace_session", max_sequence))
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

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::WorkItemPlanSessionOptions;
    use crate::product::models::ProviderName;
    use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};

    fn create_input(
        workspace_type: WorkspaceType,
        work_item_plan_options: Option<WorkItemPlanSessionOptions>,
    ) -> CreateWorkspaceSessionInput {
        CreateWorkspaceSessionInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "entity_0001".to_string(),
            workspace_type,
            author_provider: ProviderName::Codex,
            reviewer_provider: ProviderName::ClaudeCode,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options,
        }
    }

    #[test]
    fn create_workspace_session_persists_work_item_plan_options() {
        let temp = tempdir().unwrap();
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));
        let options = WorkItemPlanSessionOptions {
            flow_kind: WorkItemPlanFlowKind::SingleCandidate,
            run_policy: RunPolicy::AutoIfValid,
            rollout_snapshot: true,
        };

        let session = store
            .create_workspace_session(create_input(
                WorkspaceType::WorkItemPlan,
                Some(options.clone()),
            ))
            .unwrap();

        assert_eq!(session.flow_kind, options.flow_kind);
        assert_eq!(session.run_policy, options.run_policy);
        assert_eq!(
            session.single_candidate_phase,
            Some(crate::product::models::SingleCandidatePhase::Prepare)
        );
        let reissued = store
            .create_workspace_session_with_id(
                create_input(WorkspaceType::WorkItemPlan, Some(options.clone())),
                session.id.clone(),
            )
            .expect("reissued create returns immutable existing snapshot");
        assert_eq!(reissued.run_policy, options.run_policy);
        assert_eq!(
            store.get_workspace_session(&session.id).unwrap().flow_kind,
            WorkItemPlanFlowKind::SingleCandidate
        );
    }

    #[test]
    fn create_workspace_session_rejects_work_item_plan_options_for_other_workspace_types() {
        let temp = tempdir().unwrap();
        let store = LifecycleStore::new(ProductAppPaths::new(temp.path()));

        let error = store
            .create_workspace_session(create_input(
                WorkspaceType::Story,
                Some(WorkItemPlanSessionOptions::default()),
            ))
            .unwrap_err();

        assert!(matches!(
            error,
            ProductStoreError::InvalidRecord {
                kind: "workspace_session",
                ..
            }
        ));
    }
}
