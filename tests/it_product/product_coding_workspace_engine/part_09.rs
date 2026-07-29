fn coding_store_with_attempt(
    root: &Path,
    work_item_id: &str,
    branch_name: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    let store = CodingAttemptStore::new(ProductAppPaths::new(root.join(".aria")));
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: branch_name.to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    (store, attempt)
}

fn final_confirm_attempt(
    paths: ProductAppPaths,
    work_item_id: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    let lifecycle = LifecycleStore::new(paths.clone());
    lifecycle
        .create_work_item(CreateWorkItemInput {
            id: Some(work_item_id.to_string()),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            repository_id: "repository_0001".to_string(),
            story_spec_ids: Vec::new(),
            design_spec_ids: Vec::new(),
            title: "work item".to_string(),
            ..Default::default()
        })
        .expect("create work item");
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("set running");
    let attempt = store
        .update_attempt_stage(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingExecutionStage::FinalConfirm,
        )
        .expect("set final confirm stage");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::WaitingForHuman,
        )
        .expect("set waiting for human");
    let attempt = store
        .update_attempt_head_commit(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            Some("deadbeef".to_string()),
        )
        .expect("set head commit");
    (store, attempt)
}

fn failed_attempt(
    paths: ProductAppPaths,
    work_item_id: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    let store = CodingAttemptStore::new(paths);
    let attempt = store
        .create_attempt(CreateCodingAttemptInput {
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            work_item_id: work_item_id.to_string(),
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Codex,
                reviewer: Some(ProviderName::ClaudeCode),
                review_rounds: 1,
            },
            max_auto_rework: 2,
        })
        .expect("create attempt");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Running,
        )
        .expect("set running");
    let attempt = store
        .update_attempt_status(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            CodingAttemptStatus::Failed,
        )
        .expect("set failed");
    (store, attempt)
}

fn dirty_failed_attempt(
    paths: ProductAppPaths,
    work_item_id: &str,
) -> (CodingAttemptStore, CodingExecutionAttempt) {
    failed_attempt(paths, work_item_id)
}

fn run_git(cwd: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {:?} failed\nstdout:{}\nstderr:{}",
        args,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn sample_review_request(attempt_id: &str) -> ReviewRequest {
    ReviewRequest {
        id: "review_request_0001".to_string(),
        attempt_id: attempt_id.to_string(),
        kind: ReviewRequestKind::GitBranchOnly,
        remote_kind: RemoteKind::GenericGit,
        remote: "origin".to_string(),
        base_branch: "main".to_string(),
        branch_name: "aria/work-items/work_item_0001/attempt-1".to_string(),
        commit_sha: "0123456789012345678901234567890123456789".to_string(),
        push_status: PushStatus::Pushed,
        external_url: None,
        manual_instructions: vec!["create review request".to_string()],
        created_at: "2026-05-23T00:00:00Z".to_string(),
        updated_at: "2026-05-23T00:00:00Z".to_string(),
        push_error: None,
    }
}

struct FileWritingStreamingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for FileWritingStreamingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let worktree = input
            .worktree_path
            .as_ref()
            .map(PathBuf::from)
            .expect("worktree path");
        fs::write(worktree.join("generated.txt"), "generated by provider\n").map_err(|error| {
            ProviderAdapterError::incompatible_output(error.to_string(), "", "")
        })?;
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Text("created generated.txt".to_string()))
            .expect("send text chunk");
        tx.try_send(StreamChunk::Done {
            full_output: "done".to_string(),
        })
        .expect("send done chunk");
        Ok(rx)
    }
}

struct PromptCapturingProvider {
    prompt: Arc<Mutex<Option<String>>>,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for PromptCapturingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        *self.prompt.lock().expect("prompt lock") = Some(input.prompt.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Done {
            full_output: "done".to_string(),
        })
        .expect("send done chunk");
        Ok(rx)
    }
}

struct InputCapturingProvider {
    input: Arc<Mutex<Option<AdapterInput>>>,
    output: String,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for InputCapturingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        *self.input.lock().expect("input lock") = Some(input.clone());
        let (tx, rx) = mpsc::channel(8);
        tx.try_send(StreamChunk::Done {
            full_output: self.output.clone(),
        })
        .expect("send done chunk");
        Ok(rx)
    }
}

struct SessionInputCapturingProvider {
    inputs: Arc<Mutex<Vec<StreamingProviderInput>>>,
    outputs: Arc<Mutex<VecDeque<String>>>,
    provider_session_ids: Arc<Mutex<VecDeque<Option<String>>>>,
}

impl Default for SessionInputCapturingProvider {
    fn default() -> Self {
        Self::with_outputs(["coding done"], [Some("coder-session-1".to_string())])
    }
}

impl SessionInputCapturingProvider {
    fn with_outputs<const N: usize, const M: usize>(
        outputs: [&str; N],
        provider_session_ids: [Option<String>; M],
    ) -> Self {
        Self {
            inputs: Arc::new(Mutex::new(Vec::new())),
            outputs: Arc::new(Mutex::new(
                outputs.into_iter().map(ToOwned::to_owned).collect(),
            )),
            provider_session_ids: Arc::new(Mutex::new(provider_session_ids.into_iter().collect())),
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for SessionInputCapturingProvider {
    fn supports_tool_calls(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        self.inputs.lock().expect("inputs lock").push(input);
        let output = self
            .outputs
            .lock()
            .expect("outputs lock")
            .pop_front()
            .unwrap_or_else(|| "coding done".to_string());
        let provider_session_id = self
            .provider_session_ids
            .lock()
            .expect("provider session ids lock")
            .pop_front()
            .unwrap_or_else(|| Some("coder-session-1".to_string()));
        let (event_tx, event_rx) = mpsc::channel(8);
        let (command_tx, _command_rx) = mpsc::channel(8);
        tokio::spawn(async move {
            let _ = event_tx
                .send(ProviderEvent::Completed(cadence_aria::cross_cutting::streaming_provider::ProviderCompletion::plain(output, provider_session_id)))
                .await;
        });
        Ok(ProviderSession {
            events: event_rx,
            commands: command_tx,
        })
    }

    async fn run_streaming(
        &self,
        _input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "run_streaming is not used by this test provider",
            0,
        ))
    }
}

struct EventEmittingCodingProvider;
