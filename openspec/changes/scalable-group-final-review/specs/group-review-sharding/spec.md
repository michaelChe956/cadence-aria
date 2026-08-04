# group-review-sharding Specification

## ADDED Requirements

### Requirement: 组级审查材料必须来自权威 Binding

组级审查的所有材料 MUST 从权威 Reviewer Binding 解析入口取得。系统 MUST NOT 依据审查报告的存储顺序、轮次或 Provider 输出文本推断 Work Item 身份。

#### Scenario: 权威解析校验不通过时失败关闭

- **WHEN** 某个 Work Item 的 Unit Run 未完成，或 Work Item Revision、Projection Bundle 标识、Projection 哈希、Compiler 版本、已解析 Handoff Revision 中任一项与权威记录不一致
- **THEN** 系统 MUST 拒绝编译组级审查材料，MUST NOT 调用 Provider，MUST 给出可诊断的身份校验失败原因

#### Scenario: 缺少单位审查结论身份时失败关闭

- **WHEN** 某个已完成 Work Item 缺少携带 Unit 身份的单项审查结论记录
- **THEN** 系统 MUST 以身份缺失原因失败关闭，MUST NOT 按报告顺序或轮次猜测该结论所属 Work Item

#### Scenario: 身份快照写入必须幂等

- **WHEN** 单项审查成功持久化后写入单位审查结论身份快照，且同一 Unit Run 再次触发写入
- **THEN** 系统 MUST 产生与首次写入相同的内容，MUST NOT 产生重复或冲突记录

#### Scenario: 报告已持久化但身份快照缺失时失败关闭

- **WHEN** 单项审查报告已持久化，但对应的单位审查结论身份快照缺失
- **THEN** 组级材料编译 MUST 以身份缺失原因失败关闭，MUST NOT 以部分数据继续编译

#### Scenario: 缺失的身份快照可确定性重建

- **WHEN** 用户在身份缺失门禁上执行重试，且对应单项审查报告与权威 Binding 均已持久化
- **THEN** 系统 MUST 尝试从已持久化数据确定性重建该身份快照，重建内容 MUST 与首次写入一致；重建成功 MUST 继续编译，重建失败 MUST 再次落地身份缺失门禁

#### Scenario: 半写缺失的身份快照可确定性重建

- **WHEN** 用户在身份缺失门禁上执行重试，且对应的单项审查报告与权威 Binding 均已持久化
- **THEN** 系统 MUST 尝试从已持久化的报告与权威 Binding 确定性重建缺失的身份快照，重建结果 MUST 与首次写入内容一致；重建成功后 MUST 继续组级材料编译，重建仍失败时 MUST 再次落地身份缺失门禁

### Requirement: 单次 Provider 输入必须受字节预算约束

组级审查的每一次 Provider 调用 MUST 以实际发送的完整 Prompt 的 UTF-8 字节数度量输入，1 KiB MUST 按 1024 bytes 计算，并 MUST 依据质量目标与硬上限决定是否发送。系统 MUST NOT 在超过硬上限时调用 Provider。

首期默认质量目标为 28 KiB，硬上限为 30 KiB。硬上限 MUST NOT 通过配置放宽；质量目标可配置，但配置值 MUST 小于硬上限。

#### Scenario: 未超过质量目标时正常发送

- **WHEN** 构建完成的分片或归约输入不超过质量目标
- **THEN** 系统 MUST 正常调用 Provider，并 MUST 记录该次输入的分段字节

#### Scenario: 介于质量目标与硬上限之间时发送并告警

- **WHEN** 构建完成的输入超过质量目标但不超过硬上限
- **THEN** 系统 MUST 调用 Provider，并 MUST 记录预算告警与分段字节

#### Scenario: 超过硬上限时禁止调用

- **WHEN** 构建完成的输入超过硬上限
- **THEN** 系统 MUST NOT 调用 Provider，MUST 持久化包含各分段字节的溢出诊断，MUST 进入可恢复的材料溢出门禁

#### Scenario: 单个字段超限不得截断身份信息

- **WHEN** 某个 Work Item 的契约接口或写入范围材料自身超过其字段预算
- **THEN** 系统 MUST NOT 截断契约标识或身份标识后继续，MUST 进一步分片或进入溢出门禁

#### Scenario: 度量覆盖完整 Prompt

