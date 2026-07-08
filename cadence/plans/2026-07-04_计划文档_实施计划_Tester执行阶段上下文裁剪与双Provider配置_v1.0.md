# Tester执行阶段上下文裁剪与双Provider配置 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将 Coding Workspace Tester 的 `plan_tests` 与 `execute_test_plan` 上下文拆开，执行阶段默认只携带精简上下文，并允许按需懒加载 Story/Design 片段；同时支持分别选择 plan provider 与 execute provider，并重做 Coding Workspace 顶部辅助区布局以释放正式消息流空间。

**Architecture:** 现有 Tester 已经分为 plan/execution/report 三段且 Tester 不 resume provider session，本计划保留该结构，只替换 execution 阶段的 context 输入。新增 compact execution context DTO 与 builder，新增受限上下文加载工具，并把 Tester provider 配置从单一 `tester` 字段替换为 `tester_plan` / `tester_execute` 两个显式配置，不保留旧 `tester` 字段兼容。前端将 provider 设置和角色运行历史从主消息流上方移出，改成可折叠辅助抽屉/工具条，让 `ChatEntryList` 成为运行对话页的首要区域。

**Tech Stack:** Rust 2024、serde、tokio、现有 `CodingAttemptStore` / `CodingWorkspaceEngine` / `StreamingProviderAdapter`、Vitest/React 仅在需要前端选择器时触达。

---

## 当前基线

关键现状：

- `src/product/coding_workspace_engine/testing_provider.rs` 先调用 `run_provider_testing_plan_phase()`，再调用 `run_provider_testing_execution_phase()`。
- `src/product/coding_workspace_engine/testing_provider/plan.rs` 用 `build_evaluation_context_pack(..., EvaluationContextRole::Tester)` 生成完整 `evaluation_context_json`。
- `src/product/coding_workspace_engine/testing_provider.rs` 把同一份 `evaluation_context_json` 传给 execution phase。
- `src/product/coding_workspace_engine/prompts.rs::build_tester_execute_plan_prompt()` 把 `TestPlan` 和完整 `Evaluation Context JSON` 一起拼进 `execute_test_plan` prompt。
- `src/product/coding_workspace_engine/types.rs::should_resume_provider_conversation()` 当前只允许 `Coder` resume，所以 Tester 的 plan/execute 实际是两个 provider start，`resume_provider_session_id = None`。
- `web/src/pages/CodingWorkspacePage.tsx` 当前在 `ChatEntryList` 上方直接渲染 `CodingProviderConfigPanel` 与 `RoleRunHistoryPanel`，截图中这两块占据首屏高度，正式运行气泡被压到下半屏。
- `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx` 当前以 5 张角色卡横向铺开，每张卡内重复 provider 与 permission mode 按钮，信息密度高但层级不清。
- `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx` 当前以横向卡片展示 recent events、refs、reason code 等细节，运行历史一多就会继续挤压消息流。

本计划要解决的问题不是 session 隔离，而是 execution phase 复用了 plan phase 的完整 Evaluation Context。

## 文件结构

计划修改或新增：

- Modify: `src/product/coding_evaluation_context/mod.rs`
  - 新增 `EvaluationSourceArtifactRef`、`TesterExecutionContextPack`、`TesterExecutionWorkItemContext`。
- Create: `src/product/coding_evaluation_context/tester_execution.rs`
  - 从完整 attempt/work item/spec refs/repo context 构建 compact execution context。
- Modify: `src/product/coding_evaluation_context/builder.rs`
  - 导出或复用 repo/work item 查找能力；避免复制过多 store 查询。
- Modify: `src/product/coding_workspace_engine/prompts.rs`
  - `build_tester_execute_plan_prompt()` 改收 compact execution context JSON。
- Modify: `src/product/coding_workspace_engine/testing_provider/plan.rs`
  - plan phase 继续使用完整 context，但输出 phase 不再携带完整 `evaluation_context_json` 给 execution。
- Modify: `src/product/coding_workspace_engine/testing_provider/execution.rs`
  - execution phase 构建 compact context，支持 `load_test_context` 工具调用。
- Modify: `src/product/tester_agent_loop/tools.rs`
  - 新增只读工具名与输入解析：`load_test_context`。
- Create: `src/product/tester_agent_loop/context_loader.rs`
  - 根据 artifact refs + selectors 返回受限 Story/Design/WorkItem 片段。
