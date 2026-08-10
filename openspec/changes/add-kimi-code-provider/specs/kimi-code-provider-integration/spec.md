## Purpose

让日常 Workspace、Coding Workspace 与图片创作（image-create）将 Kimi Code 作为与 Claude Code、Codex 并列的真实流式 Provider 使用（Pi 仅 Auto，Kimi 支持 Auto 与 Supervised），通过 ACP（Agent Client Protocol）JSON-RPC over stdio 接入，保留一致的可用性、授权、失败报告与审计体验。

## ADDED Requirements

### Requirement: Kimi 在活跃 Provider 工作流中可发现且可选择

系统 SHALL 将 Kimi 作为真实 Provider 的健康状态条目公开（健康检查命令 `kimi --version`，最低支持版本 0.34.0），并在 Kimi 可用或当前已被配置时，允许用户在 Story、Design、Work Item Workspace 的 Author / Reviewer、Coding Workspace 的 Coder / Code Reviewer / Internal Reviewer 以及 image-create 中选择 Kimi。不可用的 Kimi SHALL 提供与其他 Provider 一致的不可用原因或安装提示。

#### Scenario: Kimi 可用时出现在角色选择器中

- **WHEN** Kimi CLI 已安装、版本 ≥ 0.34.0、可验证且用户打开任一支持 Provider 选择的 Workspace 配置
- **THEN** 相应角色的 Provider 选择器显示 Kimi
- **AND THEN** 用户可以保存 Kimi 作为该角色的 Provider

#### Scenario: Kimi 版本过低时报告可操作原因

- **WHEN** 已安装 Kimi 版本低于 0.34.0
- **THEN** 健康检查报告 Kimi 不可用
- **AND THEN** 界面说明版本过低并提示升级

#### Scenario: Kimi 不可用时保留已配置值并说明原因

- **WHEN** 某角色已配置 Kimi 且后续健康检查报告 Kimi 不可用
- **THEN** 系统保留该角色的已配置 Provider 值
- **AND THEN** 界面说明 Kimi 当前不可执行的原因或安装提示

### Requirement: Kimi 以 ACP 流式会话执行并支持控制操作

系统 SHALL 让 Kimi 在普通 Workspace、Coding Workspace 与 image-create 中以 ACP（`kimi acp`，JSON-RPC over stdio）流式会话执行，向既有前端协议发送文本、工具调用、完成、失败与会话标识事件。系统 MUST 支持取消活动 Kimi 会话。系统 SHALL 复用现有 `JsonRpcPeer` 传输层。终态由 ACP `session/prompt` 响应的 `stopReason` 判定。

#### Scenario: Kimi 执行生成角色产物

- **WHEN** 用户启动一个已选择 Kimi 的 Author、Reviewer、Coder、Code Reviewer、Internal Reviewer 或 image-create 运行
- **THEN** 系统将 Kimi 输出流转发到现有 Workspace、Coding Workspace 或 image-create 会话
- **AND THEN** 系统以与其他 Provider 一致的方式保存运行结果与 Provider 会话标识（Kimi `sessionId`）

#### Scenario: 用户取消 Kimi 运行

- **WHEN** Kimi 会话正在执行且用户请求取消
- **THEN** 系统终止该 Kimi 子进程（含子孙进程清理）
- **AND THEN** Workspace 或 Coding Workspace 显示既有的已取消状态而不继续处理后续输出
- **AND THEN** 系统不重复发送 Completed 或 Failed 终态事件

#### Scenario: Kimi 进程异常退出时报失败

- **WHEN** Kimi 子进程在未发送 `session/prompt` 终态响应前异常退出
- **THEN** 系统发送一次 Failed 事件并附进程退出信息

### Requirement: Kimi 支持逐工具审批（Supervised 模式）

系统 SHALL 在用户为 Kimi 选择 `Supervised` 权限模式时，将 ACP `session/new` 的 `permissionMode` 设为 `default`，并在收到 Kimi 的 `session/request_permission` 时映射为既有 `PermissionRequest` 事件，经现有授权桥接（ApprovalBridge）向用户请求批准；用户响应后映射回 ACP `session/request_permission` 响应。用户拒绝时工具不执行且会话继续。默认权限模式为 `Auto`（对应 ACP `auto`），与 Claude Code、Codex 一致。