- **WHEN** 系统度量某次 Provider 输入的字节数
- **THEN** 度量对象 MUST 为实际发送的完整 Prompt 的 UTF-8 字节数，MUST NOT 仅统计业务材料部分

### Requirement: 组级审查必须按确定性分片规则切分并逐片审查

组级审查 MUST 使用确定性分片规则把参与审查的 Work Item 切分为分片，每个分片 MUST NOT 超过 4 个 Work Item，并 MUST 逐片执行语义审查。同一输入 MUST 产生同一分片结果，MUST NOT 依赖权重打分、随机数或并发顺序。

order index MUST 取自权威 Binding 解析出的 Unit 顺序；当两个 Unit 的 order index 相同时，MUST 以 unit_id 字典序作为唯一 tie-break。

确定性分片规则为：以 Handoff 依赖边、共享文件边、契约边界边三类亲和边构建连通图；对连通分量按其内部最小 (order index, unit_id) 排序；按排序装箱，分量整体能装入当前分片则放入，否则开启新分片；超过 4 个 Work Item 的分量按 (order index, unit_id) 稳定切成连续的不超过 4 的组，被切断的边进入跨片边集合；分片内按 (order index, unit_id) 排序。

装箱完成后 MUST 执行确定性预算再分片：对每个分片构建材料并度量字节，超过质量目标的分片 MUST 按 (order index, unit_id) 顺序切成满足质量目标的最少数量的连续子分片；再分片产生的被切断边 MUST 进入跨片边集合。该步骤 MUST 重复直到所有分片满足质量目标或任一子分片已只含一个 Work Item；单个 Work Item 仍超过质量目标时 MUST 进入材料溢出门禁。

三类亲和边定义为：Handoff 依赖边为下游 Unit 的 input_contract 消费上游 Unit 的 output_contract；共享文件边为两个 Unit 的完成 diff 修改同一文件；契约边界边为两个 Unit 引用同一 contract_id。

#### Scenario: 二十个 Work Item 至少产生五个分片

- **WHEN** 一个 WorkItemGroup 有 20 个已完成且通过权威校验的 Work Item
- **THEN** 系统 MUST 生成至少 5 个分片，每个分片的 Work Item 数 MUST NOT 超过 4

#### Scenario: 超预算分片确定性再切分

- **WHEN** 装箱产生的某个分片构建后的输入超过质量目标
- **THEN** 系统 MUST 按 (order index, unit_id) 顺序把该分片切成满足质量目标的最少数量的连续子分片，MUST 把再分片切断的边写入跨片边集合，且两次相同输入 MUST 产生相同的再分片结果

#### Scenario: 单个 Work Item 超预算时失败关闭

- **WHEN** 再分片后某个子分片只含一个 Work Item 且其输入仍超过质量目标
- **THEN** 系统 MUST NOT 继续切分，MUST 进入材料溢出门禁

#### Scenario: Handoff 依赖项同片

- **WHEN** Unit B 的 input_contract 消费 Unit A 的 output_contract，且两者所在连通分量不超过分片上限
- **THEN** Unit A 与 Unit B MUST 位于同一分片

#### Scenario: 共享文件项同片

- **WHEN** Unit A 与 Unit B 的完成 diff 修改同一文件，且两者所在连通分量不超过分片上限
- **THEN** Unit A 与 Unit B MUST 位于同一分片

#### Scenario: 超量分量稳定切开

- **WHEN** 一个连通分量的 Work Item 数超过 4
- **THEN** 系统 MUST 按 order index 将该分量切成连续的不超过 4 的组，MUST 将被切断的边写入跨片边集合，且该集合 MUST 在归约阶段被审查

#### Scenario: 分片结果可复现

- **WHEN** 以相同的权威 Binding、ReviewRequest 与 Git 事实两次执行分片
- **THEN** 两次分片结果 MUST 逐片一致，包括片内顺序与跨片边集合

### Requirement: 确定性检查必须在调用 Provider 前完成

系统 MUST 在调用 Provider 前完成契约与能力精确匹配、写入范围与归属、Commit 与证据一致性、Requirement 与验收标准聚合的确定性检查，并 MUST 把结论作为候选发现或门禁依据。系统 MUST NOT 要求 Provider 重新执行这些确定性判断作为唯一依据。

#### Scenario: 契约缺失由确定性检查标记

- **WHEN** 某个 Work Item 声明消费的必填契约在全组任何产出中都不存在
- **THEN** 系统 MUST 在调用 Provider 前标记该契约缺失，并 MUST 将其作为候选发现提供给审查

