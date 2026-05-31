# Coding Workspace 完整闭环实施计划 — 总览

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-23
- 版本：v1.0
- 依据：
  - `cadence/designs/2026-05-23_技术方案_CodingWorkspace完整闭环_v1.0.md`
  - `cadence/prds/2026-05-23_概要需求_CodingWorkspace前端设计_v1.0.md`

---

## 一、实施范围

本计划覆盖技术方案 P0（打通最小真实闭环），并包含满足真实 E2E 所需的最小 Internal PR Review 与 blocked gate schema。目标是：

1. Work Item Plan confirmed 后出现"开始 Coding"入口
2. 创建独立 git worktree
3. Provider 驱动 coding
4. 后端真实测试
5. 基础 code review
6. commit + push branch-only ReviewRequest
7. 最小 Internal PR Review
8. Final Confirm
9. 使用 `naruto` 爬楼梯完成真实 E2E

---

## 二、已确认设计决策

| # | 决策 | 结论 |
|---|------|------|
| 1 | stage 与 status 关系 | stage 只表示执行阶段（prepare_context → final_confirm），终态由 status 表达 |
| 2 | session 概念 | attempt 本身即 session，无需 workspace_session_id |
| 3 | WebSocket endpoint | 独立 endpoint `/ws/coding-attempts/:attempt_id` |
| 4 | blocked gate 按钮 | 由后端 `coding_gate_required` 消息动态下发 `available_actions` |
| 5 | Review 结果恢复 | 基础 Code Review 使用 `CodeReviewReport` 持久化，Internal PR Review 使用 `InternalPrReview` 持久化 |

---

## 三、计划拆分

| 阶段 | 文件 | 内容 | 预估工作量 |
|------|------|------|-----------|
| P1 | `..._P1_后端数据模型与Store_v1.0.md` | 数据模型、Store CRUD、Git 服务 | 中 |
| P2 | `..._P2_后端Engine与WebSocket_v1.0.md` | CodingWorkspaceEngine、WS handler、REST API | 大 |
| P3 | `..._P3_前端CodingWorkspace_v1.0.md` | 路由、页面、组件、Store、Hook | 大 |

---

## 四、依赖关系

```
P1（数据模型与 Store）
  ↓
P2（Engine 与 WebSocket）← 依赖 P1 的模型和 Store
  ↓
P3（前端）← 依赖 P2 的 API 和 WS 消息协议
```

P1 是基础，必须先完成。P2 和 P3 在接口协议确定后可以部分并行（前端可以先用 mock 数据开发 UI）。

---

## 五、验收标准

最终验收以技术方案第 12.4 节定义的真实 E2E 用例为准：

- 仓库：`/home/michael/workspace/github/naruto`
- Work Item：爬楼梯（Python，O(n) 复杂度）
- 验收点：
  1. "开始 Coding" 入口可见
  2. 独立 worktree 创建成功
  3. Timeline 展示完整阶段
  4. 后端真实执行 Python 测试
  5. code review 通过后 commit + push
  6. 用户 final confirm 后 execution_status = completed

---

## 六、TDD 策略

每个阶段遵循：

1. 先写单元测试（模型序列化、状态转换、Store CRUD）
2. 再写实现
3. 集成测试覆盖跨模块交互
4. 前端测试覆盖组件渲染和交互

测试命令：
- 后端：`cargo fmt --check`、`cargo check --locked`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked -j 1`
- 前端：`pnpm test`（vitest）、`pnpm exec tsc --noEmit`、`pnpm run build`

---

## 七、风险与缓解

| 风险 | 缓解 |
|------|------|
| P2 Engine 复杂度高 | 先实现 happy path，rework 循环作为增强 |
| code review 只走临时 WS 消息导致刷新丢失 | P1 增加 `CodeReviewReport`，P2 snapshot 必须返回全部 review reports |
| Provider 集成不稳定 | P0 可用 fake provider 验证流程 |
| Git 操作在不同环境行为差异 | GitWorkspaceService 使用 argv，集成测试用临时 repo |
| 前端组件复用边界不清 | 严格按前端设计文档的共享边界表执行 |
