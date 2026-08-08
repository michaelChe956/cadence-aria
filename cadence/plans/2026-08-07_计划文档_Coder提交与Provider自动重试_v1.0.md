# Coder 提交与 Provider 自动重试 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development`（推荐）or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Coder 依 Work Item 的 `write_policy` 自主精确提交，并让 Coder 与独立 Code Reviewer 的瞬时技术失败在每个用户授权周期内最多自动恢复两次，同时留下可审计、可展示的每次调用记录。

**Architecture:** 将组内 Work Item 完成时的 Git 写操作替换成对 `UnitRun.start_commit` 与当前 `HEAD` 的只读观察，并让所有范围、diff 与完成门禁消费该完整区间。将 Provider 流的失败先归类为 typed outcome，再交给位于 Coder/Reviewer 调用外层的协调器决定重试、创建新 role run/timeline attempt，或在预算耗尽后调用既有人工门禁；业务结论继续流向既有 Coder rework 和 Plan Repair。

**Tech Stack:** Rust 2024、Tokio、Serde、现有 JSON `CodingAttemptStore`、Git CLI service、Axum WebSocket/REST、React、TypeScript、Zustand、Vitest。

## Global Constraints

- OpenSpec 权威来源：`openspec/changes/coder-owned-commits-and-provider-retries/`；本计划逐项覆盖 `tasks.md` 的 1.1–3.3，且不得改变其范围或验收标准。
- 不得根据 `node_modules`、`.aria`、缓存目录或任意静态路径名单决定 Coder 能否提交；是否可提交只由当前 Work Item 的规范性 `write_policy` 决定。
- Aria 在 Group Work Item completion 不得执行 `git add`、`git commit`、广泛删除或 Coder 漏提交时的补提交；保留的 Git 服务写操作不得成为该流程的后备路径。
- `start_commit == completion_commit` 必须作为空观察区间；绝不可把起始提交相对父提交的文件、diff 或提交归属给当前 Work Item。
- 每个自动失败尝试必须保留 `Failed` role run、独立 raw output、独立 timeline attempt；后续尝试不能将其改写成 `Superseded`。
- 每个 Coder/Code Reviewer 调用周期最多三次调用（首次 + 两次自动重试）。用户主动取消、权限/选择等待及其等待超时、结构化输出无效、正常审查 finding、验证失败和 Plan Defect 均不消耗也不触发自动重试。
- Coder 自动重试在同一 worktree 使用完整新上下文；Reviewer 自动重试使用完整新的只读审查上下文。Codex resume-stall 的 fresh session 也必须占用同一预算并成为独立 role run。
- 自动重试不增加 `rework_count`，不创建 rework instruction，不解释 reviewer verdict，不启动 Plan Repair；正常业务输出之后才沿用既有 rework/Plan Repair 分流。
- 历史 JSON 必须保持可读：新增的 role-run 字段使用 `#[serde(default)]`；不对历史 attempt 或 Git 历史做迁移或重写。
- 先写失败测试，再写最小实现；Rust 定向快反馈统一使用 `cargo test --locked --lib <过滤名>`，绝不传 `-j 1`；前端只使用 `pnpm`。
- 本计划完成并通过验证后，才可开始依赖它的 `2026-08-07_计划文档_人工组级最终确认_v1.0.md`。

---

## 文件结构与接口边界

| 区域 | 文件 | 责任 |
| --- | --- | --- |
| Git 事实 | `src/product/git_workspace_service.rs`、`src/product/git_workspace_service/tests.rs` | 新增只读的提交区间文件/提交列表查询；空区间返回空集合。 |
| 提交完成与范围门禁 | `src/product/coding_workspace_engine/group_completion.rs`、`gates.rs`、`gates/schema_v2.rs` | 记录 Coder 产生的 HEAD；将 `CodingUnitRun` 带入门禁事实并使用完整区间。 |
| Coder 上下文 | `src/product/work_item_projection/render.rs`、`src/product/coding_workspace_engine/plan_defect.rs` 与其测试 | 在完整 Coder prompt 中明确精确暂存、提交、报告和不可清理责任。 |
| 重试持久化 | `src/product/coding_models/role_run.rs`、`src/product/coding_attempt_store/role_run.rs` | 保存 cycle、周期序号、触发来源、前序 run 与失败原因；提供不 supersede 前序失败记录的创建 API。 |
| 重试协调 | 新建 `src/product/coding_workspace_engine/provider_retry.rs`，并修改 `mod.rs`、`types.rs`、`provider_stream.rs`、`coding.rs`、`rework.rs`、`code_review.rs` | 归类技术失败、运行三次上限、为每次调用建立 role run/timeline、移除隐藏的 inline fresh retry。 |
| 人工恢复与协议 | `gates.rs`、`failed_review_recovery.rs`、WebSocket handler 相关测试 | 将 Coder/Reviewer 人工 retry 建成新的可追溯调用周期，继续复用既有人工操作入口。 |
| Web 前端 | `src/web/coding_ws_handler/{protocol.rs,state.rs}`、`src/web/{types.rs,handlers/coding.rs}`、`web/src/api/types/coding.ts`、`web/src/state/coding-workspace-store.ts`、`web/src/components/coding-workspace/RoleRunHistoryPanel.tsx` | 传输并显示调用周期、尝试序号、失败原因和“自动重试中/已耗尽等待人工”状态。 |

