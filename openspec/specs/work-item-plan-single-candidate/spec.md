# work-item-plan-single-candidate Specification

## Purpose

Work Item Plan 以「单候选计划事务」交付：LLM 与人只接触 markdown/EARS 源文档，机器只消费确定性编译的 typed IR；reviewer 提供证据但不驱动状态机；全流程复用阶段 1 的唯一人工门与终态矩阵，自动化运行可在无人值守下到达终态。

## Requirements

### Requirement: 单候选计划事务（REQ-WSC-01）

系统 SHALL 将 workitem 段对外流程压缩为 prepare → generate → evaluate → approval → completed 五类可见状态，另加吸收态 failed。outline/draft/batch 的逐段确认、逐段 review、生成模式选择 SHALL NOT 暴露为用户可见或可应答的 WS/UI 决策；生成模式（batch/serial）SHALL 由运行时按模型能力与候选规模自行选择。interactive 策略下的人工门 SHALL 升级为对话式多轮协议（typed 反馈回合、SC 专属修订路径、approve→compile→Confirmed 关门链），其回合、预算、幂等与恢复语义以 `work-item-plan-conversational-gate` capability 为唯一来源；`auto_if_valid` 策略语义不变。

在既有单候选计划事务与对话式人工门语义不变的前提下，Work Item Plan 的批次确认与 Final Compile recovery 状态 SHALL 在 Cockpit 具有可观察、可定位的待处理投影。该投影 MAY 使用批次/recovery 专属展示和既有 typed 动作，但 MUST NOT 恢复 outline/draft/batch 的 legacy 逐段确认、逐段 review 或 generation-mode 对外决策协议。Plan 已 durable Confirmed 后仍须由客户端显式调用 `advance`，Cockpit 投影不得将可见门误作 coding 已启动。

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

### Requirement: markdown 编译器模型（REQ-WSC-02）

