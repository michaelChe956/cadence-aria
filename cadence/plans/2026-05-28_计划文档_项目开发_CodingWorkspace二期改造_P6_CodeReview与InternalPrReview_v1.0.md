# CodingWorkspace 二期 P6：CodeReview + InternalPrReview

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-28
- 版本：v1.0
- 前置：P4（Rework 分析官路由到 CodeReview）、P5（前端展示层）
- 产出：CodeReviewer Provider + InternalReviewer Provider + Review 结果展示
- 设计文档：`cadence/designs/2026-05-28_技术方案_CodingWorkspace二期改造_v1.0.md` §3.1
- 设计评审：`cadence/designs-reviews/2026-05-28_设计评审_CodingWorkspace二期改造_v1.0.md`

---

## 一、目标

实现两个独立的 Review Provider：

1. **CodeReviewer**：只分析变更 diff，输出 findings（代码质量、安全、性能）
2. **InternalReviewer**：在 ReviewRequest(push) 之后分析功能影响，输出影响范围、PR description、commit message 建议
3. Review 后进入 Rework 分析官判定是否通过

---

## 二、任务清单

### 2.1 CodeReviewer Provider（src/product/coding_workspace_engine.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 1.1 | 重构 `execute_code_review`：调用 CodeReviewer Provider（单次调用，非 Agent Loop） | 单元测试 | Provider 被正确调用 |
| 1.2 | 构建 CodeReviewer prompt：包含 diff 内容、原始需求摘要、代码规范 | 单元测试 | prompt 包含 diff |
| 1.3 | 解析 CodeReviewer 输出为 `CodeReviewReport`（findings + verdict） | 单元测试 | report 结构正确 |
| 1.4 | Review 完成后推送 CodingChatEntry + CodeReviewComplete | 单元测试 | 前端收到结果 |

### 2.2 InternalReviewer Provider（src/product/coding_workspace_engine.rs）

> 修订：InternalPrReview 以现有稳定链路为准，发生在 `ReviewRequest` 之后。其 prompt 必须包含 commit、review request、diff 和功能需求上下文。

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 2.1 | 实现 `execute_internal_pr_review`：在 ReviewRequest 之后调用 InternalReviewer Provider | 单元测试 | Provider 被正确调用 |
| 2.2 | 构建 InternalReviewer prompt：包含 commit、review request、完整变更、功能需求、影响范围分析要求 | 单元测试 | prompt 包含功能上下文 |
| 2.3 | 解析输出为 `InternalPrReview`（影响范围 + PR description + commit message） | 单元测试 | 结构正确 |
| 2.4 | Review 完成后推送 CodingChatEntry + InternalPrReviewComplete | 单元测试 | 前端收到结果 |

### 2.3 Review 后路由

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | CodeReview 完成 → 进入 Rework(Analyst) 判定 | 单元测试 | 正确路由 |
| 3.2 | Rework 判定 NoIssue → 进入 ReviewRequest | 单元测试 | 正确路由 |
| 3.3 | ReviewRequest 成功后 → 进入 InternalPrReview | 单元测试 | 正确路由 |
| 3.4 | InternalPrReview 完成 → 进入 Rework(Analyst) 最终判定 | 单元测试 | 正确路由 |
| 3.5 | 最终 Rework 判定 NoIssue → attempt 完成 | 单元测试 | 正确结束 |
| 3.6 | 任何 Rework 判定 NeedsFix → 回到 Coding | 单元测试 | 正确回退 |

### 2.4 前端 Review 结果展示

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 4.1 | CodeReview findings 展示：severity 颜色 + 文件位置 + 描述 | — | 列表清晰 |
| 4.2 | InternalPrReview 展示：影响范围列表 + PR description 预览 | — | 信息完整 |

---

## 三、验收标准

1. `cargo test` 全部通过
2. 手动测试：Rework NoIssue → CodeReview 执行 → 输出 findings 列表
3. 手动测试：CodeReview 后 Rework NoIssue → ReviewRequest 成功 → InternalPrReview 执行 → 输出影响范围
4. 手动测试：InternalPrReview 后 Rework NoIssue → attempt 完成
5. 手动测试：CodeReview 后 Rework NeedsFix → 回到 Coding

---

## 四、不做的事

- PR 实际创建（ReviewRequest 阶段已有实现）
- E2E 全流程测试（P7）
