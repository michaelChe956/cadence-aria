# claude-code-structured-interaction Delta

## Purpose

定义 aria 通过 claude code headless（`-p --input-format stream-json`）会话进行结构化交互与权限回调的协议行为：工具注册、权限模式映射、以及 AskUserQuestion 结果的所有权。

## ADDED Requirements

### Requirement: 始终注册 stdio 权限回调

无论 aria 权限策略是 Auto 还是 Supervised，`ClaudeCodeProvider::build_args` 生成的 claude code 启动参数都必须包含且仅包含一次 `--permission-prompt-tool=stdio`。

#### Scenario: Auto 模式启动参数包含 stdio 回调
- **WHEN** 以 `ProviderPermissionMode::Auto` 构造 claude code 启动参数
- **THEN** 参数包含 `--permission-prompt-tool=stdio`，且总数为一

#### Scenario: Supervised 模式启动参数包含 stdio 回调
- **WHEN** 以 `ProviderPermissionMode::Supervised` 构造 claude code 启动参数
- **THEN** 参数包含 `--permission-prompt-tool=stdio`，且总数为一

### Requirement: 权限模式映射为合法 wire 值

aria 向 claude code 发送的 `set_permission_mode` 必须是 claude code 接受的合法值；`Auto` 与 `Supervised` 都映射为 `"default"`，不得发送 `"supervised"`。

#### Scenario: Auto 映射为 default
- **WHEN** aria 权限模式为 Auto
- **THEN** 初始握手中 `set_permission_mode.mode` 为 `"default"`

#### Scenario: Supervised 映射为 default
- **WHEN** aria 权限模式为 Supervised
- **THEN** 初始握手中 `set_permission_mode.mode` 为 `"default"`

### Requirement: 普通工具权限仍由 aria 决策

映射为 `default` 后，claude code 将需要权限判断的普通工具请求通过 `control_request(can_use_tool)` 交给 aria；Auto 模式由 `ApprovalBridge` 自动批准，Supervised 模式等待用户批准。

#### Scenario: Auto 模式自动批准普通工具
- **WHEN** 收到普通工具（非 AskUserQuestion）的 `control_request(can_use_tool)` 且 aria 权限模式为 Auto
- **THEN** `ApprovalBridge` 返回批准，回写 `control_response` behavior 为 allow，并记录 Auto approval 审计事件

#### Scenario: Supervised 模式等待用户批准普通工具
- **WHEN** 收到普通工具的 `control_request(can_use_tool)` 且 aria 权限模式为 Supervised
- **THEN** 发出面向用户的权限请求，等待用户响应后再回写 `control_response`

### Requirement: AskUserQuestion 始终等待用户

无论 aria 权限模式是 Auto 还是 Supervised，AskUserQuestion 都不得被自动批准，必须通过 `ApprovalBridge::request_choice` 等待用户回答。

#### Scenario: Auto 模式下 AskUserQuestion 仍等待用户
- **WHEN** 收到 AskUserQuestion 的 `control_request(can_use_tool)` 且 aria 权限模式为 Auto
- **THEN** 发出 `ChoiceRequest` 并阻塞等待用户回答，不因 Auto 自动批准而返回

#### Scenario: Supervised 模式下 AskUserQuestion 等待用户
- **WHEN** 收到 AskUserQuestion 的 `control_request(can_use_tool)` 且 aria 权限模式为 Supervised
- **THEN** 发出 `ChoiceRequest` 并阻塞等待用户回答

### Requirement: AskUserQuestion 结果所有权属于 control_request

assistant `tool_use(AskUserQuestion)` 仅是输出事件，不得由 aria 向 stdin 注入 `user.tool_result`；唯一回答通道是带 `request_id` 的 `control_request(can_use_tool, AskUserQuestion)`，aria 回写 `control_response` 后由 claude code 生成原生 tool_result。

#### Scenario: 收到 control_request 后提问并回写
- **WHEN** 收到 AskUserQuestion 的 `control_request(can_use_tool)`
- **THEN** aria 发出 `ChoiceRequest` 等待用户，回答后回写 `control_response`，且不向 stdin 写入任何 `tool_result`

#### Scenario: 原生 tool_result 消费缓存
- **WHEN** claude code 输出 AskUserQuestion 的原生 `user.tool_result` 且为非错误
- **THEN** aria 消费对应 `tool_use_id` 的缓存，不重复提问，不产生第二个 tool_result

#### Scenario: 原生 tool_result 为错误时报协议错误
- **WHEN** claude code 输出 AskUserQuestion 的原生 `user.tool_result` 且 `is_error` 为真
- **THEN** aria 发出 AskUserQuestion 协议错误并终止该 run

#### Scenario: 无 control_request 的 tool_use 视为协议不兼容
- **WHEN** 收到 assistant `tool_use(AskUserQuestion)` 后始终未收到对应 `control_request`
- **THEN** aria 不注入手工 tool_result，按协议错误处理
