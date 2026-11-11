// `coordinator.rs` 修复轮 1 的回归测试（与 `coordinator_tests.inc.rs` 同属
// 一个测试模块，复用其假 runner / 夹具）。

// ───────────────────────── C2：失败/取消也审计 ─────────────────────────

async fn run_execute(fixture: &Fixture, key: &str) -> (KnowledgeBaseOperation, KnowledgeBaseError) {
    let operation = begin_bootstrap(fixture, key).await.unwrap();
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
    (operation, error)
}

#[tokio::test]
async fn execute_audits_writes_made_before_a_failed_turn() {
    let fixture = fixture(
        Script::WritesThenFail {
            files: vec![
                ("alpha/src/main.rs", "fn main() { /* touched */ }\n"),
                ("cadence/knowledge-base/x.md", "ok"),
            ],
            code: "provider_turn_timed_out",
            retryable: true,
        },
        false,
    );
    let (operation, error) = run_execute(&fixture, "kb-key-0001").await;
    // 对调用方仍是原始 turn 失败，不被越界掩盖。
    assert!(
        matches!(&error, KnowledgeBaseError::TurnFailed { reason_code, .. } if reason_code == "provider_turn_timed_out")
    );
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Failed);
    assert_eq!(stored.steps[0].status, KnowledgeBaseStepStatus::Failed);
    let record = stored.error.expect("失败记录");
    // 越界作为附加事实：顶层记越界码且不可重试，原始失败原因保留在 cause / 摘要。
    assert_eq!(record.reason_code, "knowledge_base_write_scope_violation");
    assert!(!record.retryable, "工作树已被污染，不得盲目重试");
    assert_eq!(
        record.cause.as_deref(),
        Some("knowledge_base_provider_turn_failed")
    );
    let summary = record.stderr_summary.expect("摘要");
    assert!(summary.contains("alpha/src/main.rs"), "{summary}");
    assert!(summary.contains("provider_turn_timed_out"), "{summary}");
    assert!(
        !summary.contains("cadence/knowledge-base/x.md"),
        "{summary}"
    );
    assert_eq!(fixture.coordinator.other_running(PROJECT).unwrap(), None);
}

#[tokio::test]
async fn execute_failed_turn_without_violation_keeps_original_failure_record() {
    let fixture = fixture(
        Script::WritesThenFail {
            files: vec![("cadence/knowledge-base/x.md", "ok")],
            code: "provider_turn_timed_out",
            retryable: true,
        },
        false,
    );
    let (operation, _) = run_execute(&fixture, "kb-key-0001").await;
    let record = fixture
        .store
        .get(PROJECT, &operation.operation_id)
        .unwrap()
        .error
        .expect("失败记录");
    assert_eq!(record.reason_code, "knowledge_base_provider_turn_failed");
    assert_eq!(record.cause.as_deref(), Some("provider_turn_timed_out"));
    assert!(record.retryable);
}

#[tokio::test]
async fn execute_audits_writes_made_before_a_cancelled_turn() {
    let fixture = fixture(
        Script::WritesThenCancel(vec![
            ("alpha/src/main.rs", "fn main() { /* touched */ }\n"),
            ("cadence/knowledge-base/x.md", "ok"),
        ]),
        false,
    );
    let (operation, error) = run_execute(&fixture, "kb-key-0001").await;
    assert!(matches!(error, KnowledgeBaseError::Cancelled));
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    // 状态仍是 Cancelled（不改写成失败），越界作为附加失败事实留痕。
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);
    assert!(stored.completed_at.is_some());
    let record = stored.error.expect("越界事实必须留痕");
    assert_eq!(record.reason_code, "knowledge_base_write_scope_violation");
    assert!(!record.retryable);
    assert_eq!(record.cause.as_deref(), Some("knowledge_base_cancelled"));
    assert!(
        record
            .stderr_summary
            .as_deref()
            .is_some_and(|text| text.contains("alpha/src/main.rs"))
    );
    assert_eq!(fixture.coordinator.other_running(PROJECT).unwrap(), None);
}

#[tokio::test]
async fn execute_cancelled_turn_without_violation_records_no_error() {
    let fixture = fixture(
        Script::WritesThenCancel(vec![("cadence/knowledge-base/x.md", "ok")]),
        false,
    );
    let (operation, error) = run_execute(&fixture, "kb-key-0001").await;
    assert!(matches!(error, KnowledgeBaseError::Cancelled));
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);
    assert!(stored.error.is_none());
}

// ───────────────────────── C3：只豁免 .git/index ─────────────────────────