以下接口名称是本计划中跨任务的固定协作边界；实施时不得用隐式计数器或单一 role run 代替：

```rust
pub(crate) enum CodingProviderRetryTrigger {
    Initial,
    AutomaticRetry,
    ManualRetry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CodingRoleRunRetryMetadata {
    pub cycle_id: String,
    pub attempt_no: u8, // 1..=3
    pub prior_run_id: Option<String>,
}

pub(crate) async fn git_commit_range_changed_files(
    &self,
    worktree_path: &Path,
    start_commit: &str,
    completion_commit: &str,
) -> Result<Vec<String>, GitWorkspaceError>;
```

`CodingRoleRunTrigger` 可保留既有枚举值以兼容历史数据，但新调用必须通过能表达上面三种语义的持久化字段。若实施时合并枚举命名，序列化值与前端类型必须同步且历史值必须继续反序列化。

## 任务清单

### Task 1: 提交区间 Git 事实与空区间回归

**对应契约：** `coder-owned-work-item-commit` 的“完整提交区间”和“未观察到新的 Coder 提交”场景；`coding-workspace-completion` 的三个场景。

**Files:**

- Modify: `src/product/git_workspace_service.rs:349-378`
- Modify: `src/product/git_workspace_service/tests.rs`
- Test: `src/product/coding_workspace_engine/tests/group_completion_authority.rs`

**Consumes:** Git repository fixture、`start_commit` 和 `completion_commit`。

**Produces:** `git_commit_range_changed_files(start, completion)` 与 `git_commit_range_commits(start, completion)`；二者在相同 SHA 时均返回空集合。

- [ ] **Step 1: 写失败的 Git service 单元测试**

  在临时仓库建立基线 `C0`、首次 Coder 提交 `C1`、rework 提交 `C2`，断言范围查询按提交拓扑顺序返回 `C1`、`C2`，文件集合包含两次改动；另建 `C0..C0` 断言既无文件又无提交。测试名称固定为：

  ```rust
  #[tokio::test]
  async fn commit_range_includes_initial_and_rework_commits() { /* C0 -> C1 -> C2 */ }

  #[tokio::test]
  async fn equal_commit_range_is_an_empty_observation() { /* C0 -> C0 */ }
  ```

- [ ] **Step 2: 运行失败测试并记录缺少的区间 API**

  Run: `cargo test --locked --lib commit_range_includes_initial_and_rework_commits`

  Expected: FAIL，因为 `git_commit_range_changed_files` / `git_commit_range_commits` 尚不存在。

- [ ] **Step 3: 实现只读区间查询**

  在 `GitWorkspaceService` 中用 `git diff --name-only --no-renames start..completion` 获取去重后的路径，用 `git rev-list --reverse start..completion` 获取有序 SHA。首先比较两个 SHA；相等时直接返回 `Vec::new()`，不得回退到 `git show <start>`。保留现有 `git_commit_changed_files` 供非本计划路径使用，不改变其单提交语义。

  ```rust
  if start_commit == completion_commit {
      return Ok(Vec::new());
  }
  let output = self.run_git(
      worktree_path,
      &["diff", "--name-only", "--no-renames", &format!("{start_commit}..{completion_commit}")],
  ).await?;
  ```

- [ ] **Step 4: 运行 Git service 与既有组完成测试**

  Run: `cargo test --locked --lib commit_range_`

  Run: `cargo test --locked --lib group_completion_authority`

  Expected: PASS；既有单提交测试仍可运行，新范围测试证明 C1/C2 都可见且空观察没有父提交污染。

- [ ] **Step 5: 提交原子变更**

  ```bash
  git add src/product/git_workspace_service.rs src/product/git_workspace_service/tests.rs src/product/coding_workspace_engine/tests/group_completion_authority.rs
  git commit -m "feat: add work item commit range facts"
  ```

### Task 2: 将提交职责与完成门禁切换到 Coder 的完整区间

**对应契约：** `tasks.md` 1.1、1.2、1.3；不得新增服务端 staging/path 门禁。

**Files:**

