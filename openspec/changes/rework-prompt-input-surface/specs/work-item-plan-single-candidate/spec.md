# Spec Delta

## MODIFIED Requirements

### Requirement: prompt 分层职责（REQ-WSC-06）

系统 SHALL 使 workitem author/reviewer prompt 仅承载产物规格（任务上下文、输出语法、边界约束、判例 few-shot），SHALL NOT 重复目标仓库已注入的通用行为教学（Superpowers/OpenSpec/项目规则）。reviewer 复评前获得的只读契约覆盖投影 SHALL 以最小内容交付：能力覆盖缺口（gaps-only）、handoff 消费闭环、同仓写范围冲突与 PVR；写范围冲突事实 SHALL 限于同一目标仓内（跨仓 forbidden/exclusive 在 multi-repo 场景为假阳性，实测 29 条中 27 条）。author 上下文 SHALL 分级供给：L1 成员仓摘要与 L2 依赖图事实由平台直接注入 prompt，MUST NOT 依赖模型自行 Read/Glob 补事实；L3 维持既有预算约束。**L1 SHALL 进一步包含 plan 基线树可用路径清单**（由平台在校验器同源的基线树上装载后直供），MUST NOT 以纪律文案替代该事实 —— 实测纪律已在 prompt 内而越界率仍达 50%（作者引用了基线树内不存在的目录）。返修轮 prompt SHALL 只携带本轮 delta 与本轮相关既有上下文，MUST NOT 逐字节重发首轮完整 prompt。**复评前轮 findings 清单 SHALL 附本轮机械复算状态并按内容身份去重分区**（待办 / 已关闭）：判定主体 SHALL 为平台对当前候选机械报告的复算，SHALL NOT 交由模型依据自由文本判断某前轮 finding 是否仍存在。

#### Scenario: 弱模型借助判例避免已知矛盾

- **WHEN** author 收到含 golden 反例（如 non_goals 禁测试却要求写测试）的 prompt
- **THEN** 产出候选不含该反例所示矛盾模式（由 campaign golden 集验证）

#### Scenario: author 教学与 canonical 口径一致

- **WHEN** author 依据能力覆盖教学产出候选计划
- **THEN** 其 WI 输入契约引用的 required capabilities 均被所引契约输出 capabilities 覆盖（或缺口被 author 自行修正），不存在「按教学应通过而 canonical 判 required_capability_missing」的口径分叉

#### Scenario: reviewer 拦截 canonical 口径的能力缺口与未消费 handoff

- **WHEN** 候选计划存在 WI 输入契约要求的能力未被所引契约输出 capabilities 覆盖（canonical 将判 required_capability_missing），或存在 provider handoff `provided_contract_refs` 中实际列出的非空引用无任何消费者逐字引用（`provided_contract_refs: []` 不构成未消费；canonical 将判 unconsumed_required_handoff）
- **THEN** reviewer 依据覆盖投影产出 must_fix finding（归类建议 contract_gap，证据含具体 edge/contract_id 与缺失能力或空消费者集事实），策略层按既有 repairable 语义处理，该候选不进入无保留通过

#### Scenario: 终端 Work Item 显式表达空 handoff

- **WHEN** 合法依赖图中的 Work Item 不存在任何下游 consumer edge，且候选仍输出完整的 Handoff Schema
- **THEN** `provided_contract_refs` 显式为 `[]`；compiler 将其确定性 lowering 为空集合；canonical validator 与 reviewer SHALL NOT 因此产生 `unconsumed_required_handoff`；非空 provided ref 仍须被下游 input_contracts 逐字消费；环、自依赖、未知 provider 仍按既有 fail-closed 规则拒绝；省略 `provided_contract_refs` 字段本身仍按缺字段规则拒绝

#### Scenario: 投影与 validator 同源且仅单候选注入

- **WHEN** 构建 reviewer context 的契约覆盖投影
- **THEN** 投影复用 `dependency.rs` 同一确定性计算逻辑（同输入同口径），覆盖能力缺口与 handoff 消费闭环两类事实；legacy/story/design reviewer 路径不接收该投影且行为不变

#### Scenario: options 教学与校验口径一致

