// C5 Task 6（REQ-INIT-C5-RESUME）：GAP-J deterministic resume 的产品层
// 定向测试——冻结 input 重放、确定性 operation id、parent/successor 关联、
// 状态分流与滥用面（Review Focus 4）。

/// 可翻转 initializer：先失败（模拟网关不可用），翻开后按既有 Completed
/// 步骤链成功——驱动"恢复后继续"完整闭环。
struct FlippableInitializer {
    fail: AtomicBool,
    count: AtomicUsize,
}

#[async_trait::async_trait]
impl RepositoryInitializer for FlippableInitializer {
    async fn initialize_repository(
        &self,
        _git_root: &std::path::Path,
        _command_timeout: Duration,
        _cancellation: CancellationToken,
        progress: Arc<dyn RepositoryInitializationProgress>,
    ) -> Result<Vec<RepositoryInitializationCommandSummary>, RepositoryRegistrationError> {
        self.count.fetch_add(1, Ordering::SeqCst);
        if self.fail.load(Ordering::SeqCst) {
            let step = RepositoryInitializationStepKind::PreCheck;
            progress.step_started(step).map_err(|error| *error)?;
            progress.step_completed(step).map_err(|error| *error)?;
            let step = RepositoryInitializationStepKind::RuleConfig;
            progress.step_started(step).map_err(|error| *error)?;
            return Err(RepositoryRegistrationError::for_command(
                "repository_initialization",
                "provider_unavailable",
                2,
                step.command().expect("Claude initialization command"),
                Some("claude gateway unavailable".to_string()),
                true,
                "Restore Claude Code availability, then resume repository registration.",
            ));
        }
        let mut summaries = Vec::with_capacity(4);
        for (index, step) in RepositoryInitializationStepKind::ALL
            .into_iter()
            .filter(|step| step.command().is_some())
            .enumerate()
        {
            let command = step.command().expect("Claude initialization command");
            progress.step_started(step).map_err(|error| *error)?;
            progress.step_completed(step).map_err(|error| *error)?;
            summaries.push(RepositoryInitializationCommandSummary {
                command_index: index + 1,
                command: command.to_string(),
                status: "completed".to_string(),
                output_summary: None,
            });
        }
        Ok(summaries)
    }
}

struct ResumeFixture {
    _temp: TempDir,
    coordinator: RepositoryRegistrationCoordinator,
    operations: RepositoryInitializationOperationStore,
    repositories: Arc<ConfigRepositoryPersistence>,
    initializer: Arc<FlippableInitializer>,
    input: RepositoryRegistrationInput,
}

fn resume_fixture() -> ResumeFixture {
    let temp = TempDir::new().unwrap();
    let root = temp.path().join("repository");
    std::fs::create_dir_all(&root).unwrap();
    let operations = RepositoryInitializationOperationStore::new(ProductAppPaths::new(
        temp.path().join(".aria"),
    ));
    let repositories = Arc::new(ConfigRepositoryPersistence::new(vec![], false));
    let initializer = Arc::new(FlippableInitializer {
        fail: AtomicBool::new(true),
        count: AtomicUsize::new(0),
    });
    let coordinator = coordinator_with_operations(
        Arc::new(ConfigProjectLookup { exists: true }),
        repositories.clone(),
        operations.clone(),
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AvailableHealth))),
        Arc::new(StaticCadence {
            failure: None,
            count: AtomicUsize::new(0),
            source_root: temp.path().join("cadence"),
        }),
        Arc::new(|| Ok(())),
        Arc::new(ConfigRunner {
            root: root.clone(),
            rev_parse: Mutex::new(None),
            statuses: Mutex::new(
                vec![
                    command_result(Some(0), "", ""),
                    command_result(Some(0), "?? generated.txt\0", ""),
                ]
                .into(),
            ),
            call_count: AtomicUsize::new(0),
        }),
        initializer.clone(),
    );

    ResumeFixture {
        _temp: temp,
        coordinator,
        operations,
        repositories,
        initializer,
        input: RepositoryRegistrationInput {
            project_id: "project_0001".to_string(),
            name: "Aria".to_string(),
            path: root,
            default_policy_preset: Some("manual-write".to_string()),
            default_provider_mode: Some("claude_code".to_string()),
        },
    }
}

/// 契约固定：`Uuid::NAMESPACE_URL` + UTF-8 名称
/// `cadence/repository-initialization/v1\0{failed_operation_id}\0{command_id}`，
/// 再加 `repository_initialization_` 前缀。
fn expected_resume_operation_id(failed_operation_id: &str, command_id: &str) -> String {
    let name = format!(
        "cadence/repository-initialization/v1\0{failed_operation_id}\0{command_id}"
    );
    format!(
        "repository_initialization_{}",
        uuid::Uuid::new_v5(&uuid::Uuid::NAMESPACE_URL, name.as_bytes()).simple()
    )
}

