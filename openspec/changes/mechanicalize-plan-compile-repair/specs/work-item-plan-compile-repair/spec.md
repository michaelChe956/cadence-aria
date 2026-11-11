# 规范增量：Plan 生成的机械失败由平台处置，模型只负责业务内容

<!-- 来源：owner 2026-10-31「我觉得这样不可控性太高了」。
     设计依据：cadence/designs/2026-10-31_技术方案_Plan生成不可控性治理_v1.0.md。
     与 REQ-SG-02A（主规格 workspace-structural-governance）的关系见本 change design.md
     「与 REQ-SG-02A 的关系：O1 已由 owner 裁决（裁决 §2.2/§2.3）」：
     不改 REQ-SG-02A，内容 autorepair 迁原授权 author；本 change 不新增对任何 source 的内容 patch。 -->

<!-- 2026-11-09 补：归档时 `openspec archive` 会为新建 capability 写入
     「TBD - created by archiving change ...」的 Purpose 占位，导致归档后
     `openspec validate --specs --strict` 因 Purpose 占位告警变红。此处按本 change
     自身的 Why/What Changes 补写 Purpose；语义不新增、不放宽任何 requirement。
     注：本注释必须留在 `## Purpose` 之前 —— 归档器要求 Purpose 段体内不含
     HTML 注释（含注释即判为「不可读」而放弃承接，退回 TBD 占位）。 -->

## Purpose

本 capability 规定 Work Item Plan 生成过程中**可机械判定的失败**由平台承担处置、模型只负责业务内容的合同边界：把契约失败分为 L0 确定性自修 / L1 结构投影补全 / L2 精确缺口 / L3 业务取舍四层，钉住「L0/L1 不引入新业务事实、不改已持久或已评审 source、不删 canonical Inputs 边」三条红线，并要求计划意图分支的等价解析、环诊断带来源、Stage1 与 Final Compile 的图判定一致、以及编译失败必须一次问清（`owner_question` + ≥2 `owner_options`）。目的是消除「把机械契约交给模型一次写对」的责任错配，使机械失败不再只有「重写整份文档」一条出口。

## ADDED Requirements

### Requirement: 机械失败的四层处置与三条红线（REQ-CRP-01）

平台 MUST 把计划生成中可机械判定的失败按 L0 确定性自修、L1 结构投影补全、L2 精确缺口、L3 业务取舍四层处置。L0 仅限主规格明文允许的既有交付前规范化（前言修剪、固定中文 heading→英文、EARS 空白规范化）与不改 source 的等价解析，MUST NOT 新增 source 内容 patch 权限；L1 为机器投影/指令，内容变化由原授权 author 执行。source「尚未持久化且未被评审」MUST NOT 作为豁免 REQ-SG-02A 的条件。L0/L1 MUST NOT 引入任何新业务事实，MUST 产出可审计记录。canonical Inputs 边 MUST NOT 被自动删除。

#### Scenario: L0 只减不增

- **WHEN** 平台对一份候选执行 L0 处置
- **THEN** 结果 MUST NOT 出现原候选未声明过的 capability、契约、验收标准、验证命令或写范围

#### Scenario: canonical 数据流边不可自动删除

- **WHEN** 依赖环上任一跳的来源包含 `canonical_input`
- **THEN** 平台 MUST NOT 以删边方式让编译通过，MUST 交 author 重组或升级为 L3 业务取舍

#### Scenario: 已持久或已评审的 source 不被直接改写

- **WHEN** 候选已经成为 current 产物或已被 reviewer 消费
- **THEN** 平台 MUST NOT 直接改写其 source 或 IR，修复 MUST 经原授权 author 产出新 revision

#### Scenario: capability 缺口不由平台补

- **WHEN** 候选命中 `required_capability_missing` typed 缺口
- **THEN** 平台 MUST NOT 自动追加 capability，MUST 产出机器指令由原授权 author 回查原业务与实现承诺后在保护闭包内产新 revision

#### Scenario: 未消费的必需 handoff 不被本地删

- **WHEN** 候选命中 `unconsumed_required_handoff`
- **THEN** 平台 MUST NOT 删 ref 或替换为 `[]`，MUST 提供完整来源事实供 author 合法重组，MUST NOT 删真实边过门

#### Scenario: 未知结构化 key 的业务内容保留

- **WHEN** 候选命中 `unknown_structured_key`
- **THEN** 平台 MUST NOT 按诊断行删除业务承载行，只生成「保留内容、迁往合法字段」的指令；无法 typed 定位时 MUST NOT 自动处理

#### Scenario: 重复 trusted command 不被本地删

- **WHEN** 同一 trusted command 在候选中重复
- **THEN** 平台 MUST NOT 本地删 command 或整 check，MUST 指出重复及受影响 check/ref 交 author 处置

### Requirement: lowering 诊断必须携带 typed fault（REQ-CRP-02）

计划编译的每条 lowering 诊断 MUST 在构造点携带独立的 typed fault（含字段名与期望形态），MUST NOT 从 code 或 message 文案反推。fault 不得进入持久化 wire 枚举。无法定位到原行时 MUST 显式标注不可定位，MUST NOT 以行号 0 冒充真实行号。

#### Scenario: 分支 key 错配可路由

- **WHEN** `intent_target_kind=single_repository` 却未提供 `intent_target_repo`
- **THEN** 诊断 MUST 携带分支错配 fault 与缺失字段名，恢复策略 MUST 可据此确定性路由，MUST NOT 落入未知停点

#### Scenario: 不认措辞

- **WHEN** 某诊断 message 含「缺少」「必须」等措辞但不携带 typed fault
- **THEN** 平台 MUST NOT 授予自修或教学重驱资格

### Requirement: Plan Intent 目标分支等价规范化（REQ-CRP-03）

Plan Intent 的目标 key 值已在场但放错分支 key 时，编译器 MUST 仅在该值唯一映射到授权成员物理仓时按等价规范化解析目标，MUST NOT 改写 source（source 字节不变；本条等价解析不因内容修复授权收窄而被移除），MUST 产出审计事件。映射不唯一或值不在成员表时 MUST NOT 猜测，MUST 保留 typed 报错。

#### Scenario: 唯一映射则解析

- **WHEN** `intent_target_kind=single_repository` 且值填在 `intent_target_logical_repo`，该值唯一映射到成员物理仓
- **THEN** 编译 MUST 以该物理仓为目标继续，并 MUST 产出含原 key、原值、映射依据的审计事件

#### Scenario: 双命中则不猜

- **WHEN** 同一值既是某逻辑成员 key 又等于另一成员的物理仓值
- **THEN** 编译 MUST NOT 选择其一，MUST 以 typed 报错 fail-closed

#### Scenario: 无法推导的分支保持报错

- **WHEN** `intent_target_kind=logical_codebase` 缺少 `intent_target_codebase`
- **THEN** 编译 MUST 保留 typed 报错，MUST NOT 从会话事实臆造 codebase id

#### Scenario: 等价解析保持 source 字节不变

- **WHEN** 唯一映射成立并采用等价规范化解析
- **THEN** 交付的 source MUST 与 author 原始输出逐字节一致，解析结果与映射依据只出现在编译侧投影与审计事件

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
