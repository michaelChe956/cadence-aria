## Purpose

约束单仓 Design 段（issue → 已确认 Story → design 生成 → review → 用户反馈返修 → 确认）在弱能力模型下的 reviewer 边界判定、用户反馈修订契约、结构契约回归与实测 campaign 验收，使 Design 段达到与 Story 段同级的可用性与证据标准。仅覆盖 legacy 单仓 Design，不含 aggregate 分支。

## ADDED Requirements

### Requirement: Reviewer 边界判例 few-shot（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

Design reviewer prompt SHALL 包含一组对照判例：纯抽象 `[DEC-*]`→`[REQ-*]/[AC-*]` 追踪最高判 `suggestion`；出现可执行测试内容（测试文件/模块、测试框架/夹具、可运行命令、分步测试场景、把测试或验证职责分派给组件或文件）必须判 `must_fix`；风险章节仅提及验证归属而无具体方案时 SHALL NOT 判强返修。判例 SHALL 不含 sentinel 封装、nonce 字段或完整 verdict JSON（防照抄从源头消除），且仅注入 Design 的 review 输入构建入口；Story/WorkItem/WorkItemPlan 的 review prompt SHALL 不包含该判例。

#### Scenario: 抽象追踪不被误杀
- **WHEN** Design 产物仅包含 `[DEC-*]` 到 `[REQ-*]/[AC-*]` 的抽象验收追踪且不描述如何测试
- **THEN** 判例明示正确判定为 suggestion 或不报，绝不产生 blocking/must_fix

#### Scenario: 测试越界必须返修
- **WHEN** Design 产物出现测试计划、命令、框架选型或组件测试职责分派
- **THEN** 判例明示必须 must_fix 并触发返修

#### Scenario: 风险提及不误伤
- **WHEN** Design 风险章节仅声明「由下游阶段安排验证」而无具体方案
- **THEN** 判例明示为 pass，不得以「风险未验证」出强返修

#### Scenario: 判例不可照抄且范围隔离
- **WHEN** 构建 review prompt
- **THEN** 判例串不含 `ARIA_STRUCTURED_OUTPUT` 与 `nonce=`；非 Design workspace 的 review prompt 不含判例

### Requirement: 示例载荷不得经 repair 复活（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

Workspace reviewer 结构化输出的 envelope repair 路径 SHALL 将携带判例指纹载荷（示例固定 ID 组合）的恢复值判为不可修复并降级人工 triage；该判据 SHALL 同时覆盖 JSON nonce 错配与缺失 JSON nonce 两类错误。恒不携带恢复载荷的错误码分支 SHALL 从可修复枚举中移除并以测试锁定。repair prompt SHALL 显式排除示例 nonce 并以剥离 sentinel 后的文本回灌原始输出。

#### Scenario: 照抄载荷转人工
- **WHEN** reviewer 输出携带判例指纹业务载荷且 nonce 层错误可修复
- **THEN** 不进入 envelope repair，走人工 triage，不自动生成返修轮

#### Scenario: 正常封装修复不受影响
- **WHEN** reviewer 输出仅封装层损坏且载荷不含判例指纹
- **THEN** 既有的 envelope-only repair 行为保持不变

### Requirement: 用户反馈返修与 choice 续写入口契约（Design 分支）（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

单仓 Design 的两类非初次生成 author 入口 SHALL 注入结构契约：

1. **用户自由反馈返修**（AuthorConfirm 提交自由文本）：prompt SHALL 注入 parser-derived artifact schema 契约、artifact 输出 fence 契约、Design skeleton（防照抄提示使用 DEC/CMP/API + source id 表述）、缺失上下文 note；当前产物作为输入嵌入时 SHALL 使用四反引号围栏以容忍产物内代码块。
2. **Choice followup 续写**（用户回答 author 确认问题后的续写）：prompt SHALL 注入 artifact 输出 fence 契约、Design skeleton 与结构化决策落章 contract（决策写入「设计决策/追踪关系」并映射 DEC），防止对话体作答与决策不落章。

