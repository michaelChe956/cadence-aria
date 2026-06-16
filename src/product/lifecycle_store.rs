use std::fs;
use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::de::DeserializeOwned;

use crate::product::app_paths::ProductAppPaths;
use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::models::{
    DesignKind, DesignSpecRecord, IssueSharedWorktree, IssueSharedWorktreeStatus,
    IssueWorkItemDependencyEdge, IssueWorkItemPlan, IssueWorkItemPlanOptions,
    IssueWorkItemPlanStatus, LifecycleConfirmationStatus, LifecycleWorkItemRecord, NodeDetail,
    ProjectProviderDefaultsRecord, ProviderConversationRef, ProviderName,
    ProviderReviewRoundRecord, RepositoryProfile, RepositoryProfileConfidence, SpecVersionRecord,
    StorySpecRecord, VerificationCommand, VerificationFallbackPolicy, VerificationManualCheck,
    VerificationPlan, VerificationScope, WorkItemContextBudget, WorkItemExecutionPlanStatus,
    WorkItemKind, WorkItemPlanStatus, WorkItemStatus, WorkspaceMessageRecord,
    WorkspaceSessionRecord, WorkspaceSessionStatus, WorkspaceType,
};
use crate::web::workspace_ws_types::{ArtifactVersion, TimelineNode};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateStorySpecInput {
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateDesignSpecInput {
    pub project_id: String,
    pub issue_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_kind: DesignKind,
    pub title: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkItemInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub title: String,
    pub work_item_set_id: Option<String>,
    pub kind: WorkItemKind,
    pub sequence_hint: Option<u32>,
    pub depends_on: Vec<String>,
    pub exclusive_write_scopes: Vec<String>,
    pub forbidden_write_scopes: Vec<String>,
    pub context_budget: WorkItemContextBudget,
    pub required_handoff_from: Vec<String>,
    pub verification_plan_ref: Option<String>,
    pub require_execution_plan_confirm: bool,
    pub plan_status: WorkItemPlanStatus,
}

impl Default for CreateWorkItemInput {
    fn default() -> Self {
        Self {
            id: None,
            project_id: String::new(),
            issue_id: String::new(),
            repository_id: String::new(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: String::new(),
            work_item_set_id: None,
            kind: WorkItemKind::default(),
            sequence_hint: None,
            depends_on: Vec::new(),
            exclusive_write_scopes: Vec::new(),
            forbidden_write_scopes: Vec::new(),
            context_budget: WorkItemContextBudget::default(),
            required_handoff_from: Vec::new(),
            verification_plan_ref: None,
            require_execution_plan_confirm: false,
            plan_status: WorkItemPlanStatus::NotStarted,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateIssueWorkItemPlanInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub source_story_spec_ids: Vec<String>,
    pub source_design_spec_ids: Vec<String>,
    pub options: IssueWorkItemPlanOptions,
    pub status: IssueWorkItemPlanStatus,
    pub work_item_ids: Vec<String>,
    pub repository_profile_ref: Option<String>,
    pub verification_plan_ids: Vec<String>,
    pub dependency_graph: Vec<IssueWorkItemDependencyEdge>,
    pub created_from_provider_run: Option<String>,
    pub validator_findings: Vec<crate::product::models::WorkItemSplitFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRepositoryProfileInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub provider_run_ref: Option<String>,
    pub languages: Vec<String>,
    pub frameworks: Vec<String>,
    pub package_managers: Vec<String>,
    pub test_frameworks: Vec<String>,
    pub build_systems: Vec<String>,
    pub verification_capabilities: Vec<String>,
    pub detected_layers: Vec<String>,
    pub split_recommendation: String,
    pub confidence: RepositoryProfileConfidence,
    pub uncertainties: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateVerificationPlanInput {
    pub id: Option<String>,
    pub project_id: String,
    pub issue_id: String,
    pub work_item_id: String,
    pub repository_profile_ref: Option<String>,
    pub provider_run_ref: Option<String>,
    pub scope: VerificationScope,
    pub commands: Vec<VerificationCommand>,
    pub manual_checks: Vec<VerificationManualCheck>,
    pub required_gates: Vec<String>,
    pub risk_notes: Vec<String>,
    pub confidence: RepositoryProfileConfidence,
    pub fallback_policy: VerificationFallbackPolicy,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendSpecVersionInput {
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub markdown: String,
    pub provider_run_refs: Vec<String>,
    pub review_refs: Vec<String>,
    pub confirmed_by: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendProviderReviewRoundInput {
    pub project_id: String,
    pub issue_id: String,
    pub session_id: String,
    pub round_index: u32,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_result: String,
    pub revision_result: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateWorkspaceSessionInput {
    pub project_id: String,
    pub issue_id: String,
    pub entity_id: String,
    pub workspace_type: WorkspaceType,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateProjectProviderDefaultsInput {
    pub project_id: String,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
    pub review_rounds: u32,
    pub superpowers_enabled: bool,
    pub openspec_enabled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpsertIssueSharedWorktreeInput {
    pub project_id: String,
    pub issue_id: String,
    pub repository_id: String,
    pub branch_name: String,
    pub worktree_path: PathBuf,
    pub base_branch: String,
}

#[derive(Debug, Clone)]
pub struct LifecycleStore {
    paths: ProductAppPaths,
}

enum ExistingSpecRecord {
    Story {
        path: PathBuf,
        record: StorySpecRecord,
    },
    Design {
        path: PathBuf,
        record: DesignSpecRecord,
    },
}

impl LifecycleStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn create_story_spec(
        &self,
        input: CreateStorySpecInput,
    ) -> Result<StorySpecRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;

        let root = self.story_specs_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("story_spec", count_json_files(&root)?);
        let now = Utc::now().to_rfc3339();
        let story = StorySpecRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            repository_id: input.repository_id,
            title: input.title,
            current_version: None,
            confirmation_status: LifecycleConfirmationStatus::Draft,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &story)?;
        Ok(story)
    }

    pub fn list_story_specs(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<StorySpecRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.story_specs_root(project_id, issue_id))
    }

    pub fn delete_story_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        story_spec_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(story_spec_id)?;

        delete_required_file(
            &self
                .story_specs_root(project_id, issue_id)
                .join(format!("{story_spec_id}.json")),
            "story_spec",
            story_spec_id,
        )?;
        remove_dir_all_if_exists(&self.versions_root(project_id, issue_id, story_spec_id))?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            story_spec_id,
            WorkspaceType::Story,
        )
    }

    pub fn create_design_spec(
        &self,
        input: CreateDesignSpecInput,
    ) -> Result<DesignSpecRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_ids(&input.story_spec_ids)?;

        let root = self.design_specs_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("design_spec", count_json_files(&root)?);
        let now = Utc::now().to_rfc3339();
        let design = DesignSpecRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            story_spec_ids: input.story_spec_ids,
            design_kind: input.design_kind,
            title: input.title,
            current_version: None,
            confirmation_status: LifecycleConfirmationStatus::Draft,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &design)?;
        Ok(design)
    }

    pub fn list_design_specs(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<DesignSpecRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.design_specs_root(project_id, issue_id))
    }

    pub fn delete_design_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        design_spec_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(design_spec_id)?;

        delete_required_file(
            &self
                .design_specs_root(project_id, issue_id)
                .join(format!("{design_spec_id}.json")),
            "design_spec",
            design_spec_id,
        )?;
        remove_dir_all_if_exists(&self.versions_root(project_id, issue_id, design_spec_id))?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            design_spec_id,
            WorkspaceType::Design,
        )
    }

    pub fn create_work_item(
        &self,
        input: CreateWorkItemInput,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;
        validate_relative_ids(&input.story_spec_ids)?;
        validate_relative_ids(&input.design_spec_ids)?;

        let root = self.work_items_root(&input.project_id, &input.issue_id);
        let id = match input.id {
            Some(ref id) => {
                validate_relative_id(id)?;
                id.clone()
            }
            None => next_sequential_id("work_item", count_json_files(&root)?),
        };
        let now = Utc::now().to_rfc3339();
        let work_item = LifecycleWorkItemRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            repository_id: input.repository_id,
            story_spec_ids: input.story_spec_ids,
            design_spec_ids: input.design_spec_ids,
            title: input.title,
            plan_status: input.plan_status,
            execution_status: WorkItemStatus::Pending,
            worktree_path: None,
            work_item_set_id: input.work_item_set_id,
            kind: input.kind,
            sequence_hint: input.sequence_hint,
            depends_on: input.depends_on,
            exclusive_write_scopes: input.exclusive_write_scopes,
            forbidden_write_scopes: input.forbidden_write_scopes,
            context_budget: input.context_budget,
            required_handoff_from: input.required_handoff_from,
            verification_plan_ref: input.verification_plan_ref,
            require_execution_plan_confirm: input.require_execution_plan_confirm,
            execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
            handoff_summary_ref: None,
            completion_commit: None,
            completion_diff_summary_ref: None,
            created_at: now.clone(),
            updated_at: now,
        };

        write_json(&root.join(format!("{id}.json")), &work_item)?;
        Ok(work_item)
    }

    pub fn count_work_items(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<usize, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        count_json_files(&self.work_items_root(project_id, issue_id))
    }

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

    pub fn confirm_issue_work_item_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<(IssueWorkItemPlan, Vec<LifecycleWorkItemRecord>), ProductStoreError> {
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

    pub fn request_issue_work_item_plan_change(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
        _note: Option<String>,
    ) -> Result<(IssueWorkItemPlan, Vec<LifecycleWorkItemRecord>), ProductStoreError> {
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

    pub fn create_verification_plan(
        &self,
        input: CreateVerificationPlanInput,
    ) -> Result<VerificationPlan, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.work_item_id)?;

        let root = self.verification_plans_root(&input.project_id, &input.issue_id);
        let id = match input.id {
            Some(ref id) => {
                validate_relative_id(id)?;
                id.clone()
            }
            None => next_sequential_id("verification_plan", count_json_files(&root)?),
        };
        let now = Utc::now().to_rfc3339();
        let plan = VerificationPlan {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            work_item_id: input.work_item_id,
            repository_profile_ref: input.repository_profile_ref,
            provider_run_ref: input.provider_run_ref,
            scope: input.scope,
            commands: input.commands,
            manual_checks: input.manual_checks,
            required_gates: input.required_gates,
            risk_notes: input.risk_notes,
            confidence: input.confidence,
            fallback_policy: input.fallback_policy,
            created_at: now.clone(),
            updated_at: now,
        };
        write_json(&root.join(format!("{id}.json")), &plan)?;
        Ok(plan)
    }

    pub fn get_verification_plan(
        &self,
        project_id: &str,
        issue_id: &str,
        plan_id: &str,
    ) -> Result<VerificationPlan, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(plan_id)?;
        read_json(
            &self
                .verification_plans_root(project_id, issue_id)
                .join(format!("{plan_id}.json")),
        )
    }

    pub fn list_verification_plans(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<VerificationPlan>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.verification_plans_root(project_id, issue_id))
    }

    pub fn list_work_items(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Vec<LifecycleWorkItemRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        list_json_records(&self.work_items_root(project_id, issue_id))
    }

    pub fn delete_work_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        delete_required_file(
            &self
                .work_items_root(project_id, issue_id)
                .join(format!("{work_item_id}.json")),
            "work_item",
            work_item_id,
        )?;
        self.delete_workspace_sessions_for_entity(
            project_id,
            issue_id,
            work_item_id,
            WorkspaceType::WorkItem,
        )
    }

    pub fn append_version(
        &self,
        input: AppendSpecVersionInput,
    ) -> Result<SpecVersionRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.entity_id)?;

        let spec = self.load_existing_spec(&input.project_id, &input.issue_id, &input.entity_id)?;
        let root = self.versions_root(&input.project_id, &input.issue_id, &input.entity_id);
        let versions: Vec<SpecVersionRecord> = list_json_records(&root)?;
        let version = next_version_number(&versions)?;
        let id = next_sequential_id(
            "version",
            usize::try_from(version - 1).map_err(|_| {
                ProductStoreError::Io(format!("version sequence overflow: {version}"))
            })?,
        );
        let now = Utc::now().to_rfc3339();
        let record = SpecVersionRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            entity_id: input.entity_id,
            version,
            markdown: input.markdown,
            provider_run_refs: input.provider_run_refs,
            review_refs: input.review_refs,
            confirmed_by: input.confirmed_by,
            created_at: now.clone(),
        };

        let target_path = root.join(format!("{id}.json"));
        ensure_target_absent(&target_path)?;
        write_json(&target_path, &record)?;
        self.update_spec_current_version(spec, version, now)?;
        Ok(record)
    }

    pub fn list_versions(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
    ) -> Result<Vec<SpecVersionRecord>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(entity_id)?;
        list_json_records(&self.versions_root(project_id, issue_id, entity_id))
    }

    pub fn append_provider_review_round(
        &self,
        input: AppendProviderReviewRoundInput,
    ) -> Result<ProviderReviewRoundRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.session_id)?;

        let root = self.provider_review_rounds_root(&input.project_id, &input.issue_id);
        let id = next_sequential_id("review_round", count_json_files(&root)?);
        let record = ProviderReviewRoundRecord {
            id: id.clone(),
            project_id: input.project_id,
            issue_id: input.issue_id,
            session_id: input.session_id,
            round_index: input.round_index,
            author_provider: input.author_provider,
            reviewer_provider: input.reviewer_provider,
            review_result: input.review_result,
            revision_result: input.revision_result,
            created_at: Utc::now().to_rfc3339(),
        };

        let target_path = root.join(format!("{id}.json"));
        ensure_target_absent(&target_path)?;
        write_json(&target_path, &record)?;
        Ok(record)
    }

    pub fn create_workspace_session(
        &self,
        input: CreateWorkspaceSessionInput,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.entity_id)?;

        let root = self.workspace_sessions_root(&input.project_id, &input.issue_id);
        let id = self.next_workspace_session_id()?;
        let now = Utc::now().to_rfc3339();
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
            superpowers_enabled: input.superpowers_enabled,
            openspec_enabled: input.openspec_enabled,
            provider_conversations: Vec::new(),
            messages: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        };

        let target_path = root.join(format!("{id}.json"));
        ensure_target_absent(&target_path)?;
        write_json(&target_path, &session)?;
        Ok(session)
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

    pub fn get_workspace_session(
        &self,
        session_id: &str,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError> {
        validate_relative_id(session_id)?;
        read_json(&self.find_workspace_session_path(session_id)?)
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
        session.updated_at = Utc::now().to_rfc3339();
        write_json(&session_path, &session)?;
        Ok(session)
    }

    pub fn update_workspace_session_providers(
        &self,
        session_id: &str,
        author_provider: ProviderName,
        reviewer_provider: ProviderName,
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

    pub fn save_node_detail(
        &self,
        session_id: &str,
        node_id: &str,
        detail: &NodeDetail,
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
    ) -> Result<NodeDetail, ProductStoreError> {
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

    pub fn update_spec_confirmation_status(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
        status: LifecycleConfirmationStatus,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(entity_id)?;

        let spec = self.load_existing_spec(project_id, issue_id, entity_id)?;
        let updated_at = Utc::now().to_rfc3339();
        match spec {
            ExistingSpecRecord::Story {
                path,
                record: mut story,
            } => {
                story.confirmation_status = status;
                story.updated_at = updated_at;
                write_json(&path, &story)
            }
            ExistingSpecRecord::Design {
                path,
                record: mut design,
            } => {
                design.confirmation_status = status;
                design.updated_at = updated_at;
                write_json(&path, &design)
            }
        }
    }

    pub fn update_work_item_plan_status(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        plan_status: WorkItemPlanStatus,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        let path = self
            .work_items_root(project_id, issue_id)
            .join(format!("{work_item_id}.json"));
        if !path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "work_item",
                id: work_item_id.to_string(),
            });
        }

        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.plan_status = plan_status;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn update_work_item_execution_status(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        execution_status: WorkItemStatus,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let path = self.work_item_path(project_id, issue_id, work_item_id)?;
        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.execution_status = execution_status;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn update_work_item_execution_plan_status(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        execution_plan_status: WorkItemExecutionPlanStatus,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let path = self.work_item_path(project_id, issue_id, work_item_id)?;
        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.execution_plan_status = execution_plan_status;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn update_work_item_handoff_summary(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        handoff_summary_ref: Option<String>,
        completion_commit: Option<String>,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let path = self.work_item_path(project_id, issue_id, work_item_id)?;
        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.handoff_summary_ref = handoff_summary_ref;
        record.completion_commit = completion_commit;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn update_work_item_worktree_path(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
        worktree_path: Option<PathBuf>,
    ) -> Result<LifecycleWorkItemRecord, ProductStoreError> {
        let path = self.work_item_path(project_id, issue_id, work_item_id)?;
        let mut record: LifecycleWorkItemRecord = read_json(&path)?;
        record.worktree_path = worktree_path;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn upsert_project_provider_defaults(
        &self,
        input: CreateProjectProviderDefaultsInput,
    ) -> Result<ProjectProviderDefaultsRecord, ProductStoreError> {
        validate_relative_id(&input.project_id)?;

        let defaults = ProjectProviderDefaultsRecord {
            project_id: input.project_id,
            author_provider: input.author_provider,
            reviewer_provider: input.reviewer_provider,
            review_rounds: input.review_rounds,
            superpowers_enabled: input.superpowers_enabled,
            openspec_enabled: input.openspec_enabled,
            updated_at: Utc::now().to_rfc3339(),
        };

        write_json(
            &self
                .paths
                .project_provider_defaults_path(&defaults.project_id),
            &defaults,
        )?;
        Ok(defaults)
    }

    pub fn upsert_issue_shared_worktree(
        &self,
        input: UpsertIssueSharedWorktreeInput,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(&input.project_id)?;
        validate_relative_id(&input.issue_id)?;
        validate_relative_id(&input.repository_id)?;

        let path = self.issue_shared_worktree_path(&input.project_id, &input.issue_id);
        let now = Utc::now().to_rfc3339();
        let record = if path_is_regular_file(&path)? {
            let mut existing: IssueSharedWorktree = read_json(&path)?;
            existing.branch_name = input.branch_name;
            existing.worktree_path = input.worktree_path;
            existing.base_branch = input.base_branch;
            existing.updated_at = now.clone();
            existing
        } else {
            IssueSharedWorktree {
                id: format!(
                    "issue_shared_worktree_{}_{}",
                    input.project_id, input.issue_id
                ),
                project_id: input.project_id,
                issue_id: input.issue_id,
                repository_id: input.repository_id,
                branch_name: input.branch_name,
                worktree_path: input.worktree_path,
                base_branch: input.base_branch,
                status: IssueSharedWorktreeStatus::Ready,
                current_active_work_item_id: None,
                last_completed_work_item_id: None,
                created_at: now.clone(),
                updated_at: now,
            }
        };

        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn get_issue_shared_worktree(
        &self,
        project_id: &str,
        issue_id: &str,
    ) -> Result<Option<IssueSharedWorktree>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;

        let path = self.issue_shared_worktree_path(project_id, issue_id);
        if !path_is_regular_file(&path)? {
            return Ok(None);
        }
        read_json(&path).map(Some)
    }

    pub fn try_acquire_issue_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        let path = self.issue_shared_worktree_path(project_id, issue_id);
        let mut record: IssueSharedWorktree = read_json(&path).map_err(|error| match error {
            ProductStoreError::NotFound { .. } => ProductStoreError::NotFound {
                kind: "issue_shared_worktree",
                id: format!("{project_id}/{issue_id}"),
            },
            other => other,
        })?;

        if let Some(active_id) = &record.current_active_work_item_id {
            if active_id != work_item_id {
                return Err(ProductStoreError::Io(format!(
                    "issue_worktree_active: issue {issue_id} locked by {active_id}"
                )));
            }
            return Ok(record);
        }

        record.current_active_work_item_id = Some(work_item_id.to_string());
        record.status = IssueSharedWorktreeStatus::Running;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    pub fn release_issue_worktree_lock(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        let path = self.issue_shared_worktree_path(project_id, issue_id);
        let mut record: IssueSharedWorktree = read_json(&path).map_err(|error| match error {
            ProductStoreError::NotFound { .. } => ProductStoreError::NotFound {
                kind: "issue_shared_worktree",
                id: format!("{project_id}/{issue_id}"),
            },
            other => other,
        })?;

        if record.current_active_work_item_id.as_deref() == Some(work_item_id) {
            record.current_active_work_item_id = None;
            record.status = IssueSharedWorktreeStatus::Ready;
            record.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &record)?;
        }

        Ok(record)
    }

    pub fn mark_issue_worktree_completed_item(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<IssueSharedWorktree, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;

        let path = self.issue_shared_worktree_path(project_id, issue_id);
        let mut record: IssueSharedWorktree = read_json(&path).map_err(|error| match error {
            ProductStoreError::NotFound { .. } => ProductStoreError::NotFound {
                kind: "issue_shared_worktree",
                id: format!("{project_id}/{issue_id}"),
            },
            other => other,
        })?;

        record.last_completed_work_item_id = Some(work_item_id.to_string());
        if record.current_active_work_item_id.as_deref() == Some(work_item_id) {
            record.current_active_work_item_id = None;
        }
        record.status = IssueSharedWorktreeStatus::Ready;
        record.updated_at = Utc::now().to_rfc3339();
        write_json(&path, &record)?;
        Ok(record)
    }

    fn load_existing_spec(
        &self,
        project_id: &str,
        issue_id: &str,
        entity_id: &str,
    ) -> Result<ExistingSpecRecord, ProductStoreError> {
        let story_path = self
            .story_specs_root(project_id, issue_id)
            .join(format!("{entity_id}.json"));
        if path_is_regular_file(&story_path)? {
            let record = read_json(&story_path)?;
            return Ok(ExistingSpecRecord::Story {
                path: story_path,
                record,
            });
        }

        let design_path = self
            .design_specs_root(project_id, issue_id)
            .join(format!("{entity_id}.json"));
        if path_is_regular_file(&design_path)? {
            let record = read_json(&design_path)?;
            return Ok(ExistingSpecRecord::Design {
                path: design_path,
                record,
            });
        }

        Err(ProductStoreError::NotFound {
            kind: "spec",
            id: entity_id.to_string(),
        })
    }

    fn update_spec_current_version(
        &self,
        spec: ExistingSpecRecord,
        version: u32,
        updated_at: String,
    ) -> Result<(), ProductStoreError> {
        match spec {
            ExistingSpecRecord::Story {
                path,
                record: mut story,
            } => {
                story.current_version = Some(version);
                story.updated_at = updated_at;
                write_json(&path, &story)
            }
            ExistingSpecRecord::Design {
                path,
                record: mut design,
            } => {
                design.current_version = Some(version);
                design.updated_at = updated_at;
                write_json(&path, &design)
            }
        }
    }

    fn next_workspace_session_id(&self) -> Result<String, ProductStoreError> {
        let max_sequence = max_workspace_session_sequence(&self.paths.projects_root())?;
        Ok(next_sequential_id("workspace_session", max_sequence))
    }

    fn work_item_path(
        &self,
        project_id: &str,
        issue_id: &str,
        work_item_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(issue_id)?;
        validate_relative_id(work_item_id)?;
        let path = self
            .work_items_root(project_id, issue_id)
            .join(format!("{work_item_id}.json"));
        if !path_is_regular_file(&path)? {
            return Err(ProductStoreError::NotFound {
                kind: "work_item",
                id: work_item_id.to_string(),
            });
        }
        Ok(path)
    }

    fn find_workspace_session_path(&self, session_id: &str) -> Result<PathBuf, ProductStoreError> {
        let mut matched_path = None;
        for project_path in child_directories(&self.paths.projects_root())? {
            let issues_root = project_path.join("issues");
            for issue_path in child_directories(&issues_root)? {
                let workspace_sessions_root = issue_path.join("workspace-sessions");
                for session_path in workspace_session_file_paths(&workspace_sessions_root)? {
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
        }

        matched_path.ok_or_else(|| ProductStoreError::NotFound {
            kind: "workspace_session",
            id: session_id.to_string(),
        })
    }

    fn workspace_timeline_root_for_session(
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

    fn story_specs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("story-specs")
    }

    fn design_specs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("design-specs")
    }

    fn work_items_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("work-items")
    }

    fn versions_root(&self, project_id: &str, issue_id: &str, entity_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("versions")
            .join(entity_id)
    }

    fn workspace_sessions_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("workspace-sessions")
    }

    fn provider_review_rounds_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("provider-review-rounds")
    }

    fn issue_shared_worktree_path(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("issue-shared-worktree.json")
    }

    fn issue_work_item_plans_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("issue-work-item-plans")
    }

    fn repository_profiles_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("repository-profiles")
    }

    fn verification_plans_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("verification-plans")
    }

    fn provider_runs_root(&self, project_id: &str, issue_id: &str) -> PathBuf {
        self.paths
            .issue_lifecycle_root(project_id, issue_id)
            .join("provider-runs")
    }

    fn delete_workspace_sessions_for_entity(
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
}

