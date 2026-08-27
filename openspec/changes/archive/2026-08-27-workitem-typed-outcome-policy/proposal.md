## Why

workitem 段实测（2026-08-25，codex 四连跑）暴露的直接死因：reviewer 判 `needs_human` 后流程无确定出口，自动化路径死循环 11 轮烧 25 分钟硬超时；返修无预算、无去重，重复问题反复重审。

本变更是 workitem 架构半重构的**阶段 1（控制面止血）**：在现有 JSON 生成路径上引入 typed finding 分类、中央策略层与持久化运行状态，先消除死循环、让每次运行到达明确终态。完整架构方向（markdown 编译器模型 C′、单候选事务、对话流人工门）见 `openspec/changes/rearch-workitem-plan-pipeline/`（阶段 2-4）。

关键设计原则（oracle 裁决，用户已批准）：

- `HumanRequired` 是正常业务结果，不是系统故障；auto 模式下以 `StoppedNeedsHuman` 终态落盘，不标 Fatal、不空转等待
- 自动返修、人工返修、初评与复评分别受 durable counter 约束：`repairs_used`、`manual_repairs_used`、`initial_review_count`、`verification_review_count`
- 人工反馈返修不消耗自动返修预算，但受独立人工返修预算约束（默认 3）；人工返修后同一指纹重现 → 回到同一个人工门，不自动再试
- review 语义两阶段：初评 ≤1 + 返修后复评 ≤1（总 ≤2）；复评只验证原指纹、改动区域与重跑机械校验，不再开放式找问题

## What Changes

- reviewer finding schema 扩展：`category`（可枚举）、`class_hint`、`contract_field` 三个机器可读字段；旧数据经 serde default 兼容；未知 category 失败关闭
- parser 保留 provider 原始判决：`ParsedReviewEnvelope { raw_verdict, normalized_gate, findings }`。分类器只依据 `raw_verdict`，策略路由只依据 typed outcome；强 finding 不得把原生 `needs_human` 改写为 `revise` 后再分类
- 新增确定性分类器 `classify_review`：消费 reviewer 归类建议 + 确定性兜底规则，产出 `ClassifiedFinding`；策略层不再接触原始 `ReviewVerdict`
- 新增中央策略评估器 `evaluate`：只消费已分类 finding、服务端生成并持久化的 review invocation scope、运行历史与预算，裁决四类 typed outcome（`valid / repairable / human_required / fatal`）
- 分类解析错误按固定链路失败关闭：parser error → `ClassificationError` → `FatalReason`（含 `UnknownCategory`）→ durable `failed` 状态；诊断码 `unknown_finding_category`；classification error 不得进入 `HumanRequired`
- 终态矩阵（D-A）：交互模式重复指纹 → 唯一人工门（AwaitingHuman）；auto 模式重复指纹/原生 human_required/返修预算耗尽 → `StoppedNeedsHuman` 终态落盘；仅状态损坏、未知协议、持久化失败、安全不变量破坏、transition budget 耗尽等系统错误 → `Fatal`
- review 两阶段（D-B）：初评发现可自动修复问题时才允许 1 次**全局聚合**自动返修；机械错误与 reviewer repairable finding 共享同一个 `max_repairs=1` 预算；返修后复评 ≤1 次，且只限原指纹、改动区域和机械报告
- session 持久化三个字段：`flow_kind`（legacy/single_candidate，创建时按 rollout flag 固定，运行中不变）、`run_policy`（interactive/auto_if_valid，prepare 请求显式传入）、`run_history`（指纹集与四个 durable counter）；旧 JSON 经 serde default 恢复为 legacy+interactive
- 人工门持久化快照：待决人工门写入 SessionState，断线重连后恢复，不丢门、不重复门；`stopped_needs_human` 的接管以 interactive 策略启动**新运行**，原运行终态记录不可篡改
- 指标分离：`fatal` 与 `stopped_needs_human` 分别计数，campaign 成功率统计不被人工门事件污染
- campaign driver：prepare 显式传 `run_policy`；首条 session state 校验 flow_kind 不符即失败关闭；review/返修次数从服务端持久计数读取，不靠 stage 名猜测

## Capabilities

### New Capabilities
- `work-item-typed-outcome-policy`: workitem 段的 typed finding 分类、中央策略裁决、终态矩阵、两阶段 review scope、运行历史持久化与人工门快照恢复

### Modified Capabilities
（无——本变更在现有路径上叠加策略层，旧协议消息不变；旧路径退役由后续阶段变更承担）

## Impact

- **新增**：`src/product/work_item_plan_policy/`（类型、分类器、评估器、预算、指纹、review scope、人工门快照）
- **修改**：`src/web/workspace_ws_types/review.rs`（finding schema 扩展）、reviewer 输出 parser、`src/product/workspace_engine/review/routing.rs`（verdict → 策略层，不直接跳转）、session 持久化链路（`models/workspace.rs`、`lifecycle_store`、`web/handlers/lifecycle.rs` prepare）、`src/web/workspace_ws_handler/`（运行中策略变更拒绝、重连快照）
- **campaign**：`cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs` 适配
- **不动**：`work_item_split_validator` 校验规则、compile 事务、coding engine/WS、story/design 段、前端（本阶段人工门 UI 沿用现有 human_confirm 呈现；对话流式改造在阶段 3）
- **真实 provider 验证需操作者授权**（项目规则 Case A/B），未授权前只跑 dry-run/fixture/单测