#### Scenario: 禁止范围命中由确定性检查标记

- **WHEN** 某个 Work Item 的变更路径命中其禁止写入范围
- **THEN** 系统 MUST 在调用 Provider 前标记该范围违规

#### Scenario: 审查请求 Commit 不一致由确定性检查标记

- **WHEN** 审查请求的 Commit 不可达，或未包含某个 Work Item 的完成 Commit
- **THEN** 系统 MUST 在调用 Provider 前标记该一致性失败

### Requirement: 变更材料必须提供完整索引并按级别选择片段

组级审查 MUST 提供完整的变更文件索引，并 MUST 按级别从高到低选择变更片段正文。系统 MUST NOT 以固定长度前缀截断变更正文作为唯一策略。所有进入 Prompt 的片段 MUST 先经过敏感信息脱敏。

片段级别定义为：A 级为命中某 Unit 禁止写入范围的片段；B 级为归属歧义片段；C 级为共享文件片段；D 级为契约相关文件片段；E 级为单 Unit 独占文件片段。同一文件同时命中多个级别时 MUST 归入其中最高级别，级别顺序为 A 高于 B、B 高于 C、C 高于 D、D 高于 E。

owner 定义为：其完成 diff 唯一包含该文件的 Unit Run；无法唯一归属时该文件 MUST 标记为归属歧义。契约相关文件定义为：被某个 Unit 的 output_contract 或 input_contract 声明包含的文件。敏感信息定义为：共享脱敏能力识别的凭据、私钥与令牌模式；脱敏 MUST 复用该共享能力，MUST NOT 由组级材料自行实现替代逻辑。

#### Scenario: 索引始终完整

- **WHEN** 组级审查构建变更材料
- **THEN** 变更文件索引 MUST 包含全部变更路径、增删行数、归属 Unit Run、共享标记、歧义标记、范围违规标记

#### Scenario: 分片选取本片相关片段

- **WHEN** 分片构建变更材料
- **THEN** 片段正文 MUST 只包含与本片 Unit 相关的 A、B、C、D 级片段，E 级 MUST 只提供文件头部

#### Scenario: 归约选取集合机械确定

- **WHEN** 归约阶段构建变更材料
- **THEN** 片段选取集合 MUST 为 A 级命中、B 级命中、owner 属于不同分片的 C 级文件、owner 属于不同分片的 D 级文件的并集，E 级片段 MUST NOT 进入归约

#### Scenario: 无法映射到文件的跨片关系由关系图承载

- **WHEN** 某条被分片切断的 Handoff 依赖边或契约边界边没有对应的变更文件
- **THEN** 该关系 MUST 以全局跨片关系图条目提供给归约阶段，MUST NOT 因缺少可映射文件而从归约材料中省略

#### Scenario: 同级选取顺序稳定

- **WHEN** 同一片段级别内存在多个候选文件
- **THEN** 系统 MUST 按文件路径字典序选取，同文件内 MUST 按 diff 原始顺序选取

#### Scenario: 片段必须脱敏并标注截断状态

- **WHEN** 任一变更片段进入 Prompt
- **THEN** 该片段 MUST 已经过敏感信息脱敏；若被截断，MUST 标注截断状态，并 MUST 记录原始片段哈希与未展示片段数量

### Requirement: 归约阶段必须产出唯一最终结论

组级审查 MUST 由归约阶段产出唯一最终结论，包含最终判定、发现集合、影响范围、PR 描述与提交信息建议。分片结论 MUST NOT 直接作为组级最终结论完成 Attempt。

#### Scenario: 归约只在全部分片产出有效结论后启动

- **WHEN** 同一快照下存在未完成、传输失败未耗尽或输出无效的分片
- **THEN** 系统 MUST NOT 启动归约阶段

#### Scenario: 有效结论不取决于判定值

- **WHEN** 某个分片产出了可解析且通过输出校验的结论，其判定为 approve、request_changes 或 blocked 之一
- **THEN** 该分片 MUST 视为已产出有效结论，MUST NOT 因判定不是 approve 而阻止归约启动

#### Scenario: 返修只从归约结论路由

- **WHEN** 某个分片结论为 request_changes 或 blocked
- **THEN** 系统 MUST NOT 据该分片结论直接路由 Work Item 返修或计划修订，MUST 等待归约阶段产出最终结论后按既有流程决策路由

