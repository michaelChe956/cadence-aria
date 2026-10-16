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

Supervised 的 ACP permissionMode SHALL 为 default；普通 session/request_permission 经既有 PermissionRequest 与 ApprovalBridge 映射，用户批准/拒绝回送原生响应。AskUserQuestion SHALL 继续走结构化提问而非普通审批。审批不得代替统一宿主执行合同、解除 author 岗位代码编辑限制或使已作废 Reviewer 报告恢复有效。

#### Scenario: Supervised 模式下工具调用触发审批
- **WHEN** Supervised Kimi 发起非 AskUserQuestion 的普通工具调用
- **THEN** 系统 SHALL 映射 PermissionRequest 并展示既有二元批准选项，批准回 allow_once、拒绝回 reject_once，不新增会话级全批选项；批准的 terminal 使用宿主全开语义，不回到只读 grammar

#### Scenario: 用户拒绝审批后会话继续
- **WHEN** 用户拒绝某次权限请求
- **THEN** 系统 SHALL 不执行该工具但继续会话，允许后续工具/提问，不以全开基线自动越过该拒绝

#### Scenario: 全开不是越过岗位与采信合同
- **WHEN** author 请求代码编辑工具，或 Reviewer 输出已因违规作废
- **THEN** 用户普通工具审批 SHALL 不解除作者岗位限制、不洗白已作废报告；其它读取、terminal、MCP 与 skills 不因 author 身份统一禁用
