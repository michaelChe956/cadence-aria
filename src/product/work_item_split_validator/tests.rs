use super::*;
use crate::product::models::{
    WorkItemDraftCandidate, WorkItemOutline, WorkItemOutlineDependencyEdge,
    WorkItemOutlineSessionFit,
};

#[test]
fn outline_validator_rejects_duplicate_outline_ids() {
    let mut outline = valid_outline();
    outline.work_item_outlines[1].outline_id = "outline_backend".to_string();

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "duplicate_outline_id");
}

#[test]
fn outline_validator_rejects_missing_dependency() {
    let mut outline = valid_outline();
    outline.work_item_outlines[1].depends_on = vec!["outline_missing".to_string()];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "dependency_not_in_outline");
}

#[test]
fn outline_validator_rejects_dependency_cycle() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0].depends_on = vec!["outline_frontend".to_string()];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "dependency_cycle");
}

#[test]
fn outline_validator_requires_traceability_and_write_scopes() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0].source_story_spec_ids.clear();
    outline.work_item_outlines[0].source_design_spec_ids.clear();
    outline.work_item_outlines[0].goal.clear();
    outline.work_item_outlines[0].scope.clear();
    outline.work_item_outlines[0].exclusive_write_scopes.clear();

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "traceability_required");
    assert_has_code(&report, "outline_goal_required");
    assert_has_code(&report, "outline_scope_required");
    assert_has_code(&report, "write_scope_required");
}

#[test]
fn outline_validator_requires_single_session_budget() {
    let mut outline = valid_outline();
    outline.work_item_outlines[0].estimated_context_tokens = None;
    outline.work_item_outlines[0].session_fit = None;
    outline.work_item_outlines[1].estimated_context_tokens = Some(20_000);
    outline.work_item_outlines[1].session_fit = Some(WorkItemOutlineSessionFit::TooLargeMustSplit);

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "outline_budget_required");
    assert_has_code(&report, "outline_session_fit_required");
    assert_has_code(&report, "outline_exceeds_single_session_budget");
    assert_has_code(&report, "outline_too_large_must_split");
}

#[test]
fn outline_validator_detects_direct_scope_conflict() {
    let mut outline = valid_outline();
    outline.work_item_outlines[1].depends_on.clear();
    outline.dependency_graph.clear();
    outline.work_item_outlines[1].exclusive_write_scopes = vec!["src/product/api.rs".to_string()];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "parallel_scope_overlap");
}

#[test]
fn outline_validator_detects_dependent_scope_conflict() {
    let mut outline = valid_outline();
    outline.work_item_outlines[1].exclusive_write_scopes = vec!["src/product/api.rs".to_string()];

    let report = WorkItemPlanOutlineValidator::validate(&outline);

    assert_has_code(&report, "write_scope_conflict");
}

#[test]
fn local_validator_allows_valid_single_draft() {
    let outline = valid_outline();
    let current_outline = outline.work_item_outlines[1].clone();
    let dependency = valid_draft_candidate("outline_backend", vec![]);
    let current = valid_draft_candidate("outline_frontend", vec!["outline_backend"]);

    let report = WorkItemDraftLocalValidator::validate(&current, &[dependency], &current_outline);

    assert!(
        !report.has_errors(),
        "expected valid local draft, got {:?}",
        report.findings
    );
}

#[test]
fn local_validator_blocks_missing_write_scope() {
    let outline = valid_outline();
    let current_outline = outline.work_item_outlines[0].clone();
    let mut current = valid_draft_candidate("outline_backend", vec![]);
    current.exclusive_write_scopes.clear();

    let report = WorkItemDraftLocalValidator::validate(&current, &[], &current_outline);

    assert_has_code(&report, "write_scope_required");
}

#[test]
fn local_validator_blocks_required_gate_missing() {
    let outline = valid_outline();
    let current_outline = outline.work_item_outlines[0].clone();
    let mut current = valid_draft_candidate("outline_backend", vec![]);
    current.verification_plan["required_gates"] = serde_json::json!(["cmd_missing"]);

    let report = WorkItemDraftLocalValidator::validate(&current, &[], &current_outline);

    assert_has_code(&report, "verification_required_gate_missing");
}

