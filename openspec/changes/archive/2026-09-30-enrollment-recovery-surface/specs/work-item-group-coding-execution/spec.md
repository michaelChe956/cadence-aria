# Spec Delta

## ADDED Requirements

### Requirement: 换代后的 compile child 精确绑定（REQ-C1-CHILD-01）

compile child/session 的选择 SHALL 同时匹配当前 plan 身份、plan revision、work item revision 与当前 enrollment binding version/身份；仅按 work item 类型、实体编号或“最新 session”猜测 SHALL 不被接受。用户明确换代/重新绑定后，系统 SHALL 创建属于新绑定版本的新 child；旧 child 与旧 binding SHALL 保持只读可查，不清空、不覆盖、不迁移其历史事实。

#### Scenario: 同 Work Item 编号跨代创建新 child

- **WHEN** 同一 Work Item 编号在旧代失败后，用户明确选择新 plan/session/source/target/provider 并提交换代
- **THEN** compile 为新绑定版本创建新 child 并成功匹配当前 plan/revision，旧 child 仍可按旧绑定查询且不被改写

#### Scenario: 旧 child 不冒充当前代

- **WHEN** 当前代 compile 查找时仅存在旧 plan/revision/binding 的 child
- **THEN** 系统拒绝复用旧 child，返回需要绑定/重建的等待操作，不清理旧 binding，不把旧成功回执写入当前代

#### Scenario: 同代重放不创建第二 child

- **WHEN** 当前绑定版本、plan/revision/work item 身份均相同的 compile 命令以相同或重复 command_id 重放
- **THEN** 系统命中同一 child 和 durable 结果，不创建第二 child，不修改旧历史
