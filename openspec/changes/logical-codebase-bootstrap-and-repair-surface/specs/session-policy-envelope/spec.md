# Spec Delta

## MODIFIED Requirements

### Requirement: 两层政策结构（REQ-ENV-01）

逻辑代码库任一真实 provider run（规划、编码、评审或初始化）使用的 policy/target 引用 SHALL 由唯一 LC authority resolver 解析并校验，且 SessionPolicyEnvelope SHALL 冻结 policy_id、revision、digest、target identity 与 provider capability 快照。resolver 无法唯一解析、policy 材料缺失、digest/revision 不一致或目标跨 kind 时，启动 SHALL 在 provider spawn 前 fail-closed，返回可诊断、可操作的核验/准备/重试等待项；不得回落到成员仓路径、项目级历史布局、旧 pointer 或裸 provider input。用户完成允许操作后，系统 SHALL 复用原始登记/编排链继续，不创建第二套 durable operation 状态机。

#### Scenario: 启动逻辑代码库 provider run

- **WHEN** 逻辑代码库流程启动任何真实 provider run（规划/编码/评审/初始化；本 requirement 例外入口除外）
- **THEN** envelope SHALL 由 resolver 从 policy artifact 解析并校验，缺省或不一致时 fail-closed 拒绝启动

#### Scenario: legacy 直连例外启动

- **WHEN** workspace 引擎 author/revision/review 以 legacy 直连启动 provider
- **THEN** 该启动 SHALL 携带经 builder 工厂设置的角色工具策略（REQ-ENV-09），通过 adapter 守卫，并写入 durable 启动审计；不得以裸输入绕过策略

#### Scenario: coding Coder 直连例外启动

- **WHEN** coding Coder（Executor）以 legacy 直连启动 provider
- **THEN** 该启动 SHALL 经 engine builder 工厂构造（Executor 禁带工具策略，REQ-ENV-09；携带策略即拒），通过 adapter 双向守卫，并以既有 execution_event_audit 通道接受按角色适用的等价启动审计（非 durable_tool_policy_audit 分区）；不得以裸输入绕过 builder 工厂

#### Scenario: LC 冷启动生成有效 envelope

- **WHEN** 逻辑代码库完成 authority、manifest/checkout、rules/policy 与成员 index 所需材料并请求 provider run
- **THEN** resolver SHALL 生成包含稳定目标、政策 revision/digest 和 capability 快照的 envelope，provider 仅能以该校验过的 envelope 启动

#### Scenario: policy 或 authority 不一致时拒绝启动

- **WHEN** policy digest 变化、authority 与 target 不一致、规则引用无法解析或 capability 尚未通过实际 gateway 预检
- **THEN** provider SHALL 零启动并返回可审计等待项；用户确认前不得自动重试未知外部副作用、不得写入假 capability、不得从备用路径读取
