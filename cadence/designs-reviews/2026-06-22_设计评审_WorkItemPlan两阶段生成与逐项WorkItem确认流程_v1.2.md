# WorkItemPlan 两阶段生成与逐项 Work Item 确认流程设计评审

## 文档信息

- 文档类型：设计评审
- 版本：v1.2
- 日期：2026-06-22
- 分支：feat-b-0616
- 被评审方案：`cadence/designs/2026-06-22_技术方案_WorkItemPlan两阶段生成与逐项WorkItem确认流程_v1.4.1.md`（原评审对象为 v1.4，已按本评审建议优化为 v1.4.1）
- 评审方式：只读调研 + 与当前 `WorkspaceEngine` / WS contract / 前端 store / `LifecycleStore` 对照

## 评审结论

v1.4 已基本吸收 v1.1 评审中的核心修订：不可变 `draft_id`、downstream invalidation、`WorkItemPlanCompileTransaction`、`work_item_plan_context_blocker` 节点、`repository_profile` 兼容策略、自动模式上下文术语统一。

但 v1.4 作为"待实现计划拆解"的文档，仍有 4 个 P0 和 5 个 P1 缺口会直接影响实现计划拆解与后续返工。本评审发出后，方案已按建议优化为 **v1.4.1**，P0/P1 缺口已基本补齐，可进入实现计划拆解阶段。

## 修订后复核（v1.4.1）

| 本评审问题 | 优先级 | v1.4.1 修复状态 | 说明 |
| --- | --- | --- | --- |
| P0-1 `generation_mode_select` 返修消息未明确 | P0 | ✅ 已修复 | 新增 `request_outline_revision` 消息；`select_work_item_generation_mode` 仅 mode 为 `serial`/`batch`；禁止该节点使用 `author_decision`。 |
| P0-2 串行模式局部校验时序未明确 | P0 | ✅ 已修复 | author 输出后自动运行 `WorkItemDraftLocalValidator`；通过后展示"接受"；失败后只展示"重写/暂停"与 findings。 |
| P0-3 自动模式降级串行后 batch draft 迁移未明确 | P0 | ✅ 已修复 | 受影响 outline 之前的 batch drafts 复制为新串行 draft 并重新跑局部校验/review；受影响 outline 及之后按串行重新生成。 |
| P0-4 `recovery_required` 缺少 stage/node/WS 消息 | P0 | ✅ 已修复 | 新增 `work_item_plan_compile_recovery` 节点（复用 `human_confirm`），`work_item_plan_compile_recovery_action` 消息支持 `continue`/`abort_and_rollback`/`human_triage`。 |
| P1-1 `WorkItemPlanReviewComplete` 代码层面未明确 | P1 | ✅ 已修复 | 给出 Rust struct/enum 草案，明确嵌入 `ReviewComplete` 的方式与兼容降级规则。 |
| P1-2 复用旧 draft 状态机位置未对齐 | P1 | ✅ 已修复 | 入口从 `generation_mode_select`/`draft_confirm` 改为 Outline 返修通过后的"重新生成准备阶段"。 |
| P1-3 reviewer sentinel block 迁移路径未明确 | P1 | ✅ 已修复 | 明确本次统一改造所有 WorkspaceType 的 reviewer 输出与解析，旧 markdown fence 可降级解析一个版本。 |
| P1-4 `batch_review` 流转未明确 | P1 | ✅ 已修复 | 通过后自动进入 `final_compile`；不通过后自动回到 `batch_confirm` 并展示 findings。 |
| P1-5 `outline_context_index.json` 未明确 | P1 | ✅ 已修复 | 明确为必须实现项，给出 schema 与更新规则。 |

## 项目进度摸底

- worktree：`.worktrees/feat-b-0616`
- 分支状态：`feat-b-0616` 已关联 `origin/feat-b-0616`，工作区干净，`git fetch origin main` 后 `main...HEAD` 显示该分支领先 main 约 30+ 个 commit，全部与 WorkItemPlan 相关。
- 分支内容：相对 `origin/main` 已包含 WorkItem 拆分、WorkItemPlan 对话式 Workspace、配置弹窗、分组删除、WorkItemPlan 流式进度与恢复等实现，覆盖后端 engine/store/WS、前端 store/page/component、集成测试。
- 当前方案状态：v1.4 已按本评审建议优化为 v1.4.1，核心协议与状态机缺口已补齐，可进入实现计划拆解；代码实现仍停留在旧"一次性生成完整 work items + 自动返修 loop"流程。

