# work-item-plan-single-candidate Delta

## MODIFIED Requirements

### Requirement: markdown 编译器模型（REQ-WSC-02）

系统 SHALL 以 markdown/EARS 文档作为 work item plan 的唯一可编辑源，并提供确定性编译器将其单向编译为顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`；每个 item SHALL 为 `PlanCandidateItemIr { target_repository_id, contract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`。typed IR 的 source revision hash 与 compiler version 仅位于顶层；**publish 前** hash 或版本不匹配时系统 SHALL 拒绝发布并提示重新编译，hash/version 随不可变 publication provenance 落盘。coding 段只消费已发布的 immutable runtime binding，SHALL NOT 在执行期间解析 markdown 或重新解释 compiler version；write_policy 与 trusted commands 等安全边界 SHALL 保持强类型。对 markdown 的人工或模型修改 SHALL 产生新 revision 并触发重新编译。

SC 交付（author 与人工修订两条路径）进入编译前，系统 SHALL 允许两类确定性归一化：其一为结构标题行归一化（固定中文→英文映射表逐字映射，仅覆盖固定词表已知翻译变体）；其二为 EARS 关键字空白归一化（`WHEN` 后/`THE SYSTEM SHALL` 前缺半角空格或邻位 U+3000/NBSP 时确定性补齐/归一）。两类归一化只触碰上述空白与固定词表位置，正文其余内容零触碰；表外未知标题 SHALL NOT 被猜测改写；任一归一化发生时 SHALL 落一条救回诊断/事件。

**lowering 重复字段累积语义**：同一 Outputs contract 的多行 `capabilities`（或同一 Inputs contract 的多行 `required_capabilities`）SHALL 按出现顺序累积为全体行的并集（完全相同值仅保留首次），SHALL NOT last-write-wins；相邻 contract 字段严格隔离；累积与去重不改变值形态。

**候选校验携带存储 options 与基线树**：SC 候选校验上下文 SHALL 携带创建计划时落库的 `IssueWorkItemPlanOptions` 与 plan 基线（worktree fork base / target ref 树），SHALL NOT 从 IR items 反推 options。options×items 一致性（integration_work_item_required / e2e_work_item_required / frontend_backend_split_required 三族）SHALL 在 generate/evaluate 期校验，Error 级结果经既有机械 ReviewVerdict 回灌修订，SHALL NOT 延迟至 Approval/Final Compile 才首次出现。验收标准（AC）/验证计划引用的仓库内文件路径 SHALL 与基线树交叉核对：引用基线中不存在的路径 SHALL 产生 Error（附路径清单与修复建议），SHALL NOT 进入人工确认后才由 coder 发现。

#### Scenario: markdown 与 IR 漂移被拒绝

- **WHEN** 发布前 typed IR 的 source_revision_hash 与当前 markdown 源不匹配，或 compiler_version 过期
- **THEN** 系统拒绝发布，提示重新编译；已发布的 binding 不受影响

#### Scenario: 编译错误可诊断

- **WHEN** markdown 源不满足 grammar（缺 section、ID 格式错误、EARS 句式非法）
- **THEN** 编译器返回行号、字段名与一个修复示例；该错误信息可直接作为返修反馈回喂模型

#### Scenario: 固定词表中文结构标题确定性归一化后可编译

- **WHEN** provider 交付的 markdown 把固定词表结构标题翻成已知中文变体，且正文其余部分满足语法
- **THEN** 系统在编译前把结构标题行逐字归一化为规范英文并编译通过，正文中文逐字保留，并记录一条归一化诊断/事件；表外未知标题不被猜测改写，仍按既有语法契约 fail-closed 拒绝

#### Scenario: EARS 关键字缺空格确定性补齐后可编译

- **WHEN** statement 的条件以 CJK 标点收尾后无半角空格直接接 `THE SYSTEM SHALL`，或 `WHEN` 后缺半角空格，或关键字邻位为 U+3000/NBSP
- **THEN** 系统在编译前确定性补齐/归一为单个半角空格后编译通过；条件文本与标点本身逐字保留，并记录一条归一化诊断/事件

#### Scenario: 归一化不救非空白语法错误

- **WHEN** statement 的语法错误不是关键字空白形态（如缺 WHEN 前缀、THEN 语义缺失、顺序错误）
- **THEN** 归一化层不修改该行，编译器照旧以 fail-closed 拒绝并返回行号诊断

#### Scenario: 重复能力行累积不丢能力

- **WHEN** 同一 Outputs contract 的 markdown 含 N 行 `capabilities`
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
- **THEN** generate/evaluate 期产出 Error（附路径清单与三种修复建议：删除该 AC / 改用基线内路径 / 显式声明基线恢复依赖并纳入写范围）；SHALL NOT 让 coder 在执行期发现并被迫越界恢复基线

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
