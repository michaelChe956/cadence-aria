# 最终审核修复报告

- 提交：`3fc8b2cf fix(workspace): require authored revision feedback`
- 范围：最终审查的人工返修来源、WorkItemPlan Markdown canonical contract 与 OpenSpec 术语一致性。

## 已修复

- `user_triage_required` 且 findings 为空时，后端仅接受 `source="human"` 的非空 description；无 source 或 `review_findings` 均被拒绝。Story、Design、WorkItem 与 WorkItemPlan Outline 均有回归覆盖。
- GatePromptEntry 不再以 reviewer summary/comments/content 生成快捷返修；显示用户在下方输入人工修改说明的提示。可信 findings 的快捷返修标记 `source="review_findings"`；ChatInputBar 人工输入标记 `source="human"`。
- WorkItemPlan 基准 Markdown prose 删除手写 heading/token 列表，仅由 canonical validator contract 提供 heading 和 `[TASK-001]` 示例。
- OpenSpec/design/tasks/plan 明确：用户主动 Abort 保持既有取消语义；repair provider 启动/运行失败才 fail-closed；Markdown 通用路径与 JSON Outline/Draft 主链使用不同 schema。

## TDD 证据

- RED：`cargo test --locked --lib human_confirm_request_change_requires_context` 在实现前失败，因为无 source description 被接受。
- RED：受影响 Vitest 在实现前失败，因为 UI 仍展示 triage 快捷返修且快捷 payload 不带 source。
- GREEN：
  - `cargo test --locked --lib human_confirm_request_change_requires_context`
  - `cargo test --locked --lib work_item_plan_output_schema`
  - `cd web && pnpm test -- src/components/chat-workspace/entries/p1-entries.test.tsx src/components/chat-workspace/ChatInputBar.test.tsx src/pages/ChatWorkspacePage.review.test.tsx`
  - `cd web && pnpm tsc -b`
  - `cargo fmt --check`
  - `git diff --check`

## 约束

- 未触碰 `.aria`，未调用真实 Provider。
- 未执行完整 Rust suite；交由最终集成验证阶段执行。

## 后续全量测试夹具修复

- 复现：`outline_human_confirm_revision_is_recoverable_before_provider_spawn` 与 `work_item_plan_outline_human_confirm_change_uses_outline_revision` 都以缺少 `source` 的 payload 调用 `RequestChange`，因此命中无可信 findings 的新后端 guard，错误为“请提供 source=human 的非空修改说明”。
- 修复：仅将两个测试夹具 payload 补为 `{"description": ..., "source": "human"}`；没有修改生产逻辑。
- 验证：两个定向测试分别通过；`cargo test --locked --lib human_confirm_request_change_requires_context` 也通过；`cargo fmt --check` 与 `git diff --check` 通过。

## 最终复审 Important 修复

- I1：GatePromptEntry 原来错误地把可信 findings 的快捷返修限制在 `user_confirm_allowed`。现在只要存在有效 findings 就展示“采纳建议并返修”，仍发送 `source="review_findings"`；`user_triage_required` 且 findings 为空的人工输入提示与快捷按钮隐藏行为保持不变。
- I2：增强 Kimi repair Abort 回归，断言 repair 启动次数为 2、Abort 后回到 `PrepareContext`、不存在 `latest_review_verdict` fallback。该断言在现有生产行为下通过；除 I1 前端判断外没有生产逻辑改动。
- RED：新增 triage+findings 前端用例后，`pnpm test -- src/components/chat-workspace/entries/p1-entries.test.tsx` 找不到“采纳建议并返修”，证明旧条件阻断入口。
- GREEN：前端定向 Vitest（106 files / 816 tests）及 `cargo test --locked --lib repair_abort_closes_started_event_as_failed` 通过。

## Scoped re-review payload 收敛

- 根因：triage+findings 的快捷入口已显示，但 `requestChangeDescription` 仍只在 `user_confirm_allowed` 时输出纯 findings，导致 triage 路径把 summary 与 comments 混入 `source="review_findings"` payload。
- RED：增强 triage+findings 测试后，payload 包含“不可信摘要不得作为返修依据”和“不可信 comments 不得作为返修依据”。
- 修复：只要存在有效 findings，快捷 payload 均使用 `formatFindingsForRevision`；triage+空 findings 不产生快捷入口，继续使用人工输入路径。
- GREEN：`cd web && pnpm test -- src/components/chat-workspace/entries/p1-entries.test.tsx`（106 files / 816 tests）、`pnpm tsc -b`、`cargo fmt --check`、`git diff --check` 通过。
