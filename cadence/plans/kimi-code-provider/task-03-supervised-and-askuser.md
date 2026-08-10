# Task 3: Supervised 审批与 AskUserQuestion 提问（tasks 2.2, 2.3）

> 自包含任务文件。执行前请先读 `00-overview.md` 的 Global Constraints 与跨任务接口契约。依赖 Task 2 的 `run_kimi_session` 占位审批逻辑。

**Goal:** 实装 Task 2 中占位的 `session/request_permission` 往返：① Supervised 普通工具审批（PermissionRequest→ApprovalBridge，收窄二元 approved→allow_once/reject_once）；② AskUserQuestion 提问（request_permission title 区分 → ChoiceRequest，逐题串行，allow_multiple=false，自由文本 free_text 优先 → Cancelled + 下轮 prompt，对齐 Claude）。

**对应 spec requirement:**
- 「Kimi 支持逐工具审批（Supervised 模式）」
- 「Kimi 支持 AskUserQuestion 结构化提问」

**Files:**
- Modify: `src/cross_cutting/kimi_code_provider/session.rs`（替换 Task 2 占位的 request_permission 处理，实装两条路径）
- Modify: `src/cross_cutting/kimi_code_provider/parse.rs`（补 ChoiceRequest 构造辅助）
- Modify: `src/cross_cutting/kimi_code_provider/tests.rs`（审批/提问回归测试）
- Create: `src/cross_cutting/kimi_code_provider/tests/fixtures/askuser_multiquestion.jsonl`（多问题逐题场景）

**Interfaces:**
- Consumes（来自 Task 2）：`run_kimi_session`、`KimiPermissionRequest{request_id,tool_call_id,title,options,content_text}`、`ProviderEvent::PermissionRequest`/`ChoiceRequest`、`ProviderCommand::PermissionResponse{approved:bool}`/`ChoiceResponse{selected_option_ids,free_text}`、`ApprovalBridge`、`ChoiceRequestData{options,allow_free_text,allow_multiple,source}`、`ChoiceRequestSource::AskUserQuestion`。
- Produces: Kimi 完整的审批 + 提问往返能力。**Task 4-7 依赖（产品层会触发这些路径）。**

**参照：** `claude_code_provider/ask_user_question.rs:3-40`（ChoiceRequest 构造 + source=AskUserQuestion）、`codex_provider/parse.rs:229-300`（ChoiceRequest from request）、`claude_code_provider/ask_user_question.rs:75-97`（free_text 优先逻辑——本任务对齐）。

---

## Step 1: Supervised 工具审批往返（失败测试先行）

`tests.rs` 加：
```rust
#[tokio::test]
async fn supervised_tool_approval_approved_maps_to_allow_once() {
    // fixture: 普通工具(非AskUserQuestion)的 request_permission
    // 用户回 PermissionResponse{approved:true}
    // 断言适配器回 ACP result options:[{optionId: <allow_once 的 id>, outcome:"selected"}]
}
#[tokio::test]
async fn supervised_tool_approval_rejected_maps_to_reject_once_and_continues() {
    // approved:false → 回 reject_once optionId；工具不执行，会话继续
}
#[tokio::test]
async fn auto_mode_skips_permission_request_for_normal_tools() {
    // permissionMode=auto → 普通工具不产生 PermissionRequest（但 AskUserQuestion 仍会，见下）
}
```

- [ ] Run: `cargo test --locked --lib supervised_tool_approval` 与 `auto_mode_skips`
- Expected: FAIL

实现（`session.rs`，替换 Task 2 占位）：
- 收到 `session/request_permission` 且 `title != "AskUserQuestion"`：
  - 发 `ProviderEvent::PermissionRequest(PermissionRequestData{ tool_call_id, tool_name:title, input:content_text, ... })`。
  - 等 `command_rx` 的 `PermissionResponse{approved}`。
  - `approved==true` → 选 `kind=="allow_once"` 的 optionId，回 ACP result `{options:[{optionId, outcome:"selected"}]}`。
  - `approved==false` → 选 `kind=="reject_once"`（或 `"reject"`）的 optionId 回送。
  - 保留原 `request_id`（可 number/string）用于回包。
- Auto 模式：Kimi 对普通工具不发 request_permission（spike 实证），故此分支在 Auto 下天然不触发；无需额外代码。

- [ ] Run: `cargo test --locked --lib supervised_tool_approval` 与 `auto_mode_skips`
- Expected: PASS

## Step 2: AskUserQuestion → ChoiceRequest 映射（失败测试先行）

