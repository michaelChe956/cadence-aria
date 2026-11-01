## Purpose

让日常 Workspace 与 Coding Workspace 将 Pi 作为与 Claude Code、Codex 并列的真实 Provider 使用，同时保留一致的可用性、授权、失败报告与审计体验。

## ADDED Requirements

### Requirement: Pi 在活跃 Provider 工作流中可发现且可选择

系统 SHALL 将 Pi 作为真实 Provider 的健康状态条目公开，并在 Pi 可用或当前已被配置时，允许用户在 Story、Design、Work Item Workspace 的 Author / Reviewer 以及 Coding Workspace 的 Coder、Code Reviewer、Internal Reviewer 中选择 Pi。不可用的 Pi SHALL 提供与其他 Provider 一致的不可用原因或安装提示。

#### Scenario: Pi 可用时出现在角色选择器中

- **WHEN** Pi CLI 已安装、可验证且用户打开任一支持 Provider 选择的 Workspace 配置
- **THEN** 相应角色的 Provider 选择器显示 Pi
- **AND THEN** 用户可以保存 Pi 作为该角色的 Provider

#### Scenario: Pi 不可用时保留已配置值并说明原因

- **WHEN** 某角色已配置 Pi 且后续健康检查报告 Pi 不可用
- **THEN** 系统保留该角色的已配置 Provider 值
- **AND THEN** 界面说明 Pi 当前不可执行的原因或安装提示

### Requirement: Pi 以流式会话执行并支持控制操作

系统 SHALL 让 Pi 在普通 Workspace 与 Coding Workspace 中以流式会话执行，向既有前端协议发送文本、工具调用、完成、失败与会话标识事件。系统 MUST 支持取消活动 Pi 会话，并在既有流程请求继续同一 Provider 会话时恢复 Pi 会话。

#### Scenario: Pi 执行生成角色产物

- **WHEN** 用户启动一个已选择 Pi 的 Author、Reviewer、Coder、Code Reviewer 或 Internal Reviewer 运行
- **THEN** 系统将 Pi 输出流转发到现有 Workspace 或 Coding Workspace 会话
- **AND THEN** 系统以与其他 Provider 一致的方式保存运行结果与 Provider 会话标识

#### Scenario: 用户取消 Pi 运行

- **WHEN** Pi 会话正在执行且用户请求取消
- **THEN** 系统终止该 Pi 会话
- **AND THEN** Workspace 或 Coding Workspace 显示既有的已取消状态而不继续处理后续输出

### Requirement: Pi 支持结构化提问

系统 SHALL 通过 Aria 自带的 `aria-ask.ts` 扩展让 Pi 在遇到必须由用户决策的需求歧义时，以与 Claude Code 的 `AskUserQuestion` 和 Codex 的 `requestUserInput` 一致的方式向用户提问。扩展注册一个 `ask_user` 自定义工具（不拦截任何工具调用），LLM 需要澄清时主动调用它；工具内 `ctx.ui.select()` 在 RPC 模式下经 `extension_ui_request/response` 往返，Aria 适配器将其映射为既有 `ProviderEvent::ChoiceRequest`（source 为 `ProviderChoice`），用户回答后映射回 `extension_ui_response(value)`，使答案在同一 Pi 进程内接续，上下文完整。

#### Scenario: Pi 遇到需求歧义时弹出选择卡片

- **WHEN** Pi 在生成角色产物时发现必须由用户决定的需求/范围/验收歧义
- **THEN** Pi 通过已注册的 `ask_user` 工具调用 `ctx.ui.select()`
- **AND THEN** 适配器发出的 `ChoiceRequest` 的 `source` 为 `ProviderChoice`
- **AND THEN** 选择卡片保留扩展请求的题目/标题及所有选项（含顺序与显示值）
- **AND THEN** 用户 `ChoiceResponse` 的所选 `value` 被原样封装为 `extension_ui_response(value)` 发回**同一** Pi RPC 进程
- **AND THEN** Pi 在收到该响应后继续同一会话的生成

