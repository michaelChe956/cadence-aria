# CodingWorkspace 角色运行事件日志与刷新恢复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把 `cadence/designs/2026-06-13_技术方案_CodingWorkspace角色运行事件日志与刷新恢复_v1.0.md` 拆成可在单个 session 内完成、可测试、可独立交付的实施计划。

**Architecture:** 采用三阶段拆分：P1 先补后端 role run event JSONL 持久化和 engine 双写；P2 再把事件摘要放入 `CodingSessionState` 并扩展 `RoleRunHistoryPanel`；P3 最后把 retry diagnostic summary 接入 prompt，并用真实 E2E 覆盖刷新后重试。每个阶段都以 P1 产出的 store API 为边界，避免把 UI、prompt 和日志底座混在一个提交里。

**Tech Stack:** Rust 1.95.0、Serde JSON、Tokio、Axum WebSocket、Zustand、React、Vitest、Playwright。

---

## 落地条件检查

当前 design 已满足实施落地条件：

- 需求边界清楚：只覆盖 `Tester`、`Analyst`、`CodeReviewer`、`InternalReviewer`，不覆盖 `Coder`。
- 数据形态清楚：每个 `CodingRoleRun` 对应一个 JSONL 事件日志文件，role run 主 JSON 只保留状态、refs、重跑关系。
- 实时语义清楚：provider event 先规范化并尝试落盘，再继续按现有 WebSocket 发送；日志写入失败不能阻断实时输出。
- JSON 契约清楚：最终 `TestPlan`、`TestingReport`、`AnalystDecision`、`CodeReviewReport`、`InternalPrReview` 仍然保持 JSON-only；过程事件承担实时可读进度。
- 验收点清楚：刷新恢复、blocked/running 最近事件、retry 保留旧 run 日志、新 run 独立日志、`No tasks found` 类过程输出可追溯。

作为单个实施计划，当前 design 范围过大。它同时改动后端 store、engine、WebSocket contract、前端类型、前端 UI、retry prompt 和 E2E。直接放进一个 plan 会跨越多个故障域，且很难在一个 session 内完成完整 TDD 与验收。因此拆成三个 plan。

## 拆分计划

1. `cadence/plans/2026-06-13_计划文档_实施计划_CodingWorkspace角色运行事件日志与刷新恢复_P1_后端事件日志_v1.0.md`

   目标：新增 `CodingRoleRunEvent` 模型、JSONL append/list store API、事件大字段 artifact 截断，并在 `run_provider_stream_to_completion` 对目标角色做双写。P1 完成后，后端已经能把 Tester/Analyst/Reviewer/InternalReviewer 的过程事件写入 `.aria`，且实时 WebSocket 行为保持不变。

2. `cadence/plans/2026-06-13_计划文档_实施计划_CodingWorkspace角色运行事件日志与刷新恢复_P2_刷新快照与历史UI_v1.0.md`

   目标：在 `CodingSessionState` 中返回每个 role run 的 `event_summary` 和 `recent_events`，前端 `RoleRunHistoryPanel` 展示 event count、last event、recent events 和 artifact ref。P2 完成后，刷新页面可以看到 running/blocked role run 的最近过程事件。

3. `cadence/plans/2026-06-13_计划文档_实施计划_CodingWorkspace角色运行事件日志与刷新恢复_P3_重试诊断与真实E2E_v1.0.md`

   目标：为 retry prompt 引入上一轮 role run 的诊断摘要和 refs，保证 retry 不注入完整日志全文；补充真实 E2E 覆盖 Tester 过程事件、刷新恢复、blocked retry，以及 Analyst/Reviewer/InternalReviewer 的重试诊断链路。

## 依赖顺序

- P1 必须先做。P2/P3 都依赖 `append_role_run_event`、`list_role_run_events` 和事件模型。
- P2 可在 P1 合并后独立完成，不依赖 P3。
- P3 需要 P1 的事件日志读取能力，真实 E2E 中的 UI 断言依赖 P2。

## 每阶段验收边界

P1 验收：

- 后端 store 测试证明事件写入、读取、sequence、artifact 截断可用。
- engine 测试证明 `provider_prompt`、`provider_start`、`execution_event`、`text_delta`、`tool_call`、`tool_result`、`message_complete`、`timeout` 会写入目标 role run。
- 既有 WebSocket forwarding 测试继续通过。

P2 验收：

- WebSocket snapshot 测试证明 role run 携带 `event_summary` 与 `recent_events`。
- 前端类型、store、组件测试证明刷新后能展示最近事件。
- 主 chat 不新增全量日志回放。

P3 验收：

- retry prompt 测试证明上一轮 reason code、terminal event、recent events、raw refs、event artifact refs 被压缩成诊断摘要。
- Tester JSON-only 最终产物契约不变，过程输出继续走 event log 和 WebSocket。
- Playwright E2E 覆盖刷新后 recent events 与 retry 可见行为。

## 执行建议

推荐按 P1、P2、P3 顺序执行，并在每个 plan 完成后独立提交。每个 plan 都控制在一个 session 可完成范围内，且单个文档内容小于 50k。

