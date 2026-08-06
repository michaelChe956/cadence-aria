# Design: Scalable Group Final Review

## 术语

- 分片（shard / leaf）：一次分片语义审查，最多 4 个 Work Item，三个词等价，正文统一用"分片"。
- 归约（final / reduction）：最终全局审查调用，正文统一用"归约"。
- 快照（snapshot）：组级审查材料快照 `GroupReviewMaterialSnapshot`。

## 架构总览

组级审查改为三段式：Rust 侧确定性编译、分片语义审查、全局归约。

```text
Authoritative Bindings / ReviewRequest / Unit Review Conclusion Snapshots / Git 与 Handoff 事实
                                    │
                                    ▼
                    Group Review Material Compiler（Rust）
              确定性检查 + 全局关系图 + 按级别选择变更片段
                                    │
                     ┌──────────────┴──────────────┐
                     ▼                             ▼
          Deterministic Partitioner          Global Compact Ledger
          每片最多 4 项，完全确定               全局单位摘要与跨片边
                     │                             │
                     ▼                             │
              分片语义审查（Provider）               │
                     │                             │
                     └────── 分片结论 ─────────────┐│
                                                  ▼▼
                                     归约审查（Provider）
                                                   │
                                                   ▼
                              既有 Group Finding Validator 与 Flow Decision
                                                   │
                                                   ▼
                                            InternalPrReview
```

分片输入有界：单个分片 Prompt 不随 Work Item 总数增长。归约输入在首期 20 项容量内有界（其中全局单位账目与分片数线性相关，该线性项在容量内受预算约束；超过容量按容量门禁失败关闭，而不是放宽预算）。

| Work Item 数 | 分片调用（基准） | 归约调用 | 合计调用（基准） |
|---:|---:|---:|---:|
| 2 | 1 | 1 | 2 |
| 5 | 2 | 1 | 3 |
| 10 | 3 | 1 | 4 |
| 20 | ≥5 | 1 | ≥6 |

基准分片数为 ceil(N/4)；预算压力可产生更多、更小的分片（见分片策略）。

## 职责边界

### Rust 侧承担确定性判断

以下检查具有确定性结论，必须在调用 Provider 前完成，其结果作为候选发现或明确门禁：

1. 权威 Binding 与 Handoff 完整性：Run 完成状态、Work Item Revision、Projection Bundle 标识与版本、已解析 Handoff Revision 一致性。
2. 契约与能力精确匹配：必填契约缺失、必填能力缺失、多提供者歧义、消费方来源未声明、孤立产出、跨片契约边。
3. 写入范围与归属：变更路径命中禁止范围、允许与禁止范围交集、同一文件被多个 Work Item 修改、变更片段归属不唯一。
4. Commit 与证据一致性：审查请求 Commit 可达且包含各项完成 Commit、最终变更基线与头部正确、必需证据引用归属正确且字段完整。
5. Requirement 与验收标准聚合：缺失、重复声明、Revision 不一致、已确认 Requirement 未被覆盖。
6. Finding 可路由性预校验：Reason Code、允许路由、目标种类与契约引用的合法集合。

### Provider 承担语义判断

1. 多个 Work Item 合并修改同一文件后的语义冲突。
2. 契约名称或能力相近但实现消费语义不一致。
3. 多 Work Item 对同一 Requirement 的覆盖是否名义重复而实质遗漏。
4. 最终变更、证据状态与交付叙事是否自相矛盾。
5. 影响范围、PR 描述与提交信息建议的生成。
6. 对 Rust 标注的可疑跨片关系与变更片段做语义判断。

## 材料模型

### 材料快照

组级审查启动前生成不可变快照，字段为：

- schema_version、attempt_id、review_request_id；
- base_branch、final_commit；
- authoritative_binding_digest（权威 Binding 集合的规范化摘要）；
- unit_records（单位记录集合，见下）；
- global_graph（契约图、范围图、Commit 关系、Requirement 聚合）；
- diff_index（变更文件完整索引）；
- deterministic_findings（确定性检查产出的候选发现）；
- partition_result（分片结果与跨片边集合）；
- routing_authority_index（按 reason_code、目标 Work Item Revision、允许路由与契约引用建立的规范性路由权限索引）；
- content_hash。

