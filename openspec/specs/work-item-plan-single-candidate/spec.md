# work-item-plan-single-candidate Specification

## Purpose

Work Item Plan 以「单候选计划事务」交付：LLM 与人只接触 markdown/EARS 源文档，机器只消费确定性编译的 typed IR；reviewer 提供证据但不驱动状态机；全流程复用阶段 1 的唯一人工门与终态矩阵，自动化运行可在无人值守下到达终态。

## Requirements

### Requirement: 单候选计划事务（REQ-WSC-01）

系统 SHALL 将 workitem 段对外流程压缩为 prepare → generate → evaluate → approval → completed 五类可见状态，另加吸收态 failed

#### Scenario: 自动化 campaign 无人工干预到达终态

- **WHEN** 以 `auto_if_valid` 策略运行且候选通过全部机械校验、无未决 human_required finding、无重复指纹
- **THEN** 系统直接完成原子 compile 并发布 Confirmed plan 与 Work Items，全程不需要任何 WS 决策消息

#### Scenario: 生成模式不再询问

- **WHEN** 候选计划生成需要选择 batch 或 serial 执行策略
- **THEN** 运行时依据模型能力与候选规模内部选择，不产生 `select_work_item_generation_mode` 类型的对外决策请求

#### Scenario: interactive 门内多轮修订后批准

- **WHEN** 以 `interactive` 策略运行且 reviewer 判定需要人工处理，人工门打开后人在门内连续给出多轮 typed 反馈
- **THEN** 每轮反馈经 SC 专属修订路径产出通过校验的新候选并回呈；人 approve 后门关闭、确定性 compile 成功后 Plan 进入 durable Confirmed，全程不经过 legacy 逐段确认消息

#### Scenario: Confirmed 后经 advance 进入 coding 就绪

- **WHEN** Plan 已 durable Confirmed 且客户端显式调用 `advance`
- **THEN** 系统建立该 plan 唯一的 WorkItemGroup coding attempt 并返回 group workspace 入口（ready-only，不启动 coding provider）；advance 语义以 `work-item-plan-advance` capability 为唯一来源

#### Scenario: 批次状态可观察但不恢复逐段协议

- **WHEN** 单候选 plan 成功或失败恢复路径进入整组 Work Item Draft 确认状态
- **THEN** Cockpit 显示一个整组确认门及其可用 typed 动作，不显示逐段确认或 generation-mode 决策，不改变单候选状态机

#### Scenario: Compile recovery 可观察且不绕过 Confirmed/advance

- **WHEN** 单候选 plan 处于可恢复 Final Compile 状态
- **THEN** Cockpit 显示 recovery 门并允许既有 recovery action；仅服务端既有 compile 结果可使 plan 进入 Confirmed，Cockpit 不发送隐式 `advance` 或 coding 启动

#### Scenario: 逐段决策不暴露

- **WHEN** workitem 段流程面向 WS/UI 呈现决策
- **THEN** outline/draft/batch 的逐段确认、逐段 review、生成模式选择 SHALL NOT 暴露为用户可见或可应答的 WS/UI 决策

#### Scenario: 生成模式运行时自选

- **WHEN** 运行时确定生成模式（batch/serial）
- **THEN** 生成模式（batch/serial）SHALL 由运行时按模型能力与候选规模自行选择

#### Scenario: 人工门对话式升级

- **WHEN** interactive 策略下进入人工门
- **THEN** interactive 策略下的人工门 SHALL 升级为对话式多轮协议（typed 反馈回合、SC 专属修订路径、approve→compile→Confirmed 关门链），其回合、预算、幂等与恢复语义以 `work-item-plan-conversational-gate` capability 为唯一来源

#### Scenario: auto_if_valid 语义不变

- **WHEN** 以 `auto_if_valid` 策略运行
- **THEN** `auto_if_valid` 策略语义不变

#### Scenario: 批次与恢复可观察

- **WHEN** Work Item Plan 进入批次确认或 Final Compile recovery 状态
- **THEN** 在既有单候选计划事务与对话式人工门语义不变的前提下，Work Item Plan 的批次确认与 Final Compile recovery 状态 SHALL 在 Cockpit 具有可观察、可定位的待处理投影

#### Scenario: 投影不恢复旧协议

- **WHEN** Cockpit 呈现该投影的展示与动作
- **THEN** 该投影 MAY 使用批次/recovery 专属展示和既有 typed 动作，但 MUST NOT 恢复 outline/draft/batch 的 legacy 逐段确认、逐段 review 或 generation-mode 对外决策协议

#### Scenario: 确认后仍须显式推进

- **WHEN** Plan 已 durable Confirmed 且 Cockpit 门可见
- **THEN** Plan 已 durable Confirmed 后仍须由客户端显式调用 `advance`，Cockpit 投影不得将可见门误作 coding 已启动

### Requirement: markdown 编译器模型（REQ-WSC-02）

系统 SHALL 以 markdown/EARS 文档作为 work item plan 的唯一可编辑源，并提供确定性编译器将其单向编译为顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`

#### Scenario: markdown 与 IR 漂移被拒绝

- **WHEN** 发布前 typed IR 的 source_revision_hash 与当前 markdown 源不匹配，或 compiler_version 过期
- **THEN** 系统拒绝发布，提示重新编译；已发布的 binding 不受影响

#### Scenario: 编译错误可诊断

- **WHEN** markdown 源不满足 grammar（缺 section、ID 格式错误、EARS 句式非法）
- **THEN** 编译器返回行号、字段名与一个修复示例；该错误信息可直接作为返修反馈回喂模型

#### Scenario: 固定词表中文结构标题确定性归一化后可编译

- **WHEN** provider 交付的 markdown 把固定词表结构标题翻成已知中文变体（如 `# 工作项计划`、`## 工作项 WI-001: x`、`### 身份`、`### 写入策略 (Write Policy)`），且正文其余部分满足语法
- **THEN** 系统在编译前把结构标题行逐字归一化为规范英文并编译通过，正文中文逐字保留，并记录一条归一化诊断/事件；表外未知标题（如 `### 溯源清单`）不被猜测改写，仍按既有语法契约 fail-closed 拒绝

#### Scenario: EARS 关键字缺空格确定性补齐后可编译

- **WHEN** provider 交付的 statement 条件以 CJK 标点（如全角引号 `」`）收尾后无半角空格直接接 `THE SYSTEM SHALL`，或 `WHEN` 后缺半角空格，或关键字邻位为 U+3000/NBSP
- **THEN** 系统在编译前确定性补齐/归一为单个半角空格后编译通过；条件文本与标点本身逐字保留，并记录一条归一化诊断/事件；除关键字邻位空白外任何正文内容不被改写

#### Scenario: 归一化不救非空白语法错误

- **WHEN** statement 的语法错误不是关键字空白形态（如缺 WHEN 前缀、THEN 语义缺失、顺序错误）
- **THEN** 归一化层不修改该行，编译器照旧以 fail-closed 拒绝并返回行号诊断

#### Scenario: 重复能力行累积不丢能力

- **WHEN** 同一 Outputs contract 的 markdown 含 N 行 `capabilities`（如 F-52 CT-001 的 6 行复合/独立能力混排）
- **THEN** IR 中该 contract 的 capabilities 为全部 N 行 split 值的并集（完全相同值只保留一次），coverage 校验不再因行序或行数产生缺口；真实缺能力仍报 Error

#### Scenario: 重复需求行同构累积

- **WHEN** 同一 Inputs contract 含多行 `required_capabilities`
- **THEN** 与 capabilities 同构累积并集，不得 last-write-wins

#### Scenario: 相邻 contract 严格隔离

- **WHEN** lowering 顺序经过两个相邻 contract 且前者的字段行尚未闭合
- **THEN** 边界 flush 使前 contract 的字段不得归并到后 contract，反向亦然

#### Scenario: 累积不改写值形态

- **WHEN** 累积与去重生效
- **THEN** capability 值保持原文（顺序、大小写、标点、内部空白零改写），仅完全相同的重复元素被消除

#### Scenario: 重新编译的 IR 差异仅源于重复字段语义

- **WHEN** 含重复字段的相同 source 在修复前后各编译一次
- **THEN** 新旧 IR 的差异仅体现为重复字段的累积并集，无其他字段变化；历史 publication 不被重算或迁移

#### Scenario: options 缺口首轮即拦

- **WHEN** 创建选项 include_integration_tests=true 而候选首轮交付不含 integration 类 Work Item
- **THEN** generate/evaluate 期校验产出 Error 级 finding（附修复动作：新增对应 kind 或回创建选项关闭 flag），经既有回灌通道驱动修订；SHALL NOT 到 approve→Final Compile 才首次报错

#### Scenario: options 三族同报

- **WHEN** integration/e2e/split 三个 flag 同时启用而候选各有缺口
- **THEN** 一次校验报告全部缺口；flag=false 时保持既有 skipped-risk warning、不产生 Error

#### Scenario: AC 引用基线外路径被拦

- **WHEN** 候选的验收标准或验证计划引用仓库内文件路径（如某分支才存在的文件），而该路径不存在于 plan 基线树
- **THEN** generate/evaluate 期产出 Error（附路径清单与三种修复建议：删除该 AC / 改用基线内路径 / 将新建文件先提交入基线或改引基线内既有脚本，不以写范围声明豁免）；SHALL NOT 让 coder 在执行期发现并被迫越界恢复基线

#### Scenario: item 映射为 typed IR

- **WHEN** 编译器产出每个 item
- **THEN** 每个 item SHALL 为 `PlanCandidateItemIr { target_repository_id, contract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`

#### Scenario: 版本指纹仅顶层

