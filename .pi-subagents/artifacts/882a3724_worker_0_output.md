## Completed

已完成 `/image-create` 移动端响应式适配并提交：

- 移动端会话列表改为左侧抽屉，支持遮罩/关闭按钮关闭，选择或创建会话后自动收起。
- 移动端生成参数默认折叠，生成按钮始终可见；`lg+` 参数完整展示。
- Header 移动端紧凑化，设置和返回入口保持可见。
- 聊天区移动端占主要视口高度，消息气泡及生成图片全宽展示。
- 交互控件补充 44px 最小触摸高度，输入控件移动端使用 16px 字号。
- 保留桌面端 `lg` 会话栏和 `xl` 参数侧栏布局。
- 报告已在提交后写入 `/tmp/mobile-responsive-report.md`。

提交 Hash：`fa794d35cc18b381e40009c73a40d865c782b55d`

## Files Changed

- `web/src/pages/ImageCreatePage.tsx` - 增加移动端会话抽屉状态、遮罩、紧凑 Header 和响应式主布局。
- `web/src/pages/ImageCreatePage.test.tsx` - 覆盖抽屉交互和桌面布局断点。
- `web/src/components/image-create/SessionList.tsx` - 支持抽屉关闭回调、会话选择后关闭及触摸友好尺寸。
- `web/src/components/image-create/ParamsPanel.tsx` - 增加移动端参数折叠，保持生成按钮可见。
- `web/src/components/image-create/ParamsPanel.test.tsx` - 覆盖默认折叠、展开及生成按钮尺寸。
- `web/src/components/image-create/ChatPane.tsx` - 调整移动端高度、消息宽度、输入字号和触摸尺寸。
- `web/src/components/image-create/ChatPane.test.tsx` - 覆盖移动端聊天布局样式。
- `web/src/components/image-create/CustomSelect.tsx` - 优化移动端字号及下拉选项触摸高度。
- `web/src/components/image-create/PromptBlock.tsx` - 提示词输入框移动端使用 16px 字号。
- `web/src/components/image-create/ReferenceImageUpload.tsx` - 增大移除参考图按钮触摸区域。

## Verification

- `git diff --check`：通过。
- `cd web && pnpm tsc -b`：通过。
- `cd web && pnpm test`：100 个测试文件、777 个测试全部通过。
- `cd web && pnpm build`：通过。
- 已确认暂存区为空。
- 已确认提交仅包含目标范围内的 10 个 Web 文件。

## Notes

- 尝试生成 375px 和 1280px 浏览器截图，但本机缺少 Playwright Chromium 可执行文件，未完成真实浏览器截图验证；关键断点通过组件测试和生产构建验证。
- 构建仍输出既有的 `>500 kB` chunk 警告。
- 工作区存在任务开始前已有的两个 `.pyc` 修改及 `.pi-subagents/` 未跟踪目录，均未暂存或纳入本次提交。