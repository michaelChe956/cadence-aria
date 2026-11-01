# 规范增量：多仓 coding 跨仓最终评审

<!-- 来源：cadence/designs/2026-10-22_技术方案_coding跨仓最终评审_v1.1.md（commit ded59f08）。
     本 delta 全部为 ADDED：不覆盖本 capability 既有条款（internal PR review 门禁、组就绪检查、
     人工最终确认、历史 Group Final 产物），也不触碰其它在途 change（本 capability 当前无在途修订方）。
     全部新增行为只对多仓库逻辑代码库（logical_codebase_id == Some(_)）生效；
     单仓库代码库的每仓/组评审、阶段机、窗口与零锁恢复维持既有条款。 -->

## ADDED Requirements

### Requirement: 多仓形态停用每仓内部评审与组终审（REQ-GFR-CR-01）

多仓库逻辑代码库下，系统 MUST NOT 为每个仓各跑一次 internal review，MUST NOT 执行 GroupFinalReview，MUST NOT 启动 InternalReviewer provider 做阶段性收口。

同时系统 MUST 保留：code reviewer（per work item）及其返工（送回 Coder）与转人工分诊门；组形态（同一仓多个 work item 聚合为一个 group attempt）；组就绪视图与人工最终确认。

单仓库代码库 MUST 保留 per-attempt internal review、GroupFinalReview、阶段机、窗口与零锁恢复、`group_final_review_blocked` / `internal_review_*` 原因码与既有事件，行为逐项不变。

#### Scenario: 多仓不启动内部评审 provider

- **WHEN** 多仓 issue 的 3 个仓各自完成 code review 并进入收口
- **THEN** InternalReviewer provider 启动次数 MUST 为 0，MUST NOT 产生 GroupFinalReview 结论或对应阻塞门

#### Scenario: 多仓 code reviewer 仍是门

- **WHEN** 多仓某 work item 的 code review 结论为要求修改
- **THEN** 系统 MUST 仍走既有返工或转人工分诊路径，MUST NOT 因停用内部评审而放行该 work item

#### Scenario: 多仓保留组形态但无组终审

- **WHEN** 同一仓的多个 work item 聚合为 group attempt 且全部完成
- **THEN** 系统 MUST 生成组就绪视图并进入人工最终确认，MUST NOT 启动任何组终审 provider

#### Scenario: 单仓链路逐项不变

- **WHEN** 单仓 issue 的全部 work item 完成并进入 internal review 与 FinalConfirm
- **THEN** 事件序列、状态迁移、门控、provider 启动次数与评审窗记录 MUST 与改造前一致

### Requirement: 存量内部评审阶段 attempt 的形态分派（REQ-GFR-CR-02）

对磁盘上 `stage = internal_pr_review` 的存量 attempt，系统 MUST 按代码库形态分派：多仓库逻辑代码库 MUST NOT 启动 provider 评审，MUST 走组就绪到人工最终确认的兼容路径；单仓库代码库 MUST 按既有路径完成 internal review。

`CodingExecutionStage::InternalPrReview` 成员 MUST 保留，MUST NOT 删除或重命名其既有持久化字符串。

#### Scenario: 多仓存量 attempt 不启动 provider

- **WHEN** 多仓 issue 存在 `stage = internal_pr_review` 的存量 attempt 被推进
- **THEN** 系统 MUST 走 readiness 到 FinalConfirm 的路径，provider 启动次数 MUST 为 0

#### Scenario: 单仓存量 attempt 走原路径

- **WHEN** 单仓存量 attempt 处于 `stage = internal_pr_review`
- **THEN** 系统 MUST 按改造前路径完成 internal review，事件与门控 MUST 不变

#### Scenario: 阶段枚举字符串不变

- **WHEN** 读取或写入任何 attempt 的 `stage` 字段
- **THEN** 既有 7 个成员的序列化字符串 MUST 与改造前逐字一致，相对顺序 MUST 不变

### Requirement: issue 级跨仓最终评审恰好一次（REQ-GFR-CR-03）