fn list_json_records<T: DeserializeOwned>(path: &Path) -> Result<Vec<T>, ProductStoreError> {
    let entries = json_file_paths(path)?;

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        records.push(read_json(&entry)?);
    }
    Ok(records)
}

fn count_json_files(path: &Path) -> Result<usize, ProductStoreError> {
    Ok(json_file_paths(path)?.len())
}

fn json_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        let entry_path = entry.path();
        if file_type.is_file()
            && entry_path.extension().and_then(|value| value.to_str()) == Some("json")
        {
            entries.push(entry_path);
        }
    }
    entries.sort();
    Ok(entries)
}

fn list_workspace_session_records(
    path: &Path,
) -> Result<Vec<WorkspaceSessionRecord>, ProductStoreError> {
    let entries = workspace_session_file_paths(path)?;

    let mut records = Vec::with_capacity(entries.len());
    for entry in entries {
        if let Some(record) = read_workspace_session_record(&entry)? {
            records.push(record);
        }
    }
    Ok(records)
}

fn workspace_session_file_paths(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    Ok(json_file_paths(path)?
        .into_iter()
        .filter(|path| workspace_session_file_stem(path).is_some())
        .collect())
}

fn read_workspace_session_record(
    path: &Path,
) -> Result<Option<WorkspaceSessionRecord>, ProductStoreError> {
    let Some(file_id) = workspace_session_file_stem(path) else {
        return Ok(None);
    };
    let session: WorkspaceSessionRecord = read_json(path)?;
    if session.id == file_id {
        Ok(Some(session))
    } else {
        Ok(None)
    }
}

