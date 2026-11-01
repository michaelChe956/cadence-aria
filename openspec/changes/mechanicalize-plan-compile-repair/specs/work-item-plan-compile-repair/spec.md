# 规范增量：Plan 生成的机械失败由平台处置，模型只负责业务内容

<!-- 来源：owner 2026-10-31「我觉得这样不可控性太高了」。
     设计依据：cadence/designs/2026-10-31_技术方案_Plan生成不可控性治理_v1.0.md。
     与未实施 change fix-error-experience-structural-governance 的 REQ-SG-02A 的关系见本 change design.md「遗留裁决 O1」：
     本 change 不新增对已持久或已评审 source 的直接改写。 -->

## ADDED Requirements

### Requirement: 机械失败的四层处置与三条红线（REQ-CRP-01）

平台 MUST 把计划生成中可机械判定的失败按 L0 确定性自修、L1 结构投影补全、L2 精确缺口、L3 业务取舍四层处置。L0/L1 MUST NOT 引入任何新业务事实，MUST 只作用于尚未持久化且未被评审的 source，MUST 产出可审计记录。canonical Inputs 边 MUST NOT 被自动删除。

#### Scenario: L0 只减不增

- **WHEN** 平台对一份候选执行 L0 处置
- **THEN** 结果 MUST NOT 出现原候选未声明过的 capability、契约、验收标准、验证命令或写范围

#### Scenario: canonical 数据流边不可自动删除

- **WHEN** 依赖环上任一跳的来源包含 `canonical_input`
- **THEN** 平台 MUST NOT 以删边方式让编译通过，MUST 交 author 重组或升级为 L3 业务取舍

#### Scenario: 已持久或已评审的 source 不被直接改写

- **WHEN** 候选已经成为 current 产物或已被 reviewer 消费
- **THEN** 平台 MUST NOT 直接改写其 source 或 IR，修复 MUST 经原授权 author 产出新 revision

### Requirement: lowering 诊断必须携带 typed fault（REQ-CRP-02）

计划编译的每条 lowering 诊断 MUST 在构造点携带独立的 typed fault（含字段名与期望形态），MUST NOT 从 code 或 message 文案反推。fault 不得进入持久化 wire 枚举。无法定位到原行时 MUST 显式标注不可定位，MUST NOT 以行号 0 冒充真实行号。

#### Scenario: 分支 key 错配可路由

- **WHEN** `intent_target_kind=single_repository` 却未提供 `intent_target_repo`
- **THEN** 诊断 MUST 携带分支错配 fault 与缺失字段名，恢复策略 MUST 可据此确定性路由，MUST NOT 落入未知停点

#### Scenario: 不认措辞

- **WHEN** 某诊断 message 含「缺少」「必须」等措辞但不携带 typed fault
- **THEN** 平台 MUST NOT 授予自修或教学重驱资格

### Requirement: Plan Intent 目标分支等价规范化（REQ-CRP-03）

Plan Intent 的目标 key 值已在场但放错分支 key 时，编译器 MUST 仅在该值唯一映射到授权成员物理仓时按等价规范化解析目标，MUST NOT 改写 source，MUST 产出审计事件。映射不唯一或值不在成员表时 MUST NOT 猜测，MUST 保留 typed 报错。

#### Scenario: 唯一映射则解析

- **WHEN** `intent_target_kind=single_repository` 且值填在 `intent_target_logical_repo`，该值唯一映射到成员物理仓
- **THEN** 编译 MUST 以该物理仓为目标继续，并 MUST 产出含原 key、原值、映射依据的审计事件

#### Scenario: 双命中则不猜

- **WHEN** 同一值既是某逻辑成员 key 又等于另一成员的物理仓值
- **THEN** 编译 MUST NOT 选择其一，MUST 以 typed 报错 fail-closed

#### Scenario: 无法推导的分支保持报错

- **WHEN** `intent_target_kind=logical_codebase` 缺少 `intent_target_codebase`
- **THEN** 编译 MUST 保留 typed 报错，MUST NOT 从会话事实臆造 codebase id

### Requirement: 修不了时给精确缺口且不造假证据（REQ-CRP-04）

平台无法自修时 MUST 产出 L2 精确缺口，MUST NOT 只输出「请重新生成」。每个缺口 MUST 含 Work Item 与字段定位、缺口事实、可参照的既有合法样例与下一步动作。平台 MUST NOT 自动补出 reviewer check 或验证命令；同一事实 MUST NOT 分多轮重复呈现。

#### Scenario: 验收标准缺 check 引用时列清单

- **WHEN** 某 Work Item 的验收标准缺少 reviewer check 引用
- **THEN** 缺口 MUST 写明该 Work Item、缺失的 AC id 与已有 check 清单，MUST NOT 由平台补出 check

#### Scenario: 同一缺口不重复问

- **WHEN** 某缺口已被 owner 处置并落入会话已处置事实清单且事实未变
- **THEN** 后续门 turn MUST NOT 再次把它作为待处理项呈现

### Requirement: prompt 体量与硬措辞受闸门约束（REQ-CRP-05）

SC author 固定教学块 MUST 有可测的字符数与硬措辞（必须、不得、禁止、拒绝）上限，由计量测试锁定。任何条款从 prompt 移除前，对应约束 MUST 已由平台侧校验或自修覆盖并有测试引用。上限提升 MUST 随契约变更声明，MUST NOT 静默放宽。

#### Scenario: 计量测试锁定上限

- **WHEN** 固定教学块的字符数或硬措辞数超过契约上限
- **THEN** 计量测试 MUST 失败，改动者 MUST 先声明契约变更

#### Scenario: 移除条款需有平台侧覆盖

- **WHEN** 某条机械契约从 prompt 删除
- **THEN** 变更说明 MUST 指向覆盖它的校验或自修测试，MUST NOT 出现既不在 prompt 也不在平台校验中的规则
