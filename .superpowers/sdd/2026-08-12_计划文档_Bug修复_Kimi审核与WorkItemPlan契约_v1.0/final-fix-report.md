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
