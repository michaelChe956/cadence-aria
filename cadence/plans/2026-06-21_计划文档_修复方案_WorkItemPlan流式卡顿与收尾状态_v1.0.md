# WorkItemPlan 流式卡顿与收尾状态 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Work Item Plan 的流式气泡在 provider 运行、校验、自动返修和完成时都呈现清晰、稳定的状态，不再出现“已生成但气泡仍 active / 看起来卡住”的体验。

**Architecture:** 不伪造 provider 正文流，也不把系统状态混进 `<ARIA_STRUCTURED_OUTPUT>`。后端把每次 provider invocation 作为一个独立 timeline node：初次生成用 `author_run`，自动返修用 `revision`；每轮 provider 输出完成后先完成当前节点，再进入校验、下一轮自动返修或 AuthorConfirm。prompt 增加“长操作前输出简短状态”的约束，用来改善真实 provider 的文本节奏，但仍以 provider 实际 `TextDelta` 为准。

**Tech Stack:** Rust 1.95.0、Axum WebSocket、tokio mpsc、serde_json、React 19、Zustand、Vitest、pnpm、Cargo 宿主机命令。

---

## 当前证据

- 本地实测的 `workspace_session_0003` 已有新链路的 `author_run` 节点，说明 WorkItemPlan 已经走 streaming provider。
- 后端日志显示大量 `stream_chunk` 已经实时发出，前端 flush 仅 `80ms`，后端 timeline detail 写盘阈值是 `200ms / 4KB`；这些不是“卡几秒”的主因。
- Claude Code 在本轮生成中执行了多次 Bash 探索命令。provider 工具执行/思考期间可能没有 assistant text delta，因此会出现一段时间无正文、随后突然输出一段总结。
- 发现明确 Aria bug：`provider_run_split_0008` 已 completed，`issue_work_item_plan_0001` 已生成 8 个 work items，但 `timeline_node_002` 仍为 `author_run active`，`workspace_session_0003` 仍为 `running`。这是 WorkItemPlan 新 streaming collector 的收尾状态没有对齐 Story/Design。

## 文件结构

- Modify: `src/product/workspace_engine.rs`
  - 完成 WorkItemPlan provider 节点收尾。
  - 新增 WorkItemPlan AutoRevision 独立 revision 节点 helper。
  - 在 validator error / success / human confirm 三类 outcome 中写清楚 timeline summary。
- Modify: `src/web/workspace_ws_handler.rs`
  - WorkItemPlanAuthor / WorkItemPlanRevision 的 AutoRevision loop 不再复用旧 active node。
  - 每轮自动返修启动前创建新的 `revision` node。
- Modify: `src/product/work_item_split_engine.rs`
  - prompt 明确要求长分析/工具调用前先输出简短可读状态，缓解 provider 空档感。
- Modify: `tests/it_web/web_work_item_plan_author.rs`
  - 覆盖 author provider 完成后 `author_run` 被标记 completed。
  - 覆盖 validator error 触发 AutoRevision 时会创建独立 `revision` node 并流式输出。
- Modify: `tests/it_web/web_work_item_plan_revert.rs`
  - 如 revision 自动返修相关断言受节点数影响，更新为语义断言。
- Modify: `web/src/state/workspace-ws-store.test.ts`
  - 覆盖恢复时 completed `author_run` 不被当作仍在运行。
- Modify: `web/src/hooks/useWorkspaceWs.test.tsx`
  - 覆盖 WorkItemPlan AutoRevision 新 revision 节点的 stream chunk 正常进入 provider_stream。

## 设计决策

1. **不把 provider 卡顿伪装成流式正文。** 如果 Claude Code 没有 `TextDelta`，Aria 不应伪造 assistant 正文；只能展示 timeline/execution 状态，避免污染最终内容。
2. **每次 provider 调用一个 timeline node。** 初次生成、自动返修第 N 轮、用户触发 revision 都是不同节点，避免一个气泡长时间 active，让用户以为卡死。
3. **provider 完成不等于候选可确认。** provider 输出完成后节点可以 completed，后续校验失败则创建新的 AutoRevision revision 节点；校验成功再进入 AuthorConfirm。
4. **prompt 只改善节奏，不作为可靠机制。** 真实 provider 仍可能在工具调用期间不输出正文；可靠状态来自 Aria timeline node 和 execution_event。

## Task 1: 红灯测试覆盖 WorkItemPlan author_run 收尾

**Files:**
- Modify: `tests/it_web/web_work_item_plan_author.rs`

- [ ] **Step 1: 写失败测试**

在现有 author streaming 集成测试旁新增：

