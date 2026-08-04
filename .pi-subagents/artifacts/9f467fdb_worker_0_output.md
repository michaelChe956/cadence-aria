## Completed

已完成图片创作页面自定义下拉组件改造并提交。

- 状态：完成
- 提交：`56eb7742e60870345abbded03aec185b0c535f96`
- 提交信息：`style(image-create): 自定义下拉组件替换原生 select（美化选项列表）`
- 测试摘要：TypeScript 检查通过；100 个测试文件、773 个测试全部通过；生产构建通过
- 完整报告：`/tmp/custom-select-report.md`

实现内容：

- 新增 Claymorphism 风格 `CustomSelect`
- 自定义圆角弹出列表、双层阴影、滚动区域、hover/选中样式及 Check 图标
- 支持点击选择、外部关闭、Escape 关闭、disabled 和 opacity/scale 动画
- 保留 `combobox`、`listbox`、`option` 等无障碍语义
- 替换 `ParamsPanel`、`SessionList`、`SettingsDialog` 中全部原生 `<select>`
- 未新增依赖，未修改下拉之外的业务逻辑

## Files Changed

- `web/src/components/image-create/CustomSelect.tsx` - 新增自定义下拉组件
- `web/src/components/image-create/CustomSelect.test.tsx` - 新增组件交互、关闭和禁用态测试
- `web/src/components/image-create/ParamsPanel.tsx` - 参数下拉替换为 `CustomSelect`
- `web/src/components/image-create/ParamsPanel.test.tsx` - 更新自定义 listbox 交互测试
- `web/src/components/image-create/SessionList.tsx` - 模板和 Provider 下拉替换
- `web/src/components/image-create/SessionList.test.tsx` - 更新创建会话下拉交互测试
- `web/src/components/image-create/SettingsDialog.tsx` - 默认参数下拉替换
- `web/src/components/image-create/SettingsDialog.test.tsx` - 更新设置下拉交互测试

## Verification

- `cd web && pnpm exec vitest --run src/components/image-create/CustomSelect.test.tsx`
  - 3/3 测试通过
- `cd web && pnpm exec vitest --run src/components/image-create/CustomSelect.test.tsx src/components/image-create/ParamsPanel.test.tsx src/components/image-create/SessionList.test.tsx src/components/image-create/SettingsDialog.test.tsx`
  - 4 个测试文件、16 个测试全部通过
- `cd web && pnpm tsc -b`
  - 通过
- `cd web && pnpm test`
  - 100 个测试文件、773 个测试全部通过
- `cd web && pnpm build`
  - 通过
- `git diff --cached --quiet`
  - 通过，当前无 staged 文件

## Notes

- 构建仍有既有的“大于 500 kB chunk”警告，不影响构建成功，也不属于本次功能改动。
- 工作树中保留了任务开始前已有的 `.codex/.../__pycache__` 修改和 `.pi-subagents/` 未跟踪目录；它们未被纳入提交。