#[test]
fn write_scope_audit_flags_member_git_metadata_other_than_index() {
    let (_temp, root) = audit_root();
    for (path, content) in [
        ("alpha/.git/config", "[core]\n"),
        ("alpha/.git/HEAD", "ref: refs/heads/main\n"),
        ("alpha/.git/hooks/pre-commit", "#!/bin/sh\n"),
        ("alpha/.git/refs/heads/main", "abc\n"),
        ("alpha/.git/index", "idx"),
    ] {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, content).unwrap();
    }
    let before = snapshot_aggregate_root(&root).unwrap();
    // 修改 config / HEAD、新增 hook、删除 ref；同时刷新 index（应被豁免）。
    std::fs::write(root.join("alpha/.git/config"), "[core]\nevil = 1\n").unwrap();
    std::fs::write(root.join("alpha/.git/HEAD"), "ref: refs/heads/other\n").unwrap();
    std::fs::write(root.join("alpha/.git/hooks/post-commit"), "#!/bin/sh\n").unwrap();
    std::fs::remove_file(root.join("alpha/.git/refs/heads/main")).unwrap();
    std::fs::write(root.join("alpha/.git/index"), "idx-two-longer").unwrap();
    let after = snapshot_aggregate_root(&root).unwrap();
    assert_eq!(
        write_scope_violations(&before, &after, KnowledgeBaseOperationKind::Bootstrap),
        vec![
            "alpha/.git/HEAD".to_string(),
            "alpha/.git/config".to_string(),
            "alpha/.git/hooks/post-commit".to_string(),
            "alpha/.git/refs/heads/main".to_string(),
        ]
    );
}

#[test]
fn write_scope_audit_git_index_exemption_is_exact() {
    let (_temp, root) = audit_root();
    let before = snapshot_aggregate_root(&root).unwrap();
    // 只有名为 `.git` 目录下的 `index` 豁免；同名但不在 `.git` 下的不豁免。
    std::fs::create_dir_all(root.join("alpha/.git")).unwrap();
    std::fs::write(root.join("alpha/.git/index"), "idx").unwrap();
    std::fs::write(root.join("alpha/index"), "not git").unwrap();
    std::fs::write(root.join("alpha/.git/index.lock"), "lock").unwrap();
    let after = snapshot_aggregate_root(&root).unwrap();
    assert_eq!(
        write_scope_violations(&before, &after, KnowledgeBaseOperationKind::Bootstrap),
        vec![
            "alpha/.git/index.lock".to_string(),
            "alpha/index".to_string()
        ]
    );
}

#[tokio::test]
async fn execute_fails_when_only_member_git_metadata_changes() {
    let fixture = fixture(
        Script::Writes(vec![("alpha/.git/config", "[core]\nevil = 1\n")]),
        false,
    );
    let (operation, error) = run_execute(&fixture, "kb-key-0001").await;
    match &error {
        KnowledgeBaseError::WriteScopeViolation { paths } => {
            assert_eq!(paths, &vec!["alpha/.git/config".to_string()]);
        }
        other => panic!("expected WriteScopeViolation, got {other:?}"),
    }
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Failed);
}

#[tokio::test]
async fn execute_succeeds_when_only_git_index_is_refreshed() {
    let fixture = fixture(
        Script::Writes(vec![
            ("alpha/.git/index", "refreshed"),
            ("cadence/knowledge-base/x.md", "ok"),
        ]),
        false,
    );
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    fixture
        .coordinator
        .execute(
            PROJECT,
            &operation.operation_id,
            &fixture.root,
            CancellationToken::new(),
        )
        .await
        .expect("只读 git 命令刷新 index 不得判越界");
}

// ───────── I1/R1：平台写入可信归因（聚合索引运行期隔离） ─────────