```rust
#[tokio::test]
async fn work_item_plan_author_completes_provider_node_before_author_confirm() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "依赖自检查拆分",
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
    let mut saw_author_completed = false;
    let mut saw_author_confirm = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);

    while tokio::time::Instant::now() < deadline {
        let remaining = deadline - tokio::time::Instant::now();
        let value = match timeout(remaining, ws.next()).await {
            Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Value>(&text).unwrap(),
            Ok(Some(Ok(Message::Close(_)))) => break,
            Ok(Some(Ok(_))) => continue,
            Ok(Some(Err(error))) => panic!("ws error: {error}"),
            Ok(None) | Err(_) => break,
        };

        match value["type"].as_str() {
            Some("timeline_node_created")
                if value["node"]["node_type"] == "author_run"
                    && value["node"]["title"] == "Work Item Plan 生成" =>
            {
                author_node_id = value["node"]["node_id"].as_str().map(str::to_string);
            }
            Some("timeline_node_updated")
                if author_node_id
                    .as_deref()
                    .is_some_and(|node_id| value["node_id"].as_str() == Some(node_id))
                    && value["status"] == "completed" =>
            {
                saw_author_completed = true;
            }
            Some("timeline_node_created") if value["node"]["node_type"] == "author_confirm" => {
                saw_author_confirm = true;
            }
            Some("error") => panic!("ws error message: {value}"),
            _ => {}
        }

        if saw_author_completed && saw_author_confirm {
            break;
        }
    }

    assert!(author_node_id.is_some(), "expected WorkItemPlan author_run node");
    assert!(saw_author_completed, "expected author_run to be completed");
    assert!(saw_author_confirm, "expected AuthorConfirm node after provider completion");
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --locked --test it_web work_item_plan_author_completes_provider_node_before_author_confirm`

Expected: FAIL。当前 WorkItemPlan 成功或校验后可能没有完成 `author_run`，尤其真实场景会残留 active。

## Task 2: 完成 WorkItemPlan provider 节点收尾

**Files:**
- Modify: `src/product/workspace_engine.rs`

- [ ] **Step 1: 新增 summary helper**

在 `impl WorkspaceEngine` 附近或私有 helper 区增加：

```rust
fn work_item_plan_findings_summary(prefix: &str, findings: &[WorkItemSplitFinding]) -> String {
    let errors = findings
        .iter()
        .filter(|finding| finding.severity == WorkItemSplitFindingSeverity::Error)
        .count();
    let warnings = findings
        .iter()
        .filter(|finding| finding.severity == WorkItemSplitFindingSeverity::Warning)
        .count();
    format!("{prefix}（errors: {errors}, warnings: {warnings}）")
}
```

- [ ] **Step 2: 在 author complete 的所有 outcome 完成 active node**

修改 `complete_work_item_plan_author`：

```rust
if report.has_errors() {
    self.work_item_plan_author_retry_count += 1;
    if self.work_item_plan_author_retry_count >= 3 {
        if let Err(error) = lifecycle.replace_issue_work_item_plan_candidate(
            &project_id,
            &issue_id,
            &plan_id,
            &output,
            findings.clone(),
        ) {
            tracing::warn!(%error, "persist final validate findings before HumanConfirm failed");
        }
        self.complete_active_node(Some(work_item_plan_findings_summary(
            "WorkItemPlan 校验失败，转人工确认",
            &findings,
        )))
        .await;
        self.enter_human_confirm_for_work_item_plan_author_failure(&findings)
            .await;
        return Ok(WorkItemPlanAuthorOutcome::HumanConfirm {
            reason: "validate 连续 3 次失败".to_string(),
        });
    }

    lifecycle
        .replace_issue_work_item_plan_candidate(
            &project_id,
            &issue_id,
            &plan_id,
            &output,
            findings.clone(),
        )
        .map_err(|e| format!("replace candidate failed: {e}"))?;
    self.complete_active_node(Some(work_item_plan_findings_summary(
        "WorkItemPlan 校验失败，准备自动返修",
        &findings,
    )))
    .await;
    return Ok(WorkItemPlanAuthorOutcome::AutoRevision { findings });
}
```

成功分支在 `enter_author_confirm` 前补：

```rust
self.complete_active_node(Some("WorkItemPlan provider 输出完成".to_string()))
    .await;
self.enter_author_confirm(Some("WorkItemPlan 候选已生成，等待确认".to_string()))
    .await;
```

- [ ] **Step 3: revision complete 同步处理**

在 `complete_work_item_plan_revision` 中对 `report.has_errors()` 和成功分支使用同样模式，summary 文案改为：

