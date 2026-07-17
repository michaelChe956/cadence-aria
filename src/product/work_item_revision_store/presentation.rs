use std::cmp::Ordering;

use chrono::{DateTime, FixedOffset};

use crate::product::id::next_sequential_id;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id};
use crate::product::models::{HumanPresentationRevision, WorkItemPlanLineage};

use super::{
    WorkItemRevisionStore, identity_mismatch, json_file_paths, with_exclusive_lock, write_immutable,
};

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
        presentation_source_bundle_id(value)?;
        DateTime::parse_from_rfc3339(&value.created_at).map_err(|error| {
            ProductStoreError::Json(format!(
                "invalid human presentation created_at for {}: {error}",
                value.id
            ))
        })?;
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

    pub fn put_human_presentation_revision_cas(
        &self,
        plan: &WorkItemPlanLineage,
        mut value: HumanPresentationRevision,
    ) -> Result<HumanPresentationRevision, ProductStoreError> {
        self.ensure_plan_scope(plan)?;
        let source_projection_bundle_id = presentation_source_bundle_id(&value)?.to_string();
        validate_optional_id(value.supersedes.as_deref())?;
        let lineage_lock = self
            .human_presentation_revisions_root(&plan.project_id, &plan.issue_id, &plan.id)
            .join("lineage");

        with_exclusive_lock(&lineage_lock, || {
            let latest =
                self.get_latest_human_presentation_revision(plan, &source_projection_bundle_id)?;
            let expected_supersedes = latest.as_ref().map(|revision| revision.id.as_str());
            if value.supersedes.as_deref() != expected_supersedes {
                return Err(ProductStoreError::Io(format!(
                    "human_presentation_supersedes_conflict: source={source_projection_bundle_id} expected={expected_supersedes:?} actual={:?}",
                    value.supersedes
                )));
            }

            if value.id.is_empty() {
                let existing = json_file_paths(&self.human_presentation_revisions_root(
                    &plan.project_id,
                    &plan.issue_id,
                    &plan.id,
                ))?;
                value.id = next_sequential_id("human_presentation_revision", existing.len());
            }
            self.put_human_presentation_revision(plan, &value)?;
            Ok(value.clone())
        })
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

fn presentation_source_bundle_id(
    value: &HumanPresentationRevision,
) -> Result<&str, ProductStoreError> {
    match (
        value.source_plan_projection_bundle_id.as_deref(),
        value.source_work_item_projection_bundle_id.as_deref(),
    ) {
        (Some(bundle_id), None) | (None, Some(bundle_id)) => Ok(bundle_id),
        _ => Err(ProductStoreError::Json(
            "human presentation revision must have exactly one projection bundle source"
                .to_string(),
        )),
    }
}
