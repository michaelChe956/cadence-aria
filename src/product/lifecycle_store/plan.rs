use chrono::Utc;

use std::collections::BTreeSet;

use crate::product::advance_store::{AdvancePlanArtifacts, AdvanceStore};
use crate::product::entity_deletion_store::{
    DeletionEntityKind, EntityDeletionManifest, EntityReference, EntityScope, read_json_if_exists,
};
use crate::product::entity_operation_store::EntityDeletionLease;
use crate::product::id::next_sequential_id_in_directory;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    IssueWorkItemPlan, IssueWorkItemPlanStatus, LifecycleWorkItemRecord, ProviderName,
    ProviderReviewRoundRecord, RepositoryProfile, SpecVersionRecord, VerificationPlan,
    WorkItemPlanLineage, WorkItemPlanStatus, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::work_item_split_engine::WorkItemSplitProviderOutput;

use super::{
    CreateIssueWorkItemPlanInput, CreateRepositoryProfileInput, CreateVerificationPlanInput,
    CreateWorkItemInput, IssueWorkItemPlanUpdate, LifecycleStore, WorkItemPlanCandidateSnapshot,
    child_directories, delete_required_file, list_json_records, path_exists, path_is_regular_file,
    remove_dir_all_if_exists, remove_file_if_exists, validate_relative_ids,
};

impl LifecycleStore {
    pub fn create_issue_work_item_plan(
        &self,
        input: CreateIssueWorkItemPlanInput,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_ids(&input.work_item_ids)?;
        validate_relative_ids(&input.verification_plan_ids)?;

        // D1.3a：issue 级 ID 分配先走关系短锁；关联创建同时核上游 Story/Design
        // scope（plan §0.2 锁序 scope→关系，上游准入跨分配与写全程持有）。
        let root = self.issue_work_item_plans_root(&input.project_id, &input.issue_id);
        let upstream_scopes: Vec<EntityScope> = input
            .source_story_spec_ids
            .iter()
            .map(|story_id| EntityScope::new(&input.project_id, &input.issue_id, DeletionEntityKind::Story, story_id))
            .chain(input.source_design_spec_ids.iter().map(|design_id| EntityScope::new(&input.project_id, &input.issue_id, DeletionEntityKind::Design, design_id)))
            .collect();
        let project_id = input.project_id.clone();
        let issue_id = input.issue_id.clone();
        let input_id = input.id.clone();
        let mut admission_scopes = upstream_scopes;
        // 显式 id 的自身 scope 一并准入（墓碑拒复用）。
        if let Some(id) = &input_id {
            admission_scopes.push(EntityScope::new(
                &project_id,
                &issue_id,
                DeletionEntityKind::Plan,
                id,
            ));
        }
        self.with_entity_live_writes(&admission_scopes, move || {
            let id = match &input_id {
                Some(id) => {
                    validate_relative_id(id)?;
                    id.clone()
                }
                None => self.entity_operation_store().with_issue_relations(
                    &project_id,
                    &issue_id,
                    || {
                        self.next_fresh_entity_id(
                            "issue_work_item_plan",
                            &root,
                            DeletionEntityKind::Plan,
                            &project_id,
                            &issue_id,
                        )
                    },
                )?,
            };
            let now = Utc::now().to_rfc3339();
            let plan = IssueWorkItemPlan {
                id: id.clone(),
                project_id: input.project_id,
                issue_id: input.issue_id,
                source_story_spec_ids: input.source_story_spec_ids,
                source_design_spec_ids: input.source_design_spec_ids,
                options: input.options,
                status: input.status,
                work_item_ids: input.work_item_ids,
                repository_profile_ref: input.repository_profile_ref,
                verification_plan_ids: input.verification_plan_ids,
                dependency_graph: input.dependency_graph,
                created_from_provider_run: input.created_from_provider_run,
                validator_findings: input.validator_findings,
                review_summary: None,
                created_at: now.clone(),
                updated_at: now,
            };
            write_json(&root.join(format!("{id}.json")), &plan)?;
            Ok(plan)
        })
    }

