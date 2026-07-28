## ADDED Requirements

### Requirement: 交接摘要不再存在

系统 MUST NOT 生成、存储、读取或经接口暴露 work item 交接摘要产物。完成 coding unit 时 MUST NOT 为生成交接摘要调用 provider。

#### Scenario: 完成 unit 不产生交接摘要产物

- **WHEN** 一个 coding unit 完成并发布交接
- **THEN** attempt 目录下 MUST NOT 出现交接摘要文件，且 MUST NOT 发生用于生成交接摘要的 provider 调用

#### Scenario: 完成 unit 不因交接摘要失败

- **WHEN** 一个 coding unit 完成
- **THEN** 完成流程 MUST NOT 因交接摘要缺失、生成失败或字段不全而失败或降级

### Requirement: 交接契约由 HandoffRevision 单一承担

跨 unit 的交接语义 MUST 完全由 `HandoffRevision` 承担。其 `provided_contracts`、`provided_capabilities`、`contract_hash`、`commit_sha` 语义 MUST 保持不变，运行时权威校验 MUST 保持不变。

#### Scenario: 下游仍获得上游契约与能力

- **WHEN** 某 unit 依赖的上游 unit 已完成并发布交接
- **THEN** 下游 MUST 仍能获得上游的 `provided_contracts` 与 `provided_capabilities`，内容与移除前一致

#### Scenario: 运行时权威校验未被削弱

- **WHEN** 某 `HandoffRevision` 的 commit、work item revision 或 unit run 状态与其绑定的 unit run 不一致
- **THEN** 系统 MUST 仍然失败关闭，判定口径与移除前一致

### Requirement: HandoffRevision 不再携带测试与产物清单

`HandoffRevision` MUST NOT 包含 `tests` 与 `artifacts` 字段：二者唯一数据源为被移除的交接摘要。既有持久化记录中含这两个字段时，反序列化 MUST 成功并忽略它们。

#### Scenario: 既有 lineage 记录仍可读取

- **WHEN** 读取一条在本变更前写入、含 `tests` 与 `artifacts` 字段的 `HandoffRevision`
- **THEN** 读取 MUST 成功，契约与能力字段 MUST 完整可用

### Requirement: 交接摘要不再作为验收或前置依据

系统 MUST NOT 以交接摘要的存在或字段完整度作为任何验收、门禁或前置条件。

#### Scenario: group final review 不因摘要缺失判要求修改

- **WHEN** 整组 unit 的产品代码与契约一致，且不存在任何交接摘要产物
- **THEN** group final review MUST NOT 因交接摘要缺失或字段不全而给出要求修改或阻塞结论

#### Scenario: 启动 coding 不因上游缺少摘要引用被拒绝

- **WHEN** 某 work item 的上游依赖已完成，但不存在交接摘要引用
- **THEN** 启动 coding MUST NOT 因缺少交接摘要引用而被拒绝

### Requirement: schema v2 契约体系不受影响

移除范围 MUST NOT 触及 schema v2 契约体系中的 `handoff_contract` 结构与 `handoff_field` 证据类型：二者是 `HandoffRevision` 的来源，与交接摘要无关。

#### Scenario: 契约编译产出不变

- **WHEN** 从 work item 契约编译出 projection 与 `HandoffRevision`
- **THEN** `handoff_contract` 的解析与 `handoff_field` 证据类型 MUST 保持可用，编译产出的契约与能力 MUST 与移除前一致
