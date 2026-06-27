# WorkItemPlan 工具调用作者展示错误调试记录

## 现象

- 手工端到端测试中，WorkItemPlan Workspace 的工具调用气泡标题显示为 `系统 · Claude Code`。
- 截图中可见工具调用内容为 Claude Code 作者运行中的 `find`、`grep` 等命令。
- 预期显示为作者身份，例如 `作者 · Claude Code`，并使用作者气泡样式，而不是系统红色虚线样式。

## 当前证据

- 当前运行数据位于 `.aria/projects/project_0001/issues/issue_0001/workspace-sessions/workspace_session_0003.json`：
  - `workspace_type = work_item_plan`
  - `author_provider = claude_code`
  - `status = running`
- 对应 timeline 数据位于 `.aria/projects/project_0001/issues/issue_0001/workspace-timelines/workspace_session_0003/`：
  - `timeline_nodes.json` 中 `timeline_node_002` 的 `node_type = work_item_plan_outline_run`
  - `timeline_node_002` 的 `agent = claude_code`
  - `timeline_node_details/timeline_node_002.json` 中 `agent_role = author`
  - `provider.name = claude_code`
  - 该 detail 中包含截图所示命令，例如：
    - `find /Users/michaelche/Documents/git-folder/github-folder/cadence-aria/src -maxdepth 2 -type d | sort`
    - `grep -rn "provider_probe\|provider_availability\|provider_name_available\|provider_type_available" ...`

结论：后端持久化数据已经正确表达“作者 + Claude Code”，问题主要在前端展示映射。

## 初步根因

前端聊天分组标题由 `MessageGroupView.groupTitle()` 生成：

- `web/src/components/chat-workspace/MessageGroupView.tsx`
  - 基础标签来自 `group.role`
  - provider 来自 entry metadata 的 `provider` 或 `agent`
  - 当 `group.role = system` 且 metadata provider 为 `claude_code` 时，标题组合为 `系统 · Claude Code`

`group.role` 来自 `web/src/components/chat-workspace/message-grouping.ts`：

- 如果一个 group 只有 `execution_event`，没有 `provider_stream`，会使用第一条 entry 的 `role`
- 这类工具调用 entry 当前被构造为 `role = system`

role 推导缺口在两个地方：

- `web/src/hooks/useWorkspaceWs.ts` 的 `entryRoleForNode()` 只识别：
  - `reviewer_run -> reviewer`
  - `author_run -> author`
  - detail 中 `agent_role = reviewer/author`
  - 但实时 execution event 到达时，detail 可能尚未带有后端完整 `agent_role`
- `web/src/state/workspace-ws-store.ts` 的 `chatRoleForNode()` 和 `agentRoleFor()` 只识别旧节点：
  - `author_run`
  - `reviewer_run`
  - `revision`
  - 未覆盖 WorkItemPlan 新增节点：
    - `work_item_plan_outline_run`
    - `work_item_draft_run`
    - `work_item_batch_run`
    - 对应 review 节点也应按 reviewer 映射

## 影响范围

- 直接影响 WorkItemPlan 两阶段流程的作者运行节点，尤其是只有工具调用/执行事件、还没有 provider stream 文本时的气泡标题和样式。
- 可能影响刷新恢复后的 WorkItemPlan 新节点消息重建，因为 `buildChatEntries()` 依赖 `chatRoleForNode()`，但该函数当前不使用 `detail.agent_role`。
- Story / Design / 普通 WorkItem 的旧 `author_run`、`reviewer_run` 路径目前不受影响。

## 后续修复建议

先补失败测试，再改映射：

- `web/src/state/workspace-ws-store.test.ts`
  - 覆盖 `work_item_plan_outline_run`、`work_item_draft_run`、`work_item_batch_run` 的 execution event 恢复为 `role = author`
  - 覆盖 `work_item_plan_outline_review`、`work_item_draft_review`、`work_item_batch_review` 恢复为 `role = reviewer`
- `web/src/hooks/useWorkspaceWs.test.tsx`
  - 覆盖实时 `execution_event` 挂在 `work_item_plan_outline_run` 时，chat entry role 为 `author`
- 可选补 `web/src/components/chat-workspace/MessageGroupView.test.tsx`
  - 覆盖只有 author execution events、无 stream 时标题为 `作者 · Claude Code`

实现方向：

- 在前端集中抽取 node type 到 chat role 的映射，避免 `useWorkspaceWs.ts` 和 `workspace-ws-store.ts` 各维护一份。
- 映射应包含 WorkItemPlan 专属 author/reviewer 节点，并在可用时优先使用 `detail.agent_role`。

## 当前处理状态

- 仅完成定位和记录。
- 未修改实现代码。
- 当前服务仍在运行中，可继续手工观察。
