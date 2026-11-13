# Spec Delta

## ADDED Requirements

### Requirement: 契约类 finding 的有效必关闭在中央策略中的投影（REQ-TOP-07）

中央策略 SHALL 以「有效必关闭」判定消费 finding：`class_hint=advisory` 与 raw `pass` fallback MUST NOT 把有效必关闭的契约类 finding 判为 `Advisory`（raw verdict 指**未经有效必关闭重开**的原始判定）。闭合资格为真但缺可验证定位或服务端授权修复许可的 finding SHALL 落 `HumanRequired` 而非 `Repairable`；自动修复许可证据 MUST NOT 凭 `category`、`severity` 或 `class_hint` 构成。本投影 MUST NOT 提高 A 池 `max_repairs` 或 B 池 `max_manual_repairs` 余额，MUST NOT 改变 Verification scope 新 finding 的既有停止规则。

#### Scenario: advisory 提示不得盖过有效必关闭

- **WHEN** 一条 finding 在解析平台硬化后属有效必关闭的契约类 finding，且模型给出 `class_hint=advisory`
- **THEN** 中央策略 MUST NOT 将其判为 `Advisory`；其分类 MUST 与平台侧有效必关闭结论一致

#### Scenario: raw pass 不得把契约类 finding 判为建议项

- **WHEN** 一条闭合资格为真的契约类 finding 所在轮次的原始结论为 `pass` 且未经有效必关闭重开，该 finding 无任何 `class_hint`，并经未发生重开的路径被消费（如读持久化旧 verdict，或 `routing_scope.rs:88`、`routing_platform_rescue.rs:71`、`prompts/review_context.rs:557` 上的分类与指纹消费点）
- **THEN** 中央策略 MUST NOT 以 raw `pass` 的建议项兜底把它判为 `Advisory`；MUST NOT 因此把它排除在返修需求判定之外

#### Scenario: 分类优先级与原始 verdict 来源不冲突

- **WHEN** 分类器为 finding 判定分类
- **THEN** 分类的 verdict 维度 SHALL 继续使用 raw_verdict（不因本投影改用归一后的 gate）；但对有效必关闭的契约类 finding，其分类 MUST NOT 由 `class_hint` 或 raw verdict 的兜底决定，平台侧的有效必关闭事实 SHALL 优先

#### Scenario: 两级判定的分类落点

- **WHEN** 一条契约类 finding 的闭合资格（`effective_disposition == actionable` 且未被 owner 显式排除）为真，分别具备与不具备「服务端授权的既有合法机械修复许可」证据和「可验证定位」（`contract_field` 非空或可定位的 typed 事实引用）证据
- **THEN** 闭合资格真且自动修复许可真时 MUST 判 `Repairable`（沿既有许可，不新造 L1）；闭合资格真但自动修复许可假（许可或定位任一缺失）时 MUST 判 `HumanRequired`（不得 `Advisory`，不得 `Repairable`）；闭合资格假时维持既有分类行为

#### Scenario: 无定位转人工而非可自动返修

- **WHEN** 一条闭合资格为真的契约类 finding 的自动修复许可为假——无可验证定位（`contract_field` 为空且无可定位的 typed 事实引用），或无服务端授权的既有合法机械修复许可，二者任一缺失
- **THEN** 中央策略 SHALL 将其裁决为 `HumanRequired`；MUST NOT 裁决为 `Repairable`，MUST NOT 因此触发自动返修或新增自动重写次数

#### Scenario: 不得凭类别、严重度或提示自造修复许可

- **WHEN** 中央策略仅依据 finding 的 `category`、`severity` 或 `class_hint` 判断其可自动修复性（包括 `class_hint=repairable`，以及仅凭「`Revise` + 契约类 + `contract_field` 非空」的分类兜底）
- **THEN** 策略 MUST NOT 据此新造可自动修复的许可或自动调用 author；是否可自动返修 MUST 依据既有可验证定位与服务端授权的机械修复许可证据（`RepairReservation` / 机械修复指令链的在场证据）

#### Scenario: 不得重新解析 raw 绕过 owner 排除

- **WHEN** 一条契约类 finding 已被 owner 显式排除，而中央策略仍可读到其原始模型文本（包括经历史重建路径读到的原始文本）
- **THEN** 策略 MUST NOT 通过重新解析原始文本恢复其闭合资格或绕过 owner 排除；原始模型文本仅保留用于审计，owner 排除事实源 MUST 保持优先

#### Scenario: 不提高余额且不改既有停止规则

- **WHEN** 契约类 finding 的有效必关闭投影生效
- **THEN** A 池 `max_repairs` 与 B 池 `max_manual_repairs` 的余额 MUST 保持不变；Verification scope 中出现不属于原始指纹集合的新 finding 时，MUST 继续按既有 `VerificationNewFindings` 停止规则处理，MUST NOT 因本投影改变该规则

#### Scenario: 闭合资格为假时 hint 不得升级

- **WHEN** 一条 finding 在解析平台硬化后的**闭合资格为假**（WorkItemPlan 作用域下 `severity` 不属于 `{blocking, must_fix}` 且 `category` 不属于契约类），其 `disposition` 为 `actionable`，且模型给出 `class_hint=repairable` 或 `class_hint=human_required`
- **THEN** 中央策略 MUST 将其判为 `Advisory`；MUST NOT 因 `class_hint` 将其升级为 `Repairable` 或 `HumanRequired`；MUST NOT 计入非 advisory 指纹集合、MUST NOT 消耗返修预算、MUST NOT 触发自动返修或人工门；更换 `class_hint` MUST NOT 改变该结论

#### Scenario: 反向投影的作用域限于 WorkItemPlan

- **WHEN** 通用 Story/Design 等非 WorkItemPlan 评审产出 finding 并被分类
- **THEN** 该 finding MUST 继续按该评审的既有分类规则处理，MUST NOT 因本反向投影发生改变；本投影的作用域 MUST 随既有可信入口传入的作用域确定，MUST NOT 由模型输出的任何标签授予或扩大

#### Scenario: 机械 Error 不被模型建议覆盖

- **WHEN** 一条机械 finding 沿可信机械入口进入控制面，或一条模型建议携带 `contract_field="mechanical.xxx"`
- **THEN** 机械 Error MUST 继续沿该可信机械入口被判定与呈现，MUST NOT 被模型建议覆盖或降级；模型建议 MUST NOT 伪装成机械通道，MUST NOT 因此获得自动修复许可或闭合资格

#### Scenario: 许可证据须绑定修复对象且不得以预算载体反证

- **WHEN** 中央策略判定一条 finding 的自动修复许可，且此刻尚不存在服务端 `RepairReservation`
- **THEN** 资格判定 MUST NOT 以「先创建 `RepairReservation`、再以该载体反证自己有权修复」的方式进行；自动修复许可 MUST 与该 finding 的 source / context / 修复对象绑定；`class_hint`、`category`、`severity`、非空 `contract_field` 以及预算余额 MUST NOT 单独构成许可证据

#### Scenario: 反向投影不改写 finding 身份且 hint 降为审计字段

- **WHEN** 中央策略因闭合资格为假把一条 finding 判为 `Advisory`
- **THEN** 该 finding 的 `severity`、`message` 与内容身份 key MUST 保持模型原值，owner 既有排除项的匹配与幂等语义 MUST NOT 改变；`class_hint` MUST 仅保留为审计信息，MUST NOT 再决定分类结果
