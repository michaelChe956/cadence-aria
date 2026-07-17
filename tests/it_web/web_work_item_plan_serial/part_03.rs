fn valid_draft_output_with_title(outline_id: &str, task_statement: &str) -> Value {
    let stable_title = match outline_id {
        "outline_frontend_expiry" => "实现前端会话过期提示",
        "outline_integration_session" => "集成测试：会话过期端到端",
        _ => "实现后端登录会话 API",
    };
    let mut output = valid_canonical_draft_output(outline_id, stable_title);
    output["draft"]["canonical_contract"]["tasks"][0]["statement"] = json!(task_statement);
    output
}

fn valid_frontend_draft_output() -> Value {
    valid_canonical_draft_output("outline_frontend_expiry", "实现前端会话过期提示")
}

fn valid_integration_draft_output() -> Value {
    valid_canonical_draft_output(
        "outline_integration_session",
        "集成测试：会话过期端到端",
    )
}

fn invalid_draft_output_missing_scope(outline_id: &str) -> Value {
    let mut output = valid_draft_output(outline_id);
    output["draft"]["canonical_contract"]["write_policy"]["exclusive_scopes"] = json!([]);
    output
}
