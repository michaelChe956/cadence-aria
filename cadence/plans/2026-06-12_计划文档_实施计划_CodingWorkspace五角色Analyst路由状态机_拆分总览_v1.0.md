# CodingWorkspace 五角色 Analyst 路由状态机拆分总览 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement each detailed P plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking in the detailed plans.

**Goal:** 将 `cadence/designs/2026-06-12_技术方案_CodingWorkspace五角色Analyst路由状态机_v1.0.md` 拆成多个可在单个 session 内完成、可独立验证的实施计划。

**Architecture:** Analyst 作为统一路由决策节点，Tester、CodeReviewer、InternalReviewer 只产出证据。实施按“契约先行、后端路由、人工门禁、前端展示、真实验收”的顺序推进，每个阶段都保留现有工作流可运行。

**Tech Stack:** Rust 1.95.0、Cargo、Axum WebSocket、serde JSON、React、TypeScript、Zustand、Vitest、Playwright。

---

## 拆分原则

- 单个详细 plan 控制在一个实现 session 内完成，目标文档规模小于 150k tokens，并给实现、调试、测试留出上下文。
- 每个详细 plan 必须产生可验证的软件变化，测试随功能内聚，不把全部测试堆到最后。
- 不新增 `CodingExecutionStage::Analyst`。短期继续复用现有 `CodingExecutionStage::Rework` 作为 Analyst 执行阶段，降低迁移范围。
- 保留现有 `AnalystVerdict` 聊天展示兼容层，新增结构化 `AnalystDecision` 持久化契约作为后续路由依据。
- P1 完成前不修改完整状态机路由；P2-P4 基于 P1 的结构化决策逐步替换硬编码阶段推进。

## 当前前置状态

- 工作目录：`.worktrees/bugfix_branch`
- 当前分支：`bugfix_branch`
- 设计方案：`cadence/designs/2026-06-12_技术方案_CodingWorkspace五角色Analyst路由状态机_v1.0.md`
- 当前已有未提交源码改动：
  - `src/product/coding_workspace_engine.rs`
  - `src/web/coding_ws_handler.rs`
  - `tests/it_web/web_coding_ws_handler.rs`
- 执行 P1 前必须先确认这些改动是继续保留、单独提交，还是合并到同一工作流中；不得回滚用户或前序 agent 已做的改动。

## P1：AnalystDecision 契约与持久化

**目标：** 新增结构化 Analyst 决策模型、解析兼容层、attempt store 持久化和后端测试。

**范围：**

- 新增 `AnalystDecisionRecord`、`AnalystDecisionVerdict`、`AnalystDecisionNextStage` 等模型。
- 让 Analyst provider 新旧 JSON 输出都能解析。
- 每次 `execute_rework_with_commands` 完成 Analyst 输出解析后保存一条 decision record。
- 继续保留现有 `AnalystVerdict` chat entry，不改变当前阶段路由。

**不做：**

- 不把 Testing passed/blocked 全部改成自动进入 Analyst。
- 不消费 `next_stage` 改变状态机。
- 不改前端页面展示。

**验证：**

- `cargo test --locked --test it_product analyst_decision`
- `cargo test --locked --test it_product execute_rework_persists_structured_analyst_decision`
- `cargo fmt --check`

**详细计划文档：**

- `cadence/plans/2026-06-12_计划文档_实施计划_CodingWorkspace五角色Analyst路由状态机_P1_AnalystDecision契约_v1.0.md`

## P2：Testing 后统一进入 Analyst

**目标：** Testing 完成后统一生成证据并进入 Analyst，由 Analyst 结构化 decision 决定下一阶段。

**依赖：** P1。

**范围：**

- 将 `testing_report_should_enter_analyst` 从局部布尔升级为“Testing 后是否需要 Analyst”的稳定规则。
- Testing passed、failed、blocked、passed_with_warnings 都进入 Analyst。
- Analyst `next_stage = coding` 时回 Coder。
- Analyst `next_stage = testing` 时允许重跑 Tester。
- Analyst `next_stage = code_review` 时进入 CodeReviewer。
- Analyst `next_stage = human_gate` 时创建 blocked gate。

**关键测试：**

