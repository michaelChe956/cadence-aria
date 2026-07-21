# Coding Workspace 缺陷记录

## 文档信息

- 首次记录日期：2026-07-14
- 本次更新日期：2026-07-15
- 版本：v1.2
- 类型：分析报告
- 适用对象：Coding Workspace、Coding Attempt 创建、Code Reviewer、Group Final Review、完成门禁
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

## 问题三：同一 Issue 并发创建 Coding Attempt 的竞态风险

### 现象

产品规则要求同一个 Issue 同一时间只能存在一个活跃的 Coding Workspace。正常顺序操作时，后续创建请求会被 active Attempt 或 Issue Shared Worktree lock 阻止。

当前创建流程仍存在一个很短的并发窗口：两个针对同一 Issue 的创建请求可能同时读取到“当前没有活跃 Attempt”，并继续分配 Coding Attempt ID 和写入状态文件。典型情况包括：

- 创建整个 Work Item Group 的 Group Attempt，同时为该 Group 的第一个 Work Item 创建普通 Attempt。
- 同一 Issue 下两个不同 Work Item 的普通 Attempt 同时创建。
- 同一 Work Item 因重复点击、浏览器重试或多个标签页而并发创建。

### 根因

- Group Attempt 创建只使用 `work_item_group:{project_id}:{issue_id}:{plan_id}` 维度的进程内锁，普通 Attempt 创建没有获取同一个 Issue 级创建锁。
- Issue Shared Worktree lock 采用“读取 JSON、检查状态、写回 JSON”的方式更新，不是覆盖整个创建流程的原子互斥区。
- Group Attempt 与首个 Work Item 使用相同的 `work_item_id` 获取 worktree lock；相同 ID 会被视为当前调用可重入，因此不能阻止这两条创建路径并发继续。
- Coding Attempt 序号文件按 Issue 保存，但 ID 分配同样是无共享互斥的读取、递增和写回。并发请求可能获得相同的下一个 Attempt ID。

### 影响

若两个请求获得相同 Attempt ID，后写入的 Attempt 可能覆盖先写入的记录。Group 创建流程随后创建 Coding Unit 时，如果读到被普通 Attempt 覆盖的记录，会因 scope 不匹配执行回滚并删除该 Attempt。

可能表现为：

- 创建接口已经返回成功，但前端随后加载 Attempt 时提示记录不存在。
- Attempt、Coding Unit 或 Provider 配置快照之间出现孤立或不一致状态。
- Issue Shared Worktree 仍显示被占用，用户无法正常重新开始、恢复或终止 Coding Workspace。
- 需要人工清理本地 Cadence 状态后重新创建 Attempt。

该问题影响的是 Cadence 本地 Coding Workspace/Attempt 状态，不会直接删除或覆盖目标仓库中的业务代码。

### 影响范围

- 风险边界为同一个 `project_id` 和 `issue_id` 下的并发创建。
- 不同 Issue 使用不同的 Attempt 目录、序号文件和 Issue Shared Worktree 状态，因此不会因该问题互相覆盖。
- 正常顺序创建时，现有 active Attempt 与 worktree lock 检查仍然有效。

### 当前风险判断

当前产品是本地单用户系统，没有用户或租户并发模型，主要由用户人工顺序操作：

- 触发概率较低，需要两个创建请求在很短的窗口内重叠。
- 单个用户仍可能通过重复点击、网络重试、多个浏览器标签页或未来的自动重试触发。
- 发生后主要损坏本机某次 Attempt 状态，通常可通过清理状态并重新开始恢复。
- 综合判断为低概率、中等影响的已知风险，当前不作为 `feat-b-0709` 合入 `main` 的阻塞项。

### 后续修复时机与方向

出现以下任一条件前应完成修复：

- 支持多人或多进程同时操作同一项目。
- 引入后台调度、自动重试或无人值守创建 Coding Attempt。
- Coding Attempt 状态开始承载不可接受人工清理或重建的重要数据。

建议后续使用 `(project_id, issue_id)` 级统一创建互斥，将唯一性检查、Attempt ID 分配、Attempt 与 Provider 配置写入、Group Coding Unit 初始化放入同一个原子边界或可恢复 Journal，并补充 Group/普通 Attempt 交叉并发及普通 Attempt 并发创建的回归测试。

本次仅记录风险、适用边界和后续修复条件，不修改现有实现。