多仓库逻辑代码库下，issue 下每个 work item 的最新 attempt 均已完成（完成态，或等待人工最终确认态）后，系统 MUST 为整个 issue 触发恰好一次跨仓最终评审。

该评审记录 MUST 按 issue 唯一；重复进入触发路径 MUST NOT 重复评审，MUST 复用已存在的记录。该评审 MUST NOT 作为 attempt 阶段推进，MUST NOT 修改任何 attempt 的 `stage`。

单仓库代码库 MUST NOT 触发该评审，MUST NOT 产生跨仓评审工件目录或跨仓待办提醒。

#### Scenario: 多仓全部完成后恰好一次

- **WHEN** 多仓 issue 的 3 个仓（3 个 attempt）全部完成
- **THEN** 系统 MUST 产生恰好一份 issue 级跨仓评审记录

#### Scenario: 重复触发幂等

- **WHEN** 收口钩子被重复进入两次
- **THEN** MUST NOT 新增第二份评审记录，MUST 复用已存在的记录

#### Scenario: 不新增 attempt 阶段

- **WHEN** 跨仓评审被触发
- **THEN** 任何 attempt 的 `stage` MUST 保持不变，MUST NOT 出现专属的 attempt 阶段值

#### Scenario: 单仓不产生跨仓评审

- **WHEN** 单仓 issue 的 work item 全部完成
- **THEN** MUST NOT 触发跨仓评审、MUST NOT 产生跨仓评审工件目录、MUST NOT 新增跨仓待办提醒；缺 `internal_reviewer` 的既有停等门 MUST 保持不变

### Requirement: 跨仓评审材料与 provider 选择（REQ-GFR-CR-04）

跨仓最终评审材料 MUST 为逐仓 diff 分段：每个已完成 attempt 一段，段内至少含仓名、work item 列表、attempt、分支与 diff 区间。分段顺序 MUST 稳定可复现，diff 口径 MUST 与 code reviewer 所见同源。

材料超过预算时 MUST 按仓顺序截断，MUST 在材料中显式标注被截断的仓，MUST NOT 静默省略。

provider MUST 取该 attempt 快照的 `internal_reviewer` 配置；其缺失时 MUST 回退同一快照的 `code_reviewer` 配置；两者皆缺时 MUST 不启动 provider 并把记录状态置为 `Skipped`。

#### Scenario: 逐仓分段与稳定排序

- **WHEN** 3 个仓的 attempt 全部完成并触发跨仓评审
- **THEN** 材料 MUST 含 3 段逐仓 diff，顺序 MUST 按仓稳定排序

#### Scenario: provider 缺失回退 code reviewer

- **WHEN** 角色 provider 快照缺 `internal_reviewer` 但有 `code_reviewer`
- **THEN** 跨仓评审 MUST 正常执行（状态为完成，provider 启动次数为 1），MUST NOT 判为 `Skipped`

#### Scenario: 两者皆缺则不评

- **WHEN** 角色 provider 快照同时缺 `internal_reviewer` 与 `code_reviewer`
- **THEN** MUST NOT 启动 provider，记录状态 MUST 为 `Skipped`

#### Scenario: 材料超预算显式标注

- **WHEN** 逐仓 diff 合计超过材料预算
- **THEN** 系统 MUST 按仓顺序截断并在材料中显式标注被截断的仓，MUST NOT 静默省略整仓

### Requirement: 跨仓最终评审的 advisory 红线（REQ-GFR-CR-05）

跨仓最终评审结论 MUST 仅为建议性。其结论 MUST NOT 进入任何交付门、采信门、自动返工、人工分诊或阻塞门路径；MUST NOT 阻止用户执行最终确认；MUST NOT 修改任何 attempt 的 status 或 stage；MUST NOT 创建待处理门；MUST NOT 开启评审窗或执行零锁恢复；MUST NOT 把 findings 送入计划缺陷路由。

评审 provider 失败、超时或输出不可解析时，系统 MUST 记记录状态为失败并保留原因，MUST NOT 阻断交付、MUST NOT 自动重试。