## 对照 v1.1 评审的修复状态

| v1.1 问题 | 优先级 | v1.4 修复状态 | 说明 |
| --- | --- | --- | --- |
| P0-1 Draft store 以 `outline_id` 为主键，历史无法保留 | P0 | ✅ 已修复 | 引入不可变 `draft_id`，`outline_id` 仅作业务关联，active index 指向当前 draft，历史 draft 不覆盖。 |
| P0-2 最终 strict validator item 级失败与"已确认中间项不可重写"冲突 | P0 | ✅ 已修复 | 增加 `WorkItemDraftLocalValidator` 局部校验，定义 downstream invalidation（目标 item 及下游 draft 标记 `superseded`）。 |
| P1-1 `final_compile` 缺少原子性、恢复与 abort 语义 | P1 | ✅ 已修复 | 引入 `WorkItemPlanCompileTransaction`，定义状态机、step_cursor、created ids、commit marker、幂等续跑规则。 |
| P1-2 新 reviewer verdict 未落到 WS / ReviewGate 协议 | P1 | ⚠️ 部分修复 | 提出 `WorkItemPlanReviewComplete` 子结构承载 `revise_batch` / `plan_reopen_required`，但代码层面（Rust struct/enum、serde、兼容策略）仍未明确。 |
| P1-3 `context_blockers` 与 Outline 轻量校验失败缺少人工入口 | P1 | ✅ 已修复 | 新增 `work_item_plan_context_blocker` 节点（`human_confirm` 阶段），允许用户补充上下文或终止。 |
| P2-1 `repository_profile` 移除范围不完整 | P2 | ✅ 已修复 | 有专门兼容策略章节，明确新流程中 `IssueWorkItemPlan.repository_profile_ref = None`、`VerificationPlan.repository_profile_ref = None`。 |
| P2-2 自动模式上下文术语不准确 | P2 | ✅ 已修复 | 统一改为"当前 batch 中前序已生成并被调度器接收的 draft records"。 |

## 主要问题

### P0-1 `generation_mode_select` 节点"返回 Outline 返修"的输入消息未明确

v1.4 要求 `generation_mode_select` 节点提供"逐个生成 / 自动生成 / 返回 Outline 返修"三个分支，并新增 `select_work_item_generation_mode` 消息。但 Stage / WS 矩阵中该节点的可接收消息写为 `select_work_item_generation_mode` / `request_revision`，说明"返回 Outline 返修"可能走 `request_revision`。

问题：

- `request_revision` 是通用消息，携带 `StructuredFeedback`（`feedback_types[]` + `description`）。但 Outline 返修需要携带针对整个 Outline 的反馈，这与单 item / batch 返修消息已分离的设计相矛盾。
- 如果 `select_work_item_generation_mode` 的 `mode` 枚举扩展出 `back_to_outline_revision`，则该消息名与语义不匹配（不是"选择生成模式"而是"请求 Outline 返修"）。
- 后端 handler 需要同时校验 `WorkspaceStage == author_confirm` 与 `active_node.node_type == work_item_generation_mode`，但没有说明收到 `request_revision` 时如何校验。

建议修订：

1. 明确三种分支分别用哪个消息承载：
   - "逐个生成"：`select_work_item_generation_mode { mode: "serial" }`
   - "自动生成全部"：`select_work_item_generation_mode { mode: "batch" }`
   - "返回 Outline 返修"：新增专用消息，例如 `request_outline_revision { feedback: OutlineRevisionFeedback }`，或复用 `request_revision` 但要求后端在 `work_item_generation_mode` 节点下将其解释为 Outline 级返修。
2. 在协议矩阵中补全 `generation_mode_select` 节点的所有可能输入消息与前端操作映射。
3. 若复用 `request_revision`，需在消息处理文档中明确：在 `work_item_generation_mode` 节点收到 `request_revision` 等价于"返回 Outline 返修"，不触发旧整组 revision 路径。

相关位置：v1.4 第 316-371 行。

### P0-2 串行模式 item 级局部校验与用户"接受"操作的时序未明确

v1.4 原文：

