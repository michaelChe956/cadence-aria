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

### Requirement: Kimi 权限模式默认 Auto 且支持 Supervised

普通 Workspace 的 Author 与 Reviewer、以及 Coding Workspace 的每个 Provider 角色 SHALL 默认使用 `Auto` 权限模式（对应 ACP `session/new` 的 `permissionMode="auto"`），与 Claude Code、Codex 一致。Kimi 额外支持 `Supervised`（对应 ACP `permissionMode="default"`），用户可为每个适用角色独立切换。Auto 模式下工具调用直接执行；Supervised 模式下经 ApprovalBridge 逐工具审批。该默认值与权限能力与 Pi 区分（Pi 仅 Auto）。

#### Scenario: 默认 Auto 运行 Kimi

- **WHEN** 用户启动一个已选择 Kimi 的角色运行
- **THEN** 该角色以 `Auto` 模式执行（ACP `permissionMode="auto"`）
- **AND THEN** Kimi 工具调用不要求用户逐项确认

#### Scenario: Kimi 可切换 Supervised

- **WHEN** 用户为已选择 Kimi 的角色切换权限模式
- **THEN** 系统提供 `Auto` 与 `Supervised` 两种选项
- **AND THEN** `Supervised` 模式下逐工具审批（见「逐工具审批」Requirement）

### Requirement: Kimi 支持逐工具审批（Supervised 模式）

系统 SHALL 在用户为 Kimi 选择 `Supervised` 权限模式时，将 ACP `session/new` 的 `permissionMode` 设为 `default`，并在收到 Kimi 的 `session/request_permission`（`toolCall.title != "AskUserQuestion"`）时映射为既有 `PermissionRequest` 事件，经现有授权桥接（ApprovalBridge）向用户请求批准；用户响应后映射回 ACP `session/request_permission` 响应。用户拒绝时工具不执行且会话继续。AskUserQuestion 的 request_permission 走「结构化提问」Requirement，不走本审批路径。

#### Scenario: Supervised 模式下工具调用触发审批

- **WHEN** Kimi 以 Supervised 模式运行且 Agent 发起非 AskUserQuestion 的工具调用
- **THEN** Kimi 发送 `session/request_permission`
- **AND THEN** 适配器将其映射为既有 `PermissionRequest`，授权桥接向用户展示二元批准选项（第一阶段不提供「允许本次会话」）
- **AND THEN** 用户批准后适配器回送 ACP `allow_once`，工具执行；拒绝则回 `reject_once`

#### Scenario: 用户拒绝审批后会话继续

- **WHEN** 用户对某次 `session/request_permission` 选择拒绝
- **THEN** 工具不执行
- **AND THEN** Kimi 会话继续，可能发起新的工具调用或新的审批请求

### Requirement: Kimi 支持会话恢复（resume）

系统 SHALL 复用既有 `StreamingProviderInput.resume_provider_session_id` 机制，当传入历史 Kimi sessionId 时，适配器发 ACP `session/load` 续接同一会话（spike 实证 initialize 声明 `loadSession:true` + `sessionCapabilities.resume`），与 Claude Code、Codex、Pi 一致。第一阶段不实现同 Provider 失败重试（artifact retry / resume-stall fresh retry）。

#### Scenario: resume 续接历史 Kimi 会话

- **WHEN** 既有流程请求继续同一 Kimi 会话并传入历史 sessionId
- **THEN** 适配器发 ACP `session/load` 加载该 sessionId
- **AND THEN** Kimi 上下文完整续接

#### Scenario: resume 不等于失败重试

- **WHEN** Kimi 启动或运行失败
- **THEN** 系统直接报告失败，不自动重试同一 Kimi 会话

### Requirement: Kimi 复用既有授权与失败边界

系统 SHALL 在 Kimi 启动或运行失败时直接报告失败，不在运行期自动切换到其他 Provider，且不实现 artifact retry 或 resume-stall fresh retry。对于 reviewer，系统仅在首轮结构化输出包含 recoverable JSON、且错误仅为结束标签/nonce 包装缺陷时，允许 Kimi 在同一会话进行最多一次 structured-output repair；repair 后 JSON 必须与首轮 recoverable JSON 逐值相等并重新通过原始 nonce 严格校验。Pi SHALL 继续排除 review repair。任何不满足该条件、repair 失败、JSON 变化或仍然格式错误的情况 SHALL fail-closed，进入人工确认，且不得将未可信的可读文本作为 author 自动返修依据。系统 SHALL 在 task-run 所有入口（HTTP 调度、RoutingProviderAdapter、节点契约、step runner）显式拒绝 Kimi，返回稳定错误，不使用 `unreachable!`。