#### Scenario: 分片结论不能完成组级审查

- **WHEN** 任一分片给出通过结论
- **THEN** 系统 MUST NOT 据此完成组级审查，MUST 等待归约阶段产出最终结论

#### Scenario: 交付叙事只由归约阶段生成

- **WHEN** 组级审查需要影响范围、PR 描述或提交信息建议
- **THEN** 这些内容 MUST 由归约阶段生成，MUST NOT 取自任一单个分片结论

#### Scenario: 最终发现必须通过权威目标校验

- **WHEN** 归约结论包含任何发现
- **THEN** 每个发现 MUST 通过既有组级权威目标校验，且修复目标 MUST 唯一绑定到权威 Work Item Revision

### Requirement: 发现合并必须使用结构化指纹

系统 MUST 使用结构化指纹合并分片与归约阶段的发现。系统 MUST NOT 依据自然语言文本相似度自动合并发现。

指纹 MUST 由缺陷类别、原因码、修复目标标识、契约引用、能力引用、规范化路径与变更片段标识共同决定。

#### Scenario: 相同指纹合并来源

- **WHEN** 两个发现的结构化指纹相同
- **THEN** 系统 MUST 合并其证据与来源，严重度 MUST 取最高

#### Scenario: 不同目标不得合并

- **WHEN** 两个发现的修复目标、契约引用或变更片段标识不同
- **THEN** 系统 MUST 保留为两个独立发现，即使其自然语言描述相近

#### Scenario: 计划类缺陷不被实现类发现覆盖

- **WHEN** 同一指纹下同时存在计划类缺陷与实现类发现
- **THEN** 系统 MUST 保留该计划类缺陷的修复目标种类、逻辑 Work Item 标识集合与 Work Item Revision 标识集合

#### Scenario: 结论按严重程度规约

- **WHEN** 分片与归约结论包含不同判定
- **THEN** 最终判定 MUST 按阻塞高于要求修改、要求修改高于通过的顺序规约

### Requirement: 失败必须区分类型、可按环节重试且不得产生通过结论

组级审查 MUST 区分传输失败与完成但无法解析两类失败，并 MUST 支持仅重试失败环节。任何未解决的失败 MUST NOT 产生通过结论。

传输失败为非用户意图的 Provider 调用失败，包括超时、意外中断与 adapter 错误；完成但无法解析为 Provider 正常完成但输出无法解析为结论 JSON。两类失败 MUST 使用互不相同的运行失败码。用户主动取消 MUST NOT 归为传输失败。

运行失败码与门禁原因码 MUST 分层：运行失败码记录在环节运行与产物上，用于区分传输失败耗尽与补救无效等具体成因；门禁原因码用于区分门禁种类。同一门禁种类 MAY 对应多个运行失败码。

#### Scenario: 传输失败直接重试该环节

- **WHEN** 某个分片或归约的 Provider 调用发生传输失败
- **THEN** 系统 MUST 以全新运行重试该环节，MUST NOT 执行结论转写补救，MUST NOT 重跑输入未变化的其他成功分片

#### Scenario: 用户主动取消不自动重试

- **WHEN** 用户主动取消某个分片或归约的 Provider 运行
- **THEN** 系统 MUST 停止该环节并保留已有产物，MUST NOT 自动以全新运行重试

#### Scenario: 用户主动取消不自动重试

- **WHEN** 用户主动取消某个分片或归约的 Provider 运行
- **THEN** 系统 MUST 停止该环节，MUST NOT 自动以全新运行重试，MUST 保留已产生的原始输出引用

#### Scenario: 传输失败重试必须有上限

- **WHEN** 同一环节反复发生传输失败
- **THEN** 系统 MUST 限制该环节的最大重试次数，重试耗尽时 MUST 停止重试并进入该环节的输出无效门禁

#### Scenario: 完成但无法解析时执行受限补救

- **WHEN** Provider 正常完成但未产出结论 JSON
- **THEN** 系统 MUST 执行一次结论转写补救，补救输入 MUST 只含原始输出与结论契约，MUST NOT 重新包含完整分片或归约材料，且 MUST 禁止在补救中重新审查或新增发现

#### Scenario: 结论输出必须包含可提取判定标记

- **WHEN** 分片或归约的结论契约被构建
- **THEN** 该契约 MUST 要求输出包含机器可提取的判定标记行，标记值 MUST 为 approve、request_changes、blocked 之一

