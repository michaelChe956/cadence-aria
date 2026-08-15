# Tasks: adopt-review-findings

## 1. 数据源与组件

- [ ] 1.1 前端取数：确认/接通"最新 review 报告文本"数据源（store 或 chat rebuild 产物），暴露给 ChatWorkspacePage
- [ ] 1.2 `ChatInputBar` 新增 `latestReviewReport` prop 与「采纳 Review 意见」按钮（覆盖式预填），配套组件测试（带入格式/无报告不渲染/重复带入一致）
- [ ] 1.3 `ChatWorkspacePage` props 传递与回归验证：`cd web && pnpm tsc -b && pnpm test`
