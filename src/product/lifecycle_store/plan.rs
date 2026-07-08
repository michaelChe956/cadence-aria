use chrono::Utc;

use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    IssueWorkItemPlan, IssueWorkItemPlanStatus, ProviderName, RepositoryProfile,
    WorkItemPlanStatus, WorkspaceSessionRecord, WorkspaceType,
};
use crate::product::work_item_split_engine::WorkItemSplitProviderOutput;

use super::{
    CreateIssueWorkItemPlanInput, CreateRepositoryProfileInput, CreateVerificationPlanInput,
    CreateWorkItemInput, IssueWorkItemPlanUpdate, LifecycleStore, WorkItemPlanCandidateSnapshot,
    child_directories, count_json_files, delete_required_file, list_json_records,
    remove_file_if_exists, validate_relative_ids,
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

        let root = self.issue_work_item_plans_root(&input.project_id, &input.issue_id);
        let id = match input.id {
            Some(ref id) => {
                validate_relative_id(id)?;
                id.clone()
            }
            None => next_sequential_id("issue_work_item_plan", count_json_files(&root)?),
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
                reviewer_provider: reviewer_provider.clone().unwrap_or(ProviderName::Codex),
                review_rounds,
                superpowers_enabled,
                openspec_enabled,
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
    }

    pub fn update_issue_work_item_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        update: IssueWorkItemPlanUpdate,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
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
    }

    pub fn commit_issue_work_item_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        update: IssueWorkItemPlanUpdate,
    ) -> Result<IssueWorkItemPlan, ProductStoreError> {
        let mut plan = self.update_issue_work_item_plan(project_id, issue_id, plan_id, update)?;
        plan.status = IssueWorkItemPlanStatus::Confirmed;
        plan.updated_at = Utc::now().to_rfc3339();
        let path = self
            .issue_work_item_plans_root(project_id, issue_id)
            .join(format!("{plan_id}.json"));
        write_json(&path, &plan)?;
        Ok(plan)
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
        let mut plan = snapshot.clone();
        plan.updated_at = Utc::now().to_rfc3339();
        let path = self
            .issue_work_item_plans_root(project_id, issue_id)
            .join(format!("{plan_id}.json"));
        write_json(&path, &plan)?;
        Ok(plan)
    }

    pub fn replace_issue_work_item_plan_candidate(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        output: &WorkItemSplitProviderOutput,
        validator_findings: Vec<crate::product::models::WorkItemSplitFinding>,
    ) -> Result<WorkItemPlanCandidateSnapshot, ProductStoreError> {
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
                planned_handoff_summary: wi.planned_handoff_summary.clone(),
                kind: wi.kind.clone(),
                sequence_hint: wi.sequence_hint,
                depends_on: wi.depends_on.clone(),
                exclusive_write_scopes: wi.exclusive_write_scopes.clone(),
                forbidden_write_scopes: wi.forbidden_write_scopes.clone(),
                context_budget: wi.context_budget.clone(),
                required_handoff_from: wi.required_handoff_from.clone(),
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
    }

    pub fn create_repository_profile(
        &self,
        input: CreateRepositoryProfileInput,
    ) -> Result<RepositoryProfile, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;

        let root = self.repository_profiles_root(&input.project_id, &input.issue_id);
        let id = match input.id {
            Some(ref id) => {
                validate_relative_id(id)?;
                id.clone()
            }
            None => next_sequential_id("repository_profile", count_json_files(&root)?),
        };
        let now = Utc::now().to_rfc3339();
        let profile = RepositoryProfile {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            repository_id: input.repository_id,
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
        let existing = child_directories(&root)?.len();
        let id = next_sequential_id("provider_run_split", existing);
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
}
