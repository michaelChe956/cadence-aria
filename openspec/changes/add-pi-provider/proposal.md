## Why

产品当前只支持 Claude Code 与 Codex 两个真实 Provider，无法让用户在日常 Workspace 与 Coding Workspace 中使用本机已安装的 Pi。接入 Pi 能提供第三个可供用户选择的执行端，使用户可在日常 Workspace 与 Coding Workspace 中使用本机已安装的 Pi。

## What Changes

- 在日常产品执行链路中新增 Pi Provider：Story、Design、Work Item Workspace 的 Author / Reviewer，以及 Coding Workspace 的 Coder、Code Reviewer、Internal Reviewer 均可选择 Pi。
- 为 Pi 提供健康检查、可用性展示、会话流式输出、取消与会话续接能力，并与现有 Provider Registry 和 WebSocket 运行链路集成。
- 所有上述角色默认使用 `Auto` 权限模式；Claude Code 与 Codex 保留既有 `Supervised` 逐工具确认，Pi 因无逐工具批准机制仅以 `Auto` 运行。
- Provider 启动或运行失败时保持现有行为：直接报告失败，不在运行期自动切换到其他 Provider。
- 保持添加代码库与仓库初始化仅使用 Claude Code，不展示或调用 Pi。
- 保持 Task Runner、其 CLI/API、专用协议与旧的 Fake Provider Workspace Runner 的可调度 Provider 范围和运行行为不变；为满足流式链路的共享类型约束，`ProviderType` 会增加 `Pi` 变体，但 Pi 在这些路径中被显式拒绝且不被调度。

## Capabilities

### New Capabilities

- `pi-provider-integration`: 在活跃的 Workspace 与 Coding Workspace 工作流中选择、执行 Pi Provider（Auto 模式）。

### Modified Capabilities

- 无。

## Impact

- Provider 名称与前后端 API contract、Provider 健康检查及可选 Provider 目录。
- Workspace 与 Coding Workspace 的 WebSocket 流式 Provider Registry、会话控制与现有授权桥接（Pi 走 Auto，不新增授权交互）。
- 普通 Workspace 与 Coding Workspace 的 Provider 配置持久化、默认权限模式和界面。
- 新增 Pi CLI/RPC 适配及按会话加载的 Aria 扩展；不引入全局 Pi 配置修改。