#[test]
fn local_validator_blocks_scope_conflict_with_direct_dependency() {
    let outline = valid_outline();
    let current_outline = outline.work_item_outlines[1].clone();
    let dependency = valid_draft_candidate("outline_backend", vec![]);
    let mut current = valid_draft_candidate("outline_frontend", vec!["outline_backend"]);
    current.exclusive_write_scopes = vec!["src/product/api.rs".to_string()];

    let report = WorkItemDraftLocalValidator::validate(&current, &[dependency], &current_outline);

    assert_has_code(&report, "direct_dependency_scope_conflict");
}

fn assert_has_code(report: &WorkItemSplitValidationReport, code: &str) {
    assert!(
        report.findings.iter().any(|finding| finding.code == code),
        "expected code {code}, got {:?}",
        report.findings
    );
}

fn valid_outline() -> WorkItemPlanOutline {
    WorkItemPlanOutline {
        id: "outline_artifact_1".to_string(),
        project_id: "project_0001".to_string(),
        issue_id: "issue_0001".to_string(),
        source_story_spec_ids: vec!["story_spec_0001".to_string()],
        source_design_spec_ids: vec!["design_spec_0001".to_string()],
        strategy_summary: "后端先行，前端随后接入".to_string(),
        work_item_outlines: vec![
            WorkItemOutline {
                outline_id: "outline_backend".to_string(),
                title: "后端 API".to_string(),
                kind: WorkItemKind::Backend,
                goal: "实现 API".to_string(),
                scope: vec!["src/product".to_string()],
                non_goals: vec![],
                estimated_context_tokens: Some(12_000),
                session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
                source_story_spec_ids: vec!["story_spec_0001".to_string()],
                source_design_spec_ids: vec!["design_spec_0001".to_string()],
                exclusive_write_scopes: vec!["src/product/api.rs".to_string()],
                forbidden_write_scopes: vec!["web/**".to_string()],
                depends_on: vec![],
                verification_intent: vec!["cargo test --locked --lib api".to_string()],
                handoff_notes: "提供 API contract".to_string(),
            },
            WorkItemOutline {
                outline_id: "outline_frontend".to_string(),
                title: "前端 UI".to_string(),
                kind: WorkItemKind::Frontend,
                goal: "接入 API".to_string(),
                scope: vec!["web/src".to_string()],
                non_goals: vec![],
                estimated_context_tokens: Some(10_000),
                session_fit: Some(WorkItemOutlineSessionFit::FitsSingleAgentSession),
                source_story_spec_ids: vec!["story_spec_0001".to_string()],
                source_design_spec_ids: vec!["design_spec_0001".to_string()],
                exclusive_write_scopes: vec!["web/src/session.ts".to_string()],
                forbidden_write_scopes: vec!["src/product/**".to_string()],
                depends_on: vec!["outline_backend".to_string()],
                verification_intent: vec!["pnpm -C web test".to_string()],
                handoff_notes: "消费 API contract".to_string(),
            },
        ],
        dependency_graph: vec![WorkItemOutlineDependencyEdge {
            from_outline_id: "outline_backend".to_string(),
            to_outline_id: "outline_frontend".to_string(),
        }],
        risks: vec![],
        handoff_strategy: "后端输出 contract 给前端".to_string(),
        status: "draft".to_string(),
    }
}

fn valid_draft_candidate(
    outline_id: &str,
    depends_on_outline_ids: Vec<&str>,
) -> WorkItemDraftCandidate {
    WorkItemDraftCandidate {
        outline_id: outline_id.to_string(),
        title: format!("Draft {outline_id}"),
        kind: WorkItemKind::Backend,
        goal: "实现局部 work item".to_string(),
        implementation_context: "实现必要代码并保持 handoff。".to_string(),
        exclusive_write_scopes: if outline_id == "outline_backend" {
            vec!["src/product/api.rs".to_string()]
        } else {
            vec!["web/src/session.ts".to_string()]
        },
        forbidden_write_scopes: vec![],
        depends_on_outline_ids: depends_on_outline_ids
            .into_iter()
            .map(str::to_string)
            .collect(),
        required_handoff_from_outline_ids: vec![],
        handoff_summary: "handoff summary".to_string(),
        verification_plan: serde_json::json!({
            "commands": [
                {
                    "id": "cmd_test",
                    "label": "test",
                    "command": "cargo test --locked --lib api",
                    "cwd": "",
                    "purpose": "验证局部 draft",
                    "required": true,
                    "timeout_seconds": 120,
                    "safety": "approved",
                    "source": "provider"
                }
            ],
            "manual_checks": [],
            "required_gates": ["cmd_test"]
        }),
    }
}
