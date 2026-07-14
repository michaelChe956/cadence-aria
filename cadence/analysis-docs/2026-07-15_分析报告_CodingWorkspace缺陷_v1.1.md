# Coding Workspace 缺陷记录

## 文档信息

- 首次记录日期：2026-07-14
- 本次更新日期：2026-07-15
- 版本：v1.1
- 类型：分析报告
- 适用对象：Coding Workspace、Code Reviewer、Group Final Review、完成门禁
- 当前案例 Coding Attempt：`coding_attempt_0001`

## 问题一：Code Reviewer 证据包截断全量测试输出

Code Reviewer 会出现非阻塞 observation：“全量测试输出在证据包中被截断”。

Coder 虽然执行了全量测试，但 Reviewer 获得的证据包没有保留完整输出，无法完整核对命令末尾的测试统计、失败数和最终退出结果。当前 Review 仍可能通过，但会产生该非阻塞 observation。

该问题属于 Coding Workspace 向 Code Reviewer 传递测试证据不完整，不属于 Work Item Workspace 生成缺陷。

## 问题二：Work Item 有测试证据但没有 TestingReport

### 现象

`coding_attempt_0001` 修复前共有 10 个 completed Work Item。每个 Unit 的 `work-item-handoff.json` 都包含：

- `tests_run`
- `test_result_summary`

各 Work Item 的 Code Review 已完成，Group Final Review 也已返回 `approve`，但 Attempt 下没有任何独立的：

```text
testing-reports/testing_report_*.json
```

因此这不是“Coder 没有执行或报告测试”，而是“已有 handoff 测试证据没有被转换或持久化成完成门禁要求的 TestingReport”。

### 触发条件

2026-07-06 的 Coding Workspace 精简改造已经从主执行编排中摘除 Testing 阶段，但最终完成门禁仍按旧模型执行以下要求：

- Work Item 有 Verification Plan required gates 时，必须存在 Passed 或 PassedWithWarnings 的 TestingReport。
- WorkItemGroup 中的 TestingReport 还必须通过 `plan_id` 对应到各 Work Item 的 Verification Plan。

当前执行链不会生成该产物，完成链却仍把它作为必需输入，形成证据模型不一致。

### 影响

- Coder、单项 Code Review 和 Group Final Review 都可能已经完成并通过。
- Group Final Review approve 后，Attempt 仍可能在最终确定性门禁中触发 `verification_gate_result_missing`。
- 页面表现为“全部工作已完成，但 Attempt 无法最终完成”。
- 为恢复历史数据，只能临时根据 Unit handoff 重建 TestingReport；这不应成为正常产品流程。
- 如果恢复时未明确区分“历史证据重建”和“后端重新执行测试”，可能造成证据可信度被误解。

### 当前案例的数据恢复方式

本次仅为恢复 `coding_attempt_0001`：

- 根据 10 个 Unit handoff 的 `tests_run` 和 `test_result_summary` 重建了 10 份 TestingReport。
- 每份报告通过 `plan_id` 关联相应 Verification Plan。
- 全部明确设置 `backend_verified=false`。
- 没有重新执行测试，也没有把历史 handoff 证据描述成后端实时执行结果。

### 缺陷归属

该问题属于 Coding Workspace 的执行阶段、证据持久化和最终门禁契约不一致，不属于 Work Item Workspace 的生成缺陷。

### 后续待讨论事项

后续讨论永久方案时需要明确：

1. 摘除 Testing 阶段后，Code Reviewer 的结构化 verification assessment 是否应成为 required gates 的权威证据。
2. Unit handoff 是否直接作为完成门禁证据，还是应在 Unit 完成时生成单独的 completion attestation。
3. TestingReport 是继续保留为兼容产物，还是只用于历史 Testing/Tester 流程。
4. 历史 Attempt 缺少 TestingReport 时应如何迁移，不能依赖人工逐个补 JSON。
5. 页面应如何区分后端实际执行的测试证据与根据 Coder/handoff 重建的历史证据。
6. 永久方案不能重新限制 Coder 和 Reviewer 只能执行 Verification Plan 已列测试；普通单元测试、非浏览器集成测试、编译、构建、类型检查、静态分析、格式和 lint 仍应允许。
7. Code Reviewer 与 Group Final Review 继续禁止生成 E2E、Playwright 或浏览器自动化测试相关 findings。

当前只记录问题与讨论边界，不在本文中确定最终实现。

相关初步方案记录：

- `cadence/plans/2026-07-15_计划文档_CodingWorkspace最终门禁一致性修复_v1.0.md`