> "当前 work item 在进入 `accepted` 前必须先通过 draft 局部严格校验。"
> "用户接受当前 work item author 结果后，进入该 work item 的 reviewer 审核。"

问题：

- 局部校验是在用户点击"接受"前自动运行，还是点击"接受"后、进入 reviewer 前运行？
- 如果校验在"接受"前运行且失败，用户尚未点击"接受"就要返修，此时前端应展示"校验失败，需要重写"而不是"接受/重写/暂停"。
- 如果校验在"接受"后运行，那么"接受"按钮点击后存在两种结果：accepted 并进入 reviewer，或校验失败退回当前 item 返修。这会影响前端状态机与 WS 消息设计。
- 两种时序对应的节点状态不同：前者 `item_draft_confirm` 节点不展示"接受"按钮；后者 `item_draft_confirm` 节点允许点击"接受"，点击后进入子状态或新节点。

建议修订：

1. 明确选择一种时序并在方案中写死。推荐：
   - author 输出后，后端自动运行 `WorkItemDraftLocalValidator`。
   - 若校验通过，`item_draft_confirm` 节点展示"接受 / 重写 / 暂停"；用户点击"接受"后状态变为 `accepted` 并进入 `item_draft_review`。
   - 若校验失败，`item_draft_confirm` 节点不展示"接受"，只展示"重写 / 暂停"，并附带 validator findings。
2. 在状态机矩阵中补充"draft 局部校验中"这一状态或子状态，明确前端展示内容与可接收消息。
3. 在验证策略章节补充对应测试："draft 局部校验失败时 item 不可被接受"。

相关位置：v1.4 第 186-207 行、第 322-329 行。

### P0-3 自动模式降级为串行模式后，batch drafts 如何迁移到串行状态未明确

v1.4 规定：自动模式 strict validator item 级失败时，用户可"明确选择降级为串行模式重新生成；降级后按串行模式规则从第一个受影响 item 开始处理"。

问题：

- 降级前 batch 中已生成的 drafts 状态是 `draft`（未确认）。降级为串行后，这些 draft 是否自动转为串行模式中的"已接受"上下文？
- 如果自动转为已接受，它们需要重新跑局部校验和 review 吗？串行模式下每个 item 都必须通过 review（若开启）。
- 如果不自动转为已接受，而是全部废弃从第一个 item 重新生成，那么"降级"与"整组重写"的差异不大，产品价值变低。
- 降级操作发生在哪个节点？是 `batch_confirm` 失败后展示"降级为串行"入口，还是 `batch_review` 失败后？

建议修订：

1. 明确降级语义：推荐"从第一个受影响 item 开始按串行模式重新生成"，但允许未受影响的 batch drafts 被复制为新的串行 draft 并重新跑局部校验（不自动 accepted）。
2. 定义降级后的状态转换：
   - 受影响 item 之前的 batch drafts：复制为新串行 draft，进入 `item_draft_confirm` 节点（需用户逐个确认）。
   - 受影响 item 及之后：按串行模式重新生成。
   - 所有复制的 draft 必须重新跑 `WorkItemDraftLocalValidator`，若 reviewer 开启还需进入 `item_draft_review`。
3. 在 WS 矩阵中补充"降级为串行"的触发节点、输入消息与前端操作。

相关位置：v1.4 第 240-269 行、第 331 行。

### P0-4 `WorkItemPlanCompileTransaction` 进入 `recovery_required` 后缺少 stage / node type / WS 消息定义

v1.4 详细定义了 `recovery_required` 状态与三种人工操作（继续 / 放弃 / 转人工整理），但 Stage / WS 矩阵中没有对应的恢复阶段。

问题：

- 最终编译阶段在矩阵中只写了"`abort`（仅 transaction 进入 `committing` 前有效）"，没有写 recovery 时的操作。
- `recovery_required` 时应进入哪个 `WorkspaceStage`？是 `human_confirm` 还是新增 stage？
- 应创建什么 timeline node type？矩阵中的 `work_item_plan_compile` 节点已经结束，recovery 是否需要新节点（如 `work_item_plan_compile_recovery`）？
- 前端操作区应展示什么？三种人工操作如何映射为 WS 输入消息？
- 若用户选择"放弃"，需要清理已创建实体并回到 `batch_confirm` 或 `item_draft_confirm`。但`plan_commit_state=committed` 时不能放弃，这个条件如何在前端展示？