- **WHEN** 检查 typed IR 中 source revision hash 与 compiler version 的位置
- **THEN** typed IR 的 source revision hash 与 compiler version 仅位于顶层

#### Scenario: 发布前校验指纹

- **WHEN** **publish 前** hash 或版本不匹配
- **THEN** **publish 前** hash 或版本不匹配时系统 SHALL 拒绝发布并提示重新编译，hash/version 随不可变 publication provenance 落盘

#### Scenario: 执行只消费发布绑定

- **WHEN** coding 段执行期间取用计划内容
- **THEN** coding 段只消费已发布的 immutable runtime binding，SHALL NOT 在执行期间解析 markdown 或重新解释 compiler version

#### Scenario: 安全边界强类型

- **WHEN** 编译产出承载 write_policy 与 trusted commands
- **THEN** write_policy 与 trusted commands 等安全边界 SHALL 保持强类型

#### Scenario: 修改触发重编译

- **WHEN** 对 markdown 的人工或模型修改发生
- **THEN** 对 markdown 的人工或模型修改 SHALL 产生新 revision 并触发重新编译

#### Scenario: 结构标题行归一化

- **WHEN** SC 交付（author 与人工修订两条路径）进入编译前
- **THEN** SC 交付（author 与人工修订两条路径）进入编译前，系统 SHALL 允许两类确定性归一化：其一为结构标题行（一级文档标题、Work Item 二级标题、三级结构 section 标题）归一化——以固定中文→英文映射表逐字映射回规范英文，仅覆盖固定词表的已知翻译变体

#### Scenario: EARS 空白归一化

- **WHEN** statement 行内 `WHEN` 后或 `THE SYSTEM SHALL` 前缺失半角空格、或关键字邻位为 U+3000/NBSP
- **THEN** 其二为 **EARS 关键字空白归一化**——当 statement 行内 `WHEN` 后或 `THE SYSTEM SHALL` 前缺失半角空格、或关键字邻位为 U+3000/NBSP 时，确定性补齐/归一为单个半角空格

#### Scenario: 归一化零触碰正文

- **WHEN** 两类归一化执行
- **THEN** 两类归一化均只触碰上述空白与固定词表位置，正文其余内容（含 CJK 标点如全角引号）零触碰

#### Scenario: 未知标题不猜测

- **WHEN** 结构标题行归一化遇到表外未知标题
- **THEN** 表外未知标题 SHALL NOT 被猜测改写，SHALL 照旧交给编译器以 fail-closed 拒绝

#### Scenario: 归一化落诊断

- **WHEN** 任一归一化发生
- **THEN** 任一归一化发生时系统 SHALL 落一条可判定该次交付被救回的诊断/事件

#### Scenario: 重复字段累积并集

- **WHEN** lowering 处理同一 Outputs contract 的多行 `capabilities`（或同一 Inputs contract 的多行 `required_capabilities`）
- **THEN** **lowering 重复字段累积语义**：同一 Outputs contract 的多行 `capabilities`（或同一 Inputs contract 的多行 `required_capabilities`）SHALL 按出现顺序累积为全体行的并集（每行 split 后的值依次 append），完全相同的 capability 值仅保留首次出现

#### Scenario: 禁止末行覆盖

- **WHEN** lowering 累积多行重复字段
- **THEN** SHALL NOT 以最后出现的行覆盖先行行（last-write-wins 禁止）

#### Scenario: 相邻契约严格隔离

- **WHEN** lowering 经过相邻 contract 的字段
- **THEN** 相邻 contract 的字段 SHALL 严格隔离（contract 边界 flush），不得跨 contract 归并或串项

#### Scenario: 累积不改原文形态

- **WHEN** 累积与去重执行
- **THEN** 累积与去重 SHALL 不改变 capability 值的原文形态（不排序、不改大小写/标点/内部空白）

#### Scenario: 校验携带存储选项

- **WHEN** 构建 SC 候选校验上下文
- **THEN** **候选校验携带存储 options 与基线树**：SC 候选校验上下文 SHALL 携带创建计划时落库的 `IssueWorkItemPlanOptions` 与 plan 基线（worktree fork base / target ref 树），SHALL NOT 从 IR items 反推 options

#### Scenario: 选项一致性前移

- **WHEN** 校验 options×items 一致性（integration_work_item_required / e2e_work_item_required / frontend_backend_split_required 三族）
- **THEN** options×items 一致性（integration_work_item_required / e2e_work_item_required / frontend_backend_split_required 三族）SHALL 在 generate/evaluate 期校验，Error 级结果经既有机械 ReviewVerdict 回灌修订，SHALL NOT 延迟至 Approval/Final Compile 才首次出现

#### Scenario: 基线路径交叉核对

- **WHEN** 验收标准（AC）/验证计划引用仓库内文件路径
- **THEN** 验收标准（AC）/验证计划引用的仓库内文件路径 SHALL 与基线树交叉核对：引用基线中不存在的路径 SHALL 产生 Error（附路径清单与修复建议），SHALL NOT 进入人工确认后才由 coder 发现

### Requirement: 中央策略层与 typed outcome（REQ-WSC-03）

系统 SHALL 提供中央策略层（在阶段 1 `workitem-typed-outcome-policy` 落地，本 change 复用），将每次评估结果归入四类 typed outcome：`valid`、`repairable`、`human_required`、`fatal`。reviewer 的 verdict 与 severity SHALL NOT 直接驱动状态跳转；策略层依据机械校验结果、finding 归类建议、指纹与预算做确定性裁决。机械校验 SHALL 覆盖 options×items 一致性与 AC 路径×基线树核对（REQ-WSC-02）；此类缺口 SHALL NOT 延迟至 Approval 阶段才首次出现。

#### Scenario: reviewer 发现语义矛盾

- **WHEN** reviewer 产出 must_fix 级语义矛盾 finding（如 non_goals 与任务自相矛盾）
- **THEN** 策略层归类为 `repairable`，在尚未消费全局聚合自动返修预算时触发一次携带该 finding 的聚合自动返修；findings 完整落盘可查

#### Scenario: 相同问题重现即终态

- **WHEN** 返修后同一 finding 指纹再次出现
- **THEN** 交互模式进入唯一人工门 awaiting_human，auto 模式以 stopped_needs_human 终态落盘；不自动再试、不标 fatal，输出完整诊断（finding、证据、已尝试次数）

#### Scenario: 预算耗尽

- **WHEN** 整次运行的 repair budget 已耗尽且仍存在自动可修复问题
- **THEN** 按阶段 1 终态矩阵：交互模式进入 awaiting_human，auto 模式以 stopped_needs_human 终态落盘；不标 fatal、不空转，产物与诊断全部落盘

#### Scenario: transition budget 耗尽

- **WHEN** 整次运行的 transition budget 耗尽
- **THEN** 系统以 fatal 终态停止，产物与诊断全部落盘，不出现无限循环

### Requirement: 运行策略持久化（REQ-WSC-05）

系统 SHALL 在 workspace session 创建时写入并持久化运行策略（`interactive` 手动最终批准，或 `auto_if_valid`），运行期间 SHALL NOT 接受客户端变更策略的请求。

#### Scenario: 策略创建时固定

- **WHEN** 运行进行中客户端试图变更审批策略
- **THEN** 系统拒绝并返回协议错误

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

### Requirement: 旧协议退役与单路径收敛（REQ-WSC-08）

REQ-WSC-07 退役门已按 `legacy-protocol-retirement` REQ-RET-01 全口径重测解锁后(2026-09-19 用户终裁 B:pi 全子项达标=协议质量实证;codex Confirmed 子项登记已知例外——系统性 provider 内容缺陷与协议无关,门文本据此显式修订,属 1c 裁决预留的「后续专项裁决」路径显式行使;**后续义务:codex/claude_code/kimi_code 全部 provider 最终 SHALL 全测通过(DEF-PVR-ALL),在义前不得视为 provider 面收官**),旧协议(generation-mode 决策、逐段确认消息、review_decision 双选项语法、`HumanConfirmDecision` 旧枚举及其消息族、SelectRevisionPath 族及其专属 DTO)SHALL 删除

#### Scenario: 已删除消息协议错误拒绝

- **WHEN** 退役完成后客户端发送任一已删除的 legacy 决策消息(generation-mode 决策、逐段确认、review_decision 双选项、human_confirm)
- **THEN** 系统返回 stage-specific protocol error 且零副作用,会话状态与事件流不变

#### Scenario: 多仓 preflight 按勾选上界放行

- **WHEN** 多仓 Issue 的 selection.focus_repository_ids 非空(≥2 或 =1),design involved 为 focus 集的非空子集(含全集)
- **THEN** 确定性 preflight 通过,plan 事务按单候选流继续

#### Scenario: 多仓 preflight 失败收敛新路径终态

- **WHEN** 多仓 Issue 的确定性 preflight 失败(无论新路径是否已产生副作用)
- **THEN** 失败以新路径 durable fatal/recoverable 终态记录并含原因,系统不存在任何 legacy fallback 或 `flow_kind` 切换路径

#### Scenario: 多仓 preflight 界外/空集确定性失败收敛终态

- **WHEN** design involved 为空集,或含 focus 界外成员(preflight 无论新路径是否已产生副作用)
- **THEN** 失败以新路径 durable fatal/recoverable 终态记录并含原因,系统不存在任何 legacy fallback 或 `flow_kind` 切换路径

#### Scenario: 新会话一律单候选流

- **WHEN** 退役完成后创建任意 workitem workspace 会话
- **THEN** 会话走单候选流(prepare → generate → evaluate → approval → completed),不存在 legacy 逐段路径选项