Story/WorkItem 分支的 prompt 内容 SHALL 保持不变。历史压缩在上述入口暂缓注入，待 campaign usage 数据支持后再评估。

#### Scenario: 反馈返修携带完整结构契约
- **WHEN** 单仓 Design 用户提交自由反馈触发返修 prompt 构建
- **THEN** prompt 含 artifact schema、输出 fence 契约、Design skeleton 与 missing context notes，当前产物以四反引号围栏包裹

#### Scenario: Choice followup 不再裸奍
- **WHEN** 用户回答 Design 生成过程中的确认问题触发续写 prompt
- **THEN** prompt 含输出 fence 契约、Design skeleton 与决策落章 contract，模型续写被引导产出合规 artifact 且决策落入指定章节

#### Scenario: 非 Design 分支零变化
- **WHEN** Story 或 WorkItem 用户提交自由反馈返修
- **THEN** 对应 prompt 与既有行为逐字节一致

### Requirement: 单仓确认红线（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

单仓 Design review 通过后 SHALL 停留在作者确认状态而非自动完成；仅在用户显式确认后生命周期记录才进入 Confirmed；单仓 Design 的 author 与 revision 请求 SHALL 不设置 aggregate structured output contract。

#### Scenario: pass 不自动完成
- **WHEN** reviewer 给出 pass 且无未关闭强返修
- **THEN** 工作区停在 AuthorConfirm，等用户 AcceptFinalize 后才 Confirmed

#### Scenario: 无 aggregate contract
- **WHEN** 构建单仓 Design 的初次 author 或 revision 输入
- **THEN** structured output contract 为空，不触发 aggregate metadata 协议

### Requirement: Design corpus/golden/campaign 实测验收（SHALL）

系统 SHALL 满足本 requirement 的全部场景约束。

Design campaign SHALL 使用冻结语料（≥6 形态，每形态配冻结上游已确认 Story Spec fixture 与 SHA-256 digest）：至少覆盖单仓 API 设计、数据模型设计、用户 choice 映射 DEC、抽象追踪正例、测试越界反例、多约束返修。golden 规范化比对 SHALL 覆盖必需 heading 集、DEC/CMP/API ID 集、上游 REQ/AC 引用不丢失、dec_req_links、source 覆盖与用户决策不反转/不改绑；SHALL NOT 要求 DEC/CMP/API 集合与 golden 完全一致。manifest SHALL 分开记录 author/reviewer 的 provider/model/version、fresh/resume strategy、usage（input 与 cache_read）、retry、超时分类与 choice；校验器按 case_id 去重并拒绝矛盾样本。验收 gate：主 gate 为 3 provider 组合 × 12 样本（6 形态 × 2 重复），fresh full-chain 全数通过；测试越界反例（D05）的 full-chain 成功定义为含一次正确 must_fix 返修的全链走通且首轮边界判定正确，不按「首轮 pass」计；边界分类另立 mini-campaign：抽象追踪正例与测试越界反例各 5 重复 × 3 provider = 各 15 观测，假阳/假阴均为 0 方可通过。所有成功率类结论 SHALL 附样本量与置信上界。baseline SHALL 在任何 prompt 改造前采集。manifest SHALL 记录 `resume_available` 维度供 compact_history 启用决策。

#### Scenario: gate 达标
- **WHEN** 三组合 revised 主 campaign 完成
- **THEN** fresh full-chain 36/36 通过（D05 按特例口径计），边界 mini-campaign 假阳/假阴均 0/15，报告附样本量与置信上界并含 usage（或如实记录不可用原因）

#### Scenario: baseline 先行
- **WHEN** 任一 prompt 改造合并前
- **THEN** 已存在冻结 corpus 上的 baseline manifest，供改造收益对比
