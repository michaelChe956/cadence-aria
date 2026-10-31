# work-item-group-structure Specification

## Purpose
TBD - created by archiving change fix-work-item-group-plan-parity. Update Purpose after archive.

## Requirements

### Requirement: 候选结构评估与 Final Compile 的依赖图结论一致（REQ-WG-01）

同一 source revision 经候选结构评估（`assess_single_candidate_structure`）与 Final Compile（`work_item_record_attribution` + `apply_change_order_cross_repo_depends_on`）时，两处对**有效边集合、成环结论与环 witness** MUST 一致。不一致 MUST 判定为平台缺陷：以结构失败停等并保留两侧边集差异，MUST NOT 静默放行候选。平台 MUST 以固定 fixture 的奇偶性测试保护该一致性。

#### Scenario: 同一候选在两处得出同一环结论

- **WHEN** 一个多仓候选声明 `intent_target_kind: single_repository` 且某 Work Item 的 Inputs provider 位于 change_order 更靠后的成员仓
- **THEN** 候选结构评估 MUST 与 Final Compile 一致地判定成环，且 witness 的端点与逐跳来源一致

#### Scenario: 两处结论不一致时停等而非放行

- **WHEN** 同一 source 在候选结构评估无环、在 Final Compile 成环（或反之）
- **THEN** 平台 MUST 以结构失败停等并保留两侧有效边差异，MUST NOT 放行到 review 或确认

### Requirement: 物理成员仓到逻辑成员的反查必须单一来源（REQ-WG-02）

「物理成员仓 ID → 逻辑成员」的反查 MUST 只有一个实现，候选结构评估与 Final Compile MUST 共用。反查结果 MUST 是三态：唯一命中、无命中、多命中。非「唯一命中」时 MUST NOT 猜测，MUST 保持既有失败语义（Final Compile 报 `work_item_target_missing`；候选结构评估不计入仓序自动边）。

#### Scenario: 唯一命中则两处得到同一逻辑成员

- **WHEN** 某 Work Item 声明的物理仓在授权成员表中唯一对应一个逻辑成员
- **THEN** 候选结构评估与 Final Compile MUST 解析出同一逻辑成员，仓序自动边在两处同时生效

#### Scenario: 多命中不猜

- **WHEN** 同一物理仓 ID 在成员表中对应多个逻辑成员
- **THEN** 反查 MUST 返回多命中，两处 MUST NOT 任选其一，MUST 按既有失败语义停等

### Requirement: 依赖环必须给出可执行选项而非归因断言（REQ-WG-05）

依赖环呈现在 owner 面时 MUST 只由 typed witness 事实构成：环上有向链、各 Work Item 所在仓的 change_order 位次、逆仓序的那一跳及其来源。witness 完整时 MUST 给出 2~3 个可执行选项及各自后果。MUST NOT 断言根因在 design（仓序本身可能完全正常）；MUST NOT 声称存在唯一合法托管仓（写范围约束可能使多个解并存）。witness 不完整时 MUST 只陈述稳定码与已知事实，MUST NOT 虚构环形状或原因。

#### Scenario: witness 完整时给选项

- **WHEN** Final Compile 因 `dependency_cycle` 失败且 witness 完整
- **THEN** owner 面 MUST 含环上逐跳来源与两个仓序位次，并给出「去掉该跨仓消费」与「迁移托管仓及写范围」两类选项及各自后果

#### Scenario: 仓序正常时不得归因 design

- **WHEN** 逆仓序的那一跳源于某 Work Item 的托管仓选择，而 design 的 change_order 与成员表无矛盾
- **THEN** owner 面 MUST NOT 出现「根因在 design」「需要 owner 修改 design」这类断言

#### Scenario: witness 不完整时不猜

- **WHEN** 环 witness 缺失或不闭合
- **THEN** owner 面 MUST 只含稳定码与已确认事实，MUST NOT 生成具体仓序位次或选项归因

### Requirement: 结构修复指令必须携带算出的选项与保护项（REQ-WG-06）

witness 完整时，平台交给 author 的教学内容 MUST 携带上述选项与被算出的具体位次，并 MUST 含保护项：保留全部 canonical Inputs 的业务语义、不得修改 design 的 change_order、不得为消环删除真实数据流。同一结构指纹重复出现时 MUST 升级为 owner 决策并 MUST NOT 继续自动重写。

#### Scenario: 教学块含算出的位次

- **WHEN** witness 完整且触发自动返修
- **THEN** author 教学块 MUST 含被逆的两个仓序位次与可选修法，MUST NOT 只有四条通用原则

#### Scenario: 同指纹不烧预算

- **WHEN** 重写后产出结构指纹相同的环
- **THEN** 平台 MUST 停止自动重写并升级为 owner 决策，MUST NOT 再消耗重写预算

### Requirement: Plan Intent 取值来源必须按会话类型唯一且受计量闸门约束（REQ-WG-08）

prompt 对同一 `Plan Intent` key 的取值来源 MUST 按会话类型绑定（单仓会话取 `[issue]` 头 `repo=`；LC 多仓会话取成员仓清单里的物理成员仓 ID），MUST NOT 以无条件措辞出现；同一会话内 MUST 只有一个适用来源。平台 MUST 以计量测试锁定固定教学块的字符数与硬性措辞数上限（只降不升），新增条款 MUST 先证明平台侧未覆盖。

#### Scenario: 互斥取值来源必须按会话类型绑定

- **WHEN** prompt 对同一 `Plan Intent` key 给出两条取值来源（单仓会话取 `[issue]` 头 `repo=`；LC 多仓会话取成员仓清单里的物理成员仓 ID）
- **THEN** 每条 MUST 绑定其适用的会话类型，MUST NOT 以无条件措辞出现
- **AND** 同一会话内 MUST 只有一个适用来源；计量测试 MUST 断言「规定该 key 取值来源的句子」全部带会话类型限定，且权威来源句恰好一条
- **AND** 平台接管该字段的取值后，MUST 删除「模型自己填」那条规则（此时不再需要按会话类型分支）

#### Scenario: 体量回长被测试拦住

- **WHEN** 固定教学块字符数或硬性措辞数超过锁定上限
- **THEN** 计量测试 MUST 失败，MUST NOT 静默通过