- Modify: `src/product/work_item_projection/render.rs:221-375`
- Modify: `src/product/coding_workspace_engine/plan_defect.rs`
- Modify: `src/product/coding_workspace_engine/group_completion.rs:42-92,419-490`
- Modify: `src/product/coding_workspace_engine/gates.rs:301-380`
- Modify: `src/product/coding_workspace_engine/gates/schema_v2.rs:7-138`
- Test: `src/product/coding_workspace_engine/tests/group_completion_authority.rs`
- Test: `src/product/coding_workspace_engine/tests/runtime_handoff_group_completion.rs`
- Test: `src/product/coding_workspace_engine/tests/schema_v2_runtime.rs`

**Consumes:** Task 1 的区间 Git API、`CodingUnitRun.start_commit`、Coder projection 的 `write_policy`。

**Produces:** `completion_commit` 只读记录当前 HEAD；`SchemaV2GroupCompletionGateFacts` 含完成的 `CodingUnitRun`，范围校验从 `run.start_commit..run.completion_commit` 派生。

- [ ] **Step 1: 写失败的 completion 与 prompt 测试**

  增加以下最小回归：

  ```rust
  #[tokio::test]
  async fn group_completion_records_existing_coder_head_without_staging_or_commit() { /* HEAD = C1 */ }

  #[tokio::test]
  async fn group_scope_gate_validates_full_unit_run_range_after_rework() { /* C0..C2 */ }

  #[tokio::test]
  async fn group_scope_gate_treats_equal_start_and_completion_as_empty() { /* C0..C0 */ }
  ```

  在 Coder renderer 的快照/字符串测试中断言文本同时包含：检查完整 Git 状态、按当前 `write_policy` 精确暂存、创建 commit、报告 SHA/暂存清单/提交后状态；断言不包含目录名黑名单语义，并要求范围外未知内容“保留并报告”。

- [ ] **Step 2: 运行失败测试**

  Run: `cargo test --locked --lib group_completion_records_existing_coder_head_without_staging_or_commit`

  Run: `cargo test --locked --lib group_scope_gate_validates_full_unit_run_range_after_rework`

  Expected: FAIL；当前路径仍调用 `git_add_work_item_changes`、`git_commit`，门禁仍只读取尾部 commit。

- [ ] **Step 3: 以 Coder 的 HEAD 替换编排提交，并更新 Coder 上下文**

  将 `commit_current_group_unit_changes` 重命名为只读意图的函数，例如 `record_current_group_unit_completion_head`；它只调用 `git_current_head` 和现有 `persist_group_unit_completion_commit`。删除该函数中对 `git_add_work_item_changes`、`git_has_staged_changes`、`git_commit` 的调用，不删除或清理 worktree 中的任何路径。

  在 `coder_sections` 以及 Plan Defect/rework 会使用的完整 Coder 上下文中加入以下不可省略内容：

  ```text
  提交责任：先检查完整 Git 状态；仅根据本 Work Item 的 write_policy 精确暂存允许路径；创建本 Work Item 的提交。
  报告：列出暂存文件、提交 SHA、提交后的 Git 状态。
  禁止：不得使用无差别全量暂存；不得删除、清理或提交无法由当前 write_policy 解释的内容。遇到它们时保留并报告。
  ```

- [ ] **Step 4: 将门禁事实改为 `CodingUnitRun` 区间**

  给 `SchemaV2GroupCompletionGateFacts` 加入 `run: CodingUnitRun`。将 `changed_files_for_unit_completion_commit` 改为接收完整 `&CodingUnitRun`，要求 `status == Completed` 且存在 `completion_commit`；调用 Task 1 的 range helper。schema-v2 和 legacy 分支均须使用 unit run 的起止提交；遇到缺失 run/commit 继续失败关闭。

  ```rust
  let changed_files = self
      .changed_files_for_unit_completion_range(attempt, &facts.run)
      .await?;
  self.validate_changed_files_for_runtime(&facts.runtime, &changed_files, worktree_path.as_ref())?;
  ```

- [ ] **Step 5: 运行完成、范围、handoff 与 renderer 测试**

  Run: `cargo test --locked --lib group_completion_authority`

  Run: `cargo test --locked --lib runtime_handoff_group_completion`

  Run: `cargo test --locked --lib schema_v2_runtime`

  Expected: PASS；策略允许的生成目录不会因目录名受阻，范围外残留既不被提交也不被删除，C1+C2 都进入证据，空范围不会误报 C0 的父提交差异。

- [ ] **Step 6: 提交原子变更**

  ```bash
  git add src/product/work_item_projection/render.rs src/product/coding_workspace_engine/plan_defect.rs src/product/coding_workspace_engine/group_completion.rs src/product/coding_workspace_engine/gates.rs src/product/coding_workspace_engine/gates/schema_v2.rs src/product/coding_workspace_engine/tests/group_completion_authority.rs src/product/coding_workspace_engine/tests/runtime_handoff_group_completion.rs src/product/coding_workspace_engine/tests/schema_v2_runtime.rs
  git commit -m "feat: let coder own group work item commits"
  ```

