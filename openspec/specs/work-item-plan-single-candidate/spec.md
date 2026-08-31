# work-item-plan-single-candidate Specification

## Purpose

Work Item Plan 以「单候选计划事务」交付：LLM 与人只接触 markdown/EARS 源文档，机器只消费确定性编译的 typed IR；reviewer 提供证据但不驱动状态机；全流程复用阶段 1 的唯一人工门与终态矩阵，自动化运行可在无人值守下到达终态。

## Requirements

### Requirement: 单候选计划事务（REQ-WSC-01）

系统 SHALL 将 workitem 段对外流程压缩为 prepare → generate → evaluate → approval → completed 五类可见状态，另加吸收态 failed。outline/draft/batch 的逐段确认、逐段 review、生成模式选择 SHALL NOT 暴露为用户可见或可应答的 WS/UI 决策；生成模式（batch/serial）SHALL 由运行时按模型能力与候选规模自行选择。

#### Scenario: 自动化 campaign 无人工干预到达终态

- **WHEN** 以 `auto_if_valid` 策略运行且候选通过全部机械校验、无未决 human_required finding、无重复指纹
- **THEN** 系统直接完成原子 compile 并发布 Confirmed plan 与 Work Items，全程不需要任何 WS 决策消息

#### Scenario: 生成模式不再询问

- **WHEN** 候选计划生成需要选择 batch 或 serial 执行策略
- **THEN** 运行时依据模型能力与候选规模内部选择，不产生 `select_work_item_generation_mode` 类型的对外决策请求

### Requirement: markdown 编译器模型（REQ-WSC-02）

系统 SHALL 以 markdown/EARS 文档作为 work item plan 的唯一可编辑源，并提供确定性编译器将其单向编译为顶层 `PlanCandidateIr { source_revision_hash, compiler_version, items: Vec<PlanCandidateItemIr> }`；每个 item SHALL 为 `PlanCandidateItemIr { target_repository_id, contract, verification_plan: WorkItemDraftVerificationPlan, trusted_commands }`。typed IR 的 source revision hash 与 compiler version 仅位于顶层；**publish 前** hash 或版本不匹配时系统 SHALL 拒绝发布并提示重新编译，hash/version 随不可变 publication provenance 落盘。coding 段只消费已发布的 immutable runtime binding，SHALL NOT 在执行期间解析 markdown 或重新解释 compiler version；write_policy 与 trusted commands 等安全边界 SHALL 保持强类型。对 markdown 的人工或模型修改 SHALL 产生新 revision 并触发重新编译。

#### Scenario: markdown 与 IR 漂移被拒绝

- **WHEN** 发布前 typed IR 的 source_revision_hash 与当前 markdown 源不匹配，或 compiler_version 过期
- **THEN** 系统拒绝发布，提示重新编译；已发布的 binding 不受影响

#### Scenario: 编译错误可诊断

- **WHEN** markdown 源不满足 grammar（缺 section、ID 格式错误、EARS 句式非法）
- **THEN** 编译器返回行号、字段名与一个修复示例；该错误信息可直接作为返修反馈回喂模型

### Requirement: 中央策略层与 typed outcome（REQ-WSC-03）

系统 SHALL 提供中央策略层（在阶段 1 `workitem-typed-outcome-policy` 落地，本 change 复用），将每次评估结果归入四类 typed outcome：`valid`、`repairable`、`human_required`、`fatal`。reviewer 的 verdict 与 severity SHALL NOT 直接驱动状态跳转；策略层依据机械校验结果、finding 归类建议、指纹与预算做确定性裁决。

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

reviewer（单候选）SHALL 在复评前获得只读契约覆盖投影，内容 SHALL 至少包括：逐 WI→contract edge 的 required capabilities、所引契约输出 capabilities 与 compatibility_policy；节点依赖图事实（depends_on/边/环/重复边/未知 provider）；handoff 消费闭环（每个 `provided_contract_refs` 的消费者集合与消费状态，无消费者时显式空集）；跨 work item 写范围冲突事实（exclusive/forbidden 重叠）。投影数据 SHALL 复用 `src/product/work_item_contract/dependency.rs` 的确定性共享计算逻辑生成，SHALL NOT 以独立重述口径替代；reviewer SHALL 对能力覆盖缺口与未被消费的 handoff 产出 must_fix finding（归类建议 contract_gap），SHALL NOT 将 canonical 将判 `required_capability_missing` 或 `unconsumed_required_handoff` 的候选评为无 must_fix 通过。canonical 校验器保持 fail-closed 原样；reviewer 投影为其前置防线而非替代。契约覆盖投影 SHALL 仅注入单候选 reviewer 路径，legacy/story/design reviewer SHALL NOT 接收。

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

### Requirement: 新旧路径并存与可验证退役（REQ-WSC-07）

系统 SHALL 以持久化 `flow_kind` 维持新旧两条 workitem 路径并存，直到以下全部满足才允许删除旧协议（generation-mode 决策、逐段确认消息、review_decision 双选项语法）：codex 与 pi 各 1 案例到达 Confirmed（2/2）、单案例时长 ≤12 分钟、初评 ≤1 次且复评 ≤1 次（总 ≤2）、自动返修 ≤1 次（均从服务端持久计数读取）、阶段 1 的 14 条 classifier golden（rep2/3/4 的 9 条、rep1 round-1 的 2 条 Advisory、3 条人工标注 class_hint 变体）全部归入预期 finding 分类、仅明确属 grammar/lowering 的 reviewer finding 通过 compiler diagnostic golden（其余明确仅为 prompt few-shot 素材）、断线重连/恢复测试通过、legacy 路径回归全绿。

对于多仓 Issue，legacy fallback 只允许在确定性 preflight 失败且新路径尚未产生副作用时发生。一旦已经持久化新路径状态或启动 provider，失败 SHALL 收敛为新路径的 durable fatal/recoverable 终态，SHALL NOT 静默切换 `flow_kind` 或回落 legacy。

#### Scenario: 退役门未达成不删除

- **WHEN** campaign 验收指标未全部满足
- **THEN** 旧 WS 协议与中间状态结构保持可用，新路径缺陷可回退

#### Scenario: 多仓 preflight 后失败不静默回落

- **WHEN** 多仓新路径已写入任一 durable record 或已启动 provider 后发生失败
- **THEN** 原 flow_kind 保持不变，失败以 durable fatal/recoverable 状态记录；系统不得改走 legacy
