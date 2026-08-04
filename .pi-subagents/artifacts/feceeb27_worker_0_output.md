## Completed

状态：已完成并提交 `/image-create` 页面 Claymorphism 视觉升级。

- 提交：`799b8a96bf4a94f907e59a2d48bea8789b3abc2d`
- 提交信息：`style(image-create): 融合 Claymorphism 视觉升级（柔和阴影/圆润/微交互/Lucide 图标）`
- 测试摘要：TypeScript 检查通过，99 个测试文件、770 个测试全部通过，生产构建成功。
- 完整报告：`/tmp/image-create-visual-upgrade-report.md`

主要完成内容：

- 使用 Aria 现有青蓝色令牌和系统字体，未引入新依赖。
- 为页面增加专业化 hero 标题区。
- 统一卡片圆角、柔和双层阴影、区域背景层次和 200ms 微交互。
- 使用 Lucide 图标升级空状态、按钮、上传区、进度态和图片下载区。
- 输入控件统一可见 focus ring、圆角和柔和内阴影。
- 动画使用 `motion-safe:`，尊重 `prefers-reduced-motion`。
- 未修改 props、store action、状态管理、API 调用或业务事件逻辑。

## Files Changed

- `web/src/pages/ImageCreatePage.tsx` - 增加 hero 标题区，升级整体布局、导航按钮和页面层次。
- `web/src/components/image-create/SessionList.tsx` - 升级会话卡片、空状态、创建弹窗和会话项交互。
- `web/src/components/image-create/ChatPane.tsx` - 升级聊天气泡、空状态、忙碌态、输入区和生成图片卡片。
- `web/src/components/image-create/PromptBlock.tsx` - 升级提示词卡片、标题图标和编辑输入区。
- `web/src/components/image-create/ParamsPanel.tsx` - 升级参数区、下拉框、进度状态和生成按钮。
- `web/src/components/image-create/ReferenceImageUpload.tsx` - 升级上传空状态、图片预览和移除操作。
- `web/src/components/image-create/SettingsDialog.tsx` - 升级设置弹窗、输入区域、状态提示和操作按钮。

## Verification

- `cd web && pnpm tsc -b`
  - 通过，无 TypeScript 错误。
- `cd web && pnpm test`
  - 首轮发现既有测试依赖可访问标题“会话”；恢复该可访问文案后重新验证。
  - 最终结果：99 个测试文件、770 个测试全部通过。
- `cd web && pnpm build`
  - 通过，Vite 生产构建成功。
  - 构建仍有仓库既存的主 chunk 超过 500 kB 警告。
- `git diff --check`
  - 通过，无空白错误。
- `git diff --cached --name-only`
  - 无输出，当前没有 staged 文件。

## Notes

- 未执行真实浏览器截图或多视口人工视觉检查，建议后续检查长会话、长提示词、大图和窄屏场景。
- 工作树中存在任务开始前已有、未纳入提交的变更：
  - `.codex/skills/ui-ux-pro-max/scripts/__pycache__/core.cpython-314.pyc`
  - `.codex/skills/ui-ux-pro-max/scripts/__pycache__/design_system.cpython-314.pyc`
  - `.pi-subagents/`
- 未修改或清理上述无关文件。