#### Scenario: 补救判定必须等于原始判定标记

- **WHEN** 结论转写补救产出判定，且原始输出包含判定标记
- **THEN** 补救判定 MUST 等于该标记值；不相等时 MUST 视为无效并进入输出无效门禁

#### Scenario: 原始输出缺少判定标记时补救无效

- **WHEN** 结论转写补救的原始输出不包含可提取的判定标记
- **THEN** 系统 MUST 视该补救为无法证明判定，MUST 直接进入输出无效门禁，MUST NOT 推断判定值

#### Scenario: 补救发现必须可溯源到原始输出

- **WHEN** 结论转写补救产出任何发现
- **THEN** 每个发现的描述文本 MUST 为原始输出的子串；任一发现无法溯源时 MUST 视为无效并进入输出无效门禁

#### Scenario: 补救输出禁止 approve

- **WHEN** 结论转写补救产出的判定为 approve
- **THEN** 系统 MUST 视该补救输出为无效，并 MUST 进入输出无效门禁

#### Scenario: 结论 Prompt 必须要求判定标记

- **WHEN** 系统构建分片或归约的结论 Prompt
- **THEN** 该 Prompt MUST 要求输出包含机器可提取的判定标记行，标记值 MUST 为 approve、request_changes 或 blocked 之一

#### Scenario: 补救判定必须等于原始判定标记

- **WHEN** 结论转写补救产出判定
- **THEN** 该判定 MUST 等于从原始输出提取的判定标记值；原始输出缺少判定标记时 MUST 视补救输出为无效并进入输出无效门禁，MUST NOT 由补救自行推断判定

#### Scenario: 补救发现必须可溯源到原始输出

- **WHEN** 结论转写补救产出任一发现
- **THEN** 该发现的描述文本 MUST 是原始输出的子串；无法逐条溯源时 MUST 视补救输出为无效并进入输出无效门禁

#### Scenario: 补救输出必须通过保真校验

- **WHEN** 结论转写补救产出非 approve 判定
- **THEN** 系统 MUST 校验发现数量不超过环节发现上限、证据引用为该 attempt 已有引用的子集、目标与契约引用通过组级权威校验，且原始输出与补救输出的哈希均已持久化；任一校验失败 MUST 进入输出无效门禁

#### Scenario: 补救输入超限时失败关闭

- **WHEN** 构建后的补救 Prompt（含原始输出、结论契约与指令）的完整 UTF-8 字节数超过 16 KiB
- **THEN** 系统 MUST NOT 截断原始输出后补救，MUST 直接进入输出无效门禁

#### Scenario: 补救失败进入输出无效门禁

- **WHEN** 结论转写补救仍未产出通过校验的结论
- **THEN** 系统 MUST 保存原始输出与补救输出引用，MUST 进入输出无效门禁，MUST NOT 产生无发现的通过结论

#### Scenario: 归约失败不重跑成功分片

- **WHEN** 归约阶段发生传输失败或完成但无法解析
- **THEN** 系统 MUST 仅重试归约阶段，MUST NOT 重跑输入未变化的成功分片

#### Scenario: 返修只失效受影响环节

- **WHEN** 某个 Work Item 因归约结论要求修改而产生新的完成记录或 Revision
- **THEN** 系统 MUST 使包含该 Work Item 的分片、相连跨片关系与归约结论失效，MUST NOT 失效输入未变化的其他分片结论

#### Scenario: 目标歧义视为无效输出

- **WHEN** 某个发现的修复目标缺失、存在多重匹配、Revision 不一致或原因码不存在
- **THEN** 系统 MUST 视为无效 Provider 输出并进入人工门禁，MUST NOT 任意选择首个 Work Item 作为目标

### Requirement: 分片与归约必须复用同一不可变材料快照

组级审查 MUST 在启动前生成不可变材料快照，快照 MUST 采用稳定排序与规范化序列化计算内容哈希，并 MUST 使所有分片与归约引用同一快照标识。

#### Scenario: 输入事实未变化时复用快照

- **WHEN** 某个分片或归约阶段被重试，且完成 Commit、Work Item Revision、Handoff Revision、审查请求 Commit、权威 Binding 均未变化
- **THEN** 该次重试 MUST 复用原快照的内容与内容哈希，MUST NOT 重新采集产生不同内容，并 MAY 重新构建 Prompt 与重新度量字节