- Modify: `src/product/coding_models/testing.rs`
  - 如需持久化审计，新增 `TestingContextRequest` 字段；优先保持兼容并用 `unplanned_evidence`/`context_warnings` 过渡。
- Modify: `src/product/coding_models/provider_config.rs`
  - 将 tester provider 配置从单一 `tester` 字段替换为 `tester_plan` / `tester_execute` 两个槽位。
- Modify: `src/product/coding_attempt_store/attempt.rs`
  - 更新 provider config snapshot 的读写结构；不添加旧 `tester` 字段 fallback。
- Modify: `web/src/api/types/coding.ts`
  - 补充 `tester_plan` / `tester_execute` 类型，删除旧 `tester` 类型依赖。
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`
  - 从 5 张大卡改为紧凑工具条 + 弹出配置面板；Tester 显示 plan/execute 两个 provider。
- Modify: `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx`
  - 从主消息流上方的横向卡片区改为可折叠运行历史抽屉或弹层摘要。
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
  - 调整运行对话页布局，让 `ChatEntryList` 占据主高度；provider 设置与运行历史默认收起。
- Modify: `web/src/pages/CodingWorkspaceControls.tsx`
  - 在运行对话 tab bar 增加辅助操作入口，例如 Provider、Runs 两个图标按钮。

测试重点：

- `src/product/coding_evaluation_context/tests.rs`
- `src/product/coding_workspace_engine/tests/provider_driven.rs`
- `tests/it_product/product_coding_workspace_engine/part_02.rs`
- `src/product/tester_agent_loop/tests.rs`
- 可能新增 `src/product/tester_agent_loop/context_loader.rs` 单元测试。
- `web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx`
- `web/src/pages/CodingWorkspacePage.reports.test.tsx`
- `web/src/pages/CodingWorkspacePage.gates.test.tsx`

## Task 1: 新增 compact execution context 模型

**Files:**
- Modify: `src/product/coding_evaluation_context/mod.rs`
- Create: `src/product/coding_evaluation_context/tester_execution.rs`
- Test: `src/product/coding_evaluation_context/tests.rs`

- [x] **Step 1: 写失败测试，确认 execution context 不含全文**

在 `src/product/coding_evaluation_context/tests.rs` 增加测试，构造含 Story/Design markdown 的 work item 与 attempt，调用新 builder 后断言：

```rust
#[test]
fn tester_execution_context_uses_refs_not_full_spec_markdown() {
    let pack = build_tester_execution_context_pack(paths.clone(), &attempt).expect("context");
    let json = serde_json::to_string_pretty(&pack).expect("json");

    assert!(json.contains("story_spec_0001"));
    assert!(json.contains("design_spec_0001"));
    assert!(json.contains("changed_files"));
    assert!(!json.contains("raw_markdown_or_sections"));
    assert!(!json.contains("完整 Story Spec 正文"));
    assert!(!json.contains("完整 Design Spec 正文"));
}
```

Run:

```bash
cargo test --locked --lib tester_execution_context_uses_refs_not_full_spec_markdown
```

Expected: FAIL，因为 builder 和类型尚不存在。

- [x] **Step 2: 定义 DTO**

在 `src/product/coding_evaluation_context/mod.rs` 增加：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationSourceArtifactRef {
    pub artifact_id: String,
    pub version_id: Option<String>,
    pub version: Option<u32>,
    pub title: String,
    pub workspace_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TesterExecutionWorkItemContext {
    pub artifact_id: String,
    pub title: String,
    pub repository_id: String,
    pub story_spec_ids: Vec<String>,
    pub design_spec_ids: Vec<String>,
    pub workspace_session_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TesterExecutionContextPack {
    pub issue_id: String,
    pub attempt_id: String,
    pub work_item: TesterExecutionWorkItemContext,
    pub source_artifacts: TesterExecutionSourceArtifacts,
    pub group_context: Option<CodingGroupContextPack>,
    pub repo_context: EvaluationRepoContext,
    pub context_warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TesterExecutionSourceArtifacts {
    pub story_specs: Vec<EvaluationSourceArtifactRef>,
    pub design_specs: Vec<EvaluationSourceArtifactRef>,
}
```

- [x] **Step 3: 实现 builder**

