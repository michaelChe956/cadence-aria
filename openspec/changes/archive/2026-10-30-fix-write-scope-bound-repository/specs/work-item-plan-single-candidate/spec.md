# Spec Delta

## Purpose

多仓（LC 聚合根）plan 会话的单候选机械校验补「写范围 × 被授权仓归属」核对：work item 的 exclusive 写面必须落在本项被授权成员仓的 checkout 内；落在聚合根非成员区或其他成员 checkout 的写面在 generate/evaluate 期即产出稳定错误码 `write_scope_outside_bound_repository` 的 Error finding（preflight 族，经既有机械 ReviewVerdict 回灌修订），不再依赖 reviewer 人工发现或 coder 执行期代偿。

## ADDED Requirements

### Requirement: 写范围必须落在被授权仓 checkout 内（REQ-WSC-10）

多仓（LC 聚合根）plan 会话的候选机械校验 SHALL 逐 work item 核对 `write_policy.exclusive_scopes` 的路径域归属：带固定路径锚点的写范围 SHALL 落在本项被授权成员仓（IR `target_repository_id`）的 checkout 目录之内。越界（聚合根非成员区或其他成员 checkout）SHALL 产出稳定错误码 `write_scope_outside_bound_repository` 的 Error finding（preflight 族，经既有机械 ReviewVerdict 回灌修订）；核对只判归属、SHALL NOT 判文件存在性；无固定锚点的 scope 与空列表 SHALL NOT 触发；前缀匹配 SHALL 复用平台既有 scope 匹配权威。

#### Scenario: 聚合根非成员区写范围被拦

- **WHEN** 多仓会话候选中某 work item 绑定成员仓 busi（checkout `busi/`），其 exclusive 写面声明 `scripts/dev.sh`（聚合根本地文件区，不属于任何成员 checkout）
- **THEN** generate/evaluate 期产出 Error finding，code 为 `write_scope_outside_bound_repository`，消息点名该路径与「聚合根非成员区」归属事实并附修复建议；finding 属 preflight 族，报告保留并经既有机械回灌通道驱动修订

#### Scenario: 其他成员 checkout 的跨成员写范围被拦

- **WHEN** 某 work item 绑定成员仓 busi，其 exclusive 写面声明 `frontend/src/**`，而 `frontend/` 是另一成员仓的 checkout
- **THEN** 产出同码 Error finding，消息点名该写范围实际落在哪个成员 checkout 与本项被授权仓，按既有授权语义（每 WI 恰落一被授权成员仓）判越界并附修复建议

#### Scenario: 被授权 checkout 内新建路径不误伤

- **WHEN** 某 work item 绑定成员仓 busi，其 exclusive 写面声明 `busi/src/new_feature/**`，该路径不存在于 plan 基线树（纯新建写面）
- **THEN** 归属核对放行，不产出本规则 finding（只判归属不判存在）

#### Scenario: 单仓/Manual 会话零回归

- **WHEN** 候选校验上下文未提供成员 checkout 前缀表（legacy 单仓或非 enrolled/Manual 会话）
- **THEN** 本规则不触发，既有校验行为与 finding 集零变化

#### Scenario: 成员映射缺失 fail-closed

- **WHEN** 成员 checkout 前缀表在场，而某 work item 的被授权仓不在表内（成员归属映射缺失/漂移），无法确定其 checkout 归属
- **THEN** 按仓内既有 fail-closed 惯例产出 Error finding 说明「无法核对归属」的理由，SHALL NOT 以跳过吞错或降级放行

#### Scenario: preflight verdict 携带修复动作

- **WHEN** 本规则产出 Error finding 并被适配为机械返修 verdict
- **THEN** finding 的 required_action 与共享修复文案常量逐字同源（REQ-WSC-06 口径一致纪律），指引把写范围改写为被授权成员 checkout 内的聚合根相对路径或移交给拥有该路径的 work item
