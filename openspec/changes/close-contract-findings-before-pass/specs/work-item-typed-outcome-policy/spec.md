# Spec Delta

## ADDED Requirements

### Requirement: 契约类 finding 的有效必关闭在中央策略中的投影（REQ-TOP-07）

中央策略 SHALL 以「有效必关闭」判定消费 finding：`class_hint=advisory` 与 raw `pass` fallback MUST NOT 把有效必关闭的契约类 finding 判为 `Advisory`。有效必关闭但缺可验证定位或服务端修复许可的 finding SHALL 落 `HumanRequired` 而非 `Repairable`。本投影 MUST NOT 提高 A 池 `max_repairs` 或 B 池 `max_manual_repairs` 余额，MUST NOT 改变 Verification scope 新 finding 的既有停止规则。

#### Scenario: advisory 提示不得盖过有效必关闭

- **WHEN** 一条 finding 在解析平台硬化后属有效必关闭的契约类 finding，且模型给出 `class_hint=advisory`
- **THEN** 中央策略 MUST NOT 将其判为 `Advisory`；其分类 MUST 与平台侧有效必关闭结论一致

#### Scenario: raw pass 不得把契约类 finding 判为建议项

- **WHEN** 一条有效必关闭的契约类 finding 所在轮次的原始结论为 `pass`，且该 finding 没有任何 `class_hint`
- **THEN** 中央策略 MUST NOT 以 raw `pass` 的建议项兜底把它判为 `Advisory`；MUST NOT 因此把它排除在返修需求判定之外

#### Scenario: 分类优先级与原始 verdict 来源不冲突

- **WHEN** 分类器为 finding 判定分类
- **THEN** 分类的 verdict 维度 SHALL 继续使用 raw_verdict（不因本投影改用归一后的 gate）；但对有效必关闭的契约类 finding，其分类 MUST NOT 由 `class_hint` 或 raw verdict 的兜底决定，平台侧的有效必关闭事实 SHALL 优先

#### Scenario: 无定位转人工而非可自动返修

- **WHEN** 一条有效必关闭的契约类 finding 没有可验证定位（例如缺 `contract_field` 或其值不可用），且不存在已有的合法机械修复证据
- **THEN** 中央策略 SHALL 将其裁决为 `HumanRequired`；MUST NOT 裁决为 `Repairable`，MUST NOT 因此触发自动返修或新增自动重写次数

#### Scenario: 不得凭类别或严重度自造修复许可

- **WHEN** 中央策略仅依据 finding 的 `category` 或 `severity` 判断其可自动修复性
- **THEN** 策略 MUST NOT 据此新造可自动修复的许可；是否可自动返修 MUST 依据既有定位与服务端授权证据

#### Scenario: 不提高余额且不改既有停止规则

- **WHEN** 契约类 finding 的有效必关闭投影生效
- **THEN** A 池 `max_repairs` 与 B 池 `max_manual_repairs` 的余额 MUST 保持不变；Verification scope 中出现不属于原始指纹集合的新 finding 时，MUST 继续按既有 `VerificationNewFindings` 停止规则处理，MUST NOT 因本投影改变该规则
