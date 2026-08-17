# Proposal: spec-design-dialog-revision

## Why

Story spec / design spec 的生成流程目前是"author 产出 → 用户只有「重新编写（推倒重来）/进入 Review」两个出口"，自由文本反馈被推迟到流程尾部的 human_confirm 阶段；且只要配置了 reviewer，确认后就强制进入 CrossReview，用户无法跳过。这与业界主流（Kiro、Claude Code plan mode、GitHub Spec Kit）的"停等-对话-增量修订 + 可选只读 review"模式相悖，用户迭代成本高、上下文丢失、review 定位错位（门禁 vs 辅助）。

## What Changes

- **AuthorConfirm 对话式修订循环**：Story/Design workspace 的 AuthorConfirm 阶段开放自由文本反馈，author 基于当前稿 + 反馈做增量修订（保留版本），修订完成后回到 AuthorConfirm，可无限循环直到用户满意；每轮修订要求 author 输出「改动摘要」随对话流展示。
- **移除推倒重来**：AuthorConfirm 不再提供「重新编写」（清空 artifact 回 PrepareContext）出口；"全部重写"意图通过反馈文本表达，由修订通道实现。`AuthorDecision::Reject` 消息保留协议兼容，但 Story/Design 流程收到时返回引导性错误。
- **确认拆分与 review 开关**：`AuthorDecision::Accept` 拆分为 `AcceptWithReview` / `AcceptFinalize` 两个决策；创建 workspace 时的 `reviewer_enabled` 配置语义从"开关"变为"默认值"（决定前端默认高亮哪个确认按钮），两个按钮始终可点，用户每次确认时可临时偏离。
- **单确认点收尾**：review 完成后不再进入 ReviewDecision 阶段驱动状态机，reviewer 报告作为消息进入对话流，流程回到 AuthorConfirm，由用户决定继续修订或点「确认定稿」；「确认定稿」直接进入 Completed（人工拍板即终态）。HumanConfirm 与 ReviewDecision 阶段从 Story/Design 流程退役（枚举保留，WorkItem/WorkItemPlan 类型不受影响）。
- **存量会话兼容**：interrupted_run_recovery 中停留在 HumanConfirm/ReviewDecision 的 Story/Design 存量会话，恢复时引导走新流程完成或安全关闭。

## Capabilities

### New Capabilities

- `spec-design-dialog-revision`: Story/Design workspace 的对话式 spec 修订循环与可选 review 确认流程（AuthorConfirm 反馈修订、确认双出口、review 报告回对话流、单点定稿、存量会话兼容）。

### Modified Capabilities

（无——现有 specs 目录无 Story/Design workspace 流程的既有 capability，本次为新建。）

## Impact

- **受影响代码**：
  - 引擎：`src/product/workspace_engine/decisions.rs`（新增 Revise/AcceptWithReview/AcceptFinalize 分支、provisional reviewer 恢复）、`review/routing.rs`（review 后路由回 AuthorConfirm）、`prompts/`（新增 author 反馈修订 prompt，含改动摘要要求）、`lifecycle.rs`（存量会话恢复 fallback 与 provisional 保留）
  - 协议：`src/web/workspace_ws_types/in_.rs`（AuthorDecision 变体）、`common.rs`、`workspace_ws_handler/protocol.rs`（阶段守卫）、`run/provider_run.rs`（修订 run 接线，⚠与 add-monorepo 分支重叠，走新增 match 臂规避）
  - 前端：`web/src/components/chat-workspace/ChatInputBar.tsx`（输入框 + 三动作 + 默认高亮）、`ChatWorkspacePage.tsx`、修订摘要/review 报告消息渲染
- **非目标**：daemon 全自动链（`run_planning_full_chain`）不改；逐行 diff 视图不做；结构化反馈标签（feedback_types）不做；WorkItem/WorkItemPlan 类型流程不动；Kiro 式下游级联同步（story→design 联动）留作后续。
- **合并顺序约定**：本 change 建议先于 `feat-b-0808-add-monorepo` 合入 main；新增逻辑优先放新函数/新文件（独立 prompt 构建函数、新测试文件），最小化与该分支的文本重叠（重叠文件为 `types.rs`/`lifecycle.rs`/`prompts/revision.rs`/`run/provider_run.rs` 四处，详见 design §7）。