#### Scenario: 历史 legacy 记录只读保留

- **WHEN** 退役完成后读取历史 legacy session 的 durable 记录
- **THEN** 记录与事件前缀原样保留可读,未被迁移或清洗

#### Scenario: design involved 界内收敛(钉定上界)

- **WHEN** issue 勾选 N 个成员,AI 生成的 design involved 实际只含其中 M 个(1 ≤ M ≤ N)
- **THEN** sentinel 输出的 involved/change_order 均 ⊆ 勾选集且 involved 非空;界外仓出现在 sentinel 时系统 SHALL 拒绝该回写并以可见诊断提示走修订反馈

#### Scenario: 新会话单候选收敛

- **WHEN** 旧协议删除后创建新会话
- **THEN** 删除后:新会话 SHALL 一律走单候选流(prepare → generate → evaluate → approval → completed,另加吸收态 failed)

#### Scenario: 上界确定性放行

- **WHEN** 多仓 Issue 的确定性 preflight 计算 selection resolved 上界
- **THEN** 多仓 Issue 的确定性 preflight SHALL 按 selection **resolved 上界**放行——上界=focus_repository_ids 非空时取该集,为空(存量 AllMembers)时取 resolve_effective_members 有效成员集

#### Scenario: involved 子集校验

- **WHEN** preflight 以 design involved 对照上界判定
- **THEN** design involved 为上界的子集且(非空,或空且上界恰一仓的回退口径成立)时通过,其余情形确定性失败

#### Scenario: 失败落终态无回退

- **WHEN** preflight 判定为确定性失败
- **THEN** 失败以新路径 durable fatal/recoverable 终态记录并含原因,系统不存在任何 legacy fallback 或 `flow_kind` 切换路径

#### Scenario: items 限于 involved

- **WHEN** 校验 plan items 的 `target_repository_id` 范围
- **THEN** plan items 的 `target_repository_id` SHALL ⊆ design involved(下游按 target 分流为 MTG 已验收现状)

### Requirement: generate 段编译语法失败的教学重驱（REQ-WSC-09）

SC 候选编译（generate 段）失败时，系统 SHALL 在会话进入终态前提供恰好一次教学重驱，该机会以前置授权、合法 provider 与剩余机会为条件，混合 operational 集合 MUST NOT 获驱。资格 MUST 由 typed 判据决定：parse 语法类、事实完整的 typed lowering fault、四类 typed IR 诊断（unknown_done_when_ref、unknown_reviewer_check_ref、acceptance_criterion_without_reviewer_check、unknown_requirement_ref）与确定性结构拒收；数据库/基线/成员解析失败为 Operational 不获资格；新增类别 MUST 显式进入规格/typed 白名单。资格 MUST NOT 取决于 message 措辞；重驱 prompt MUST 渲染 typed 事实，仍失败按既有失败路径终态。每 candidate 总额外驱动 ≤1，既有上限不变。

#### Scenario: invalid_ears 触发一次教学重驱

- **WHEN** 候选编译产出 invalid_ears 且为该 candidate 首次编译失败
- **THEN** 系统以行号+错误信息回灌 author 重驱一次；重驱交付通过编译则继续既有链路，仍失败则终态（不再二次重驱）

#### Scenario: 事实完整的 lowering fault 获得一次重驱

- **WHEN** 候选因携带字段名与期望形态的 typed lowering fault 编译失败，且为首次编译失败
- **THEN** 系统 MUST 以 typed 事实回灌 author 重驱一次，MUST NOT 立即终态

#### Scenario: 无 typed fault 的 lowering 失败仍不重驱

- **WHEN** lowering 诊断不携带 typed fault，仅 message 含「缺少」等措辞
- **THEN** 系统 MUST NOT 授予重驱资格，按既有终态路径处理

#### Scenario: 重驱上限不被突破

- **WHEN** 同一 candidate 的编译失败类别先后命中两个可教学类别
- **THEN** 教学重驱总共仍至多一次，第二次失败直接按既有失败路径处理

#### Scenario: parse 语法类按码表获驱

- **WHEN** 候选编译产出 missing_section、invalid_ears、invalid_work_item_id 或 unknown_structured_key，且为该 candidate 首次编译失败
- **THEN** 系统按既有 parse 教学链路以行号与错误信息回灌 author 重驱一次

#### Scenario: 四类 typed IR 诊断获得重驱且载荷完整

- **WHEN** 候选产出 `unknown_done_when_ref`、`unknown_reviewer_check_ref`、`acceptance_criterion_without_reviewer_check` 或 `unknown_requirement_ref` 之一，且每条携带 Work Item、task/check/criterion 定位、错误 ref、本 WI 合法集合或真实 spec 集合、原行（无行即「不可定位」）与所需字段形态
- **THEN** 系统 MUST 以该 typed 载荷回灌 author 重驱一次；载荷不全或无原行时 MUST NOT 授予资格

#### Scenario: operational 失败混入不获重驱

- **WHEN** 失败集合含数据库、基线或成员解析等 Operational 失败，另含可教类别
- **THEN** 系统 MUST NOT 以 `any(可教)` 放行，MUST NOT 授予重驱，按既有 Operational 路径处理

#### Scenario: 未入白名单的类别不自动获权

- **WHEN** 某失败 code 带可教前缀或 message 含「缺少」措辞，但未显式进入规格/typed 白名单且事实不完整
- **THEN** 系统 MUST NOT 授予重驱资格，按既有失败路径处理

<!-- 2026-07-31 owner 裁定「复杂内容先不做」：**本 change 只保留未交付条款**（四层处置 / typed lowering fault / 目标分支等价规范化 / 精确缺口 / prompt 体量闸门 / 教学重驱资格改为 typed 完备）。
     本轮已交付条款已拆到 `fix-work-item-group-plan-parity` 归档：判定一致（REQ-WSC-23 ≡ REQ-WG-01，重复已删）、环诊断带来源（REQ-WSC-24）、尝试可序列观测（REQ-WSC-25）、owner 回显与一次答完（REQ-CG-08/09）、反查单一来源（REQ-WG-02）、可执行选项（REQ-WG-05）、修复指令携带选项与保护项（REQ-WSC-06 ≡ REQ-WG-06）、取值来源按会话类型唯一（REQ-WG-08）。 -->

### Requirement: existing/create 计划意图与停等（REQ-C1-PLAN-01）

plan contract SHALL 明确每个需要写入的 Work Item 是 `existing` 还是 `create`，并携带对应 provider Work Item 身份以及可校验的 exclusive/forbidden scope 约束。`existing` SHALL 仅引用可解析且授权的既有目标；`create` SHALL 仅在计划明确声明、依赖和作用域均可验证时成立。缺失意图、错拼身份、缺少依赖或越权 create SHALL fail-closed，并向驾驶舱/系统通知说明是“未声明”还是“不能执行”，提供“修订计划／重新反馈”操作；系统 MUST NOT 自动扩大写范围或将 git 基线存在性当作 create 授权。

#### Scenario: 合法 create 通过编译

- **WHEN** plan contract 明确声明 create、provider Work Item、依赖和允许 scope，且校验通过
- **THEN** plan 可进入确定性 compile，create 意图和约束在后续 binding 中可追溯

#### Scenario: 未声明 create 停等修订

- **WHEN** provider 输出涉及不存在路径但 plan 未声明该项为 create
- **THEN** 系统明确提示“未声明”，进入 feedback/修订等待，不自动新增 scope、不批准 compile

#### Scenario: 错拼或越权 create 被拒

- **WHEN** create 的 provider Work Item 身份、依赖、exclusive/forbidden scope 或 target 不匹配
- **THEN** 系统明确提示“不能执行”及冲突事实，拒绝 compile 并提供受限修订操作，计划既有 finding 不被清空

### Requirement: 写范围必须落在被授权仓 checkout 内（REQ-WSC-10）

多仓（LC 聚合根）plan 会话的候选机械校验 SHALL 逐 work item 核对 `write_policy.exclusive_scopes` 的路径域归属：带固定路径锚点的写范围 SHALL 落在本项被授权成员仓（IR `target_repository_id`）的 checkout 目录之内。越界（聚合根非成员区或其他成员 checkout）SHALL 产出稳定错误码 `write_scope_outside_bound_repository` 的 Error finding（preflight 族，经既有机械 ReviewVerdict 回灌修订）；核对只判归属、SHALL NOT 判文件存在性；无固定锚点的 scope 与空列表 SHALL NOT 触发；前缀匹配 SHALL 复用平台既有 scope 匹配权威。

#### Scenario: 聚合根非成员区写范围被拦

- **WHEN** 多仓会话候选中某 work item 绑定成员仓 busi（checkout `busi/`），其 exclusive 写面声明 `scripts/dev.sh`（聚合根本地文件区，不属于任何成员 checkout）
- **THEN** generate/evaluate 期产出 Error finding，code 为 `write_scope_outside_bound_repository`，消息点名该路径与「聚合根非成员区」归属事实并附修复建议；finding 属 preflight 族，报告保留并经既有机械回灌通道驱动修订

#### Scenario: 其他成员 checkout 的跨成员写范围被拦

- **WHEN** 某 work item 绑定成员仓 busi，其 exclusive 写面声明 `frontend/src/**`，而 `frontend/` 是另一成员仓的 checkout
- **THEN** 产出同码 Error finding，消息点名该写范围实际落在哪个成员 checkout 与本项被授权仓，按既有授权语义（每 WI 恰落一被授权成员仓）判越界并附修复建议

#### Scenario: 被授权 checkout 内新建路径不误伤

