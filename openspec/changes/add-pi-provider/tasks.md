## 1. Provider 目录与可用性

- [x] 1.1 将 Pi 纳入活跃流式 Provider 的名称（`ProviderName`）、健康检查、状态接口和前端选择目录。
- [x] 1.2 给 `ProviderType` 加 `Pi` 变体并保证 `provider_type_for_name` 可映射；Task Runner HTTP 入口与 `RoutingProviderAdapter` 对 `ProviderType::Pi` 补单元测试，断言明确拒绝且 adapter 不被调用；断言 `adapter_compatibility` 兼容性矩阵无 Pi 条目、所有静态节点契约不产生 Pi。
- [x] 1.3 保持仓库初始化的 Claude Code 专用选项与执行路径不受 Pi 影响。

## 2. Pi 流式会话适配

- [x] 2.1 实现 Pi RPC 会话适配，覆盖流式输出、会话标识、恢复、取消和错误映射（Auto-only）。
- [ ] 2.2 实现结构化提问扩展（`aria-ask.ts`，`include_str!` 交付）：Pi 需要用户决策时经 `ask_user` 工具→`extension_ui_request(select)`→既有 `ChoiceRequest` 往返，答案在同进程内接续；更新 Pi prompt 指引从「输出文本暂停信号」改为「使用 `ask_user` 工具提问」。

## 3. Workspace 角色配置与执行

- [x] 3.1 为普通 Workspace 的 Author 与 Reviewer 持久化独立权限模式，并将默认值设为 `Auto`。
- [x] 3.2 让普通 Workspace 的 Author、Reviewer 与返修运行支持 Pi（Auto 模式；失败直接报错，不做运行期降级）。

## 4. Coding Workspace 角色配置与执行

- [x] 4.1 将 Coding Workspace 各角色的新建默认权限模式改为 `Auto`，保留独立的 `Supervised` 配置。
- [x] 4.2 让 Coder、Code Reviewer、Internal Reviewer 支持 Pi（Auto 模式；失败直接报错，不做运行期降级）。

## 5. 用户界面与运行可见性

- [x] 5.1 在普通 Workspace 与 Coding Workspace 的 Provider 配置中展示 Pi；Claude Code 与 Codex 提供一致的 `Auto` / `Supervised` 控制，Pi 仅显示 `Auto`。
- [x] 5.2 在运行事件与界面状态中呈现不可用原因与失败状态。

## 6. 回归验证

- [x] 6.1 为 Pi 健康检查、目录展示、会话协议、取消、恢复和 Auto 运行补充后端测试。
- [x] 6.2 为 Story、Design、Work Item 三种 Workspace 入口（共享 `workspace_engine`）及 Coding 角色补充 Provider、权限，以及所选 Provider 启动或运行失败时直接报告且不切换 Provider 的回归测试。
- [x] 6.3 验证仓库初始化与 Task Runner 未被 Pi 扩张（含 Task Runner HTTP 入口、router、兼容性矩阵、节点契约拒绝 Pi 的回归断言），并执行相关前后端质量检查。
