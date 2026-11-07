# human-gate-convergence Specification

## Purpose

人工门（SC 修订门/批次确认门）的预算权威、收敛可见性与呈现区分：预算只由 durable 事实证明并真实递减；用户在门卡上能看到「还剩多少轮、距通过差什么、这轮新解决/复现了什么」；两道连续确认门可区分。

## Requirements

### Requirement: 门预算权威与真实递减（REQ-HGC-01）

当前 logical gate 的 `manual_repairs_remaining` SHALL 是**反馈可发送性**与**人工返修余量呈现**的唯一授权事实：CAS 预留恰扣一次（同 command replay 不重复扣）；同一 logical gate 内的修订轮次 SHALL carry-forward 预算（不得每轮重置回默认）；仅新 logical gate（新候选/新会话/REQ-CG-07 修订门重开按其接续语义）才建立新预算；门卡 SHALL 显示当前真值。当前门卡的重建 / 重连 SHALL 刷新同门当前事实，旧卡只读历史事实、SHALL NOT 呈现为当前授权。人工返修预算耗尽时门保持开启仅接受 approve/abandon（既有语义），门卡如实显示耗尽态（「耗尽」指 durable 事实为 0 的那一池，SHALL NOT 由 `repair_budget_exhausted` trigger 推断）。

#### Scenario: 三轮修订真实递减

- **WHEN** 同一 logical gate 内用户连续提交 3 次反馈且每次修订成功
- **THEN** 门卡剩余预算依次递减（如 3→2→1→0），耗尽后仅剩确认/终止且如实显示

#### Scenario: 刷新不回填默认

- **WHEN** 刷新/重连后门卡重建
- **THEN** 预算取 durable snapshot/turn 真值，不显示默认满额

#### Scenario: 三个额度各自命名

- **WHEN** 界面呈现任一类额度
- **THEN** 该额度 SHALL 以自己的名字呈现（「平台自动重写」/「人工返修」/「你已说明 N 轮」），MUST NOT 使用「自动修复」统称；机械返修池（`max_repairs`）MUST NOT 以额度名出现在门卡上

#### Scenario: 共用上限只作说明、不得承诺合并值

- **WHEN** 界面同时呈现平台自动重写与人工返修
- **THEN** 文案 MAY 用说明性文字表达「平台自动重写与你的反馈共用同一份额度上限」，MUST NOT 承诺任何算术合并后的余量值，MUST NOT 宣称两者已并成一个池，MUST NOT 让 owner 误以为界面上已经给出可用总额

#### Scenario: 两个账面事实分别呈现

- **WHEN** 门卡呈现额度余量
- **THEN** 门卡 SHALL 分别呈现 B1（`auto_rewrite.remaining`）与 B2（`manual_repairs_remaining`）两个账面事实，并各自标注来源与用途；MUST NOT 任选其一充当「共享余额」，MUST NOT 合成跨账本的猜测值，MUST NOT 隐藏其中之一后宣称共享余额已确定

#### Scenario: 共享余额须有后端权威事实

- **WHEN** 后端尚未提供同一预算域的权威合并事实
- **THEN** 门卡 MUST NOT 展示「共享余额」，SHALL 呈现两个来源各自的账面事实 + 「两处账本尚未合并」的说明

#### Scenario: 「已用完」不得由 trigger 直接断言

- **WHEN** 门卡触发原因为 `repair_budget_exhausted`
- **THEN** 文案 MUST NOT 直接断言「次数已用完」；SHALL 按触发路径（`flowKind`）分流呈现，无法判定路径时 SHALL 使用中性文案「本轮评审自动返修已停止，需要你决定下一步。」；SC 分流的固定标签 MUST NOT 被渲染为「次数已用完」

#### Scenario: 停机态文案不得断言与 durable 事实相反

- **WHEN** 会话停在等 owner 回话
- **THEN** 停机卡文案 MUST NOT 断言「人工说明次数已用尽」或「预算已耗尽」，除非 durable 事实确实如此；SHALL 指向「写说明 + 点继续自动化」这一实际出路

#### Scenario: 原始事实仍可在技术详情中查看

- **WHEN** owner 展开技术详情
- **THEN** 三类额度的原始计数值 SHALL 仍可见，事实 MUST NOT 因改名而丢失

#### Scenario: 两个来源余量不一致时分别呈现且不承诺共享余额

- **WHEN** B1 的 `auto_rewrite.remaining` = 2 且 B2 的 `manual_repairs_remaining` = 3
- **THEN** 门卡 SHALL 分别写出两个账面事实与「两处账本尚未合并」的说明；MUST NOT 承诺共享余额为 2、3 或 5

#### Scenario: 结构或复验阻断不产生「次数用完」

- **WHEN** 门因 Verification 或结构 grant 阻断，且 cycle 尚未耗尽
- **THEN** 门卡 MUST NOT 出现「次数用完」；MUST NOT 补造机械返修池计数为 0

#### Scenario: 停止原因与正余额并存且互不覆盖

- **WHEN** B1 已 stopped 且 `auto_rewrite.remaining` > 0，且重复环命中
- **THEN** 门卡 SHALL 同时保留「因同环重复停止」的原因与该正余额，两者 MUST NOT 互相覆盖（符合 REQ-WSC-22）

