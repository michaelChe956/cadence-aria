# group-final-review-triage Specification

## Purpose
为 internal PR review 与 group final review 阶段需要人工介入的流程决策（送回 Coder 返修、重试验证、人工分诊、运维阻塞）落地阻塞门禁，把 attempt 置为阻塞状态而非静默退出；四类决策使用互不相同的原因码，同一次评审结论至多落地一个门禁，门禁提供重试评审、人工继续、终止三个可操作动作，且原因码判定不含不可达分支。

## Requirements

### Requirement: internal PR review 的人工路由决策必须落地阻塞门禁

internal PR review 与 group final review 阶段，当评审结论推出需要人工介入的流程决策时，系统 MUST 落地阻塞门禁并把 attempt 置为阻塞状态。系统 MUST NOT 在该阶段静默退出流程。

需要人工介入的决策为：送回 Coder 返修、重试验证、人工分诊、运维阻塞。

#### Scenario: 要求修改结论落地门禁

- **WHEN** group final review 给出要求修改的结论，且流程决策为送回 Coder 返修
- **THEN** 系统 MUST 落地阻塞门禁，attempt 状态 MUST 为阻塞，MUST NOT 停留在运行中

#### Scenario: 验证证据不完整落地门禁

- **WHEN** internal PR review 的流程决策为重试验证
- **THEN** 系统 MUST 落地阻塞门禁，attempt 状态 MUST 为阻塞

#### Scenario: 人工分诊与运维阻塞落地门禁

- **WHEN** internal PR review 的流程决策为人工分诊或运维阻塞
- **THEN** 系统 MUST 落地阻塞门禁，attempt 状态 MUST 为阻塞

#### Scenario: 通过结论不落地门禁

- **WHEN** internal PR review 给出通过结论且流程决策为通过后继续
- **THEN** MUST NOT 落地阻塞门禁，完成路径 MUST 与本变更前一致

#### Scenario: 计划修订决策不落地门禁

- **WHEN** internal PR review 的流程决策为启动计划修订
- **THEN** MUST NOT 落地阻塞门禁，计划修订编排 MUST 与本变更前一致

### Requirement: 停机原因以互不相同的原因码区分

四个需要人工介入的决策 MUST 使用互不相同的门禁原因码。

#### Scenario: 四个决策原因码互不相同

- **WHEN** 分别以送回 Coder 返修、重试验证、人工分诊、运维阻塞四个决策落地门禁
- **THEN** 四次落地的原因码 MUST 互不相同

### Requirement: 同一次评审结论只落地一个阻塞门禁

系统 MUST NOT 为同一次评审结论落地多个阻塞门禁。门禁落地判定 MUST 由流程决策驱动，且各判定条件 MUST 互斥。

#### Scenario: 每次结论至多一个门禁

- **WHEN** internal PR review 完成任意一次评审并推出流程决策
- **THEN** 落地的阻塞门禁数量 MUST 不超过一个

#### Scenario: 阻塞结论与分诊决策不重复落地

- **WHEN** 评审结论为阻塞，且该结论推出的流程决策属于需要人工介入的四类之一
- **THEN** 系统 MUST 只落地一个阻塞门禁，MUST NOT 因结论与决策各自落地而产生两个

### Requirement: 分诊门禁提供可操作动作

分诊门禁 MUST 提供重试评审、人工继续、终止三个动作，使用户在任何停机原因下都能继续或结束流程。

#### Scenario: 门禁动作可用

- **WHEN** internal PR review 的分诊门禁向用户呈现
- **THEN** 动作集合 MUST 含重试评审、人工继续、终止

#### Scenario: 分诊动作不触发计划修订

- **WHEN** 用户在分诊门禁上执行任意动作
- **THEN** MUST NOT 唤起计划修订流程

### Requirement: 门禁原因码判定不含不可达分支

系统 MUST NOT 保留因前置条件永不满足而不可达的门禁原因码分支。判定条件 MUST 与实际可达的结论与决策组合一致。