新增 `src/product/coding_evaluation_context/tester_execution.rs`，查询与 `build_evaluation_context_pack()` 相同的数据源，但只输出 refs 与 repo context。

核心规则：

- 不包含 `raw_markdown_or_sections`。
- 保留 `artifact_id/version_id/version/title/workspace_session_id`。
- 保留 `repo_context.changed_files/diff_stat/worktree_path`。
- 保留 `context_warnings`，但不因未读取全文引入 `context_truncated`。

- [x] **Step 4: 跑定向测试**

Run:

```bash
cargo test --locked --lib tester_execution_context_uses_refs_not_full_spec_markdown
```

Expected: PASS。

## Task 2: execution prompt 改用 compact context

**Files:**
- Modify: `src/product/coding_workspace_engine/prompts.rs`
- Modify: `src/product/coding_workspace_engine/testing_provider/plan.rs`
- Modify: `src/product/coding_workspace_engine/testing_provider/execution.rs`
- Modify: `src/product/coding_workspace_engine/testing_provider.rs`
- Test: `tests/it_product/product_coding_workspace_engine/part_02.rs`

- [x] **Step 1: 写失败测试，捕获 execute prompt 不含完整 Evaluation Context**

在 `tests/it_product/product_coding_workspace_engine/part_02.rs` 增加或扩展现有 prompt 捕获测试：

```rust
assert!(inputs[1].prompt.contains("Phase: execute_test_plan"));
assert!(inputs[1].prompt.contains("Execution Context JSON"));
assert!(inputs[1].prompt.contains("source_artifacts"));
assert!(!inputs[1].prompt.contains("Evaluation Context JSON"));
assert!(!inputs[1].prompt.contains("raw_markdown_or_sections"));
```

Run:

```bash
cargo test --locked --test it_product product_coding_workspace_engine::part_02::coding_tester_uses_separate_provider_sessions_for_plan_and_execute
```

若测试过滤路径不匹配，改用该文件现有测试名的精确过滤词。

- [x] **Step 2: 修改 phase 数据结构**

`ProviderTestingPlanPhase` 删除或不再向后暴露 `evaluation_context_json`：

```rust
pub(crate) struct ProviderTestingPlanPhase {
    pub(crate) tester_provider: ProviderName,
    pub(crate) plan: TestPlan,
    pub(crate) chat_entry_sequence: usize,
}
```

- [x] **Step 3: execution phase 内部构建 compact context**

在 `run_provider_testing_execution_phase()` 开头调用：

```rust
let execution_context = build_tester_execution_context_pack(self.store.paths(), &attempt)?;
let execution_context_json = serde_json::to_string_pretty(&execution_context)?;
let prompt = build_tester_execute_plan_prompt(&attempt, &plan, &execution_context_json);
```

- [x] **Step 4: 修改 prompt 标题**

`build_tester_execute_plan_prompt()` 中将：

```text
Evaluation Context JSON:
```

改成：

```text
Execution Context JSON:
```

并加入指令：

```text
If TestPlan is insufficient, do not redesign it inside execute_test_plan.
Use load_test_context only for targeted source artifact snippets.
If the plan is wrong or out of scope, mark the affected required step blocked.
```

- [x] **Step 5: 跑 prompt 回归测试**

Run:

```bash
cargo test --locked --test it_product coding_tester_uses_separate_provider_sessions_for_plan_and_execute
```

Expected: PASS。

## Task 3: 新增 load_test_context 受限懒加载工具

**Files:**
- Modify: `src/product/tester_agent_loop/tools.rs`
- Create: `src/product/tester_agent_loop/context_loader.rs`
- Modify: `src/product/coding_workspace_engine/testing_provider/execution.rs`
- Test: `src/product/tester_agent_loop/tests.rs`

- [x] **Step 1: 写 selector 解析测试**

新增测试覆盖输入：

```json
{
  "step_id": "step_t1",
  "reason": "Need exact DEC-006 package metadata",
  "artifact_refs": ["design_spec_0001@version_0002"],
  "selectors": ["DEC-006", "CMP-001"]
}
```

断言：

- `step_id` 非空。
- `reason` 非空。
- `selectors` 最多 8 个。
- 不支持 `mode: "full"`。

