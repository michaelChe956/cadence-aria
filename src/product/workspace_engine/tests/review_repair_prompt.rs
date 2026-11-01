// Task 10 review-repair prompt regressions, extracted to keep part_10.rs below the
// large_file_guard 1200-line limit. `super::*` retains the test module scope and
// the helpers/imports included from part_01.rs.
use super::*;

#[test]
fn review_repair_prompt_uses_readable_output_and_excludes_stale_nonce() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_reviewer_repair_prompt");
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.",
    ));
    session.reviewer_provider = Some(ProviderName::Codex);
    let checkpoint_tmp = TempDir::new().expect("checkpoint tempdir");
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    let base_input = engine.build_review_input().expect("review input");
    let mut completion = ProviderCompletion::plain(
        "<ARIA_STRUCTURED_OUTPUT nonce=\"stale_nonce_0001\">partial structured review",
        None,
    );
    completion.readable_output = "partial structured review".to_string();
    let parse_error = ReviewCompletionError::Syntax(StructuredOutputError {
        code: crate::cross_cutting::structured_output::StructuredOutputErrorCode::MissingEndTag,
        message: "missing end tag".to_string(),
        expected_nonce: Some("nonce_0001".to_string()),
        observed_nonce: None,
        recoverable_value: Some(serde_json::json!({
            "verdict": "pass",
            "summary": "unchanged",
            "findings": []
        })),
    });

    let repair = engine
        .build_review_repair_input(&base_input, &completion, &parse_error, None)
        .expect("repair input");

    assert!(
        repair
            .prompt
            .contains("只能修复 JSON 与 ARIA_STRUCTURED_OUTPUT 封装")
    );
    assert!(repair.prompt.contains("nonce 必须是本请求签发值"));
    assert!(repair.prompt.contains("禁止使用 EXAMPLE_NONCE"));
    assert!(
        repair
            .prompt
            .contains("禁止使用 EXAMPLE_NONCE 或原始输出中出现的任何其他 nonce")
    );
    assert!(repair.prompt.contains(&completion.readable_output));
    assert!(
        !repair
            .prompt
            .contains("<ARIA_STRUCTURED_OUTPUT nonce=\"stale_nonce_0001\">")
    );
    assert!(
        !repair.prompt.contains(&completion.full_output),
        "repair prompt must not replay the original sentinel opening tag"
    );
    assert!(!repair.prompt.contains("[cadence_project_rules]"));
    assert!(!repair.prompt.contains("当前阶段："));
    assert!(!repair.prompt.contains("using-superpowers"));
}

#[test]
fn review_repair_prompt_omits_original_output_when_readable_output_is_empty() {
    let (event_tx, _event_rx) = mpsc::channel(8);
    let mut session = make_session("sess_reviewer_repair_empty_readable_output");
    session.artifact = Some(artifact_payload(
        "# Story Spec\n\n## 功能需求\n- [REQ-001] Draft.",
    ));
    session.reviewer_provider = Some(ProviderName::Codex);
    let checkpoint_tmp = TempDir::new().expect("checkpoint tempdir");
    let engine = WorkspaceEngine::new(
        Arc::new(CheckpointStore::new(checkpoint_tmp.path().to_path_buf())),
        event_tx,
        session,
    );
    let base_input = engine.build_review_input().expect("review input");
    let mut completion = ProviderCompletion::plain("unreadable raw output", None);
    completion.readable_output.clear();
    let parse_error = ReviewCompletionError::Syntax(StructuredOutputError {
        code: crate::cross_cutting::structured_output::StructuredOutputErrorCode::MissingEndTag,
        message: "missing end tag".to_string(),
        expected_nonce: Some("nonce_0001".to_string()),
        observed_nonce: None,
        recoverable_value: Some(serde_json::json!({
            "verdict": "pass",
            "summary": "unchanged",
            "findings": []
        })),
    });

    let repair = engine
        .build_review_repair_input(&base_input, &completion, &parse_error, None)
        .expect("repair input");

    assert!(
        !repair.prompt.contains("原始输出："),
        "empty readable output must omit the original-output section"
    );
    assert!(!repair.prompt.contains(&completion.full_output));
}