#[tokio::test]
async fn execute_mutually_excludes_platform_index_writes_behind_the_aggregate_index_lock() {
    // 平台写入方是独立线程：真实持有 `.aggregate-index.lock` 并在其临界区内
    // 写聚合索引产物（codegraph init/sync 的行为）——与假 runner（provider）
    // 是两个来源，不再用同一个假 runner 冒充平台写入方。
    use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
    use crate::product::logical_codebase::aggregate_index::AggregateIndexStore;

    let started = Arc::new(tokio::sync::Notify::new());
    let proceed = Arc::new(tokio::sync::Notify::new());
    let fixture = fixture(
        Script::GatedWrites {
            files: vec![("cadence/knowledge-base/x.md", "kb")],
            started: started.clone(),
            proceed: proceed.clone(),
        },
        false,
    );
    let lock_path = AggregateIndexStore::for_lc(fixture.paths.clone(), LC.to_string())
        .lock_path(PROJECT)
        .unwrap();

    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let platform_root = fixture.root.clone();
    let platform_thread = std::thread::spawn(move || {
        // 平台索引重建：持锁期间写索引产物（含 write_json 的原子临时文件）。
        with_exact_exclusive_lock(&lock_path, || {
            write_files(
                &platform_root,
                &[
                    ("codegraph.json", "{\"exclude\":[]}"),
                    (".codegraph/index.db", "db"),
                    (".codegraph.json.123.456.tmp", "tmp"),
                ],
            );
            held_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(30))
                .ok();
            Ok(())
        })
        .unwrap();
    });
    held_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();

    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    let execute = {
        let coordinator = fixture.coordinator.clone();
        let operation_id = operation.operation_id.clone();
        let root = fixture.root.clone();
        tokio::spawn(async move {
            coordinator
                .execute(PROJECT, &operation_id, &root, CancellationToken::new())
                .await
        })
    };

    // 平台索引持锁期间 turn 不得启动：快照与 provider turn 都被隔离在锁外。
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert!(
        fixture.runner.calls().is_empty(),
        "平台索引持锁期间 turn 不得启动（运行期隔离）"
    );

    // 释放平台锁：execute 获得隔离锁，平台产物已落进 before 快照（不进差分）。
    release_tx.send(()).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(10), started.notified())
        .await
        .expect("释放平台锁后 turn 应启动");
    proceed.notify_one();
    execute
        .await
        .unwrap()
        .expect("平台索引写入不得归因给 provider");
    platform_thread.join().unwrap();
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Completed);
}

#[tokio::test]
async fn execute_allows_platform_runtime_writes_during_turn() {
    // `.aria/runtime/**` 是平台运行时子树（issue 运行绑定/任务状态，无锁
    // 保护、无法归因）：turn 进行中由平台（测试线程，非 runner）写入不算
    // provider 越界；`.aria` 其余内容仍照常审计（见纯函数测试）。
    let started = Arc::new(tokio::sync::Notify::new());
    let proceed = Arc::new(tokio::sync::Notify::new());
    let fixture = fixture(
        Script::GatedWrites {
            files: vec![("cadence/knowledge-base/x.md", "kb")],
            started: started.clone(),
            proceed: proceed.clone(),
        },
        false,
    );
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    let execute = {
        let coordinator = fixture.coordinator.clone();
        let operation_id = operation.operation_id.clone();
        let root = fixture.root.clone();
        tokio::spawn(async move {
            coordinator
                .execute(PROJECT, &operation_id, &root, CancellationToken::new())
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), started.notified())
        .await
        .expect("turn 应启动");
    let runtime_state = fixture
        .root
        .join(".aria/runtime/tasks/binding-0001/state.json");
    std::fs::create_dir_all(runtime_state.parent().unwrap()).unwrap();
    std::fs::write(&runtime_state, "{}").unwrap();
    proceed.notify_one();
    execute
        .await
        .unwrap()
        .expect("平台运行时写入不得判 provider 越界");
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Completed);
    assert!(stored.error.is_none());
}

#[test]
fn write_scope_audit_platform_runtime_exemption_is_root_level_and_exact() {
    let (_temp, root) = audit_root();
    let before = snapshot_aggregate_root(&root).unwrap();
    // 平台运行时子树（根层 `.aria/runtime/**`）：不计入。
    for path in [
        ".aria/runtime/state.json",
        ".aria/runtime/tasks/binding-0001/state.json",
    ] {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "x").unwrap();
    }
    // 对照：聚合索引产物（turn 期间已由锁互斥，出现在差分即 provider 写）、
    // `.aria` 其余内容、嵌套 `.aria/runtime`、近似名都判 provider 越界。
    for path in [
        "codegraph.json",
        ".codegraph/index.db",
        ".codegraph.json.1.2.tmp",
        ".aria/evil.sh",
        ".aria/runtime-evil/x",
        "alpha/.aria/runtime/x",
        ".mcp.json",
    ] {
        let target = root.join(path);
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(target, "x").unwrap();
    }
    let after = snapshot_aggregate_root(&root).unwrap();
    assert_eq!(
        write_scope_violations(&before, &after, KnowledgeBaseOperationKind::Bootstrap),
        vec![
            ".aria/evil.sh".to_string(),
            ".aria/runtime-evil/x".to_string(),
            ".codegraph.json.1.2.tmp".to_string(),
            ".codegraph/index.db".to_string(),
            ".mcp.json".to_string(),
            "alpha/.aria/runtime/x".to_string(),
            "codegraph.json".to_string(),
        ]
    );
}