- **WHEN** author 依据 options 镜像教学产出候选（flag 启用则建对应 kind Work Item）
- **THEN** 不存在「按教学应通过而校验判 options 缺口」的口径分叉；教学文案与校验器错误信息引用同一修复路径描述

#### Scenario: AC 基线教学生效

- **WHEN** author 产出引用仓库文件的验收标准
- **THEN** 引用路径均存在于 plan 基线树；引用其他分支路径的候选在教学+校验双重防线下的发生率显著低于无教学基线

#### Scenario: 复评注入前轮清单

- **WHEN** 单候选复评 prompt 构建
- **THEN** 前轮非 advisory findings 的 canonical identity 与 required_action 注入为结构化清单；同一问题重现时 reviewer 输出带重复标注

#### Scenario: 完备度意见降级

- **WHEN** reviewer prompt 组织 finding 输出
- **THEN** reviewer prompt SHALL 将完备度类意见降级为 advisory，并要求每条 finding 附归类建议

#### Scenario: 契约覆盖教学

- **WHEN** author prompt（单候选）教学 WI `input_contracts` 所引契约的能力覆盖
- **THEN** author prompt（单候选）SHALL 教学契约能力覆盖纪律：WI `input_contracts` 所引契约的输出 capabilities SHALL 覆盖该引用的全部 `required_capabilities`（require_all 全量覆盖、require_any 至少一项、同 contract_id 集合语义），端点/动作类能力 SHALL 显式声明、字段/记录形态承诺不隐含端点能力

#### Scenario: 覆盖口径对齐校验器

- **WHEN** 陈述该教学判定口径
- **THEN** 该教学判定口径 SHALL 与 canonical 校验器 `report_contract_requirements` 一致，并明示 `required_capability_missing` 的 fail-closed 后果

#### Scenario: handoff 字段教学

- **WHEN** author prompt（单候选）教学 handoff 消费闭环纪律
- **THEN** author prompt（单候选）SHALL 教学 handoff 消费闭环纪律：每个 Work Item 的 `Handoff Schema` 三个字段 SHALL 显式存在

#### Scenario: 提供引用逐字对应

- **WHEN** author 填写 `provided_contract_refs`
- **THEN** `provided_contract_refs` SHALL 仅列出会被下游 Work Item 的 `input_contracts` 以（`provider_logical_work_item_id`, `contract_id`）二元组逐字消费的契约引用，数组元素 SHALL 唯一且非空白

#### Scenario: 无下游保留空集

- **WHEN** 某 Work Item 在合法计划依赖图中不存在任何下游 consumer edge
- **THEN** 若该 Work Item 在合法计划依赖图中不存在任何下游 consumer edge，则 `provided_contract_refs` SHALL 保留字段并显式写为 `[]`

#### Scenario: 禁止回避校验

- **WHEN** 交付面临 handoff 消费闭环校验
- **THEN** 不得通过省略 Handoff Schema、删除必需字段、写 blocker、修改 contract ID、依赖 `depends_on` 或自然语言描述来回避校验

#### Scenario: handoff 口径对齐

- **WHEN** 确定该纪律判定口径
- **THEN** 该纪律判定口径 SHALL 与 `unconsumed_required_handoff` 校验一致

#### Scenario: 选项镜像教学

- **WHEN** author prompt（单候选）教学创建计划选项与对应 Work Item kind 要求
- **THEN** author prompt（单候选）SHALL 教学 options 镜像纪律：创建计划选项（include_integration_tests / include_e2e_tests / 前后端拆分）的启用条件与对应 Work Item kind 要求 SHALL 与校验器口径逐字一致

#### Scenario: 缺类错误后果明示

- **WHEN** 教学陈述 flag 启用而缺对应 kind 的情形
- **THEN** 教学须明示 flag 启用而缺对应 kind 时的 Error 后果与两种修复路径

#### Scenario: 验收路径基线纪律

- **WHEN** author prompt 教学验收标准（AC）/验证计划的文件路径引用
- **THEN** author prompt 亦 SHALL 教学验收标准基线纪律：AC/验证计划引用的仓库内文件路径必须存在于 plan 基线树，不得引用其他分支才存在的文件

#### Scenario: 复评前只读投影