### Task 3: 为独立 Provider 调用持久化 retry-cycle 身份

**对应契约：** `coding-provider-transport-retry` 的“每次自动重试必须保留独立可审计运行记录”。

**Files:**

- Modify: `src/product/coding_models/role_run.rs`
- Modify: `src/product/coding_attempt_store/role_run.rs`
- Modify: `src/product/coding_models/mod.rs`（仅在新增独立 retry model 文件时）
- Test: `src/product/coding_attempt_store/tests/role_run.rs`（若测试按现有聚合文件组织则修改对应模块）
- Test: `src/product/coding_workspace_engine/tests/provider_start_persistence.rs`

**Consumes:** 既有 `CodingRoleRun`、`create_role_run`、`update_role_run_status`。

**Produces:** 无损的 `retry_metadata`，以及 `create_retry_role_run`：新 run 链接前序 run，但绝不把前序 `Failed` 改成 `Superseded`。

- [ ] **Step 1: 写 role-run 序列化与状态保留失败测试**

  ```rust
  #[test]
  fn retry_run_keeps_failed_predecessor_and_records_cycle_metadata() { /* Failed #1, Running #2 */ }

  #[test]
  fn historical_role_run_without_retry_metadata_still_deserializes() { /* legacy JSON */ }
  ```

  断言 cycle `attempt_no` 从 1 递增到 3，retry run 的 `prior_run_id` 指向失败 run，前序的 `status == Failed` 与其 raw refs 保持不变；不允许调用 `supersede_latest_role_run_and_create`。

- [ ] **Step 2: 运行失败测试**

  Run: `cargo test --locked --lib retry_run_keeps_failed_predecessor_and_records_cycle_metadata`

  Expected: FAIL，因为当前数据模型没有 cycle/ordinal，现有 store helper 会把前序记录改成 `Superseded`。

- [ ] **Step 3: 新增兼容 retry metadata 与专用 store API**

  在 `CodingRoleRun` 添加 `#[serde(default)] pub retry_metadata: Option<CodingRoleRunRetryMetadata>`；将 trigger 显式保存为 `Initial`、`AutomaticRetry`、`ManualRetry`（或保留兼容值并在 metadata 中表达等价语义）。新增 API 的最小语义如下：

  ```rust
  pub fn create_retry_role_run(
      &self,
      attempt: &CodingExecutionAttempt,
      stage: CodingExecutionStage,
      role: CodingProviderRole,
      trigger: CodingRoleRunTrigger,
      node_id: Option<String>,
      retry: CodingRoleRunRetryMetadata,
  ) -> Result<CodingRoleRun, ProductStoreError>;
  ```

  该 API 必须验证 attempt/cycle/run ID、创建新的 `Running` run、保存 `prior_run_id`，但不改写前一个终态。首次和人工周期起点同样写入 cycle metadata，使 UI 不依赖推测。

- [ ] **Step 4: 运行持久化与历史兼容测试**

  Run: `cargo test --locked --lib retry_run_`

  Run: `cargo test --locked --lib provider_start_persistence`

  Expected: PASS；role run 事件与 raw-output refs 继续绑定各自 run，旧 JSON 无需迁移即可读取。

- [ ] **Step 5: 提交原子变更**

  ```bash
  git add src/product/coding_models/role_run.rs src/product/coding_attempt_store/role_run.rs src/product/coding_models/mod.rs src/product/coding_attempt_store/tests src/product/coding_workspace_engine/tests/provider_start_persistence.rs
  git commit -m "feat: persist provider retry cycles"
  ```

### Task 4: 将流式失败转换为可归类的 typed outcome

**对应契约：** `tasks.md` 2.1、2.2；技术失败和业务/交互结果的边界。

**Files:**

- Modify: `src/product/coding_workspace_engine/types.rs`
- Modify: `src/product/coding_workspace_engine/provider_stream.rs`
- Create: `src/product/coding_workspace_engine/provider_retry.rs`
- Modify: `src/product/coding_workspace_engine/mod.rs`
- Test: `src/product/coding_workspace_engine/tests/provider_failure_recovery.rs`
- Test: `src/product/coding_workspace_engine/tests/provider_start_persistence.rs`

**Consumes:** Task 3 的专用 retry role-run API；现有 provider 事件、取消 token、raw output persistence。

**Produces:** `ProviderInvocationOutcome` / `RetryableProviderFailure`，它携带分类、失败原因、可持久化的 partial raw output 与交互等待标记；stream 层在协调器拥有该调用时不抢先创建 gate。

