# WorkItemPlan 完整流式气泡 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Work Item Plan 生成和 Story Spec、Design Spec 一样，通过标准作者气泡持续展示 provider 生成过程，最终仍产出可校验的 Issue Work Item Plan candidate。

**Architecture:** 后端不再把 Work Item Plan 的 provider 输出塞进 `start_generation` 节点，而是在 `start_generation` 后创建标准 `author_run` / `revision` active 节点，并使用 `StreamingProviderAdapter.start()` 接收 `ProviderEvent::TextDelta`。完成时从 `Completed.full_output` 解析最后一个 `<ARIA_STRUCTURED_OUTPUT>` JSON，继续复用现有 `complete_work_item_plan_author` / `complete_work_item_plan_revision` 的校验、候选落盘和 AutoRevision 逻辑。

**Tech Stack:** Rust 1.95.0、Axum WebSocket、tokio mpsc、serde_json、React 19、Zustand、Vitest、pnpm、Cargo 宿主机命令。

---

## 当前结论

- `src/product/work_item_split_engine.rs` 当前使用 `ProviderAdapter.run()`，这是阻塞式调用，只有最终 structured output，没有 provider text delta。
- `src/web/workspace_ws_handler.rs` 的 WorkItemPlanAuthor / WorkItemPlanRevision 分支只发送“准备上下文 / 调用 provider / 解析校验”这类包装进度文案。
- `start_generation` 节点是 completed system anchor，`agent = None`，不适合承载 provider 正文流；Story/Design 的标准气泡来自 `author_run` 节点。
- 现有 prompt 要求“只能输出 sentinel JSON block”，即使接入 streaming，也可能只流 JSON。为了接近 Story/Design 的体验，应允许 provider 在 sentinel 前输出可读生成过程，后端只解析最后一个 sentinel JSON。

## 文件结构

- Modify: `src/product/work_item_split_engine.rs`
  - 拆出 Work Item Plan prompt 构造与 structured output 解析/保存 helper。
  - prompt 改为允许 sentinel 前可读流式说明，末尾必须输出 structured sentinel。
- Modify: `src/product/workspace_engine.rs`
  - 新增 Work Item Plan author run 节点创建 helper。
  - 新增 provider session collector：复用 `ProviderEvent` 处理、向标准 author/revision 节点写 `stream_chunk`，完成时返回 `full_output`。
- Modify: `src/web/workspace_ws_handler.rs`
  - WorkItemPlanAuthor / WorkItemPlanRevision 改用 provider registry 中的 streaming provider。
  - provider 完成后解析 structured output 并进入现有 candidate 校验逻辑。
- Modify: `src/cross_cutting/streaming_provider.rs`
  - Fake streaming provider 对 `AdapterRole::WorkItemSplitter` 输出可读流 + sentinel JSON，方便集成测试覆盖真实流式链路。
- Modify: `tests/it_web/web_work_item_plan_author.rs`
  - 断言 Work Item Plan 生成期间收到 provider 正文流，节点类型为 `author_run`，且在 candidate 前出现。
  - 保留 candidate artifact_update 断言。
- Modify: `tests/it_web/web_workspace_recovery_consistency.rs`
  - 恢复测试改为断言 Work Item Plan author stream 属于 `author_run` / `revision` 节点，不再依赖 `start_generation` 承载流。
- Modify: `web/src/state/workspace-ws-store.test.ts`
  - 更新 Work Item Plan 恢复重建测试，使用 `author_run` 节点。
- Modify: `web/src/hooks/useWorkspaceWs.test.tsx`
  - 覆盖 Work Item Plan 标准 author node 的 stream chunk 可以进入 `provider_stream`，并带 `provider` metadata。
- Modify: `web/src/components/chat-workspace/message-grouping.test.ts`
  - 确认 `start_generation` 不会和 Work Item Plan provider stream 合并成同一个系统气泡。

## 设计决策

1. **不用全局 `/api/events` 的 provider.output_stream。** 该事件没有当前 workspace timeline node id，也不是当前 ChatWorkspace 的 WebSocket 数据源；接它会绕开已有 Workspace 恢复模型。
2. **不再向 `start_generation` 写 provider 内容。** `start_generation` 只表示“用户点击开始生成 / provider 已锁定”，实际生成内容必须进入 `author_run` 或 `revision` 节点。
3. **不复用现有 `drive_provider_session` 的完成逻辑。** 现有方法在 `Completed` 时会调用 Story/Design 的 `complete_generation(full_output)`；Work Item Plan 需要先解析 structured JSON，再调用专用 `complete_work_item_plan_author`。
4. **允许 sentinel 前可读输出。** parser 已经使用 `parse_last_structured_output`，可以解析最后一个 sentinel；这样气泡展示的是可读过程，后端仍拿结构化 JSON 落盘。

## Task 1: 后端红灯测试覆盖完整 provider 流

**Files:**
- Modify: `tests/it_web/web_work_item_plan_author.rs`

- [ ] **Step 1: 写失败测试**

把现有 `work_item_plan_author_streams_progress_before_candidate_artifact` 升级为 provider 正文流测试。核心断言不是“正在生成 Work Item Plan”，而是 fake streaming provider 发出的正文内容，并且节点类型必须是 `author_run`。

