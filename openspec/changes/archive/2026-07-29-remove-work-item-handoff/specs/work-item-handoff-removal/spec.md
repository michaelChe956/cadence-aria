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

`HandoffRevision` MUST NOT 包含 `tests` 与 `artifacts` 字段：二者唯一数据源为被移除的交接摘要。

#### Scenario: 发布交接不产出测试与产物清单

- **WHEN** 某 unit 完成并发布 `HandoffRevision`
- **THEN** 该 `HandoffRevision` MUST 只承载契约与能力语义，MUST NOT 包含测试清单与产物清单

### Requirement: 组完成的写入范围门禁必须保持有效

组完成门禁 MUST 继续校验每个已完成 unit 的实际变更文件是否越过其 `write_policy` 的 `forbidden_scopes` 与 `exclusive_scopes`。该校验 MUST NOT 因交接摘要与 `artifacts` 字段移除而失效。

#### Scenario: 越界写入仍被拒绝

- **WHEN** 某已完成 unit 的实际变更包含其 `forbidden_scopes` 匹配的文件，且不存在任何交接摘要产物
- **THEN** 组完成门禁 MUST 拒绝，并给出写入范围越界错误

#### Scenario: 门禁数据源来自 git 事实

- **WHEN** 组完成门禁校验写入范围
- **THEN** 其变更文件清单 MUST 来自 git 事实，MUST NOT 依赖任何交接摘要字段
- **AND** 判定口径 MUST 与移除前一致

#### Scenario: 合规写入不被误拒

- **WHEN** 各已完成 unit 的实际变更均在其 `exclusive_scopes` 内且不触碰 `forbidden_scopes`
- **THEN** 组完成门禁 MUST 放行

### Requirement: 评审不再以交接摘要承诺为审查对象

reviewer 提示词 MUST NOT 要求以交接摘要的自然语言承诺或其字段完整度作为审查对象。

#### Scenario: 评审协议不引用已移除字段

- **WHEN** 平台构造 Code Review 或 GroupFinalReview 的 reviewer 提示词
- **THEN** 提示词 MUST NOT 要求确认交接摘要承诺是否闭环
- **AND** MUST NOT 点名已移除的交接摘要字段
- **AND** 跨 unit 交接的审查对象 MUST 为 `HandoffRevision` 的契约与能力语义

#### Scenario: 评审否决权限边界不变

- **WHEN** 改写后的提示词生效
- **THEN** reviewer 的 verdict 取值口径与除交接摘要外的否决依据 MUST 保持不变

### Requirement: work item 完成 commit 记录不受影响

work item 的完成 commit 记录 MUST 在交接摘要移除后继续写入与暴露。

#### Scenario: 完成 commit 仍被写入并可读

- **WHEN** 某 work item 完成
- **THEN** 其完成 commit MUST 被持久化，并 MUST 仍可经接口读取用于依赖交接引用展示

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
