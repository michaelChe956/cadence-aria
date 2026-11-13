## MODIFIED Requirements

### Requirement: prompt 分层职责（REQ-WSC-06）

workitem author/reviewer prompt SHALL 仅承载产物规格（任务上下文、输出语法、边界约束、判例 few-shot），SHALL NOT 重复目标仓库已注入的通用行为教学。author 上下文 SHALL 由平台直接注入 plan 基线树可用路径清单（基线树事实直供），该清单 MUST NOT 占用固定规范面预算、MUST 计入上下文面预算。前轮 finding 状态 SHALL 由平台按内容身份机械复算后求交、去重并分区注入（重复判定不靠模型记忆）。

成员仓事实 MUST 由平台在首轮 author prompt 的 `structure:` 段**以成员优先的完整行**交付：预算内**所有成员仓的身份头 MUST 全部在场**（顶层条目数与总行数上限只约束**可轮转的子项**）；超限时 SHALL 显式报出并给出总数与所需字节，MUST NOT 假称全员在场，MUST NOT 因材料不全而让作者轮失败或新增模型调用。聚合根 MUST 取 launch 的工作目录，MUST NOT 以 focus 仓路径代替。

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

#### Scenario: 越界路径回灌最近候选

- **WHEN** 平台判定某验收标准引用的仓库内路径不存在于 plan 基线树
- **THEN** 该 Error 级 finding 的 message SHALL 附**基线树中同前缀最近的现存路径**（并列时全部列出；无同前缀候选时如实说明无可用候选）；候选路径 MUST 取自同一棵基线树，MUST NOT 由平台臆造路径

#### Scenario: 基线树事实直供

- **WHEN** 平台构造单候选 author prompt（首轮与返修轮均适用）
- **THEN** 平台 SHALL 在**作者轮启动前**从校验器同源的 plan 基线树（`git ls-tree -r --name-only refs/heads/<基线分支>`）装载可用路径清单并作为 L1 的一部分注入 prompt；清单 SHALL 按成员仓分组、成员仓优先；**基线树不可得（无仓 issue）时该段 SHALL 缺席**且零回归；**基线树装载失败 SHALL NOT 阻断作者轮**（既有 fail-closed 点仍在 Evaluate 阶段）

#### Scenario: 基线树清单不得占用固定规范面预算

- **WHEN** 注入基线树可用路径清单
- **THEN** 清单 SHALL 落在**上下文面**（`WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES` = 32,000），MUST NOT 挤占固定规范面质量预算（`WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES` = 23,500，实测余量仅 18~40 B）；**两个预算常量 SHALL NOT 上调**；新增直供内容 SHALL 计入其所属预算面（固定规范面内容计入 23,500、上下文面内容计入 32,000），SHALL NOT 以「动态段」名义规避任一面；清单超上限时 SHALL 按成员仓保底截断，MUST NOT 使上下文面超预算

#### Scenario: 前轮 finding 状态求交按内容身份

- **WHEN** 平台为复评 prompt 计算前轮 findings 的本轮状态
- **THEN** 求交 SHALL 以**内容身份**（前轮 finding 的 `contract_field` 与当前候选机械报告 finding 的 `code`）为键，MUST NOT 以含 `message` 的指纹为键（机械报告 message 与评审输出 message 不同，按指纹求交恒不相交）；求交数据源 SHALL 为**当前候选**的机械报告（与评审输入同代）

#### Scenario: 前轮 finding 分区注入

- **WHEN** reviewer prompt（单候选复评）注入前轮 findings 清单
- **THEN** 清单 SHALL 分为**待办**（当前候选机械报告仍存在同内容身份，附 `required_action` 与 `contract_field`）与**已关闭**（当前候选机械报告已无该内容身份，附关闭依据）两区；**已关闭区条目 MUST NOT 要求作者再次修改**，SHALL NOT 触发「无可改对象」式的重复上报

#### Scenario: 前轮清单同身份去重

- **WHEN** 前轮非 advisory findings 含多条同内容身份条目（同一机械码对多个 Work Item 各登记一次）
- **THEN** 清单 SHALL 只保留一条并折叠为计数；MUST NOT 以多条重复条目呈现（实测同一 preflight 码曾产生 4 条完全相同的待办）