    /// P1 WIGA Task 3：绑定创建——固定 id 既存时必须与冻结身份（project/issue/
    /// source/options）完全一致才复用；绝不覆盖。覆盖式 `create_issue_work_item_plan`
    /// （id=Some 直接写）对 bound 路径禁用。
    pub fn ensure_issue_work_item_plan_with_identity(
        &self,
        input: CreateIssueWorkItemPlanInput,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
        let Some(id) = input.id.clone() else {
            return Err(ProductStoreError::InvalidRecord {
                kind: "issue_work_item_plan",
                reason: "bound plan creation requires an explicit id".to_string(),
            });
        };
        let project_id = input.project_id.clone();
        let issue_id = input.issue_id.clone();
        let mut admission_scopes: Vec<EntityScope> = input
            .source_story_spec_ids
            .iter()
            .map(|story_id| {
                EntityScope::new(&project_id, &issue_id, DeletionEntityKind::Story, story_id)
            })
            .chain(input.source_design_spec_ids.iter().map(|design_id| {
                EntityScope::new(&project_id, &issue_id, DeletionEntityKind::Design, design_id)
            }))
            .collect();
        admission_scopes.push(EntityScope::new(
            &project_id,
            &issue_id,
            DeletionEntityKind::Plan,
            &id,
        ));
        self.with_entity_live_writes(&admission_scopes, move || {
            let path = self
                .issue_work_item_plans_root(&input.project_id, &input.issue_id)
                .join(format!("{id}.json"));
            if path_exists(&path)? {
                let existing: IssueWorkItemPlan = read_json(&path)?;
                if existing.id == id
                    && existing.project_id == input.project_id
                    && existing.issue_id == input.issue_id
                    && existing.source_story_spec_ids == input.source_story_spec_ids
                    && existing.source_design_spec_ids == input.source_design_spec_ids
                    && existing.options == input.options
                {
                    return Ok(existing);
                }
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "issue_work_item_plan",
                    id,
                });
            }
            self.create_issue_work_item_plan(input)
        })
    }

    pub fn get_issue_work_item_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        read_json(
            &self
                .issue_work_item_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json")),
        )
    }

    pub fn list_issue_work_item_plans(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<IssueWorkItemPlan>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.issue_work_item_plans_root(project_id, issue_id))
    }

    pub fn delete_issue_work_item_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
        let plan = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;

        delete_required_file(
            &self
                .issue_work_item_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json")),
            "issue_work_item_plan",
            plan_id,
        )?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            plan_id,
            WorkspaceType::WorkItemPlan,
        )?;
        for verification_plan_id in &plan.verification_plan_ids {
            self.delete_verification_plan(project_id, issue_id, verification_plan_id)?;
        }
        if let Some(repository_profile_id) = &plan.repository_profile_ref {
            self.delete_repository_profile(project_id, issue_id, repository_profile_id)?;
        }

        Ok(plan)
    }

    pub fn delete_schema_v2_issue_work_item_plan_metadata(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
        let plan = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
        delete_required_file(
            &self
                .issue_work_item_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json")),
            "issue_work_item_plan",
            plan_id,
        )?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            plan_id,
            WorkspaceType::WorkItemPlan,
        )?;
        Ok(plan)
    }

    pub fn confirm_issue_work_item_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<
        (
            IssueWorkItemPlan,
            Vec<crate::product::models::LifecycleWorkItemRecord>,
        ),
        ProductStoreError,
    > {
        let plan_scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        self.with_entity_live_write(&plan_scope, move || {
            let mut plan = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
        if plan.status != IssueWorkItemPlanStatus::Draft {
            return Err(ProductStoreError::Io(
                "issue_work_item_plan_not_draft".to_string(),
            ));
        }

        let work_items = self.list_work_items(project_id, issue_id)?;
        let mut linked = Vec::with_capacity(plan.work_item_ids.len());
        for work_item_id in &plan.work_item_ids {
            let item = work_items
                .iter()
                .find(|item| &item.id == work_item_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "work_item",
                    id: work_item_id.clone(),
                })?;
            if item.project_id != project_id || item.issue_id != issue_id {
                return Err(ProductStoreError::Io(
                    "issue_work_item_plan_project_issue_mismatch".to_string(),
                ));
            }
            linked.push(item.clone());
        }

        for verification_plan_id in &plan.verification_plan_ids {
            self.get_verification_plan(project_id, issue_id, verification_plan_id)?;
        }

        let now = Utc::now().to_rfc3339();
        plan.status = IssueWorkItemPlanStatus::Confirmed;
        plan.updated_at = now.clone();
        write_json(
            &self
                .issue_work_item_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json")),
            &plan,
        )?;

        let mut confirmed = Vec::with_capacity(linked.len());
        for item in linked {
            let updated = self.update_work_item_plan_status(
                project_id,
                issue_id,
                &item.id,
                WorkItemPlanStatus::Confirmed,
            )?;
            confirmed.push(updated);
        }
            Ok((plan, confirmed))
        })
    }

    /// 为 plan 关联的每个 WorkItem 幂等创建 WorkspaceType::WorkItem 子 session。
    ///
    /// 在 HumanConfirm::Confirm 时调用。若 WorkItem 已有子 session（重试场景），跳过。
    /// 返回新建的 session 列表（已存在的跳过不计入）。
    #[allow(clippy::too_many_arguments)]
    pub fn ensure_work_item_sessions_for_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        author_provider: ProviderName,
        reviewer_provider: Option<ProviderName>,
        review_rounds: u32,
        superpowers_enabled: bool,
        openspec_enabled: bool,
    ) -> Result<Vec<WorkspaceSessionRecord>, ProductStoreError> {
        let plan = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
        let existing_sessions = self.list_workspace_sessions(project_id, issue_id)?;
        let mut created = Vec::new();
        for wi_id in &plan.work_item_ids {
            let already_exists = existing_sessions
                .iter()
                .any(|s| s.workspace_type == WorkspaceType::WorkItem && s.entity_id == *wi_id);
            if already_exists {
                continue;
            }
            let session = self.create_workspace_session(super::CreateWorkspaceSessionInput {
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                entity_id: wi_id.clone(),
                workspace_type: WorkspaceType::WorkItem,
                author_provider: author_provider.clone(),
                // C2 Task 5（REQ-CRO-05）：不回填 Codex——reviewer 缺失保持空 effective。
                reviewer_provider: reviewer_provider.clone(),
                review_rounds,
                superpowers_enabled,
                openspec_enabled,
                work_item_plan_options: None,
            })?;
            created.push(session);
        }
        Ok(created)
    }

    pub fn request_issue_work_item_plan_change(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        _note: Option<String>,
    ) -> Result<
        (
            IssueWorkItemPlan,
            Vec<crate::product::models::LifecycleWorkItemRecord>,
        ),
        ProductStoreError,
    > {
        let plan_scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        self.with_entity_live_write(&plan_scope, move || {
            let mut plan = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
        if plan.status != IssueWorkItemPlanStatus::Draft
            && plan.status != IssueWorkItemPlanStatus::Confirmed
        {
            return Err(ProductStoreError::Io(
                "issue_work_item_plan_not_actionable".to_string(),
            ));
        }

        let work_items = self.list_work_items(project_id, issue_id)?;
        let mut linked = Vec::with_capacity(plan.work_item_ids.len());
        for work_item_id in &plan.work_item_ids {
            let item = work_items
                .iter()
                .find(|item| &item.id == work_item_id)
                .ok_or_else(|| ProductStoreError::NotFound {
                    kind: "work_item",
                    id: work_item_id.clone(),
                })?;
            if item.project_id != project_id || item.issue_id != issue_id {
                return Err(ProductStoreError::Io(
                    "issue_work_item_plan_project_issue_mismatch".to_string(),
                ));
            }
            linked.push(item.clone());
        }

        for verification_plan_id in &plan.verification_plan_ids {
            self.get_verification_plan(project_id, issue_id, verification_plan_id)?;
        }

        let now = Utc::now().to_rfc3339();
        plan.status = IssueWorkItemPlanStatus::ChangeRequested;
        plan.updated_at = now.clone();
        write_json(
            &self
                .issue_work_item_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json")),
            &plan,
        )?;

        let mut updated_items = Vec::with_capacity(linked.len());
        for item in linked {
            let updated = self.update_work_item_plan_status(
                project_id,
                issue_id,
                &item.id,
                WorkItemPlanStatus::Draft,
            )?;
            updated_items.push(updated);
        }
            Ok((plan, updated_items))
            })
    }

    pub fn update_issue_work_item_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        update: IssueWorkItemPlanUpdate,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
        let plan_scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        self.with_entity_live_write(&plan_scope, move || {
            let mut plan = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
            plan.work_item_ids = update.work_item_ids;
            plan.verification_plan_ids = update.verification_plan_ids;
            plan.repository_profile_ref = update.repository_profile_ref;
            plan.dependency_graph = update.dependency_graph;
            plan.created_from_provider_run = update.created_from_provider_run;
            plan.validator_findings = update.validator_findings;
            plan.updated_at = Utc::now().to_rfc3339();
            let path = self
                .issue_work_item_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json"));
            write_json(&path, &plan)?;
            Ok(plan)
        })
    }

    pub fn commit_issue_work_item_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        update: IssueWorkItemPlanUpdate,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
        validate_relative_ids(&update.work_item_ids)?;
        validate_relative_ids(&update.verification_plan_ids)?;
        let plan_scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        self.with_entity_live_write(&plan_scope, move || {
            let mut plan = self.update_issue_work_item_plan(project_id, issue_id, plan_id, update)?;
            plan.status = IssueWorkItemPlanStatus::Confirmed;
            plan.updated_at = Utc::now().to_rfc3339();
            let path = self
                .issue_work_item_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json"));
            write_json(&path, &plan)?;
            Ok(plan)
        })
    }

    pub fn restore_issue_work_item_plan_snapshot(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        snapshot: &IssueWorkItemPlan,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        if snapshot.id != plan_id
            || snapshot.project_id != project_id
            || snapshot.issue_id != issue_id
        {
            return Err(ProductStoreError::Io(format!(
                "issue_work_item_plan_snapshot_mismatch: {plan_id}"
            )));
        }
        let plan_scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        self.with_entity_live_write(&plan_scope, move || {
            let mut plan = snapshot.clone();
            plan.updated_at = Utc::now().to_rfc3339();
            let path = self
                .issue_work_item_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json"));
            write_json(&path, &plan)?;
            Ok(plan)
        })
    }

    pub fn replace_issue_work_item_plan_candidate(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        output: &WorkItemSplitProviderOutput,
        validator_findings: Vec<crate::product::models::WorkItemSplitFinding>,
    ) -> Result<WorkItemPlanCandidateSnapshot, ProductStoreError> {
        let plan_scope = EntityScope::new(project_id, issue_id, DeletionEntityKind::Plan, plan_id);
        self.with_entity_live_write(&plan_scope, move || {
        let existing = self.get_issue_work_item_plan(project_id, issue_id, plan_id)?;
        if existing.status != IssueWorkItemPlanStatus::Draft {
            return Err(ProductStoreError::Io(format!(
                "issue_work_item_plan_not_draft: {plan_id} status={:?}",
                existing.status
            )));
        }

        for old_wi_id in &existing.work_item_ids {
            let path = self
                .work_items_root(project_id, issue_id)
                .join(format!("{old_wi_id}.json"));
            remove_file_if_exists(&path)?;
        }
        for old_vp_id in &existing.verification_plan_ids {
            let path = self
                .verification_plans_root(project_id, issue_id)
                .join(format!("{old_vp_id}.json"));
            let _ = remove_file_if_exists(&path);
        }
        if let Some(old_profile_id) = &existing.repository_profile_ref {
            let path = self
                .repository_profiles_root(project_id, issue_id)
                .join(format!("{old_profile_id}.json"));
            let _ = remove_file_if_exists(&path);
        }

        self.create_repository_profile(CreateRepositoryProfileInput {
            id: Some(output.repository_profile.id.clone()),
            project_id: project_id.to_string(),
            issue_id: issue_id.to_string(),
            repository_id: output.repository_profile.repository_id.clone(),
            provider_run_ref: output.repository_profile.provider_run_ref.clone(),
            languages: output.repository_profile.languages.clone(),
            frameworks: output.repository_profile.frameworks.clone(),
            package_managers: output.repository_profile.package_managers.clone(),
            test_frameworks: output.repository_profile.test_frameworks.clone(),
            build_systems: output.repository_profile.build_systems.clone(),
            verification_capabilities: output.repository_profile.verification_capabilities.clone(),
            detected_layers: output.repository_profile.detected_layers.clone(),
            split_recommendation: output.repository_profile.split_recommendation.clone(),
            confidence: output.repository_profile.confidence.clone(),
            uncertainties: output.repository_profile.uncertainties.clone(),
        })?;

        for vp in &output.verification_plans {
            self.create_verification_plan(CreateVerificationPlanInput {
                id: Some(vp.id.clone()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                work_item_id: vp.work_item_id.clone(),
                repository_profile_ref: vp.repository_profile_ref.clone(),
                provider_run_ref: vp.provider_run_ref.clone(),
                scope: vp.scope.clone(),
                commands: vp.commands.clone(),
                manual_checks: vp.manual_checks.clone(),
                required_gates: vp.required_gates.clone(),
                risk_notes: vp.risk_notes.clone(),
                confidence: vp.confidence.clone(),
                fallback_policy: vp.fallback_policy.clone(),
            })?;
        }

        for wi in &output.work_items {
            self.create_work_item(CreateWorkItemInput {
                id: Some(wi.id.clone()),
                project_id: project_id.to_string(),
                issue_id: issue_id.to_string(),
                repository_id: wi.repository_id.clone(),
                story_spec_ids: wi.story_spec_ids.clone(),
                design_spec_ids: wi.design_spec_ids.clone(),
                title: wi.title.clone(),
                work_item_set_id: wi.work_item_set_id.clone(),
                source_work_item_plan_id: wi.source_work_item_plan_id.clone(),
                source_outline_id: wi.source_outline_id.clone(),
                source_draft_id: wi.source_draft_id.clone(),
                planned_implementation_context: wi.planned_implementation_context.clone(),
                kind: wi.kind.clone(),
                sequence_hint: wi.sequence_hint,
                depends_on: wi.depends_on.clone(),
                exclusive_write_scopes: wi.exclusive_write_scopes.clone(),
                forbidden_write_scopes: wi.forbidden_write_scopes.clone(),
                context_budget: wi.context_budget.clone(),
                verification_plan_ref: wi.verification_plan_ref.clone(),
                require_execution_plan_confirm: wi.require_execution_plan_confirm,
                plan_status: WorkItemPlanStatus::Draft,
            })?;
        }

        let new_wi_ids: Vec<String> = output.work_items.iter().map(|wi| wi.id.clone()).collect();
        let new_vp_ids: Vec<String> = output
            .verification_plans
            .iter()
            .map(|vp| vp.id.clone())
            .collect();
        let new_profile_id = output.repository_profile.id.clone();
        let new_graph = output.plan.dependency_graph.clone();
        let provider_run_ref = output.plan.created_from_provider_run.clone();
        self.update_issue_work_item_plan(
            project_id,
            issue_id,
            plan_id,
            IssueWorkItemPlanUpdate {
                work_item_ids: new_wi_ids.clone(),
                verification_plan_ids: new_vp_ids.clone(),
                repository_profile_ref: Some(new_profile_id.clone()),
                dependency_graph: new_graph,
                created_from_provider_run: provider_run_ref,
                validator_findings,
            },
        )?;

        Ok(WorkItemPlanCandidateSnapshot {
            plan_id: plan_id.to_string(),
            work_item_ids: new_wi_ids,
            verification_plan_ids: new_vp_ids,
            repository_profile_id: new_profile_id,
        })
        })
    }

    pub fn create_repository_profile(
        &self,
        input: CreateRepositoryProfileInput,
    ) -> Result<RepositoryProfile, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;

        let root = self.repository_profiles_root(&input.project_id, &input.issue_id);
        let id =
            match input.id {
                Some(ref id) => {
                    validate_relative_id(id)?;
                    id.clone()
                }
                None => next_sequential_id_in_directory("repository_profile", &root).map_err(
                    |error| ProductStoreError::Io(format!("read {}: {error}", root.display())),
                )?,
            };
        let now = Utc::now().to_rfc3339();
        let profile = RepositoryProfile {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            repository_id: input.repository_id,
            logical_repository_id: None,
            membership_revision: 0,
            provider_run_ref: input.provider_run_ref,
            languages: input.languages,
            frameworks: input.frameworks,
            package_managers: input.package_managers,
            test_frameworks: input.test_frameworks,
            build_systems: input.build_systems,
            verification_capabilities: input.verification_capabilities,
            detected_layers: input.detected_layers,
            split_recommendation: input.split_recommendation,
            confidence: input.confidence,
            uncertainties: input.uncertainties,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(&root.join(format!("{id}.json")), &profile)?;
        Ok(profile)
    }

    pub fn get_repository_profile(
        &self,
        project_id: &str,
        issue_id: &str,
        profile_id: &str,
    ) -> Result<RepositoryProfile, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(profile_id)?;
        read_json(
            &self
                .repository_profiles_root(project_id, issue_id)
                .join(format!("{profile_id}.json")),
        )
    }

    pub fn list_repository_profiles(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<RepositoryProfile>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.repository_profiles_root(project_id, issue_id))
    }

    pub fn delete_repository_profile(
        &self,
        project_id: &str,
        issue_id: &str,
        repository_profile_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(repository_profile_id)?;

        let path = self
            .repository_profiles_root(project_id, issue_id)
            .join(format!("{repository_profile_id}.json"));
        delete_required_file(&path, "repository_profile", repository_profile_id)
    }

    pub fn save_work_item_split_provider_run(
        &self,
        project_id: &str,
        issue_id: &str,
        provider_type: &ProviderName,
        prompt: &str,
        structured_output: &serde_json::Value,
    ) -> Result<String, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;

        let root = self.provider_runs_root(project_id, issue_id);
        // Task 1c(carry④/Ruling 5B):split 运行身份与 1b
        // `WorkItemSplitProviderRunHandle` 装配对齐——legacy 落档路径改铸与
        // handle 同族的 `ws-<scope>-split-run-<seq>` 前缀(scope 取本路径
        // 仅有的 project/issue 二元组);run_ref 是不透明字符串,流经
        // created_from_provider_run/provider_run_ref 无任何格式解析,前缀族
        // 对齐不影响既有消费者。序号仍按 per-issue 目录单调分配(4 位零
        // 填充,与既有 next_sequential_id 同宽);旧 provider_run_split_NNNN
        // 记录不迁移、不冲突(不同前缀名空间)。
        let id = next_ws_split_run_id(project_id, issue_id, &root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?;
        let dir = root.join(&id);
        let now = Utc::now().to_rfc3339();
        write_json(
            &dir.join("run.json"),
            &serde_json::json!({
                "provider_run_id": id,
                "provider_type": provider_type,
                "status": "completed",
                "prompt_chars": prompt.chars().count(),
                "structured_output_ref": format!("{id}_structured_output"),
                "created_at": now
            }),
        )?;
        write_json(&dir.join("structured_output.json"), structured_output)?;
        Ok(id)
    }

    // ==== 实体级联删除 Task D1.1：真实 manifest、审计子依赖与根最后 ====
    // （fix-cascade-deletion plan §0.2/§0.5；REQ-GRP-01、REQ-DEL-01/03/07）
    //
    // 归属事实一律来自 server 实际拥有关系（记录 refs/真实 source/binding/身份
    // 索引），不依赖健康 candidate 或先 get 根；读取错误 fail-closed，不当空。

    /// scope 归属 session 纯 ID 集：D1.0 身份 list-by-owner（显式绑定＋
    /// owned-index，purging/deleted 可查）∪ 存活 session 记录的真实
    /// parent/Plan binding 归约（§0.5 WI/session 行）。
    fn entity_owned_session_ids(
        &self,
        scope: &EntityScope,
    ) -> Result<BTreeSet<String>, ProductStoreError> {
        let mut ids: BTreeSet<String> = self
            .entity_operation_store()
            .list_workspace_session_ids_by_owner(scope)?
            .into_iter()
            .collect();
        for session in self.list_workspace_sessions(&scope.project_id, &scope.issue_id)? {
            if self.entity_scope_for_session(&session)?.as_ref() == Some(scope) {
                ids.insert(session.id);
            }
        }
        Ok(ids)
    }

    /// review 纯 ID 集：`review.session_id` join owned session（§0.5）。
    fn entity_owned_review_ids(
        &self,
        scope: &EntityScope,
        session_ids: &BTreeSet<String>,
    ) -> Result<BTreeSet<String>, ProductStoreError> {
        let rounds: Vec<ProviderReviewRoundRecord> = list_json_records(
            &self.provider_review_rounds_root(&scope.project_id, &scope.issue_id),
        )?;
        Ok(rounds
            .into_iter()
            .filter(|round| session_ids.contains(&round.session_id))
            .map(|round| round.id)
            .collect())
    }

    /// Story/Design 版本的 provider_run_refs（§0.5 versions 行；目录缺失=空）。
    fn spec_entity_run_ids(
        &self,
        scope: &EntityScope,
    ) -> Result<BTreeSet<String>, ProductStoreError> {
        let versions: Vec<SpecVersionRecord> = list_json_records(&self.versions_root(
            &scope.project_id,
            &scope.issue_id,
            &scope.entity_id,
        ))?;
        Ok(versions
            .into_iter()
            .flat_map(|version| version.provider_run_refs)
            .collect())
    }

    /// Plan 拥有 WI 纯 ID 集与 target repository 集：计划记录 refs ∪ 真实
    /// `source_work_item_plan_id` 归属（摘要缺项补齐）∪ 一致 `work_item_set_id`
    /// join（与 D1.3c 归属归约同惯例）∪ D1.0 child owner 索引（根缺仍反查）。
    /// 返回 (Plan 记录 read-if-exists, WI IDs, target repository IDs)。
    fn plan_owned_work_item_ids(
        &self,
        scope: &EntityScope,
    ) -> Result<
        (
            Option<IssueWorkItemPlan>,
            BTreeSet<String>,
            BTreeSet<String>,
        ),
        ProductStoreError,
    > {
        let plan: Option<IssueWorkItemPlan> = read_json_if_exists(
            &self
                .issue_work_item_plans_root(&scope.project_id, &scope.issue_id)
                .join(format!("{}.json", scope.entity_id)),
        )?;
        let work_items = self.list_work_items(&scope.project_id, &scope.issue_id)?;
        let mut ids: BTreeSet<String> = plan
            .iter()
            .flat_map(|record| record.work_item_ids.iter().cloned())
            .collect();
        let joined_sets: BTreeSet<&String> = work_items
            .iter()
            .filter(|item| item.source_work_item_plan_id.as_deref() == Some(&scope.entity_id))
            .filter_map(|item| item.work_item_set_id.as_ref())
            .collect();
        let mut targets: BTreeSet<String> = BTreeSet::new();
        for item in &work_items {
            let owned = item.source_work_item_plan_id.as_deref() == Some(&scope.entity_id)
                || (item.source_work_item_plan_id.is_none()
                    && item
                        .work_item_set_id
                        .as_ref()
                        .is_some_and(|set| joined_sets.contains(set)));
            if owned {
                ids.insert(item.id.clone());
                if let Some(target) = item.target_repository_id {
                    targets.insert(target.0.to_string());
                }
            }
        }
        ids.extend(
            self.entity_operation_store()
                .list_child_entity_ids_by_owner(scope)?,
        );
        Ok((plan, ids, targets))
    }

    /// issue 内全部 plan lineage 记录（`work-item-revisions/{plan}/lineage.json`，
    /// §0.5 revision 行固定路径只读扫描；坏 JSON fail-closed）。
    fn list_plan_lineage_records(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<WorkItemPlanLineage>, ProductStoreError> {
        let revisions_root = self
            .paths
            .issue_root(project_id, issue_id)
            .join("work-item-revisions");
        let mut records = Vec::new();
        for plan_dir in child_directories(&revisions_root)? {
            let lineage_path = plan_dir.join("lineage.json");
            if path_is_regular_file(&lineage_path)? {
                records.push(read_json(&lineage_path)?);
            }
        }
        Ok(records)
    }

    /// 真实归属 manifest 采集（§0.2）：只真实 ID，无内容/digest；root JSON
    /// 缺失仍按身份索引/存活记录/固定子树枚举（重入残留反查同源）。
    /// advance/retry 集合由 D1.2c `list_plan_artifacts` 接入（不先 get Plan）。
    pub fn collect_entity_deletion_manifest(
        &self,
        scope: &EntityScope,
    ) -> Result<EntityDeletionManifest, ProductStoreError> {
        scope.validate()?;
        let session_ids = self.entity_owned_session_ids(scope)?;
        let review_ids = self.entity_owned_review_ids(scope, &session_ids)?;
        let sorted = |set: BTreeSet<String>| set.into_iter().collect::<Vec<String>>();
        match scope.kind {
            DeletionEntityKind::Story | DeletionEntityKind::Design => {
                let provider_run_ids = self.spec_entity_run_ids(scope)?;
                Ok(EntityDeletionManifest {
                    scope: scope.clone(),
                    session_ids: sorted(session_ids),
                    work_item_ids: Vec::new(),
                    review_ids: sorted(review_ids),
                    provider_run_ids: sorted(provider_run_ids),
                    verification_plan_ids: Vec::new(),
                    repository_profile_ids: Vec::new(),
                    target_repository_ids: Vec::new(),
                    advance_ids: Vec::new(),
                    advance_retry_command_ids: Vec::new(),
                })
            }
            DeletionEntityKind::Plan => {
                let (plan, work_item_ids, target_repository_ids) =
                    self.plan_owned_work_item_ids(scope)?;
                let mut verification_plan_ids: BTreeSet<String> = plan
                    .iter()
                    .flat_map(|record| record.verification_plan_ids.iter().cloned())
                    .collect();
                let mut provider_run_ids: BTreeSet<String> = plan
                    .iter()
                    .filter_map(|record| record.created_from_provider_run.clone())
                    .collect();
                let mut repository_profile_ids: BTreeSet<String> = plan
                    .iter()
                    .filter_map(|record| record.repository_profile_ref.clone())
                    .collect();
                // VP 按 owned WI 真实 join（D1.3a 归属归约同源）；配置 refs
                // （provider_run_ref/repository_profile_ref）反查 run/profile。
                let plans: Vec<VerificationPlan> = list_json_records(
                    &self.verification_plans_root(&scope.project_id, &scope.issue_id),
                )?;
                for verification_plan in &plans {
                    if work_item_ids.contains(&verification_plan.work_item_id) {
                        verification_plan_ids.insert(verification_plan.id.clone());
                        if let Some(run) = &verification_plan.provider_run_ref {
                            provider_run_ids.insert(run.clone());
                        }
                        if let Some(profile) = &verification_plan.repository_profile_ref {
                            repository_profile_ids.insert(profile.clone());
                        }
                    }
                }
                let artifacts = AdvanceStore::new(self.paths.clone()).list_plan_artifacts(
                    &scope.project_id,
                    &scope.issue_id,
                    &scope.entity_id,
                )?;
                Ok(EntityDeletionManifest {
                    scope: scope.clone(),
                    session_ids: sorted(session_ids),
                    work_item_ids: sorted(work_item_ids),
                    review_ids: sorted(review_ids),
                    provider_run_ids: sorted(provider_run_ids),
                    verification_plan_ids: sorted(verification_plan_ids),
                    repository_profile_ids: sorted(repository_profile_ids),
                    target_repository_ids: sorted(target_repository_ids),
                    advance_ids: artifacts.advance_ids,
                    advance_retry_command_ids: artifacts.retry_command_ids,
                })
            }
        }
    }

    /// §0.5 根已删后的 list-by-owner 固定来源残留反查：不先 get 根；逐 ID 核
    /// 实际内容存在性（纯身份绑定/owned-index 不当残留），advance 集由
    /// `AdvanceStore::list_plan_artifacts` 存在性枚举。
    pub fn list_entity_owned_residue(
        &self,
        scope: &EntityScope,
    ) -> Result<EntityDeletionManifest, ProductStoreError> {
        let manifest = self.collect_entity_deletion_manifest(scope)?;
        let lifecycle_root = self
            .paths
            .issue_lifecycle_root(&scope.project_id, &scope.issue_id);
        let sessions_root = self.workspace_sessions_root(&scope.project_id, &scope.issue_id);
        let app_root = self.paths.root();
        let mut residue = EntityDeletionManifest::empty(scope.clone());

        for session_id in &manifest.session_ids {
            let present = path_exists(&sessions_root.join(format!("{session_id}.json")))?
                || path_exists(&sessions_root.join(session_id))?
                || path_exists(&sessions_root.join(format!(".{session_id}.json.lock")))?
                || path_exists(&lifecycle_root.join("workspace-timelines").join(session_id))?
                || path_exists(&app_root.join("work-item-split-runs").join(session_id))?
                || path_exists(&app_root.join("tool-policy-run-audit").join(session_id))?;
            if present {
                residue.session_ids.push(session_id.clone());
            }
        }
        for work_item_id in &manifest.work_item_ids {
            if path_exists(
                &lifecycle_root
                    .join("work-items")
                    .join(format!("{work_item_id}.json")),
            )? {
                residue.work_item_ids.push(work_item_id.clone());
            }
        }
        for review_id in &manifest.review_ids {
            if path_exists(
                &self
                    .provider_review_rounds_root(&scope.project_id, &scope.issue_id)
                    .join(format!("{review_id}.json")),
            )? {
                residue.review_ids.push(review_id.clone());
            }
        }
        for run_id in &manifest.provider_run_ids {
            if path_exists(
                &self
                    .provider_runs_root(&scope.project_id, &scope.issue_id)
                    .join(run_id),
            )? {
                residue.provider_run_ids.push(run_id.clone());
            }
        }
        for verification_plan_id in &manifest.verification_plan_ids {
            if path_exists(
                &self
                    .verification_plans_root(&scope.project_id, &scope.issue_id)
                    .join(format!("{verification_plan_id}.json")),
            )? {
                residue
                    .verification_plan_ids
                    .push(verification_plan_id.clone());
            }
        }
        for profile_id in &manifest.repository_profile_ids {
            if path_exists(
                &self
                    .repository_profiles_root(&scope.project_id, &scope.issue_id)
                    .join(format!("{profile_id}.json")),
            )? {
                residue.repository_profile_ids.push(profile_id.clone());
            }
        }
        // target repository 是引用不是文件：仅当仍被残留 owned WI 记录持有时算残留。
        let mut held_targets: BTreeSet<String> = BTreeSet::new();
        for work_item_id in &residue.work_item_ids {
            let record: LifecycleWorkItemRecord = read_json(
                &lifecycle_root
                    .join("work-items")
                    .join(format!("{work_item_id}.json")),
            )?;
            if let Some(target) = record.target_repository_id {
                held_targets.insert(target.0.to_string());
            }
        }
        residue.target_repository_ids = manifest
            .target_repository_ids
            .iter()
            .filter(|target| held_targets.contains(*target))
            .cloned()
            .collect();
        if scope.kind == DeletionEntityKind::Plan {
            let artifacts = AdvanceStore::new(self.paths.clone()).list_plan_artifacts(
                &scope.project_id,
                &scope.issue_id,
                &scope.entity_id,
            )?;
            residue.advance_ids = artifacts.advance_ids;
            residue.advance_retry_command_ids = artifacts.retry_command_ids;
        }
        Ok(residue)
    }

    /// 存活 incoming references（REQ-DEL-03）：Story/Design 被 Design、Plan、
    /// 生命周期 WI 或发布 lineage 引用即列（不得仅凭当前摘要放行）；Plan 的
    /// 共享 run/VP/profile 被外部实体配置引用同样列出（协调层
    /// `entity_referenced` 拒删素材）。删除记录本身不构成业务引用。
    pub fn list_incoming_entity_references(
        &self,
        scope: &EntityScope,
    ) -> Result<Vec<EntityReference>, ProductStoreError> {
        scope.validate()?;
        let (project_id, issue_id) = (scope.project_id.as_str(), scope.issue_id.as_str());
        let mut references: Vec<EntityReference> = Vec::new();
        match scope.kind {
            DeletionEntityKind::Story | DeletionEntityKind::Design => {
                // Story 被 Design.story_spec_ids 引用；Design 无上游 Design。
                if scope.kind == DeletionEntityKind::Story {
                    for design in self.list_design_specs(project_id, issue_id)? {
                        if design
                            .story_spec_ids
                            .iter()
                            .any(|id| id == &scope.entity_id)
                        {
                            references.push(EntityReference {
                                kind: "design_spec".to_string(),
                                id: design.id,
                            });
                        }
                    }
                }
                for plan in self.list_issue_work_item_plans(project_id, issue_id)? {
                    let referenced = match scope.kind {
                        DeletionEntityKind::Story => plan
                            .source_story_spec_ids
                            .iter()
                            .any(|id| id == &scope.entity_id),
                        DeletionEntityKind::Design => plan
                            .source_design_spec_ids
                            .iter()
                            .any(|id| id == &scope.entity_id),
                        DeletionEntityKind::Plan => false,
                    };
                    if referenced {
                        references.push(EntityReference {
                            kind: "issue_work_item_plan".to_string(),
                            id: plan.id,
                        });
                    }
                }
                for item in self.list_work_items(project_id, issue_id)? {
                    let referenced = match scope.kind {
                        DeletionEntityKind::Story => {
                            item.story_spec_ids.iter().any(|id| id == &scope.entity_id)
                        }
                        DeletionEntityKind::Design => {
                            item.design_spec_ids.iter().any(|id| id == &scope.entity_id)
                        }
                        DeletionEntityKind::Plan => false,
                    };
                    if referenced {
                        references.push(EntityReference {
                            kind: "work_item".to_string(),
                            id: item.id,
                        });
                    }
                }
                // 发布 lineage（work-item-revisions/{plan}/lineage.json）：
                // 即使当前 Plan 摘要已不包含该 ID 也阻挡（REQ-DEL-03）。
                for lineage in self.list_plan_lineage_records(project_id, issue_id)? {
                    let referenced = match scope.kind {
                        DeletionEntityKind::Story => lineage
                            .story_spec_refs
                            .iter()
                            .any(|id| id == &scope.entity_id),
                        DeletionEntityKind::Design => lineage
                            .design_spec_refs
                            .iter()
                            .any(|id| id == &scope.entity_id),
                        DeletionEntityKind::Plan => false,
                    };
                    if referenced {
                        references.push(EntityReference {
                            kind: "plan_lineage".to_string(),
                            id: lineage.id,
                        });
                    }
                }
            }
            DeletionEntityKind::Plan => {
                let manifest = self.collect_entity_deletion_manifest(scope)?;
                for plan in self.list_issue_work_item_plans(project_id, issue_id)? {
                    if plan.id == scope.entity_id {
                        continue;
                    }
                    let shares_run = plan
                        .created_from_provider_run
                        .as_ref()
                        .is_some_and(|run| manifest.provider_run_ids.contains(run));
                    let shares_profile = plan
                        .repository_profile_ref
                        .as_ref()
                        .is_some_and(|profile| manifest.repository_profile_ids.contains(profile));
                    let shares_vp = plan
                        .verification_plan_ids
                        .iter()
                        .any(|vp| manifest.verification_plan_ids.contains(vp));
                    if shares_run || shares_profile || shares_vp {
                        references.push(EntityReference {
                            kind: "issue_work_item_plan".to_string(),
                            id: plan.id,
                        });
                    }
                }
                for item in self.list_work_items(project_id, issue_id)? {
                    let owned = item.source_work_item_plan_id.as_deref() == Some(&scope.entity_id);
                    if !owned
                        && item
                            .verification_plan_ref
                            .as_ref()
                            .is_some_and(|vp| manifest.verification_plan_ids.contains(vp))
                    {
                        references.push(EntityReference {
                            kind: "work_item".to_string(),
                            id: item.id,
                        });
                    }
                }
            }
        }
        references.sort_by(|left, right| (&left.kind, &left.id).cmp(&(&right.kind, &right.id)));
        references.dedup_by(|left, right| left.kind == right.kind && left.id == right.id);
        Ok(references)
    }

    /// 精准 purge（§0.2/Step 3 依赖序）：review/run/session 审计先于 refs 来源
    /// 记录；advance 闭包（retry→journal→record）由 D1.2c `purge_plan_artifacts`
    /// 提供；owned WI/VP/profile 随后；versions 固定子树按 scope 清；对象缺失
    /// =已清（幂等重入），身份不符/JSON 错误 fail-closed。
    pub fn purge_entity_owned_children(
        &self,
        lease: &EntityDeletionLease,
        manifest: &EntityDeletionManifest,
    ) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        if &manifest.scope != scope {
            return Err(ProductStoreError::InvalidRecord {
                kind: "entity_deletion_manifest",
                reason: "manifest scope must match the deletion lease".to_string(),
            });
        }
        scope.validate()?;
        for id in manifest
            .session_ids
            .iter()
            .chain(manifest.work_item_ids.iter())
            .chain(manifest.review_ids.iter())
            .chain(manifest.provider_run_ids.iter())
            .chain(manifest.verification_plan_ids.iter())
            .chain(manifest.repository_profile_ids.iter())
        {
            validate_relative_id(id)?;
        }
        let lifecycle_root = self
            .paths
            .issue_lifecycle_root(&scope.project_id, &scope.issue_id);
        let sessions_root = self.workspace_sessions_root(&scope.project_id, &scope.issue_id);

        // ① review 审计先于其 refs 来源（session 记录）删除。
        for review_id in &manifest.review_ids {
            let path = self
                .provider_review_rounds_root(&scope.project_id, &scope.issue_id)
                .join(format!("{review_id}.json"));
            if !path_is_regular_file(&path)? {
                continue;
            }
            let record: ProviderReviewRoundRecord = read_json(&path)?;
            if record.id != *review_id || !manifest.session_ids.contains(&record.session_id) {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "provider_review_round",
                    id: review_id.clone(),
                });
            }
            remove_file_if_exists(&path)?;
        }

        // ② provider-run 档案（refs 来源记录尚未删，目录按 manifest 精确清）。
        for run_id in &manifest.provider_run_ids {
            remove_dir_all_if_exists(
                &self
                    .provider_runs_root(&scope.project_id, &scope.issue_id)
                    .join(run_id),
            )?;
        }

        // ③ 两个全局 session 审计分区（§0.5 D1.1 行）先于 session 记录。
        for session_id in &manifest.session_ids {
            remove_dir_all_if_exists(
                &self
                    .paths
                    .root()
                    .join("work-item-split-runs")
                    .join(session_id),
            )?;
            remove_dir_all_if_exists(
                &self
                    .paths
                    .root()
                    .join("tool-policy-run-audit")
                    .join(session_id),
            )?;
        }

        // ④ timeline/session 附属目录/锁/记录。
        for session_id in &manifest.session_ids {
            remove_dir_all_if_exists(&lifecycle_root.join("workspace-timelines").join(session_id))?;
            remove_dir_all_if_exists(&sessions_root.join(session_id))?;
            remove_file_if_exists(&sessions_root.join(format!(".{session_id}.json.lock")))?;
            remove_file_if_exists(&sessions_root.join(format!("{session_id}.json")))?;
        }

        // ⑤ advance 闭包（D1.2c：retry→初始化 journal→record；专属 engine
        //    checkpoint 的按 session 清理在 D2.1d/D2.2，不动全局根）。
        if scope.kind == DeletionEntityKind::Plan {
            AdvanceStore::new(self.paths.clone()).purge_plan_artifacts(
                lease,
                &AdvancePlanArtifacts {
                    advance_ids: manifest.advance_ids.clone(),
                    retry_command_ids: manifest.advance_retry_command_ids.clone(),
                },
            )?;
        } else if !manifest.advance_ids.is_empty() || !manifest.advance_retry_command_ids.is_empty()
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "entity_deletion_manifest",
                reason: "advance artifacts only belong to plan entities".to_string(),
            });
        }

        // ⑥ owned child records（run 引用的 VP/profile 随后于 ②，WI 最后）。
        if scope.kind != DeletionEntityKind::Plan
            && (!manifest.work_item_ids.is_empty()
                || !manifest.verification_plan_ids.is_empty()
                || !manifest.repository_profile_ids.is_empty()
                || !manifest.target_repository_ids.is_empty())
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "entity_deletion_manifest",
                reason: "work item/verification/profile artifacts only belong to plan entities"
                    .to_string(),
            });
        }
        for verification_plan_id in &manifest.verification_plan_ids {
            let path = self
                .verification_plans_root(&scope.project_id, &scope.issue_id)
                .join(format!("{verification_plan_id}.json"));
            if !path_is_regular_file(&path)? {
                continue;
            }
            let record: VerificationPlan = read_json(&path)?;
            if record.id != *verification_plan_id
                || !manifest.work_item_ids.contains(&record.work_item_id)
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "verification_plan",
                    id: verification_plan_id.clone(),
                });
            }
            remove_file_if_exists(&path)?;
        }
        for profile_id in &manifest.repository_profile_ids {
            let path = self
                .repository_profiles_root(&scope.project_id, &scope.issue_id)
                .join(format!("{profile_id}.json"));
            if !path_is_regular_file(&path)? {
                continue;
            }
            let record: RepositoryProfile = read_json(&path)?;
            if record.id != *profile_id {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "repository_profile",
                    id: profile_id.clone(),
                });
            }
            remove_file_if_exists(&path)?;
        }
        for work_item_id in &manifest.work_item_ids {
            let path = self
                .work_items_root(&scope.project_id, &scope.issue_id)
                .join(format!("{work_item_id}.json"));
            if !path_is_regular_file(&path)? {
                continue;
            }
            let record: LifecycleWorkItemRecord = read_json(&path)?;
            // 重入期计划根可能已删：source==他 Plan 即归属错误；source 缺失的
            // 旧记录按捕获的 manifest（refs/身份索引）为准。
            if record
                .source_work_item_plan_id
                .as_deref()
                .is_some_and(|source| source != scope.entity_id)
            {
                return Err(ProductStoreError::IdentityMismatch {
                    kind: "work_item",
                    id: work_item_id.clone(),
                });
            }
            remove_file_if_exists(&path)?;
        }

        // ⑦ versions 固定子树（§0.5 Story/Design versions 行；scope 键控）。
        remove_dir_all_if_exists(&self.versions_root(
            &scope.project_id,
            &scope.issue_id,
            &scope.entity_id,
        ))?;
        Ok(())
    }

    /// 根最后（§0.2）：实体主记录删除；根缺=已清（不 404，REQ-DEL-07 重入）。
    pub fn delete_entity_root(&self, lease: &EntityDeletionLease) -> Result<(), ProductStoreError> {
        let scope = lease.scope();
        scope.validate()?;
        let (project_id, issue_id) = (scope.project_id.as_str(), scope.issue_id.as_str());
        let path = match scope.kind {
            DeletionEntityKind::Story => self
                .story_specs_root(project_id, issue_id)
                .join(format!("{}.json", scope.entity_id)),
            DeletionEntityKind::Design => self
                .design_specs_root(project_id, issue_id)
                .join(format!("{}.json", scope.entity_id)),
            DeletionEntityKind::Plan => self
                .issue_work_item_plans_root(project_id, issue_id)
                .join(format!("{}.json", scope.entity_id)),
        };
        if !path_is_regular_file(&path)? {
            return Ok(());
        }
        match scope.kind {
            DeletionEntityKind::Story => {
                let record: crate::product::models::StorySpecRecord = read_json(&path)?;
                ensure_entity_root_identity(
                    &record.id,
                    &record.project_id,
                    &record.issue_id,
                    scope,
                )?;
            }
            DeletionEntityKind::Design => {
                let record: crate::product::models::DesignSpecRecord = read_json(&path)?;
                ensure_entity_root_identity(
                    &record.id,
                    &record.project_id,
                    &record.issue_id,
                    scope,
                )?;
            }
            DeletionEntityKind::Plan => {
                let record: IssueWorkItemPlan = read_json(&path)?;
                ensure_entity_root_identity(
                    &record.id,
                    &record.project_id,
                    &record.issue_id,
                    scope,
                )?;
            }
        }
        remove_file_if_exists(&path)
    }
}

