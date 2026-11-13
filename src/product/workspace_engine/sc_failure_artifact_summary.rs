//! `ArtifactVersion` -> `ArtifactVersionSummary` 的摘要构造（自 session_state 等价搬迁的单一实现）。

use super::preview;
use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion, ArtifactVersionSummary};

pub(crate) fn build_artifact_version_summary(version: &ArtifactVersion) -> ArtifactVersionSummary {
    let (markdown_size, markdown_preview) = match &version.payload {
        ArtifactPayload::Markdown { markdown, .. } => (markdown.len(), preview(markdown)),
        ArtifactPayload::WorkItemPlanCandidate { candidate } => {
            // For the candidate variant, `markdown_size`/`markdown_preview` reuse the
            // summary schema fields for compatibility: the size is the JSON length of
            // the candidate and the preview is the title of the first work item (or the
            // plan id as a fallback). In a future iteration we may rename these fields.
            let size = serde_json::to_string(candidate).map_or(0, |s| s.len());
            let preview_text = candidate
                .work_items
                .first()
                .map(|item| item.title.as_str())
                .unwrap_or(candidate.plan.id.as_str())
                .to_string();
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanOutlineCandidate { outline_candidate } => {
            let size = serde_json::to_string(outline_candidate).map_or(0, |s| s.len());
            let preview_text = outline_candidate.outline.strategy_summary.clone();
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanContextBlocker { context_blocker } => {
            let size = serde_json::to_string(context_blocker).map_or(0, |s| s.len());
            let preview_text = context_blocker.exploration_summary.clone();
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemDraftCandidate { draft_candidate } => {
            let size = serde_json::to_string(draft_candidate).map_or(0, |s| s.len());
            let preview_text = format!(
                "{}: {}",
                draft_candidate.draft_record.outline_id,
                draft_candidate
                    .draft_record
                    .candidate
                    .canonical_contract_candidate
                    .identity
                    .title
            );
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemBatchState { batch_state } => {
            let size = serde_json::to_string(batch_state).map_or(0, |s| s.len());
            let preview_text = format!(
                "{}: {:?} ({} drafts)",
                batch_state.batch_id,
                batch_state.batch_status,
                batch_state.draft_records.len()
            );
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanCompileReport { compile_report } => {
            let size = serde_json::to_string(compile_report).map_or(0, |s| s.len());
            let preview_text = format!(
                "{}: {:?} ({} work items)",
                compile_report.compile_id,
                compile_report.status,
                compile_report.work_item_ids.len()
            );
            (size, preview(&preview_text))
        }
        ArtifactPayload::WorkItemPlanProjection { projection } => {
            let size = serde_json::to_string(projection).map_or(0, |s| s.len());
            (size, preview(&projection.id))
        }
        ArtifactPayload::WorkItemProjection { projection } => {
            let size = serde_json::to_string(projection).map_or(0, |s| s.len());
            (size, preview(&projection.id))
        }
        ArtifactPayload::WorkItemRevisionHistory { history } => {
            let size = serde_json::to_string(history).map_or(0, |s| s.len());
            let preview_text = format!("{} history entries", history.entries.len());
            (size, preview(&preview_text))
        }
        ArtifactPayload::ProjectionValidation { report } => {
            let size = serde_json::to_string(report).map_or(0, |s| s.len());
            let preview_text = format!("{} projection findings", report.findings.len());
            (size, preview(&preview_text))
        }
        ArtifactPayload::PlanAmendmentManifest { manifest } => {
            let size = serde_json::to_string(manifest).map_or(0, |s| s.len());
            (size, preview(&manifest.id))
        }
    };
    ArtifactVersionSummary {
        version: version.version,
        generated_by: version.generated_by.clone(),
        reviewed_by: version.reviewed_by.clone(),
        review_verdict: version.review_verdict.clone(),
        confirmed_by: version.confirmed_by.clone(),
        is_current: version.is_current,
        created_at: version.created_at.clone(),
        source_node_id: version.source_node_id.clone(),
        markdown_size,
        markdown_preview,
    }
}
