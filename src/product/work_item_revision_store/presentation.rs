use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id};
use crate::product::models::{HumanPresentationRevision, WorkItemPlanLineage};

use super::{WorkItemRevisionStore, identity_mismatch, json_file_paths, write_immutable};

impl WorkItemRevisionStore {
    pub fn put_human_presentation_revision(
        &self,
        plan: &WorkItemPlanLineage,
        value: &HumanPresentationRevision,
    ) -> Result<(), ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(&value.id)?;
        validate_optional_id(value.source_plan_projection_bundle_id.as_deref())?;
        validate_optional_id(value.source_work_item_projection_bundle_id.as_deref())?;
        validate_optional_id(value.supersedes.as_deref())?;
        write_immutable(
            &self.human_presentation_revision_path(
                &plan.project_id,
                &plan.issue_id,
                &plan.id,
                &value.id,
            ),
            "human_presentation_revision",
            &value.id,
            value,
        )
    }

    pub fn get_latest_human_presentation_revision(
        &self,
        plan: &WorkItemPlanLineage,
        source_projection_bundle_id: &str,
    ) -> Result<Option<HumanPresentationRevision>, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        validate_relative_id(source_projection_bundle_id)?;
        let mut latest: Option<HumanPresentationRevision> = None;
        for path in json_file_paths(&self.human_presentation_revisions_root(
            &plan.project_id,
            &plan.issue_id,
            &plan.id,
        ))? {
            let value: HumanPresentationRevision = read_json(&path)?;
            let file_id = path
                .file_stem()
                .and_then(|value| value.to_str())
                .ok_or_else(|| {
                    ProductStoreError::Io(format!("invalid presentation path: {}", path.display()))
                })?;
            if value.id != file_id {
                return Err(identity_mismatch("human_presentation_revision", file_id));
            }
            let matches_source = value.source_plan_projection_bundle_id.as_deref()
                == Some(source_projection_bundle_id)
                || value.source_work_item_projection_bundle_id.as_deref()
                    == Some(source_projection_bundle_id);
            if matches_source
                && latest.as_ref().is_none_or(|current| {
                    (value.created_at.as_str(), value.id.as_str())
                        > (current.created_at.as_str(), current.id.as_str())
                })
            {
                latest = Some(value);
            }
        }
        Ok(latest)
    }
}

fn validate_optional_id(value: Option<&str>) -> Result<(), ProductStoreError> {
    if let Some(value) = value {
        validate_relative_id(value)?;
    }
    Ok(())
}
