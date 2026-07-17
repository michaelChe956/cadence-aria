use chrono::Utc;

use crate::product::json_store::{ProductStoreError, validate_relative_id, write_json};
use crate::product::models::{
    LogicalWorkItem, VerificationPlanRevision, WorkItemDraftRevision, WorkItemDraftRevisionState,
    WorkItemDraftRevisionStatus, WorkItemPlanLineage, WorkItemRevision,
};

use super::{
    WorkItemRevisionStore, identity_mismatch, path_exists, read_required_json, with_exclusive_lock,
    write_immutable,
};

impl WorkItemRevisionStore {
    pub fn put_logical_work_item(
        &self,
        plan: &WorkItemPlanLineage,
        value: &LogicalWorkItem,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.plan_id)?;
        if value.plan_id != plan.id {
            return Err(identity_mismatch("logical_work_item", &value.id));
        }
        if let Some(revision_id) = value.active_revision_id.as_deref() {
            validate_relative_id(revision_id)?;
        }
        write_immutable(
            &self.logical_work_item_path(&plan.project_id, &plan.issue_id, &plan.id, &value.id),
            "logical_work_item",
            &value.id,
            value,
        )
    }

    pub fn set_active_work_item_revision(
        &self,
        plan: &WorkItemPlanLineage,
        logical_work_item: &LogicalWorkItem,
        expected_revision_id: Option<&str>,
        next_revision_id: &str,
    ) -> Result<LogicalWorkItem, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(next_revision_id)?;
        if let Some(expected_revision_id) = expected_revision_id {
            validate_relative_id(expected_revision_id)?;
        }
        if logical_work_item.plan_id != plan.id {
            return Err(identity_mismatch(
                "logical_work_item",
                &logical_work_item.id,
            ));
        }
        self.get_work_item_revision(plan, &logical_work_item.id, next_revision_id)?;
        let path = self.logical_work_item_path(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
            &logical_work_item.id,
        );
        with_exclusive_lock(&path, || {
            let mut stored = self.get_logical_work_item(plan, &logical_work_item.id)?;
            if stored.plan_id != plan.id
                || stored.id != logical_work_item.id
                || stored.active_revision_id.as_deref() != expected_revision_id
            {
                return Err(identity_mismatch(
                    "active_work_item_revision",
                    &logical_work_item.id,
                ));
            }
            stored.active_revision_id = Some(next_revision_id.to_string());
            stored.updated_at = Utc::now().to_rfc3339();
            write_json(&path, &stored)?;
            Ok(stored)
        })
    }

    pub fn put_draft_revision(
        &self,
        plan: &WorkItemPlanLineage,
        value: &WorkItemDraftRevision,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.logical_work_item_id)?;
        self.get_logical_work_item(plan, &value.logical_work_item_id)?;
        let revision_path =
            self.draft_revision_path(&plan.project_id, &plan.issue_id, &plan.id, &value.id);
        write_immutable(&revision_path, "work_item_draft_revision", &value.id, value)?;

        let state_path =
            self.draft_revision_state_path(&plan.project_id, &plan.issue_id, &plan.id, &value.id);
        if !path_exists(&state_path)? {
            write_json(
                &state_path,
                &WorkItemDraftRevisionState {
                    draft_revision_id: value.id.clone(),
                    status: WorkItemDraftRevisionStatus::Drafting,
                    updated_at: Utc::now().to_rfc3339(),
                },
            )?;
        }
        Ok(())
    }

    pub fn update_draft_revision_state(
        &self,
        plan: &WorkItemPlanLineage,
        draft_revision_id: &str,
        status: WorkItemDraftRevisionStatus,
    ) -> Result<WorkItemDraftRevisionState, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(draft_revision_id)?;
        let draft: WorkItemDraftRevision = read_required_json(
            &self.draft_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                draft_revision_id,
            ),
            "work_item_draft_revision",
            draft_revision_id,
        )?;
        if draft.id != draft_revision_id {
            return Err(identity_mismatch(
                "work_item_draft_revision",
                draft_revision_id,
            ));
        }
        let state = WorkItemDraftRevisionState {
            draft_revision_id: draft_revision_id.to_string(),
            status,
            updated_at: Utc::now().to_rfc3339(),
        };
        write_json(
            &self.draft_revision_state_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                draft_revision_id,
            ),
            &state,
        )?;
        Ok(state)
    }

    pub fn put_work_item_revision(
        &self,
        plan: &WorkItemPlanLineage,
        value: &WorkItemRevision,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.logical_work_item_id)?;
        self.get_logical_work_item(plan, &value.logical_work_item_id)?;
        write_immutable(
            &self.work_item_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.logical_work_item_id,
                &value.id,
            ),
            "work_item_revision",
            &value.id,
            value,
        )
    }

    pub fn get_work_item_revision(
        &self,
        plan: &WorkItemPlanLineage,
        logical_work_item_id: &str,
        revision_id: &str,
    ) -> Result<WorkItemRevision, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(logical_work_item_id)?;
        validate_relative_id(revision_id)?;
        let value: WorkItemRevision = read_required_json(
            &self.work_item_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                logical_work_item_id,
                revision_id,
            ),
            "work_item_revision",
            revision_id,
        )?;
        if value.id != revision_id || value.logical_work_item_id != logical_work_item_id {
            return Err(identity_mismatch("work_item_revision", revision_id));
        }
        Ok(value)
    }

    pub fn put_verification_plan_revision(
        &self,
        plan: &WorkItemPlanLineage,
        value: &VerificationPlanRevision,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_relative_id(&value.logical_work_item_id)?;
        self.get_logical_work_item(plan, &value.logical_work_item_id)?;
        write_immutable(
            &self.verification_plan_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "verification_plan_revision",
            &value.id,
            value,
        )
    }

    pub fn get_verification_plan_revision(
        &self,
        plan: &WorkItemPlanLineage,
        revision_id: &str,
    ) -> Result<VerificationPlanRevision, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(revision_id)?;
        let value: VerificationPlanRevision = read_required_json(
            &self.verification_plan_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                revision_id,
            ),
            "verification_plan_revision",
            revision_id,
        )?;
        if value.id != revision_id {
            return Err(identity_mismatch("verification_plan_revision", revision_id));
        }
        validate_relative_id(&value.logical_work_item_id)?;
        Ok(value)
    }

    pub(super) fn get_logical_work_item(
        &self,
        plan: &WorkItemPlanLineage,
        logical_work_item_id: &str,
    ) -> Result<LogicalWorkItem, ProductStoreError> {
        validate_relative_id(logical_work_item_id)?;
        let value: LogicalWorkItem = read_required_json(
            &self.logical_work_item_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                logical_work_item_id,
            ),
            "logical_work_item",
            logical_work_item_id,
        )?;
        if value.id != logical_work_item_id || value.plan_id != plan.id {
            return Err(identity_mismatch("logical_work_item", logical_work_item_id));
        }
        Ok(value)
    }
}