#### Scenario: 输入事实变化生成新快照

- **WHEN** 完成 Commit、Work Item Revision、Handoff Revision、审查请求 Commit 或权威 Binding 中任一项发生变化
- **THEN** 系统 MUST 生成新的快照，并 MUST 将旧快照标记为 superseded，MUST 使依赖旧快照的分片与归约结论失效

#### Scenario: 晚到结果不得覆盖 active 快照

- **WHEN** 属于旧快照的 Provider 结果在新快照激活后到达
- **THEN** 系统 MUST 将该结果保存为 stale 审计记录，MUST NOT 用其覆盖 active 结果、关闭 active 门禁或生成最终结论

#### Scenario: 结果写入必须原子校验 active 快照

- **WHEN** 分片或归约结果被持久化
- **THEN** active 快照标识的校验与结果写入 MUST 为单个原子操作，MUST NOT 在校验通过后到写入之间允许新快照激活而使旧结果被接受

#### Scenario: active 快照校验与写入必须原子

- **WHEN** 系统持久化某个分片或归约结果
- **THEN** active 快照标识校验与结果写入 MUST 作为单一原子操作完成；若在校验与写入之间新快照被激活，该写入 MUST 失败并将结果保存为 stale 审计记录

#### Scenario: 同一环节并发单飞

- **WHEN** 同一分片或归约在同一快照下已有进行中的 Provider 运行，又触发了重试
- **THEN** 系统 MUST 等待或复用该运行的结果，MUST NOT 并行发起第二个 Provider 调用

### Requirement: 组级审查容量超限必须失败关闭

系统 MUST 在组级审查启动时校验 Work Item 数量。超过首期支持上限（20）时，MUST 在调用任何 Provider 前失败关闭。

#### Scenario: 超限落地容量门禁

- **WHEN** 一个 WorkItemGroup 已完成且通过权威校验的 Work Item 数超过 20
- **THEN** 系统 MUST 落地容量超限阻塞门禁，MUST 记录实际数量与支持上限，MUST NOT 调用任何 Provider

#### Scenario: 超限不得回退或放宽

- **WHEN** 容量超限门禁已落地
- **THEN** 系统 MUST NOT 回退单次全量审查，MUST NOT 提高字节预算上限，MUST NOT 将容量超限与材料溢出混用同一原因码

### Requirement: 组级执行身份与 legacy 执行上下文字段必须兼容

新组级架构 MUST NOT 使用分片或归约材料调用既有 per-unit execution-context 绑定接口。组级执行身份 MUST 由材料快照的 schema 版本、compiler 版本与内容哈希承载。既有 internal_reviewer_execution_context_hash 字段 MUST 只读保留，MUST NOT 补写。

#### Scenario: 历史已绑定 Attempt 可重试

- **WHEN** 一个历史 Attempt 的 UnitRun 已持久化 internal_reviewer_execution_context_hash，且其权威 Binding 校验通过、每个 Unit 具有身份快照
- **THEN** 该 Attempt MUST 能在新架构下重试，MUST NOT 触发身份不匹配错误，既有哈希字段 MUST 保持不变

#### Scenario: 历史未绑定 Attempt 可重试

- **WHEN** 一个历史 Attempt 的 UnitRun 未持久化 internal_reviewer_execution_context_hash，且其权威 Binding 校验通过、每个 Unit 具有身份快照
- **THEN** 该 Attempt MUST 能在新架构下重试，系统 MUST NOT 为该 UnitRun 补写该字段

#### Scenario: 新旧产物审计可区分

- **WHEN** 审查一个组级审查结论的执行身份引用
- **THEN** 引用材料快照哈希的结论与引用 renderer 哈希的历史结论 MUST 可区分，MUST NOT 混用两类身份

### Requirement: 结论输出上限与分片并发必须受约束

分片结论的发现数 MUST NOT 超过 8，归约结论的发现数 MUST NOT 超过 16。超过上限的 Provider 输出 MUST 视为无效并进入对应环节的输出无效门禁。分片并发数 MUST 有上限，默认值为 2，该默认值 MAY 配置。

#### Scenario: 分片发现数超限视为无效

- **WHEN** 某个分片结论包含超过 8 个发现
- **THEN** 系统 MUST 视该输出为无效，MUST 进入分片输出无效门禁

#### Scenario: 归约发现数超限视为无效