快照采用稳定排序与规范化序列化：所有集合按字典序排序，序列化输出不含时间戳、随机数或环境相关字段；content_hash 为规范化字节的 SHA-256。重试必须复用同一快照。以下事实变化时生成新快照并使旧快照标记 superseded：Work Item 返修产生新的完成 Commit、Work Item Revision 变化、Handoff Revision 变化、审查请求 Commit 变化、权威 Binding 变化。

### 单位记录与全局账目

分片使用的单位记录（UnitCrossReviewRecord）只保留组级语义必需内容：身份标识（unit_id、unit_run_id、logical_work_item_id、work_item_revision_id、completion_commit）、依赖关系、写入范围摘要、契约接口摘要与证据状态摘要。它不复制完整 Reviewer Projection，也不得内嵌 reason_code、路由目标或完整路由权限；这些内容只存在于快照级 routing_authority_index。该拆分保持既有单条 1,200-byte 持久化上限，不以提高该上限换取材料容量。

归约使用的全局账目行（GlobalUnitLedgerRow）进一步折叠为：身份摘要、所属分片、Commit 状态、证据状态、共享范围标记、未决接口计数。

### 路由权限索引与 Prompt 投影

材料编译器从权威 Binding 和既有目标校验规则一次性生成 routing_authority_index。每个条目至少包含 reason_code、允许的目标 Work Item Revision、允许的路由种类以及必需的契约引用；索引与其他快照字段一起稳定排序、规范化序列化并纳入 content_hash。它是 Provider 所见路由权限的唯一来源，不能由 UnitCrossReviewRecord、分片报告或自然语言推断补全。

分片 Prompt 只投影与该分片 Unit 或其跨片边相关的权限条目；归约 Prompt 必须投影同一快照的完整权限索引，因为归约可以把最终发现路由给任一参与 Work Item。二者均把权限索引计入完整 Prompt 的字节预算，且不得静默裁剪 reason_code、目标 Revision 或契约引用。若所需权限缺失，或完整 Prompt 超过硬上限，编译在调用 Provider 前失败关闭。

Provider 输出解析后，分片结果在持久化前必须以其分片权限投影校验；归约结果在持久化、分诊或路由前必须以完整权限索引校验。任何不存在的 reason_code、不允许的目标、路由种类或契约引用均为无效输出，不能通过猜测、改写为普通 finding 或选择首个 Work Item 修复。

### 单位审查结论身份快照

既有单项审查报告记录缺少 Work Item 身份字段，不能作为组级聚合的身份来源。新增独立记录 UnitReviewConclusionSnapshot，字段为：attempt_id、unit_id、unit_run_id、logical_work_item_id、work_item_revision_id、code_review_report_id、verdict、finding_digest、evidence_refs、diff_refs、raw_report_hash。

该记录在单项审查成功持久化的同一流程中生成，写入必须幂等：同一 unit_run 重复写入产生相同内容。若报告已持久化但身份快照写入失败，组级材料编译必须以身份缺失原因失败关闭，不得以部分数据继续。

缺少该记录的历史数据不得按顺序或轮次推断身份，必须以身份缺失原因失败关闭。

### 分片与归约产物

分片结论（GroupReviewShardReport）持久化字段：id、attempt_id、snapshot_hash、ordered_unit_run_ids、partition_rationale、verdict、findings、unresolved_obligations、selected_diff_refs、raw_provider_output_refs、role_run_ids。

归约结论（GroupReviewReductionReport）持久化字段：id、attempt_id、snapshot_hash、shard_report_ids、verdict、findings、impact_scope、pr_description、commit_message_suggestion、provenance。

最终用户可见结论继续使用既有 InternalPrReview。

## 确定性分片策略

分片完全确定：同一输入必须产生同一分片结果，不依赖权重打分、随机数或并发顺序。

第一步，构建亲和图。边分三类：

