# Phase 4 Task 3 报告

## 完成内容

- 新增 `useCockpitHotkeys`：以 `COCKPIT_HOTKEYS` 唯一映射注册并清理 document 键盘监听；忽略已阻止事件、Alt 修饰键和 input/textarea/select/contenteditable 焦点。
- 语义模块导出 `CODING_WORKSPACE_HOTKEYS` 为 `COCKPIT_HOTKEYS` 的同一引用；两页均消费 `COCKPIT_HOTKEYS`，测试使用 `toBe` 验证引用同一性。
- 对话驾驶舱：确认与推进均先检查当前门/可推进状态；反馈快捷键仅聚焦反馈编辑器；接管快捷键经同一 `ConfirmTwiceButton` ref 的 `arm()`，第二击才沿既有接管路径发起请求。
- 编码工作区：confirm 按 Plan Repair、最终确认、阶段门顺序调用现有动作；advance 仅在 prepare_context 或可恢复 review_request 调用既有 `startCoding`；反馈仅聚焦 Composer，不发送输入。
- 对话页覆盖当前门打开、关闭及不存在时的 confirm guard；门卡 input 与收件箱 textarea 均不能抢占快捷键。

## 红灯与绿灯证据

- 红灯：`cd web && pnpm test src/hooks/useCockpitHotkeys.test.tsx src/state/cockpit-operation-semantics.test.ts src/pages/ChatCockpitPage.test.tsx src/pages/CodingWorkspacePage.test.tsx`
  - hook 模块和 `CODING_WORKSPACE_HOTKEYS` 尚不存在；页面 document keydown 无分发，目标用例失败。
- 绿灯（最终）：同一精确命令通过，4 个文件、59 个用例全部通过。
- 类型检查：`cd web && pnpm tsc -b` 通过。
- 全量测试：`cd web && pnpm test` 通过，161 个文件、1351 个用例全部通过。

## Commit 列表

- `4e7532d0 feat: 统一两页操作快捷键语义`

## 自我审查

- 未改动 `src/`、ImageCreate 页面或前端依赖。
- 危险接管快捷键未绕过 `ConfirmTwiceButton`，首击仅进入 armed 状态。
- 无门确认、无已确认可推进状态、编辑控件焦点及 Alt 修饰组合均无协议发送。
- 全量测试输出仍包含既有 jsdom navigation stderr（`IssueLifecycleWorkbench.queue-density.test.tsx`），但退出状态为 0、全部用例通过。