建议修订：

1. 在状态机矩阵中新增一行：
   - 业务阶段：Compile recovery
   - 复用 `WorkspaceStage`：`human_confirm`
   - active timeline node type：`work_item_plan_compile_recovery`（或复用 `work_item_plan_compile` 但 detail 中展示 recovery 状态）
   - 前端主操作：继续提交 / 放弃本次 compile（仅 `plan_commit_state=not_started`） / 转人工整理
   - WS 输入消息：新增 `work_item_plan_compile_recovery_action` 或复用 `human_confirm` 携带 recovery payload
   - 关键恢复 payload：`compile transaction`、transaction report、已创建 ids、可执行操作列表
2. 明确"继续""放弃""转人工整理"三种操作的幂等语义与回滚范围。
3. 若选择不复用 `human_confirm`，需评估是否新增 `WorkspaceStage` 变体（v1.4 主张不新增 stage，因此推荐复用 `human_confirm`）。

相关位置：v1.4 第 508-545 行、第 333 行。

## 次要问题

### P1-1 `WorkItemPlanReviewComplete` 子结构的代码层面协议未明确

v1.4 建议新增 `WorkItemPlanReviewComplete` 子结构放在 `ReviewComplete.work_item_plan_review` 下，不直接扩展共享 `ReviewVerdictType` / `ReviewGate` enum。这是正确的兼容策略，但缺少代码层面的 struct/enum 草案。

问题：

- 当前 `ReviewComplete`（`WsOutMessage::ReviewComplete`）字段为 `verdict: ReviewVerdictType`、`review_gate: ReviewGate`、`findings: Vec<ReviewFinding>` 等，没有 `work_item_plan_review` 子结构的位置。
- `ReviewFinding` 当前没有 `target_outline_id`、`severity` 等字段与 v1.4 的 finding schema 对齐（`ReviewFinding` 已有 `severity`、`message`、`evidence`、`impact`、`required_action`，但没有 `target_outline_id`）。
- `WorkItemPlanReviewComplete` 中的 `review_action`、`gates` 等字段如何影响后端状态机跳转，尚未定义。
- Story / Design / 普通 WorkItem Workspace 收到带 `work_item_plan_review` 的 `ReviewComplete` 时应降级为 `human_triage`，这个降级逻辑在哪个层级实现？

建议修订：

1. 给出 `WorkItemPlanReviewComplete` 的 Rust struct 草案，例如：
   ````rust
   pub struct WorkItemPlanReviewComplete {
       pub verdict: WorkItemPlanReviewVerdict,
       pub review_scope: WorkItemPlanReviewScope,
       pub target_outline_id: Option<String>,
       pub generation_round_id: String,
       pub draft_id: Option<String>,
       pub batch_id: Option<String>,
       pub review_action: WorkItemPlanReviewAction,
       pub gates: Vec<WorkItemPlanReviewGate>,
   }
   ````
2. 明确 `ReviewComplete` 如何携带该子结构，例如：
   ````rust
   pub struct ReviewComplete {
       pub node_id: String,
       pub round: u32,
       pub verdict: ReviewVerdictType,
       pub comments: String,
       pub summary: String,
       pub findings: Vec<ReviewFinding>,
       pub review_gate: ReviewGate,
       #[serde(default, skip_serializing_if = "Option::is_none")]
       pub work_item_plan_review: Option<WorkItemPlanReviewComplete>,
   }
   ````
3. 明确兼容降级规则：前端解析 `ReviewComplete` 时，若存在 `work_item_plan_review` 则按 WorkItemPlan 专属逻辑路由；否则走通用 review 逻辑；未知 verdict 降级为 `human_triage`。
4. 明确后端 `parse_review_json` 的改造：对 WorkItemPlan reviewer run，先尝试解析通用 `ReviewVerdictType`，再尝试解析 sentinel block 内的 `WorkItemPlanReviewComplete`。

相关位置：v1.4 第 374-398 行；当前协议见 `src/web/workspace_ws_types.rs` 第 96-103 行、第 564-609 行。

### P1-2 downstream invalidation 后"复用未受影响旁路 draft"的状态机位置未对齐

v1.4 说：