```rust
"WorkItemPlan 返修校验失败，准备自动返修"
"WorkItemPlan 返修校验失败，转人工确认"
"WorkItemPlan 返修 provider 输出完成"
```

- [ ] **Step 4: 运行红灯测试**

Run: `cargo test --locked --test it_web work_item_plan_author_completes_provider_node_before_author_confirm`

Expected: PASS。

## Task 3: AutoRevision 每轮使用独立 revision 节点

**Files:**
- Modify: `src/product/workspace_engine.rs`
- Modify: `src/web/workspace_ws_handler.rs`
- Modify: `tests/it_web/web_work_item_plan_author.rs`

- [ ] **Step 1: 写失败测试**

在 `tests/it_web/web_work_item_plan_author.rs` 中，更新现有 `work_item_plan_validate_errors_auto_revision_uses_generate_revision` 或新增断言：

```rust
assert!(
    messages.iter().any(|message| {
        message["type"] == "timeline_node_created"
            && message["node"]["node_type"] == "revision"
            && message["node"]["title"]
                .as_str()
                .unwrap_or("")
                .contains("Work Item Plan 自动返修")
    }),
    "expected AutoRevision to create a dedicated revision node"
);
assert!(
    messages.iter().any(|message| {
        message["type"] == "stream_chunk"
            && message["content"]
                .as_str()
                .unwrap_or("")
                .contains("Fake Work Item Plan streaming draft")
    }),
    "expected AutoRevision provider stream on a revision node"
);
```

- [ ] **Step 2: 新增 engine helper**

在 `impl WorkspaceEngine` 增加：

```rust
pub async fn begin_work_item_plan_auto_revision_run(&mut self, round: u32) -> String {
    self.transition_stage(WorkspaceStage::Revision).await;
    self.create_timeline_node(TimelineNodeDraft {
        node_type: TimelineNodeType::Revision,
        agent: Some(self.session.author_provider.clone()),
        stage: WorkspaceStage::Revision,
        round: Some(round),
        title: format!("Work Item Plan 自动返修 Round {round}"),
        summary: Some("根据 Work Item Plan 校验结果自动返修".to_string()),
        status: TimelineNodeStatus::Active,
    })
    .await
}
```

- [ ] **Step 3: 修改 WorkItemPlanAuthor AutoRevision loop**

在 `ProviderRunKind::WorkItemPlanAuthor` 的 `AutoRevision` 分支中，替换：

```rust
let Some(node_id) = engine.active_timeline_node_id() else { ... };
```

为：

```rust
let node_id = engine
    .begin_work_item_plan_auto_revision_run(revision_iterations)
    .await;
```

这样初次 `author_run` 已由 Task 2 完成，自动返修不会继续写进旧气泡。

- [ ] **Step 4: 修改 WorkItemPlanRevision AutoRevision loop**

在 `ProviderRunKind::WorkItemPlanRevision` 的 AutoRevision 分支中也替换为：

```rust
let node_id = engine
    .begin_work_item_plan_auto_revision_run(revision_iterations)
    .await;
```

用户主动发起的第一轮 revision 仍使用 `request_work_item_plan_revision` 创建的 active revision node；只有校验失败后的自动返修轮次创建新 revision node。

- [ ] **Step 5: 运行测试**

Run:

```bash
cargo test --locked --test it_web work_item_plan_validate_errors_auto_revision_uses_generate_revision
cargo test --locked --test it_web work_item_plan_author_completes_provider_node_before_author_confirm
```

Expected: PASS。

## Task 4: prompt 增加长操作前的可读状态约束

**Files:**
- Modify: `src/product/work_item_split_engine.rs`

- [ ] **Step 1: 写 prompt 单测**

在 `work_item_split_engine.rs` tests 中补充：

```rust
#[test]
fn split_prompt_requests_progress_before_long_operations() {
    let request = make_generate_request();
    let issue = make_issue();
    let repository = make_repository();

    let prompt = build_split_prompt(&request, &issue, &repository, &[], &[], "(empty)");

    assert!(prompt.contains("长时间分析、探索代码库或自动修正前"));
    assert!(prompt.contains("先输出一行简短可读状态"));
}
```

- [ ] **Step 2: 修改 generate / revision prompt**

在 `build_split_prompt` 和 `build_revision_prompt` 的 `[output_schema]` 或 `[workflow_discipline]` 附近加入：

```rust
"长时间分析、探索代码库或自动修正前，先输出一行简短可读状态，供 Workbench 流式展示；不要等待所有工具调用结束后才给第一段说明。\n\
如果需要执行多步代码库探索，每完成一组探索后输出一句当前发现摘要。\n\
这些可读状态必须位于最终 <ARIA_STRUCTURED_OUTPUT> 之前；最终结构化 JSON 仍只放在最后一个 sentinel block 中。\n\
"
```