fn workspace_session_file_stem(path: &Path) -> Option<&str> {
    let stem = path.file_stem()?.to_str()?;
    let suffix = stem.strip_prefix("workspace_session_")?;
    if suffix.is_empty() {
        return None;
    }
    Some(stem)
}

fn child_directories(path: &Path) -> Result<Vec<PathBuf>, ProductStoreError> {
    if !path_exists(path)? {
        return Ok(Vec::new());
    }

    let mut entries = Vec::new();
    for entry in fs::read_dir(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", path.display()))
        })?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!(
                "read {} entry type: {error}",
                entry.path().display()
            ))
        })?;
        if file_type.is_dir() {
            entries.push(entry.path());
        }
    }
    entries.sort();
    Ok(entries)
}

fn next_version_number(records: &[SpecVersionRecord]) -> Result<u32, ProductStoreError> {
    records
        .iter()
        .map(|record| record.version)
        .max()
        .unwrap_or(0)
        .checked_add(1)
        .ok_or_else(|| ProductStoreError::Io("version sequence overflow".to_string()))
}

fn max_workspace_session_sequence(projects_root: &Path) -> Result<usize, ProductStoreError> {
    let mut max_sequence = 0usize;
    for project_path in child_directories(projects_root)? {
        let issues_root = project_path.join("issues");
        for issue_path in child_directories(&issues_root)? {
            let workspace_sessions_root = issue_path.join("workspace-sessions");
            for session_path in workspace_session_file_paths(&workspace_sessions_root)? {
                let Some(session) = read_workspace_session_record(&session_path)? else {
                    continue;
                };
                if let Some(sequence) = parse_sequential_id(&session.id, "workspace_session") {
                    max_sequence = max_sequence.max(sequence);
                }
            }
        }
    }
    Ok(max_sequence)
}

