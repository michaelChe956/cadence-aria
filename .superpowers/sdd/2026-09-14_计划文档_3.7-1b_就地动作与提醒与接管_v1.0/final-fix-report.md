# 3.7-1b 终审修复报告

## 状态

已完成六项终审修复，并覆盖对应前端回归。

## 修复内容

1. 根路由将驾驶舱“去处理”直接导航至 `/workbench/workspace/$sessionId`，移除无消费者的自定义事件。
2. 会话观察器按 `observerRefreshIntervalMs` 定期刷新项目、Issue 和生命周期目录；新进入聚合窗口 K 的会话会进入观察集合。
3. 当前会话即使在 K 外也保留在页面级 `inbox`；新增 `countedInbox`，仅用于 L1/L2/L3/L4 全局提醒、浏览器标题、favicon、系统通知与升级。
4. 以 `flowKind` 统一 typed/legacy action facade；single-candidate 持久快照即使尚无 live turn 也为 typed，且无 command id 时不会发送反馈。
5. 收件箱新项脉冲使用 `motion-safe:animate-pulse`，在 reduced-motion 偏好下自动降级。
6. 两个 typed 反馈输入框默认空值、提供输入提示、空白时禁用提交；typed 快照等待命令同步时不显示遗留返修路径。

## 改动文件

- `web/src/components/chat-workspace/cockpit/CockpitInbox.tsx`
- `web/src/components/chat-workspace/entries/GatePromptEntry.tsx`
- `web/src/components/chat-workspace/entries/p1-entries.test.tsx`
- `web/src/components/cockpit/CockpitShell.test.tsx`
- `web/src/components/cockpit/CockpitShell.tsx`
- `web/src/hooks/useWorkspaceSessionObservers.test.tsx`
- `web/src/hooks/useWorkspaceSessionObservers.ts`
- `web/src/pages/ChatCockpitPage.test.tsx`
- `web/src/pages/ChatWorkspacePage.test.tsx`
- `web/src/router.test.tsx`
- `web/src/router.tsx`
- `web/src/state/cockpit-action-routing.test.ts`
- `web/src/state/cockpit-action-routing.ts`
- `web/src/state/workspace-chat-rebuild.ts`
- `web/src/state/workspace-ws-store.rebuild.test.ts`

## 验证

- 定向回归：8 文件、126 测试通过。
- 全量前端：`pnpm test`，145 文件、1226 测试通过。
- TypeScript：`pnpm exec tsc -b --pretty false` 通过。
- 已执行 `git diff --check`，无空白错误。

## 已知输出

全量测试期间 `IssueLifecycleWorkbench.queue-density.test.tsx` 会打印 jsdom 对非 hash 页面导航未实现的既有 stderr；该测试本身通过，整套测试全绿。