#### Scenario: 无不可达分支

- **WHEN** 审查 internal PR review 的门禁原因码判定
- **THEN** 每个分支 MUST 在至少一种可达的结论与决策组合下生效

#### Scenario: 阻塞结论加可操作发现的组合仍可达

- **WHEN** 评审结论为阻塞且存在可操作发现，据此推出送回 Coder 返修决策
- **THEN** 该组合对应的原因码分支 MUST 保持生效，MUST NOT 被当作不可达分支移除

### Requirement: 新 Group attempt 的最终审查不得调用 Group Reviewer Provider

当 Group attempt 的每个 Work Item 已完成其独立 Coder 与 Code Reviewer 流程后，系统 MUST NOT 启动 Internal Group Reviewer、shard、reduction 或任何替代的自动语义审查 Provider 调用。系统 MUST 直接生成组就绪检查并进入人工最终确认流程。

#### Scenario: 全部 Work Item 独立审查完成

- **WHEN** Group attempt 的全部 Work Item 已完成且各自拥有最新的通过 Code Review 记录
- **THEN** 系统 MUST 生成组就绪检查，MUST NOT 创建 Internal Group Reviewer、shard 或 reduction Provider 运行

#### Scenario: Group Final 不产生 Provider 输出无效门禁

- **WHEN** 新 Group attempt 到达最终审查步骤
- **THEN** 系统 MUST NOT 因不存在的 group Provider 输出创建 `reduction_output_invalid`、shard 输出无效或对应重试门禁

### Requirement: 组就绪检查必须提供客观的人工审查证据

组就绪检查 MUST 从权威执行记录生成每个 Work Item 的 UnitRun `start_commit`、terminal completion commit、完整提交区间及有序提交引用、最新独立 Code Review 的结论/发现/摘要与原始输出证据引用、resolved handoff 和绑定的 Work Item Plan revision。所有文件与 diff 引用 MUST 由各 execution 真实的 `start_commit..completion_commit` 区间聚合派生，MUST NOT 只使用末尾 commit 的父提交 diff，MUST NOT 把两个 execution 区间之间由人工产生的提交归属给当前 Work Item。检查 MUST 验证全部 Unit 已完成、必需 completion commit 与独立审查记录存在、handoff 与依赖解析一致，以及 attempt 的计划 binding 与活跃 revision 一致。若检查存在经用户批准的验证处理记录，检查 MUST 将其与对应 check 一并展示。

该检查 MUST NOT 生成代码语义 finding、重新解释 Code Review 结论、执行新的文件写入范围判断或自动启动 Coder rework 与 Plan Repair。`start_commit` 与 completion commit 相同 MUST 表示空观察区间，而非把该起始提交相对父提交的改动归属给当前 Work Item。

#### Scenario: 组就绪检查完整

- **WHEN** 全部 Work Item 的权威 UnitRun、completion commit、独立审查、handoff 和计划 binding 均一致
- **THEN** 系统 MUST 持久化包含这些证据的完整组就绪检查，并允许进入人工最终确认

#### Scenario: Coder rework 的全部提交在人工审查中可见

- **WHEN** 一个 Work Item 在同一 UnitRun 内先由 Coder 创建提交、后经 Code Review rework 创建新的 terminal commit
- **THEN** 就绪检查和人工 Final 面板 MUST 展示从该 UnitRun `start_commit` 至 terminal commit 的完整提交区间、相关 diff/evidence 与独立审查结论，不得只展示返修提交

#### Scenario: Coder 未产生新的可观察提交

- **WHEN** 一个 Work Item 的 terminal completion commit 等于其 UnitRun `start_commit`
- **THEN** 就绪检查 MUST 将其展示为无可观察 Git 增量，并保留该 Coder 的原始输出引用；系统 MUST NOT 将起始提交的父提交 diff 显示为当前 Work Item 的证据