- [x] **Step 2: 实现工具输入 DTO**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LoadTestContextInput {
    pub step_id: String,
    pub reason: String,
    #[serde(default)]
    pub artifact_refs: Vec<String>,
    #[serde(default)]
    pub selectors: Vec<String>,
}
```

- [x] **Step 3: 实现 snippet loader**

`context_loader.rs` 根据 `artifact_refs` 找到版本 markdown，然后按 selector 匹配：

- `REQ-001` / `AC-001` / `DEC-001` / `CMP-001` / `NFR-001` 精确段落优先。
- 返回每个 selector 附近有限字符，例如 2,000 chars。
- 总返回不超过 8,000 chars。
- 找不到 selector 返回空 snippets 和 warning，不 panic。

- [x] **Step 4: 接入 execution tool call**

在 `execute_tester_tool_call` 分支或 execution phase 工具处理处支持 `load_test_context`：

- 它只返回 snippets。
- 它不调用 shell。
- 它不生成 `TestingStepResult`。
- 它记录为 `unplanned_evidence` 或新增 `context_requests`。

- [x] **Step 5: 更新 prompt 允许工具**

`tester_allowed_tools()` 增加：

```rust
["run_command", "read_file", "list_files", "search_code", "load_test_context"]
```

并在 execute prompt 中声明：

```text
load_test_context is read-only and cannot satisfy a required step by itself.
```

- [x] **Step 6: 跑定向测试**

Run:

```bash
cargo test --locked --lib load_test_context
```

Expected: PASS。

## Task 4: TestPlan 不足时 blocked，不自动重规划

**Files:**
- Modify: `src/product/coding_workspace_engine/testing_provider/execution.rs`
- Modify: `src/product/tester_agent_loop/report.rs`
- Test: `src/product/tester_agent_loop/tests.rs`

- [x] **Step 1: 写回归测试**

构造 provider 输出：

```json
{
  "step_results": [
    {
      "step_id": "step_missing_context",
      "status": "blocked",
      "evidence_refs": [],
      "provider_analysis": "test_plan_insufficient: selector DEC-014 unavailable"
    }
  ]
}
```

断言：

- `TestingReport.overall_status == Blocked`。
- `missing_required_steps` 为空，因为 step 有结果。
- `skipped_required_steps` 包含该 step 或新增 blocked reason 可见。
- 不触发 plan repair。

- [x] **Step 2: 明确 prompt 规则**

在 execute prompt 加：

```text
Do not generate new TestPlan steps during execute_test_plan.
If the current TestPlan is wrong, out of scope, or impossible to execute reliably, return blocked step_results for affected required steps with provider_analysis prefixed by "test_plan_insufficient:".
```

- [x] **Step 3: 跑测试**

Run:

```bash
cargo test --locked --lib test_plan_insufficient
```

Expected: PASS。

## Task 5: 支持 Tester plan provider 与 execute provider 分别选择

**Files:**
- Modify: `src/product/coding_models/provider_config.rs`
- Modify: `src/product/coding_attempt_store/attempt.rs`
- Modify: `src/product/coding_workspace_engine/testing_provider/plan.rs`
- Modify: `src/product/coding_workspace_engine/testing_provider/execution.rs`
- Test: `tests/it_product/product_coding_attempt_store/part_01.rs`
- Test: `tests/it_product/product_coding_workspace_engine/part_02.rs`

- [x] **Step 1: 写模型结构测试，确认只接受新字段**

新增或更新 provider config snapshot 测试，使用新 JSON：

```json
{
  "author": "claude_code",
  "reviewer": "codex",
  "tester_plan": "claude_code",
  "tester_execute": "codex"
}
```

断言：

```rust
assert_eq!(snapshot.tester_plan, ProviderName::ClaudeCode);
assert_eq!(snapshot.tester_execute, ProviderName::Codex);
```

同时增加旧 JSON 负例，明确不再保留旧 `tester` 字段兼容：

```rust
let old_json = r#"{
  "author": "claude_code",
  "reviewer": "codex",
  "tester": "claude_code"
}"#;
let result = serde_json::from_str::<CodingRoleProviderConfigSnapshot>(old_json);
assert!(result.is_err());
```

Run:

```bash
cargo test --locked --test it_product coding_role_provider_config_requires_tester_plan_and_execute
```

Expected: FAIL，因为模型仍是旧 `tester` 字段。

- [x] **Step 2: 扩展 config snapshot**

在 provider config 中删除：

```rust
pub tester: ProviderName,
```

加入：

```rust
pub tester_plan: ProviderName,
pub tester_execute: ProviderName,
```

更新 `provider_for_role()`：

```rust
pub fn tester_plan_provider(&self) -> &ProviderName {
    &self.tester_plan
}

