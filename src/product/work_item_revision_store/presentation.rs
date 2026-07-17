use std::cmp::Ordering;

use chrono::{DateTime, FixedOffset};

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
        let mut latest: Option<(DateTime<FixedOffset>, HumanPresentationRevision)> = None;
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
            if matches_source {
                let created_at =
                    DateTime::parse_from_rfc3339(&value.created_at).map_err(|error| {
                        ProductStoreError::Json(format!(
                            "invalid human presentation created_at for {}: {error}",
                            value.id
                        ))
                    })?;
                let is_later =
                    latest.as_ref().is_none_or(|(current_time, current)| {
                        match created_at.cmp(current_time) {
                            Ordering::Greater => true,
                            Ordering::Equal => value.id > current.id,
                            Ordering::Less => false,
                        }
                    });
                if is_later {
                    latest = Some((created_at, value));
                }
            }
        }
        Ok(latest.map(|(_, value)| value))
    }
}

fn validate_optional_id(value: Option<&str>) -> Result<(), ProductStoreError> {
    if let Some(value) = value {
        validate_relative_id(value)?;
    }
    Ok(())
}