#### Scenario: 带人工 WIP 的零提交重跑可确认

- **WHEN** 某 Work Item 先有一次失败 execution，用户随后在工作树中留下人工提交，重试 execution 零提交完成且其余证据完整
- **THEN** 就绪检查 MUST 把该重试 execution 显示为空区间，人工提交不计入该 Work Item，人工 Final Confirm 可执行

#### Scenario: 越界提交仍被拒绝

- **WHEN** 某 execution 真实区间内包含超出当前 Work Item `write_policy` 的提交
- **THEN** 既有终态写入范围检查 MUST 仍在 Final Confirm 时拒绝，不因区间派生方式改变而放行

#### Scenario: 缺少客观完成证据

- **WHEN** 任一 Work Item 缺少 completion commit、最新独立审查记录、resolved handoff 或与活跃计划不一致的 binding
- **THEN** 系统 MUST 持久化具体的不一致诊断，MUST NOT 允许人工最终确认完成该 attempt

### Requirement: 人工最终确认是新 Group attempt 的唯一最终语义决策

完整的组就绪检查完成后，系统 MUST 向用户展示全部 Work Item 的客观证据并等待明确的人工 Final Confirm。只有该确认才可以使 Group attempt 进入完成终态。人工最终确认 MUST NOT 生成新的 Coder rework、Plan Repair 或自动 Provider 调用。

#### Scenario: 用户确认完整组

- **WHEN** 用户已查看完整组就绪检查并执行明确的 Final Confirm
- **THEN** 系统 MUST 完成该 Group attempt，且 MUST 不启动任何 Group Reviewer Provider

#### Scenario: 人工确认不绕过不完整检查

- **WHEN** 组就绪检查包含未解决的不一致诊断
- **THEN** 系统 MUST 拒绝 Final Confirm 并保持 attempt 未完成

### Requirement: 人工 Final Confirm 保留既有终态完整性检查

完整组就绪检查只决定用户是否具备作出最终语义决定的证据，不得取消既有的 completion binding、基于 UnitRun 完整提交区间的写入范围检查或 shared worktree 清洁性检查。用户执行 Final Confirm 时，系统 MUST 继续执行这些既有终态检查；其失败 MUST 以既有确定性诊断或人工清理动作呈现，MUST NOT 转换为 Group Reviewer finding、Coder rework 或 Plan Repair。

#### Scenario: 范围外残留不被自动处理

- **WHEN** Coder 报告共享 worktree 中存在不属于当前 Work Item `write_policy` 的未跟踪或修改内容，且组就绪检查其余证据完整
- **THEN** 系统 MUST 不自动暂存、提交或删除该残留；若 Final Confirm 命中既有 shared-worktree-clean 检查，系统 MUST 保留其人工清理语义而不是创建新的 Group Review 门禁

### Requirement: 历史 Group Final 产物保持可读且不得触发新 Provider 调用

已持久化的 shard、reduction 或 InternalPrReview 产物 MUST 保持可读，以供历史 attempt 审计。恢复未终态的历史 Group attempt 时，系统 MUST 使用权威记录生成组就绪检查并转入人工最终确认；系统 MUST NOT 为恢复而启动新的 shard、reduction 或 Internal Group Reviewer 调用。权威身份不一致时 MUST 失败关闭并显示诊断。

#### Scenario: 恢复含 reduction 产物的历史 attempt

- **WHEN** 历史 Group attempt 已保存 reduction 原始输出或审查产物但尚未完成
- **THEN** 系统 MUST 保持其可读取，并以当前权威记录生成组就绪检查，MUST NOT 重新调用 reduction Provider

#### Scenario: 历史身份不一致

- **WHEN** 恢复历史 Group attempt 时其 UnitRun、handoff 或 plan binding 无法唯一校验
- **THEN** 系统 MUST 失败关闭并提供身份诊断，MUST NOT 推断或重建 Group Provider 审查结论

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