- **WHEN** 某 work item 绑定成员仓 busi，其 exclusive 写面声明 `busi/src/new_feature/**`，该路径不存在于 plan 基线树（纯新建写面）
- **THEN** 归属核对放行，不产出本规则 finding（只判归属不判存在）

#### Scenario: 单仓/Manual 会话零回归

- **WHEN** 候选校验上下文未提供成员 checkout 前缀表（legacy 单仓或非 enrolled/Manual 会话）
- **THEN** 本规则不触发，既有校验行为与 finding 集零变化

#### Scenario: 成员映射缺失 fail-closed

- **WHEN** 成员 checkout 前缀表在场，而某 work item 的被授权仓不在表内（成员归属映射缺失/漂移），无法确定其 checkout 归属
- **THEN** 按仓内既有 fail-closed 惯例产出 Error finding 说明「无法核对归属」的理由，SHALL NOT 以跳过吞错或降级放行

#### Scenario: preflight verdict 携带修复动作

- **WHEN** 本规则产出 Error finding 并被适配为机械返修 verdict
- **THEN** finding 的 required_action 与共享修复文案常量逐字同源（REQ-WSC-06 口径一致纪律），指引把写范围改写为被授权成员 checkout 内的聚合根相对路径或移交给拥有该路径的 work item

### Requirement: 面向 owner 的人工裁决呈现义务（REQ-WSC-11）

reviewer 输出 `verdict=needs_human` 时，系统 SHALL 要求其 findings 至少一条同时携带 `owner_question`（一句话人话提问：卡在哪 + 为什么要你决定）与 `owner_options`（**至少 2 个**互斥选项，每项含非空 `label` 与 `consequence`；不设上限）。二者 SHALL 用日常语言表达，MUST NOT 以内部字段名、枚举值、指纹或追踪 ID 作为唯一呈现。缺失或空白 SHALL 判结构化输出不合规，并按既有 `structured_output_diagnostic` 通道收口。`pass` 与 `revise` SHALL NOT 被要求提供这两个字段，其呈现 MUST 保持不变。

#### Scenario: needs_human 带选项 → 界面呈现人话问题与选项

- **WHEN** 某轮 reviewer 输出 `verdict=needs_human`，且其一条 finding 携带 `owner_question="WI-004 还要不要继续负责 scripts/dev.sh 的改动？"` 与两个 `owner_options`（各含 `label`/`consequence`）
- **THEN** 结论卡**与人工门卡主区** SHALL 先呈现该人话问题、逐个选项的 `label` 与 `consequence`，以及「把选项名原文作为补充指令发回去」的指引（门卡是 owner 真正动手的地方，不得只给技术原文）
- **AND** 该轮技术 findings（`message`/`evidence`/`required_action`）SHALL 折叠在「技术细节」内、默认收起，不被删除

#### Scenario: needs_human 缺字段 → 判不合规并走既有诊断通道

- **WHEN** reviewer 输出 `verdict=needs_human`，但没有任何 finding 同时携带非空 `owner_question` 与非空 `owner_options`（缺失、全空白或空数组）
- **THEN** 系统 SHALL 判该结构化输出不合规，诊断码为 `missing_owner_guidance`；有效选项仅 1 条（不构成互斥选择）同判不合规
- **AND** 该轮 SHALL 经既有 `structured_output_diagnostic` 通道落 `needs_human` 结论卡（保留原始输出预览与 reviewer comments），MUST NOT 新造第二套上报或展示机制

#### Scenario: 选项字段半截 → 判不合规

- **WHEN** 某条 finding 只填了 `owner_question` 或只填了 `owner_options`，或某个选项缺 `label`/`consequence`（trim 后为空）
- **THEN** 系统 SHALL 判结构化输出不合规（`invalid_finding_field`），不得把半截字段呈现给 owner

#### Scenario: pass 与 revise 呈现不变

- **WHEN** reviewer 输出 `verdict=pass` 或 `verdict=revise`（含 `revise_batch`、`plan_reopen_required` 等自动返修结论），且未携带人话字段（即全部契约合规 payload 与全部存量记录）
- **THEN** 系统 SHALL NOT 要求 `owner_question`/`owner_options`，结论卡与门卡 SHALL 沿用既有 findings 分组呈现，不出现人话裁决区，也不折叠技术 findings

#### Scenario: 要求写的人话字段必在至少一张卡可见

- **WHEN** 模型宣称 `verdict=needs_human` 并写出人话字段，但同轮存在 `blocking`/`must_fix` finding 使归一后 verdict 变为 `revise`
- **THEN** 该 payload 的人话字段 SHALL 至少在结论卡或人工门卡之一可见（呈现以「是否携带人话字段」为准，不以 verdict 字面为准），MUST NOT 出现「被强制要求写、却永不上屏」的死字段

#### Scenario: 存量 reviewer 消息重解析不被新判据拦下

- **WHEN** 系统从本 change 之前的 reviewer 消息文本重解析出 `needs_human`（该记录不可能带人话字段）
- **THEN** 历史重解析入口 SHALL 不强制人话字段，SHALL 保留既有 `summary` 与 `findings`（作者修订轮仍需待关闭 finding 清单）；live 入口对同一 payload 仍判不合规

#### Scenario: 复评轮漏写人话字段不被升级成协议失败

- **WHEN** 单候选 Verification scope 的复评轮模型输出 `needs_human` 但未写人话字段
- **THEN** 系统 SHALL 保留诊断码 `missing_owner_guidance` 并落 needs_human 结论卡，MUST NOT 升级为 `verification_scope_violation` / durable failed

#### Scenario: 全部面向 owner 的审查路径同一口径

- **WHEN** 任一 reviewer prompt builder 构建输出契约（通用 workspace 评审、单候选 WorkItemPlan outline 评审、Plan Projection 评审、Work Item Plan Outline 评审、单 Work Item Draft 评审、整 Batch 评审、单候选 Plan 评审）
- **THEN** 该 prompt SHALL 携带同一份面向 owner 的呈现硬约束，且其示例 schema SHALL 给出 `owner_question`/`owner_options` 字段形状，使各路径不出现「一类有人话、一类没有」的分叉

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

- **WHEN** 服务端可信会话确认为 WorkItemPlan 且为单候选，某 finding 的 `severity=Suggestion`、category 显式属于完备度/验证不可归因/其它、且 `class_hint=Repairable`，且该轮 verdict 为 `Pass`/`Revise`、无 `structured_output_diagnostic`、该 finding 无 `machine_repair`/`owner_question`/`owner_options`
- **THEN** 会话有效视图 MUST 将该 finding 投影为 `advisory`，MUST NOT 计入必关闭 finding 清单，MUST NOT 单独驱动作者返修，且该投影 MUST NOT 回写 durable 记录

#### Scenario: 窄例外的边界（契约类与强严重度仍按既有语义）

- **WHEN** 同一 finding 的 category 为契约类缺口或自相矛盾，或 `severity` 为 `must_fix`/`blocking`，或 category 缺失或未知，或该 finding 带 `machine_repair`/`owner_question`/`owner_options`，或该轮 verdict 为 `NeedsHuman`，或该轮携带 `structured_output_diagnostic`
- **THEN** 平台 MUST NOT 适用该窄例外，MUST 按既有语义处理（契约类仍必关闭、强严重度维持既有分类行为、未知类别按既有 fail-closed 口径、**原生 NeedsHuman 与带机械诊断的轮次仍按既有语义**）

#### Scenario: 缺 disposition 时仅在窄例外命中时降级

- **WHEN** 某 finding **缺失** `disposition`（按 fail-safe 视为 `actionable`）
- **THEN** 平台 MUST 仅在该 finding 同时命中窄例外的全部条件时投影为 `advisory`；未命中时 MUST 继续按 fail-closed 的 `actionable` 处理，且历史记录 MUST 照旧可解析

### Requirement: 返修触发路径只认 actionable（REQ-WSC-17）

非 advisory 指纹闸门、transition budget 与 `revise` 归一 MUST 与阻断判定同口径：`advisory` MUST NOT 进指纹集合、MUST NOT 驱作者返修、MUST NOT 占 transition budget；`revise` 但无任何 actionable finding 时 MUST 归一为可通过或人工确认。`contract_gap` / `self_contradiction` MUST NOT 仅凭模型标 `advisory` 获得免阻断。

#### Scenario: 指纹闸门与返修预算只认 actionable

- **WHEN** 同一轮 verdict 里既有 `advisory`（哪怕标成 `must_fix`/`human_required`）又有 `actionable`
- **THEN** 非 advisory 指纹集合与 transition budget MUST 只由 `actionable` 构成；只有 `advisory` 时该指纹集合 MUST 为空、MUST NOT 消耗返修预算

#### Scenario: revise 但无 actionable 不驱作者返修

- **WHEN** reviewer 输出 `verdict=revise`，但全部 finding 都是 `advisory`
- **THEN** 平台 MUST 归一为可通过（或人工确认）的结论，MUST NOT 进入作者返修通道，MUST NOT 因此消耗 transition budget

#### Scenario: 契约类缺口不得以 advisory 漏检

- **WHEN** reviewer 把 `category=contract_gap` 或 `category=self_contradiction` 的本 Work Item 缺口标成 `disposition=advisory`
- **THEN** 平台 MUST 按 `actionable` 处理，MUST 让它进入必关闭清单（判定在平台侧，不依赖模型自觉）

### Requirement: owner 排除项落 durable 并回喂下一轮评审（REQ-WSC-13）

门卡 SHALL 为每条 finding 提供结构化「排除此项」，身份 MUST 是平台算的内容 key（`sha256(severity + \u{1} + message)` 前 16 位小写 hex）。该 key MUST 由平台在 WS 出站与门卡 metadata 给出，模型 MUST NOT 生成；live 与 rebuild MUST 算出同一个 key。