```rust
#[tokio::test]
async fn work_item_plan_author_streams_provider_output_before_candidate_artifact() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "登录拆分",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false,
            "review_rounds": 1
        }),
    )
    .await;

    let session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();
    let mut ws = connect_ws(app, &session_id).await;

    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 1 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let mut author_node_id: Option<String> = None;
    let mut saw_provider_stream = false;
    let mut saw_candidate = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        let value = match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => {
                serde_json::from_str::<Value>(&text).expect("ws json")
            }
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(other))) => panic!("expected text ws message, got {other:?}"),
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) => break,
            Err(_) => break,
        };

        match value["type"].as_str() {
            Some("timeline_node_created")
                if value["node"]["node_type"] == "author_run"
                    && value["node"]["title"] == "Work Item Plan 生成" =>
            {
                author_node_id = value["node"]["node_id"].as_str().map(str::to_string);
            }
            Some("stream_chunk")
                if author_node_id
                    .as_deref()
                    .is_some_and(|node_id| value["node_id"].as_str() == Some(node_id))
                    && value["content"]
                        .as_str()
                        .unwrap_or("")
                        .contains("Fake Work Item Plan streaming draft") =>
            {
                saw_provider_stream = true;
            }
            Some("artifact_update") if value.get("candidate").is_some() => {
                saw_candidate = true;
                break;
            }
            Some("error") => panic!("ws error message: {value}"),
            _ => {}
        }
    }

    assert!(author_node_id.is_some(), "expected dedicated author_run node");
    assert!(saw_provider_stream, "expected provider text stream before candidate");
    assert!(saw_candidate, "expected candidate artifact_update");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --locked --test it_web work_item_plan_author_streams_provider_output_before_candidate_artifact`

Expected: FAIL。当前不会创建 Work Item Plan 专用 `author_run`，也不会流出 `Fake Work Item Plan streaming draft`。

- [ ] **Step 3: 提交红灯测试**

```bash
git add tests/it_web/web_work_item_plan_author.rs
git commit -m "test: require work item plan provider stream"
```

## Task 2: 拆出 WorkItemSplitEngine 的 prompt 与 structured 解析

**Files:**
- Modify: `src/product/work_item_split_engine.rs`

- [ ] **Step 1: 写单元测试锁定 prompt 规则**

在 `src/product/work_item_split_engine.rs` tests 中替换原“只能输出 sentinel”断言，改成“允许 sentinel 前可读内容，但末尾必须有 structured sentinel”。

```rust
#[test]
fn build_split_prompt_allows_readable_stream_before_final_sentinel() {
    let request = make_generate_request();
    let issue = make_issue();
    let repository = make_repository();

    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

    assert!(prompt.contains("可以在最终结构化 JSON 前输出简短、可读的拆分过程"));
    assert!(prompt.contains("最后必须输出一个 <ARIA_STRUCTURED_OUTPUT> JSON block"));
    assert!(prompt.contains("</ARIA_STRUCTURED_OUTPUT>"));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --locked --lib build_split_prompt_allows_readable_stream_before_final_sentinel`

Expected: FAIL，当前 prompt 仍要求“最终输出必须只包含一个 sentinel JSON block”。

- [ ] **Step 3: 新增 invocation 类型和 helper**

在 `src/product/work_item_split_engine.rs` 增加：

```rust
#[derive(Debug, Clone)]
pub struct WorkItemSplitInvocation {
    pub prompt: String,
    pub provider_type: ProviderType,
    pub worktree_path: String,
    pub author_provider: ProviderName,
}
```

在 `impl WorkItemSplitEngine` 中增加：

```rust
pub fn build_generate_invocation(
    request: &GenerateWorkItemsRequest,
    lifecycle: &LifecycleStore,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    author_provider: ProviderName,
) -> ApiResult<WorkItemSplitInvocation> {
    let story_context = collect_story_context(lifecycle, request, issue)?;
    let design_context = collect_design_context(lifecycle, request, issue)?;
    let repository_structure = summarize_repository_structure(&repository.path);
    let prompt = build_split_prompt(
        request,
        issue,
        repository,
        &story_context,
        &design_context,
        &repository_structure,
    );

    Ok(WorkItemSplitInvocation {
        prompt,
        provider_type: provider_name_to_type(&author_provider),
        worktree_path: repository.path.to_string_lossy().to_string(),
        author_provider,
    })
}

pub fn build_revision_invocation(
    request: &GenerateWorkItemsRequest,
    lifecycle: &LifecycleStore,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    author_provider: ProviderName,
    retained: &[LifecycleWorkItemRecord],
    redo_specs: &[RedoSpec],
) -> ApiResult<WorkItemSplitInvocation> {
    let story_context = collect_story_context(lifecycle, request, issue)?;
    let design_context = collect_design_context(lifecycle, request, issue)?;
    let repository_structure = summarize_repository_structure(&repository.path);
    let prompt = build_revision_prompt(
        request,
        issue,
        repository,
        retained,
        redo_specs,
        &story_context,
        &design_context,
        &repository_structure,
    );

    Ok(WorkItemSplitInvocation {
        prompt,
        provider_type: provider_name_to_type(&author_provider),
        worktree_path: repository.path.to_string_lossy().to_string(),
        author_provider,
    })
}
```

