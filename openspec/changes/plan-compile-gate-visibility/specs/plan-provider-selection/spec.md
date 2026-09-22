# plan-provider-selection Delta

## Purpose

为 Workbench 的 plan/story/design 创建与生成入口建立确定的 provider 选择和默认应用契约，消除服务端默认与页面加载后补发 provider 之间的竞态，并让用户在生成前知道实际使用的 provider。

## ADDED Requirements

### Requirement: 创建入口携带 provider（REQ-PPS-01）

Workbench 创建 plan、story 或 design 时，系统 SHALL 允许用户选择 provider，并在创建请求中携带用户当前选定值或默认值的快照。创建请求未提供 provider 时可保留既有服务端兼容默认，但正常的用户创建路径 MUST NOT 依赖服务端可用性回退来替换用户已选 provider；provider 不可用时 SHALL 在创建/开始生成前给出可诊断错误或明确可见的重新选择，不得静默换用另一 provider。

#### Scenario: 用户选择随创建请求生效

- **WHEN** 用户在 Workbench 创建入口选择可用 provider 并提交 plan/story/design 创建
- **THEN** 创建请求携带该 provider，创建出的会话以该 provider 作为初始实际 provider，不等待页面加载后再补发选择

#### Scenario: 默认 provider 随创建请求生效

- **WHEN** 用户未在本次表单显式选择 provider，但存在有效用户默认 provider
- **THEN** 创建请求携带该默认 provider，服务端不先按环境默认创建后再依赖客户端纠正

#### Scenario: 选择的 provider 不可用

- **WHEN** 创建时用户选择的 provider 当前不可用
- **THEN** 创建/开始生成被明确阻止并返回可诊断原因或要求重新选择，系统不静默改用 codex、claude_code 或其他 provider

### Requirement: 默认应用覆盖页面形态（REQ-PPS-02）

用户默认 provider SHALL 通过共享默认应用机制同时覆盖 Cockpit 与 legacy workspace 页面；页面加载或重连恢复时应用默认 SHALL 是幂等的，并 SHALL 避免在非 `PrepareContext` 阶段重复发送无效选择。已有 provider 选择与锁定规则保持不变。

#### Scenario: Cockpit 与 legacy 页一致应用默认

- **WHEN** 用户分别从 Cockpit 或 legacy workspace 打开尚未锁定 provider 的新会话
- **THEN** 两种页面均应用相同的有效默认 provider，并通过既有合法 provider-select 通路保存，不因页面形态不同而漂移

#### Scenario: 已锁定 provider 不被默认覆盖

- **WHEN** 会话已持久化或已开始使用 provider，页面重新挂载并读取用户默认
- **THEN** 页面显示已锁定实际 provider，不发送覆盖性默认选择，不改变正在运行的会话

### Requirement: 生成前实际 provider 可见（REQ-PPS-03）

在用户点击「开始生成」或等价启动动作前，Workbench 与 workspace 页面 SHALL 显示将要使用的实际 provider（含用户选择、默认或不可用状态），并在 provider 尚未确定或不可用时阻止不透明启动。

#### Scenario: 开始生成前显示实际 provider

- **WHEN** 会话准备阶段即将开始生成
- **THEN** 用户可在创建/开始入口看到明确的 provider 名称与当前状态，点击启动后使用该 provider

#### Scenario: provider 竞态被阻止

- **WHEN** 页面尚未完成 provider 确定或服务端返回 provider 不可用
- **THEN** 开始生成按钮不可用或启动被 fail-closed 拒绝，并展示选择/可用性原因，不出现“先用服务端默认、再由页面纠正”的隐式窗口
