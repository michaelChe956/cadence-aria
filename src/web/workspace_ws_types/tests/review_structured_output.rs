use crate::web::workspace_ws_types::{
    ReviewGate, ReviewVerdict, ReviewVerdictType, StructuredOutputDiagnostic, WsOutMessage,
};

#[test]
fn review_structured_output_diagnostic_is_backward_compatible_and_reaches_websocket_json() {
    let old: ReviewVerdict = serde_json::from_value(serde_json::json!({
        "verdict": "needs_human",
        "comments": "旧数据",
        "summary": "旧数据",
        "findings": [],
        "review_gate": "user_triage_required"
    }))
    .unwrap();
    assert!(old.structured_output_diagnostic.is_none());

    let value = serde_json::to_value(WsOutMessage::ReviewComplete {
        node_id: "node_review_001".to_string(),
        round: 1,
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "需要人工确认".to_string(),
        summary: "需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: "malformed_findings".to_string(),
            message: "review findings are malformed".to_string(),
            repair_attempted: false,
            repair_succeeded: false,
            raw_output_preview: Some("raw review output".to_string()),
        }),
    })
    .unwrap();

    assert_eq!(
        value["structured_output_diagnostic"]["code"],
        "malformed_findings"
    );
}
