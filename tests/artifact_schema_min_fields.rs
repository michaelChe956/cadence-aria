use cadence_aria::cross_cutting::artifact_validate::{
    artifact_validation_matrix, canonical_validator, ArtifactContent, ArtifactValidateError,
};
use cadence_aria::protocol::artifacts::ArtifactKind;
use serde_json::{json, Value};

#[test]
fn canonical_validator_accepts_minimal_positive_fixture_for_all_phase1_artifact_kinds() {
    let matrix = artifact_validation_matrix();
    assert_eq!(matrix.len(), 17);

    for kind in ArtifactKind::all_phase1() {
        let content = minimal_positive_content(kind);
        let result = canonical_validator(kind, &content)
            .unwrap_or_else(|error| panic!("{kind:?}: {error:?}"));
        assert!(result.valid, "{kind:?} should be valid");
    }
}

#[test]
fn canonical_validator_rejects_minimal_negative_fixture_for_all_phase1_artifact_kinds() {
    for kind in ArtifactKind::all_phase1() {
        let (content, missing_field) = minimal_negative_content(kind);
        let error = canonical_validator(kind, &content)
            .unwrap_err_or_else(|| panic!("{kind:?} should be invalid"));
        assert_eq!(
            error,
            ArtifactValidateError::CanonicalMissingField {
                field: missing_field.to_string(),
                artifact_kind: kind
            },
            "{kind:?} should report its missing canonical field"
        );
    }
}

fn minimal_positive_content(kind: ArtifactKind) -> ArtifactContent {
    match kind {
        ArtifactKind::Spec => ArtifactContent::Markdown(
            "# Spec\n\n## 功能需求\n\n- [REQ-001] 示例需求。\n".to_string(),
        ),
        ArtifactKind::Design => ArtifactContent::Markdown(
            "# Design\n\n## 设计决策\n\n- [DEC-001] 示例决策。\n".to_string(),
        ),
        ArtifactKind::Plan => ArtifactContent::Markdown(
            "# Plan\n\n## 工作包\n\n- [WP-001] 示例工作包。\n".to_string(),
        ),
        _ => ArtifactContent::Json(minimal_json(kind)),
    }
}

fn minimal_negative_content(kind: ArtifactKind) -> (ArtifactContent, &'static str) {
    match kind {
        ArtifactKind::Spec | ArtifactKind::Design | ArtifactKind::Plan => {
            (ArtifactContent::Markdown(String::new()), "markdown_body")
        }
        _ => {
            let mut value = minimal_json(kind);
            let field = first_required_json_field(kind);
            value.as_object_mut().expect("object").remove(field);
            (ArtifactContent::Json(value), field)
        }
    }
}

fn minimal_json(kind: ArtifactKind) -> Value {
    match kind {
        ArtifactKind::IntakeBrief => json!({
            "artifact_kind": "intake_brief",
            "request_summary": "实现 Aria MVP",
            "raw_user_request": "继续 mvp 内容开发",
            "repo_context": {},
            "initial_constraints": [],
            "requested_goal": "完成下一阶段基础能力"
        }),
        ArtifactKind::ClarificationRecord => json!({
            "artifact_kind": "clarification_record",
            "goal_summary": "明确目标",
            "constraints": [],
            "assumptions": [],
            "open_questions": [],
            "suggested_scope": "phase1"
        }),
        ArtifactKind::SpecGateDecision => json!({
            "artifact_kind": "spec_gate_decision",
            "decision": "pass",
            "review_notes": []
        }),
        ArtifactKind::DesignReview => json!({
            "artifact_kind": "design_review",
            "review_decision": "pass",
            "findings": []
        }),
        ArtifactKind::DesignRevisionRecord => json!({
            "artifact_kind": "design_revision_record",
            "revision_summary": "更新设计",
            "resolved_findings": []
        }),
        ArtifactKind::ReadinessCheck => json!({
            "artifact_kind": "readiness_check",
            "ready": true,
            "blocking_items": []
        }),
        ArtifactKind::DispatchPackage => json!({
            "artifact_kind": "dispatch_package",
            "worktask_routing": []
        }),
        ArtifactKind::CodingReport => json!({
            "artifact_kind": "coding_report",
            "worktask_id": "work_001",
            "files_modified": ["src/lib.rs"],
            "commands_run": [],
            "candidate_traceability_refs": [],
            "status": "completed"
        }),
        ArtifactKind::TestingReport => json!({
            "artifact_kind": "testing_report",
            "worktask_id": "work_001",
            "commands_run": [],
            "tests_passed": true,
            "failures": [],
            "candidate_traceability_refs": []
        }),
        ArtifactKind::CodeReviewReport => json!({
            "artifact_kind": "code_review_report",
            "worktask_id": "work_001",
            "findings": [],
            "blocking": false,
            "candidate_traceability_refs": []
        }),
        ArtifactKind::IntegrationReport => json!({
            "artifact_kind": "integration_report",
            "integrated_worktasks": [],
            "status": "completed"
        }),
        ArtifactKind::FinalReview => json!({
            "artifact_kind": "final_review",
            "overall_decision": "pass",
            "coverage_summary": {},
            "uncovered_items": [],
            "followup_required": false
        }),
        ArtifactKind::FinalSummary => json!({
            "artifact_kind": "final_summary",
            "overall_status": "closed",
            "next_steps": [],
            "remaining_risks": []
        }),
        ArtifactKind::RuntimeSnapshot => json!({
            "artifact_kind": "runtime_snapshot",
            "phase": "intake",
            "timestamp": "2026-04-27T00:00:00Z",
            "risk_registry": {"risks": []}
        }),
        ArtifactKind::Spec | ArtifactKind::Design | ArtifactKind::Plan => unreachable!(),
    }
}

fn first_required_json_field(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::IntakeBrief => "request_summary",
        ArtifactKind::ClarificationRecord => "goal_summary",
        ArtifactKind::SpecGateDecision => "decision",
        ArtifactKind::DesignReview => "review_decision",
        ArtifactKind::DesignRevisionRecord => "revision_summary",
        ArtifactKind::ReadinessCheck => "ready",
        ArtifactKind::DispatchPackage => "worktask_routing",
        ArtifactKind::CodingReport => "worktask_id",
        ArtifactKind::TestingReport => "worktask_id",
        ArtifactKind::CodeReviewReport => "worktask_id",
        ArtifactKind::IntegrationReport => "integrated_worktasks",
        ArtifactKind::FinalReview => "overall_decision",
        ArtifactKind::FinalSummary => "overall_status",
        ArtifactKind::RuntimeSnapshot => "phase",
        ArtifactKind::Spec | ArtifactKind::Design | ArtifactKind::Plan => "markdown_body",
    }
}

trait UnwrapErrOrElse<T, E> {
    fn unwrap_err_or_else<F: FnOnce() -> T>(self, op: F) -> E;
}

impl<T, E> UnwrapErrOrElse<T, E> for Result<T, E> {
    fn unwrap_err_or_else<F: FnOnce() -> T>(self, op: F) -> E {
        match self {
            Ok(_) => {
                op();
                unreachable!("op must panic")
            }
            Err(error) => error,
        }
    }
}
