# CodingWorkspace Provider 驱动测试审查与恢复机制实施计划总览

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 将原单体计划拆成可在单个实现 session 内完成的阶段计划，避免一次性加载过多上下文。

**Architecture:** 每个阶段只覆盖一个相对独立的交付面，并在结束时形成可测试状态。执行顺序为后端基础 -> Provider 运行时与恢复 -> 前端展示 -> 真实 E2E 验收。

**Tech Stack:** Rust 1.95、serde/serde_json、tokio、Axum WebSocket、React 19、TypeScript、Zustand、Vitest、Cargo。

---

## 拆分评估

原单体计划：

- 文件：`cadence/plans/2026-06-10_计划文档_实施计划_CodingWorkspaceProvider驱动测试审查与恢复机制_v1.0.md`
- 规模：2526 行，82790 bytes，`wc` 词数 5791。
- 结论：单看体量低于 150k 上限，但覆盖后端模型、上下文、Provider runtime、blocked gate、WebSocket、前端 UI 与真实 E2E，实施跨度过大，不适合单个实现 session。

本次拆分后，单个阶段计划应只加载对应文件和少量邻近上下文；不要把四个阶段一次性塞给同一个实现 session。

## 阶段计划

1. `cadence/plans/2026-06-10_计划文档_实施计划_CodingWorkspaceProvider驱动测试审查与恢复机制_P1_后端基础_v1.0.md`
   - 范围：模型、EvaluationContextPack、store 持久化、TestPlan parser/report builder。
   - 结束状态：后端基础数据结构和纯函数测试通过，但不改完整工作流。

2. `cadence/plans/2026-06-10_计划文档_实施计划_CodingWorkspaceProvider驱动测试审查与恢复机制_P2_Provider运行时与恢复闭环_v1.0.md`
   - 范围：Tester 两段式、Review raw output、Analyst/Internal Reviewer 契约、blocked gate 后端恢复。
   - 结束状态：后端工作流能创建可恢复 blocked gate，且 prompt 全部显式使用 OpenSpec/Superpowers。

3. `cadence/plans/2026-06-10_计划文档_实施计划_CodingWorkspaceProvider驱动测试审查与恢复机制_P3_前端展示与交互_v1.0.md`
   - 范围：TypeScript 类型、Zustand store、WebSocket hook、Testing/Gate UI。
   - 结束状态：前端能展示 TestPlan、step evidence、missing required steps、blocked gate metadata，并发送恢复动作。

4. `cadence/plans/2026-06-10_计划文档_实施计划_CodingWorkspaceProvider驱动测试审查与恢复机制_P4_真实E2E验收_v1.0.md`
   - 范围：全量验证命令、真实 Coding Workspace attempt、验收记录。
   - 结束状态：真实场景证明 provider-driven testing/review/recovery 闭环可用。

## 强制边界

- Aria 不写死任何语言、框架、包管理器、测试命令或安全工具。
- Tester、Analyst、Code Reviewer、Internal Reviewer 都必须使用 OpenSpec 与 Superpowers 契约。
- Story Spec、Design Spec、Work Item 都进入 Tester 和 Reviewer 上下文，但通过 `EvaluationContextPack` 做角色裁剪。
- Tester 必须 `plan_tests` -> `execute_test_plan`，required step 未执行不能 passed。
- `request_changes` 进入 Analyst 返修；`blocked` 创建可恢复 gate；两者不能混用。
- Raw provider output 必须落盘。
- Rust 验证使用宿主机 cargo，禁止 `cargo test -j 1`。

## 执行方式

推荐按 P1 -> P2 -> P3 -> P4 顺序执行，每个 P 作为独立 implementation session。每个阶段完成后先提交，再进入下一阶段。

如果用 subagent-driven development：每个阶段计划内部按 Task 派发 subagent；不要把所有阶段交给同一个 subagent。
