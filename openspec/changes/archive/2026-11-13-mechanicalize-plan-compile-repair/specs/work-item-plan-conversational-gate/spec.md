# 规范增量：自动处置回显与一次答完

<!-- 设计：cadence/designs/2026-10-31_技术方案_Plan生成不可控性治理_v1.0.md §5.3。
     复用 REQ-WSC-13/14 的已处置事实清单与 owner_question/owner_options 渲染，不新造状态机。 -->

## ADDED Requirements

### Requirement: 平台自动处置结果必须回显给 owner（REQ-CG-08）

候选存在自动修复或等价规范化时，人工门 MUST 呈现「平台已自动处理 N 项」并可展开每项的依据与前后内容。owner MUST NOT 被要求回答平台已机械解决的问题，门卡文案 MUST NOT 声称平台修改了 design 或业务范围。

#### Scenario: 处置结果可见

- **WHEN** 候选编译前应用了 2 项自动处置后通过
- **THEN** 门卡 MUST 显示处置计数与明细入口，且 MUST NOT 把这两项列入待处理项

#### Scenario: 不越权表述

- **WHEN** 平台对 plan 内字段做了等价规范化
- **THEN** 门卡文案 MUST NOT 声称修改了 design 或业务范围

### Requirement: 需要 owner 判断的事项一次答完（REQ-CG-09）

编译失败需要 owner 判断时，verdict MUST 同时填充 owner_question 与至少两个 owner_options，问句与选项 MUST 由 typed 事实生成并标注各选项后果。同一轮门 MUST 把全部待决事项合并呈现，owner 的回答 MUST 落入已处置事实清单，后续轮次 MUST NOT 就同一未变事实再次提问。

#### Scenario: 环根因在 design 时合并一次问清

- **WHEN** 依赖环无法在 plan 内消解且根因是 design 仓序
- **THEN** 门 MUST 一次呈现一个问题与至少两个选项并逐项说明影响

#### Scenario: verdict 不得留空问句

- **WHEN** 门判定需要 owner 决定
- **THEN** verdict MUST 含 owner_question 与至少两个 owner_options，缺失时 MUST NOT 渲染空白裁决块

#### Scenario: 已回答事实不再重复提问

- **WHEN** owner 已回答某事实且后续轮次该事实未变化
- **THEN** 门 MUST NOT 再次以 owner_question 呈现该事实
