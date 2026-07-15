# Work Item 8 范围修订与 Attempt 回退实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修订 Work Item 8 的最小合法写入范围，补入 Repository integration-safe seam 与大文件门禁要求，并把 `coding_attempt_0001` 回退到 Work Item 8 尚未开始 Coding 的合法状态。

**Architecture:** 同步修改 Source Draft、Compiled Work Item 和 Verification Plan，避免范围版本漂移；代码范围只开放 Repository 注册 seam 所需的四个 Web 文件。回退以 Work Item 7 完成提交 `7e6114fc94af579273ff1578128c7c2d29f0dcfa` 为边界，保留 Work Item 1–7，暂存并清理 Work Item 8 的未提交代码与运行记录。

**Tech Stack:** Cadence Aria `.aria` JSON 持久化、Git worktree、jq、Bash、Rust/Cargo 验证命令。

## Global Constraints

- 只操作 `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0709` 中的 Work Item/Attempt 数据和 `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001` 中的 Work Item 8 未提交代码。
- 保留 Work Item 1–7 的 Unit、Timeline、Role Run、Review、Handoff 和提交。
- 不修改或回退边界提交 `7e6114fc94af579273ff1578128c7c2d29f0dcfa`。
- 不开放整个 `src/**`，只开放方案 A 确认的四个精确源文件。
- 不修改 `src/web/handlers/coding.rs`、`src/web/handlers/lifecycle.rs`、`src/web/provider_availability.rs`、`src/product/**`、`src/cross_cutting/**`、`web/src/**` 或 `web/e2e/**`。
- 当前 Work Item 8 未提交代码使用 `git stash -u` 保存，不直接丢弃。
- 不重启前后端服务；最终停在 `attempt.stage=prepare_context` 与 `coding_unit_0008.status=running`。
- Cargo 命令不得携带 `-j 1`。

---

### Task 1: 同步修订 Work Item 8 三份权威上下文

**Files:**

- Modify: `.aria/projects/project_0001/issues/issue_0001/work_item_plan_drafts/issue_work_item_plan_0001/round_002/draft_010.json`
- Modify: `.aria/projects/project_0001/issues/issue_0001/work-items/work_item_compile_20260712024139064_008.json`
- Modify: `.aria/projects/project_0001/issues/issue_0001/verification-plans/verification_plan_compile_20260712024139064_008.json`

**Interfaces:**

- Consumes: 已确认方案 A、Work Item 8 原始 AC/Design 追踪、Work Item 6/7 handoff。
- Produces: Source Draft、Compiled Work Item 与 Verification Plan 一致的范围和门禁。

- [x] **Step 1: 扩展精确 Exclusive Write Scopes**

加入：

```text
src/web/state.rs
src/web/handlers/product_resources.rs
src/web/handlers/repository_registration.rs
src/web/handlers/mod.rs
```

移除与这些精确文件冲突的 `src/**` Forbidden Scope；其余文件继续由 Exclusive Scope allowlist 和明确禁止说明约束。

- [x] **Step 2: 追加正式范围修订说明**

说明 Work Item 8 现在拥有最小 integration-safe seam：临时 HOME、命令环境、共享 Runner/Gate/Registry、Cadence preparation、Repository initializer 和 host readiness；禁止修改 Coding/Lifecycle/Provider Gate 实现。

- [x] **Step 3: 固化大文件拆分职责**

规定 Repository 注册依赖构造优先拆入 `src/web/handlers/repository_registration.rs`，`product_resources.rs` 只保留 HTTP handler 接线，禁止通过把辅助逻辑搬入无关模块规避 800 行门禁。

- [x] **Step 4: 更新 Verification Plan**

在全量回归前新增必需命令：

```text
cargo test --locked --test it_core large_file_guard::product_source_and_test_files_stay_under_line_limit
```

同时更新 Draft 内嵌 Verification Plan、Compiled Work Item 的实施上下文和 handoff 描述。

- [x] **Step 5: 校验三份 JSON**

Run:

```text
jq empty <draft_010.json>
jq empty <work_item_008.json>
jq empty <verification_plan_008.json>
```

Expected: 三条命令均退出码 0，三份 Exclusive/Forbidden Scope 与门禁命令一致。