- [ ] **Step 4: 新增 structured completion helper**

仍在 `impl WorkItemSplitEngine` 中增加：

```rust
pub fn complete_generate_from_structured_output(
    request: &GenerateWorkItemsRequest,
    lifecycle: &LifecycleStore,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    author_provider: &ProviderName,
    prompt: &str,
    structured_output: serde_json::Value,
) -> ApiResult<WorkItemSplitProviderOutput> {
    let run_ref = lifecycle
        .save_work_item_split_provider_run(
            &issue.project_id,
            &issue.id,
            author_provider,
            prompt,
            &structured_output,
        )
        .map_err(product_store_api_error)?;

    parse_provider_output(
        lifecycle,
        request,
        issue,
        repository,
        run_ref,
        &structured_output,
    )
}
```

Revision helper 保留 retained/redo 分支：

```rust
pub fn complete_revision_from_structured_output(
    request: &GenerateWorkItemsRequest,
    lifecycle: &LifecycleStore,
    issue: &IssueRecord,
    repository: &RepositoryRecord,
    author_provider: &ProviderName,
    prompt: &str,
    structured_output: serde_json::Value,
    retained: &[LifecycleWorkItemRecord],
    redo_specs: &[RedoSpec],
) -> ApiResult<WorkItemSplitProviderOutput> {
    let run_ref = lifecycle
        .save_work_item_split_provider_run(
            &issue.project_id,
            &issue.id,
            author_provider,
            prompt,
            &structured_output,
        )
        .map_err(product_store_api_error)?;

    if retained.is_empty() && redo_specs.is_empty() {
        return parse_provider_output(
            lifecycle,
            request,
            issue,
            repository,
            run_ref,
            &structured_output,
        );
    }

    let redo = parse_revision_redo_output(&structured_output)?;
    materialize_revision_output(
        lifecycle,
        request,
        issue,
        repository,
        run_ref,
        redo,
        retained,
        redo_specs,
    )
}
```

`materialize_revision_output` 是从现有 `generate_revision` 中抽出的私有 helper，保持原有 id mapping、DAG repatch、repository profile 构造逻辑不变。

- [ ] **Step 5: 修改 prompt 文案**

把 `build_split_prompt` 和 `build_revision_prompt` 的 output_schema 说明改成：

```rust
"[output_schema]\n\
 可以在最终结构化 JSON 前输出简短、可读的拆分过程，供 Workbench 流式展示。\n\
 最后必须输出一个 <ARIA_STRUCTURED_OUTPUT> JSON block。\n\
 后端只解析最后一个 <ARIA_STRUCTURED_OUTPUT>...</ARIA_STRUCTURED_OUTPUT> block。\n\
 标签内部必须是一个完整 JSON object，不要输出 Markdown code fence。\n\
 严格按以下 JSON schema 输出。\n\
 ..."
```

- [ ] **Step 6: 保持旧 generate API 兼容**

让现有阻塞式 `generate` 调用新 helper，避免影响其他调用点：

```rust
let invocation = Self::build_generate_invocation(
    request,
    lifecycle,
    issue,
    repository,
    author_provider,
)?;
let provider_output = self
    .invoke_provider(
        &invocation.prompt,
        repository,
        invocation.author_provider.clone(),
        lifecycle,
        issue,
    )
    .await?;
Self::complete_generate_from_structured_output(
    request,
    lifecycle,
    issue,
    repository,
    &invocation.author_provider,
    &invocation.prompt,
    provider_output.structured_output,
)
```

- [ ] **Step 7: 运行单元测试**

Run: `cargo test --locked --lib build_split_prompt_allows_readable_stream_before_final_sentinel`

Expected: PASS。

- [ ] **Step 8: 提交**

```bash
git add src/product/work_item_split_engine.rs
git commit -m "refactor: split work item plan provider invocation"
```

## Task 3: Fake streaming provider 支持 WorkItemSplitter

**Files:**
- Modify: `src/cross_cutting/streaming_provider.rs`

- [ ] **Step 1: 写失败测试**

在 `src/cross_cutting/streaming_provider.rs` tests 中增加：

```rust
#[tokio::test]
async fn fake_streaming_provider_outputs_work_item_split_sentinel() {
    let provider = FakeStreamingProvider;
    let input = StreamingProviderInput {
        provider_type: ProviderType::Fake,
        role: AdapterRole::WorkItemSplitter,
        prompt: "你是 Aria 的 Work Item Splitter".to_string(),
        working_dir: std::env::current_dir().unwrap(),
        workspace_session_id: Some("workspace_session_0001".to_string()),
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Supervised,
        env_vars: BTreeMap::new(),
        timeout_secs: 60,
    };

    let mut session = provider.start(input, CancellationToken::new()).await.unwrap();
    let mut streamed = String::new();
    let mut completed = None;
    while let Some(event) = session.events.recv().await {
        match event {
            ProviderEvent::TextDelta { content } => streamed.push_str(&content),
            ProviderEvent::Completed { full_output, .. } => {
                completed = Some(full_output);
                break;
            }
            _ => {}
        }
    }

    let full_output = completed.expect("completed output");
    assert!(streamed.contains("Fake Work Item Plan streaming draft"));
    assert!(full_output.contains("<ARIA_STRUCTURED_OUTPUT>"));
    assert!(full_output.contains("\"work_items\""));
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --locked --lib fake_streaming_provider_outputs_work_item_split_sentinel`