- [ ] **Step 1: 写失败的分类矩阵测试**

  增加表驱动测试，逐项断言：start I/O error、提前结束、连接/进程中断、执行 timeout、可识别 503/504 为 retryable；`Aborted`、`ProviderProtocol`、正常完成后的 parser/structured-output error、`PermissionTimeout`、`ChoiceTimeout` 为非 retryable。

  ```rust
  #[test]
  fn provider_retry_classifier_retries_only_transport_failures() { /* failure matrix */ }

  #[test]
  fn permission_wait_timeout_preserves_interaction_without_retry_budget() { /* waiting */ }
  ```

- [ ] **Step 2: 运行失败测试**

  Run: `cargo test --locked --lib provider_retry_classifier_retries_only_transport_failures`

  Expected: FAIL，因为 `ProviderStreamOutcome` 只能表达成功，stream 失败会直接进入 `fail_provider_stream`。

- [ ] **Step 3: 定义 typed failure 并保留调用证据**

  在 `provider_retry.rs` 定义分类器与一次调用的结果；在 `provider_stream.rs` 让协调器路径收到成功、取消、非重试失败、可重试失败，而不是立刻改变 attempt/timeline。无论成功或失败，partial text 都保存到该 role run 的 raw output refs；普通调用没有 coordinator 时仍走当前失败收口，避免影响未纳入范围的角色。

  ```rust
  pub(crate) enum ProviderInvocationOutcome {
      Completed(ProviderStreamOutcome),
      Cancelled,
      NonRetryable { reason_code: String, error: CodingWorkspaceEngineError },
      RetryableTransport { reason_code: String, message: String, partial_output: String },
  }
  ```

- [ ] **Step 4: 移除嵌套 Codex fresh retry 绕过**

  删除 `CodingProviderStreamRun.fresh_retry` 的循环重启职责，以及 `coding.rs` / `rework.rs` 中向其提供 `CodingProviderFreshRetry` 的构造。将 resume-stall 映射成 `RetryableTransport`；新鲜会话的完整输入由 Task 5 协调器创建，并计入 `attempt_no`。不允许一个 role run 内出现第二次 Provider start。

- [ ] **Step 5: 运行 stream 与失败恢复测试**

  Run: `cargo test --locked --lib provider_retry_classifier_`

  Run: `cargo test --locked --lib provider_failure_recovery`

  Run: `cargo test --locked --lib provider_start_persistence`

  Expected: PASS；可重试失败尚未耗尽时不存在人工 blocked gate，权限/choice 仍停在原交互状态，取消不会排队下一次调用。

- [ ] **Step 6: 提交原子变更**

  ```bash
  git add src/product/coding_workspace_engine/types.rs src/product/coding_workspace_engine/provider_stream.rs src/product/coding_workspace_engine/provider_retry.rs src/product/coding_workspace_engine/mod.rs src/product/coding_workspace_engine/coding.rs src/product/coding_workspace_engine/rework.rs src/product/coding_workspace_engine/tests/provider_failure_recovery.rs src/product/coding_workspace_engine/tests/provider_start_persistence.rs
  git commit -m "refactor: classify provider retry outcomes"
  ```

### Task 5: 在 Coder、rework 与 Code Reviewer 外层执行有界重试

**对应契约：** `tasks.md` 2.2、2.3；自动重试与 rework/Plan Repair 隔离。

**Files:**

- Modify: `src/product/coding_workspace_engine/coding.rs`
- Modify: `src/product/coding_workspace_engine/rework.rs`
- Modify: `src/product/coding_workspace_engine/code_review.rs`
- Modify: `src/product/coding_workspace_engine/provider_retry.rs`
- Test: `src/product/coding_workspace_engine/tests/provider_failure_recovery.rs`
- Test: `src/product/coding_workspace_engine/tests/provider_rework_context.rs`
- Test: `src/product/coding_workspace_engine/tests/coder_resume_recovery.rs`

**Consumes:** Task 3 的 cycle store API、Task 4 的 typed outcome、现有 `execute_coding_with_commands_outcome`、`execute_coder_fix_from_review_outcome`、`execute_code_review_with_commands`。

**Produces:** `run_coder_with_retry_cycle` 与 `run_code_reviewer_with_retry_cycle`（名称可等价调整），总调用数严格小于等于 3；每次自动 retry 是新的 timeline/role run 和新的 provider session。

- [ ] **Step 1: 写 Coder 与 Reviewer 重试失败测试**

  ```rust
  #[tokio::test]
  async fn coder_retries_once_then_succeeds_with_two_auditable_runs() { /* transport, complete */ }

  #[tokio::test]
  async fn reviewer_three_transport_failures_open_one_human_gate_after_third_run() { /* 3 failures */ }

  #[tokio::test]
  async fn codex_fresh_session_recovery_consumes_one_of_three_attempts() { /* resume stall */ }

  #[tokio::test]
  async fn successful_retry_then_reviewer_finding_starts_rework_without_incrementing_retry_as_rework() { /* retry + finding */ }
  ```

  同时断言 Coder 第二次输入的 `resume_provider_session_id == None` 且仍使用原 worktree 与完整 prompt；Reviewer 第二次输入重新构建只读 diff/review context。

