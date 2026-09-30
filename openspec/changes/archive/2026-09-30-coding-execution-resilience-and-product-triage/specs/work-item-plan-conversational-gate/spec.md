# Spec Delta

## MODIFIED Requirements

### Requirement: SC manual revision 专属路径（REQ-CG-03）

人工反馈触发的单候选修订 SHALL 走 SC 专属路径：以反馈文本、当前候选 markdown 全文与必要 grammar 边界构造独立预算的 revision prompt；provider SHALL 输出完整修订版 markdown（非 diff/patch），输出经确定性前言修剪后进入 SC compiler 与 canonical validator。该路径 SHALL NOT 经过 legacy 中文标题 artifact 约束链；SHALL NOT 复用或扩张 SC author prompt 的 19,000 字节预算；修订 prompt SHALL 注入 `.claude/rules/language.md` 全文与结构字面量优先规则句以维持中文 plan 与 grammar 字面量纪律，SHALL NOT 注入 code-usage/code-reading 摘要。

反馈文本 SHALL 受确定性 bounded-field 长度限制；超限 SHALL 在创建 turn 前拒绝，零预算消耗、零 provider 启动。创建 turn 与扣减回合预算之前，系统 SHALL 以 UTF-8 字节计算完整组装输入（当前候选全文＋固定合同＋feedback＋上下文）并与所选 provider 的真实输入预算比较：不超过 inline 预算时整体内联发送；超过 inline 预算但在 provider 硬限内时 SHALL 通过既有 artifact 读取能力或完整有序分块传输完整候选原文，并记录组装 digest；候选 SHALL NOT 被截断、摘要替代或以“模型可自行读取本地路径”为前提，SHALL NOT 新增第二个候选权威。artifact 不可读、分块缺失或超过 provider 硬限时，系统 SHALL 在 turn CAS 之前拒绝并通知用户，门状态与预算不变；用户点击“分段返修／重试”后才开启新回合。每个 `turn_id` 的 `attempt_no` SHALL 有固定上限；每次真实 provider start SHALL 写入 provider-start ledger，但逻辑预算每个回合只消耗一次，SHALL NOT 以 WebSocket 事件推断启动次数。修订教学 SHALL 写死「只改反馈点名的内容，其余逐字保留」及反面清单（禁止删字段、清空 Outputs、省略 Handoff Schema 三字段）。修订后 findings 与既有指纹重复时 SHALL 按阶段 1 契约回到同一人工门。

#### Scenario: 修订成功回呈

- **WHEN** 回合的 provider 输出通过 SC compiler 与 canonical validator
- **THEN** 系统以 `human_gate_turn_completed` 回呈新候选 artifact 引用，人工门回到等待态，人可继续反馈或 approve

#### Scenario: 修订不过校验

- **WHEN** 修订输出被 canonical validator 拒绝
- **THEN** turn 置 `Failed(validation_reject)`，当前候选保持不变，已扣预算不退还，人可在剩余预算内再开回合

#### Scenario: 反馈超长零副作用拒绝

- **WHEN** 反馈文本超过 bounded-field 上限
- **THEN** 系统拒绝创建 turn，预算与 provider 启动计数不变；人收到超限原因后可改短重发（新 `command_id`）

#### Scenario: 大候选完整返修

- **WHEN** 当前候选为 28,695 字节，与固定合同和 feedback 组装后超过 inline 预算但在 provider 硬限内
- **THEN** 系统以 artifact 或完整有序分块传输完整候选原文，provider 基于完整候选返修，组装 digest 被记录，候选未被截断

#### Scenario: 分块缺失或超硬限在 CAS 前停等

- **WHEN** 组装时 artifact 不可读、任一分块缺失，或完整输入超过 provider 硬限
- **THEN** 系统在创建 turn 与扣减预算前拒绝并通知用户，门保持等待态、预算与 provider 启动计数不变；用户点击“分段返修／重试”后才开启新回合

#### Scenario: 同指纹重现回同一门

- **WHEN** 修订后的新候选经复评产出的 findings 与既有指纹重复
- **THEN** 按阶段 1 防环契约回到同一人工门，不新建门实例，不重置预算

#### Scenario: 不触碰 legacy 约束链

- **WHEN** SC manual revision 产出英文 grammar 标题的 markdown
- **THEN** 产物只经 SC compiler/canonical validator 判定，不被 legacy 中文标题 artifact 约束拦截
