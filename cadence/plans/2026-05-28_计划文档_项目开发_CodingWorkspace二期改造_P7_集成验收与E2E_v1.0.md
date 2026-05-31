# CodingWorkspace 二期 P7：集成验收与 E2E

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-28
- 版本：v1.0
- 前置：P0-P6 全部完成
- 产出：全流程 E2E 测试 + 边界场景覆盖 + 回归验证
- 设计文档：`cadence/designs/2026-05-28_技术方案_CodingWorkspace二期改造_v1.0.md` §7
- 设计评审：`cadence/designs-reviews/2026-05-28_设计评审_CodingWorkspace二期改造_v1.0.md`

---

## 一、目标

验证二期改造的完整流程和边界场景：

1. Happy path 全流程串联
2. Rework 循环场景
3. Stage Gate 交互场景
4. 错误恢复场景
5. WebSocket 重连场景

---

## 二、任务清单

### 2.1 Happy Path E2E

| # | 任务 | 验收 |
|---|------|------|
| 1.1 | 全流程测试：Coding → Gate → Testing → Gate → Rework(NoIssue) → Gate → CodeReview → Gate → Rework(NoIssue) → ReviewRequest → Gate → InternalPrReview → Gate → Rework(NoIssue) → 完成 | attempt 状态为 Completed |
| 1.2 | 验证每个阶段的 CodingChatEntry 正确生成 | chatEntries 数量和类型正确 |
| 1.3 | 验证 Timeline 节点状态正确更新 | 所有节点为 completed |

### 2.2 Rework 循环场景

| # | 任务 | 验收 |
|---|------|------|
| 2.1 | Testing 发现 bug → Rework NeedsFix → Coding → Testing → Rework NoIssue → CodeReview | 循环正确执行 |
| 2.2 | 连续 3 次 NeedsFix → 第 4 次直接跳到 CodeReview + warning | rewrite_count 正确，warning 可见 |
| 2.3 | NeedsHumanInput → 暂停 → 用户输入 ContextNote → 恢复 → 重新 Rework | 暂停恢复正确 |

### 2.3 Stage Gate 交互场景

| # | 任务 | 验收 |
|---|------|------|
| 3.1 | Gate 超时自动确认（5s 无操作） | 自动进入下一阶段 |
| 3.2 | Gate 期间点击"立即开始" | 立即进入下一阶段 |
| 3.3 | Gate 期间切换 Provider → 倒计时重置 | 新 Provider 生效 |
| 3.4 | Gate 期间中止 attempt | attempt 状态为 Aborted |

### 2.4 Provider 切换场景

| # | 任务 | 验收 |
|---|------|------|
| 4.1 | 非 Gate 期间切换非当前阶段 Provider | 配置更新成功 |
| 4.2 | 尝试切换当前阶段 Provider | 收到错误提示 |

### 2.5 错误恢复场景

| # | 任务 | 验收 |
|---|------|------|
| 5.1 | Provider 连接失败 → 重试 2 次 → 暂停在 Gate | 用户可切换 Provider 重试 |
| 5.2 | Tester 约束违反 3 次 → 终止 Agent Loop | warning 可见，进入 Rework |
| 5.3 | Tester 超时（5 分钟） | 输出已完成结果 + timeout warning |

### 2.6 WebSocket 重连场景

| # | 任务 | 验收 |
|---|------|------|
| 6.1 | 断线重连后收到完整 chatEntries 历史 | 消息不丢失 |
| 6.2 | streaming 中断线 → 重连后继续推送 | 内容连续 |
| 6.3 | Gate 倒计时中断线 → 重连后重新计算剩余时间 | 倒计时正确 |

---

## 三、验收标准

1. 所有 E2E 测试用例通过
2. 无回归：现有 ChatWorkspace / SpecWorkspace 功能不受影响
3. `cargo test` + `pnpm build` 全部通过
4. 手动全流程走通至少一次

---

## 四、不做的事

- 性能优化（后续迭代）
- 并发 attempt 支持（设计文档开放问题 #4）