### Task 2: 保存并清理 Work Item 8 代码工作树

**Files:**

- Worktree: `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001`

**Interfaces:**

- Consumes: 当前 Work Item 8 tracked/untracked 修改。
- Produces: HEAD 仍为 Unit 7 边界提交、工作树干净、可恢复的 Git stash。

- [x] **Step 1: 再次确认 HEAD 与边界一致**

Run:

```text
git rev-parse HEAD
```

Expected: `7e6114fc94af579273ff1578128c7c2d29f0dcfa`。

- [x] **Step 2: 保存当前 Work Item 8 修改**

Run:

```text
git stash push -u -m "pre-work-item8-reset-2026-07-14"
```

Expected: tracked 与 untracked Work Item 8 文件进入 stash。

- [x] **Step 3: 验证工作树干净**

Run:

```text
git status --short --branch -uall
```

Expected: 只显示分支头，无文件修改。

### Task 3: 回退 Coding Attempt 到 Work Item 8 未开始边界

**Files:**

- Modify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json`
- Modify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/units/coding_unit_0008.json`
- Modify: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/timeline-nodes.json`
- Delete: Work Item 8 对应的 Role Run、Role Run Event、Artifact、Chat Entry、Code Review、Provider Raw、Stage Gate、Blocked Gate、Rework Instruction 和 `coding_context_note_0004.json`

**Interfaces:**

- Consumes: Unit 7 `completed_at=2026-07-14T10:43:58.672025244+00:00`、commit `7e6114fc94af579273ff1578128c7c2d29f0dcfa`。
- Produces: Unit 8 是唯一活动 Unit，Attempt 停在 `prepare_context`。

- [x] **Step 1: 列出所有边界后文件并逐项核对**

Expected: Timeline 仅删除 node 32–37；Role Run 仅删除 31–36；其他类别严格按 `created_at > boundary_time` 或引用关系确定。

- [x] **Step 2: 清理 Work Item 8 时间线和关联文件**

保留 Timeline node 1–31，删除所有只属于 Work Item 8 失败尝试的关联数据。

- [x] **Step 3: 恢复 Unit 8**

写入：

```json
{
  "status": "running",
  "started_at": "2026-07-14T10:43:58.677615646+00:00",
  "completed_at": null,
  "handoff_ref": null,
  "completion_commit": null,
  "summary": "进入下一个 Work Item",
  "updated_at": "2026-07-14T10:43:58.677615646+00:00"
}
```

- [x] **Step 4: 恢复 Attempt 顶层状态**

写入：

```json
{
  "status": "running",
  "stage": "prepare_context",
  "rework_count": 0,
  "current_work_item_id": "work_item_compile_20260712024139064_008",
  "active_unit_id": "coding_unit_0008",
  "head_commit": "7e6114fc94af579273ff1578128c7c2d29f0dcfa",
  "provider_conversations": [],
  "updated_at": "2026-07-14T10:43:58.677615646+00:00"
}
```

### Task 4: 验证回退与页面可继续状态

**Files:**

- Test: `.codex/skills/rollback-coding-attempt/scripts/validate-rollback-state.sh`

**Interfaces:**

- Consumes: 修订后的 Work Item 8 与回退后的 Attempt。
- Produces: 可由 UI 重新启动 Work Item 8 的一致状态。

- [x] **Step 1: 校验 JSON 与状态不变量**

Run:

```text
bash .codex/skills/rollback-coding-attempt/scripts/validate-rollback-state.sh \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/units
```

Expected: `rollback_state_ok`。

- [x] **Step 2: 校验引用与计数**

确认 Role Run、Event、Artifact 三类 id 集合一致；Timeline 最后一个节点为 `coding_node_0031`；不存在 Work Item 8 未处理 blocked/choice gate。

- [x] **Step 3: 校验 Work Item 8 修订内容**

确认 Draft/Compiled 范围一致，Verification Plan 含 large file guard，Work Item 8 仍为 pending/not_started，Unit 9–10 仍为 pending。

- [x] **Step 4: UI 交接**

保持服务运行，不自动启动 Provider。用户刷新 Coding Workspace 后，点击继续 Coding 重新启动 Work Item 8。