#### Scenario: Kimi 启动或运行失败直接报告

- **WHEN** Kimi 子进程启动失败或运行中失败
- **THEN** 系统直接报告失败
- **AND THEN** 不切换到其他 Provider，不自动重试同一 Kimi 会话

#### Scenario: Kimi reviewer 仅修复完整 JSON 的结构化包装

- **WHEN** Kimi reviewer 输出了完整可解析的审核 JSON，但 `<ARIA_STRUCTURED_OUTPUT>` 结束标签缺少 nonce、缺失结束标签或 nonce 不匹配
- **THEN** 系统至多发起一次同会话 repair
- **AND THEN** repair 输出必须使用匹配原始 nonce 的完整 sentinel，且 JSON 值与首轮 recoverable JSON 完全一致
- **AND THEN** 通过后正常路由审核 verdict

#### Scenario: Kimi repair 不能改变审核业务内容

- **WHEN** Kimi reviewer 的 repair 输出改变 JSON、仍缺少有效 sentinel，或 repair 运行失败
- **THEN** 系统不得接受输出，也不得再次 repair
- **AND THEN** 系统保持 fail-closed 的人工确认状态，不把不可信 reviewer 可读文本作为 author 返修目标

#### Scenario: 人工请求返修必须提供目标

- **WHEN** 审核因无可信结构化 findings 而进入人工确认，用户选择请求修改但没有提供非空修改说明
- **THEN** 系统保持人工确认状态并返回可操作错误
- **AND THEN** 系统不得启动 author revision

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

### Requirement: Kimi 支持 AskUserQuestion 结构化提问

Phase 0 Spike 实证：Kimi 有 `AskUserQuestion` 工具（与 Claude Code 同名同 schema），并把提问选项统一收敛到 ACP `session/request_permission`。系统 SHALL 识别 `session/request_permission` 中 `toolCall.title == "AskUserQuestion"` 并映射为既有 `ProviderEvent::ChoiceRequest`（source=`AskUserQuestion`，options 来自 request_permission.options，`allow_free_text=true`，`allow_multiple=false`）；其他 title 映射为 `PermissionRequest`。系统 SHALL 不为 Kimi 复用 Pi 的 `aria-ask.ts` 扩展。**Auto 与 Supervised 模式下 AskUserQuestion 均产生 ChoiceRequest**（提问是用户输入请求，Auto 不藏提问）；Auto 仅对普通工具不产生 PermissionRequest。

#### Scenario: 用户选择提问选项

- **WHEN** Kimi 调用 AskUserQuestion 工具发起提问，用户选择某选项
- **THEN** 适配器把所选 optionId 以 ACP `Selected(optionId)` 回传
- **AND THEN** Kimi 在同一会话轮内继续

#### Scenario: 多问题逐题串行呈现

- **WHEN** Kimi 需问多个问题
- **THEN** Kimi 逐题依次发多个 `session/request_permission`，每次单题单选
- **AND THEN** 每题映射为单个 `ChoiceRequest`（`allow_multiple=false`），用户答完一题后 Kimi 自行发下一题

#### Scenario: 选项都不合适时用户自由输入（free_text 优先）

- **WHEN** AskUserQuestion 提问的选项都不符合用户意图，用户在自由文本框输入自己的回答
- **THEN** 适配器以 ACP `Cancelled` 关闭原 request_permission（避免 Kimi 挂起）
- **AND THEN** 适配器忽略 selected_option_ids（不拼接，free_text 优先，对齐 Claude）
- **AND THEN** 适配器不发中间 Failed/Completed，内部发第二个 `session/prompt` 注入用户 free_text
- **AND THEN** 第二轮 `session/prompt` result 为唯一终态，Kimi 上下文完整续接

#### Scenario: UI 呈现选项与自由文本框并存

- **WHEN** Kimi 发起 AskUserQuestion 提问
- **THEN** 卡片同时展示选项按钮与始终可编辑的自由文本框
- **AND THEN** 用户可选择选项或填自由文本（方案 D）

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