#### Scenario: 成员仓身份头在预算内全部在场

- **WHEN** 聚合根下有多个成员仓，且顶层条目数超过上限、目录总行数超过上限
- **THEN** `structure:` 段 MUST 在预算内包含**全部成员仓的身份头**（成员仓优先、完整行、不被点文件挤掉），超限部分 MUST 以**轮转子项**方式截断并显式标记

#### Scenario: 成员事实超预算时显式失败而非假装在场

- **WHEN** 全部成员仓的身份头本身即超出 `structure:` 段预算
- **THEN** 平台 MUST 显式报出成员总数与所需字节，MUST NOT 假称全员在场，MUST NOT 截断身份头行，MUST NOT 因此使作者轮失败，MUST NOT 新增模型调用

#### Scenario: 单仓摘要逐字不变

- **WHEN** 本轮无已确认成员集合（成员集合缺席）
- **THEN** 目录摘要 MUST 与既有单仓 fixture **逐字一致**（分层上限、单层截断与空仓文案均不变）

#### Scenario: 首轮事实直供后不得重复全仓探测

- **WHEN** 首轮 author prompt 已获得成员仓事实
- **THEN** 首轮固定纪律 MUST 表述为「已给事实不重复全仓探测；缺事实才针对性查证」，且 MUST NOT 禁止必要查证或禁止产出真实验证命令

### Requirement: 审查 finding 必须落在 Work Item 范围内，范围外事实降级为 advisory（REQ-WSC-12）

每条 finding MUST 显式声明 `disposition`：`actionable` 或 `advisory`。平台 MUST 只把 `actionable` 计入阻断判定、返修触发与必关闭 finding 清单。范围之外的事实 MUST 标 `advisory`，其 `required_action` MUST NOT 要求作者执行。缺失 `disposition` MUST 视作 `actionable`（fail-safe）；finding MAY 携带 `target_work_item_id`。契约类缺口的平台加固见 REQ-WSC-17。

**SC 会话有效投影（窄例外）**：可信会话为 WorkItemPlan 单候选时，`severity=Suggestion`、category 显式属完备度/验证不可归因/其它、`class_hint=Repairable` 的 finding MUST 在内存有效视图投影为 `advisory`，MUST NOT 计入必关闭清单，MUST NOT 单独驱返修，MUST NOT 回写 durable。

#### Scenario: 范围外文档缺口降级为 advisory、不阻断

- **WHEN** reviewer 发现仓库里某份产品侧规范文档仍写着「中文 locale 尚未配置」，而本 Work Item 的写入范围只有代码与语言包
- **THEN** 该 finding MUST 带 `disposition=advisory`，`required_action` 只作提示而不要求作者执行；平台 MUST NOT 因此判定返修，owner MUST 仍能在门卡上看到它

#### Scenario: 本 Work Item 的验收标准缺口仍是 actionable

- **WHEN** reviewer 发现本 Work Item 的 `busi/gateway` 能力契约缺少「非数字 id → 400」的验收标准
- **THEN** 该 finding MUST 带 `disposition=actionable`（或缺失，按 fail-safe 视作 actionable），MUST 进入阻断与返修通道

#### Scenario: 模型把 advisory 误标为 must_fix 也不阻断

- **WHEN** reviewer 输出 `severity=must_fix` 同时 `disposition=advisory`
- **THEN** 平台 MUST NOT 因该 finding 把 `pass` 归一为返修、MUST NOT 把它列入必关闭清单——判定在平台侧，不依赖模型自觉标对

#### Scenario: 缺失 disposition 的历史记录仍阻断

- **WHEN** 平台解析一条 wire 上既无 `disposition` 也无 `target_work_item_id` 的 finding（历史记录或旧 provider 输出）
- **THEN** 解析 MUST 成功且该 finding 语义上等同 `actionable`，既有阻断行为逐字不变

#### Scenario: 非法 disposition 值判不合规

- **WHEN** reviewer 输出 `disposition: "maybe"`
- **THEN** 系统 MUST 以既有 `invalid_finding_field` 判结构化输出不合规，MUST NOT 猜测成任一合法值

#### Scenario: SC 会话内建议级非契约项不单独驱返修