- Handoff 依赖边：Unit B 的 input_contract 消费 Unit A 的 output_contract；
- 共享文件边：两个 Unit 的完成 diff 修改同一文件；
- 契约边界边：两个 Unit 引用同一 contract_id（不论生产或消费方向）。

order index 取自权威 Binding 解析出的 Unit 顺序；order index 相同时以 unit_id 字典序作为唯一 tie-break。

第二步，对亲和图求连通分量，按分量内最小 (order index, unit_id) 排序。

第三步，装箱。按排序依次处理每个分量：分量整体能装入当前分片（当前大小加分量大小不超过 4）则放入；否则开启新分片；分量本身超过 4 个 Work Item 时按 (order index, unit_id) 稳定切成连续的不超过 4 的组，被切断的边进入跨片边集合。

第四步，分片内按 (order index, unit_id) 排序。

第五步，确定性预算再分片。对每个分片构建材料并度量字节；超过质量目标的分片按 (order index, unit_id) 顺序切成满足质量目标的最少数量的连续子分片，再分片切断的边进入跨片边集合。该步骤重复直到所有分片满足质量目标，或任一子分片已只含一个 Work Item。单个 Work Item 仍超过质量目标时进入材料溢出门禁。

容量验收：20 个 Work Item 至少产生 5 个分片；每个分片不超过 4 个；预算再分片可产生更多、更小的分片，且相同输入必须产生相同结果。

所有跨片边、被切断边必须进入全局跨片关系集合并在归约阶段审查。

## 变更片段选择

不得把完整变更或固定长度前缀直接放入 Prompt。

变更文件索引始终完整提供：路径、增删行数、归属 Unit Run、shared 标记、ambiguous 标记、forbidden_scope_hit 标记。

片段正文按五级分类从高到低选取：

- A：命中某 Unit 禁止写入范围的片段；
- B：归属歧义（片段无法唯一归到某个 Unit）；
- C：共享文件（不少于两个 Unit 修改）；
- D：契约相关文件（被某 Unit 的 output_contract 或 input_contract 声明包含）；
- E：单 Unit 独占文件。

一个文件同时命中多级时归入最高级别，顺序为 A 高于 B 高于 C 高于 D 高于 E。owner 定义为其完成 diff 唯一包含该文件的 Unit Run，无法唯一归属时标记为归属歧义。敏感信息定义为共享脱敏能力识别的凭据、私钥与令牌模式。

选取范围：

- 分片：选取与本片 Unit 相关的 A、B、C、D 级片段；E 级只提供文件头部（路径与统计）；
- 归约：选取集合为 A 级命中、B 级命中、owner 属于不同分片的 C 级文件、owner 属于不同分片的 D 级文件；E 级永不进入归约。

没有对应变更文件的被切断 Handoff 依赖边或契约边界边，以全局跨片关系图条目提供给归约阶段，不因缺少可映射文件而省略。

同级内部按文件路径字典序稳定选取，同文件内按 diff 原始顺序；预算耗尽时按 UTF-8 字符边界截断并标注 truncated。每个被选片段记录原始 hunk hash 与未展示片段数量。

所有片段在进入 Prompt 前必须经过敏感信息脱敏。

## 预算与门禁

度量口径：以实际发送给 Provider 的完整 Prompt 的 UTF-8 字节数度量，1 KiB 等于 1024 bytes。

| 项目 | 取值 | 定性 |
|---|---:|---|
| 单次 Prompt 质量目标 | 28 KiB | 可配置，配置值必须小于硬上限 |
| 单次 Prompt 硬上限 | 30 KiB | 安全不变式；配置只能收紧，不能放宽 |
| 分片最大 Work Item 数 | 4 | 硬契约 |
| 分片结论最大发现数 | 8 | 硬契约 |
| 归约结论最大发现数 | 16 | 硬契约 |
| 分片并发上限 | 2 | 可配置默认值 |

每次构建 Prompt 后记录分段字节：fixed_protocol、identity、unit_records、routing_authority、evidence_digest、graph、diff、retry_diagnostic_reserve、total。