- Tester passed -> Analyst -> CodeReview。
- Tester failed with evidence -> Analyst -> Coding。
- Tester blocked with `skipped_required_steps` -> Analyst -> Coding。
- Analyst 输出 `next_stage = testing` -> 重跑 Testing。
- Analyst 输出 `next_stage = human_gate` -> attempt 等待人工处理。

## P3：CodeReviewer / InternalReviewer 后统一回 Analyst

**目标：** CodeReviewer 和 InternalReviewer 只产证据，结束后统一由 Analyst 决策。

**依赖：** P1。

**范围：**

- CodeReviewer approve/request_changes/blocked 均转为证据输入给 Analyst。
- InternalReviewer approve/request_changes/blocked 均转为证据输入给 Analyst。
- 保持当前顺序：`CodeReview -> Analyst -> ReviewRequest -> InternalPrReview -> Analyst -> FinalConfirm`。
- 对 `ReviewRequest` 阶段只保留推送/创建审查请求职责，不让它承担质量判断。

**关键测试：**

- CodeReviewer approve -> Analyst -> ReviewRequest。
- CodeReviewer request_changes -> Analyst -> Coding。
- InternalReviewer approve -> Analyst -> FinalConfirm。
- InternalReviewer request_changes -> Analyst -> Coding 或 Testing。

## P4：Human Gate 与 manual_continue 质量豁免

**目标：** 将 `manual_continue` 固定为质量豁免动作，并让豁免记录进入后续 Analyst、CodeReviewer、InternalReviewer 上下文。

**依赖：** P1、P2，建议在 P3 后执行。

**范围：**

- `manual_continue` 必须填写原因。
- 写入 `quality-bypass-audits`。
- `manual_continue` 不再表示普通继续，也不硬编码进入 CodeReviewer。
- Human Gate action 与 Analyst `human_gate.available_actions` 对齐。
- 达到 `max_auto_rework` 后进入 Human Gate，而不是直接跳过 Coding 进入 CodeReview。

**关键测试：**

- `manual_continue` 空原因被拒绝。
- `manual_continue` 写入质量豁免审计。
- 后续 EvaluationContextPack 包含质量豁免。
- 达到 `max_auto_rework` 后进入 Human Gate。

## P5：前端展示与真实 E2E 验收

**目标：** 前端清楚区分证据节点和 Analyst 路由决策节点，并完成真实 E2E 验收。

**依赖：** P1-P4。

**范围：**

- TypeScript 类型补齐 Analyst decision。
- Timeline 展示 Analyst decision 的 verdict、next_stage、reason、evidence refs。
- TestingReport 旁展示“等待 Analyst 决策”或“Analyst 已决策”。
- Human Gate 展示 Analyst 推荐动作和 `manual_continue` 风险文案。
- 真实 E2E 覆盖当前 `skipped_required_steps` 场景。

**关键测试：**

- `web/src/api/types.test.ts`
- `web/src/state/coding-workspace-store.test.ts`
- `web/src/hooks/useCodingWorkspaceWs.test.tsx`
- `web/src/pages/CodingWorkspacePage.test.tsx`
- 真实浏览器 E2E：Tester blocked -> Analyst -> Coder -> Tester 重跑。

## 推荐执行顺序

1. 先处理当前 worktree 未提交 bugfix：确认保留并提交，或明确纳入当前分支变更。
2. 执行 P1，完成契约与持久化。
3. P1 通过定向测试和 `cargo fmt --check` 后，再写或展开 P2 详细计划。
4. P2-P4 每个阶段完成后做一次真实 E2E 快速检查，避免到 P5 才发现状态机偏差。
5. P5 只做前端展示和全链路验收，不再承载后端状态机大改。

## 实现条件判断

当前实现具备拆分实施条件：

- 现有 `CodingExecutionStage::Rework` 已能承载 Analyst 节点。
- 现有 `CodingReworkInstruction` 可继续承载 Coder 返修指令。
- 现有 attempt store 已有按目录存 JSON 的模式，可新增 `analyst-decisions/`。
- 现有 WebSocket 和测试夹具已经覆盖 CodingWorkspace 状态机。
- 当前最大风险是状态机改动范围大，因此必须按 P1-P5 顺序推进。
