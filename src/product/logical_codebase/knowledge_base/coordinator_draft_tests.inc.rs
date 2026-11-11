// `coordinator.rs` 的 UserInput 起草 operation 内联测试（与
// `coordinator_tests.inc.rs` 同属一个测试模块，复用其假 runner / 夹具；
// 独立成文件以守住 large_file_guard 的 1200 行上限）。

// ───────────────────────── 夹具：起草 operation 入口 ─────────────────────────

async fn begin_user_input(
    fixture: &Fixture,
    key: &str,
    provider: &str,
) -> Result<KnowledgeBaseOperation, KnowledgeBaseError> {
    fixture
        .coordinator
        .begin(
            derive_operation_id(PROJECT, LC, key),
            PROJECT,
            LC,
            KnowledgeBaseOperationKind::UserInput,
            input(key, provider, None),
            selection(),
        )
        .await
}

// ───────────────────────── UserInput 起草 operation ─────────────────────────

#[tokio::test]
async fn user_input_joins_the_single_flight_slot() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let first = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();

    // 起草与 bootstrap/update 同一单飞槽位：任一知识库 operation 在跑时拒绝新启。
    let rejected = begin_user_input(&fixture, "kb-key-0002", "claude_code").await;
    match rejected {
        Err(KnowledgeBaseError::Preflight(
            KnowledgeBasePreflightFailure::OtherKnowledgeBaseRunning { operation_id },
        )) => assert_eq!(operation_id, first.operation_id),
        other => panic!("expected OtherKnowledgeBaseRunning, got {other:?}"),
    }
}

#[tokio::test]
async fn execute_user_input_runs_skill_command_in_aggregate_root_and_completes() {
    let fixture = fixture(
        Script::Writes(vec![(
            "cadence/knowledge-base/user-input/nested/goal.md",
            "goal\n",
        )]),
        false,
    );
    let operation = begin_user_input(&fixture, "kb-key-0001", "claude_code")
        .await
        .unwrap();
    fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .unwrap();

    // 起草 turn：cwd = canonical 聚合根，命令 = 前缀 + knowledge-base-user-input。
    assert_eq!(
        fixture.runner.calls(),
        vec![RecordedCall {
            command: "/knowledge-base-user-input".to_string(),
            provider: ProviderName::ClaudeCode,
            canonical_root: fixture.root.clone(),
        }]
    );
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Completed);
    assert_eq!(stored.steps.len(), 1);
    assert_eq!(
        stored.steps[0].step_id,
        KnowledgeBaseStepKind::PrepareUserInput
    );
    assert_eq!(stored.steps[0].status, KnowledgeBaseStepStatus::Completed);
    assert_eq!(stored.steps[0].output_summary.as_deref(), Some("turn ok"));
    assert!(stored.error.is_none());
}

#[tokio::test]
async fn execute_user_input_restricts_writes_to_user_input_directory() {
    // 允许面 = cadence/knowledge-base/user-input/**：面外写入（含 knowledge-base
    // 其余内容、project-rules、根入口文件、成员仓）一律判越界。
    let fixture = fixture(
        Script::Writes(vec![
            ("cadence/knowledge-base/user-input/goal.md", "in scope\n"),
            (
                "cadence/knowledge-base/user-input/nested/style.md",
                "in scope\n",
            ),
            ("cadence/knowledge-base/bootstrap.md", "out\n"),
            ("cadence/project-rules/knowledge-base-usage.md", "out\n"),
            ("CLAUDE.md", "out\n"),
            ("alpha/src/main.rs", "fn main() { /* touched */ }\n"),
        ]),
        false,
    );
    let operation = begin_user_input(&fixture, "kb-key-0001", "claude_code")
        .await
        .unwrap();
    let error = fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    match &error {
        KnowledgeBaseError::WriteScopeViolation { paths } => assert_eq!(
            paths,
            &vec![
                "CLAUDE.md".to_string(),
                "alpha/src/main.rs".to_string(),
                "cadence/knowledge-base/bootstrap.md".to_string(),
                "cadence/project-rules/knowledge-base-usage.md".to_string(),
            ]
        ),
        other => panic!("expected WriteScopeViolation, got {other:?}"),
    }
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Failed);
    let record = stored.error.expect("失败记录");
    assert_eq!(record.reason_code, "knowledge_base_write_scope_violation");
    assert!(!record.retryable);
    assert!(
        record
            .stderr_summary
            .as_deref()
            .is_some_and(|text| text.contains("alpha/src/main.rs"))
    );
    // remediation 文案按起草允许面给出，而不是 bootstrap/update 的允许面。
    assert!(record.action.contains("cadence/knowledge-base/user-input"));
    assert_eq!(
        fixture.coordinator.other_running(PROJECT).unwrap(),
        None,
        "失败后单飞槽位必须释放"
    );
}
