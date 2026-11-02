# 规范增量：work item group 的平台派生与仓序持有（延后部分）

<!-- 来源：owner 2026-10-31「我最大的问题就是这个 work item group 这一块一直过不去」「我没太懂这块逻辑为什么会搞得如此复杂」。
     设计：cadence/designs/2026-10-31_技术方案_Plan生成不可控性治理_v2.0.md
     本 delta 不新增 provider attempt、不改状态机、不动 CandidateCompileStatus 线格式。

     **本 change 只保留 2026-07-31 owner 裁定「复杂内容先不做」的延后条款**：Plan Intent 四键平台派生（REQ-WG-03）、
     仓序边平台持有 / Inputs 方向冲突检出（REQ-WG-04）、平台接管后移除 prompt 结构契约（REQ-WG-07）。
     本轮已兑现的条款（判定一致 / 反查单一来源 / 可执行选项 / 修复指令携带选项与保护项 / 取值来源按会话类型唯一）
     已拆出到 `fix-work-item-group-plan-parity` 并归档。 -->

## ADDED Requirements

### Requirement: Plan Intent 目标四键由平台派生（REQ-WG-03）

`intent_target_kind`、`intent_target_repo`、`intent_target_codebase`、`intent_target_logical_repo` 四键 MUST 由平台按已确认成员表与 Work Item 写范围前缀派生。该派生 MUST 建立在 lowering 上下文持有成员 checkout 前缀表之上；派生能力上线前，平台 MUST NOT 允许省略 `### Plan Intent` 段（现状省略会静默落到 `fallback_physical_id`，把所有 Work Item 归到 change_order 首仓）。`intent` 与 `provider_work_item_id` MUST 仍由 author 声明，平台 MUST NOT 代填（REQ-C1-PLAN-01）。author 填写的目标键与派生值冲突时 MUST 以派生值为准并留下可审计记录；写范围无法唯一定位到单个成员仓时 MUST 以 typed 缺口拒绝，MUST NOT 任选。

#### Scenario: 省略 Plan Intent 段时不得静默归首仓

- **WHEN** author 输出的多仓 Work Item 省略 `### Plan Intent` 段，且写范围能唯一定位成员仓
- **THEN** 平台 MUST 按写范围前缀派生目标，MUST NOT 落到 `fallback_physical_id` 静默归属首仓；若派生能力尚未上线，MUST 以 typed 缺口停等而不是静默归属

#### Scenario: intent 与 provider 不由平台代填

- **WHEN** author 未声明 `intent`（create/existing）或 `provider_work_item_id`
- **THEN** 平台 MUST NOT 自动补声明，MUST 按 REQ-C1-PLAN-01 的 `intent_undeclared` 语义停等修订

#### Scenario: 冲突以派生为准并留证

- **WHEN** author 声明的 `intent_target_repo` 与写范围前缀指向的成员仓不一致
- **THEN** 平台 MUST 采用派生值并记录冲突审计，MUST NOT 因冲突使候选失败

#### Scenario: 写范围无法唯一定位时拒绝

- **WHEN** 某 Work Item 的 `exclusive_scopes` 同时落在两个成员仓目录下
- **THEN** 平台 MUST 以 typed 缺口拒绝并点名涉及的成员仓，MUST NOT 任选其一

### Requirement: 仓序边由平台持有，Inputs 方向由平台检出冲突（REQ-WG-04）

跨成员仓的仓序边 MUST 由平台按 confirmed design 的 `change_order` 派生并持有（既有的「后仓每个 Work Item 依赖前仓每个 Work Item」规则）。author 声明的 Inputs（消费上游哪条能力）是业务语义，平台 MUST NOT 代写、MUST NOT 改写其 provider/能力串。当 author 的 Inputs 方向与仓序边冲突（即形成环）时，平台 MUST 检出并按 REQ-WG-05 给出选项。任何面（prompt、错误提示、教学块）MUST NOT 要求 author 理解或复述 change_order 规则。

#### Scenario: 跨仓消费逆向时被检出

- **WHEN** 某 Work Item 的 Inputs provider 位于 change_order 更靠后的成员仓
- **THEN** 平台 MUST 判定为依赖环（canonical_input 与 change_order 自动边互逆）并按 REQ-WG-05 呈现，MUST NOT 静默改写该 Inputs

#### Scenario: 平台不得代写 Inputs

- **WHEN** 候选缺少某条跨仓 Inputs 声明
- **THEN** 平台 MUST NOT 自动补写该 Inputs（能力串是业务语义），MUST 按既有 canonical 契约缺口语义停等

### Requirement: prompt 中的结构契约必须随平台接管而移除（REQ-WG-07）

已被平台接管的结构契约条款 MUST 从 author prompt 删除，MUST NOT 以「再写一遍规则」方式保留。平台 MUST 以计量测试锁定固定教学块的字符数与硬性措辞数上限；新增条款 MUST 先证明平台侧未覆盖。

#### Scenario: 互斥取值来源必须按会话类型绑定

- **WHEN** prompt 对同一 `Plan Intent` key 给出两条取值来源（单仓会话取 `[issue]` 头 `repo=`；LC 多仓会话取成员仓清单里的物理成员仓 ID）
- **THEN** 每条 MUST 绑定其适用的会话类型，MUST NOT 以无条件措辞出现
- **AND** 同一会话内 MUST 只有一个适用来源；计量测试 MUST 断言「规定该 key 取值来源的句子」全部带会话类型限定，且权威来源句恰好一条
- **AND** 平台接管该字段的取值后，MUST 删除「模型自己填」那条规则（此时不再需要按会话类型分支）

#### Scenario: 体量回长被测试拦住

- **WHEN** 固定教学块字符数或硬性措辞数超过锁定上限
- **THEN** 计量测试 MUST 失败，MUST NOT 静默通过
