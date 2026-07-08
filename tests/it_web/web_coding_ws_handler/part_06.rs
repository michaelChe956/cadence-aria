#[derive(Clone, Copy)]
enum BlockedReviewerStage {
    CodeReview,
    InternalPrReview,
}

struct ReviewerBlockedProvider {
    blocked_stage: BlockedReviewerStage,
}

impl ReviewerBlockedProvider {
    fn code_review() -> Self {
        Self {
            blocked_stage: BlockedReviewerStage::CodeReview,
        }
    }

    fn internal_pr_review() -> Self {
        Self {
            blocked_stage: BlockedReviewerStage::InternalPrReview,
        }
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ReviewerBlockedProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        start_web_test_provider_driven_testing_session(&input.prompt, cancel)
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        match input.role {
            AdapterRole::Executor => {
                let worktree = input
                    .worktree_path
                    .as_ref()
                    .map(PathBuf::from)
                    .expect("worktree path");
                fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                })?;
                tx.try_send(StreamChunk::Done {
                    full_output: "implemented climb_stairs".to_string(),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer if input.output_schema == "coding_workspace_code_review_json" => {
                let full_output = match self.blocked_stage {
                    BlockedReviewerStage::CodeReview => {
                        r#"{"verdict":"blocked","summary":"code review 缺少人工确认信息","findings":[]}"#
                    }
                    BlockedReviewerStage::InternalPrReview => {
                        r#"{"verdict":"approve","summary":"code review ok","findings":[]}"#
                    }
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send code review done");
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_internal_pr_review_json" =>
            {
                let full_output = match self.blocked_stage {
                    BlockedReviewerStage::CodeReview => {
                        r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#
                    }
                    BlockedReviewerStage::InternalPrReview => {
                        r#"{"verdict":"blocked","summary":"internal review 需要人工确认发布窗口","findings":[],"impact_scope":["release"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#
                    }
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send internal review done");
            }
            _ => {
                tx.try_send(StreamChunk::Done {
                    full_output: "ok".to_string(),
                })
                .expect("send done");
            }
        }
        Ok(rx)
    }
}

#[derive(Default)]
struct InternalReviewReworkProvider {
    state: Mutex<InternalReviewReworkState>,
}

#[derive(Default)]
struct InternalReviewReworkState {
    coding_calls: usize,
    internal_review_calls: usize,
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for InternalReviewReworkProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        start_web_test_provider_driven_testing_session(&input.prompt, cancel)
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        match input.role {
            AdapterRole::Executor => {
                let worktree = input
                    .worktree_path
                    .as_ref()
                    .map(PathBuf::from)
                    .expect("worktree path");
                let coding_call = {
                    let mut state = self.state.lock().expect("state lock");
                    state.coding_calls += 1;
                    state.coding_calls
                };
                fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                })?;
                if coding_call >= 2 {
                    fs::write(
                        worktree.join("src/internal_fix.rs"),
                        "pub const FIXED: bool = true;\n",
                    )
                    .map_err(|error| {
                        ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                    })?;
                }
                tx.try_send(StreamChunk::Text(format!("coding round {coding_call}")))
                    .expect("send coding chunk");
                tx.try_send(StreamChunk::Done {
                    full_output: format!("coding round {coding_call} done"),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer
                if input.output_schema == "coding_workspace_internal_pr_review_json" =>
            {
                let internal_review_call = {
                    let mut state = self.state.lock().expect("state lock");
                    state.internal_review_calls += 1;
                    state.internal_review_calls
                };
                let full_output = if internal_review_call == 1 {
                    r#"{"verdict":"request_changes","summary":"需要 internal fix","findings":[{"severity":"medium","file":"src/internal_fix.rs","description":"缺少 internal fix","recommendation":"补充 internal_fix.rs"}],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#
                } else {
                    r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send internal review done");
            }
            AdapterRole::Reviewer => {
                tx.try_send(StreamChunk::Done {
                    full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                        .to_string(),
                })
                .expect("send review done");
            }
            _ => {
                tx.try_send(StreamChunk::Done {
                    full_output: "ok".to_string(),
                })
                .expect("send done");
            }
        }
        Ok(rx)
    }
}

#[derive(Default)]
struct CodeReviewReworkProvider {
    state: Mutex<CodeReviewReworkState>,
}

#[derive(Default)]
struct CodeReviewReworkState {
    coding_calls: usize,
    code_review_calls: usize,
    coding_prompts: Vec<String>,
}

impl CodeReviewReworkProvider {
    fn coding_prompts(&self) -> Vec<String> {
        self.state
            .lock()
            .expect("state lock")
            .coding_prompts
            .clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for CodeReviewReworkProvider {
    fn supports_provider_driven_testing(&self) -> bool {
        true
    }

    async fn start(
        &self,
        input: StreamingProviderInput,
        cancel: CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        start_web_test_provider_driven_testing_session(&input.prompt, cancel)
    }

    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        match input.role {
            AdapterRole::Executor => {
                let worktree = input
                    .worktree_path
                    .as_ref()
                    .map(PathBuf::from)
                    .expect("worktree path");
                let coding_call = {
                    let mut state = self.state.lock().expect("state lock");
                    state.coding_calls += 1;
                    state.coding_prompts.push(input.prompt.clone());
                    state.coding_calls
                };
                fs::write(worktree.join("src/lib.rs"), CLIMB_STAIRS_LIB).map_err(|error| {
                    ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                })?;
                if coding_call == 1 {
                    fs::create_dir_all(worktree.join("__pycache__")).map_err(|error| {
                        ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                    })?;
                    fs::write(
                        worktree.join("__pycache__/climbing_stairs.cpython-310.pyc"),
                        b"pyc",
                    )
                    .map_err(|error| {
                        ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                    })?;
                } else {
                    let _ = fs::remove_dir_all(worktree.join("__pycache__"));
                    fs::write(
                        worktree.join("src/review_fix.rs"),
                        "pub const FIXED: bool = true;\n",
                    )
                    .map_err(|error| {
                        ProviderAdapterError::incompatible_output(error.to_string(), "", "")
                    })?;
                }
                tx.try_send(StreamChunk::Done {
                    full_output: format!("coding round {coding_call} done"),
                })
                .expect("send coding done");
            }
            AdapterRole::Reviewer if input.output_schema == "coding_workspace_code_review_json" => {
                let code_review_call = {
                    let mut state = self.state.lock().expect("state lock");
                    state.code_review_calls += 1;
                    state.code_review_calls
                };
                let full_output = if code_review_call == 1 {
                    r#"{"verdict":"request_changes","summary":"运行产物进入 diff","findings":[{"severity":"medium","file":"__pycache__/climbing_stairs.cpython-310.pyc","description":"不应提交 pyc","recommendation":"移除 __pycache__ 和 .pyc 文件"}]}"#
                } else {
                    r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                };
                tx.try_send(StreamChunk::Done {
                    full_output: full_output.to_string(),
                })
                .expect("send code review done");
            }
            AdapterRole::Reviewer => {
                tx.try_send(StreamChunk::Done {
                    full_output: r#"{"verdict":"approve","summary":"internal review ok","findings":[],"impact_scope":["src"],"pr_description":"实现 work item","commit_message_suggestion":"feat: implement work item"}"#.to_string(),
                })
                .expect("send internal review done");
            }
            _ => {
                tx.try_send(StreamChunk::Done {
                    full_output: "ok".to_string(),
                })
                .expect("send done");
            }
        }
        Ok(rx)
    }
}

fn start_web_test_provider_driven_testing_session(
    prompt: &str,
    cancel: CancellationToken,
) -> Result<ProviderSession, ProviderAdapterError> {
    let Some(output) = web_test_provider_driven_testing_output(prompt) else {
        return Err(ProviderAdapterError::execution_failed(
            None,
            String::new(),
            "streaming provider start is not implemented",
            0,
        ));
    };
    let (event_tx, event_rx) = mpsc::channel(8);
    let (command_tx, _command_rx) = mpsc::channel(8);

    tokio::spawn(async move {
        if cancel.is_cancelled() {
            return;
        }
        if event_tx
            .send(ProviderEvent::TextDelta {
                content: output.clone(),
            })
            .await
            .is_err()
        {
            return;
        }
        if cancel.is_cancelled() {
            return;
        }
        let _ = event_tx
            .send(ProviderEvent::Completed {
                full_output: output,
                provider_session_id: None,
            })
            .await;
    });

    Ok(ProviderSession {
        events: event_rx,
        commands: command_tx,
    })
}

fn web_test_provider_driven_testing_output(prompt: &str) -> Option<String> {
    if prompt.contains("Tester Provider Runtime") && prompt.contains("Phase: plan_tests") {
        return Some(
            json!({
                "summary": "web integration provider-driven test plan",
                "steps": [{
                    "id": "cargo_test",
                    "title": "Cargo test",
                    "intent": "verify the coding worktree with the provider-managed test fixture",
                    "required": true,
                    "tool": "provider_managed",
                    "risk_level": "low",
                    "command_or_tool_input": {
                        "command": "cargo test --locked"
                    },
                    "evidence_expectation": "provider reports deterministic cargo test evidence",
                    "related_requirements": ["REQ-CARGO"],
                    "related_design_constraints": ["DEC-CARGO"],
                    "related_work_item_tasks": ["TASK-CARGO"]
                }]
            })
            .to_string(),
        );
    }
    if prompt.contains("Tester Provider Runtime") && prompt.contains("Phase: execute_test_plan") {
        return Some(
            json!({
                "step_results": [{
                    "step_id": "cargo_test",
                    "status": "passed",
                    "evidence_refs": ["web-it-provider-driven-testing.log"],
                    "provider_analysis": "web integration fixture completed deterministic provider-managed testing"
                }]
            })
            .to_string(),
        );
    }
    None
}

struct HangingCodingProvider;

#[async_trait::async_trait]
impl StreamingProviderAdapter for HangingCodingProvider {
    async fn run_streaming(
        &self,
        input: &AdapterInput,
        _cancel: CancellationToken,
    ) -> Result<mpsc::Receiver<StreamChunk>, ProviderAdapterError> {
        let (tx, rx) = mpsc::channel(8);
        if input.role == AdapterRole::Executor {
            tokio::spawn(async move {
                let _ = tx
                    .send(StreamChunk::Text("hanging provider started".to_string()))
                    .await;
                tokio::time::sleep(Duration::from_secs(60)).await;
            });
        } else {
            tx.try_send(StreamChunk::Done {
                full_output: r#"{"verdict":"approve","summary":"review ok","findings":[]}"#
                    .to_string(),
            })
            .expect("send done");
        }
        Ok(rx)
    }
}

const CLIMB_STAIRS_LIB: &str = r#"pub fn climb_stairs(n: u32) -> u32 {
    if n <= 2 {
        return n;
    }
    let mut prev = 1;
    let mut curr = 2;
    for _ in 3..=n {
        let next = prev + curr;
        prev = curr;
        curr = next;
    }
    curr
}

#[cfg(test)]
mod tests {
    use super::climb_stairs;

    #[test]
    fn computes_climb_stairs_examples() {
        assert_eq!(climb_stairs(1), 1);
        assert_eq!(climb_stairs(2), 2);
        assert_eq!(climb_stairs(3), 3);
        assert_eq!(climb_stairs(5), 8);
        assert_eq!(climb_stairs(10), 89);
    }
}
"#;
