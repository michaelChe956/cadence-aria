## ADDED Requirements

### Requirement: 逻辑 issue 的 selection 初始写入

多仓 project 创建 issue 时服务端自动写入 all_members 代码库选区，保证 Story/Design/WorkItemPlan 生成可达。

#### Scenario: 自动 all_members
- **WHEN** 多仓 project 创建 issue（请求体与单仓一致，repository_id 为 primary repository，须属于 manifest active member）
- **THEN** 服务端原子化创建 issue 并写入覆盖全部 active member 的 selection；selection 写入失败时删除刚创建的 issue 并返回 422 issue_selection_write_failed（删除亦失败时记录 orphan 告警并返回 500）；不存在只建 issue 不建 selection 的中间态

#### Scenario: primary repository 语义
- **WHEN** 多仓 issue 携带 repository_id
- **THEN** 该字段语义为 primary repository（UI 归属与兼容投影），不影响 selection 成员范围（恒为 all_members）

### Requirement: 多仓 Design change_order 强制

多仓 Design 涉及多个 involved repository 时必须提供 change_order，缺失即 blocker。

#### Scenario: 缺 change_order 被拒
- **WHEN** 多仓 Design 涉及多个 involved repository 且未提供 change_order
- **THEN** 生成被拒并产生 blocker change_order_required_for_logical_codebase，不得进入 compile；模型层不再保留"缺失不强制"的相反语义
