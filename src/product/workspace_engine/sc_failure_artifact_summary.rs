//! `ArtifactVersion` -> `ArtifactVersionSummary` 的摘要构造（自 session_state 等价搬迁的单一实现）。

use super::WorkspaceEngine;
use super::preview;
use crate::product::work_item_plan_policy::PolicyDiagnostic;
use crate::web::workspace_ws_types::ScFailureScope;
use crate::web::workspace_ws_types::ScPackageCompleteness;
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
        ArtifactPayload::ScFailureRepairPackage {
            sc_failure_repair_package,
        } => {
            // A4 修复包不是 markdown：size 是包 JSON 的真实字节数；preview 是只读
            // 摘要（真实轮次/缺失数/完整性与字节数），不截断包正文、不回显序列化。
            let size = serde_json::to_string(sc_failure_repair_package).map_or(0, |s| s.len());
            let completeness = match &sc_failure_repair_package.completeness {
                ScPackageCompleteness::Complete => "complete",
                ScPackageCompleteness::Incomplete => "incomplete",
            };
            let summary = serde_json::json!({
                "bytes": size,
                "completeness": completeness,
                "missing_count": sc_failure_repair_package.missing_evidence.len(),
                "round_count": sc_failure_repair_package.rounds.len(),
            });
            let preview_text = serde_json::to_string(&summary).unwrap_or_default();
            (size, preview(&preview_text))
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

/// A4 payload 判定：只有 SC 失败修复包走「已发布」过滤。
fn is_sc_failure_repair_package(payload: &ArtifactPayload) -> bool {
    matches!(payload, ArtifactPayload::ScFailureRepairPackage { .. })
}

impl WorkspaceEngine {
    /// A4（SC 失败修复包）公开只读视图的版本表：只有严格发布回执通过才可见。
    /// 缺回执的 pending 不出现；坏回执/摘要显式 Err——投影为 policy_diagnostics，
    /// 不吞成空列表成功，普通 entries 保留。内存 raw 版本表不被过滤结果覆盖。
    pub(crate) fn sc_published_versions(&self) -> (Vec<ArtifactVersion>, Option<PolicyDiagnostic>) {
        let ordinary_versions = || {
            self.artifact_versions
                .iter()
                .filter(|version| !is_sc_failure_repair_package(&version.payload))
                .cloned()
                .collect::<Vec<_>>()
        };
        let has_sc_package = self
            .artifact_versions
            .iter()
            .any(|version| is_sc_failure_repair_package(&version.payload));
        if !has_sc_package {
            return (self.artifact_versions.clone(), None);
        }
        let Some(store) = self.lifecycle_store.as_ref() else {
            // 无 durable store：无回执可读，A4 一律不可见（pending），普通 entries 照常。
            return (ordinary_versions(), None);
        };
        // 会话四元组（plan_id 即会话实体）——与 store 准入同源。
        let scope = ScFailureScope {
            session_id: self.session.session_id.clone(),
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
        };
        match store.filter_sc_published_artifact_versions(&scope, self.artifact_versions.clone()) {
            Ok(published) => (published, None),
            Err(failure) => (
                ordinary_versions(),
                Some(PolicyDiagnostic {
                    code: failure.code.clone(),
                    message: format!("{}: {}", failure.step, failure.detail),
                    field: Some("sc_failure_repair_package".to_string()),
                }),
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::*;

    const SHA256_HEX: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn available_ref(locator: &str, bytes: u64) -> ScEvidenceRef {
        ScEvidenceRef::Available {
            locator: locator.to_string(),
            sha256: SHA256_HEX.to_string(),
            bytes,
        }
    }

    fn round_evidence(ordinal: u32) -> ScRoundEvidence {
        ScRoundEvidence {
            identity: ScRoundIdentity {
                chain_id: "chain-sc-1".to_string(),
                round_id: format!("round-{ordinal}"),
                node_id: "node-sc-1".to_string(),
                round_ordinal: ordinal,
                provider: ProviderName::Fake,
                workspace_session_id: "session-sc-1".to_string(),
                role_run_seq: u64::from(ordinal),
                run_ref: format!("run-{ordinal}"),
            },
            launch_state: ScRoundLaunchState::Started,
            failure_class: None,
            blocking_reasons: vec!["compile_failed".to_string()],
            repair_instructions: Vec::new(),
            raw: available_ref("work-item-split-runs/issue-1/run-1/run.json", 42),
            source: available_ref("work-item-split-runs/issue-1/run-1/source.txt", 17),
            existing_failure_id: None,
            attempt_key: None,
            prompts: Vec::new(),
            teaching_decision: ScTeachingDecision::NotRequested,
        }
    }

    fn repair_package(
        completeness: ScPackageCompleteness,
        rounds: Vec<ScRoundEvidence>,
        missing_evidence: Vec<ScMissingEvidence>,
    ) -> ScFailureRepairPackage {
        ScFailureRepairPackage {
            schema_version: 1,
            package_id: "pkg-chain-sc-1".to_string(),
            chain_id: "chain-sc-1".to_string(),
            session_id: "session-sc-1".to_string(),
            project_id: "project-sc-1".to_string(),
            issue_id: "issue-sc-1".to_string(),
            plan_id: "plan-sc-1".to_string(),
            terminal_class: "compile".to_string(),
            terminal_reason: "compile failed".to_string(),
            completeness,
            rounds,
            last_provider_output: available_ref("work-item-split-runs/issue-1/run-1/run.json", 42),
            last_failure_source: ScFailureSourceEvidence {
                source_round_id: "round-1".to_string(),
                source: available_ref("work-item-split-runs/issue-1/run-1/source.txt", 17),
            },
            teaching_prompts: Vec::new(),
            evidence_log_ref: "sc-failure-evidence/chain-sc-1".to_string(),
            evidence_digest: SHA256_HEX.to_string(),
            evidence_through_entry_key: "chain-sc-1/round-1/terminal".to_string(),
            missing_evidence,
        }
    }

    fn artifact_version(package: ScFailureRepairPackage) -> ArtifactVersion {
        ArtifactVersion {
            version: 7,
            payload: ArtifactPayload::ScFailureRepairPackage {
                sc_failure_repair_package: Box::new(package),
            },
            generated_by: ProviderName::Fake,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: false,
            created_at: "2026-11-13T00:00:00Z".to_string(),
            source_node_id: "node-sc-1".to_string(),
        }
    }

    #[test]
    fn sc_failure_repair_package_summary_reports_real_counts_and_bytes() {
        let complete = repair_package(
            ScPackageCompleteness::Complete,
            vec![round_evidence(1), round_evidence(2)],
            Vec::new(),
        );
        let complete_bytes = serde_json::to_string(&complete)
            .expect("package json")
            .len();
        let summary = build_artifact_version_summary(&artifact_version(complete));
        assert_eq!(summary.markdown_size, complete_bytes);
        let preview: serde_json::Value =
            serde_json::from_str(&summary.markdown_preview).expect("preview is plain json");
        assert_eq!(preview["completeness"].as_str(), Some("complete"));
        assert_eq!(preview["round_count"].as_u64(), Some(2));
        assert_eq!(preview["missing_count"].as_u64(), Some(0));
        assert_eq!(preview["bytes"].as_u64(), Some(complete_bytes as u64));

        let incomplete = repair_package(
            ScPackageCompleteness::Incomplete,
            vec![round_evidence(1)],
            vec![ScMissingEvidence {
                round_id: "round-1".to_string(),
                kind: "prompt".to_string(),
                code: "prompt_pruned".to_string(),
                detail: "teaching prompt body pruned before send".to_string(),
                locator: None,
            }],
        );
        let incomplete_bytes = serde_json::to_string(&incomplete)
            .expect("package json")
            .len();
        let summary = build_artifact_version_summary(&artifact_version(incomplete));
        assert_eq!(summary.markdown_size, incomplete_bytes);
        let preview: serde_json::Value =
            serde_json::from_str(&summary.markdown_preview).expect("preview is plain json");
        assert_eq!(preview["completeness"].as_str(), Some("incomplete"));
        assert_eq!(preview["round_count"].as_u64(), Some(1));
        assert_eq!(preview["missing_count"].as_u64(), Some(1));
        assert_eq!(preview["bytes"].as_u64(), Some(incomplete_bytes as u64));
    }

    #[test]
    fn sc_failure_repair_package_summary_preserves_ordinary_metadata() {
        let package = repair_package(
            ScPackageCompleteness::Complete,
            vec![round_evidence(1)],
            Vec::new(),
        );
        let version = artifact_version(package.clone());
        let summary = build_artifact_version_summary(&version);
        assert_eq!(summary.version, 7);
        assert_eq!(summary.generated_by, ProviderName::Fake);
        assert_eq!(summary.reviewed_by, None);
        assert_eq!(summary.review_verdict, None);
        assert_eq!(summary.confirmed_by, None);
        assert!(!summary.is_current);
        assert_eq!(summary.created_at, "2026-11-13T00:00:00Z");
        assert_eq!(summary.source_node_id, "node-sc-1");
        assert_ne!(
            summary.markdown_preview,
            serde_json::to_string(&package).expect("package json"),
            "summary preview must be meaning fields, not a serializer echo"
        );
    }
}