owner 的排除 MUST 落到会话 durable 已处置事实清单（含 key、message 摘要、处置方式），同 key 重复提交 MUST 幂等。下一轮评审 prompt MUST 携带该清单并声明已处置事实不得再报为 actionable；只有拿得到上轮上下文的路径才携带。

`human_gate_feedback` 的 `excluded_finding_ids` 缺省或空 MUST 与改动前逐字相同。排除-only 形态（`feedback` 为空且至少一个合法 key）MUST 被受理：落 durable 即成立，MUST NOT 建修订 turn，MUST NOT 以「反馈为空」拒收。

#### Scenario: owner 排除一条后不再被报为 actionable

- **WHEN** owner 在门卡上对「中文 locale 尚未配置」这条 finding 点「排除此项」，随后再提交一次反馈
- **THEN** 该 finding 的 key 与摘要 MUST 落 durable 已处置事实清单，下一轮 reviewer prompt MUST 携带该清单并声明不得重复报为 actionable

#### Scenario: 未处置的事实下一轮仍可报

- **WHEN** owner 只排除了 A 条，B 条（本 Work Item 的验收标准缺口）未处置
- **THEN** 下一轮 reviewer MUST 仍可把 B 报为 `actionable` 并阻断

#### Scenario: 重启后清单仍可读

- **WHEN** 平台重启后再次为该会话构建评审 prompt
- **THEN** 已处置事实清单 MUST 从 durable 读回同一批条目，MUST NOT 依赖内存态

#### Scenario: live 与 rebuild 算出同一个 key

- **WHEN** 同一份 findings 先经 live 解析投影到 WS 出站，再经 durable verdict 反序列化重建
- **THEN** 两条路径 MUST 给出逐字相同的 `finding_key`，前端不需要自己实现算法

#### Scenario: 排除-only 帧被受理且不建修订 turn

- **WHEN** owner 在门卡上点「排除此项」，客户端发来 `feedback=""` 且 `excluded_finding_ids` 含合法平台 key 的帧
- **THEN** 平台 MUST 把该 key 落 durable 已处置清单并回受理回执，MUST NOT 返回 `INVALID_HUMAN_GATE_FEEDBACK`，MUST NOT 建修订 turn（零 provider 启动）

### Requirement: 重复报告必须说明未收敛原因（REQ-WSC-14）

reviewer 重复报告同一事实时 MUST 在 `message` 说明「上一轮处置为何没有解决它」。事实已被 owner 处置且未变化时 MUST NOT 再报为 `actionable`。

已处置事实 MUST 由**平台**裁决，而不只是注入 prompt 期望模型自觉：`finding_key` 命中已处置清单的 finding MUST 在平台侧按 `advisory` 处理——即便模型仍报 `actionable`，MUST NOT 因此驱作者返修、MUST NOT 进必关闭清单、MUST NOT 占 transition budget。

门 turn 以 `validation_reject` 失败时，平台 MUST 给出被拒反馈原文、可读原因（不是错误码）、至少一条下一步动作与逐字诊断，并 MUST 落 durable 使重启后可读。

作者返修输入面 SHALL 同口径：被降级的 finding MUST NOT 以 `message`/`required_action` 进入作者返修 prompt 的「必须修复」清单，至多「仅供提示，本轮不要修改」且无正文。

#### Scenario: 同一事实再次出现必须带未收敛说明

- **WHEN** reviewer 在第二轮再次报告第一轮已报告过的同一缺口
- **THEN** 该 finding 的 `message` MUST 说明上一轮处置为何没有解决它，MUST NOT 只是换措辞重复原文

#### Scenario: 已处置且事实未变不得报为 actionable

- **WHEN** 某事实已在 `[human_dispositions]` 清单里，且事实本身没有变化
- **THEN** reviewer MUST NOT 把它报为 `actionable`；若平台仍收到 `actionable`，该 finding 的处置归属以 owner 已处置事实为准，MUST NOT 因此把流程卡在返修循环

#### Scenario: 门 turn 被拒后 owner 能看到原因与下一步

- **WHEN** 门 turn 因 `validation_reject` 终态失败（例如 owner 的反馈无法被平台转成可执行修订指令）
- **THEN** 平台 MUST 给出被拒反馈原文、可读原因、至少一条下一步动作与逐字诊断，并 MUST 把它们落到该会话的 durable 记录，使重启后仍可复盘

#### Scenario: 模型仍报 actionable 也不驱返修

- **WHEN** 某事实已在会话已处置清单里，而某轮 reviewer 仍把它报为 `actionable`（措辞相同、key 命中）
- **THEN** 平台 MUST 把它按 `advisory` 处理——MUST NOT 进入返修通道、MUST NOT 进必关闭清单、MUST NOT 占 transition budget；该判定 MUST 不依赖模型是否照 prompt 听话

#### Scenario: 刷新后被拒原因仍可读

- **WHEN** owner 在门 turn 被拒后刷新页面或重连
- **THEN** 门卡 SHALL 仍能呈现最近一次拒收的四要素（来自 durable 记录的重建投影），MUST NOT 只剩一条失败行

#### Scenario: 降级后的 finding 不进入作者返修输入面

- **WHEN** owner 已排除事实 A，某轮 reviewer 再报 A + B（B 是本 Work Item 可收敛的缺口），平台已把 A 降级为 `advisory`
- **THEN** 作者返修 prompt MUST 只把 B 的 `message` / `required_action` 列入「必须修复」清单；MUST NOT 含 A 的 `message` / `required_action`，A 至多以「仅供提示，本轮不要修改」的无正文提示出现；上一轮 finding 身份清单同样 MUST 先按会话已处置清单过滤，同一份 prompt MUST NOT 既称其为「上一轮未收敛项」又称其为「已被排除」

### Requirement: 人工门在拒绝与 advisory 场景下必须可操作（REQ-WSC-15）

一次反馈触发的修订被判 `validation_reject` 时，门卡 SHALL 呈现被拒反馈原文、可读原因、至少一条下一步动作与逐字编译诊断；这些字段全缺席时 MUST NOT 渲染空白块（回落既有失败行）。门卡 SHALL 把 `disposition: "advisory"` 的 finding 与需 owner 处理的 finding 分组隔离，原因行「需要处理 N 项」SHALL 只计 actionable，advisory 的 `required_action` MUST NOT 呈现为要求 owner 执行的祈使样式。

门卡 SHALL 提供逐条 finding 的「排除此项」，仅当该 finding 带平台算的 finding key 时才渲染（旧记录无 key → fail-closed）；发起失败 MUST NOT 显示为已排除。排除成功后 SHALL 即时显示「已排除」与「下一轮评审不再报为需处理」的说明。

#### Scenario: 被拒后门卡给出四要素

- **WHEN** 门 turn 以 `validation_reject` 终态失败且 durable 拒收记录四要素齐备
- **THEN** 门卡 SHALL 逐字呈现被拒反馈、可读原因、下一步动作与诊断原文，owner 无需猜测「预判」是什么

#### Scenario: 四要素全缺席不渲染空白块

- **WHEN** 门卡 metadata 不含任何拒收字段（旧记录或非拒收失败）
- **THEN** 系统 MUST NOT 渲染空白或只有标题的拒收块，MUST 回落既有失败行呈现

#### Scenario: advisory 与 actionable 视觉可分且计数只计 actionable

- **WHEN** 本轮 findings 含 1 条 advisory 与 2 条 actionable 的 must_fix
- **THEN** advisory 落「仅供参考（不阻断）」组且 `required_action` 呈弱化提示样式；原因行 SHALL 写「有 2 条必须处理项」，MUST NOT 计入 advisory

#### Scenario: 无平台 key 的 finding 不渲染排除操作

- **WHEN** 某 finding 来自历史记录（无平台算的 finding key）
- **THEN** 门卡 MUST NOT 渲染该行的「排除此项」，MUST NOT 以无 key 的引用发起排除

#### Scenario: 排除成功即时可见

- **WHEN** owner 点击某行的「排除此项」且发送成功
- **THEN** 该行 SHALL 立即显示「已排除」状态，门卡 SHALL 说明「下一轮评审不再报为需处理」，MUST NOT 等到下一轮刷新才可见

#### Scenario: 排除发送失败不冒充已排除

- **WHEN** 排除动作的发送返回 false 或抛错（门面不可用/被阻断）
- **THEN** 该行 MUST NOT 显示「已排除」状态，MUST 保持可再次发起

### Requirement: owner 可以撤销已提交的排除（REQ-WSC-16）

owner 排错后 SHALL 能撤销某条 finding 的排除。撤销 MUST 删除该会话 durable 已处置清单里的该 key（幂等），此后该 finding MUST 恢复为「可报为 actionable」：下一轮 reviewer prompt MUST NOT 再携带它，平台裁决 MUST NOT 再把它降级，撤销 MUST NOT 影响清单里的其它条目。

#### Scenario: 撤销后该 finding 恢复可报为 actionable

- **WHEN** owner 撤销某条此前排除的 finding key
- **THEN** 该条目 MUST 从 durable 清单删除；下一轮 reviewer 把它报为 `actionable` 时 MUST 照常阻断（与从未排除一致）

#### Scenario: 撤销幂等

- **WHEN** 同一 key 被重复撤销，或该 key 本不在清单里
- **THEN** 清单 MUST 保持不变、平台 MUST NOT 报错

#### Scenario: 撤销入口形状

- **WHEN** 客户端向 `POST /api/workspace-sessions/{session_id}/human-actions` 发送
  `{"type":"revoke_finding_exclusions","command_id":"<id>","finding_keys":["<key>"]}`