// ───────────────────────── I2：跨进程单飞 ─────────────────────────

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn begin_admits_exactly_one_of_many_concurrent_distinct_operations() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    let mut handles = Vec::new();
    for index in 0..8 {
        // 各自独立的 coordinator / store 句柄，模拟共享 store 的多个调用方。
        let coordinator = fixture.coordinator.clone();
        handles.push(tokio::spawn(async move {
            let key = format!("kb-key-{index:04}");
            coordinator
                .begin(
                    derive_operation_id(PROJECT, LC, &key),
                    PROJECT,
                    LC,
                    KnowledgeBaseOperationKind::Bootstrap,
                    input(&key, "claude_code", None),
                    selection(),
                )
                .await
        }));
    }
    let mut admitted = 0;
    let mut rejected = 0;
    for handle in handles {
        match handle.await.unwrap() {
            Ok(_) => admitted += 1,
            Err(KnowledgeBaseError::Preflight(
                KnowledgeBasePreflightFailure::OtherKnowledgeBaseRunning { .. },
            )) => rejected += 1,
            Err(other) => panic!("unexpected error: {other:?}"),
        }
    }
    assert_eq!((admitted, rejected), (1, 7));
    assert_eq!(fixture.store.list_running(PROJECT).unwrap().len(), 1);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn begin_waits_on_the_cross_process_lock_held_by_another_handle() {
    let fixture = fixture(Script::Writes(Vec::new()), false);
    // 另一个「进程」（独立 fd 的 store 句柄）持有 begin 锁，并在持锁期间
    // 已把一条 Running operation 落盘——begin 必须等到它释放，并看到该占位。
    let holder_store = fixture.store.clone();
    let (held_tx, held_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        holder_store
            .with_begin_lock(PROJECT, || {
                let key = "kb-key-holder";
                holder_store.create(KnowledgeBaseOperation {
                    operation_id: derive_operation_id(PROJECT, LC, key),
                    project_id: PROJECT.to_string(),
                    logical_codebase_id: LC.to_string(),
                    kind: KnowledgeBaseOperationKind::Bootstrap,
                    input: input(key, "claude_code", None),
                    status: KnowledgeBaseOperationStatus::Running,
                    steps: Vec::new(),
                    provider_selection: Some(selection()),
                    error: None,
                    created_at: NOW.to_string(),
                    started_at: Some(NOW.to_string()),
                    completed_at: None,
                })?;
                held_tx.send(()).unwrap();
                release_rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .ok();
                Ok(())
            })
            .unwrap();
    });
    held_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .unwrap();

    let coordinator = fixture.coordinator.clone();
    let mut contender = tokio::spawn(async move {
        coordinator
            .begin(
                derive_operation_id(PROJECT, LC, "kb-key-contender"),
                PROJECT,
                LC,
                KnowledgeBaseOperationKind::Update,
                input("kb-key-contender", "claude_code", Some(PACKAGE)),
                selection(),
            )
            .await
    });
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(300), &mut contender)
            .await
            .is_err(),
        "持锁期间 begin 不得完成查重与创建"
    );
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    match contender.await.unwrap() {
        Err(KnowledgeBaseError::Preflight(
            KnowledgeBasePreflightFailure::OtherKnowledgeBaseRunning { operation_id },
        )) => assert_eq!(
            operation_id,
            derive_operation_id(PROJECT, LC, "kb-key-holder")
        ),
        other => panic!("expected OtherKnowledgeBaseRunning, got {other:?}"),
    }
}

// ───────── R1：provider 改写聚合索引配置必须判越界 ─────────

#[tokio::test]
async fn execute_flags_provider_overwrite_of_root_codegraph_config() {
    // 没有平台索引任务在跑（隔离锁空闲）：provider 覆盖根 `codegraph.json`
    // 与 `.codegraph/**` 必须判越界——平台索引产物不再按路径免审。
    let fixture = fixture(
        Script::Writes(vec![
            ("codegraph.json", "{\"exclude\":[\"provider/\"]}"),
            (".codegraph/index.db", "provider-written db"),
        ]),
        false,
    );
    let (operation, error) = run_execute(&fixture, "kb-key-0001").await;
    assert!(
        matches!(&error, KnowledgeBaseError::WriteScopeViolation { paths }
            if paths == &vec![".codegraph/index.db".to_string(), "codegraph.json".to_string()]),
        "{error:?}"
    );
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Failed);
    assert_eq!(
        stored.error.expect("失败记录").reason_code,
        "knowledge_base_write_scope_violation"
    );
}