- **WHEN** reviewer（单候选）进入复评前取数
- **THEN** reviewer（单候选）SHALL 在复评前获得只读契约覆盖投影，内容 SHALL 至少包括：逐 WI→contract edge 的 required capabilities、所引契约输出 capabilities 与 compatibility_policy

#### Scenario: 投影含图与冲突事实

- **WHEN** 只读契约覆盖投影组装内容
- **THEN** 节点依赖图事实（depends_on/边/环/重复边/未知 provider）；handoff 消费闭环（每个 `provided_contract_refs` 的消费者集合与消费状态，无消费者时显式空集）；跨 work item 写范围冲突事实（exclusive/forbidden 重叠）

#### Scenario: 投影复用共享计算

- **WHEN** 生成契约覆盖投影数据
- **THEN** 投影数据 SHALL 复用 `src/product/work_item_contract/dependency.rs` 的确定性共享计算逻辑生成，SHALL NOT 以独立重述口径替代

#### Scenario: 缺口产 must_fix

- **WHEN** reviewer 评审能力覆盖缺口与未被消费的 handoff
- **THEN** reviewer SHALL 对能力覆盖缺口与未被消费的 handoff 产出 must_fix finding（归类建议 contract_gap），SHALL NOT 将 canonical 将判 `required_capability_missing` 或 `unconsumed_required_handoff` 的候选评为无 must_fix 通过

#### Scenario: 校验器原样保持

- **WHEN** canonical 校验器面对候选计划
- **THEN** canonical 校验器保持 fail-closed 原样

#### Scenario: 投影为前置防线

- **WHEN** reviewer 投影与 canonical 校验器协同工作
- **THEN** reviewer 投影为其前置防线而非替代

#### Scenario: 投影仅注单候选

- **WHEN** 契约覆盖投影注入 reviewer 路径
- **THEN** 契约覆盖投影 SHALL 仅注入单候选 reviewer 路径，legacy/story/design reviewer SHALL NOT 接收

#### Scenario: 注入前轮结构化清单

- **WHEN** reviewer prompt（单候选复评）组装
- **THEN** reviewer prompt（单候选复评）SHALL 注入前轮结构化 findings：每轮注入前一轮非 advisory findings 的 canonical identity（结构化 key/fingerprint）、category、required_action 与当前候选 revision；**并 SHALL 附本轮机械复算状态**（该 finding 在当前候选是否仍存在）**且按内容身份去重**（同一内容身份只保留一条，重复条目折叠为计数）

#### Scenario: 重复判定不靠记忆

- **WHEN** reviewer 复评遇到同 identity 重现的 finding
- **THEN** reviewer SHALL 依据该清单执行重复判定纪律（同 identity 重现须显式标注），SHALL NOT 依赖自由文本记忆；**「该前轮 finding 在本轮是否仍存在」SHALL 以平台给出的复算分区事实为准**，SHALL NOT 由模型依据清单文本自行推断

#### Scenario: 投影最小内容为缺口与消费闭环

- **WHEN** reviewer（单候选）复评前取得只读契约覆盖投影
- **THEN** 投影呈现内容 SHALL 收窄为四类：能力覆盖缺口（gaps-only）、handoff 消费闭环、同仓写范围冲突、Projection Validation Report；该收窄 MUST 以本 requirement 的显式条款生效，MUST NOT 静默裁剪

#### Scenario: 写范围冲突事实按同仓限定

- **WHEN** 只读契约覆盖投影组装写范围冲突事实
- **THEN** 冲突事实 SHALL 只含同一目标仓内的 exclusive/forbidden 重叠；跨仓 forbidden/exclusive 重叠 SHALL NOT 进入投影（跨仓重叠在 multi-repo 场景无意义，实测 29 条写范围冲突中 27 条为跨仓假阳性）

#### Scenario: 投影收窄不删平台侧校验

- **WHEN** 平台对同一 candidate 执行 source/IR/机械报告的绑定与同代校验
- **THEN** 校验 SHALL 与 reviewer 侧投影呈现裁剪无关地照常执行，MUST NOT 因投影内容收窄而跳过或降级

#### Scenario: author 上下文分级直供

- **WHEN** 平台构造单候选 author prompt
- **THEN** L1 成员仓摘要与 L2 依赖图事实 SHALL 由平台直接注入 prompt，SHALL NOT 依赖模型自行 Read/Glob/搜索补齐；L3 上下文 SHALL 维持既有预算约束