#### Scenario: 同门反馈受理后的真实递减与重建不回填

- **WHEN** 同一 logical gate 受理一次反馈
- **THEN** B2 余量 SHALL 由 3 递减为 2、已说明轮次由 0 递增为 1；成功重建与重连 SHALL NOT 回填为 3；结构释放后 B2 余量 MAY 恢复，而已说明轮次 SHALL 保留真实受理事实

#### Scenario: 旧数据缺事实时的保守呈现

- **WHEN** 门卡所属记录缺少额度事实
- **THEN** 门卡 SHALL 显示「预算历史不可用／共享余额无法确认」，MUST NOT 补默认值，MUST NOT 让旧卡读成当前授权

#### Scenario: 旧卡只读历史事实、不得呈现为当前授权

- **WHEN** 打开一张非当前门卡
- **THEN** 该卡 SHALL 只读历史事实，SHALL NOT 呈现为当前授权

#### Scenario: 人工返修预算耗尽时的门与耗尽态呈现

- **WHEN** 人工返修预算的 durable 事实为 0
- **THEN** 门 SHALL 保持开启且仅接受 approve/abandon；门卡 SHALL 如实显示耗尽态，且该耗尽态 SHALL NOT 由 `repair_budget_exhausted` trigger 推断

#### Scenario: 共池权威事实缺席时只验收呈现修复

- **WHEN** 共享权威事实尚不存在
- **THEN** 本次只验收额度呈现层修复；MUST NOT 把 REQ-WSC-21 的预算隔离与共池执法标记为已修复

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

### Requirement: 过期人工门卡按 durable 停点分流指引（REQ-HGC-04）

对话流中的**过期门卡**（该门已不是当前需处理的门）SHALL 按会话的 durable 停点状态给出对应文案与**真实可操作入口**；MUST NOT 把 owner 引向会作废当前这一轮的动作（终止），除非该动作确实是此刻唯一可用的出口。

**本 requirement 只适用于过期门卡。** 非过期门卡（正在等 owner 拍板的那张）的既有文案与动作面 SHALL 保持不变，**其「终止此门」按钮 MUST 保留**。

文案与入口 MUST 由**单一事实源**产出，MUST NOT 散落在多处各自拼接。分流 MUST 以 durable 会话状态为主键（MUST NOT 仅凭「这张卡过期了」推断）。

#### Scenario: 停在等 owner 回话时指向继续而非终止

- **WHEN** 会话 durable 状态为「停在等 owner 回话」，且该门卡已过期
- **THEN** 门卡文案 SHALL 说明该卡已过期，并指向页面底部的输入框 +「继续自动化」（该入口实际可用时）；门卡 MUST NOT 出现「只能终止此门后重新发起」类文案，MUST NOT 出现「终止此门」按钮

#### Scenario: 本轮失败时指向重试

- **WHEN** 会话 durable 状态为「本轮失败」，且该门卡已过期
- **THEN** 门卡文案 SHALL 说明该轮已失败，并指向「继续自动化」重试（并说明可附补充说明）；MUST NOT 出现「终止此门」按钮

#### Scenario: 会话已终止时视为历史记录

- **WHEN** 会话 durable 状态为「已终止」，且该门卡已过期
- **THEN** 门卡文案 SHALL 说明该卡是历史记录、不需处理；MUST NOT 出现「终止此门」按钮

#### Scenario: 推进到别的门时以最新门卡为准

- **WHEN** 会话已推进到别的门，该门卡已过期
- **THEN** 门卡文案 SHALL 指明以页面最新的一张门卡为准；若该轮确实需要作废，SHALL 保留「终止此门」作为可选出口，但 MUST NOT 表述为唯一出路

#### Scenario: 会话已确认完成的历史卡不回归

- **WHEN** 会话 durable 状态为「已确认完成」，且该门卡已过期
- **THEN** 文案 SHALL 维持「这一版已确认完成，此卡是历史记录，无需处理」的既有表述，且 MUST NOT 出现「终止此门」按钮

#### Scenario: 非过期门卡的动作面不变

- **WHEN** 该门卡并未过期（owner 仍可在此拍板）
- **THEN** 其既有文案与动作面 SHALL 保持不变；「终止此门」按钮 SHALL 仍然可用

#### Scenario: 无真实续跑入口时不得给出假指引

- **WHEN** 会话已停在等 owner 回话，但服务端未提供继续动作（页面底部没有真实入口）
- **THEN** 门卡文案 MUST NOT 指引 owner 去点「继续自动化」；SHALL 改为指向刷新页面查看最新状态或到自动化控制面处理

#### Scenario: 「本轮是否可继续」在全部会话状态投递路径上一致

- **WHEN** 会话状态因刷新、重连或观察者镜像重建而重新投递
- **THEN** 「本轮是否可继续」的取值 SHALL 与首次投递一致；MUST NOT 在这些路径上退化为「未知」而使门卡给出假指引

#### Scenario: 文案与真实按钮措辞一致

- **WHEN** 门卡指引指向「继续自动化」
- **THEN** 该指引所指按钮的文案 SHALL 与页面底部真实按钮的文案逐字一致
