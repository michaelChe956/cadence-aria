# workspace-artifact-open-item-validation Delta

## Purpose

定义 workspace Story artifact「待确认项」章节的已解决/未解决判定行为，保证合法的"无待确认项 + 解释"表述不被误判，同时真正未解决的开放问题仍被拦截。

## ADDED Requirements

### Requirement: 空标记开头的待确认项节视为已解决

若 Story artifact「待确认项」章节的首个非空行以空标记词开头（如「无待确认项」「无」「暂无」「none」等），则整节判定为已解决；后续解释性文字不触发未解决判定，artifact 校验不得因此拒绝该 artifact。

#### Scenario: 空标记后附带解释
- **WHEN** 待确认项节正文为「无待确认项。Issue 已明确需求与验收约束，实现细节留待 Design 阶段决策，不构成本 Story 的未决问题。」
- **THEN** 该节判定为已解决，artifact 不因待确认项被校验拒绝

#### Scenario: 真未解决待确认项仍被拒绝
- **WHEN** 待确认项节正文为「单元测试运行器选型仍待确认。」且不含已解决 cue
- **THEN** 该节判定为未解决，artifact 校验报告「待确认项未通过 AskUserQuestion 交互解决」类禁止内容

### Requirement: 生成提示约束空标记写法

Story 生成阶段的待确认项策略提示必须明确：若无开放问题，「待确认项」正文只写「无待确认项」，解释性内容写入其他章节。

#### Scenario: 提示文案包含只写空标记指示
- **WHEN** 构造 Story 的待确认项策略提示
- **THEN** 提示包含「正文只写『无待确认项』，不得附加解释」含义的指示
