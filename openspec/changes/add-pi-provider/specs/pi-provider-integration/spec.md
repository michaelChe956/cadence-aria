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

### Requirement: Provider 权限模式默认为 Auto 且可按角色监督

普通 Workspace 的 Author 与 Reviewer、以及 Coding Workspace 的每个 Provider 角色 SHALL 默认使用 `Auto` 权限模式。`Auto` 模式 MUST 允许 Provider 直接执行其工具调用且保留运行事件。用户 SHALL 能为每个适用角色独立切换至 `Supervised`；在该模式下，Pi 的每次工具调用 MUST 等待用户允许或拒绝。

#### Scenario: 默认 Auto 运行 Pi 工具调用

- **WHEN** 用户未更改某角色的权限模式并启动 Pi 运行
- **THEN** 该角色以 `Auto` 模式执行
- **AND THEN** Pi 工具调用不要求用户逐项确认

#### Scenario: Supervised 模式等待用户决定

- **WHEN** 用户将某个 Pi 角色切换为 `Supervised` 且 Pi 发起工具调用
- **THEN** 系统向页面发送工具调用的授权请求
- **AND THEN** Pi 在收到用户允许或拒绝前不执行该调用

#### Scenario: 用户拒绝受监督工具调用

- **WHEN** 页面拒绝一个待处理的 Pi 工具调用
- **THEN** 系统将拒绝结果返回给 Pi
- **AND THEN** 系统记录该授权决定而不把该调用作为已执行处理

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

#### Scenario: Task Runner 保持冻结

- **WHEN** 用户调用现有 Task Runner CLI 或 HTTP API
- **THEN** Task Runner 不将 Pi 作为可选 Provider
- **AND THEN** 其既有 Claude Code、Codex 或 Fake 行为保持不变