系统 SHALL 以 markdown/EARS 文档作为 work item plan 的唯一可编辑源，并提供确定性编译器将其单向编译为顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`；每个 item SHALL 为 `PlanCandidateItemIr { target_repository_id, contract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`。typed IR 的 source revision hash 与 compiler version 仅位于顶层；**publish 前** hash 或版本不匹配时系统 SHALL 拒绝发布并提示重新编译，hash/version 随不可变 publication provenance 落盘。coding 段只消费已发布的 immutable runtime binding，SHALL NOT 在执行期间解析 markdown 或重新解释 compiler version；write_policy 与 trusted commands 等安全边界 SHALL 保持强类型。对 markdown 的人工或模型修改 SHALL 产生新 revision 并触发重新编译。

SC 交付（author 与人工修订两条路径）进入编译前，系统 SHALL 允许两类确定性归一化：其一为结构标题行（一级文档标题、Work Item 二级标题、三级结构 section 标题）归一化——以固定中文→英文映射表逐字映射回规范英文，仅覆盖固定词表的已知翻译变体；其二为 **EARS 关键字空白归一化**——当 statement 行内 `WHEN` 后或 `THE SYSTEM SHALL` 前缺失半角空格、或关键字邻位为 U+3000/NBSP 时，确定性补齐/归一为单个半角空格。两类归一化均只触碰上述空白与固定词表位置，正文其余内容（含 CJK 标点如全角引号）零触碰；表外未知标题 SHALL NOT 被猜测改写，SHALL 照旧交给编译器以 fail-closed 拒绝；任一归一化发生时系统 SHALL 落一条可判定该次交付被救回的诊断/事件。

**lowering 重复字段累积语义**：同一 Outputs contract 的多行 `capabilities`（或同一 Inputs contract 的多行 `required_capabilities`）SHALL 按出现顺序累积为全体行的并集（每行 split 后的值依次 append），完全相同的 capability 值仅保留首次出现；SHALL NOT 以最后出现的行覆盖先行行（last-write-wins 禁止）。相邻 contract 的字段 SHALL 严格隔离（contract 边界 flush），不得跨 contract 归并或串项。累积与去重 SHALL 不改变 capability 值的原文形态（不排序、不改大小写/标点/内部空白）。

**候选校验携带存储 options 与基线树**：SC 候选校验上下文 SHALL 携带创建计划时落库的 `IssueWorkItemPlanOptions` 与 plan 基线（worktree fork base / target ref 树），SHALL NOT 从 IR items 反推 options。options×items 一致性（integration_work_item_required / e2e_work_item_required / frontend_backend_split_required 三族）SHALL 在 generate/evaluate 期校验，Error 级结果经既有机械 ReviewVerdict 回灌修订，SHALL NOT 延迟至 Approval/Final Compile 才首次出现。验收标准（AC）/验证计划引用的仓库内文件路径 SHALL 与基线树交叉核对：引用基线中不存在的路径 SHALL 产生 Error（附路径清单与修复建议），SHALL NOT 进入人工确认后才由 coder 发现。

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

系统 SHALL 使 workitem author/reviewer prompt 仅承载产物规格（任务上下文、输出语法、边界约束、判例 few-shot），SHALL NOT 重复目标仓库已注入的通用行为教学（Superpowers/OpenSpec/项目规则）。reviewer prompt SHALL 将完备度类意见降级为 advisory，并要求每条 finding 附归类建议。

author prompt（单候选）SHALL 教学契约能力覆盖纪律：WI `input_contracts` 所引契约的输出 capabilities SHALL 覆盖该引用的全部 `required_capabilities`（require_all 全量覆盖、require_any 至少一项、同 contract_id 集合语义），端点/动作类能力 SHALL 显式声明、字段/记录形态承诺不隐含端点能力；该教学判定口径 SHALL 与 canonical 校验器 `report_contract_requirements` 一致，并明示 `required_capability_missing` 的 fail-closed 后果。

author prompt（单候选）SHALL 教学 handoff 消费闭环纪律：每个 Work Item 的 `Handoff Schema` 三个字段 SHALL 显式存在；`provided_contract_refs` SHALL 仅列出会被下游 Work Item 的 `input_contracts` 以（`provider_logical_work_item_id`, `contract_id`）二元组逐字消费的契约引用，数组元素 SHALL 唯一且非空白。若该 Work Item 在合法计划依赖图中不存在任何下游 consumer edge，则 `provided_contract_refs` SHALL 保留字段并显式写为 `[]`。不得通过省略 Handoff Schema、删除必需字段、写 blocker、修改 contract ID、依赖 `depends_on` 或自然语言描述来回避校验。该纪律判定口径 SHALL 与 `unconsumed_required_handoff` 校验一致。

author prompt（单候选）SHALL 教学 options 镜像纪律：创建计划选项（include_integration_tests / include_e2e_tests / 前后端拆分）的启用条件与对应 Work Item kind 要求 SHALL 与校验器口径逐字一致；教学须明示 flag 启用而缺对应 kind 时的 Error 后果与两种修复路径。author prompt 亦 SHALL 教学验收标准基线纪律：AC/验证计划引用的仓库内文件路径必须存在于 plan 基线树，不得引用其他分支才存在的文件。

reviewer（单候选）SHALL 在复评前获得只读契约覆盖投影，内容 SHALL 至少包括：逐 WI→contract edge 的 required capabilities、所引契约输出 capabilities 与 compatibility_policy；节点依赖图事实（depends_on/边/环/重复边/未知 provider）；handoff 消费闭环（每个 `provided_contract_refs` 的消费者集合与消费状态，无消费者时显式空集）；跨 work item 写范围冲突事实（exclusive/forbidden 重叠）。投影数据 SHALL 复用 `src/product/work_item_contract/dependency.rs` 的确定性共享计算逻辑生成，SHALL NOT 以独立重述口径替代；reviewer SHALL 对能力覆盖缺口与未被消费的 handoff 产出 must_fix finding（归类建议 contract_gap），SHALL NOT 将 canonical 将判 `required_capability_missing` 或 `unconsumed_required_handoff` 的候选评为无 must_fix 通过。canonical 校验器保持 fail-closed 原样；reviewer 投影为其前置防线而非替代。契约覆盖投影 SHALL 仅注入单候选 reviewer 路径，legacy/story/design reviewer SHALL NOT 接收。

reviewer prompt（单候选复评）SHALL 注入前轮结构化 findings：每轮注入前一轮非 advisory findings 的 canonical identity（结构化 key/fingerprint）、category、required_action 与当前候选 revision；reviewer SHALL 依据该清单执行重复判定纪律（同 identity 重现须显式标注），SHALL NOT 依赖自由文本记忆。

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

### Requirement: 旧协议退役与单路径收敛（REQ-WSC-08）

REQ-WSC-07 退役门已按 `legacy-protocol-retirement` REQ-RET-01 全口径重测解锁后（2026-09-19 用户终裁 B：pi 全子项达标=协议质量实证；codex Confirmed 子项登记已知例外——系统性 provider 内容缺陷与协议无关，门文本据此显式修订，属 1c 裁决预留的「后续专项裁决」路径显式行使；**后续义务：codex/claude_code/kimi_code 全部 provider 最终 SHALL 全测通过（defer-ledger DEF-PVR-ALL），在义前不得视为 provider 面收官**），旧协议（generation-mode 决策、逐段确认消息、review_decision 双选项语法、`HumanConfirmDecision` 旧枚举及其消息族、SelectRevisionPath 族及其专属 DTO）SHALL 删除。删除后：新会话 SHALL 一律走单候选流，不存在 legacy 入口；收到已删除消息类型系统 SHALL 返回 stage-specific protocol error 且零副作用；多仓 Issue 的确定性 preflight 失败 SHALL 收敛为新路径 durable fatal/recoverable 终态（含失败原因），系统 MUST NOT 存在 legacy fallback 路径，MUST NOT 静默切换 `flow_kind`（本句修订并取代 REQ-WSC-07 原「legacy fallback 只允许在确定性 preflight 失败且新路径尚未产生副作用时发生」条款）；历史 legacy session 的 durable 记录与事件前缀 SHALL 只读保留（处置细则以 `legacy-protocol-retirement` REQ-RET-03 为唯一来源）；SC 门消息面（`human_gate_feedback`/approve/abandon）以 `work-item-plan-conversational-gate` 为唯一来源。

#### Scenario: 已删除消息协议错误拒绝

- **WHEN** 退役完成后客户端发送任一已删除的 legacy 决策消息（generation-mode 决策、逐段确认、review_decision 双选项、human_confirm）
- **THEN** 系统返回 stage-specific protocol error 且零副作用，会话状态与事件流不变

#### Scenario: 多仓 preflight 失败收敛新路径终态

- **WHEN** 多仓 Issue 的确定性 preflight 失败（无论新路径是否已产生副作用）
- **THEN** 失败以新路径 durable fatal/recoverable 终态记录并含原因，系统不存在任何 legacy fallback 或 `flow_kind` 切换路径

#### Scenario: 新会话一律单候选流

- **WHEN** 退役完成后创建任意 workitem workspace 会话
- **THEN** 会话走单候选流（prepare → generate → evaluate → approval → completed），不存在 legacy 逐段路径选项

#### Scenario: 历史 legacy 记录只读保留

- **WHEN** 退役完成后读取历史 legacy session 的 durable 记录
- **THEN** 记录与事件前缀原样保留可读，未被迁移或清洗

### Requirement: generate 段编译语法失败的教学重驱（REQ-WSC-09）

SC 候选编译（generate 段）失败时，系统 SHALL 在会话进入终态前提供恰好一次教学重驱：失败类别为可教学修复的 parse 语法类（至少含 missing_section、invalid_ears、invalid_work_item_id）时，以既有 code:line:message 错误回灌 author prompt 重驱一次；重驱后仍失败则按既有失败路径终态。每 candidate 至多一次额外驱动（既有上限不变）；非 parse 语法类失败不触发重驱、按既有路径处理。

#### Scenario: invalid_ears 触发一次教学重驱

- **WHEN** 候选编译产出 invalid_ears 且为该 candidate 首次编译失败
- **THEN** 系统以行号+错误信息回灌 author 重驱一次；重驱交付通过编译则继续既有链路，仍失败则终态（不再二次重驱）

#### Scenario: 重驱上限不被突破

- **WHEN** 同一 candidate 的编译失败类别先后命中两个可教学类别
- **THEN** 教学重驱总共仍至多一次，第二次失败直接按既有失败路径处理

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