fn parse_sequential_id(value: &str, prefix: &str) -> Option<usize> {
    value
        .strip_prefix(prefix)
        .and_then(|suffix| suffix.strip_prefix('_'))
        .and_then(|suffix| suffix.parse().ok())
}

fn ensure_target_absent(path: &Path) -> Result<(), ProductStoreError> {
    if path_exists(path)? {
        return Err(ProductStoreError::Io(format!(
            "refuse to overwrite {}",
            path.display()
        )));
    }
    Ok(())
}

fn delete_required_file(
    path: &Path,
    kind: &'static str,
    id: &str,
) -> Result<(), ProductStoreError> {
    if !path_is_regular_file(path)? {
        return Err(ProductStoreError::NotFound {
            kind,
            id: id.to_string(),
        });
    }
    remove_file_if_exists(path)
}

fn remove_file_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

fn remove_dir_all_if_exists(path: &Path) -> Result<(), ProductStoreError> {
    match fs::remove_dir_all(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ProductStoreError::Io(format!(
            "remove {}: {error}",
            path.display()
        ))),
    }
}

fn path_is_regular_file(path: &Path) -> Result<bool, ProductStoreError> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(false),
        Err(error) => Err(ProductStoreError::Io(format!(
            "metadata {}: {error}",
            path.display()
        ))),
    }
}