/// 扫描 per-issue provider-runs 目录,返回下一个与 1b handle 同族的
/// `ws-{project}-{issue}-split-run-NNNN` run id(空目录从 0001 起,取既有
/// 最大序号 +1;旧 `provider_run_split_NNNN` 名字不在新前缀名空间,不参与
/// 计数)。目录缺失视为空(fresh issue)。
fn next_ws_split_run_id(
    project_id: &str,
    issue_id: &str,
    root: &std::path::Path,
) -> std::io::Result<String> {
    let prefix = format!("ws-{project_id}-{issue_id}-split-run-");
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(format!("{prefix}0001"));
        }
        Err(error) => return Err(error),
    };
    let mut max = 0usize;
    for entry in entries {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let name = name.strip_suffix(".json").unwrap_or(&name);
        if let Some(suffix) = name.strip_prefix(&prefix)
            && let Ok(sequence) = suffix.parse::<usize>()
        {
            max = max.max(sequence);
        }
    }
    Ok(format!("{prefix}{:04}", max + 1))
}

/// `delete_entity_root` 的根记录身份复核（路径已定 scope，防错档；JSON/身份
/// 错误 fail-closed，不当已清）。
fn ensure_entity_root_identity(
    record_id: &str,
    record_project_id: &str,
    record_issue_id: &str,
    scope: &EntityScope,
) -> Result<(), ProductStoreError> {
    if record_id != scope.entity_id
        || record_project_id != scope.project_id
        || record_issue_id != scope.issue_id
    {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "entity_root",
            id: scope.entity_id.clone(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;

    /// Task 1c(carry④/Ruling 5B):legacy split 落档路径的运行身份与 1b
    /// `WorkItemSplitProviderRunHandle` 装配对齐——改铸与 handle 同族的
    /// `ws-<scope>-split-run-<seq>` 前缀(scope 取该路径仅有的 project/issue
    /// 二元组);run_ref 是不透明字符串,流经 created_from_provider_run/
    /// provider_run_ref 无任何格式解析,前缀族对齐不影响既有消费者,序号仍
    /// 按 per-issue 目录单调分配。旧 `provider_run_split_NNNN` 前缀不再铸造
    /// (先红:当前仍铸造旧前缀)。
    #[test]
    fn save_work_item_split_provider_run_mints_handle_family_run_ref() {
        let tmp = tempfile::tempdir().expect("tempdir");
        let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let structured = serde_json::json!({"work_items": []});

        let first = store
            .save_work_item_split_provider_run(
                "project_0001",
                "issue_0001",
                &ProviderName::ClaudeCode,
                "split prompt",
                &structured,
            )
            .expect("save first split provider run");
        assert_eq!(
            first, "ws-project_0001-issue_0001-split-run-0001",
            "legacy split 落档必须铸造与 1b handle 同族的 run_ref 前缀"
        );

        let second = store
            .save_work_item_split_provider_run(
                "project_0001",
                "issue_0001",
                &ProviderName::ClaudeCode,
                "split prompt",
                &structured,
            )
            .expect("save second split provider run");
        assert_eq!(
            second, "ws-project_0001-issue_0001-split-run-0002",
            "同 issue 内序号必须单调递增"
        );

        // run 记录落在该 id 目录,provider_run_ref 与铸造 id 一致(不透明流经)。
        let run_root = store.provider_runs_root("project_0001", "issue_0001");
        let run: serde_json::Value =
            crate::product::json_store::read_json(&run_root.join(&second).join("run.json"))
                .expect("read back run record");
        assert_eq!(
            run.get("provider_run_id").and_then(|value| value.as_str()),
            Some(second.as_str())
        );
    }
}