`tests.rs` 加：
```rust
#[tokio::test]
async fn askuserquestion_maps_to_choice_request() {
    // fixture: title=="AskUserQuestion" 的 request_permission
    // 断言发 ProviderEvent::ChoiceRequest，source==AskUserQuestion，
    // options 来自 request_permission.options，allow_free_text==true，allow_multiple==false
}
#[tokio::test]
async fn askuserquestion_select_option_returns_selected_and_continues() {
    // 用户回 ChoiceResponse{selected_option_ids:[oid], free_text:None}
    // 断言回 ACP Selected(oid)；会话同轮继续
}
```

- [ ] Run: `cargo test --locked --lib askuserquestion_maps` 与 `askuserquestion_select`
- Expected: FAIL

实现：
- 收到 `session/request_permission` 且 `title == "AskUserQuestion"`：
  - 构造 `ChoiceRequestData{ id, prompt:content_text, options:request_permission.options 转为 ChoiceOptionData, allow_multiple:false, allow_free_text:true, questions:vec![], source:ChoiceRequestSource::AskUserQuestion }`。
  - 发 `ProviderEvent::ChoiceRequest(...)`。
  - 等 `command_rx` 的 `ChoiceResponse{selected_option_ids, free_text}`。
  - **free_text 优先（对齐 Claude，B2）**：见 Step 3。
  - 否则（selected 非空、free_text 空）：取 `selected_option_ids[0]`，回 ACP `Selected(optionId)`，会话同轮继续。
  - 多问题（B3 spike 实证逐题串行）：本路径处理"当前这一题"，Kimi 答完后会自行发下一题的 request_permission，无需特殊处理 questions[]。

- [ ] Run: `cargo test --locked --lib askuserquestion_maps` 与 `askuserquestion_select`
- Expected: PASS

## Step 3: 自由文本 free_text 优先路径（失败测试先行，对齐 Claude）

`tests.rs` 加：
```rust
#[tokio::test]
async fn askuserquestion_free_text_takes_priority_over_selected() {
    // 用户回 ChoiceResponse{selected_option_ids:[oid], free_text:Some("自定义回答")}
    // 断言：忽略 selected（不拼接）；对原 request_permission 回 ACP Cancelled；
    //       不发中间 Failed/Completed；内部发第二个 session/prompt(注入 free_text)；
    //       第二轮 session/prompt result 为唯一终态
}
#[tokio::test]
async fn askuserquestion_free_text_only_no_selected() {
    // selected 空、free_text 有 → 同上 Cancelled+下轮 prompt
}
```

- [ ] Run: `cargo test --locked --lib askuserquestion_free_text`
- Expected: FAIL

实现（`session.rs`，AskUserQuestion 分支内）：
- `ChoiceResponse.free_text` 非空（trim 后）：
  1. 对原 `session/request_permission(request_id)` 回 ACP `Cancelled`（关闭原请求，避免 Kimi 挂起）。
  2. **忽略** `selected_option_ids`（不拼接，free_text 优先，对齐 Claude `ask_user_question_answers_from_decision`）。
  3. **不发** 中间 `Failed`/`Completed`。
  4. 内部发第二个 `session/prompt(sessionId, [free_text])` 注入用户文本（用新 JSON-RPC id）。
  5. 第二轮的 `session/prompt` result 为唯一终态（end_turn→Completed，其他→Failed）。
  6. 会话继续期间若有新的 request_permission（Kimi 追问），按 Step 1/2 正常处理。

- [ ] Run: `cargo test --locked --lib askuserquestion_free_text`
- Expected: PASS

## Step 4: 多问题逐题串行回归

`tests.rs` 加（用 `askuserquestion_multiquestion.jsonl` fixture，模拟 Kimi 连续发两题）：
```rust
#[tokio::test]
async fn multiquestion_serial_one_at_a_time() {
    // fixture: Q1 request_permission → 用户选 → Q2 request_permission → 用户选 → end_turn
    // 断言：两次独立 ChoiceRequest（每次单题），用户答完 Q1 后才出现 Q2
}
```

- [ ] Run: `cargo test --locked --lib multiquestion_serial`
- Expected: PASS（无需特殊代码，天然由串行 request_permission 驱动；测试保证回归）

## Step 5: 质量检查与提交

- [ ] Run: `cargo fmt --check` / `cargo clippy --all-targets --all-features --locked -- -D warnings` / `cargo test --locked`
- Expected: 全绿
- [ ] Commit:
```bash
git add -A
git commit -m "feat(kimi): Task 3 Supervised 审批(收窄二元 allow_once/reject_once) + AskUserQuestion 提问(逐题串行/allow_multiple=false) + 自由文本(free_text 优先/Cancelled+下轮 prompt/对齐 Claude)"
```