行为规则：不超过质量目标时正常发送；介于质量目标与硬上限之间时发送并记录预算告警；超过硬上限时禁止调用 Provider，持久化包含各分段字节的溢出诊断。单个字段超限时不得静默截断身份与契约标识，应进一步分片或进入溢出门禁。

## 容量边界

首期支持上限为单组 20 个 Work Item。超过上限时，在调用任何 Provider 前落地 capacity_exceeded 阻塞门禁，记录实际数量与支持上限；不得回退单次全量审查，不得提高预算上限，不得把容量超限与材料溢出混用同一原因码。更大规模由后续独立变更引入树形归约。

## 失败、重试与并发

### 失败分类

- 传输失败：非用户意图的 Provider 调用失败，包括超时、意外中断、adapter 错误。处理：直接重试该环节（全新运行），不执行结论转写补救。每环节最大重试次数为可配置默认值，耗尽后进入该环节的输出无效门禁。
- 用户主动取消：停止该环节，不自动重试，保留已产生的原始输出引用。
- 完成但无法解析：Provider 正常完成但输出无法解析为结论 JSON。处理：先执行一次结论转写补救，失败再进入输出无效门禁。

两类失败使用互不相同的运行失败码，门禁与重试动作分开。

运行失败码与门禁原因码分层：运行失败码记录在环节运行与产物上，区分传输失败耗尽与补救无效等具体成因；门禁原因码区分门禁种类。同一门禁种类可对应多个运行失败码，门禁同时记录两者。

### 结论转写补救的保真校验

补救仅适用于"完成但无法解析"。补救输入只含原始输出与结论契约，不得重新包含完整分片或归约材料，并禁止重新审查或新增发现。

为使保真可机械验证，分片与归约的结论 Prompt 必须要求输出包含机器可提取的判定标记行（值为 approve、request_changes 或 blocked）。补救输出必须通过以下机械校验，任一失败即进入输出无效门禁：

1. 判定必须等于从原始输出提取的判定标记值；原始输出缺少标记时补救无效，不得由补救推断判定；
2. verdict 不得为 approve；
3. 每条发现的描述文本必须是原始输出的子串，确保可逐条溯源；
4. 发现数量不超过环节上限（分片 8、归约 16）；
5. 发现的证据引用必须是该 attempt 已有引用的子集；
6. 发现的目标与契约引用通过既有组级权威目标校验；
7. 原始输出与补救输出的哈希均持久化，供事后比对。

补救 Prompt（含原始输出、结论契约与指令）的完整 UTF-8 字节数上限为 16 KiB；超过该上限时不截断，直接进入输出无效门禁。

残留风险：上述规则可防止判定被改写与发现被凭空新增，但无法机械检测原始输出中存在的发现被补救省略。缓解手段为双哈希留证与原始输出保留，供事后人工比对。

### 快照并发与晚到结果

- 输入事实未变化时，重试复用原快照的内容与 content_hash，不得重新采集；重试允许重新构建 Prompt 并重新度量字节。
- 输入事实任一变化时先生成新快照，旧快照标记 superseded。
- active snapshot 标识校验与结果写入必须作为单一原子操作完成：匹配则写入；不匹配或在校验与写入之间新快照被激活，则该结果只能保存为 stale 审计记录，不得覆盖 active 结果、不得关闭 active 门禁、不得生成 InternalPrReview。
- 同一分片或归约在同一快照下只允许一个进行中的 Provider 运行；重复触发的重试必须等待或复用已有运行的结果。

### 失败门禁单活

同一时刻组级审查最多一个开放的失败门禁。优先级为容量超限高于材料溢出，材料溢出高于身份缺失，身份缺失高于归约输出无效，归约输出无效高于分片输出无效。

替换规则：更高优先级失败到达时关闭旧门禁并落地新门禁，旧门禁保留审计记录；更低优先级失败到达时保留现有门禁并把该失败记录为附加证据；同一门禁种类的失败到达时复用现有门禁并合并证据与运行失败码，不关闭后重开。门禁的落地与替换必须原子完成。

动作集合：组级失败门禁只提供重试该环节与终止，不提供人工继续；任何动作都不得把失败门禁转为通过结论。评审结论分诊门禁继续提供重试评审、人工继续、终止。

