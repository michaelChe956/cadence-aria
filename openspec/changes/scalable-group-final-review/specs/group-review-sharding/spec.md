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

### Requirement: 单次 Provider 输入必须受字节预算约束

组级审查的每一次 Provider 调用 MUST 在构建后度量输入字节，并 MUST 依据质量目标与硬上限决定是否发送。系统 MUST NOT 在超过硬上限时调用 Provider。

质量目标为 28 KiB，硬上限为 30 KiB。

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

### Requirement: 组级审查必须按分片上限切分并逐片审查

组级审查 MUST 把参与审查的 Work Item 切分为多个分片，每个分片 MUST NOT 超过 4 个 Work Item，并 MUST 逐片执行语义审查。

#### Scenario: 二十个 Work Item 切分为五个分片

- **WHEN** 一个 WorkItemGroup 有 20 个已完成且通过权威校验的 Work Item
- **THEN** 系统 MUST 生成 5 个分片，每个分片的 Work Item 数 MUST NOT 超过 4，并 MUST 在归约前完成全部分片审查

#### Scenario: 强耦合项优先同片

- **WHEN** 两个 Work Item 之间存在 Handoff 依赖边或修改同一文件
- **THEN** 分片 MUST 优先将其归入同一分片，除非该连通分量超过分片上限

#### Scenario: 超过上限的连通分量必须记录跨片关系

- **WHEN** 一个强耦合连通分量的 Work Item 数超过分片上限而被切分
- **THEN** 所有被切断的契约边、共享路径边与归属歧义边 MUST 进入全局跨片关系集合，并 MUST 在归约阶段被审查

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

### Requirement: 变更材料必须提供完整索引并按风险选择片段

组级审查 MUST 提供完整的变更文件索引，并 MUST 按风险选择变更片段正文。系统 MUST NOT 以固定长度前缀截断变更正文作为唯一策略。

#### Scenario: 索引始终完整

- **WHEN** 组级审查构建变更材料
- **THEN** 变更文件索引 MUST 包含全部变更路径、增删行数、归属 Unit Run、共享与歧义标记、范围违规标记

#### Scenario: 归约只接收跨片可疑片段

- **WHEN** 归约阶段构建变更材料
- **THEN** 片段正文 MUST 只包含跨分片的共享、歧义、范围或契约可疑片段

#### Scenario: 片段必须标注截断与脱敏状态

- **WHEN** 任一变更片段被截断或包含被脱敏内容
- **THEN** 该片段 MUST 标注截断状态、脱敏状态、原始片段哈希与未展示数量

### Requirement: 归约阶段必须产出唯一最终结论

组级审查 MUST 由归约阶段产出唯一最终结论，包含最终判定、发现集合、影响范围、PR 描述与提交信息建议。分片结论 MUST NOT 直接作为组级最终结论完成 Attempt。

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
- **THEN** 系统 MUST 保留完整的计划类修复目标信息

#### Scenario: 结论按严重程度规约

- **WHEN** 分片与归约结论包含不同判定
- **THEN** 最终判定 MUST 按阻塞高于要求修改、要求修改高于通过的顺序规约

### Requirement: 失败必须可按环节重试且不得产生通过结论

组级审查 MUST 区分分片失败与归约失败，并 MUST 支持仅重试失败环节。任何未解决的失败 MUST NOT 产生通过结论。

#### Scenario: 单个分片失败只重试该分片

- **WHEN** 某个分片的 Provider 调用失败或未产出结论 JSON
- **THEN** 系统 MUST 仅重试该分片，MUST NOT 重跑输入未变化的其他成功分片

#### Scenario: 未产出结论时执行受限补救

- **WHEN** Provider 正常完成但未产出结论 JSON
- **THEN** 系统 MUST 执行一次结论转写补救，补救输入 MUST NOT 重新包含完整分片或归约材料，且 MUST 禁止在补救中重新审查或新增发现

#### Scenario: 补救失败进入输出无效门禁

- **WHEN** 结论转写补救仍未产出可解析结论
- **THEN** 系统 MUST 保存原始输出与补救输出引用，MUST 进入输出无效门禁，MUST NOT 产生无发现的通过结论

#### Scenario: 归约失败不重跑成功分片

- **WHEN** 归约阶段调用失败或未产出结论 JSON
- **THEN** 系统 MUST 仅重试归约阶段，MUST NOT 重跑输入未变化的成功分片

#### Scenario: 返修只失效受影响环节

- **WHEN** 某个 Work Item 因分片结论要求修改而产生新的完成记录或 Revision
- **THEN** 系统 MUST 使该分片、相连跨片关系与归约结论失效，MUST NOT 失效输入未变化的其他分片结论

#### Scenario: 目标歧义视为无效输出

- **WHEN** 某个发现的修复目标缺失、存在多重匹配、Revision 不一致或原因码不存在
- **THEN** 系统 MUST 视为无效 Provider 输出并进入人工门禁，MUST NOT 任意选择首个 Work Item 作为目标

### Requirement: 分片与归约必须复用同一不可变材料快照

组级审查 MUST 在启动前生成不可变材料快照，并 MUST 使所有分片与归约引用同一快照标识。重试 MUST 复用同一快照。

#### Scenario: 重试复用同一快照

- **WHEN** 某个分片或归约阶段被重试且其输入事实未变化
- **THEN** 该次重试 MUST 复用原快照标识，MUST NOT 重新编译出不同内容的材料

#### Scenario: 输入事实变化生成新快照

- **WHEN** Work Item 完成 Commit、Work Item Revision、Handoff Revision、审查请求 Commit 或权威 Binding 中任一项发生变化
- **THEN** 系统 MUST 生成新的快照，并 MUST 使依赖旧快照的分片与归约结论失效

### Requirement: 组级材料协议对所有 Provider 一致

组级审查的材料模型与结论契约 MUST 对全部受支持 Provider 保持一致。系统 MUST NOT 为特定 Provider 引入不同的组级材料结构或结论格式。

#### Scenario: 三家 Provider 使用同一材料结构

- **WHEN** 组级审查分别以 Claude Code、Codex、Pi 作为审查 Provider
- **THEN** 分片与归约的材料结构、字节预算与结论契约 MUST 相同

#### Scenario: 原始输出始终保留审计

- **WHEN** 任一分片或归约阶段收到 Provider 输出
- **THEN** 系统 MUST 持久化该次原始输出引用，无论其是否可解析