#[tokio::test]
async fn resume_replays_frozen_input_as_new_operation_with_deterministic_id() {
    let fixture = resume_fixture();

    // 网关不可用：原初始化失败，留下 Failed 终态与冻结 input。
    let launch = fixture
        .coordinator
        .begin_initialization(fixture.input.clone(), CancellationToken::new())
        .await
        .unwrap();
    let original_id = launch.operation_id().to_string();
    fixture
        .coordinator
        .execute_initialization(launch, CancellationToken::new())
        .await
        .unwrap_err();
    let original = fixture.operations.get("project_0001", &original_id).unwrap();
    assert_eq!(original.status, RepositoryInitializationOperationStatus::Failed);
    let frozen_input = original.input.clone();

    // 恢复后继续：网关可用，resume 以冻结 input 创建确定性 successor。
    fixture.initializer.fail.store(false, Ordering::SeqCst);
    let command_id = "cmd-repo-init-resume-1".to_string();
    let resume = fixture
        .coordinator
        .resume_initialization("project_0001", &original_id, &command_id, CancellationToken::new())
        .await
        .unwrap();
    assert!(resume.execute, "fresh successor must be executable once");
    let successor_id = expected_resume_operation_id(&original_id, &command_id);
    assert_eq!(resume.snapshot.operation_id, successor_id);
    assert_eq!(resume.snapshot.status, RepositoryInitializationOperationStatus::Created);
    assert_eq!(resume.snapshot.input, frozen_input, "input must be replayed frozen");
    assert_eq!(resume.snapshot.parent_operation_id.as_deref(), Some(original_id.as_str()));
    assert_eq!(resume.snapshot.resume_command_id.as_deref(), Some(command_id.as_str()));

    // 执行完整步骤至 completed；登记只发生一次。
    fixture
        .coordinator
        .execute_initialization(resume.launch.expect("executable resume carries launch"), CancellationToken::new())
        .await
        .unwrap();
    let successor = fixture.operations.get("project_0001", &successor_id).unwrap();
    assert_eq!(successor.status, RepositoryInitializationOperationStatus::Completed);
    assert_eq!(fixture.repositories.create_count.load(Ordering::SeqCst), 1);

    // 原 Failed 记录只读保留，仅补 superseded_by 关联。
    let original = fixture.operations.get("project_0001", &original_id).unwrap();
    assert_eq!(original.status, RepositoryInitializationOperationStatus::Failed);
    assert_eq!(original.superseded_by.as_deref(), Some(successor_id.as_str()));
    assert_eq!(original.input, frozen_input);

    // 同 command 重放：命中同一 successor，只读返回，不重复 acquire/execute。
    let replay = fixture
        .coordinator
        .resume_initialization("project_0001", &original_id, &command_id, CancellationToken::new())
        .await
        .unwrap();
    assert!(!replay.execute, "completed successor must be read-only");
    assert_eq!(replay.snapshot.operation_id, successor_id);
    assert_eq!(replay.snapshot.status, RepositoryInitializationOperationStatus::Completed);
    assert_eq!(fixture.repositories.create_count.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.initializer.count.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn resume_rejects_non_failed_original_and_invalid_command_ids() {
    let fixture = resume_fixture();

    // 非 Failed 终态（成功链）→ 拒绝且不创建 successor。
    fixture.initializer.fail.store(false, Ordering::SeqCst);
    let launch = fixture
        .coordinator
        .begin_initialization(fixture.input.clone(), CancellationToken::new())
        .await
        .unwrap();
    let completed_id = launch.operation_id().to_string();
    fixture
        .coordinator
        .execute_initialization(launch, CancellationToken::new())
        .await
        .unwrap();
    let error = fixture
        .coordinator
        .resume_initialization(
            "project_0001",
            &completed_id,
            "cmd-1",
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(error.reason_code, "repository_initialization_resume_not_failed");
    assert!(
        fixture
            .operations
            .get(
                "project_0001",
                &expected_resume_operation_id(&completed_id, "cmd-1")
            )
            .is_err()
    );

    // command_id 为空 / 含控制字符 → 422 语义拒绝。
    fixture.initializer.fail.store(true, Ordering::SeqCst);
    let launch = fixture
        .coordinator
        .begin_initialization(fixture.input.clone(), CancellationToken::new())
        .await
        .unwrap();
    let failed_id = launch.operation_id().to_string();
    fixture
        .coordinator
        .execute_initialization(launch, CancellationToken::new())
        .await
        .unwrap_err();
    for command_id in ["", "cmd\u{0}bad", "cmd\u{7}bell"] {
        let error = fixture
            .coordinator
            .resume_initialization("project_0001", &failed_id, command_id, CancellationToken::new())
            .await
            .unwrap_err();
        assert_eq!(
            error.reason_code, "repository_initialization_resume_invalid_command",
            "command_id {command_id:?} must be rejected"
        );
    }

    // 未知 operation → 404 语义（稳定码沿用 operation not found）。
    let error = fixture
        .coordinator
        .resume_initialization(
            "project_0001",
            "repository_initialization_missing",
            "cmd-1",
            CancellationToken::new(),
        )
        .await
        .unwrap_err();
    assert_eq!(
        error.reason_code, "repository_initialization_operation_not_found"
    );
}

#[tokio::test]
async fn resume_still_unavailable_fails_successor_without_fake_success() {
    let fixture = resume_fixture();

    let launch = fixture
        .coordinator
        .begin_initialization(fixture.input.clone(), CancellationToken::new())
        .await
        .unwrap();
    let original_id = launch.operation_id().to_string();
    fixture
        .coordinator
        .execute_initialization(launch, CancellationToken::new())
        .await
        .unwrap_err();

    // 网关仍不可用：successor 再次 Failed，保留 parent 链与诊断，不伪造成功、
    // 不创建 Repository 记录。
    let command_id = "cmd-repo-init-resume-still-down".to_string();
    let resume = fixture
        .coordinator
        .resume_initialization("project_0001", &original_id, &command_id, CancellationToken::new())
        .await
        .unwrap();
    let successor_id = expected_resume_operation_id(&original_id, &command_id);
    fixture
        .coordinator
        .execute_initialization(resume.launch.expect("executable resume carries launch"), CancellationToken::new())
        .await
        .unwrap_err();
    let successor = fixture.operations.get("project_0001", &successor_id).unwrap();
    assert_eq!(successor.status, RepositoryInitializationOperationStatus::Failed);
    assert_eq!(successor.parent_operation_id.as_deref(), Some(original_id.as_str()));
    assert_eq!(
        successor.error.as_ref().map(|error| error.reason_code.clone()),
        Some("provider_unavailable".to_string())
    );
    assert_eq!(fixture.repositories.create_count.load(Ordering::SeqCst), 0);

    // 失败后：新的显式 command 派生新的 operation（新的恢复链叶）。
    let second_command_id = "cmd-repo-init-resume-recovered".to_string();
    fixture.initializer.fail.store(false, Ordering::SeqCst);
    let second = fixture
        .coordinator
        .resume_initialization("project_0001", &successor_id, &second_command_id, CancellationToken::new())
        .await
        .unwrap();
    let second_id = expected_resume_operation_id(&successor_id, &second_command_id);
    assert_ne!(second_id, successor_id);
    assert_eq!(second.snapshot.parent_operation_id.as_deref(), Some(successor_id.as_str()));
    fixture
        .coordinator
        .execute_initialization(second.launch.expect("executable resume carries launch"), CancellationToken::new())
        .await
        .unwrap();
    assert_eq!(fixture.repositories.create_count.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn resume_replays_created_successor_once_across_repeated_calls() {
    let fixture = resume_fixture();

    let launch = fixture
        .coordinator
        .begin_initialization(fixture.input.clone(), CancellationToken::new())
        .await
        .unwrap();
    let original_id = launch.operation_id().to_string();
    fixture
        .coordinator
        .execute_initialization(launch, CancellationToken::new())
        .await
        .unwrap_err();
    fixture.initializer.fail.store(false, Ordering::SeqCst);

    // 模拟 resume 后、执行前服务中断：Created successor 落盘但未执行。
    let command_id = "cmd-repo-init-resume-created".to_string();
    let first = fixture
        .coordinator
        .resume_initialization("project_0001", &original_id, &command_id, CancellationToken::new())
        .await
        .unwrap();
    drop(first);

    // 重复点击：同 command 命中同一 Created successor，仍可执行一次（不重复创建）。
    let second = fixture
        .coordinator
        .resume_initialization("project_0001", &original_id, &command_id, CancellationToken::new())
        .await
        .unwrap();
    assert!(second.execute, "created successor stays executable exactly once");
    assert_eq!(
        second.snapshot.operation_id,
        expected_resume_operation_id(&original_id, &command_id)
    );

    let listed = fixture.operations.list("project_0001").unwrap();
    let chain: Vec<&str> = listed
        .iter()
        .filter(|op| op.parent_operation_id.is_some())
        .map(|op| op.operation_id.as_str())
        .collect();
    assert_eq!(chain.len(), 1, "no duplicate successors: {listed:?}");
}