- [ ] **Step 2: 运行失败测试**

  Run: `cargo test --locked --lib coder_retries_once_then_succeeds_with_two_auditable_runs`

  Run: `cargo test --locked --lib reviewer_three_transport_failures_open_one_human_gate_after_third_run`

  Expected: FAIL；当前首次 stream 失败直接落 gate，Codex fresh retry 在同一 role run 内重启。

- [ ] **Step 3: 实现调用周期协调器并接入三个入口**

  协调器从 `attempt_no == 1` 开始调用，每次 `RetryableTransport`：先将当前 run 置为 `Failed`、写 reason/raw ref、关闭当前 timeline attempt，再在 `attempt_no < 3` 时创建 trigger 为 `AutomaticRetry` 的新 cycle run/node 并用新完整输入调用。第三次失败才调用现有角色专属 failure path。`Cancelled` 直接返回 `Aborted`；任何 `NonRetryable` 直接交还现有 protocol/interaction/business 分支。

  ```rust
  for attempt_no in 1..=3 {
      let role_run = create_run_for_cycle(attempt_no, prior_run_id)?;
      match invoke_fresh_session(role_run, attempt_no).await? {
          ProviderInvocationOutcome::Completed(output) => return Ok(output),
          ProviderInvocationOutcome::RetryableTransport { .. } if attempt_no < 3 => continue,
          ProviderInvocationOutcome::RetryableTransport { error, .. } => return terminal_failure(error).await,
          ProviderInvocationOutcome::Cancelled => return Err(CodingWorkspaceEngineError::Aborted),
          ProviderInvocationOutcome::NonRetryable { error, .. } => return Err(error),
      }
  }
  ```

  `execute_coder_fix_from_review_outcome` 使用同一个 Coder 协调器，但不得重置/增加 `rework_count`；输出正常后仍调用原有 Plan Defect parser 和 review-flow decision。

- [ ] **Step 4: 运行重试、rework、Plan Defect 边界测试**

  Run: `cargo test --locked --lib coder_retries_once_then_succeeds_with_two_auditable_runs`

  Run: `cargo test --locked --lib reviewer_three_transport_failures_open_one_human_gate_after_third_run`

  Run: `cargo test --locked --lib provider_rework_context`

  Run: `cargo test --locked --lib coder_resume_recovery`

  Expected: PASS；自动次数不影响 rework，Plan Defect 仍进入 Plan Repair，结构化输出无效仍不自动重试。

- [ ] **Step 5: 提交原子变更**

  ```bash
  git add src/product/coding_workspace_engine/coding.rs src/product/coding_workspace_engine/rework.rs src/product/coding_workspace_engine/code_review.rs src/product/coding_workspace_engine/provider_retry.rs src/product/coding_workspace_engine/tests/provider_failure_recovery.rs src/product/coding_workspace_engine/tests/provider_rework_context.rs src/product/coding_workspace_engine/tests/coder_resume_recovery.rs
  git commit -m "feat: retry coder and reviewer transport failures"
  ```

### Task 6: 将人工 retry 显式建为新的有限调用周期

**对应契约：** `coding-provider-transport-retry` 的“人工重试在自动预算耗尽后重新取得有限自动恢复机会”。

**Files:**

- Modify: `src/product/coding_workspace_engine/gates.rs`
- Modify: `src/product/coding_workspace_engine/failed_review_recovery.rs`
- Modify: `src/web/coding_ws_handler/runner.rs`（仅恢复调度所需处）
- Test: `src/web/coding_ws_handler/tests/failed_review_recovery.rs`
- Test: `src/web/coding_ws_handler/tests/failed_review_recovery/repeated.rs`
- Test: `src/web/coding_ws_handler/tests/plan_repair/provider_start_failure_recovery.rs`

**Consumes:** Task 3 的 `ManualRetry` cycle metadata 和 Task 5 的 role coordinator。

**Produces:** 人工 Coder/Reviewer retry 建立新 `cycle_id`，首 run 标记 `ManualRetry`，`prior_run_id` 指向耗尽周期最后一次失败；新周期仍至多三次调用。