身份缺失门禁的重试尝试从已持久化的报告与权威 Binding 确定性重建缺失的身份快照；重建成功后继续编译，重建仍失败则再次落地门禁。

### 归约前置条件与返修路由

归约只允许在同一快照下全部分片均已产出有效结论后启动。有效结论定义为可解析且通过输出校验的结论，其判定为 approve、request_changes 或 blocked 均属有效；判定值不影响归约是否启动。存在未完成、传输失败未耗尽或输出无效的分片时不得启动归约。

返修只从归约结论路由：分片结论为 request_changes 或 blocked 时不得据此直接路由 Work Item 返修或计划修订，必须等归约阶段仲裁并产出最终结论后按既有流程决策路由。归约结论要求修改产生新的完成记录或 Revision 时，包含该 Work Item 的分片、相连跨片关系与归约结论失效，输入未变化的其他分片结论可复用。

不得因输出包装失败而产生无发现的通过结论。目标缺失、多重匹配、Revision 不一致或原因码不存在的发现视为无效 Provider 输出，进入人工门禁，不得任意选择首个 Work Item。

## 兼容与迁移

### 组级执行身份与 legacy 字段

- 新架构不得使用分片或归约材料调用既有 per-unit execution-context 绑定接口；该接口语义是 per-unit rendered projection。
- 既有 internal_reviewer_execution_context_hash 作为 legacy 审计字段只读保留；新架构不要求补写，缺失值保持缺失。
- 组级执行身份由材料快照承载：schema_version、compiler_version、content_hash。
- 历史 Attempt 无论 legacy 字段为存在或缺失，只要权威 Binding 校验通过且每个 Unit 具有身份快照，都可在新架构下重试。

### 与 per-unit Reviewer 绑定的适用范围

既有运行期投影契约要求 Reviewer 读取绑定的 Reviewer Projection 与 Renderer 内容哈希。该要求的适用范围明确为单项 Coding Unit 的 Reviewer Provider。Group Final Review 的组级材料编译是例外：它从权威 Binding 解析的规范性 Revision 数据编译材料，不要求为组级材料绑定 per-unit Reviewer 执行上下文，其执行身份由组级材料快照承载。该例外不改变单项 Coding Unit 的绑定要求。

### Provider

三家 Provider 使用同一材料协议与同一文本结论契约，保留原始输出审计，不引入 Provider 特判。

### 前端

首期最终用户可见结果继续为 InternalPrReview；分片与归约产物先作为后端审计数据，前端仅需表达组级分片进行中与可重试状态。

## Attempt 与共享 worktree 锁的终态清理

共享 worktree 锁的所有权属于 Coding Attempt，而不是某一时刻的 active Work Item。组级审查的失败、用户终止、attempt 删除、最终确认与组完成均必须走同一 owner-based 释放入口：仅当持久化锁的 current_lock_owner_id 等于该 attempt_id 时清除锁与运行态游标；不匹配或锁不存在时幂等 no-op。终态释放不得以 current_work_item_id、回退得到的首个 Work Item 或调用方传入的 Work Item 作为前置条件。

若业务流程已产生失败原因，锁释放或清理自身的异常只能记录为诊断，不能覆盖原始失败原因、转换 attempt 为另一种失败，或阻止其进入终态。这样已在 Work Item 间转移的共享锁不会因组级材料失败而成为孤儿锁。

## 权衡

1. 以增加 Provider 调用次数换取单次输入可靠性。20 个 Work Item 由 1 次超大调用变为至少 6 次预算受控调用，并获得按分片重试能力。
2. 以新增材料编译层与身份快照换取组级聚合的确定性，避免从缺少身份的历史报告反推。
3. 以压缩单位材料换取容量，压缩范围限定为单项审查已覆盖的细节；接口、范围、证据与路由元数据保留。
4. 确定性分片不追求最小化跨片边总数，而追求确定与不漏；跨片边由归约阶段统一审查。
5. 补救禁止 approve 是 fail-closed 取舍：真正通过的结论应能输出结构化 JSON，无法解析本身即异常信号。