#[tokio::test]
async fn execute_flags_provider_deletion_of_root_codegraph_config() {
    let fixture = fixture(Script::DeletesThenOk("codegraph.json"), false);
    std::fs::write(fixture.root.join("codegraph.json"), "{\"exclude\":[]}").unwrap();
    let (operation, error) = run_execute(&fixture, "kb-key-0001").await;
    assert!(
        matches!(&error, KnowledgeBaseError::WriteScopeViolation { paths }
            if paths == &vec!["codegraph.json".to_string()]),
        "{error:?}"
    );
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Failed);
}

// ───────── C2：取消路径审计闭合（停止未确认 + 取消 API 交叉） ─────────

#[tokio::test]
async fn execute_preserves_audit_when_cancel_api_won_over_failed_turn() {
    // 交叉情形：runner 返回普通失败、且取消 API 已把 operation 置为
    // Cancelled——已算出的越界审计事实不得丢弃，须附加到取消记录上。
    let started = Arc::new(tokio::sync::Notify::new());
    let proceed = Arc::new(tokio::sync::Notify::new());
    let fixture = fixture(
        Script::GatedWritesThenFail {
            files: vec![("alpha/src/main.rs", "fn main() { /* touched */ }\n")],
            code: "provider_turn_timed_out",
            retryable: true,
            started: started.clone(),
            proceed: proceed.clone(),
        },
        false,
    );
    let operation = begin_bootstrap(&fixture, "kb-key-0001").await.unwrap();
    let execute = {
        let coordinator = fixture.coordinator.clone();
        let operation_id = operation.operation_id.clone();
        let root = fixture.root.clone();
        tokio::spawn(async move {
            coordinator
                .execute(PROJECT, &operation_id, &root, CancellationToken::new())
                .await
        })
    };
    tokio::time::timeout(std::time::Duration::from_secs(10), started.notified())
        .await
        .expect("turn 应启动");
    // turn 进行中（违规文件已写）：取消 API 先落盘。
    fixture
        .coordinator
        .cancel(PROJECT, &operation.operation_id)
        .unwrap();
    proceed.notify_one();
    let error = execute.await.unwrap().unwrap_err();
    assert!(matches!(error, KnowledgeBaseError::Cancelled));

    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);
    let record = stored.error.expect("交叉情形不得丢弃已算出的审计事实");
    assert_eq!(record.reason_code, "knowledge_base_write_scope_violation");
    assert!(!record.retryable);
    let summary = record.stderr_summary.expect("摘要");
    assert!(summary.contains("alpha/src/main.rs"), "{summary}");
    assert!(summary.contains("provider_turn_timed_out"), "{summary}");
}

#[tokio::test]
async fn execute_records_indeterminate_audit_when_cancelled_with_unconfirmed_provider_stop() {
    // 取消返回但 provider 停止未确认：即使算不出违规，也必须按「审计不确定」
    // fail-closed 留痕，不得静默记成干净取消。
    let fixture = fixture(
        Script::WritesThenCancelUnconfirmed(vec![("cadence/knowledge-base/x.md", "kb")]),
        false,
    );
    let (operation, error) = run_execute(&fixture, "kb-key-0001").await;
    assert!(matches!(error, KnowledgeBaseError::Cancelled));
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);
    let record = stored.error.expect("停止未确认的取消不得记成干净取消");
    assert_eq!(
        record.reason_code,
        "knowledge_base_cancel_audit_indeterminate"
    );
    assert!(!record.retryable);
    assert!(
        record
            .stderr_summary
            .as_deref()
            .is_some_and(|text| text.contains("下界")),
        "{record:?}"
    );
}

#[tokio::test]
async fn execute_appends_indeterminacy_note_when_unconfirmed_cancel_has_violations() {
    // 停止未确认 + 已算出越界：越界事实照常留痕，并注明结果只是下界。
    let fixture = fixture(
        Script::WritesThenCancelUnconfirmed(vec![
            ("alpha/src/main.rs", "fn main() { /* touched */ }\n"),
            ("cadence/knowledge-base/x.md", "kb"),
        ]),
        false,
    );
    let (operation, error) = run_execute(&fixture, "kb-key-0001").await;
    assert!(matches!(error, KnowledgeBaseError::Cancelled));
    let stored = fixture.store.get(PROJECT, &operation.operation_id).unwrap();
    assert_eq!(stored.status, KnowledgeBaseOperationStatus::Cancelled);
    let record = stored.error.expect("越界事实必须留痕");
    assert_eq!(record.reason_code, "knowledge_base_write_scope_violation");
    let summary = record.stderr_summary.expect("摘要");
    assert!(summary.contains("alpha/src/main.rs"), "{summary}");
    assert!(summary.contains("下界"), "{summary}");
}
