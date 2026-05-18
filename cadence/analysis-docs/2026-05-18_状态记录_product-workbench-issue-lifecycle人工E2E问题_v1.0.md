# product-workbench-issue-lifecycle 人工 E2E 问题记录

## 文档信息

- 文档类型：状态记录
- 日期：2026-05-18
- 版本：v1.0
- 分支：`product-workbench-issue-lifecycle`
- Worktree：`.worktrees/product-workbench-issue-lifecycle`
- 测试目标仓库：`/home/michael/workspace/github/naruto`

## 发现的问题

### 1. Workspace 已生成完整上下文后仍要求用户再次输入

现象：

- 用户从 Issue `爬楼梯问题` 创建 Story Workspace 后，页面已经展示完整的系统上下文 prompt。
- 上下文已经包含 Issue 描述、Repository 路径、OpenSpec 约束、输出 schema 和完成规则。
- 但 Workspace 不会自动开始 provider 执行，也没有明确的“开始生成”按钮。
- 用户必须在输入框再次输入一条消息并按 Enter，才会触发执行。

影响：

- 用户容易误以为需要重新描述需求。
- 输入框承担了“开始执行”和“补充要求”两个语义，交互意图不清晰。

建议：

- 在 `prepare_context` 阶段提供显式 `开始生成` 按钮。
- 或创建 Workspace 后自动触发首轮 provider 执行。
- 若仍保留输入框触发，应把 placeholder 和辅助文案改成“输入补充要求，或直接开始生成”。

### 2. Story Workspace 执行结果直接拼接 system/user，并跳到人工确认

现象：

- 用户输入 `开始执行` 后，页面展示的 assistant 内容为系统上下文和 `[user]: 开始执行` 的拼接文本。
- 状态直接进入 `人工确认`，出现 `确认通过` 按钮。
- 用户未观察到清晰的 `运行中` 和 `交叉审查` 过程。

影响：

- 如果使用 fake provider，这是当前 fake echo 行为导致的可解释现象，但不适合作为真实用户体验。
- 如果使用真实 provider，也应避免把内部 system prompt 作为候选 Story Spec 正文展示给用户确认。
- `交叉审查` 阶段在当前 UI 路径中不可感知，用户无法判断 reviewer 是否参与。

建议：

- fake provider 的 E2E 输出应改成符合 Story Spec schema 的最小 Markdown，而不是回显完整 prompt。
- 真实 provider 输出应只展示候选产物正文，系统 prompt 可保留在调试/执行面板。
- 若 review round 尚未真正执行，UI 不应暗示已完成交叉审查；若已跳过，应显示“未执行 review / fake provider 快速路径”。

### 3. confirmed Story Spec 无法从 UI 继续生成 Design Spec

现象：

- 后端 lifecycle API 返回 `story_spec_0001.confirmation_status=confirmed`。
- 用户回到工作台后能在 `Story Spec` 列看到卡片。
- 点击 Story Spec 卡片会直接进入对应 Workspace，而不是在工作台中选中该卡片。
- 因为 `生成 Design Spec` 按钮依赖 `selectedCard`，而 Story/Design/WorkItem 卡片点击路径会立即打开 Workspace，所以用户无法稳定触达 `生成 Design Spec` 动作。

影响：

- UI 主链路在 `Issue -> Story Spec` 后被卡住，无法继续 `Story Spec -> Design Spec -> Work Item`。
- 后端已满足解锁条件，但前端交互没有暴露下一步入口。

建议：

- 将卡片“选中”和“打开 Workspace”拆成两个明确动作。
- 或在 Story Spec 卡片内部直接展示 `生成 Design Spec` 操作。
- 或在 Workspace 完成确认后提供 `返回并生成 Design Spec` / `下一步` 动作。

### 4. Story Spec 只有确认状态，没有可见正文或版本

现象：

- 后端 lifecycle API 返回 `story_spec_0001.current_version=null`。
- `.aria/projects/project_0001/issues/issue_0001/` 下存在 `story-specs/story_spec_0001.json` 和 `workspace-sessions/workspace_session_0001.json`，但没有对应 `versions/` 正文文件。
- 前端 Story Spec 卡片只展示标题、ID、状态，无法看到 Story Spec 正文、需求条目、成功标准或测试要求。

影响：

- 用户无法判断自己确认的 Story Spec 内容是否真的是有效候选产物。
- 后续 Design Spec 缺少可审阅的上游 Story 正文。

建议：

- provider 完成时应把 candidate markdown 写入 `SpecVersionRecord`，更新 `current_version`。
- 前端卡片或详情区域应展示当前版本摘要，并能打开完整正文。
- `确认通过` 应确认某个具体版本，而不是只把 entity status 改为 `confirmed`。

## 当前测试处置

- 以上问题先记录为人工 E2E 发现，不在本轮直接修改代码。
- Story Workspace 已可按当前实现点击 `确认通过`，但 UI 主链路在生成 Design Spec 前被阻塞。
- 如需继续验证后端链路，可临时通过 API 创建 Design Spec / Work Item；这不能视为 UI 主链路通过。
