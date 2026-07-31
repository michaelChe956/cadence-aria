## 1. Provider 目录与可用性

- [ ] 1.1 将 Pi 纳入活跃流式 Provider 的名称、健康检查、状态接口和前端选择目录。
- [ ] 1.2 保持仓库初始化的 Claude Code 专用选项与执行路径不受 Pi 影响。

## 2. Pi 流式会话适配

- [ ] 2.1 实现 Pi RPC 会话适配，覆盖流式输出、会话标识、恢复、取消和错误映射。
- [ ] 2.2 实现每会话临时 Pi 扩展，将工具调用转换为 `Auto` 放行或 `Supervised` 授权请求。

## 3. Workspace 角色配置与执行

- [ ] 3.1 为普通 Workspace 的 Author 与 Reviewer 持久化独立权限模式，并将默认值设为 `Auto`。
- [ ] 3.2 让普通 Workspace 的 Author、Reviewer 与返修运行支持 Pi、授权桥接及安全降级。

## 4. Coding Workspace 角色配置与执行

- [ ] 4.1 将 Coding Workspace 各角色的新建默认权限模式改为 `Auto`，保留独立的 `Supervised` 配置。
- [ ] 4.2 让 Coder、Code Reviewer、Internal Reviewer 支持 Pi、授权桥接及安全降级。

## 5. 用户界面与运行可见性

- [ ] 5.1 在普通 Workspace 与 Coding Workspace 的 Provider 配置中展示 Pi 和一致的 `Auto` / `Supervised` 控制。
- [ ] 5.2 在运行事件与界面状态中呈现 Provider 降级、实际执行 Provider 和不可用原因。

## 6. 回归验证

- [ ] 6.1 为 Pi 健康检查、目录展示、会话协议、取消、恢复和两种授权模式补充后端测试。
- [ ] 6.2 为 Story、Design、Work Item 三种 Workspace 入口（共享 `workspace_engine`）及 Coding 角色补充 Provider、权限和降级回归测试。
- [ ] 6.3 验证仓库初始化与 Task Runner 未被 Pi 扩张，并执行相关前后端质量检查。
