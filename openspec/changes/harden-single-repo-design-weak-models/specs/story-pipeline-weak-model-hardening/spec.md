## ADDED Requirements

### Requirement: 示例载荷不可经 envelope repair 复活（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

作为「few-shot 示例（防照抄）」要求在 repair 层的强化：Workspace reviewer 结构化输出的 envelope repair 路径 SHALL 将携带判例/示例指纹载荷的恢复值判为不可修复并降级人工 triage；该判据 SHALL 同时覆盖 JSON nonce 错配与缺失 JSON nonce 两类错误码。恒不携带恢复载荷的错误码分支 SHALL 从可修复枚举中移除并以测试锁定其不可达。repair prompt SHALL 显式排除示例 nonce，且回灌原始输出时 SHALL 使用剥离 sentinel 后的文本。此约束作用于全部 workspace reviewer（Story、Design、WorkItem、WorkItemPlan），不改变 coding 与 image 链路行为。

#### Scenario: 照抄示例载荷转人工而非返修
- **WHEN** reviewer 输出的业务载荷携带示例指纹且 nonce 层错误属于可修复类
- **THEN** 系统不进入 envelope repair、不以新 nonce 复活该载荷，而是降级人工 triage

#### Scenario: 正常封装修复保留
- **WHEN** reviewer 输出仅封装层损坏（如缺失 JSON nonce）且载荷不含示例指纹
- **THEN** 既有 envelope-only repair 成功路径保持不变