#### Scenario: Supervised 模式下工具调用触发审批

- **WHEN** Kimi 以 Supervised 模式运行且 Agent 发起工具调用
- **THEN** Kimi 发送 `session/request_permission`
- **AND THEN** 适配器将其映射为既有 `PermissionRequest`，授权桥接向用户展示批准选项（允许一次 / 允许本次会话 / 拒绝）
- **AND THEN** 用户批准后适配器回送 ACP 响应，工具执行

#### Scenario: 用户拒绝审批后会话继续

- **WHEN** 用户对某次 `session/request_permission` 选择拒绝
- **THEN** 工具不执行
- **AND THEN** Kimi 会话继续，可能发起新的工具调用或新的审批请求

#### Scenario: Auto 模式下不触发审批

- **WHEN** Kimi 以 Auto 模式运行
- **THEN** 适配器将 `permissionMode` 设为 `auto`
- **AND THEN** 不产生 `PermissionRequest` 事件

### Requirement: Kimi 复用既有授权与失败边界

系统 SHALL 在 Kimi 启动或运行失败时直接报告失败，不在运行期自动切换到其他 Provider，且第一阶段不实现同 Provider 内部重试（artifact retry / resume retry）。系统 SHALL 将 Kimi 排除在 structured-output repair / review repair 之外（与 Pi 一致）。系统 SHALL 在 task-run 所有入口（HTTP 调度、RoutingProviderAdapter、节点契约、step runner）显式拒绝 Kimi，返回稳定错误，不使用 `unreachable!`。

#### Scenario: Kimi 启动或运行失败直接报告

- **WHEN** Kimi 子进程启动失败或运行中失败
- **THEN** 系统直接报告失败
- **AND THEN** 不切换到其他 Provider，不自动重试同一 Kimi 会话

#### Scenario: task-run 误调度 Kimi 时返回稳定错误

- **WHEN** task-run 链路收到 `ProviderType::KimiCode`
- **THEN** 系统返回明确的 incompatible 错误
- **AND THEN** 不发生 panic（不使用 `unreachable!`）

### Requirement: Kimi 不参与仓库初始化

系统 SHALL 在仓库初始化路径中显式过滤 Kimi（与 Pi 一致），仅 Claude Code 可用于仓库初始化。

#### Scenario: 仓库初始化不展示 Kimi

- **WHEN** 用户进入仓库初始化界面
- **THEN** Provider 选项不包含 Kimi
- **AND THEN** 过滤逻辑以 capability policy 注释说明

### Requirement: Kimi 不产生需求歧义结构化提问

Phase 0 Spike 已实证：ACP 协议下 Kimi 仅有工具审批（`session/request_permission`），无独立的开方式提问/选择方法。因此系统 SHALL 不为 Kimi 产生 `ChoiceRequest` 事件；Kimi 遇到需求歧义时以纯文本提问（`agent_message_chunk`），Aria 以文本 fallback 呈现。系统 SHALL 不为 Kimi 复用 Pi 的 `aria-ask.ts` 提问扩展。

#### Scenario: Kimi 遇到歧义时以文本提问

- **WHEN** Kimi 在生成角色产物时发现需要用户决策的歧义
- **THEN** Kimi 以纯文本（`agent_message_chunk`）提问
- **AND THEN** 系统不产生 `ChoiceRequest` 事件
- **AND THEN** 不加载或调用任何 `aria-ask.ts` 扩展

### Requirement: image-create 支持 Kimi

系统 SHALL 允许 image-create 选择并执行 Kimi（image-create 复用 streaming provider 会话，无需独立图像 API）。

#### Scenario: image-create 可选择 Kimi

- **WHEN** 用户在 image-create 中选择 Provider
- **THEN** Kimi 出现在选项中
- **AND THEN** 选择 Kimi 后 image-create 经 streaming provider 会话执行

#### Scenario: Kimi 不可用时 image-create 禁用 Kimi

- **WHEN** Kimi 健康检查报告不可用（未安装 / 版本过低 / 健康探测失败）
- **THEN** image-create 的 Provider 选项中 Kimi 被禁用
- **AND THEN** 界面说明不可用原因或安装提示
