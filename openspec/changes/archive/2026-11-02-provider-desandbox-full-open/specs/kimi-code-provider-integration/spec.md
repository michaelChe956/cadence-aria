# 规范增量：Kimi 集成面配套

<!-- 来源：openspec/specs/kimi-code-provider-integration/spec.md:30-83。此处修改 ACP 会话、权限、逐工具审批三条；真实 OS/grammar/fs 三条位于 kimi-acp-client-services:80-150，并已由对应 delta 完整修改。 -->

## MODIFIED Requirements

### Requirement: Kimi 以 ACP 流式会话执行并支持控制操作

系统 SHALL 在普通 Workspace、Coding 与 image-create 复用既有 JSON-RPC over stdio ACP 流式会话、文本/工具/完成/失败/会话 ID 事件及取消能力，终态仍由 session/prompt stopReason 判定。Kimi 所有角色的 ACP terminal/fs 子执行 SHALL 消费与主会话一致的受信宿主执行合同，不遗留 planning/reviewer 沙箱或角色恒拒旁路；author 岗位规则及 Reviewer B 档采信独立生效。

#### Scenario: Kimi 执行生成角色产物
- **WHEN** 用户启动 Kimi Author、Reviewer、Coder、Code Reviewer、适用历史 Internal Reviewer 或 image-create
- **THEN** 系统 SHALL 转发输出流并按既有格式保存结果与原生 sessionId；ACP 子执行采用同一冻结执行基线，不把该能力要求用于重新启用已取消的 Group Reviewer

#### Scenario: 用户取消 Kimi 运行
- **WHEN** 用户取消活动 Kimi 会话
- **THEN** 系统 SHALL 终止 Kimi 与本轮登记的子执行资源，显示既有取消状态、不继续处理迟到输出、不重复 Completed/Failed；Reviewer 先收口窗口恢复与对账再释放写通道

#### Scenario: Kimi 进程异常退出时报失败
- **WHEN** session/prompt 终态响应前 Kimi 异常退出
- **THEN** 系统 SHALL 发送一次含真实退出信息的 Failed，保留现场证据与适用恢复，不用 fallback 或假成功掩盖失败

### Requirement: Kimi 权限模式默认 Auto 且支持 Supervised

普通 Workspace 的 Author/Reviewer 与 Coding 各角色 SHALL 默认 Auto，对应 ACP auto；仍支持独立切换 Supervised，对应 default。Auto 直接执行普通工具，Supervised 经 ApprovalBridge；二者仅决定审批，不选择 OS 隔离/只读 grammar，不扩张 author 岗位禁用。Kimi 与其它三家采用相同宿主能力下限，Pi 仍仅 Auto，AskUserQuestion 始终等待用户。

#### Scenario: 默认 Auto 运行 Kimi
- **WHEN** 用户启动已选择 Kimi 的角色
- **THEN** 该角色 SHALL 默认以 Auto 执行普通工具，不逐项确认、不探测 bwrap 作为必要前提；author 编辑代码文件工具仍不可用，Reviewer 必须满足窗口合同

#### Scenario: Kimi 可切换 Supervised
- **WHEN** 用户切换 Kimi 角色权限模式
- **THEN** 系统 SHALL 提供 Auto/Supervised，后者逐工具审批；不得要求切 Supervised 才能绕过旧 sandbox unavailable，也不得自动换权限模式

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

#### Scenario: 全开不是越过岗位与采信合同
- **WHEN** author 请求代码编辑工具，或 Reviewer 输出已因违规作废
- **THEN** 用户普通工具审批 SHALL 不解除作者岗位限制、不洗白已作废报告；其它读取、terminal、MCP 与 skills 不因 author 身份统一禁用
