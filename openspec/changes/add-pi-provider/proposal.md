## Why

产品当前只支持 Claude Code 与 Codex 两个真实 Provider，无法让用户在日常 Workspace 与 Coding Workspace 中使用本机已安装的 Pi。接入 Pi 能提供第三个可用执行端，并在其中一个 Provider 不可用时保持关键工作流可继续执行。

## What Changes

- 在日常产品执行链路中新增 Pi Provider：Story、Design、Work Item Workspace 的 Author / Reviewer，以及 Coding Workspace 的 Coder、Code Reviewer、Internal Reviewer 均可选择 Pi。
- 为 Pi 提供健康检查、可用性展示、会话流式输出、取消与会话续接能力，并与现有 Provider Registry 和 WebSocket 运行链路集成。
- 所有上述角色默认使用 `Auto` 权限模式；用户可分别切换为 `Supervised`，在页面上确认或拒绝单次工具调用。
- 为健康或启动前失败的 Provider 增加安全的自动降级：保留用户当前选择为首选，依次尝试其他健康 Provider；产生工具副作用或部分结果后不静默切换。
- 保持添加代码库与仓库初始化仅使用 Claude Code，不展示或调用 Pi。
- 保持 Task Runner、其 CLI/API、专用协议与旧的 Fake Provider Workspace Runner 不变；Pi 不在本次变更中接入这些路径。

## Capabilities

### New Capabilities

- `pi-provider-integration`: 在活跃的 Workspace 与 Coding Workspace 工作流中选择、执行、监督并安全降级 Pi Provider。

### Modified Capabilities

- 无。

## Impact

- Provider 名称与前后端 API contract、Provider 健康检查及可选 Provider 目录。
- Workspace 与 Coding Workspace 的 WebSocket 流式 Provider Registry、会话控制与授权桥接。
- 普通 Workspace 与 Coding Workspace 的 Provider 配置持久化、默认权限模式和界面。
- 新增 Pi CLI/RPC 适配及按会话加载的 Aria 扩展；不引入全局 Pi 配置修改。