- **THEN** 平台 MUST 返回 200 与 `{"command_id":"<id>","state":"accepted","remaining_finding_keys":[...]}`（撤销后仍生效的排除项）

### Requirement: 候选编译失败必须以完整可读原因呈现在人工门（REQ-WSC-18）

SC Final Compile 的失败 MUST 以**完整**诊断 durable 落盘（不截断）：稳定码、人话原因、诊断原文。门卡 `session_state` MUST 带 `candidate_compile_failure{code,message,detail?,owner_hint?,rewrite_guidance?}`，刷新/重连后仍在。只有**当前候选**（source revision 与 candidate IR 双侧与会话 durable 值逐字相等）的失败才呈现；旧候选失败与任一侧 ref 缺席 MUST 不呈现。

#### Scenario: 依赖环失败在门卡上完整可读

- **WHEN** Final Compile 以 `final_compile_prepare_failed` / `dependency_cycle` 失败并落人工门
- **THEN** 该会话 durable 落有 `candidate-compile-status.json`，`detail` 含 `failure_id` 与 typed 渲染原文（逐字、不截断）；`session_state.candidate_compile_failure.message` MUST 是人话（点名哪两个 Work Item、谁把谁写成 provider、为什么与仓序自动边合成环）

#### Scenario: 刷新后原因不消失

- **WHEN** owner 刷新页面或服务重建会话
- **THEN** `session_state.candidate_compile_failure` MUST 仍带同一 `code` / `message` / `detail`（来自 durable 读取，不依赖内存 stash）

#### Scenario: 旧候选的失败不得冒充当前候选

- **WHEN** author 已重新生成候选（source revision 或 candidate IR 变化），库中仍留有上一候选的失败记录
- **THEN** 门卡 MUST NOT 呈现该失败，确认判据 MUST NOT 因此拦截

#### Scenario: 编译成功不得残留过期失败

- **WHEN** 同一候选的编译尝试后来成功
- **THEN** durable 候选编译事实 MUST 被覆盖为「无失败」；门卡 MUST NOT 继续呈现「当前候选编译不过」

### Requirement: 候选编译不过时确认动作不得形成死路（REQ-WSC-19）

`human_confirm` 之后的 Final Compile 是必经步骤，确认动作必然重跑同一次编译。当前候选已确定编译不过时，平台 MUST fail-closed 拒绝放行（零 durable 副作用、门保持打开）并给出原因与出路；`candidate_compile_failure` 在场即是该判据的呈现面。判据 MUST 在平台侧，MUST NOT 依赖前端自行猜测。

#### Scenario: 确认当前版本被拒且给出出路

- **WHEN** 当前候选的 Final Compile 已失败，owner 仍提交「确认当前版本」
- **THEN** 平台 MUST 以稳定码 `CANDIDATE_COMPILE_FAILED` 拒绝（会话状态、相位、门快照、预算 MUST 全部不变），文案 MUST 说明「确认会原样再失败一次」并给出下一步（让 author 重新生成 / 终止）

#### Scenario: 两条确认入口同一判据

- **WHEN** 确认来自人工门 close（`close_human_gate` Approve）或既有 confirm 端点（SC Approval 臂）
- **THEN** 两条入口 MUST 走同一平台判据并给出同一稳定码，MUST NOT 出现一条放过、一条拒绝

#### Scenario: 无法证明时不拦

- **WHEN** 失败记录或会话记录任一侧缺少候选 refs，无法证明失败属于当前候选
- **THEN** 确认 MUST NOT 被拦截（fail-open）；平台 MUST NOT 凭猜测把门永久挡死

#### Scenario: 终止不受影响

- **WHEN** 当前候选编译不过，owner 选择终止
- **THEN** 终止 MUST 照常执行（fail-closed 只作用于放行确认，不阻断退出）

### Requirement: 候选编译类失败首次即提示 owner 并继续自动重写（REQ-WSC-21）

候选自身无效类失败（至少 `final_compile_prepare_failed` / `final_compile_structure_rejected`）首次发生时，平台 MUST 立即向 owner 呈现醒目提示与重写要点（不等重写结果），MUST 同时按剩余预算**真正驱动** author 重写（复用既有返修路由通道，不新造状态机）。预算 MUST 是**author 重写次数预算**（初值 `max_manual_repairs`，门卡上 owner 看到的数字同一口径），**平台自动重写与 owner 反馈触发的重写扣同一个池**，每轮消耗 1；编译成功 MUST 清零**该候选编译链上「平台自动重写」这一段的已计费消费**（同一 logical gate 内已由 owner 反馈消费或已预留的额度 MUST NOT 因此被返还，按 REQ-HGC-01 的同门接续与 REQ-CG-02 的预留 / 结算 / 退款既有语义处理）。`max_repairs`（机械返修池）MUST NOT 被当作自动重写预算。

#### Scenario: 第一次失败就出提示

- **WHEN** 候选第一次因依赖环失败
- **THEN** 门卡/气泡 MUST 立刻带 `candidate_compile_failure`（含 `auto_rewrite.phase="running"` 与剩余次数），MUST 说明「由于 design 的什么原因导致成环并无法推进、需要修改 design」，MUST 给出重写要点

#### Scenario: 提示不妨碍重写

- **WHEN** 提示已呈现且预算尚有余
- **THEN** 平台 MUST 继续自动重写，MUST NOT 因已提示而停止；剩余次数 MUST 随每次重写递减

#### Scenario: 预算用尽后停并交人工门

- **WHEN** author 重写次数预算（`max_manual_repairs`，含平台自动重写消耗）用尽
- **THEN** 平台 MUST 停止自动重写，MUST 以 `auto_rewrite.phase="stopped"`（`remaining=0`）呈现，并交人工门由 owner 决定

#### Scenario: 两个预算池互不挪用

- **WHEN** 平台自动重写触发一轮 author 重跑
- **THEN** MUST 消耗 author 重写次数预算（`max_manual_repairs` 口径，与 owner 反馈 turn 同一池）；MUST NOT 消耗 `max_repairs`（机械返修池，用于不重跑 author 的机械 repair）

#### Scenario: 非候选无效类失败不宣称自动重写

- **WHEN** 失败稳定码不属于候选自身无效族（如 store/IO 类 operational 失败）
- **THEN** 平台 MUST NOT 出现 `auto_rewrite` 字段，MUST NOT 声称已启动自动重写

#### Scenario: 自动重写与反馈互扣同一余额

- **WHEN** 平台自动重写已消耗 1 次 author 重写额度，且 owner 随后在同一 logical gate 内提交一次反馈
- **THEN** 平台 MUST 从**同一个** author 重写次数预算中再扣 1 次（可用余额如实递减，如 3→2→1）；MUST NOT 为两者各保留一份互不相干的余额；MUST NOT 因 owner 反馈而使自动重写侧的可用额度被重置或另算一份

#### Scenario: 余额只有一个权威事实

- **WHEN** 平台查询 author 重写次数预算的可用余额
- **THEN** MUST 只有**一个**权威余额（同一预算域、同一扣减序列）；MUST NOT 出现「自动重写侧余量」与「反馈侧余量」两个互不相干的数字；MUST NOT 由展示端对两个计数取 max / min / 相加推算余额；同一 command 重放 MUST NOT 重复扣减

#### Scenario: 编译成功只清编译链上的自动重写消费

- **WHEN** 候选编译成功
- **THEN** 平台 MUST 清零该候选编译链上「平台自动重写」的已计费消费（其诊断计数随之归零）；MUST NOT 返还同一 logical gate 内已由 owner 反馈消费或已预留的门反馈额度；该门反馈额度 MUST 继续按 REQ-HGC-01（同一 logical gate 内 carry-forward）与 REQ-CG-02（预留 / 结算 / 退款）的既有语义处理

#### Scenario: 平台自动重写不得消耗机械返修池

- **WHEN** 平台因候选编译失败触发一轮 author 重写（平台自动重写）
- **THEN** 该轮 MUST NOT 递增机械返修池的轮内计数（`repairs_used`）与会话观测计数；MUST NOT 因机械返修池已用尽而阻断平台自动重写；平台自动重写能否继续 MUST 只由 author 重写次数预算的可用余额决定

### Requirement: 同一依赖环重复出现必须提前停并升级提示（REQ-WSC-22）

自动重写若产出**同一个依赖环**，平台 MUST 判定 plan 层面无法修复，MUST 停止剩余自动重写预算，MUST 把提示升级为「自动重写无法解决，请修改 design」，并 MUST 交人工门。同一环的判据 MUST 是结构（成员集合 + 边集合的规范化摘要），MUST NOT 用文案、行号或编译运行 id。

#### Scenario: 同一环第二次出现即停（与预算无关）

- **WHEN** 第二次失败的结构指纹与上一次相同（即使 author 重写预算仍有剩余）
- **THEN** 平台 MUST 立即停止自动重写（不再消耗剩余预算），呈现 `auto_rewrite.phase="stopped"` 与 `repeated` 次数，并进人工门——预算管「总共能重写几次」，结构判据管「这次重写有没有意义」，两者分别成立

#### Scenario: 换措辞/改标题但同一组环仍判重复

- **WHEN** author 重写后 Work Item 标题或依赖描述换了措辞，但环的成员与边集合不变
- **THEN** 平台 MUST 判为同一个环（结构指纹相同），MUST NOT 当成新事实继续消耗预算

#### Scenario: 另一个环是新事实

- **WHEN** 重写后出现结构不同的另一个依赖环
- **THEN** 平台 MUST 允许按剩余预算继续自动重写，`repeated` MUST 从 1 重新计

