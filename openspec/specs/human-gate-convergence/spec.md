# human-gate-convergence Specification

## Purpose

人工门（SC 修订门/批次确认门）的预算权威、收敛可见性与呈现区分：预算只由 durable 事实证明并真实递减；用户在门卡上能看到「还剩多少轮、距通过差什么、这轮新解决/复现了什么」；两道连续确认门可区分。

## Requirements

### Requirement: 门预算权威与真实递减（REQ-HGC-01）

当前 logical gate 的 `manual_repairs_remaining` SHALL 是反馈可发送性与门卡预算显示的唯一授权事实：CAS 预留恰扣一次（同 command replay 不重复扣）；同一 logical gate 内的修订轮次 SHALL carry-forward 预算（不得每轮重置回默认）；仅新 logical gate（新候选/新会话/REQ-CG-07 修订门重开按其接续语义）才建立新预算。门卡 SHALL 显示当前真值；接受的人工反馈轮次 SHALL 以 gate-local `accepted_feedback_turns`（或 durable HumanGateTurn 确定性派生，二选一不双计）呈现，SHALL NOT 以 policy `repairs_used`（仅自动返修）或静态快照冒充。预算耗尽时门保持开启仅接受 approve/abandon（既有语义），门卡如实显示耗尽态。旧会话缺 gate-local 事实时保守显示「预算历史不可用」，不补预算不改历史。

#### Scenario: 三轮修订真实递减

- **WHEN** 同一 logical gate 内用户连续提交 3 次反馈且每次修订成功
- **THEN** 门卡剩余预算依次递减（如 3→2→1→0），耗尽后仅剩确认/终止且如实显示

#### Scenario: 刷新不回填默认

- **WHEN** 刷新/重连后门卡重建
- **THEN** 预算取 durable snapshot/turn 真值，不显示默认满额

### Requirement: 收敛投影与门区分（REQ-HGC-02）

门卡 SHALL 从 durable review/timeline/snapshot 事实派生并展示：当前轮 findings 的跨轮 delta（新增/已解决/复现/未能判断——findings 为空不得推断历史问题已解决，历史不全显式 unknown）；「距通过」清单至少含当前 must_fix/error 项、C1 options 预检结果、Verification scope 状态与 approve 可用性。**批次确认门**（compile 成功后等待发布确认）SHALL 使用专属标题与说明（如「确认发布整组 Work Items」+发布含义一句），与候选修订确认门（「需要人工确认」+修订上下文）在标题/原因行层级可区分；SHALL NOT 复用相同文案造成两道门不可分辨。

#### Scenario: 批次门可辨认

- **WHEN** approve→compile 成功后批次确认门打开
- **THEN** 门卡标题/文案明确表达「确认发布整组 Work Items」及其后果，用户能将其与前置的候选修订门区分

#### Scenario: 距通过清单

- **WHEN** 门卡渲染
- **THEN** 显示当前阻断项（must_fix/预检缺口）与「全部解决即可确认」的收敛路径；无阻断项时显示可直接确认的原因

#### Scenario: 跨轮 delta

- **WHEN** 本轮复评完成回到门
- **THEN** 门卡显示本轮新增/已解决/复现的 findings 计数（可折叠明细），历史不全的字段显示 unknown 而非猜测

### Requirement: 有效分类透明与解析韧性（REQ-HGC-03）

UI SHALL 同时显示 finding 的 severity（用户语义）与 effective class（class_hint 优先的策略语义），二者不一致时不得仅显示其一造成「建议不阻断」误导。reviewer 结构化输出解析失败（invalid_json 等）时系统 SHALL 在同 invocation 内执行恰一次有界重试（不增加 review 计数/返修/人工预算）；重试仍失败 SHALL 保留原始 diagnostic 进入人工处理，SHALL NOT 合成空返修目标或静默丢弃。

#### Scenario: 双轨分类可见

- **WHEN** 某 finding severity=advisory 而 effective class 为 must_fix
- **THEN** 门卡同时呈现两者（或以 effective class 为主显+severity 标注），不再出现「均为建议级」文案而门实际阻断的矛盾

#### Scenario: 解析失败一次重试

- **WHEN** reviewer 结构化输出 JSON 解析失败
- **THEN** 同 invocation 静默重试一次；成功则正常消费；失败则带原始输出片段进人工，该轮不计为一次完整 review