fn validate_relative_ids(values: &[String]) -> Result<(), ProductStoreError> {
    for value in values {
        validate_relative_id(value)?;
    }
    Ok(())
}

fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("try_exists {}: {error}", path.display())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const REPOSITORY_ID: &str = "repository_0001";

    fn setup() -> (TempDir, LifecycleStore) {
        let tmp = TempDir::new().unwrap();
        let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        (tmp, store)
    }

    fn create_session(
        store: &LifecycleStore,
        entity_id: &str,
        workspace_type: WorkspaceType,
    ) -> WorkspaceSessionRecord {
        store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: entity_id.to_string(),
                workspace_type,
                author_provider: ProviderName::Codex,
                reviewer_provider: ProviderName::ClaudeCode,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            })
            .unwrap()
    }

    #[test]
    fn delete_story_spec_removes_record_versions_session_and_timeline() {
        let (_tmp, store) = setup();
        let story = store
            .create_story_spec(CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                title: "Session expired story".to_string(),
            })
            .unwrap();
        store
            .append_version(AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: story.id.clone(),
                markdown: "story markdown".to_string(),
                provider_run_refs: vec![],
                review_refs: vec![],
                confirmed_by: None,
            })
            .unwrap();
        let session = create_session(&store, &story.id, WorkspaceType::Story);
        store.save_timeline_nodes(&session.id, &[]).unwrap();
        let versions_root = store.versions_root(PROJECT_ID, ISSUE_ID, &story.id);
        let timeline_root = store
            .workspace_timeline_root_for_session(&session.id)
            .unwrap();

        store
            .delete_story_spec(PROJECT_ID, ISSUE_ID, &story.id)
            .unwrap();

        assert!(
            store
                .list_story_specs(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_versions(PROJECT_ID, ISSUE_ID, &story.id)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .is_empty()
        );
        assert!(!versions_root.exists());
        assert!(!timeline_root.exists());
    }

    #[test]
    fn delete_design_spec_removes_record_versions_session_and_timeline() {
        let (_tmp, store) = setup();
        let design = store
            .create_design_spec(CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec!["story_spec_0001".to_string()],
                design_kind: DesignKind::Frontend,
                title: "Frontend design".to_string(),
            })
            .unwrap();
        store
            .append_version(AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: design.id.clone(),
                markdown: "design markdown".to_string(),
                provider_run_refs: vec![],
                review_refs: vec![],
                confirmed_by: None,
            })
            .unwrap();
        let session = create_session(&store, &design.id, WorkspaceType::Design);
        store.save_timeline_nodes(&session.id, &[]).unwrap();
        let versions_root = store.versions_root(PROJECT_ID, ISSUE_ID, &design.id);
        let timeline_root = store
            .workspace_timeline_root_for_session(&session.id)
            .unwrap();

        store
            .delete_design_spec(PROJECT_ID, ISSUE_ID, &design.id)
            .unwrap();

        assert!(
            store
                .list_design_specs(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_versions(PROJECT_ID, ISSUE_ID, &design.id)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .is_empty()
        );
        assert!(!versions_root.exists());
        assert!(!timeline_root.exists());
    }

    #[test]
    fn delete_work_item_removes_record_session_and_timeline() {
        let (_tmp, store) = setup();
        let work_item = store
            .create_work_item(CreateWorkItemInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                story_spec_ids: vec!["story_spec_0001".to_string()],
                design_spec_ids: vec!["design_spec_0001".to_string()],
                title: "Implement prompt component".to_string(),
                ..Default::default()
            })
            .unwrap();
        let session = create_session(&store, &work_item.id, WorkspaceType::WorkItem);
        store.save_timeline_nodes(&session.id, &[]).unwrap();
        let timeline_root = store
            .workspace_timeline_root_for_session(&session.id)
            .unwrap();

        store
            .delete_work_item(PROJECT_ID, ISSUE_ID, &work_item.id)
            .unwrap();

        assert!(
            store
                .list_work_items(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .is_empty()
        );
        assert!(
            store
                .list_workspace_sessions(PROJECT_ID, ISSUE_ID)
                .unwrap()
                .is_empty()
        );
        assert!(!timeline_root.exists());
    }
}