#### Scenario: 指纹跨尝试稳定

- **WHEN** 同一组环在不同编译尝试（编译运行 id 不同）中再次出现
- **THEN** 结构指纹 MUST 相同（成员用「编译序序号 + target 仓」表达，不掺运行身份）

### Requirement: 依赖环诊断必须携带边来源并结构化教学（REQ-WSC-24）

依赖环失败 MUST 以 typed witness 表达，逐跳标注来源（canonical_input 或 change_order）及涉及的仓序位次。平台 MUST 据此生成结构化教学指令交原授权 author 重组，指令 MUST 含保留全部 Inputs 与验收标准、不得改仓序的保护项。同一结构指纹重复出现时 MUST 升级为业务取舍并一次问清。

#### Scenario: 失败面带逐跳来源

- **WHEN** Final Compile 因依赖环失败
- **THEN** 失败记录 MUST 含逐跳 from、to、sources 与被逆的两个仓序位次，MUST NOT 只留自由文本

#### Scenario: 同一结构环重复则升级

- **WHEN** 重写后产出结构指纹相同的环
- **THEN** 平台 MUST 停止自动重写并升级为业务取舍，指纹判据 MUST 沿用 REQ-WSC-22

### Requirement: 候选生成尝试可序列观测（REQ-WSC-25）

平台 MUST 以追加写记录每次候选生成与编译尝试，MUST NOT 只保留最近一次。每条记录 MUST 含尝试序号、source revision 引用、阶段、稳定码、typed 种类、环结构指纹、已应用的自动修复与等价规范化清单、处置结论。阶段 MUST 能区分 generate、parse、typed lowering、typed IR 校验与结构拒收；每条记录 MUST 关联同一 generation，若本次触发了 provider 启动则 SHALL 记录其 attempt 序号与 attempt key，使教学重驱与原调用出现在同一可追溯序列上，若本次未触发 provider 启动则 SHALL 显式记录「无 provider 启动」，MUST NOT 伪造关联。由此 MUST 可计算首次通过率、机械自修收敛率与模型重写次数。

#### Scenario: 连续失败可逐次追溯

- **WHEN** 同一 issue 连续发生多次候选失败
- **THEN** 平台 MUST 能列出每次失败所在阶段、稳定码与应用的自动修复，MUST NOT 只剩最后一次

#### Scenario: 记录读写失败不阻断生成

- **WHEN** 追加记录时发生写入错误
- **THEN** 平台 MUST 把该错误记入运行日志，MUST NOT 因此使候选生成或确认失败

#### Scenario: 阶段与 attempt 关联可追溯

- **WHEN** 同一 generation 发生原调用与一次教学重驱，且失败阶段跨越 parse 与 typed IR
- **THEN** 平台 MUST 能在同一流水序列上列出两个 attempt 各自的阶段、稳定码、source 引用与处置结论，MUST NOT 把两者合并为一条无法区分阶段的记录

### Requirement: 已收口的人工门卡不得再以行动号召形态呈现（REQ-WSC-26）

人工门卡一旦已收口（`resolved`），SHALL 以只读陈述形态呈现：陈述式标题、不渲染人话选项块与作答指引、决定收口时不渲染「为什么找你」meta 行，并在独立的结果徽章旁给出一句来源与后果说明。本 requirement 只约束已收口门卡；未收口的活门卡与 REQ-WSC-11 逐字不变。

#### Scenario: 已终止的 triage 门卡不再呈现行动号召

- **WHEN** 一张携带人话裁决（`owner_question` + ≥2 个 `owner_options`）的 triage 门卡 `resolved=true`、`resolution="terminate"`
- **THEN** 标题 SHALL 逐字为 `本轮分诊已结束`，MUST NOT 出现 `需要判断 reviewer 意图`
- **AND** MUST NOT 渲染 `owner-guidance` 块、`owner-option` 条目与「选一个，把选项名原文作为补充指令发回去即可」
- **AND** MUST NOT 渲染 `gate-meta` 行
- **AND** 「已终止」徽章 SHALL 是独立元素、文案恰为 `已终止`（`getByText("已终止")` 精确匹配只命中它）
- **AND** SHALL 另渲染 `data-testid="gate-resolution-note"`，文案逐字为 `这一轮已终止，之前的选择未生效，无需再操作`

#### Scenario: 有被中断修订证据的已终止卡明说选择未生效

- **WHEN** 门卡 `resolved=true`、`resolution="terminate"`，且时间线存在 `status=aborted`、`summary` 含 `修订被中断，你的选择未生效` 的节点
- **THEN** `gate-resolution-note` SHALL 逐字为 `这一轮修订被中断，你的选择未生效，无需再操作`
- **AND** 无该证据时 MUST NOT 宣称「修订被中断」

#### Scenario: 其余收口标题与说明逐字固定

- **WHEN** 一张门卡 `resolved=true`
- **THEN** 标题 SHALL 逐字为：triage 门 `本轮分诊已结束`；批次确认门 `本轮发布确认已结束`；其余 `本轮人工确认已结束`
- **AND** MUST NOT 使用命令式标题 `需要判断 reviewer 意图` / `需要人工确认` / `确认发布整组 Work Items`
- **AND** `resolution="confirm"` 的说明 SHALL 为 `已按当前版本确认，这一轮无需再操作`；`resolution="request-change"` 的说明 SHALL 为 `已要求修改，这一轮已结束，无需再操作`
- **AND** 「已确认」/「已要求修改」徽章 SHALL 保持独立元素且文案逐字不变

#### Scenario: 既有已关元素保持关闭

- **WHEN** 一张门卡 `resolved=true`
- **THEN** 需人工 chip、下一步指引、why 行、findings 折叠、距通过清单、跨轮 delta、动作区 SHALL 继续不渲染

#### Scenario: 留档卡保留历史 meta 行但同样去行动化

- **WHEN** 一张门卡 `resolved=true`、`resolution="superseded"`，携带人话裁决
- **THEN** 标题 SHALL 为陈述式，MUST NOT 渲染人话选项块与作答指引
- **AND** SHALL 保留既有 `gate-archive-note`、「已留档」徽章与 `gate-meta`（含「预算历史不可用」）

#### Scenario: 活门卡不受影响

- **WHEN** 一张门卡 `resolved` 不为 true，携带人话裁决
- **THEN** SHALL 照旧渲染命令式标题与人话选项块、作答指引（REQ-WSC-11 不变）

### Requirement: 人工门卡必须表达修订与复评的在途态且刷新重连后不得退化为全新可操作卡（REQ-WSC-27）

当前门在 single_candidate 流 `human_confirm` 阶段存在在途 provider run 时，门卡 SHALL 表达在途态并 MUST NOT 提供会被后端判 Busy 的提交通路。在途态由服务端权威事实派生：`active_run_id` 非空且活动节点 `status=active`，节点类型 `author_run` 为修订中，`reviewer_run` 为复评中，其余类型不判在途。

#### Scenario: 刷新后修订中的门卡不是全新可操作卡

- **WHEN** 刷新或 WS 重连后收到 `session_state`：`stage=human_confirm`、`flow_kind=single_candidate`、`active_run_id` 非空、活动节点 `author_run` 且 `active`，store 的 `humanGateTurn` 为 null，当前门卡未收口
- **THEN** 门卡 SHALL 渲染 `gate-revision-status`，文案逐字为 `已提交，正在按反馈修订`
- **AND** MUST NOT 渲染反馈编辑器（`gate-feedback-editor`）与「确认当前版本」按钮
- **AND** SHALL 仍渲染「终止此门」
- **AND** 顶部指引 SHALL 逐字为 `引擎正在按你的反馈修订，请稍候；完成后会自动复评`，不指向「确认当前版本」

#### Scenario: 复评阶段文案区别于修订阶段

- **WHEN** 同上，但活动节点为 `reviewer_run` 且 `active`
- **THEN** 状态行 SHALL 逐字为 `修订完成，正在复评`，MUST NOT 为 `已提交，正在按反馈修订`
- **AND** 顶部指引 SHALL 逐字为 `引擎正在复评修订后的版本，请稍候；结论出来后再决定`
- **AND** 反馈编辑器与确认按钮 MUST NOT 渲染

#### Scenario: run 结束后随节点终态自动解锁

- **WHEN** 在途 run 结束，活动节点经 `timeline_node_updated` 变为终态（`completed` / `aborted` / `failed`）
- **THEN** 在途态 SHALL 为空，门卡 SHALL 按既有规则恢复，MUST NOT 因陈旧的 `active_run_id` 永久锁住

#### Scenario: 已收口卡不被在途态重新激活

- **WHEN** 会话已 `terminated`（门收口，`resolution=terminate`），但 store 里 `active_run_id` 与 `active` 的 `author_run` 节点仍陈旧存在
- **THEN** 该卡 SHALL 仍为只读陈述形态（REQ-WSC-26），MUST NOT 渲染在途状态行、反馈编辑器或确认按钮

#### Scenario: live-turn 路径与不判在途的情形不变

- **WHEN** store 存在与本卡 `turn_id` 匹配的 open / busy / awaiting_confirm `humanGateTurn`，或活动节点缺失、非 `active`、类型不是 `author_run` / `reviewer_run`，或 `active_run_id` 为空
- **THEN** 在途态选择器 SHALL 为空，门卡 SHALL 按既有路径渲染，行为不变
- **AND** 已知边界：`active_run_id` 为进程内字段，服务进程重启后为空，重启丢失的在途 turn 由既有恢复链收口，不属于本 requirement

### Requirement: generate 段教学重驱的统一授权窗口（REQ-WSC-28）