Expected: FAIL，当前 fake streaming provider 不为 WorkItemSplitter 输出 structured sentinel。

- [ ] **Step 3: 实现 fake 输出**

在 `fake_workspace_markdown` 开头加入：

```rust
if prompt.contains("Work Item Splitter") || prompt.contains("IssueWorkItemPlan") {
    return format!(
        "Fake Work Item Plan streaming draft\n\n\
         - 分析 Story/Design 约束\n\
         - 拆分可执行 Work Item\n\n\
         <ARIA_STRUCTURED_OUTPUT>{}</ARIA_STRUCTURED_OUTPUT>",
        serde_json::json!({
            "repository_profile": {
                "confidence": "high",
                "detected_layers": ["backend"],
                "split_recommendation": "single_work_item",
                "languages": ["rust"],
                "frameworks": [],
                "package_managers": ["cargo"],
                "test_frameworks": ["cargo test"],
                "build_systems": ["cargo"],
                "verification_capabilities": ["unit_tests"],
                "uncertainties": []
            },
            "plan": {
                "work_item_ids": ["wi_01"],
                "dependency_graph": []
            },
            "work_items": [{
                "title": "实现 Work Item Plan 流式输出",
                "kind": "backend",
                "sequence_hint": 1,
                "depends_on": [],
                "exclusive_write_scopes": ["src/web/workspace_ws_handler.rs"],
                "forbidden_write_scopes": [],
                "context_budget": {},
                "required_handoff_from": [],
                "require_execution_plan_confirm": false
            }],
            "verification_plans": [{
                "scope": "unit",
                "commands": [{
                    "id": "cmd_001",
                    "label": "cargo test",
                    "command": "cargo test --locked --lib work_item_plan",
                    "cwd": ".",
                    "purpose": "验证 Work Item Plan 流式输出",
                    "required": true,
                    "timeout_seconds": 120,
                    "safety": "read_only"
                }],
                "manual_checks": [],
                "required_gates": ["provider stream visible before candidate"],
                "risk_notes": [],
                "confidence": "high",
                "fallback_policy": "manual_review"
            }]
        })
    );
}
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --locked --lib fake_streaming_provider_outputs_work_item_split_sentinel`

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/cross_cutting/streaming_provider.rs
git commit -m "test: support fake work item split streaming"
```

## Task 4: WorkspaceEngine 增加 WorkItemPlan 专用 streaming collector

**Files:**
- Modify: `src/product/workspace_engine.rs`

- [ ] **Step 1: 写 engine 单元测试**

新增测试证明 Work Item Plan author run 会创建标准 `author_run` 节点并持久化 provider stream。

```rust
#[tokio::test]
async fn begin_work_item_plan_author_run_creates_standard_author_node() {
    let (mut engine, _temp) = make_work_item_plan_engine_with_draft_candidate(
        "sess_work_item_plan_author_stream_node",
    );

    let node_id = engine.begin_work_item_plan_author_run().await;
    let node = engine
        .timeline_nodes
        .iter()
        .find(|node| node.node_id == node_id)
        .expect("author node");

    assert_eq!(node.node_type, TimelineNodeType::AuthorRun);
    assert_eq!(node.stage, WsWorkspaceStage::Running);
    assert_eq!(node.agent, Some(ProviderName::ClaudeCode));
    assert_eq!(node.status, TimelineNodeStatus::Active);
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --locked --lib begin_work_item_plan_author_run_creates_standard_author_node`

Expected: FAIL，helper 不存在。

- [ ] **Step 3: 新增 author run helper**

在 `impl WorkspaceEngine` 增加：

```rust
pub async fn begin_work_item_plan_author_run(&mut self) -> String {
    if self.session.stage != WorkspaceStage::Running {
        self.transition_stage(WorkspaceStage::Running).await;
    }
    self.create_timeline_node(TimelineNodeDraft {
        node_type: TimelineNodeType::AuthorRun,
        agent: Some(self.session.author_provider.clone()),
        stage: WorkspaceStage::Running,
        round: None,
        title: "Work Item Plan 生成".to_string(),
        summary: None,
        status: TimelineNodeStatus::Active,
    })
    .await
}
```

- [ ] **Step 4: 新增 streaming input builder**

```rust
pub fn build_work_item_plan_streaming_input(
    &self,
    provider_type: ProviderType,
    prompt: String,
    worktree_path: String,
) -> StreamingProviderInput {
    StreamingProviderInput {
        provider_type,
        role: AdapterRole::WorkItemSplitter,
        prompt,
        working_dir: PathBuf::from(worktree_path),
        workspace_session_id: Some(self.session.session_id.clone()),
        resume_provider_session_id: None,
        permission_mode: ProviderPermissionMode::Supervised,
        env_vars: BTreeMap::new(),
        timeout_secs: DEFAULT_PROVIDER_TIMEOUT_SECS,
    }
}
```

- [ ] **Step 5: 新增 collector**

新增 public 方法，逻辑和 `drive_provider_session` 的事件处理保持一致，但 `Completed` 时返回 `full_output`，不调用 `complete_generation`：

```rust
pub async fn drive_work_item_plan_provider_session_to_output(
    &mut self,
    session: Result<ProviderSession, crate::cross_cutting::provider_adapter::ProviderAdapterError>,
    mut command_rx: mpsc::Receiver<ProviderCommand>,
    node_id: String,
    agent: ProviderName,
) -> Result<String, String> {
    let mut session = match session {
        Ok(session) => session,
        Err(error) => {
            let message = error.details.clone();
            let _ = self.event_tx.send(EngineEvent::Error { message: message.clone() }).await;
            self.finish_failed_run().await;
            return Err(message);
        }
    };

    let cancel = self.cancel.clone();
    let mut commands_open = true;

    loop {
        tokio::select! {
            _ = cancel.cancelled() => {
                let _ = self.flush_stream_buffer(&node_id).await;
                self.finish_aborted_run().await;
                return Err("provider run aborted".to_string());
            }
            command = command_rx.recv(), if commands_open => {
                match command {
                    Some(ProviderCommand::Abort) => {
                        let _ = session.commands.send(ProviderCommand::Abort).await;
                        cancel.cancel();
                        let _ = self.flush_stream_buffer(&node_id).await;
                        self.finish_aborted_run().await;
                        return Err("provider run aborted".to_string());
                    }
                    Some(command) => {
                        if session.commands.send(command).await.is_err() {
                            commands_open = false;
                        }
                    }
                    None => commands_open = false,
                }
            }
            event = session.events.recv() => {
                let Some(event) = event else {
                    let message = "provider stream ended before completion".to_string();
                    let _ = self.event_tx.send(EngineEvent::Error { message: message.clone() }).await;
                    self.finish_failed_run().await;
                    return Err(message);
                };

                match event {
                    ProviderEvent::TextDelta { content } => {
                        let _ = self.buffer_stream_chunk(&node_id, content.clone()).await;
                        let _ = self.event_tx.send(EngineEvent::StreamChunk {
                            role: "assistant".to_string(),
                            content,
                            node_id: Some(node_id.clone()),
                        }).await;
                    }
                    ProviderEvent::Execution(event) => {
                        self.emit_execution_event(event, Some(node_id.clone()), Some(agent.clone())).await;
                    }
                    ProviderEvent::PermissionRequest(request) => {
                        let _ = self.persist_permission_request(
                            &node_id,
                            request.id.clone(),
                            serde_json::json!({
                                "tool_name": request.tool_name.clone(),
                                "description": request.description.clone(),
                                "risk_level": risk_level_text(&request.risk_level),
                            }),
                        ).await;
                        let _ = self.event_tx.send(EngineEvent::PermissionRequest {
                            id: request.id,
                            tool_name: request.tool_name,
                            description: request.description,
                            risk_level: request.risk_level,
                        }).await;
                    }
                    ProviderEvent::ChoiceRequest(request) => {
                        let _ = self.event_tx.send(EngineEvent::ChoiceRequest {
                            id: request.id,
                            prompt: request.prompt,
                            options: request.options,
                            allow_multiple: request.allow_multiple,
                            allow_free_text: request.allow_free_text,
                            source: request.source,
                        }).await;
                    }
                    ProviderEvent::Completed { full_output, provider_session_id } => {
                        let _ = self.flush_stream_buffer(&node_id).await;
                        self.record_provider_session(
                            ProviderConversationRole::Author,
                            agent,
                            provider_session_id,
                            Some(node_id),
                        ).await;
                        return Ok(full_output);
                    }
                    ProviderEvent::Failed { message } | ProviderEvent::ProtocolError { message, .. } => {
                        let _ = self.flush_stream_buffer(&node_id).await;
                        let _ = self.event_tx.send(EngineEvent::Error { message: message.clone() }).await;
                        self.finish_failed_run().await;
                        return Err(message);
                    }
                    ProviderEvent::StatusChanged(status) => {
                        let _ = self.event_tx.send(EngineEvent::ProviderStatus { status }).await;
                    }
                    ProviderEvent::ToolCall(call) => {
                        self.emit_execution_event(
                            execution_event_from_tool_call(call),
                            Some(node_id.clone()),
                            Some(agent.clone()),
                        ).await;
                    }
                    ProviderEvent::ToolResult(result) => {
                        self.emit_execution_event(
                            execution_event_from_tool_result(result, "Tool result".to_string(), None),
                            Some(node_id.clone()),
                            Some(agent.clone()),
                        ).await;
                    }
                    ProviderEvent::PermissionTimeout { permission_id } => {
                        let message = format!("Permission request {permission_id} timed out");
                        let _ = self.event_tx.send(EngineEvent::Error { message: message.clone() }).await;
                        self.finish_failed_run().await;
                        return Err(message);
                    }
                }
            }
        }
    }
}
```

实现时可以复用/抽取 `drive_provider_session` 中已有私有 helper，避免重复代码；但行为必须保持：TextDelta 写 timeline detail + 发 `stream_chunk`，Completed 只返回 output。

- [ ] **Step 6: 运行测试**

Run: `cargo test --locked --lib begin_work_item_plan_author_run_creates_standard_author_node`

Expected: PASS。

- [ ] **Step 7: 提交**

```bash
git add src/product/workspace_engine.rs
git commit -m "feat: add work item plan streaming collector"
```

## Task 5: WorkItemPlanAuthor 改用 streaming provider

**Files:**
- Modify: `src/web/workspace_ws_handler.rs`

- [ ] **Step 1: 让 WorkItemPlan run 也获取 provider**

把 `provider_for_run` 的 special case 删除，使 WorkItemPlanAuthor / WorkItemPlanRevision 和 Story/Design 一样从 registry 取 streaming provider：

```rust
let provider_for_run = {
    let Some(p) = provider_registry.get(&provider_name) else {
        return Err(format!("provider unavailable: {provider_name:?}"));
    };
    Some(p)
};
```

- [ ] **Step 2: 替换 WorkItemPlanAuthor provider 调用**

在 `ProviderRunKind::WorkItemPlanAuthor` 分支中，用 `WorkItemSplitEngine::build_generate_invocation` 构建 prompt，然后创建 `author_run`，启动 streaming provider：

```rust
let invocation = match WorkItemSplitEngine::build_generate_invocation(
    &request,
    &lifecycle_for_run,
    &issue,
    &repository,
    author_provider.clone(),
) {
    Ok(invocation) => invocation,
    Err(error) => {
        engine.mark_active_run_finished(&run_label);
        drop(engine);
        let err = WsOutMessage::Error {
            message: format!("build split prompt failed: {}", error.message),
        };
        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
        return;
    }
};

let node_id = engine.begin_work_item_plan_author_run().await;
let provider_input = engine.build_work_item_plan_streaming_input(
    invocation.provider_type.clone(),
    invocation.prompt.clone(),
    invocation.worktree_path.clone(),
);
let provider = provider_for_run.expect("provider for work item plan author");
let session = provider.start(provider_input, run_cancel.clone()).await;
let full_output = match engine
    .drive_work_item_plan_provider_session_to_output(
        session,
        command_rx,
        node_id,
        invocation.author_provider.clone(),
    )
    .await
{
    Ok(output) => output,
    Err(message) => {
        engine.mark_active_run_finished(&run_label);
        drop(engine);
        let err = WsOutMessage::Error { message };
        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
        return;
    }
};
```

解析 structured output：

```rust
let structured = match parse_last_structured_output(&full_output) {
    Ok(Some(value)) => value,
    Ok(None) => {
        engine.mark_active_run_finished(&run_label);
        drop(engine);
        let err = WsOutMessage::Error {
            message: "split generate failed: missing structured output sentinel".to_string(),
        };
        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
        return;
    }
    Err(error) => {
        engine.mark_active_run_finished(&run_label);
        drop(engine);
        let err = WsOutMessage::Error {
            message: format!("split generate failed: {}", error.details),
        };
        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
        return;
    }
};
```

再进入现有 complete：

```rust
let output = match WorkItemSplitEngine::complete_generate_from_structured_output(
    &request,
    &lifecycle_for_run,
    &issue,
    &repository,
    &invocation.author_provider,
    &invocation.prompt,
    structured,
) {
    Ok(output) => output,
    Err(error) => {
        engine.mark_active_run_finished(&run_label);
        drop(engine);
        let err = WsOutMessage::Error {
            message: format!("split generate failed: {}", error.message),
        };
        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
        return;
    }
};

let mut outcome = match engine.complete_work_item_plan_author(output).await {
    Ok(outcome) => outcome,
    Err(message) => {
        engine.mark_active_run_finished(&run_label);
        drop(engine);
        let err = WsOutMessage::Error { message };
        let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
        return;
    }
};
```

- [ ] **Step 3: 保留 AutoRevision，但每轮也走 streaming**

把 AutoRevision loop 里的 `split_engine.generate_revision(...).await` 替换为：

1. `WorkItemSplitEngine::build_revision_invocation(...)`
2. `begin_work_item_plan_author_run()` 或复用当前 active `revision` 节点（用户触发 revision 时已有 `request_work_item_plan_revision` 创建）
3. `drive_work_item_plan_provider_session_to_output(...)`
4. `parse_last_structured_output(&full_output)`
5. `complete_revision_from_structured_output(...)`

AutoRevision 轮次的可读文案不再通过 `append_active_run_stream` 写入主气泡；如需提示，作为 `execution_event` 或 provider prompt detail 保存，避免污染 provider stream。

- [ ] **Step 4: 运行后端红灯测试**

Run: `cargo test --locked --test it_web work_item_plan_author_streams_provider_output_before_candidate_artifact`

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/web/workspace_ws_handler.rs
git commit -m "fix: stream work item plan author provider output"
```

## Task 6: WorkItemPlanRevision 走同一 streaming 链路

**Files:**
- Modify: `src/web/workspace_ws_handler.rs`
- Modify: `tests/it_web/web_work_item_plan_author.rs`

- [ ] **Step 1: 写 revision 流式测试**

在现有 `work_item_plan_revision_streams_progress_before_candidate_artifact` 基础上，改为断言 revision 节点接收 provider 正文流：

```rust
assert!(
    messages.iter().any(|m| {
        m["type"] == "timeline_node_created"
            && m["node"]["node_type"] == "revision"
            && m["node"]["agent"] == "fake"
    }),
    "expected revision node for work item plan revision"
);
assert!(
    messages.iter().any(|m| {
        m["type"] == "stream_chunk"
            && m["content"]
                .as_str()
                .unwrap_or("")
                .contains("Fake Work Item Plan streaming draft")
    }),
    "expected provider stream during work item plan revision"
);
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --locked --test it_web work_item_plan_revision_streams_progress_before_candidate_artifact`

Expected: FAIL，当前 revision 仍走阻塞式 split engine。

- [ ] **Step 3: 修改 WorkItemPlanRevision 分支**

和 Task 5 的 Author 分支保持同一套流程，但使用 `build_work_item_plan_revision_input` 产出的 `retained`、`redo_specs` 和 request：

```rust
let invocation = WorkItemSplitEngine::build_revision_invocation(
    &request,
    &lifecycle_for_run,
    &issue,
    &repository,
    author_provider.clone(),
    &retained,
    &redo_specs,
)?;
```

完成时调用：

```rust
let output = WorkItemSplitEngine::complete_revision_from_structured_output(
    &request,
    &lifecycle_for_run,
    &issue,
    &repository,
    &invocation.author_provider,
    &invocation.prompt,
    structured,
    &retained,
    &redo_specs,
)?;
let mut outcome = engine.complete_work_item_plan_revision(output).await?;
```

- [ ] **Step 4: 运行测试确认通过**

Run: `cargo test --locked --test it_web work_item_plan_revision_streams_progress_before_candidate_artifact`

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add src/web/workspace_ws_handler.rs tests/it_web/web_work_item_plan_author.rs
git commit -m "fix: stream work item plan revision provider output"
```

## Task 7: 前端气泡一致性与恢复测试

**Files:**
- Modify: `web/src/state/workspace-ws-store.test.ts`
- Modify: `web/src/hooks/useWorkspaceWs.test.tsx`
- Modify: `web/src/components/chat-workspace/message-grouping.test.ts`

- [ ] **Step 1: 更新 store 恢复测试**

把 `rebuilds work item plan author progress from timeline node streaming content` 的 node 改成 `author_run`：

```ts
node_type: "author_run",
agent: "claude_code",
stage: "running",
status: "active",
title: "Work Item Plan 生成",
```

断言保持：

```ts
expect(useWorkspaceStore.getState().chatEntries).toEqual([
  expect.objectContaining({
    type: "provider_stream",
    role: "author",
    content: "Fake Work Item Plan streaming draft",
    node_id: "timeline_node_work_item_plan_author",
    metadata: expect.objectContaining({ provider: "claude_code" }),
  }),
]);
```

- [ ] **Step 2: 增加 start_generation 不承载 provider stream 的测试**

```ts
it("does not render work item plan start_generation as provider stream", () => {
  const store = useWorkspaceStore.getState();
  store.setSessionState({
    session_id: "session_work_item_plan_start_anchor",
    workspace_type: "work_item_plan",
    stage: "running",
    messages: [],
    checkpoints: [],
    artifact: null,
    providers: { author: "claude_code", reviewer: "codex" },
    timeline_nodes: [{
      node_id: "timeline_node_start",
      node_type: "start_generation",
      agent: null,
      stage: "prepare_context",
      round: null,
      status: "completed",
      title: "开始生成",
      summary: null,
      started_at: "2026-06-20T10:00:00Z",
      completed_at: "2026-06-20T10:00:00Z",
      duration_ms: 0,
      artifact_ref: null,
      provider_config_snapshot: { author: "claude_code", reviewer: "codex", review_rounds: 1 },
    }],
    active_node_id: null,
    artifact_versions: [],
    timeline_node_details: {
      timeline_node_start: makeNodeDetail({
        node_id: "timeline_node_start",
        node_type: "start_generation",
        streaming_content: "",
      }),
    },
    active_run_id: null,
  });

  expect(useWorkspaceStore.getState().chatEntries).toEqual([
    expect.objectContaining({
      type: "start_generation",
      role: "system",
      content: "开始生成",
    }),
  ]);
});
```

- [ ] **Step 3: useWorkspaceWs 流式测试保持 author_run**

现有 `keeps work item plan stream chunks when active run arrives before provider stage` 已经使用 `author_run`。把内容从“正在生成 Work Item Plan”改为：

```ts
content: "Fake Work Item Plan streaming draft",
```

并断言：

```ts
expect(streamEntry).toMatchObject({
  type: "provider_stream",
  role: "author",
  content: "Fake Work Item Plan streaming draft",
  metadata: expect.objectContaining({ provider: "claude_code" }),
});
```

- [ ] **Step 4: 运行前端测试**

Run: `pnpm test --run src/state/workspace-ws-store.test.ts src/hooks/useWorkspaceWs.test.tsx src/components/chat-workspace/message-grouping.test.ts`

Workdir: `web`

Expected: PASS。

- [ ] **Step 5: 提交**

```bash
git add web/src/state/workspace-ws-store.test.ts web/src/hooks/useWorkspaceWs.test.tsx web/src/components/chat-workspace/message-grouping.test.ts
git commit -m "test: align work item plan chat stream entries"
```

## Task 8: 恢复一致性测试

**Files:**
- Modify: `tests/it_web/web_workspace_recovery_consistency.rs`

- [ ] **Step 1: 修改恢复断言**

把当前查找“包含 `正在生成 Work Item Plan` 的任意 detail”改成查找 `author_run` detail：

```rust
let author_node = timeline_nodes
    .iter()
    .find(|node| {
        node.node_type == TimelineNodeType::AuthorRun
            && node.title == "Work Item Plan 生成"
    })
    .expect("work_item_plan timeline should contain author_run node");

let author_detail = timeline_node_details
    .get(&author_node.node_id)
    .expect("author_run detail should be restored");

assert!(
    author_detail
        .streaming_content
        .contains("Fake Work Item Plan streaming draft"),
    "session_state should restore provider stream on author_run detail"
);
```

- [ ] **Step 2: 运行测试**

Run: `cargo test --locked --test it_web story_design_work_item_plan_recovery_consistency`

Expected: PASS。

- [ ] **Step 3: 提交**

```bash
git add tests/it_web/web_workspace_recovery_consistency.rs
git commit -m "test: recover work item plan author stream"
```

## Task 9: 验证与收口

**Files:**
- No code changes unless previous tasks expose formatting or import issues.

- [ ] **Step 1: 后端定向验证**

Run:

```bash
cargo test --locked --test it_web work_item_plan_author_streams_provider_output_before_candidate_artifact
cargo test --locked --test it_web work_item_plan_revision_streams_progress_before_candidate_artifact
cargo test --locked --test it_web story_design_work_item_plan_recovery_consistency
cargo test --locked --lib fake_streaming_provider_outputs_work_item_split_sentinel
```

Expected: all PASS。

- [ ] **Step 2: 前端定向验证**

Run in `web`:

```bash
pnpm test --run src/state/workspace-ws-store.test.ts src/hooks/useWorkspaceWs.test.tsx src/components/chat-workspace/message-grouping.test.ts
```

Expected: all PASS。

- [ ] **Step 3: 标准检查**

Run:

```bash
cargo fmt --check
cargo check --locked
```

Expected: PASS。

若改动触及共享 provider event loop 较多，再运行：

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: PASS。

- [ ] **Step 4: 手工验收**

1. 重启后端和前端服务。
2. 打开 `http://127.0.0.1:5173/workbench`。
3. 进入 Work Item Plan Workspace。
4. 点击“开始生成”。
5. 预期：先出现普通“开始生成”系统事件，随后出现和 Story/Design 一致的 `作者 · Claude Code` 气泡，并持续追加 provider 输出。
6. 预期：candidate 出现前，作者气泡里已经有真实 provider 文本。
7. 预期：刷新页面后，该作者气泡仍从 timeline detail 恢复。

- [ ] **Step 5: 最终提交**

```bash
git status --short
git add src/product/work_item_split_engine.rs src/product/workspace_engine.rs src/web/workspace_ws_handler.rs src/cross_cutting/streaming_provider.rs tests/it_web/web_work_item_plan_author.rs tests/it_web/web_workspace_recovery_consistency.rs web/src/state/workspace-ws-store.test.ts web/src/hooks/useWorkspaceWs.test.tsx web/src/components/chat-workspace/message-grouping.test.ts
git commit -m "fix: stream work item plan provider output"
```

## 自检

- Spec coverage:
  - Work Item Plan 启动气泡和 Story/Design 保持一致：Task 4、5、7。
  - Work Item Plan 生成期间持续流式输出：Task 1、3、4、5。
  - Revision / AutoRevision 不回退到阻塞式：Task 6。
  - 刷新恢复一致：Task 8。
  - Story/Design 共享链路不受影响：没有修改普通 `handle_user_message`、`drive_review_session` 的 completion 行为；如抽 helper，保留现有测试。
- Placeholder scan:
  - 无 TBD/TODO/“后续实现”占位。
- Type consistency:
  - `author_run`、`revision`、`provider_stream`、`ProviderEvent::TextDelta`、`parse_last_structured_output` 与现有类型一致。
- 风险:
  - Task 4 collector 可能和现有 `drive_provider_session` 有重复逻辑。实现时可以先保持局部重复以降低风险，等测试稳定后再抽共享私有 helper。
  - prompt 允许 sentinel 前可读输出后，真实 provider 仍可能只输出 JSON；这是模型行为，不影响后端流式链路。手工验收以“持续 chunk 可见”为准。