> "前端在 `work_item_generation_mode` 或 `work_item_draft_confirm` 节点为未受影响的 outline 提供'复用上一版 draft'入口。"

问题：

- `work_item_generation_mode` 节点的操作是"逐个生成 / 自动生成全部 / 回到 Outline 返修"，矩阵中没有"复用 draft"这一操作。
- 在 `work_item_draft_confirm` 节点提供"复用上一版 draft"也不合理：该节点出现时后端已经决定要为该 outline 生成新 draft（否则不会进入 `work_item_draft_run` 后的 confirm）。如果用户想复用旧 draft，应在生成前决定，而不是生成后。
- "复用上一版 draft"的触发时机应该是 Outline 返修通过后、重新进入生成阶段前，由系统主动识别哪些 outline 有未受影响的旧 draft，并询问用户是否复用。

建议修订：

1. 将"复用旧 draft"入口放在 Outline 返修通过后的重新生成准备阶段，而不是 `work_item_generation_mode` 或 `work_item_draft_confirm`。
2. 新增一个节点类型或扩展 `work_item_generation_mode` 节点的 payload：展示"本轮需要重新生成的 outline 列表"和"可复用旧 draft 的 outline 列表"，让用户批量选择。
3. 明确复用后的流程：后端复制旧 `WorkItemDraftRecord` 为新 `draft_id`（记录 `copied_from_draft_id`），重新跑局部校验，然后进入 `item_draft_confirm` 或 `item_draft_review`。
4. 更新 Stage / WS 矩阵与验证策略章节。

相关位置：v1.4 第 211-212 行、第 991-992 行、第 316-329 行。

### P1-3 reviewer prompt 统一改造为 sentinel structured block 的迁移路径未明确

v1.4 R18 提出"reviewer prompt 统一改造为 sentinel structured block（现状用 markdown JSON fence）"。当前代码中 reviewer prompt 仍要求 markdown JSON fence（`src/product/workspace_engine.rs:3332-3344`），而 author prompt 已使用 sentinel block。

问题：

- 改造范围是否只限于 WorkItemPlan reviewer，还是 Story / Design / 普通 WorkItem reviewer 也要同步改造？
- 如果仅改造 WorkItemPlan reviewer，会导致同一项目内不同 WorkspaceType 的 review 解析路径不一致，增加维护成本。
- 如果同步改造所有 reviewer，影响面超出本方案，需要单独 WP。
- `parse_review_json` 当前从 markdown code fence 提取 JSON，改造后需要同时支持旧格式（兼容历史数据）和新 sentinel block。

建议修订：

1. 在方案中明确 reviewer prompt 改造范围：
   - 选项 A：本次仅改造 WorkItemPlan reviewer，但要求 `parse_review_json` 同时支持 sentinel block 与 markdown fence，并声明后续再迁移其他 WorkspaceType。
   - 选项 B：本次统一改造所有 reviewer prompt 与解析路径（推荐，但需单独 WP）。
2. 给出 sentinel block 的解析规则：与 author 一致，只解析最后一个 `<ARIA_STRUCTURED_OUTPUT>...</ARIA_STRUCTURED_OUTPUT>` block。
3. 在验证策略中补充：旧 reviewer 输出（markdown fence）在新流程中可降级解析，新输出必须使用 sentinel block。

相关位置：v1.4 第 37 行（R18）、第 668-679 行、第 750-752 行；当前解析见 `src/product/workspace_engine.rs` 第 4897-4949 行。

### P1-4 `batch_review` 节点 reviewer 通过/不通过后的流转未明确

v1.4 说"用户接受全部后，若 reviewer 开启，再进入整组 reviewer 审核"，因此 `batch_review` 节点在 reviewer run 结束后需要决定下一步。

问题：

- `batch_review` 节点 reviewer 通过后，是自动进入 `final_compile`，还是进入另一个人工确认节点？
- 如果不通过，是回到 `batch_confirm`（整组重写 / 暂停 / 转人工），还是直接进入 Outline 返修（`plan_reopen_required`）？
- `batch_review` 节点的前端操作是什么？矩阵中只写了"仅展示 reviewer 进度"，没有写通过/不通过后的操作入口。

建议修订：