- [ ] **Step 1: 写人工周期与不可自动重试边界失败测试**

  ```rust
  #[tokio::test]
  async fn manual_reviewer_retry_starts_linked_cycle_with_fresh_two_retry_budget() { /* exhausted -> manual -> 3 */ }

  #[tokio::test]
  async fn manual_coder_retry_links_to_exhausted_cycle_instead_of_replacing_history() { /* audit */ }

  #[tokio::test]
  async fn permission_or_choice_wait_timeout_does_not_enqueue_automatic_retry() { /* waiting */ }
  ```

- [ ] **Step 2: 运行失败测试**

  Run: `cargo test --locked --lib manual_reviewer_retry_starts_linked_cycle_with_fresh_two_retry_budget`

  Run: `cargo test --locked --lib manual_coder_retry_links_to_exhausted_cycle_instead_of_replacing_history`

  Expected: FAIL；现有人工 retry 使用 `RetryReview`/隐式 initial，不保存完整周期关联。

- [ ] **Step 3: 仅在用户 action 后创建 ManualRetry 周期**

  修改失败 gate 回复和 failed-review recovery：识别人工 `retry` action 时取得耗尽 role run，生成新 `cycle_id`、`attempt_no = 1` 与 `ManualRetry` 触发，并把它交给 Task 5 协调器。abort/terminate action 不创建 cycle，不重启 provider；Plan Repair 的 provider 入口不接入这项 policy。

- [ ] **Step 4: 运行恢复测试**

  Run: `cargo test --locked --lib manual_`

  Run: `cargo test --locked --lib failed_review_recovery`

  Run: `cargo test --locked --lib provider_start_failure_recovery`

  Expected: PASS；人工 retry 与旧失败记录形成可追溯链，等待交互和用户 cancel 从不触发自动调用。

- [ ] **Step 5: 提交原子变更**

  ```bash
  git add src/product/coding_workspace_engine/gates.rs src/product/coding_workspace_engine/failed_review_recovery.rs src/web/coding_ws_handler/runner.rs src/web/coding_ws_handler/tests/failed_review_recovery.rs src/web/coding_ws_handler/tests/failed_review_recovery/repeated.rs src/web/coding_ws_handler/tests/plan_repair/provider_start_failure_recovery.rs
  git commit -m "feat: link manual provider retry cycles"
  ```

### Task 7: 暴露并呈现 retry 审计状态

**对应契约：** `tasks.md` 3.1；前端必须区分自动重试中和已耗尽等待人工。

**Files:**

- Modify: `src/web/coding_ws_handler/protocol.rs`
- Modify: `src/web/coding_ws_handler/state.rs`
- Modify: `src/web/types.rs`
- Modify: `src/web/handlers/coding.rs`
- Modify: `web/src/api/types/coding.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/components/coding-workspace/RoleRunHistoryPanel.tsx`
- Test: `web/src/api/types.test.ts`
- Test: `web/src/state/coding-workspace-store.test.ts`
- Test: `web/src/state/chat-entries.test.ts`
- Test: `web/src/pages/CodingWorkspacePage.test.tsx`

**Consumes:** `CodingRoleRun.retry_metadata`、status、reason/raw refs。

**Produces:** WebSocket/REST snapshot 上的 retry metadata 与 UI 文案：`第 2/3 次自动重试`、`自动重试已耗尽，等待人工处理`；失败历史不被新成功覆盖。

- [ ] **Step 1: 写 TypeScript 解码、store 与组件失败测试**

  ```ts
  it("keeps failed automatic attempt and renders the next attempt as retry 2 of 3", () => {
    expect(screen.getByText("第 2/3 次自动重试")).toBeInTheDocument();
    expect(screen.getByText("失败：provider_503")).toBeInTheDocument();
  });

  it("shows human action only after the third transport failure", () => {
    expect(screen.getByText("自动重试已耗尽，等待人工处理")).toBeInTheDocument();
  });
  ```

- [ ] **Step 2: 运行失败前端测试**

  Run: `cd web && pnpm test -- --run web/src/api/types.test.ts web/src/state/coding-workspace-store.test.ts`

  Expected: FAIL，因为 DTO 和 store 尚未携带 retry metadata。

- [ ] **Step 3: 同步 API contract、store 与历史面板**

  后端 snapshot、`CodingSessionState` 和 REST response 直接序列化 role-run retry metadata；前端类型定义相同字段，`setSessionState` 重建 role run history。面板按 cycle/ordinal 排列，显示触发类型、失败原因、raw-output ref 和当前状态；不得把 `Superseded` 用作自动技术失败的标签。复用现有 blocked gate 控件，仅在第三次技术失败后显示人工 action。

- [ ] **Step 4: 运行前端定向测试与类型检查**

  Run: `cd web && pnpm test -- --run web/src/api/types.test.ts web/src/state/coding-workspace-store.test.ts web/src/state/chat-entries.test.ts web/src/pages/CodingWorkspacePage.test.tsx`

  Run: `cd web && pnpm tsc -b`

  Expected: PASS；旧没有 metadata 的 snapshot 仍可渲染，新 retry 状态可区分。

