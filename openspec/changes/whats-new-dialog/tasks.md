## 1. 数据源与去重逻辑

- [ ] 1.1 新增 `web/src/whats-new/changelog.ts`：定义 `ChangelogEntry` 接口，导出 `CURRENT_VERSION`（"0.0.5"）与 `CHANGELOG` 数组（首条为 0.0.5 的中文要点）
- [ ] 1.2 新增 `web/src/whats-new/useWhatsNew.ts`：封装读/写 localStorage（key `aria-whats-new-seen`）与"是否应弹"判定，localStorage 不可用时静默降级；附单测 `useWhatsNew.test.ts`

## 2. 弹窗组件

- [ ] 2.1 新增 `web/src/components/whats-new/WhatsNewDialog.tsx`：受控模态弹窗，展示版本标题、日期、要点列表与"知道了"按钮，视觉参考现有 Dialog
- [ ] 2.2 新增 `WhatsNewDialog.test.tsx`：vitest + testing-library 测渲染、要点展示、关闭回调

## 3. 接入工作台

- [ ] 3.1 修改 `web/src/app-shell.tsx`：挂载时经 `useWhatsNew` 判定是否弹窗并渲染 `WhatsNewDialog`，关闭时标记已读
- [ ] 3.2 补充 `app-shell` 相关测试覆盖"未读弹窗/已读不弹/关闭写入"路径

## 4. 验证

- [ ] 4.1 运行 `pnpm test` 与类型检查，确认前端测试与构建通过