1. 明确 `batch_review` 节点结束后的自动流转：
   - reviewer `pass`：自动进入 `final_compile`（无需用户再次确认）。
   - reviewer `revise_batch`：自动回到 `batch_confirm`，展示 reviewer findings，用户可选择整组重写、暂停或转人工。
   - reviewer `plan_reopen_required`：先执行 Draft records 清理/失效规则，然后进入 Outline 返修或人工决策节点。
   - reviewer `needs_human`：进入 `human_confirm` 节点，由用户决定下一步。
2. 在状态机矩阵中补充 `batch_review` 节点的前端操作与 WS 输入消息（虽然 reviewer 运行时通常只允许 `abort`，但 reviewer 结束后需要接收决策消息）。

相关位置：v1.4 第 227-238 行、第 331 行。

### P1-5 `context_blocker_resolution` artifact 的高效查询机制未明确

v1.4 说：

> "用户在该节点补充的上下文以 `context_blocker_resolution` artifact 写入当前 timeline node detail 与 artifact store，绑定 `session_id`、`blocker_node_id`、`resolution_node_id` 与 `created_at`，作为下一次 Outline author run 的 prompt 输入。Draft active index 不保存该信息；若需要快速查询，可单独维护 Outline 阶段的 `outline_context_index.json`。"

问题：

- "可单独维护"是可选还是必须？如果不维护，下一次 Outline author run 需要扫描 timeline 才能找到 blocker resolution，性能差且不稳定。
- `outline_context_index.json` 的格式、存储路径、更新时机、与 `WorkItemPlanOutline` 的关系均未定义。
- 如果多次进入 `work_item_plan_context_blocker` 节点，是否需要保存多个 resolution artifact？下一次 prompt 如何拼接？

建议修订：

1. 明确 `outline_context_index.json` 为必须实现项，给出 schema 草案：
   ````json
   {
     "blocker_resolutions": [
       {
         "blocker_node_id": "...",
         "resolution_node_id": "...",
         "resolution_artifact_ref": "...",
         "created_at": "..."
       }
     ]
   }
   ````
2. 明确下一次 Outline author run 的 prompt 输入：按时间顺序拼接所有 blocker resolutions，并附带当前 `design_context_gaps`。
3. 在验证策略中补充测试：多次 blocker resolution 后 Outline author prompt 包含全部历史 resolution。

相关位置：v1.4 第 111 行、第 323 行。

## 建议补充的问题

### P2-1 `parallel_scope_overlap` 等涉及多 item 的 finding 定位规则可更公平

v1.4 将 `parallel_scope_overlap` 归为 item 级，处理建议是"串行模式定位最近生成的相关 item 并执行 downstream invalidation"。

问题：

- 两个 item 的 scope 冲突时，"最近生成"的 item 不一定是责任方。例如早期 item 的 scope 写得太宽，导致后续 item 无处写入，责任在早期 item。
- 自动模式下 `parallel_scope_overlap` 作为 item 级失败会触发整组重写，这与"plan 级"处理的差别不大。

建议：

1. 对 `parallel_scope_overlap` 增加更细分的 finding code 或 metadata，例如 `scope_overlap_violator_outline_id`，帮助后端定位真正需要返修的 item。
2. 串行模式下根据责任方定位目标 item，而非简单"最近生成"。

相关位置：v1.4 第 1100-1128 行。

### P2-2 warning 类 finding 在 item accept 前局部校验中的处理未说明

v1.4 将 `verification_command_needs_manual_review` 和 `integration_or_e2e_skipped_risk` 归为 warning，不阻断编译。

问题：

- 在串行模式 item accept 前的局部校验中，如果仅出现 warning，是否允许用户直接"接受"？
- 如果允许，warning 是否需要在 `item_draft_confirm` 节点展示？
- 如果不允许，warning 与 item 级失败无差别，与"不阻断编译"的定义冲突。

建议：

1. 明确局部校验中 warning 的处理：展示但不阻塞"接受"，由用户决定是否继续。
2. 在 `WorkItemDraftCandidate` 或 `WorkItemDraftRecord` 中预留 `warnings` 字段，供前端展示。

相关位置：v1.4 第 205-207 行、第 1122-1125 行。

### P2-3 reviewer 开启后用户是否可在 revise verdict 下选择"接受风险继续"

v1.4 说串行模式 reviewer 不通过时"只重写当前 work item"，自动模式 reviewer 不通过时"只允许整组重写、暂停或转人工处理"。