- [ ] **Step 5: 提交原子变更**

  ```bash
  git add src/web/coding_ws_handler/protocol.rs src/web/coding_ws_handler/state.rs src/web/types.rs src/web/handlers/coding.rs web/src/api/types/coding.ts web/src/state/coding-workspace-store.ts web/src/components/coding-workspace/RoleRunHistoryPanel.tsx web/src/api/types.test.ts web/src/state/coding-workspace-store.test.ts web/src/state/chat-entries.test.ts web/src/pages/CodingWorkspacePage.test.tsx
  git commit -m "feat: show provider retry history"
  ```

### Task 8: 交叉回归与本计划验收

**对应契约：** `tasks.md` 3.2、3.3；为人工组级最终确认提供稳定前置接口。

**Files:**

- Modify if needed: `src/product/coding_workspace_engine/tests/group_completion_authority.rs`
- Modify if needed: `src/product/coding_workspace_engine/tests/provider_failure_recovery.rs`
- Modify if needed: `web/src/pages/CodingWorkspacePage.test.tsx`
- Do not modify: OpenSpec 契约文件；本任务只在发现测试与已确认契约不一致时停止并重新确认。

**Consumes:** Tasks 1–7。

**Produces:** 可复现的验收证据，确认 Plan B 可以安全消费 `start_commit..completion_commit` 和 role-run retry history。

- [ ] **Step 1: 执行后端定向验收矩阵**

  Run: `cargo test --locked --lib group_completion_authority`

  Run: `cargo test --locked --lib runtime_handoff_group_completion`

  Run: `cargo test --locked --lib provider_failure_recovery`

  Run: `cargo test --locked --lib provider_rework_context`

  Required evidence: C1+C2 区间、空区间、范围外残留不被服务端处理、第一次失败后成功、三次失败后人工入口、fresh recovery 计数、权限/choice timeout、cancel、结构化输出无效、rework/Plan Repair 边界全部为 PASS。

- [ ] **Step 2: 执行前端定向验收矩阵**

  Run: `cd web && pnpm test -- --run web/src/api/types.test.ts web/src/state/coding-workspace-store.test.ts web/src/state/chat-entries.test.ts web/src/pages/CodingWorkspacePage.test.tsx`

  Run: `cd web && pnpm tsc -b`

  Required evidence: retry 1/2、成功后的失败历史、耗尽后的人工入口、legacy role-run 数据均为 PASS。

- [ ] **Step 3: 执行完整质量门禁**

  Run: `cargo fmt --check`

  Run: `cargo clippy --all-targets --all-features --locked -- -D warnings`

  Run: `cargo check --locked`

  Run: `cargo test --locked`

  Run: `cd web && pnpm test`

  Run: `cd web && pnpm tsc -b`

  Expected: 全部 PASS；若任一失败，先补最小回归修复，不进入 Plan B。

- [ ] **Step 4: 审查变更范围并提交验收修订**

  Run: `git diff --check`

  Run: `git status --short`

  验收：不存在新的 Aria 自动 staging/commit 后备逻辑，不存在静态目录黑名单门禁，不存在单一 role run 内的隐藏 fresh retry；不暂存用户已有的 `.pi-subagents/`、`HANDOFF.md` 或其他无关文件。

  ```bash
  git add src/product web/src web/package.json web/pnpm-lock.yaml
  git commit -m "test: verify coder commit and provider retries"
  ```

  若本任务没有新增文件，不创建空提交。

## 覆盖关系自检

| OpenSpec 工作包 / requirement | 计划任务 |
| --- | --- |
| 1.1 Coder 精确提交与不可清理 | Task 2 |
| 1.2 Coder HEAD、完整 evidence、幂等恢复 | Tasks 1–2 |
| 1.3 范围外残留、允许生成物、C1+C2、空区间、旧记录 | Tasks 1–2、8 |
| 2.1 技术失败分类与两次自动重试 | Task 4 |
| 2.2 独立 role run/timeline、无嵌套 fresh retry | Tasks 3–5 |
| 2.3 Coder/Reviewer 完整新上下文 | Task 5 |
| 2.4 人工 retry 新周期 | Task 6 |
| 3.1 前端状态 | Task 7 |
| 3.2 边界回归 | Tasks 4–8 |
| 3.3 定向与全量验证 | Task 8 |

## 实施交接

本计划完成后，先让实现者提供 Task 8 的新鲜验证输出和逐任务提交记录；随后才能开始第二份计划。实施方式可选：

1. Subagent-Driven（推荐）：每个 Task 分配独立实现者，Task 完成后做两阶段审查。
2. Inline Execution：在当前会话使用 `executing-plans`，按 Task 检查点批量执行。
