# claude-code-structured-interaction Specification

## Purpose
定义 aria 通过 claude code headless（`-p --input-format stream-json`）会话进行结构化交互与权限回调的协议行为：工具注册、权限模式映射、以及 AskUserQuestion 结果的所有权。本 capability 吸收自 fix-claude-code-ask-user-question-headless 提案（三提交 3164f6a7/e367a84e/20493b65 已于 08-21 落地），本 change 补齐验证收尾与契约化。

## Requirements

### Requirement: 始终注册 stdio 权限回调

无论 aria 权限策略是 Auto 还是 Supervised，`ClaudeCodeProvider::build_args` 生成的 claude code 启动参数 MUST 包含且仅包含一次 `--permission-prompt-tool=stdio`——该 flag 是宿主交互能力注册，与 aria 权限策略解耦。

#### Scenario: Auto 模式启动参数包含 stdio 回调

- **WHEN** 以 `ProviderPermissionMode::Auto` 构造 claude code 启动参数
- **THEN** 参数包含 `--permission-prompt-tool=stdio`，且总数为一

#### Scenario: Supervised 模式启动参数包含 stdio 回调

- **WHEN** 以 `ProviderPermissionMode::Supervised` 构造 claude code 启动参数
- **THEN** 参数包含 `--permission-prompt-tool=stdio`，且总数为一

### Requirement: 权限模式映射为合法 wire 值

aria 向 claude code 发送的 `set_permission_mode` MUST 是 claude code 接受的合法值；`Auto` 与 `Supervised` 都 SHALL 映射为 `"default"`，MUST NOT 发送 `"supervised"`。

#### Scenario: Auto 映射为 default

- **WHEN** aria 权限模式为 Auto
- **THEN** 初始握手中 `set_permission_mode.mode` 为 `"default"`

#### Scenario: Supervised 映射为 default

- **WHEN** aria 权限模式为 Supervised
- **THEN** 初始握手中 `set_permission_mode.mode` 为 `"default"`

### Requirement: 普通工具权限仍由 aria 决策

映射为 `default` 后，claude code SHALL 将需要权限判断的普通工具请求通过 `control_request(can_use_tool)` 交给 aria；Auto 模式 SHALL 由 `ApprovalBridge` 自动批准，Supervised 模式 SHALL 等待用户批准。

#### Scenario: Auto 模式自动批准普通工具

- **WHEN** 收到普通工具（非 AskUserQuestion）的 `control_request(can_use_tool)` 且 aria 权限模式为 Auto
- **THEN** `ApprovalBridge` 返回批准，回写 `control_response` behavior 为 allow，并记录 Auto approval 审计事件

#### Scenario: Supervised 模式等待用户批准普通工具

- **WHEN** 收到普通工具的 `control_request(can_use_tool)` 且 aria 权限模式为 Supervised
- **THEN** 发出面向用户的权限请求，等待用户响应后再回写 `control_response`

### Requirement: AskUserQuestion 始终等待用户

无论 aria 权限模式是 Auto 还是 Supervised，AskUserQuestion 都 MUST NOT 被自动批准，MUST 通过 `ApprovalBridge::request_choice` 等待用户回答。

#### Scenario: Auto 模式下 AskUserQuestion 仍等待用户

- **WHEN** 收到 AskUserQuestion 的 `control_request(can_use_tool)` 且 aria 权限模式为 Auto
- **THEN** 发出 `ChoiceRequest` 并阻塞等待用户回答，不因 Auto 自动批准而返回

#### Scenario: Supervised 模式下 AskUserQuestion 等待用户

- **WHEN** 收到 AskUserQuestion 的 `control_request(can_use_tool)` 且 aria 权限模式为 Supervised
- **THEN** 发出 `ChoiceRequest` 并阻塞等待用户回答

### Requirement: AskUserQuestion 结果所有权属于 control_request

assistant `tool_use(AskUserQuestion)` 仅是输出事件，aria MUST NOT 向 stdin 注入 `user.tool_result`；唯一回答通道 SHALL 是带 `request_id` 的 `control_request(can_use_tool, AskUserQuestion)`——aria 回写 `control_response` 后由 claude code 生成原生 tool_result。

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

### Requirement: 修复落地持有新鲜验证证据（REQ-CCI-06）

claude headless 修复的工程面（三提交）与本 capability 契约 SHALL 在现行 build 上持有新鲜验证证据：全量质量门禁结果留档，且真实 claude CLI smoke 记录 tool_use→control_request→control_response→tool_result 完整事件序列。

#### Scenario: 真实 CLI smoke 事件序列

- **WHEN** 以真实 claude code CLI（2.1.237+）在 headless 模式运行触发 AskUserQuestion 的会话
- **THEN** smoke 记录 tool_use→control_request→control_response→tool_result 完整事件序列作为证据留档

#### Scenario: 全量门禁留档

- **WHEN** 本 change 收口
- **THEN** `cargo fmt`/`cargo clippy`/`cargo test --workspace` 全量结果留档（吸收自被吸收提案的整体验证任务）