问题：

- 没有说明 reviewer 返回 `revise` 时，用户是否可以选择不接受 reviewer 建议、直接接受风险继续。
- 现有 `ReviewGate::UserConfirmAllowed` 支持这种降级，WorkItemPlan 是否保留？

建议：

1. 明确 WorkItemPlan 的 review 规则：若 reviewer 开启，`revise` verdict 强制进入返修，不允许用户跳过（与 story/design/workitem 的 reviewer 开关语义一致）。
2. 或明确允许降级为 `human_triage`，由用户最终决定。

相关位置：v1.4 第 203-204 行、第 238 行、第 1206-1213 行。

### P2-4 Design spec heading/section 提取的可靠性策略未细化

v1.4 要求从已确认 Design spec 中提取 `design_context_capabilities` 和 `design_context_gaps`。

问题：

- Markdown heading 层级和标题措辞不固定，如何可靠识别"架构概览""模块划分""技术选型""关键目录结构"等章节？
- 是否需要 prompt 约束 Design author 输出固定 heading？如果约束，是否影响 Design spec 模板？
- 如果提取失败，fallback 策略是什么？

建议：

1. 在 Design spec 模板中约定标准 heading（如 `## 架构概览`、`## 模块划分` 等），并同时支持同义词匹配。
2. 提取逻辑采用"严格 heading 匹配 + 模糊语义 fallback"，并记录提取置信度。
3. 当提取置信度低时，将缺口写入 `design_context_gaps`，由 Outline author 通过 CLAUDE.md + 目录探索补齐。

相关位置：v1.4 第 94-113 行。

### P2-5 Artifact 版本与结构化 Diff 建议拆分为独立 WP 或分阶段实现

v1.4 提出完整的 artifact history index、6 种 artifact kind、5 种 diff type、三段式视图，工作量较大。

问题：

- 如果与两阶段生成主流程放在同一批 WP 中实现，会显著增加前端与后端数据层复杂度，拖累主流程交付。
- 当前 `workspace-ws-store` 只保存最后一个 artifact，改造成可查询的 history index 涉及面广。

建议：

1. 将 Artifact 版本与结构化 Diff 拆分为独立 WP，分两阶段实现：
   - 第一阶段：支持 Outline / Draft / Batch 历史版本只读回放（无 diff）。
   - 第二阶段：支持同类型版本结构化 diff 与跨类型 traceability view。
2. 在主流程实现阶段，先确保 timeline node 与 Draft store 保留足够信息（`source_node_id`、`draft_id`、`outline_version_ref`、`batch_id`、`compile_id`），为后续 diff 做准备。

相关位置：v1.4 第 587-664 行。

## 整体建议

1. **v1.4.1 可进入实现计划拆解**：P0 与 P1 缺口已在 v1.4.1 中补齐，协议层（节点类型、WS 消息、`ReviewComplete` 子结构、compile recovery）已具备拆 WP 条件。
2. **优先实现协议层/枚举层扩展**：首个 WP 应首先扩展 `TimelineNodeType`、`ArtifactPayload`、`ReviewComplete` 子结构、`WsInMessage` 新消息，并补充对应单元测试；这是后续 Outline / Draft / Compile 业务 WP 的前置依赖。
3. **Artifact 历史与 Diff 独立成 WP**：避免与核心两阶段流程耦合，降低首批实现风险。主流程 WP 只需保留足够元数据（`source_node_id`、`draft_id`、`batch_id`、`compile_id`）。
4. **回归测试策略**：新增 WorkItemPlan 专属 node type 和 payload 时，必须确保 Story / Design / 普通 WorkItem Workspace 不受影响。建议在实现计划中为每个 WP 明确"共享链路回归测试"作为 DoD。
5. **代码现状提示**：当前代码仍停留在旧一次性生成 + 自动返修 loop，新两阶段模型尚未落地。首个 WP 建议从协议层扩展与最小 Outline → Draft → Compile 端到端路径开始，避免一次性改造全部旧逻辑。
6. **P2 问题可后续细化**：`parallel_scope_overlap` 定位规则、warning 类 finding 处理、reviewer `revise` 降级语义、Design spec heading 提取策略可在实现计划中逐步明确，不阻塞拆 WP。