- [ ] **Step 3: 运行 prompt 测试**

Run:

```bash
cargo test --locked --lib split_prompt_requests_progress_before_long_operations
cargo test --locked --lib build_split_prompt_allows_readable_stream_before_final_sentinel
```

Expected: PASS。

## Task 5: 前端恢复与状态一致性测试

**Files:**
- Modify: `web/src/state/workspace-ws-store.test.ts`
- Modify: `web/src/hooks/useWorkspaceWs.test.tsx`

- [ ] **Step 1: store 恢复测试覆盖 completed author_run**

新增测试：当 session_state 中 `author_run` 为 completed、`author_confirm` 为 active 时，chatEntries 应保留 provider stream，但 active stream 不应保持在 completed node。

```ts
expect(useWorkspaceStore.getState().chatEntries).toEqual(
  expect.arrayContaining([
    expect.objectContaining({
      type: "provider_stream",
      role: "author",
      node_id: "timeline_node_work_item_plan_author",
    }),
    expect.objectContaining({
      type: "stage_change",
      role: "system",
      node_id: "timeline_node_author_confirm",
    }),
  ]),
);
expect(useWorkspaceStore.getState().activeNodeId).toBe("timeline_node_author_confirm");
```

- [ ] **Step 2: hook 测试覆盖 AutoRevision revision stream**

新增或扩展测试：收到 `timeline_node_created` revision node 后，后续 `stream_chunk` 应进入该 revision node 的 provider_stream。

```ts
expect(streamEntry).toMatchObject({
  type: "provider_stream",
  role: "author",
  node_id: "timeline_node_work_item_plan_auto_revision_1",
  content: "Fake Work Item Plan streaming draft",
});
```

- [ ] **Step 3: 运行前端测试**

Run in `web`:

```bash
pnpm test --run src/state/workspace-ws-store.test.ts src/hooks/useWorkspaceWs.test.tsx
```

Expected: PASS。

## Task 6: 验证与手工观察

**Files:**
- No code changes unless previous tasks reveal compile errors.

- [ ] **Step 1: Rust 定向验证**

Run:

```bash
cargo test --locked --lib work_item_plan
cargo test --locked --lib work_item_split
cargo test --locked --test it_web work_item_plan
```

Expected: PASS。

- [ ] **Step 2: 前端定向验证**

Run in `web`:

```bash
pnpm test --run src/state/workspace-ws-store.test.ts src/hooks/useWorkspaceWs.test.tsx src/components/chat-workspace/message-grouping.test.ts
```

Expected: PASS。

- [ ] **Step 3: 标准检查**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
pnpm build
```

Expected: all PASS。`pnpm build` 如仅出现 Vite chunk size warning，不视为失败。

- [ ] **Step 4: 手工验收**

1. 重启服务。
2. 打开 `http://127.0.0.1:5173/workbench`。
3. 新建 Work Item Plan 并点击“开始生成”。
4. 预期：先出现“开始生成”系统事件，然后出现 `作者 · Claude Code` 的 `author_run` 气泡。
5. 预期：provider 工具调用期间允许短暂停顿，但 execution event 应显示正在探索/命令执行。
6. 预期：若 validator 失败，旧 `author_run` 先 completed，然后出现 `Work Item Plan 自动返修 Round N` revision 气泡。
7. 预期：provider run completed 且候选落盘后，不再残留 `author_run active`。
8. 预期：若校验连续失败进入人工确认，应进入 HumanConfirm，不再显示生成仍在 running。

## 自检

- Spec coverage:
  - WorkItemPlan 气泡残留 active：Task 1、2。
  - AutoRevision 看起来卡住：Task 3。
  - Provider 本身长空档：Task 4 用 prompt 改善，不伪造正文。
  - 前端恢复一致性：Task 5。
- Placeholder scan:
  - 未保留待补充占位。
- Type consistency:
  - 使用现有 `TimelineNodeType::AuthorRun`、`TimelineNodeType::Revision`、`WorkspaceStage::Revision`、`TimelineNodeStatus::Completed`。
  - 使用现有 `WorkItemSplitFindingSeverity::{Error, Warning}`。
  - 不新增 WebSocket message type，避免前后端协议扩散。
- 风险:
  - 真实 provider 仍可能在工具调用期间不输出正文，这是 provider 行为；本方案只确保 Aria 不额外造成 active 残留，并让 AutoRevision 状态清楚。
  - 每轮 AutoRevision 变成独立 revision node 后，部分测试中固定消息数可能需要从精确计数改为语义断言。