#### Scenario: 要求修改仍可确认

- **WHEN** 跨仓评审结论为要求修改，用户执行最终确认
- **THEN** 确认 MUST 成功，attempt MUST 进入完成态，issue MUST NOT 被阻断，待处理门 MUST 保持为空

#### Scenario: 阻塞结论不分流

- **WHEN** 跨仓评审结论为阻塞
- **THEN** 系统 MUST NOT 产生送回 Coder 返修、转人工分诊、计划修订或运维阻塞任一路径的门与指令

#### Scenario: provider 失败不阻断

- **WHEN** 跨仓评审 provider 失败或超时
- **THEN** 记录状态 MUST 为失败且含原因，用户 MUST 仍可完成最终确认，MUST NOT 自动重试

#### Scenario: 无采信与窗口副作用

- **WHEN** 跨仓评审完成
- **THEN** MUST NOT 产出采信结论或评审窗标识，MUST NOT 写入评审窗存储，MUST NOT 参与后续交付采信判定

### Requirement: 跨仓最终评审的已知风险确认留痕（REQ-GFR-CR-06）

当跨仓评审结论为要求修改或阻塞、用户忽略提醒仍然执行最终确认时，系统 MUST 持久化一条「已知风险，仍然确认」事实，内容 MUST 为：确认时间、跨仓评审记录 id、结论、findings 摘要。

该事实 MUST NOT 记录确认人身份，MUST NOT 记录确认通道。同一评审记录 id 与同一 attempt MUST 至多留痕一条。结论为通过时 MUST NOT 留痕，也 MUST NOT 投影跨仓提醒。

#### Scenario: 非通过结论确认后留痕

- **WHEN** 跨仓评审结论为要求修改且用户完成最终确认
- **THEN** 系统 MUST 追加一条留痕记录，含时间、评审记录 id、结论与 findings 摘要，且该记录 MUST NOT 含确认人或通道字段

#### Scenario: 留痕幂等

- **WHEN** 同一 attempt 对同一评审记录再次被确认
- **THEN** 系统 MUST NOT 追加第二条留痕

#### Scenario: 通过结论不留痕

- **WHEN** 跨仓评审结论为通过
- **THEN** 系统 MUST NOT 留痕，MUST NOT 投影跨仓待办提醒，确认卡片 MUST NOT 显示跨仓警告

#### Scenario: 单仓无留痕面

- **WHEN** 单仓 issue 完成最终确认
- **THEN** MUST NOT 产生任何跨仓留痕记录

### Requirement: 跨仓 findings 来源标记与既有取值兼容（REQ-GFR-CR-07）

跨仓最终评审产出的 findings，其来源阶段 MUST 为新取值 `issue_cross_repo_review`。该取值 MUST 在 provider 输出的严格来源字符串表中登记，未登记字符串 MUST 仍按硬失败处理。provider 输出省略来源阶段时，跨仓评审 MUST 兜底为同一新取值。

既有来源阶段的序列化字符串 MUST 不变；缺省兜底 MUST 仍为 `code_review`。新取值 MUST NOT 出现在任何 attempt 的 `stage` 字段。

#### Scenario: 跨仓 finding 落库为新取值

- **WHEN** 跨仓评审产出 findings
- **THEN** 每条 finding 的来源阶段 MUST 为 `issue_cross_repo_review`

#### Scenario: 省略来源时兜底

- **WHEN** provider 输出省略 finding 的来源阶段
- **THEN** 落库值 MUST 为 `issue_cross_repo_review`

#### Scenario: 既有取值与兜底不变

- **WHEN** 读取存量的 `code_review` / `internal_pr_review` findings，或缺省来源阶段的存量 findings
- **THEN** 读出的取值 MUST 与改造前一致，缺省仍为 `code_review`

#### Scenario: 新取值不进 attempt 阶段

- **WHEN** 任意 attempt 被持久化或读取
- **THEN** 其 `stage` MUST NOT 取 `issue_cross_repo_review`，既有阶段语义与相对顺序 MUST 不变