#### Scenario: 提问扩展不拦截工具调用

- **WHEN** Pi 执行工具调用（如 `read`、`bash`、`write`）
- **THEN** 工具调用直接执行（Auto 模式），不弹出授权请求
- **AND THEN** 运行事件与工具活动照常记录

### Requirement: Provider 权限模式默认为 Auto，Pi 仅支持 Auto

普通 Workspace 的 Author 与 Reviewer、以及 Coding Workspace 的每个 Provider 角色 SHALL 默认使用 `Auto` 权限模式。`Auto` 模式 MUST 允许 Provider 直接执行其工具调用且保留运行事件。Claude Code 与 Codex 保留既有的 `Supervised` 逐工具确认能力，用户可为每个适用角色独立切换。Pi 因不提供逐工具批准机制，SHALL 仅以 `Auto` 模式运行，不向用户提供 Pi 的 `Supervised` 选项。

#### Scenario: 默认 Auto 运行 Pi 工具调用

- **WHEN** 用户启动一个已选择 Pi 的角色运行
- **THEN** 该角色以 `Auto` 模式执行
- **AND THEN** Pi 工具调用不要求用户逐项确认

#### Scenario: Pi 不提供 Supervised 选项

- **WHEN** 用户查看已选择 Pi 的角色的权限模式设置
- **THEN** 系统不向该角色提供 `Supervised` 选项
- **AND THEN** 该角色保持 `Auto` 模式

#### Scenario: Claude Code 与 Codex 保留 Supervised

- **WHEN** 用户为已选择 Claude Code 或 Codex 的角色切换权限模式
- **THEN** 系统提供 `Auto` 与 `Supervised` 两种选项
- **AND THEN** `Supervised` 模式下沿用该 Provider 既有的逐工具确认能力

### Requirement: 所选 Provider 的失败直接报告且不切换

当用户所选的真实 Provider（包括 Pi）在启动或运行期间失败时，系统 SHALL 将该失败及其原因报告为当前运行的失败状态。系统 MUST NOT 在运行期自动切换、重放或重试到其他 Provider。

#### Scenario: 所选 Provider 启动失败

- **WHEN** 用户所选 Provider 在启动前健康检查、命令启动、认证或连接期间失败
- **THEN** 系统报告当前运行失败及失败原因
- **AND THEN** 系统不自动选择或启动其他 Provider

#### Scenario: 所选 Provider 运行期间失败

- **WHEN** 用户所选 Provider 已开始运行后报告错误或异常终止
- **THEN** 系统报告当前运行失败及失败原因
- **AND THEN** 系统不自动重放该运行或切换到其他 Provider

### Requirement: Pi 不扩大仓库初始化和 Task Runner 的 Provider 范围

添加代码库与仓库初始化 SHALL 继续只使用 Claude Code，且不向其用户界面或执行输入暴露 Pi。Task Runner、其 CLI/API 和旧的 Fake Provider Workspace Runner MUST 保持现有 Provider 行为，且不得因本变更调度 Pi。

#### Scenario: 添加代码库不显示 Pi

- **WHEN** 用户打开添加代码库或查看仓库初始化状态
- **THEN** Provider 选项与初始化执行不包含 Pi
- **AND THEN** 初始化继续使用 Claude Code

#### Scenario: Task Runner HTTP 调度入口拒绝 Pi

- **WHEN** 用户通过现有 Task Runner HTTP 确认或调度请求指定 `provider_type` 为 `pi`
- **THEN** Task Runner 在调用 Provider adapter 前以 machine-readable 的“不支持 Provider”错误拒绝该请求，并在错误中标识 Pi
- **AND THEN** Task Runner CLI 继续不暴露 Pi 作为 Provider 选择
- **AND THEN** 其既有 Claude Code、Codex 或 Fake 行为保持不变

> 注：`ProviderType` 会增加 `Pi` 变体以维持流式域 `StreamingProviderInput` 的类型一致性，但 Task Runner 的调度入口、Provider router、兼容性矩阵与节点契约不匹配、不路由 Pi。