pub fn tester_execute_provider(&self) -> &ProviderName {
    &self.tester_execute
}
```

`CodingProviderRole::Tester` 的既有 `provider_for_role()` 可临时返回 `tester_execute`，用于旧调用点在迁移期间仍能编译；所有 tester plan/execution 关键路径必须改用显式方法。

- [x] **Step 3: plan phase 使用 tester_plan**

`run_provider_testing_plan_phase()` 从 store snapshot 取 `tester_plan`，事件 phase 仍为 `plan_tests`。

- [x] **Step 4: execution phase 使用 tester_execute**

`run_provider_testing_execution_phase()` 使用 `tester_execute` 选择 provider 名称与 permission mode。

注意：如果当前函数只接收一个 `provider: &dyn StreamingProviderAdapter`，需要调整上层 provider factory 或调用层，让 plan/execute 分别拿到与 `tester_plan` / `tester_execute` 匹配的 adapter；不要只改 provider 名称不换实例。

- [x] **Step 5: session 隔离测试**

新增测试：

- plan provider = ClaudeCode。
- execute provider = Codex。
- 捕获两次 `StreamingProviderInput.provider_type` 不同。
- 两次 `resume_provider_session_id` 都为 `None`。

Run:

```bash
cargo test --locked --test it_product coding_tester_uses_distinct_plan_and_execute_providers
```

Expected: PASS。

## Task 6: 前端数据类型与双 Tester provider 选择

**Files:**
- Modify: `web/src/api/types/coding.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`
- Test: `web/src/hooks/useCodingWorkspaceWs.actions.test.tsx`
- Test: `web/src/pages/CodingWorkspacePage.gates.test.tsx`

- [x] **Step 1: 写类型与 store 更新测试**

当前前端类型中 `CodingRoleProviderConfigSnapshot` 只有 `tester`。先写失败测试，期望 snapshot 使用：

```ts
const snapshot: CodingRoleProviderConfigSnapshot = {
  coder: "codex",
  tester_plan: "claude_code",
  tester_execute: "codex",
  analyst: "codex",
  code_reviewer: "claude_code",
  internal_reviewer: "codex",
  review_rounds: 1,
  permission_modes: {
    coder: "supervised",
    tester: "auto",
    analyst: "supervised",
    code_reviewer: "supervised",
    internal_reviewer: "supervised",
  },
};
```

断言 store 收到 `role_provider_config_snapshot` 后可读取 `tester_plan` 与 `tester_execute`。

Run:

```bash
cd web && pnpm test -- useCodingWorkspaceWs
```

Expected: FAIL，因为类型和 store 仍使用 `tester`。

- [x] **Step 2: 更新前端类型**

`web/src/api/types/coding.ts` 中把：

```ts
tester: WorkspaceProviderName;
```

替换为：

```ts
tester_plan: WorkspaceProviderName;
tester_execute: WorkspaceProviderName;
```

- [x] **Step 3: 更新 WebSocket action**

`CodingProviderSelectRole` 增加：

```ts
export type CodingProviderSelectRole =
  | "author"
  | "reviewer"
  | CodingProviderRole
  | "tester_plan"
  | "tester_execute";
```

`sendProviderSelect()` 允许发送 `"tester_plan"` / `"tester_execute"`；后端 handler 需要在 Task 5 同步支持这两个 role key。

- [x] **Step 4: UI 增加两个 Tester provider 控制**

展示文案：

- `Tester Plan Provider`
- `Tester Execute Provider`

默认值：

- 用户可分别选择 ClaudeCode / Codex / Fake。
- Tester permission mode 仍保留一个 `tester` 授权模式，不拆成两个，避免让配置面板继续膨胀。

- [x] **Step 5: 前端测试**

断言提交 payload 同时包含：

```json
{
  "tester_plan": "claude_code",
  "tester_execute": "codex"
}
```

Run:

```bash
cd web && pnpm test
```

Expected: PASS。

## Task 7: Coding Workspace 页面 UI/UX 重排

**Files:**
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspaceControls.tsx`
- Modify: `web/src/components/coding-workspace/CodingProviderConfigPanel.tsx`
- Modify: `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx`
- Modify: `web/src/components/chat-workspace/ChatEntryList.tsx`
- Test: `web/src/components/coding-workspace/RoleRunHistoryPanel.test.tsx`
- Test: `web/src/pages/CodingWorkspacePage.reports.test.tsx`
- Test: `web/src/pages/CodingWorkspacePage.gates.test.tsx`