SC generate 段的教学重驱 SHALL 绑定原生成 run/reservation/phase，并使用 REQ-SG-05 的 SC 初轮载体。资格 SHALL 以 REQ-WSC-09 为唯一清单：parse 语法、事实完整的 typed lowering、事实完整的 typed IR 校验和确定性结构拒收均只可消费该同一窗口；MUST NOT 因失败阶段、诊断数量、provider 节点变化或类别变化建立独立机会。mixed Unknown/Operational、事实不完整、权限/上下文/基线不可证明的失败 MUST NOT 授予教学重驱。

同一候选生成链 SHALL 以原生成授权身份为锚，而非以重驱后新 source hash 为锚；每 candidate 至多一次额外教学驱动。只有已批准且耐久绑定的 RepairOnce 才可使用原调用之外的一个物理机会，原调用在内最多两次；无批准时最多一次。原窗口中的运行恢复已使用第二次物理机会时，教学重驱 MUST NOT 再产生第三次。

#### Scenario: 跨类别不乘法

- **WHEN** 同一生成链 attempt1 命中 parse/IR，attempt2 又命中 structure，或顺序相反
- **THEN** 不产生第三次启动，不新开 structure 槽，按 typed 工程故障停止并保持合法 refs

#### Scenario: 业务或运行错误不借教学

- **WHEN** 失败集合含 Operational/Unknown、基线读取失败、授权缺失或缺少必要 typed 事实
- **THEN** 零额外 author 调用；按已有等待/失败面回显真实原因，不把其渲染成模型可修语法错误

#### Scenario: 窗口只在初轮新授权建立

- **WHEN** 初轮新授权建立教学窗口后，A 或 B 的合法接力（机械返修、author 重写/反馈）再次进入生成链
- **THEN** 教学窗口 MUST NOT 被重建或重开；A/B 接力仍消费各自的 repair/turn 载体，不把 handler 再次进入当成新初轮

### Requirement: 通道 C 的独立有界预算归属（REQ-WSC-28A）

通道 C SHALL 是独立有界的生成教学机会，不消耗评审机械池 `max_repairs`，不消耗 author 重写/反馈池 `max_manual_repairs`；MUST NOT 增加 `repairs_used`、`manual_repairs_used` 或伪造用户 turn/hold/receipt。该归属不改变 A/B 池原有触发域、余额或计费语义，也不得由 A/B 池余额为 C 的第三次调用兜底。

#### Scenario: 一次教学不扣 A/B 池余额

- **WHEN** 通道 C 消费了唯一一次教学重驱机会并产生一次真实启动
- **THEN** 评审机械池 `max_repairs` 与 author 重写/反馈池 `max_manual_repairs` 的已用计数均不递增，不产生用户 turn/hold/receipt；该次教学自身的窗口消费计数递增且实际启动合计为 2

### Requirement: 教学机会的原子 claim 与耐久绑定（REQ-WSC-28B）

每次机会 SHALL 在任何外部启动前原子 claim，耐久绑定 authorization kind、原 generation/node/run 身份、业务请求及材料摘要、失败 source 引用与 hash、context digest、允许修改闭包、机器指令 digest、完整 prompt 引用与 digest、attempt key、物理上限与冻结执行策略摘要（通用字段，值由 explicit-sc-author-effort 填写）。初次调用尚无候选 source 时 SHALL 明确记录该事实并绑定原请求/材料，MUST NOT 伪造候选引用；重驱前失败原文与其 hash 必须可验证。授权、材料、上限或持久 claim 不可证明时 MUST 零额外调用并显式系统故障。RepairOnce 批准状态 SHALL 由初轮新授权在**建立窗口的同一事务内**核验并冻结，随窗口持久化，MUST NOT 因部署升级、重连、恢复、显式 fresh 或改档而变化，MUST NOT 事后补造。

#### Scenario: 旧记录不可补造授权

- **WHEN** 历史会话只有旧 `started` 占位或没有可证明的原授权/窗口
- **THEN** 读取历史不产生授权，零额外调用并说明证据缺失，不从计数为零推断存在机会

#### Scenario: claim 不可证明即零额外调用

- **WHEN** 授权、材料、上限或持久 claim 中任一项不可证明
- **THEN** 平台 MUST 零额外调用并显式系统故障，MUST NOT 以「尚未记账」推断仍有剩余机会

#### Scenario: 历史样本不得补造 grant 与 actual-start

- **WHEN** 读取历史旧样本且缺少 grant 或实际启动证据
- **THEN** MUST NOT 事后补造 grant 或 actual-start 记录；缺失按证据缺失处理，零额外调用

#### Scenario: 批准在窗口建立时一次核验并冻结

- **WHEN** 初轮新授权在同锁/CAS 下建立教学窗口，且原授权可证明、provider 路径合法、剩余物理机会 ≥1、失败事实完整
- **THEN** 窗口 SHALL 记录 RepairOnce 批准状态为「已批准」并冻结批准规则版本，物理上限为「原调用在内最多两次」；该结论 MUST NOT 因部署升级、重连、恢复、显式 fresh 或改档而改变

#### Scenario: 核验任一不成立即未批准且零额外调用

- **WHEN** 窗口建立时原授权不可证明、provider 路径不具备合法观察/准入、剩余物理机会为 0，或失败事实不完整
- **THEN** 批准状态 SHALL 为「未批准」、物理上限为「原调用在内最多一次」，MUST 零额外调用并显式系统故障，MUST NOT 以「尚未记账」推断仍有剩余机会

### Requirement: 教学重驱的 claim 与真实启动分账（REQ-WSC-28C）

claim 与 actual start SHALL 分账。该 attempt 的实际启动 MUST 写入既有 provider-start ledger（以 launch 身份为键）并携带 attempt 键关联，可关联真实 provider/run 与启动证据；MUST NOT 为同一次启动另插第二条记录。预留占位、调用前时间戳、WS 事件及旧 `started=true` MUST NOT 冒充真实启动。外部结果 Unknown 时 SHALL 沿原等待/对账，不自动重播。新 source revision、新 native 会话、重连、恢复及同请求重放 MUST NOT 重置窗口。所有允许启动仍须经过原 gateway admission/full fingerprint；显式 fresh 保留原授权/窗口/冻结输入，不获取新 RepairOnce。

#### Scenario: IR 拒收使用唯一额外机会

- **WHEN** 原调用产生事实完整的 typed IR 失败，原生成授权批准 RepairOnce，尚有一个剩余物理机会
- **THEN** 平台以完整失败 source、typed 定位和允许修改闭包 claim attempt2，原 author 重驱一次，形成两个真实 start 证据；C 消费一次，A/B 池均不扣

#### Scenario: 同键重放与不确定启动

- **WHEN** claim 后崩溃、启动回执未知、重连重投或显式 fresh
- **THEN** 读取同一 durable 窗口；已启动不重播、Unknown 等待；确定未启动才可在原 key/授权下恢复合法动作，不新增预算或物理机会

### Requirement: 教学尝试流水按 attempt 关联（REQ-WSC-28D）

候选完整输出及每次 compile/IR/structure 判定 SHALL 按 REQ-WSC-25 追加可追溯流水，关联同一 generation 与 provider attempt，包含 source ref/hash、阶段、稳定码、typed 种类、环指纹、规范化/修复事实和处置结论。原始输出无法形成 source 时 SHALL 明确记录缺失原因，不伪造引用。流水只用于审计/度量，不作为启动许可；流水写失败按 WSC-25 记日志且不阻断，授权/ledger 的持久执法失败则按 SG-05 停止或进入 Unknown 等待，二者不得混用。

#### Scenario: 流水与启动账本失败处理不同

- **WHEN** WSC-25 流水追加失败，或者 SG-05 claim/实际启动记录持久化失败
- **THEN** 前者记运行日志且不阻断；后者不得继续无账启动/重试，已发生外部动作按真实证据进入 Unknown/补偿，不伪装未启动

### Requirement: 第二次交付失败后的停止与越权处置（REQ-WSC-28E）

第二次交付仍失败或出现修复范围越权时 MUST 停止该生成自动链、保留 last-valid refs 与失败历史，MUST NOT 提升坏 candidate、发起 review/confirm 或旁路至另一自动重写入口。后续只有真实新业务授权可按其既有合同继续，MUST NOT 由平台自动制造新 turn、candidate 或 generation 来补机会。本条仅适用于 **generate 段**原生成链的第二次交付；**门段** `final_compile_*` 候选编译类失败不属于本条，其自动重写按 REQ-WSC-21 既有合同处理，REQ-WSC-21 的触发域不因本条收窄或扩张。「修复范围越权」的判据新码 `repair_scope_violation` 由裁决 §7 的 C 包交付，C 包落地前该判据不可判定。

#### Scenario: 第二次仍失败即停止

- **WHEN** 唯一教学机会已消费且第二次交付仍未通过编译或仍未满足结构判据
- **THEN** 平台停止该生成自动链并保留 last-valid refs 与失败历史；不产生第三次启动、不借 A/B 池余额、不自动制造新 turn 或 candidate 继续

#### Scenario: 门段候选编译失败仍按 REQ-WSC-21 驱动重写

- **WHEN** 门段 `final_compile_*` 候选编译类失败发生
- **THEN** 其自动重写按 REQ-WSC-21 既有合同处理（提示 owner、扣 author 重写次数预算），本条停止语义不适用于该门段失败

#### Scenario: 越权判据由 C 包交付（占位）

- **WHEN** 修复范围越权判据（新码 `repair_scope_violation`）尚未由裁决 §7 的 C 包交付
- **THEN** 该越权分支在 C 包落地前不可判定，本条不据此产生可验收断言；第二次交付失败的停止语义不受此占位影响

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