- **WHEN** 服务端可信会话确认为 WorkItemPlan 且为单候选，某 finding 的 `severity=Suggestion`、category 显式属于完备度/验证不可归因/其它、且 `class_hint=Repairable`
- **THEN** 会话有效视图 MUST 将该 finding 投影为 `advisory`，MUST NOT 计入必关闭 finding 清单，MUST NOT 单独驱动作者返修，且该投影 MUST NOT 回写 durable 记录

#### Scenario: 窄例外的边界（契约类与强严重度仍按既有语义）

- **WHEN** 同一 finding 的 category 为契约类缺口或自相矛盾，或 `severity` 为 `must_fix`/`blocking`，或 category 缺失或未知，或该 finding 带机器修复许可或 owner 问句
- **THEN** 平台 MUST NOT 适用该窄例外，MUST 按既有语义处理（契约类仍必关闭、强严重度维持既有分类行为、未知类别按既有 fail-closed 口径）

#### Scenario: 缺 disposition 时仅在窄例外命中时降级

- **WHEN** 某 finding **缺失** `disposition`（按 fail-safe 视为 `actionable`）
- **THEN** 平台 MUST 仅在该 finding 同时命中窄例外的全部条件时投影为 `advisory`；未命中时 MUST 继续按 fail-closed 的 `actionable` 处理，且历史记录 MUST 照旧可解析

## ADDED Requirements

### Requirement: 教学重驱载荷的完备性与降级可诊断性（REQ-WSC-32）

SC 候选编译失败进入教学重驱时，重驱载荷 SHALL 携带**定点修复目标**：每条可教学的 parse 语法类诊断 MUST 逐字保留 `code:line:message` 三段，并附**精确的缺失字段名**与**正确形状示例**，使作者无需自行推断「缺哪个字段、应长成什么样」。该事实块 SHALL 受有界预算约束（≤32,000 B，按完整 UTF-8 行计量）：超限时 SHALL 整条省略并**显式记账**（总数 / 呈现数 / 省略数），MUST NOT 切断半条诊断。被省略条目的完整诊断事实 MUST 另行留存于**不进 prompt** 的审计面以供完整取回。该事实块 MUST NOT 改变教学资格判定、MUST NOT 改变额外启动次数，MUST NOT 引入新的失败面。

#### Scenario: parse 诊断逐条带齐缺失字段与正确形状

- **WHEN** 候选编译失败产出多条可教学的 parse 语法类诊断，且这些诊断的 `message` 相同而缺失字段不同
- **THEN** 最终教学重驱 prompt MUST 能逐条区分它们，MUST 为每条附上精确的缺失字段名与正确形状示例

#### Scenario: 原始三段逐字保留

- **WHEN** 教学重驱 prompt 渲染 parse 诊断
- **THEN** 每条诊断的 `code:line:message` 三段 MUST 与编译器产出一致，MUST NOT 被改写、截短或重排

#### Scenario: 事实块超预算时整条省略并显式记账

- **WHEN** parse 诊断事实块的字节数超出该块预算
- **THEN** 平台 MUST 整条省略超限诊断（单条过大亦整条省略），MUST NOT 切断半条诊断，MUST 显式呈现总数 / 呈现数 / 省略数，且 MUST NOT 新增失败面或新增模型调用

#### Scenario: 被省略条目的完整诊断可从审计面取回

- **WHEN** 某条 parse 诊断因预算被省略
- **THEN** 其完整诊断事实 MUST 能在**不进 prompt** 的审计记录中完整取回（字段与形状均在），且该审计写入 MUST NOT 改变候选尝试的身份与失败码

#### Scenario: 教学资格与启动次数不受载荷变化影响

- **WHEN** 教学重驱载荷新增了字段与形状信息
- **THEN** 教学资格判定 MUST 与改前一致（不因载荷内容获得资格或失去资格），额外启动次数 MUST NOT 增加，窗口与 claim/启动分账语义 MUST NOT 改变

#### Scenario: 未知结构化键附合法键集

- **WHEN** 失败类别为未知结构化键
- **THEN** 教学重驱 prompt MUST 附上该结构化段允许的合法键集合，MUST NOT 附带其它结构化段的键集，且 MUST NOT 由此引入新的失败面