- [ ] **Step 1: 写布局回归测试，确认运行消息优先**

在 `CodingWorkspacePage.reports.test.tsx` 增加测试：

```tsx
it("keeps provider config and role history collapsed so chat remains primary", () => {
  render(<CodingWorkspacePage attemptId="coding_attempt_0001" onBack={vi.fn()} />);

  expect(screen.getByTestId("coding-chat-entry-list")).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Provider 设置" })).toBeInTheDocument();
  expect(screen.getByRole("button", { name: "角色运行历史" })).toBeInTheDocument();
  expect(screen.queryByTestId("coding-provider-config-panel")).not.toBeInTheDocument();
  expect(screen.queryByTestId("coding-role-run-history")).not.toBeInTheDocument();
});
```

如果 `ChatEntryList` 当前没有 test id，先在该组件根节点增加 `data-testid="coding-chat-entry-list"`。

Run:

```bash
cd web && pnpm test -- CodingWorkspacePage.reports
```

Expected: FAIL，因为当前 provider config 和 role run history 默认展开在消息流上方。

- [ ] **Step 2: 改运行对话页布局**

把 `CodingWorkspacePage.tsx` 中运行对话区域从：

```tsx
<CodingProviderConfigPanel ... />
<RoleRunHistoryPanel ... />
<ChatEntryList ... />
```

改成：

```tsx
<CodingWorkspaceToolbar
  providerSummary={store.roleProviderConfigSnapshot}
  roleRunCount={store.roleRuns.length}
  activeDrawer={activeDrawer}
  onOpenDrawer={setActiveDrawer}
/>
<ChatEntryList ... />
{activeDrawer === "providers" ? <CodingProviderConfigPanel ... variant="drawer" /> : null}
{activeDrawer === "runs" ? <RoleRunHistoryPanel ... variant="drawer" /> : null}
```

布局规则：

- `ChatEntryList` 必须位于主 grid 的 `minmax(0,1fr)` 行。
- Provider 和 Runs 面板默认不占主消息流高度。
- Drawer 使用右侧或顶部浮层，最大宽度 32rem，最大高度不超过视口可用高度。
- 在 1024px 以下宽度，drawer 变为底部 sheet，高度不超过 45vh。

- [ ] **Step 3: 重做 Provider 设置控件**

`CodingProviderConfigPanel` 从 5 张卡改为表格/列表式密集配置：

```text
Role                  Provider              Mode
Coder                 Codex                 Supervised
Tester Plan           Claude Code           -
Tester Execute        Codex                 Auto
Analyst               Codex                 Supervised
Code Reviewer         Claude Code           Supervised
Internal Reviewer     Codex                 Supervised
```

交互规则：

- Provider 用分段按钮或紧凑 select，不再每个角色显示三行按钮。
- Tester Plan 不显示 permission mode。
- Tester Execute 复用 Tester permission mode。
- 锁定状态显示 lock 图标和禁用态，不用整张卡变灰。
- 行高控制在 36-44px，便于扫描。

- [ ] **Step 4: 重做角色运行历史**

`RoleRunHistoryPanel` 默认只展示摘要列表：

```text
Tester #1    running    640 events    abort_attempt
Code Reviewer #1 completed 38 events  final_review
```

点击某一行后，在面板内部展开 recent events/refs，不在默认列表中直接展示全部细节。

验收：

- 默认历史列表每行高度不超过 44px。
- recent events 默认最多展示 1 行摘要，展开后最多展示 5 条。
- refs 默认只显示数量，例如 `3 refs`，展开后显示具体 ref。

- [ ] **Step 5: 更新页面测试**

更新 `CodingWorkspacePage.gates.test.tsx` 中 provider 选择相关测试：

- 点击 `Provider 设置` 后才能看到 provider 控件。
- 修改 Tester Plan provider 调用 `sendProviderSelect("tester_plan", "...")`。
- 修改 Tester Execute provider 调用 `sendProviderSelect("tester_execute", "...")`。

更新 `RoleRunHistoryPanel.test.tsx`：