#### Scenario: 成员仓摘要不得被点条目挤掉

- **WHEN** 多仓场景下聚合根顶层条目数超过成员仓摘要上限
- **THEN** 成员仓条目 SHALL 优先于点文件/点目录条目进入成员仓摘要，MUST NOT 出现全部成员仓落榜；单仓摘要口径 SHALL 维持 REQ-PLN-17 的逐字不变义务

#### Scenario: 返修轮不重发首轮全文

- **WHEN** 平台构造单候选 author 返修轮 prompt
- **THEN** prompt SHALL 只携带本轮 delta 与本轮相关既有上下文，MUST NOT 逐字节包含首轮完整 prompt；本轮 findings 回灌与逐条硬性修复指令 SHALL 保持注入

#### Scenario: 返修轮保留本轮修复所需上下文

- **WHEN** 返修轮 prompt 携带上一版 source 与本轮 reviewer findings
- **THEN** findings 的 required_action 所指向的 section 及其依赖上下文 SHALL 在返修轮可得，MUST NOT 因不再重发首轮完整 prompt 而使修复动作失去可执行对象

#### Scenario: 基线树事实直供

- **WHEN** 平台构造单候选 author prompt（首轮与返修轮均适用）
- **THEN** 平台 SHALL 在**作者轮启动前**从校验器同源的 plan 基线树（`git ls-tree -r --name-only refs/heads/<基线分支>`）装载可用路径清单并作为 L1 的一部分注入 prompt；清单 SHALL 按成员仓分组、成员仓优先；**基线树不可得（无仓 issue）时该段 SHALL 缺席**且零回归；**基线树装载失败 SHALL NOT 阻断作者轮**（既有 fail-closed 点仍在 Evaluate 阶段）

#### Scenario: 基线树清单不得占用固定规范面预算

- **WHEN** 注入基线树可用路径清单
- **THEN** 清单 SHALL 落在**上下文面**（`WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES` = 32,000），MUST NOT 挤占固定规范面质量预算（`WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES` = 23,500，实测余量仅 18~40 B）；**两个预算常量 SHALL NOT 上调**；新增直供内容 SHALL 计入其所属预算面（固定规范面内容计入 23,500、上下文面内容计入 32,000），SHALL NOT 以「动态段」名义规避任一面；清单超上限时 SHALL 按成员仓保底截断，MUST NOT 使上下文面超预算

#### Scenario: 越界路径回灌最近候选

- **WHEN** 平台判定某验收标准引用的仓库内路径不存在于 plan 基线树
- **THEN** 该 Error 级 finding 的 message SHALL 附**基线树中同前缀最近的现存路径**（并列时全部列出；无同前缀候选时如实说明无可用候选）；候选路径 MUST 取自同一棵基线树，MUST NOT 由平台臆造路径

#### Scenario: 前轮 finding 状态求交按内容身份

- **WHEN** 平台为复评 prompt 计算前轮 findings 的本轮状态
- **THEN** 求交 SHALL 以**内容身份**（前轮 finding 的 `contract_field` 与当前候选机械报告 finding 的 `code`）为键，MUST NOT 以含 `message` 的指纹为键（机械报告 message 与评审输出 message 不同，按指纹求交恒不相交）；求交数据源 SHALL 为**当前候选**的机械报告（与评审输入同代）

#### Scenario: 前轮 finding 分区注入

- **WHEN** reviewer prompt（单候选复评）注入前轮 findings 清单
- **THEN** 清单 SHALL 分为**待办**（当前候选机械报告仍存在同内容身份，附 `required_action` 与 `contract_field`）与**已关闭**（当前候选机械报告已无该内容身份，附关闭依据）两区；**已关闭区条目 MUST NOT 要求作者再次修改**，SHALL NOT 触发「无可改对象」式的重复上报

#### Scenario: 前轮清单同身份去重

- **WHEN** 前轮非 advisory findings 含多条同内容身份条目（同一机械码对多个 Work Item 各登记一次）
- **THEN** 清单 SHALL 只保留一条并折叠为计数；MUST NOT 以多条重复条目呈现（实测同一 preflight 码曾产生 4 条完全相同的待办）