- **WHEN** 归约结论包含超过 16 个发现
- **THEN** 系统 MUST 视该输出为无效，MUST 进入归约输出无效门禁

#### Scenario: 分片并发受上限约束

- **WHEN** 同一快照下存在多个待执行分片
- **THEN** 同时进行中的分片 Provider 运行数 MUST NOT 超过配置的并发上限

### Requirement: 组级审查产物必须携带规范性身份与溯源字段

组级审查的材料快照、分片结论与归约结论 MUST 持久化规范性身份与溯源字段。

材料快照 MUST 包含 schema 版本、compiler 版本、attempt 标识、审查请求标识、基线与最终 Commit、权威 Binding 摘要、单位记录、全局关系图、变更索引、确定性发现、分片结果与内容哈希。分片结论 MUST 包含所消费的快照哈希、参与的 Unit Run 标识、分片理由、判定、发现、未决义务、所选变更引用与原始输出引用。归约结论 MUST 包含所消费的快照哈希、各分片结论标识、判定、发现、影响范围、PR 描述、提交信息建议与发现溯源。

#### Scenario: 快照缺少规范性字段时失败关闭

- **WHEN** 材料快照缺少 schema 版本、compiler 版本或内容哈希中任一项
- **THEN** 系统 MUST NOT 以该快照启动任何 Provider 调用，MUST 给出可诊断的材料完整性错误

#### Scenario: 结论必须可溯源到快照与运行

- **WHEN** 系统持久化任一分片结论或归约结论
- **THEN** 该结论 MUST 记录所消费的快照哈希与原始输出引用，且 MUST 能据此还原该次审查所使用的材料身份

#### Scenario: 门禁诊断字段可观测

- **WHEN** 系统落地任一组级失败门禁
- **THEN** 该门禁 MUST 记录门禁原因码、运行失败码、实际值与限制值、所属环节标识，使失败原因可在不重跑的情况下判定

### Requirement: 组级审查输出与并发必须受上限约束

分片结论的发现数量 MUST NOT 超过 8，归约结论的发现数量 MUST NOT 超过 16。分片的并发执行 MUST 有上限，默认值为 2 且 MUST 可配置。

#### Scenario: 分片结论发现数量超限无效

- **WHEN** 某个分片结论包含超过 8 个发现
- **THEN** 系统 MUST 视该输出为无效并进入分片输出无效门禁

#### Scenario: 归约结论发现数量超限无效

- **WHEN** 归约结论包含超过 16 个发现
- **THEN** 系统 MUST 视该输出为无效并进入归约输出无效门禁

#### Scenario: 分片并发受上限约束

- **WHEN** 同一快照下有多个待执行分片
- **THEN** 同时进行中的分片 Provider 运行数 MUST NOT 超过配置的并发上限

### Requirement: 组级审查产物必须携带规范性身份字段

材料快照 MUST 携带 schema 版本、compiler 版本与内容哈希。分片结论 MUST 携带所消费的快照哈希、参与的 Unit Run 标识与原始输出引用。归约结论 MUST 携带所消费的快照哈希、参与的分片结论标识与发现来源。

#### Scenario: 快照携带身份字段

- **WHEN** 组级材料快照被持久化
- **THEN** 该快照 MUST 包含 schema 版本、compiler 版本与内容哈希

#### Scenario: 分片结论携带溯源字段

- **WHEN** 分片结论被持久化
- **THEN** 该结论 MUST 包含所消费的快照哈希、参与的 Unit Run 标识与原始输出引用

#### Scenario: 归约结论携带溯源字段

- **WHEN** 归约结论被持久化
- **THEN** 该结论 MUST 包含所消费的快照哈希、参与的分片结论标识，且每个发现 MUST 可溯源到其来源分片或归约本身

### Requirement: 组级材料协议对所有 Provider 一致

组级审查的材料模型与结论契约 MUST 对全部受支持 Provider 保持一致。系统 MUST NOT 为特定 Provider 引入不同的组级材料结构或结论格式。

#### Scenario: 三家 Provider 使用同一材料结构

- **WHEN** 组级审查分别以 Claude Code、Codex、Pi 作为审查 Provider
- **THEN** 分片与归约的材料结构、字节预算与结论契约 MUST 相同

#### Scenario: 原始输出始终保留审计

- **WHEN** 任一分片或归约阶段收到 Provider 输出
- **THEN** 系统 MUST 持久化该次原始输出引用，无论其是否可解析