- 默认只显示摘要，不显示 recent event 详细行。
- 点击 run row 后显示最近事件和 refs。

Run:

```bash
cd web && pnpm test -- CodingWorkspacePage.gates
cd web && pnpm test -- RoleRunHistoryPanel
```

Expected: PASS。

- [ ] **Step 6: 视觉验收**

用截图中的场景验收：

- 运行对话页默认状态下，`CodingPanelTabs` 下方除一条高度不超过 44px 的 toolbar 外，不得出现常驻辅助面板。
- `ChatEntryList` 必须占据主内容区 `minmax(0, 1fr)`，作为运行对话 tab 的首要可见区域。
- Provider/Runs 面板只能以 overlay drawer/sheet 出现，不得挤占或缩短 `ChatEntryList` 布局高度。
- 打开或关闭 Provider/Runs drawer 不得改变 `ChatEntryList` 的滚动位置。
- 首屏运行对话 tab 下方直接可见最新正式气泡消息。
- Provider 设置默认收起，只在 toolbar 显示当前 provider 摘要，例如 `Coder codex · Tester claude/codex`。
- 角色运行历史默认收起，只显示入口和运行数量，例如 `Runs 1 running`。
- 打开 Provider/Runs 面板不会改变消息流滚动位置。
- 文本不重叠，按钮文字不溢出，375px / 768px / 1440px 宽度无横向滚动。

## Task 8: 集成验证与全量回归

**Files:**
- No code unless previous tasks expose failures.

- [ ] **Step 1: Rust 格式检查**

Run:

```bash
cargo fmt --check
```

Expected: PASS。

- [ ] **Step 2: Rust 编译检查**

Run:

```bash
cargo check --locked
```

Expected: PASS。

- [ ] **Step 3: Rust clippy**

Run:

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: PASS。

- [ ] **Step 4: Rust 测试**

Run:

```bash
cargo test --locked
```

Expected: PASS。禁止添加 `-j 1`。

- [ ] **Step 5: 前端类型与测试**

Run:

```bash
cd web && pnpm tsc -b
cd web && pnpm test
```

Expected: PASS。

- [ ] **Step 6: 人工样例验证**

用 `coding_attempt_0001` 同类样例观察：

- `plan_tests` prompt 仍包含完整 Evaluation Context。
- `execute_test_plan` prompt 不包含完整 Story/Design markdown。
- `execute_test_plan` prompt 包含 `Execution Context JSON`、`source_artifacts`、`changed_files`。
- provider 选择中可分别指定 plan provider 与 execute provider。
- 如 executor 调 `load_test_context`，日志中可看到 selector、reason、返回 snippet。
- Coding Workspace 运行对话页首屏不再被 provider 设置和角色运行历史挤压；消息气泡区域是主要可见区域。
- Provider 设置和角色运行历史通过 toolbar/drawer 打开，默认不占用消息流高度。

## 风险与边界

- `load_test_context` 不能变成全文旁路。必须限制 selector 数量、单片段长度和总长度。
- `load_test_context` 不能满足 required step，只能辅助解释 TestPlan。
- 如果 TestPlan 本身错了，execute 阶段必须 blocked，不自动重跑 plan_tests。
- 双 provider 配置不保留旧 `tester` 字段兼容；实施后测试数据、seed、前端类型、后端 DTO 必须同步改成 `tester_plan` / `tester_execute`。
- 如果当前上层只传入一个 `StreamingProviderAdapter`，双 provider 需要先改 provider factory 调度层，否则只是字段可选但运行时仍用同一个实例。
- Coding Workspace UI 改造不能把 provider 设置和角色运行历史藏到不可发现的位置；toolbar 入口必须始终可见，并显示当前摘要。
- Drawer/sheet 打开和关闭不能重置 `ChatEntryList` 滚动位置，否则会破坏人工 E2E 观察体验。

## 自检清单

- [ ] Spec coverage: 上下文裁剪、懒加载、blocked 分流、双 provider 选择、Coding Workspace UI 拥挤治理均有任务覆盖。
- [ ] Placeholder scan: 本计划不包含未定占位项。
- [ ] Type consistency: `tester_plan` / `tester_execute` 在后端和前端命名一致。
- [ ] Validation: 所有 Rust 命令遵守项目规则，不使用 Docker，不使用 `-j 1`。
