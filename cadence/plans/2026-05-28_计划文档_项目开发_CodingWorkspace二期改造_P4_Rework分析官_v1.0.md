# CodingWorkspace 二期 P4：Rework 分析官

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-28
- 版本：v1.0
- 前置：P1（数据模型）、P3（Test Agent Loop，提供 TestingReport 输入）
- 产出：Analyst Provider + AnalystVerdict 解析 + 路由决策 + ContextNote 注入
- 设计文档：`cadence/designs/2026-05-28_技术方案_CodingWorkspace二期改造_v1.0.md` §2.3, §2.4

---

## 一、目标

将 Rework 阶段从"修改代码"改为"分析官"角色：

1. Analyst Provider 只做分析和路由决策，不修改代码，不调用 tool_use
2. 输入：上一阶段 summary + 本轮新增 ContextNote
3. 输出：结构化 AnalystVerdict（NeedsFix / NeedsHumanInput / NoIssue）
4. 根据 verdict 自动路由到下一阶段
5. Coding 重写次数限制（max_rewrite = 3）

---

## 二、任务清单

### 2.1 Analyst Provider 实现（src/product/coding_workspace_engine.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 1.1 | 重构 `execute_rework` 方法：调用 Analyst Provider（非 Agent Loop，单次调用） | 单元测试 | Provider 被正确调用 |
| 1.2 | 构建 Analyst system prompt：包含角色说明、输出格式要求（JSON AnalystVerdict） | 单元测试 | prompt 明确要求结构化输出 |
| 1.3 | 构建 Analyst user prompt：上一阶段 summary + 未消费的 ContextNote | 单元测试 | 正确拼接输入 |

### 2.2 AnalystVerdict 解析（src/product/coding_workspace_engine.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 2.1 | 从 Analyst 响应中解析 JSON → AnalystVerdict | 单元测试 | 三种 verdict 均可解析 |
| 2.2 | 解析失败 fallback：视为 NeedsHumanInput（保守策略） | 单元测试 | 格式错误不崩溃 |
| 2.3 | 解析成功后推送 `CodingChatEntryCreated`（AnalystVerdict 类型） | 单元测试 | 前端收到判定结果 |

### 2.3 路由决策执行

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | NeedsFix → 检查 rewrite_count < max_rewrite → 回到 Coding 阶段 | 单元测试 | 正确路由 |
| 3.2 | NeedsFix + rewrite_count >= max_rewrite → 跳过 Coding → 进入 CodeReview（带 warning） | 单元测试 | 上限后跳过 |
| 3.3 | NeedsHumanInput → 设置 attempt 状态为 WaitingForHuman → 暂停 | 单元测试 | 正确暂停 |
| 3.4 | NoIssue（Testing 后）→ 进入 CodeReview | 单元测试 | 正确路由 |
| 3.5 | NoIssue（CodeReview 后）→ 进入 InternalPrReview | 单元测试 | 正确路由 |

### 2.4 ContextNote 注入逻辑

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 4.1 | 查询未消费的 ContextNote（`consumed_by_rework_round IS NULL`） | 单元测试 | 只返回未消费的 |
| 4.2 | 注入后标记 `consumed_by_rework_round = current_round` | 单元测试 | 标记正确更新 |
| 4.3 | 注入总量限制：超过 10000 字符截断最早的 | 单元测试 | 截断逻辑正确 |

---

## 三、验收标准

1. `cargo test` 全部通过
2. 手动测试：Testing 完成（有 bug）→ Rework 分析官判定 NeedsFix → 自动回到 Coding
3. 手动测试：Testing 完成（无 bug）→ Rework 分析官判定 NoIssue → 进入 CodeReview
4. 手动测试：连续 3 次 Coding → Rework 判定 NeedsFix → 第 4 次直接跳到 CodeReview + warning
5. 手动测试：用户输入 ContextNote → Rework 时 Analyst 收到该 ContextNote → 消费后不再重复注入

---

## 四、不做的事

- CodeReview / InternalPrReview 的实际执行（P6）
- 前端 AnalystVerdictEntry 组件（P5）
- NeedsHumanInput 恢复后的重新触发 Rework（P7 集成测试覆盖）
