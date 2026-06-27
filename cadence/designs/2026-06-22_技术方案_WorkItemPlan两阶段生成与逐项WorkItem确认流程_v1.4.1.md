# WorkItemPlan 两阶段生成与逐项 Work Item 确认流程技术方案

## 文档信息

- 文档类型：技术方案
- 版本：v1.4.1
- 日期：2026-06-22
- 分支：feat-b-0616
- 状态：基于 v1.4 review 进一步修订，待实现计划拆解
- 评审文档：`cadence/designs-reviews/2026-06-22_设计评审_WorkItemPlan两阶段生成与逐项WorkItem确认流程_v1.1.md`

## v1.4 变更摘要（基于 v1.3 review 后的修复）

- 明确 `generation_mode_select` 节点复用 `author_confirm` stage 的歧义消除规则：前端按 `active_node.node_type` 路由选择 UI，后端同时校验 stage + node type。
- 细化 Draft 阶段局部严格校验的实现方式：新增 `WorkItemDraftLocalValidator` 投影层，仅运行可定位到单 item 的现有 validator 子函数；明确局部校验不替代阶段 4 full validator，downstream invalidation 后需重新跑局部校验。
- 收敛自动模式 final compile item 级失败的处理：自动模式不做局部重生成，仍只允许整组重写、转人工处理，或由用户明确选择降级为串行模式重新生成。
- 明确 `plan_reopen_required` 触发后复用未受影响旁路 draft 的流程：前端展示“复用上一版 draft”入口，后端复制为新的 `draft_id` 并重新跑局部校验与 review。
- 明确 `context_blockers` 用户补充上下文的持久化边界：以 `context_blocker_resolution` artifact 写入 timeline / artifact store，作为下一次 Outline author run 的 prompt 输入；Draft active index 不保存该信息。
- 明确 `WorkItemPlanCompileTransaction` 非崩溃类写入失败的处理：status 置为 `recovery_required` 进入人工处理；`IssueWorkItemPlan` 指针提交前可清理回退，提交后只能继续幂等补齐或人工整理。
- 调整 review verdict 协议扩展的兼容策略：新增 `WorkItemPlanReviewComplete` 子结构承载 `revise_batch` / `plan_reopen_required`，不直接扩展共享 `ReviewVerdictType` enum，避免旧 Workspace 数据反序列化问题。
- 补充 WorkItemPlan artifact 版本与结构化 Diff 展示：用户可查看 Outline、Draft attempts、Batch snapshots、Compile reports 的历史版本，并按结构化字段对比变化。
- 清理文档中针对后续执行阶段的表述，聚焦 WorkItemPlan 生成流程；将“后续 Workspace/执行阶段”等用语统一为中性描述，避免与具体执行策略耦合。

## v1.4 → v1.4.1 修订摘要（基于 design review v1.2 的优化）

本次修订针对 v1.4 在拆实现计划前仍需明确的协议与状态机缺口进行补齐：

- 明确 `generation_mode_select` 节点的三种分支消息：`select_work_item_generation_mode`（mode 为 `serial` / `batch`）与 `request_outline_revision`（返回 Outline 返修）；禁止在该节点使用通用 `author_decision`。
- 明确串行模式 draft 局部校验时序：author 输出后自动运行 `WorkItemDraftLocalValidator`，通过后用户才可见"接受"按钮；校验失败只展示"重写 / 暂停"与 findings。
- 明确自动模式降级为串行模式后的 batch draft 迁移规则：受影响 outline 之前的 drafts 复制为新串行 draft 并重新跑局部校验与 review，受影响 outline 及之后按串行重新生成。
- 明确 `WorkItemPlanCompileTransaction` 进入 `recovery_required` 后的阶段：`work_item_plan_compile_recovery` 节点（复用 `WorkspaceStage::human_confirm`），支持 `continue` / `abort_and_rollback`（仅 `plan_commit_state=not_started`） / `human_triage` 三种操作。
- 明确自动模式整组 review 通过/不通过后的流转：通过后自动进入 `final_compile`；不通过后自动回到 `work_item_batch_confirm` 并展示 findings 与操作入口。
- 给出 `WorkItemPlanReviewComplete` 的 Rust struct / enum 草案，明确其嵌入 `ReviewComplete` 的方式与兼容降级规则。
- 明确 reviewer prompt 统一迁移到 sentinel structured block 的路径：本次统一改造所有 WorkspaceType 的 reviewer 输出与解析，旧 markdown fence 可降级解析一个版本。
- 明确 `outline_context_index.json` 为必须实现项，给出 schema 与更新规则。
- 调整 downstream invalidation 后"复用上一版 draft"入口的位置：从 `work_item_generation_mode` / `work_item_draft_confirm` 改为 Outline 返修通过后的"重新生成准备阶段"。

## v1.3 变更摘要（基于 v1.0 设计评审 R1-R22）

- R1 reviewer 开关：三条强规则中"强制开启，不允许跳过"改为"默认开启，与 story/design/workitem 共用 reviewer 开关，用户可关闭"。
- R2 自动模式简化：删除并行/DAG/dependency layer/同层并行等描述；明确"串行自动生成全部 + 整组 review"；per-item prompt 与串行模式一致；并行相关字段标注"后续扩展"。
- R3 handoff 范围明确：只在 WorkItemPlan 生成流程内（Outline 规划 + Draft 生成消费），不进入后续执行阶段；明确"预期 handoff"语义。
- R4 repository_profile 退出 WorkItemPlan 新流程：provider 不再输出，编译结果置空 legacy ref；author 仓库知识来自 Design spec + CLAUDE.md + 目录探索。
- R5 新增前置工作：Design spec 模板补强（架构/模块/技术选型章节）。
- R6 author 探索能力：prompt 允许读 CLAUDE.md + 仓库目录结构（只读，不作为 plan 字段）。
- R7 work_item_id 阶段 4 编译时分配；Draft 阶段以不可变 `draft_id` 为主键，`outline_id` 仅作业务关联。
- R8 strict validator 复用现有 5 函数，失败分 item 级 / plan 级。
- R9 Draft 持久化改为独立 `WorkItemDraftRecord`，不复用 `LifecycleWorkItemRecord` 占位；编译时创建真实 work item、verification plan 与 child session。
- R10 编译失败处理：item 级 / plan 级分流。
- R14 状态机倾向新增语义化 node type，非纯 metadata。
- R18 reviewer prompt 统一改造为 sentinel structured block（现状用 markdown JSON fence，与 author 解析路径不同）。
- R19 `required_gates` 规则说明现有 validator 已覆盖。
- R20 `plan_reopen_required` 触发后的 Draft records 处理规则。
- R21 Outline 轻量校验失败状态归属。
- 其他：R11 修正"已确认的后续项"笔误；R12 明确轻量校验与现有 validator 复用关系；R13 `generation_mode_select` 接入点；R15 补数据流转图；R16 命名统一；R17 review 状态字段来源；R22 补测试。
- 本轮补充收敛高/中风险：
  - Draft 阶段不再复用 `LifecycleWorkItemRecord` 作为可变占位记录，改为独立 `WorkItemDraftRecord`；阶段 4 才编译为真实 `LifecycleWorkItemRecord` / `VerificationPlan` / child workspace session，避免 `id` 迁移和 session/entity 引用重写。
  - 补充 `stage` / timeline node / WS message / 前端操作 / 恢复 payload 矩阵，明确 `generation_mode_select`、逐项确认、整组确认的协议入口。
  - 自动模式统一为"整组生成完成 → 用户接受整组 → reviewer 整组审核 → final compile"；reviewer 关闭时，用户接受整组后直接进入 final compile。
  - `superseded` 只属于 `WorkItemDraftStatus`，不扩展现有 `WorkItemPlanStatus`。
  - 补充 strict validator finding code 到 item 级 / plan 级 / warning 的归因映射。
- 本轮评审修订补齐实现前置缺口：
  - Draft store 改为不可变 `draft_id` 主键，`outline_id` 仅作为业务关联字段；通过 active index 指向当前可用 draft，历史 draft 不覆盖。
  - 串行模式新增 downstream invalidation：若已确认早期 item 需重写，其所有下游 draft 默认失效并重新生成。
  - `final_compile` 改为 compile transaction + 幂等提交；strict validator 通过前不写真实 WorkItem / VerificationPlan / child session。
  - review verdict 协议显式扩展 `revise_batch` / `plan_reopen_required` 与对应 review gate / timeline metadata。
  - `context_blockers[]` 不再停在 `outline_running`，改为进入明确人工确认节点。
  - `repository_profile` 改为退出 WorkItemPlan 新流程但保留 legacy 字段兼容，不做同 PR 全局删除。

## v1.2 变更摘要

- 强化 prompt 契约：所有 author/reviewer 输出必须使用 sentinel structured block，正文进度与机器输出分离。
- 补充 WorkItemPlan Outline、WorkItemDraftCandidate、VerificationPlan、review verdict 的结构化 schema 要点。
- 明确 `required_gates` 只能引用同一 verification plan 内的 command/manual_check id，禁止自然语言 gate。
- 明确自动模式不是一个大 prompt 生成全部，而是调度器按 dependency layer 多次调用单 item prompt。

## 背景

当前 WorkItemPlan 流程由 author 一次性生成完整结构化结果，包括 `IssueWorkItemPlan`、全量 `work_items`、全量 `verification_plans`、repository profile 和 dependency graph。后端解析后立即物化并执行严格校验；校验失败时进入内部自动返修循环。

这个流程存在几个产品问题：

- author 长时间探索和生成时，用户只能看到零散 provider 工具事件，难以判断进展。
- 用户无法先确认拆分策略，只能等待完整 work item 生成完成。
- 单个 work item 或 verification plan 输出错误会导致整组校验失败。
- 校验失败后的自动返修是黑盒流程，和 Story/Design Workspace 的确认/返修体验不一致。
- 每个 work item 没有独立消息气泡，也没有独立暂停、确认、重写边界。

本方案废弃当前“一次性生成全量 Work Item + 全量校验 + 自动返修”的主流程，改为 WorkItemPlan Outline 确认后，再按用户选择逐个或自动生成真实 Work Item。

## 目标

- WorkItemPlan 第一阶段只生成”如何编写 work item 的计划”，不生成完整 work item。
- 用户先确认整体拆分方案，再选择生成模式。
- 严格串行模式下，每个 work item 独立生成、独立展示、独立确认、可独立重写。
- 自动模式下，系统按拓扑顺序串行自动生成全部 work item，只支持整组确认或整组重写（并行生成为后续扩展，不在本方案范围）。
- 生成后续 work item 时必须携带前序已确认 work item 的上下文，避免 prompt 丢失。
- 最终全部确认后，再编译成现有真实数据结构并执行严格 validator。
- reviewer 默认开启，与 story/design/workitem 共用 reviewer 开关，用户可关闭；WorkItemPlan 不做特例。

## 前置工作

实施本方案前需完成：

- **Design spec 模板补强**：要求 Design spec 包含架构/模块/技术选型章节（模块划分、技术栈、测试框架、关键目录结构）。author 生成 work item 时从此章节获取仓库结构知识，替代原有 `repository_profile` 探测。
- **author 探索能力**：author prompt 允许读取 CLAUDE.md（项目技术栈章节）和目标仓库的目录结构（只读，不得修改文件，不得创建计划文档），作为 spec 信息的补充。探索所得不作为 plan 持久化字段，仅用于本次 prompt 上下文。

### Design Spec 前置门禁与兼容策略

Design spec 模板补强不只是文档要求，需落到 WorkItemPlan Outline 生成前的上下文门禁：

- **新 Design spec**：Design author / reviewer prompt 必须要求产物包含以下二级章节或等价结构：
  - 架构概览。
  - 模块划分。
  - 技术选型与测试框架。
  - 关键目录结构与主要落点。
  - 外部依赖、运行方式与验证约束。
- **WorkItemPlan prepare 阶段**：构建 Outline prompt 前，后端对已确认 Design spec 做轻量 heading/section 提取，生成 `design_context_capabilities`：
  - `has_architecture`
  - `has_module_breakdown`
  - `has_tech_stack`
  - `has_test_strategy`
  - `has_key_paths`
- **兼容旧 Design spec**：若旧 spec 缺少上述章节，不直接阻断 WorkItemPlan 生成；后端把缺口写入 `design_context_gaps`，并强制 Outline author 通过 CLAUDE.md + 目录结构只读探索补齐假设。
- **不可恢复缺口**：若 Design spec 缺口 + CLAUDE.md + 目录结构摘要仍不足以判断模块边界或测试策略，Outline author 必须在结构化输出中返回 `context_blockers[]`；后端完成当前 outline run 后进入 `human_confirm` 阶段的 `work_item_plan_context_blocker` 节点，不进入 `outline_confirm`。该节点只允许用户补充上下文后重跑 Outline，或终止流程；不得把 blocker 当作已确认 Outline 继续推进。用户在该节点补充的上下文以 `context_blocker_resolution` artifact 写入当前 timeline node detail 与 artifact store，绑定 `session_id`、`blocker_node_id`、`resolution_node_id` 与 `created_at`，作为下一次 Outline author run 的 prompt 输入。Draft active index 不存储 `context_blocker_resolution`；必须单独维护 Outline 阶段的 `outline_context_index.json`，否则下一次 Outline author run 需要扫描全量 timeline，性能与稳定性不可接受。它不会自动反向修改已确认的 Design spec；如需永久写入 Design spec，应走 Design spec 返修流程。
- **Reviewer 责任**：Outline reviewer 必须检查 `design_context_gaps` 与 author 的补齐假设；如果补齐假设会影响拆分边界，应返回 `revise` 或 `needs_human`。

## 非目标

- 本方案不定义 WorkItem 生成之后的执行策略。
- 本方案不保留当前 WorkItemPlan 自动返修 loop 作为主流程。
- 本方案不要求一次实现所有 UI 优化，但协议与状态机必须支持最终体验。
- 本方案不细化 reviewer prompt 的全部评分细则；评分细则可在实现计划中继续拆解。

## 推荐流程

### 阶段 1：生成 WorkItemPlan Outline

author 生成一个可确认的 plan outline。该 outline 是用户可读、系统可解析的拆分方案，至少包含：

- 整体拆分策略。
- work item 大纲列表。
- 每个 work item 的稳定 outline id、标题、类型、目标、范围。
- 每个 work item 的预期写入边界和禁止写入边界。
- 每个 work item 关联的 Story/Design 来源。
- work item 之间的依赖关系。
- 推荐执行顺序（拓扑顺序）。
- 每个 work item 的验证意图概要。
- 风险、handoff 信息和上下文传递要求。

此阶段只执行轻量校验（复用现有 `WorkItemSplitValidator` 的部分纯函数，适配签名；现有入参是 `IssueWorkItemPlan` + `LifecycleWorkItemRecord[]`，Outline 阶段尚无这些数据，需提取 outline 级别的校验函数）：

- outline id 唯一且稳定。
- dependency graph 引用存在。
- dependency graph 无环。
- 每个 work item outline 有 Story/Design 追踪关系。
- 每个 work item outline 有基本目标、范围和写入边界。
- 依赖项之间的写入边界不存在明显冲突（拓扑顺序生成时，直接依赖项的 `exclusive_write_scopes` 不应互相覆盖）。

此阶段不生成完整 `LifecycleWorkItemRecord`，不生成完整 `VerificationPlan`，不运行当前 full `WorkItemSplitValidator`。

### 阶段 2：用户确认 Outline

Outline 生成后进入确认节点。用户可以：

- 确认该 plan。
- 要求重写整个 plan。
- 带反馈重写整个 plan。

用户确认后进入 WorkItemPlan Outline review。review 默认开启，与 story/design/workitem 共用 reviewer 开关；用户可关闭，关闭时不进入 review 阶段（详见 Review 规则章节）。

Reviewer 审核对象是 WorkItemPlan Outline，而不是完整 Work Item。审核范围包括：

- 拆分策略是否合理。
- work item 大纲是否覆盖 Story/Design。
- dependency graph 是否合理且无明显缺口。
- 写入边界是否存在明显冲突。
- work item 是否过粗、过细、遗漏或顺序错误。

Reviewer 通过后，页面显示两个生成入口：

- 逐个生成 Work Item。
- 自动串行生成全部。

Reviewer 不通过时，流程回到 WorkItemPlan Outline 返修；如果 reviewer 判定需要人工判断，则停在人工决策节点，不进入 Work Item 生成。

### 阶段 3A：逐个生成 Work Item

严格串行模式。系统按 outline 的拓扑顺序逐个生成 work item。

每个 work item 生成时，author prompt 必须包含：

- 已确认的 WorkItemPlan Outline。
- 当前 work item outline。
- 所有前序已确认 work item 的摘要。
- 当前 work item 直接依赖项的完整内容。
- 前序已确认 work item 的写入边界、验证约束和 handoff_summary。
- 当前生成模式和用户反馈。

每个 work item 生成完成后创建独立消息气泡和确认节点。用户可以：

- 接受当前 work item（仅在 draft 局部校验通过后可用）。
- 带反馈重写当前 work item。
- 暂停流程。
- 继续生成下一个 work item。

每个 work item 生成完成后，后端自动运行 draft 局部严格校验（见下段）。校验通过后，该 work item 进入 `work_item_draft_confirm` 节点，前端展示"接受 / 重写 / 暂停"；用户点击"接受"后，draft 状态变为 `accepted` 并进入该 work item 的 reviewer 审核（默认开启，可关闭）。若校验失败，前端只展示"重写 / 暂停"并附带 validator findings，不允许接受。串行模式下 review 粒度是单个 work item，当前 work item review 通过前不能继续生成下一个 work item。

Reviewer 审核对象包括：

- 当前 work item 是否符合对应 outline。
- 当前 work item 是否正确引用前序已确认 work item 的上下文和 handoff。
- 写入边界是否和已确认 work item 冲突。
- verification plan 是否完整、可执行，且 required gates 引用合法。
- 当前 work item 是否足以支撑后续 Workspace 的输入上下文。

Reviewer 不通过时，只重写当前 work item；重写 prompt 必须携带 reviewer finding、当前 outline、已确认前序 work item 上下文，以及用户补充反馈。

当前 work item 在进入 `accepted` 前必须先通过 draft 局部严格校验。该校验用当前 draft 投影出临时 `LifecycleWorkItemRecord` / `VerificationPlan`，并结合已确认前序 draft 检查以下可定位到单 item 的规则：traceability、write scope、context budget、verification plan 内部合法性、required gates、command cwd/safety/source、与直接依赖的 scope/handoff 一致性。未通过时停留在当前 item 返修，不允许把明显 item 级错误推迟到阶段 4。

> **实现说明**：局部校验通过新增的 `WorkItemDraftLocalValidator` 投影层完成。该投影层将当前 draft 及其已确认前序 drafts 转换为临时的 `LifecycleWorkItemRecord[]` / `VerificationPlan[]`，并调用现有 `WorkItemSplitValidator` 中可归因到单 item 的纯函数（如 `validate_verification_commands`、`validate_scopes_and_budgets` 等，需适配入参签名）。局部校验只检查当前 draft 自身及与其直接依赖相关的约束，不检查跨全 plan 的依赖图一致性。若阶段 4 的 full validator 仍发现早期已确认 item 存在问题，触发 downstream invalidation 后，目标 item 及其下游重新生成时仍需再次通过局部校验。

如果当前 work item 被重写，后续未生成项应使用重写后的版本作为上下文。串行模式严格按拓扑顺序逐个生成，常规返修只针对当前 item，前序已确认项作为后序 prompt 上下文保持不变。

例外：若后续 reviewer 或最终 strict validator 证明某个已确认早期 item 必须重写（例如 verification plan 内部错误、handoff 与 downstream 依赖不兼容、scope 改动影响依赖链），后端必须执行 downstream invalidation：目标 item 及所有通过 Outline dependency graph 可达的下游 active drafts 默认标记 `superseded`，并从目标 item 重新进入串行生成。被标记失效的 draft 不参与阶段 4 编译。

> **复用未受影响旁路 draft 的时机**：用户选择复用旧 draft 不发生在 `work_item_generation_mode` 或 `work_item_draft_confirm` 节点，而是发生在 Outline 返修通过后的"重新生成准备阶段"。后端对比新旧 Outline，识别出"未变化且未被 downstream invalidation 命中"的 outline，为这些 outline 提供"复用上一版 draft"入口。用户确认复用后，后端复制旧 `WorkItemDraftRecord` 为当前 `generation_round_id` 下的新 `draft_id`（记录 `copied_from_draft_id`），重新运行局部校验，然后进入 `work_item_draft_confirm`（若 reviewer 开启则继续进入 `work_item_draft_review`）。复用不跳过任何校验或 review 环节。

### 阶段 3B：自动串行生成全部

自动模式。系统按 outline 的拓扑顺序串行自动生成全部 work item，中途不逐个等用户确认、不逐个跑 reviewer。生成全部完成后进入整组确认 + 整组 review。

> **后续扩展（不在本方案范围）**：按 dependency layer 分层、同层无写入冲突项并行生成。并行调度需引入 scope lock 与并发 provider 调用资源控制，当前不做。

自动模式的 per-item author prompt 与串行模式完全相同，都携带完整前序上下文。区别在于自动模式没有逐项人工确认，因此上下文里的“前序 work item”指本轮 batch 中已生成并被调度器接收的 draft records，而不是用户已确认的 accepted records；这些 draft 在整组确认前状态仍为 `draft`。

自动模式的用户确认粒度是整组：

- 接受全部。
- 整组重写。
- 暂停整组。
- 降级为串行模式（仅在最终 strict validator item 级失败后的失败摘要界面提供，常规 batch_confirm 不展示）。

自动模式下，全部 work item 生成完成后先进入整组确认。用户接受全部后，若 reviewer 开启，再进入整组 reviewer 审核；若 reviewer 关闭，直接进入 final compile。Reviewer 审核对象是整组 Work Items，而不是单个 item 的暂停确认点。

Reviewer 审核范围包括：

- 所有 work item 是否整体符合 WorkItemPlan Outline。
- 每个 work item 是否覆盖对应 outline。
- work item 之间的依赖关系是否仍成立。
- verification plans 是否完整且 required gates 合法。
- handoff 链是否支持后续 item 生成（handoff 仅服务 WorkItemPlan 流程内，不进入后续执行阶段）。
- 是否有 work item 明显缺失、跑偏、重复或过粗/过细。

自动模式不支持单个 work item 重写。Reviewer 不通过时，只允许整组重写、带 reviewer finding 整组重写、暂停整组或转人工处理。这样避免”自动生成但局部返修”的半自动状态复杂化。

### 阶段 4：最终编译与严格校验

所有 work item 确认后，后端再把 Draft 结果编译为现有真实结构：

- `IssueWorkItemPlan`
- `LifecycleWorkItemRecord[]`
- `VerificationPlan[]`
- dependency graph
- child workspace sessions

阶段 4 编译必须是可恢复、幂等的短事务。进入 `final_compile` 后先创建 `WorkItemPlanCompileTransaction`，随后所有真实实体创建都绑定同一个 `compile_id`。一旦 transaction 进入 `committing`，`abort` 不再打断持久化提交；若用户发送 abort，只记录为停止请求并在本次提交结束后进入人工处理。

阶段 4 编译步骤：

1. **创建 compile transaction**：写入 `compile_id`、`generation_round_id`、`status=preparing`、`plan_commit_state=not_started`、`active_draft_ids`、当前 Outline version、步骤游标和空的 created ids。
2. **分配稳定 id（内存阶段）**：为每个当前轮次 `accepted && !superseded` 的 active `WorkItemDraftRecord` 分配稳定 `work_item_id` 和 `verification_plan_id`，构建 outline_id → work_item_id 映射表。Draft 阶段不暴露真实 work_item_id。
3. **构建真实结构（内存阶段）**：将 Draft 的 `implementation_context`、`exclusive_write_scopes`、`forbidden_write_scopes`、`verification_plan`、`handoff_summary` 等字段映射为临时 `LifecycleWorkItemRecord[]`、`VerificationPlan[]`、`IssueWorkItemPlan` dependency graph。
4. **运行 strict validator（写真实实体前）**：复用现有 `WorkItemSplitValidator` 的 5 个函数，入参为内存中的真实结构投影。validator 通过前不得写入 `work_items/`、`verification_plans/`、`issue_work_item_plans/` 或 child workspace session。
5. **进入幂等提交**：将 transaction 更新为 `status=committing`，写入确定的 id 映射。后续每一步都先检查 created ids，重复执行同一 `compile_id` 不创建重复实体。
6. **创建真实 WorkItem / VerificationPlan**：写入真实 `LifecycleWorkItemRecord` 与 `VerificationPlan`，并在 transaction 中记录 `created_work_item_ids`、`created_verification_plan_ids`。
7. **创建 child workspace sessions**：为每个 work item 幂等创建 child workspace session，将 Draft 的 `implementation_context` 迁移到 session artifact，并记录 `child_session_ids`。
8. **提交 IssueWorkItemPlan 指针**：更新 `IssueWorkItemPlan.work_item_ids`、`verification_plan_ids`、`dependency_graph`、`status` 与 validator findings，并把 transaction 的 `plan_commit_state` 置为 `committed`。不得在 child workspace sessions 创建完成前提交 plan 指针。
9. **写 commit marker**：将 transaction 更新为 `status=committed`，写入 `committed_at` 和 compile report artifact。刷新恢复时若发现 `committing` transaction，后端必须按步骤游标继续提交；若无法继续，进入人工处理并展示 transaction report。

strict validator 失败处理按失败级别区分（详见错误处理章节）：

- **item 级失败**（某 item 的 verification_plan 内部不合法、该 item 的 scope 与自身依赖冲突等）：
  - 串行模式：定位到具体 work item，执行 downstream invalidation 后从该 item 重新生成。
  - 自动模式：不做局部失效或局部重生成，整组 draft 标记为待返修后回到 `work_item_batch_confirm`，只允许整组重写、暂停、转人工处理，或降级为串行模式重新生成。
    - **降级为串行模式迁移规则**：用户选择降级后，系统以当前 batch 的 `generation_round_id` 进入串行模式。第一个受影响 outline 之前的 batch drafts：复制为新的串行 draft（新 `draft_id`，记录 `copied_from_draft_id`，状态重置为 `draft`），重新运行 `WorkItemDraftLocalValidator`，然后按串行流程进入 `work_item_draft_confirm`（需用户逐个确认；若 reviewer 开启，还需进入 `work_item_draft_review`）。第一个受影响 outline 及之后的 drafts：按串行模式重新生成。已 `superseded` 的 batch drafts 保持只读历史，不参与新串行流程。
- **plan 级失败**（dependency_graph 不一致、id 映射失败、跨 item id 重复、work_item_set_id 不一致等）：
  - 两种模式都：回 Outline 返修或转人工。

## 状态机草案

WorkItemPlan Workspace 新状态建议如下：

| 状态 | 说明 |
| --- | --- |
| `outline_running` | author 正在生成 plan outline |
| `outline_confirm` | 用户确认整体拆分方案 |
| `generation_mode_select` | 用户选择逐个生成或自动生成 |
| `item_running` | 串行模式下正在生成单个 work item |
| `item_confirm` | 串行模式下等待确认单个 work item |
| `item_review` | 串行模式下 reviewer 审核单个 work item |
| `batch_running` | 自动模式下串行自动生成全部 work item |
| `batch_confirm` | 自动模式下等待确认整组 work item |
| `batch_review` | 自动模式下 reviewer 审核整组 work items |
| `final_compile` | 编译为真实 WorkItemPlan 并运行严格校验 |
| `compile_recovery` | 最终编译写入失败或中断后等待人工处理 |
| `human_confirm` | 等待最终人工确认 |
| `completed` | WorkItemPlan 确认完成 |

`generation_mode_select` 作为 `AuthorConfirm` 的扩展分支实现：用户在 Outline review 通过后，AuthorConfirm 节点额外提供"逐个生成"、"自动生成"和"返回 Outline 返修"三个分支入口，不引入独立的新状态机分支。

> **歧义消除**：`generation_mode_select` 节点虽然复用 `WorkspaceStage::author_confirm`，但其交互语义是"选择生成模式或返回 Outline 返修"而非"确认 author 输出"。前端必须根据 `active_node.node_type == work_item_generation_mode` 渲染选择 UI（逐个生成 / 自动生成 / 返回 Outline 返修），不得直接套用通用 author_confirm 的"接受/重写"操作区。后端 handler 必须同时校验 `WorkspaceStage` 与 `active_node.node_type`：
> - 仅在 active node type 为 `work_item_generation_mode` 时才接受 `select_work_item_generation_mode` 消息（mode 为 `serial` 或 `batch`）。
> - 同在 `work_item_generation_mode` 节点下收到 `request_outline_revision` 消息时，将其解释为"返回 Outline 返修"，进入 Outline revision 流程；禁止仅因 `stage == author_confirm` 就接收该消息。
> - 禁止在 `work_item_generation_mode` 节点接受通用 `author_decision`（Accept/Reject），避免与 Outline 确认节点混淆。

Timeline node 建议：

| 节点类型 | 用途 |
| --- | --- |
| `work_item_plan_outline_run` | 生成整体拆分方案 |
| `work_item_plan_outline_confirm` | 确认或重写整体方案 |
| `work_item_plan_outline_review` | 审核整体拆分方案 |
| `work_item_plan_context_blocker` | Outline author 无法补齐上下文时等待人工补充 |
| `work_item_generation_mode` | 选择生成模式 |
| `work_item_draft_run` | 生成单个 work item |
| `work_item_draft_confirm` | 确认或重写单个 work item |
| `work_item_draft_review` | 串行模式下审核单个 work item |
| `work_item_batch_run` | 自动串行生成整组 |
| `work_item_batch_confirm` | 确认或整组重写 |
| `work_item_batch_review` | 自动模式下审核整组 work items |
| `work_item_plan_compile` | 最终编译和严格校验 |
| `work_item_plan_compile_recovery` | 编译写入失败或中断后等待人工处理 |

**状态机实现倾向**：鉴于前端需要区分 outline / item draft / batch 三种完全不同的 UI 形态（逐项气泡、整组队列、编译进度），倾向新增语义化 node type（至少 `work_item_draft_run` / `work_item_draft_review` / `work_item_batch_run` / `work_item_batch_review`），而非复用通用 `AuthorRun` / `ReviewerRun` + metadata。前者让前端按 node type 直接路由 UI 组件，避免 metadata 解析歧义。若复用现有节点类型，必须在 metadata 中明确 `work_item_plan_phase`，且前端需根据该字段二次分发——不推荐。

**与现有实现的衔接**：现有 `WorkspaceStage`（8 个变体，4 种 WorkspaceType 共用）和 `TimelineNodeType`（12 个变体）对四种 WorkspaceType 完全通用，无 WorkItemPlan 专属状态。实现时需评估是否为 WorkItemPlan 引入专属 stage 枚举值，或在现有 stage 上通过 timeline node type 区分。建议优先扩展 timeline node type（影响面小），stage 枚举保持通用，通过 active node type 推导当前 UI 形态。

### Stage / Timeline / WS 契约矩阵

实现时不新增 `WorkspaceStage` 枚举值；WorkItemPlan 专属 UI 形态由 active timeline node type 与 artifact payload 推导。前端不得只依赖 `stage` 判断 WorkItemPlan 面板，应同时读取 `active_node.node_type`。

| 业务阶段 | 复用 `WorkspaceStage` | active timeline node type | 前端主操作 | WS 输入消息 | 关键恢复 payload |
| --- | --- | --- | --- | --- | --- |
| Outline 生成 | `running` | `work_item_plan_outline_run` | 仅展示流式进度、允许 abort | `abort` | `WorkItemPlanOutlineCandidate`、node detail streaming |
| Outline 上下文阻塞 | `human_confirm` | `work_item_plan_context_blocker` | 补充上下文后重跑 / 终止 | `human_confirm` / `request_revision` | `context_blockers`、`design_context_gaps`、已探索摘要 |
| Outline 确认 | `author_confirm` | `work_item_plan_outline_confirm` | 接受 Outline / 重写 Outline / 带反馈重写 | `author_decision` + `request_revision` | current outline artifact、validator findings |
| Outline review | `cross_review` | `work_item_plan_outline_review` | 仅展示 reviewer 进度 | `abort` | review verdict metadata |
| 生成模式选择 | `author_confirm` | `work_item_generation_mode` | 逐个生成 / 自动生成全部 / 回到 Outline 返修 | `select_work_item_generation_mode` / `request_outline_revision` | confirmed outline、selected mode（可空） |
| 单 item 生成 | `running` | `work_item_draft_run` | 展示当前 item 流式进度、允许 abort | `abort` | current outline_id、draft stream |
| 单 item 校验中 | `running` | `work_item_draft_run` | 展示局部校验进度 | 无（后端自动运行） | current draft、`WorkItemDraftLocalValidator` findings |
| 单 item 确认 | `author_confirm` | `work_item_draft_confirm` | 接受当前 item（仅校验通过时可见）/ 重写当前 item / 暂停 | `work_item_draft_decision` | `WorkItemDraftRecord`、accepted draft summaries、local validator findings |
| 单 item review | `cross_review` | `work_item_draft_review` | 仅展示 reviewer 进度；reviewer 结束后自动流转 | `abort`（reviewer 运行中） | review verdict metadata、target_outline_id |
| 自动整组生成 | `running` | `work_item_batch_run` | 展示队列、允许 abort | `abort` | batch queue state、已生成 drafts |
| 自动整组确认 | `author_confirm` | `work_item_batch_confirm` | 接受全部 / 整组重写 / 暂停 / 降级为串行模式（仅在 strict validator item 级失败后） | `work_item_batch_decision` | batch draft list、batch failure summary |
| 自动整组 review | `cross_review` | `work_item_batch_review` | reviewer 通过后自动进入 final_compile；不通过自动回到 batch_confirm 并展示 findings | `abort`（reviewer 运行中） | batch review verdict metadata |
| 最终编译 | `running` | `work_item_plan_compile` | 展示编译和 strict validator 结果 | `abort`（仅 transaction 进入 `committing` 前有效） | compile transaction、compile report、outline_id → work_item_id 映射 |
| 编译恢复 | `human_confirm` | `work_item_plan_compile_recovery` | 继续提交 / 放弃本次 compile（仅 `plan_commit_state=not_started`） / 转人工整理 | `work_item_plan_compile_recovery_action` | compile transaction、transaction report、已创建 ids、允许的操作列表 |
| 最终人工确认 | `human_confirm` | `human_confirm` | 确认完成 / 带反馈返修 / 终止 | `human_confirm` | compiled plan summary、child session ids |
| 完成 | `completed` | `completed` | 查看结果 | 无 | confirmed `IssueWorkItemPlan` |

新增 WS 输入消息：

```json
{
  "type": "select_work_item_generation_mode",
  "mode": "serial|batch"
}
```

```json
{
  "type": "request_outline_revision",
  "feedback": "返回 Outline 返修的反馈"
}
```

```json
{
  "type": "work_item_draft_decision",
  "outline_id": "outline_001",
  "decision": "accept|rewrite|pause",
  "feedback": "optional"
}
```

```json
{
  "type": "work_item_batch_decision",
  "decision": "accept_all|rewrite_batch|pause|downgrade_to_serial",
  "feedback": "optional",
  "first_affected_outline_id": "outline_001"
}
```

```json
{
  "type": "work_item_plan_compile_recovery_action",
  "action": "continue|abort_and_rollback|human_triage",
  "reason": "optional"
}
```

兼容规则：

- `author_decision` 只保留给 Outline author 结果确认，不承载单 item 或 batch 的确认语义。
- `request_revision` 在 WorkItemPlan 中只用于 `work_item_plan_context_blocker` 节点（补充上下文后重跑 Outline）以及通用 revision 路径；在 `work_item_generation_mode` 节点下应使用专用 `request_outline_revision` 消息表示"返回 Outline 返修"。
- `select_work_item_generation_mode` 只能在 active node type 为 `work_item_generation_mode` 时接受，mode 只能为 `serial` 或 `batch`。
- `request_outline_revision` 只能在 active node type 为 `work_item_generation_mode` 时接受；收到后进入 Outline revision 流程，不触发旧整组 revision 路径。
- `work_item_draft_decision` 只能在 active node type 为 `work_item_draft_confirm` 时接受，且 `outline_id` 必须等于当前 active draft；`accept` 仅在当前 draft 已通过局部校验时允许。
- `work_item_batch_decision` 只能在 active node type 为 `work_item_batch_confirm` 时接受；`downgrade_to_serial` 仅在 strict validator item 级失败后的失败摘要界面可用，必须携带 `first_affected_outline_id`。
- `work_item_plan_compile_recovery_action` 只能在 active node type 为 `work_item_plan_compile_recovery` 时接受；`abort_and_rollback` 仅在 `plan_commit_state=not_started` 时允许。
- `human_confirm` 在 `work_item_plan_context_blocker` 节点中只接受补充上下文后的 `request_change` 或终止；不得把 `confirm` 解释为允许带 blocker 继续生成。
- 阶段合法性校验必须从单纯 `WorkspaceStage` 校验升级为 `WorkspaceStage + active timeline node type` 校验；新增 WorkItemPlan 消息不能只因为处于 `author_confirm` 就被接受。
- 刷新恢复时，后端 `SessionState` 必须带回 current outline、draft records、batch queue、active outline_id、accepted draft summaries、compile transaction 与 compile report；前端以这些 payload 重建独立气泡和操作区。

### ReviewComplete / ReviewGate 协议扩展

当前通用 review contract 只有 `pass | revise | needs_human`，不足以表达 WorkItemPlan 的 item/batch/outline reopen 分流。本方案通过 `ReviewComplete` 上的可选子结构承载扩展语义，而不是直接修改共享 `ReviewVerdictType` / `ReviewGate` enum，避免旧 Workspace 数据反序列化问题。

`WorkItemPlanReviewComplete` 子结构（放在 `ReviewComplete.work_item_plan_review` 下，仅 WorkItemPlan 相关 review 节点使用）。代码层面建议定义如下 Rust 类型（供后端 DTO 与前端类型共享）：

````rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewVerdict {
    Pass,
    Revise,
    ReviseBatch,
    NeedsHuman,
    PlanReopenRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewScope {
    Outline,
    Item,
    Batch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewAction {
    Continue,
    ReviseOutline,
    ReviseCurrentItem,
    ReviseBatch,
    HumanTriage,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemPlanReviewGate {
    RequiresCurrentItemRevision,
    RequiresBatchRevision,
    RequiresPlanReopen,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

// ReviewComplete 扩展示例
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

字段说明：

- `verdict`: `pass | revise | revise_batch | needs_human | plan_reopen_required`
  - `revise_batch`：仅 batch review 可返回，表示整组重写。
  - `plan_reopen_required`：item review 或 batch review 可返回，表示当前 Draft 无法局部修复，必须回 Outline 返修或人工处理。
- `review_scope`: `outline | item | batch`，用于前端按 scope 路由 UI。
- `target_outline_id`: 单 item review 必填，batch review finding 可选填。
- `generation_round_id`: 当前 Outline 确认后的生成轮次。
- `draft_id` / `batch_id`: item review 填 `draft_id`，batch review 填 `batch_id`。
- `review_action`: 后端状态机根据该字段决定下一步跳转。
  - `continue`：进入下一个阶段（如 item review 通过后的下一个 item 生成，或 batch review 通过后的 final_compile）。
  - `revise_outline`：触发 Outline reopen 与 draft 失效。
  - `revise_current_item`：串行模式当前 item 重写。
  - `revise_batch`：自动模式整组重写。
  - `human_triage`：进入 `human_confirm` 节点。
- `gates`: 可选的 WorkItemPlan 专属 gate 列表
  - `requires_current_item_revision`：串行模式当前 item 重写。
  - `requires_batch_revision`：自动模式整组重写。
  - `requires_plan_reopen`：触发 Outline reopen 与 draft supersede/downstream invalidation。

兼容策略：

- Story / Design / 普通 WorkItem Workspace 不得产生 `WorkItemPlanReviewComplete`，也不得在通用 `ReviewVerdictType` 中输出 `revise_batch` 或 `plan_reopen_required`。
- 后端 `parse_review_json` 对 WorkItemPlan reviewer run 应优先尝试解析 sentinel block 内的 `WorkItemPlanReviewComplete`；解析失败时降级为通用 `ReviewVerdictType`（`pass/revise/needs_human`），并将未知 verdict 映射为 `needs_human`。
- 前端解析 `ReviewComplete` 时：若存在 `work_item_plan_review`，按 WorkItemPlan 专属逻辑路由；否则走通用 review 逻辑。未知或无法解析的 verdict 一律降级为 `human_triage` 并保留原始文本。
- `plan_reopen_required` 的路由必须先执行 Draft record 清理/失效规则，再进入 Outline 返修或人工节点；不得直接复用旧 `request_revision` 路径跳过 Draft store 维护。

## 数据模型草案

### WorkItemPlanOutline

`WorkItemPlanOutline` 是第一阶段 artifact，不等价于现有 `IssueWorkItemPlan`。

核心字段：

- `id`
- `project_id`
- `issue_id`
- `source_story_spec_ids`
- `source_design_spec_ids`
- `strategy_summary`
- `work_item_outlines`
- `dependency_graph`
- `risks`
- `handoff_strategy`
- `status`

> `parallel_groups` 为后续并行生成扩展预留，当前自动模式仅串行，不产出该字段。

### WorkItemOutline

核心字段：

- `outline_id`
- `title`
- `kind`
- `goal`
- `scope`
- `non_goals`
- `source_story_spec_ids`
- `source_design_spec_ids`
- `exclusive_write_scopes`（预期写入边界，Draft 阶段同名字段继承）
- `forbidden_write_scopes`
- `depends_on`（引用其他 outline_id）
- `verification_intent`
- `handoff_notes`

### WorkItemDraftCandidate

第二阶段生成的单个 work item 候选。

核心字段：

- `outline_id`（Draft 阶段的业务关联字段，真实 work item id 由阶段 4 分配）
- `title`
- `kind`
- `goal`
- `implementation_context`
- `exclusive_write_scopes`（继承自 Outline，author 可细化）
- `forbidden_write_scopes`
- `depends_on_outline_ids`（引用 outline_id，编译时映射为 work_item_id）
- `required_handoff_from_outline_ids`（引用 outline_id）
- `handoff_summary`（预期交付摘要，供后序 item prompt 使用）
- `verification_plan`

`status`、`generated_from_node_id`、`accepted_at`、`superseded_at` 等状态字段属于 `WorkItemDraftRecord`，由后端设置；provider 结构化输出不得包含这些后端状态字段。

> **命名统一**：Outline 与 Draft 都用 `exclusive_write_scopes` / `forbidden_write_scopes`，体现"Outline 预期 → Draft 继承细化"的语义递进。
>
> **review_verdict 存储**：Draft 的 review verdict 不作为 `WorkItemDraftCandidate` 字段，存在对应 `work_item_draft_review` timeline node 的 metadata 里，避免污染 Record。前端刷新恢复时从 timeline node 读取 review 状态。

### WorkItemDraftRecord

`WorkItemDraftRecord` 是 Draft 阶段的持久化记录，不等价于真实 `LifecycleWorkItemRecord`。它的存在是为了避免 Draft 阶段用占位 `id` 写入 work item store 后，阶段 4 再重命名 JSON 文件、重写 `depends_on`、重写 `VerificationPlan.work_item_id`、重建 artifact 与 session 引用。

核心字段：

- `project_id`
- `issue_id`
- `plan_id`
- `draft_id`（文件名与不可变主键）
- `outline_id`（业务关联字段，不作为唯一文件名）
- `generation_round_id`（Outline 每次确认后生成新的 round）
- `attempt_index`（同一 round + outline 下的第几次生成/重写）
- `outline_version_ref`（生成该 draft 时使用的 Outline artifact/version）
- `generation_mode`（serial / batch）
- `candidate`（完整 `WorkItemDraftCandidate`）
- `status`（draft / accepted / superseded）
- `active`（是否为当前 round 中该 outline 的可用版本）
- `superseded_by_draft_id`
- `copied_from_draft_id`
- `review_node_id`
- `review_verdict_ref`
- `generated_from_node_id`
- `accepted_at`
- `superseded_at`
- `created_at`
- `updated_at`

`WorkItemDraftStatus` 仅用于 Draft store，不扩展现有 `WorkItemPlanStatus`。现有 `WorkItemPlanStatus` 继续只描述真实 work item 在 plan 中的状态。

### Draft 持久化方式

`WorkItemDraftCandidate` 使用新 Draft store 持久化：

- 存储路径建议：`.aria/projects/<project>/issues/<issue>/work_item_plan_drafts/<plan_id>/<generation_round_id>/<draft_id>.json`。
- active index 建议：`.aria/projects/<project>/issues/<issue>/work_item_plan_drafts/<plan_id>/active_index.json`，记录 `current_generation_round_id`、`outline_id -> current_draft_id`、`draft_id -> status`。
- 阶段 3（Draft）：创建或更新 `WorkItemDraftRecord`，不写入 `work_items/`，不写入 `verification_plans/`，不创建 child workspace session。
- 阶段 3 确认：只把对应 Draft record 的 `status` 改为 `accepted`，并记录确认 node 与时间。
- 阶段 3 返修：创建新的 immutable Draft record；旧 draft 标记 `superseded`、`active=false`，`superseded_by_draft_id` 指向新 draft。历史 timeline detail 引用旧 `draft_id`，因此刷新后仍可回放旧内容。
- 阶段 4（编译）：只读取 active index 指向的 `accepted && active && !superseded` Draft record，分配真实 `work_item_id` 和 `verification_plan_id`，写入现有 `work_items/`、`verification_plans/`、`issue_work_item_plans/`，创建 child workspace session。

`outline_id` 允许在 Outline 返修后复用，但每次 Outline 确认都必须产生新的 `generation_round_id`。如果用户选择复用旧 draft，后端不能把旧 record 原地改回 active，而是复制为当前 round 的新 `draft_id`，记录 `copied_from_draft_id`，并重新执行局部校验和最终 strict validator。

现有 `LifecycleWorkItemRecord.work_item_set_id` 仍用于真实 WorkItem 编译结果的归组，不再承担 Draft 阶段关联职责。

### WorkItemPlanCompileTransaction

`WorkItemPlanCompileTransaction` 是阶段 4 的恢复锚点，用来避免真实实体写到一半时刷新、abort 或进程崩溃留下不可判定状态。

核心字段：

- `compile_id`
- `project_id`
- `issue_id`
- `plan_id`
- `generation_round_id`
- `outline_version_ref`
- `active_draft_ids`
- `status`（preparing / validating / committing / committed / failed / recovery_required）
- `plan_commit_state`（not_started / committed）
- `step_cursor`
- `outline_to_work_item_id`
- `outline_to_verification_plan_id`
- `created_work_item_ids`
- `created_verification_plan_ids`
- `child_session_ids`
- `validator_findings`
- `abort_requested_at`
- `failure_reason`
- `previous_plan_snapshot`（可选，仅当后续实现决定支持 plan 指针提交后的自动回退时使用）
- `created_at`
- `updated_at`
- `committed_at`

存储路径建议：`.aria/projects/<project>/issues/<issue>/work_item_plan_compiles/<plan_id>/<compile_id>.json`。

恢复规则：

- `preparing` / `validating`：可安全放弃或重新开始同一 round compile。
- `committing`：必须按 `step_cursor` 和 created ids 幂等继续，不允许创建第二组真实实体。若 `plan_commit_state=not_started`，仍可在人工确认后清理 `created_*_ids` 并回到确认节点；若 `plan_commit_state=committed`，不得自动清理回退，只能继续幂等补齐 commit marker 或进入人工整理。
- `committed`：作为最终 compile report 的来源。
- 写入失败（非 crash）：compile transaction 进入 `committing` 后，若某一步写入失败（如磁盘错误、文件锁），后端捕获异常，将 transaction status 更新为 `recovery_required`，记录失败步骤、失败原因与当前 `step_cursor`，创建 `work_item_plan_compile_recovery` timeline node（复用 `WorkspaceStage::human_confirm`）。前端在该节点展示 transaction report 与允许的操作列表。用户通过 `work_item_plan_compile_recovery_action` 消息选择：
  - `continue`：修复环境问题后继续按 `step_cursor` 幂等执行。
  - `abort_and_rollback`：仅当 `plan_commit_state=not_started` 时允许；后端按 transaction 中 `created_*_ids` 清理已创建实体，将 transaction 标记为 `failed`，回到 `work_item_batch_confirm` 或 `work_item_draft_confirm`。
  - `human_triage`：当 `plan_commit_state=committed` 或环境无法恢复时选择；后端锁定 plan，进入通用 `human_confirm` 节点等待人工整理。
  - 若未来需要支持 `plan_commit_state=committed` 后自动回退，必须在 transaction 中保存 `previous_plan_snapshot`，并先恢复 `IssueWorkItemPlan` 指针后再清理 `created_*_ids`。
- `failed` / `recovery_required`：进入 `work_item_plan_compile_recovery` 节点或通用 `human_confirm` 节点，展示 transaction report 与已创建 ids。

### OutlineContextIndex

`OutlineContextIndex` 是阶段 1 的辅助索引，用于在多次 `work_item_plan_context_blocker` 节点后快速定位用户补充的上下文，避免每次 Outline author run 都扫描全量 timeline。

存储路径建议：`.aria/projects/<project>/issues/<issue>/work_item_plan_outlines/<plan_id>/outline_context_index.json`。

核心字段：

````json
{
  "plan_id": "plan_001",
  "generation_round_id": "round_001",
  "blocker_resolutions": [
    {
      "blocker_node_id": "node_outline_blocker_001",
      "resolution_node_id": "node_human_confirm_002",
      "resolution_artifact_ref": "context_blocker_resolution://<blocker_node_id>/<resolution_node_id>",
      "created_at": "2026-06-22T10:00:00Z"
    }
  ],
  "design_context_gaps": ["missing_test_strategy", "missing_key_paths"],
  "design_context_capabilities": {
    "has_architecture": true,
    "has_module_breakdown": true,
    "has_tech_stack": false,
    "has_test_strategy": false,
    "has_key_paths": false
  },
  "updated_at": "2026-06-22T10:00:00Z"
}
````

更新规则：

- 每次 Outline author run 前，后端读取 `outline_context_index.json` 并将 `blocker_resolutions` 与 `design_context_gaps` 注入 prompt。
- 用户补充上下文后，后端创建 `context_blocker_resolution` artifact 并追加到 `blocker_resolutions`。
- 每次 Outline 返修并产生新的 `generation_round_id` 时，复制上一份 `outline_context_index.json` 作为新 round 的起点，确保历史 blocker resolution 不丢失。
- `outline_context_index.json` 是只读索引，不保存 Draft 阶段状态；Draft active index 仍由 `active_index.json` 维护。

### 数据流转图（Outline → Draft → 编译后真实结构）

| 阶段 | 实体 | 关键字段 | → 编译后映射 |
| --- | --- | --- | --- |
| 阶段 1 Outline | `WorkItemPlanOutline` | `dependency_graph`（outline_id 边） | → `IssueWorkItemPlan.dependency_graph`（经 id 映射为 work_item_id 边） |
| 阶段 1 Outline | `WorkItemOutline` | `outline_id`、`exclusive_write_scopes` | → `WorkItemDraftRecord.outline_id`、最终 `LifecycleWorkItemRecord.exclusive_write_scopes` |
| 阶段 3 Draft | `WorkItemDraftRecord` | `draft_id`、`outline_id`、`status`、`candidate` | → 阶段 4 只读取 active index 指向的 `accepted` Draft |
| 阶段 3 Draft | `WorkItemDraftCandidate` | `implementation_context`、`verification_plan`、`handoff_summary` | → child workspace session artifact + `VerificationPlan` |
| 阶段 3 Draft | `WorkItemDraftCandidate.depends_on_outline_ids` | outline_id 列表 | → `LifecycleWorkItemRecord.depends_on`（work_item_id 列表，经映射） |
| 阶段 4 编译 | `IssueWorkItemPlan` | `work_item_ids`、`verification_plan_ids`、`dependency_graph` | 最终真实结构，供 strict validator 与下游 Workspace 使用 |

### Repository Profile 兼容策略

`repository_profile` 退出 WorkItemPlan 新流程，但不在本方案的首个实现批次中全局删除 legacy 字段，避免影响旧候选数据、HTTP DTO 和现有测试夹具。

- 新两阶段 WorkItemPlan 流程不得要求 provider 输出 `repository_profile`。
- 新两阶段 WorkItemPlan 编译结果中，`IssueWorkItemPlan.repository_profile_ref = None`。
- 新两阶段 WorkItemPlan 生成的 `VerificationPlan.repository_profile_ref = None`；字段保留为兼容 legacy / 非 WorkItemPlan 调用方。
- `WorkItemPlanCandidateDto.repository_profile` 对新 payload 不再作为必需字段；旧 `WorkItemPlanCandidateDto` 展示编译后 legacy candidate 时仍可携带。
- strict validator 在新流程中以 `plan.repository_profile_ref = None` 且 `repository_profile = None` 调用，不得产生 `repository_profile_missing` 或 `repository_profile_low_confidence` finding。
- 若后续单独做模型清理，再移除 `IssueWorkItemPlan.repository_profile_ref`、`VerificationPlan.repository_profile_ref`、相关 DTO 字段和旧 validator 分支；该清理不与本方案首个实现批次混在一起。

### WS Artifact Payload 扩展

WorkItemPlan 两阶段流程不再只依赖当前整组 `WorkItemPlanCandidateDto`。WebSocket `ArtifactPayload` 需要新增或拆分以下 payload，供刷新恢复和右侧 Artifact 面板渲染：

| Payload | 用途 | 关键字段 |
| --- | --- | --- |
| `WorkItemPlanOutlineCandidate` | 展示阶段 1 Outline | `outline`、`design_context_gaps`、`validator_findings`、`context_blockers` |
| `WorkItemPlanContextBlocker` | 展示不可自动恢复的上下文缺口 | `context_blockers`、`design_context_gaps`、`exploration_summary`、`allowed_actions` |
| `WorkItemDraftCandidate` | 展示串行模式当前/历史单 item draft | `record`、`candidate`、`review_status`、`target_outline_id` |
| `WorkItemBatchState` | 展示自动模式队列与整组结果 | `mode=batch`、`queue[]`、`draft_records[]`、`batch_status`、`failure_summary` |
| `WorkItemPlanCompileReport` | 展示阶段 4 编译结果 | `compile_id`、`transaction_status`、`outline_to_work_item_id`、`created_work_item_ids`、`created_verification_plan_ids`、`child_session_ids`、`validator_findings` |

兼容策略：

- 旧 `WorkItemPlanCandidateDto` 可以作为阶段 4 编译后的展示 DTO 保留，但不再作为 Draft 阶段的唯一 artifact。
- 前端 `workspace-ws-store` 应保留当前 WorkItemPlan artifact 的 discriminated union，而不是只保存 `workItemPlanCandidate` 单字段。
- 刷新恢复时，后端必须从 timeline node detail + Draft store + compile report 重建当前 payload 与 artifact history index；不能只依赖最后一条 `artifact_update`。

### Artifact 版本与结构化 Diff 展示

WorkItemPlan Workspace 必须像 Story / Design Workspace 一样支持用户查看历史 artifact 版本和版本变动，但 WorkItemPlan 的 artifact 是结构化对象，不应只套用 Markdown diff。

UI 壳层应尽量复用 Story / Design Workspace 已有的版本历史体验：左侧 timeline 负责定位过程节点，右侧 Artifact 面板负责展示该节点的版本内容、版本列表与对比。WorkItemPlan 的差异在于右侧 renderer 和 diff model 必须按 artifact kind 分派，而不是把所有内容降级为一份 Markdown。

右侧 Artifact 面板的默认规则：

- 默认展示当前 active timeline node 对应的 artifact。
- 用户点击左侧 timeline 历史节点时，右侧切换到该节点当时的 artifact。
- 历史 Draft 即使已 `superseded`，也必须能按 `draft_id` 回放完整内容。
- 当前 active artifact、历史 artifact 与最终 compiled entity 必须在 UI 上明确区分，避免用户误以为 Draft 已经是正式 WorkItem。

`SessionState` 或 WorkItemPlan WS DTO 应提供稳定的 `artifact_versions[]` 索引，或提供足够字段让前端确定性重建该索引。每个 version entry 至少包含：

| 字段 | 说明 |
| --- | --- |
| `artifact_version_ref` | 稳定引用，建议包含 `kind`、`version_id`、`source_node_id` |
| `artifact_kind` | outline / context_blocker_resolution / draft / batch_snapshot / compile_report / final_candidate |
| `source_node_id` | 生成该版本的 timeline node，用于 timeline 点击后定位右侧 artifact |
| `status` | active / accepted / superseded / copied / compiled / recovery_required |
| `generation_round_id` | 所属生成轮次；legacy final candidate 可为空但必须有正式实体 id |
| `entity_ref` | `outline_version_ref`、`draft_id`、`batch_id`、`compile_id` 或 `IssueWorkItemPlan.id` |
| `payload_ref` | 指向 timeline node detail、Draft store record、compile report 等真实 payload 来源 |
| `display_title` | 人类可读标题，仅用于展示，不得作为类型判断依据 |
| `review_summary` | reviewer verdict 摘要，可为空 |
| `validator_summary` | validator findings 摘要，可为空 |
| `relations` | `superseded_by_draft_id`、`copied_from_draft_id`、`compiled_entity_refs` 等关系 |

Artifact 历史版本入口建议采用三段式：

- `当前内容`：展示 active node 或用户选中节点的 artifact。
- `历史版本`：按 artifact kind 分组列出所有版本。
- `对比`：支持与上一版对比，以及选择两个同类型版本对比。

历史版本列表至少覆盖以下对象：

| Artifact 类型 | 版本粒度 | 关键标识 |
| --- | --- | --- |
| Outline | 每次 Outline author / revision 产出一个版本 | `outline_version_ref`、`source_node_id`、`generation_round_id` |
| Context blocker resolution | 每次用户补充上下文产出一个版本 | `blocker_node_id`、`resolution_node_id`、`created_at` |
| Draft | 同一 outline 每次生成 / 重写 / 复用产出一个版本 | `draft_id`、`outline_id`、`attempt_index`、`copied_from_draft_id` |
| Batch snapshot | 自动模式每轮 batch run / batch rewrite 产出一个快照 | `batch_id`、`generation_round_id`、`draft_ids[]` |
| Compile report | 每次 final compile transaction 产出一个报告 | `compile_id`、`transaction_status`、`plan_commit_state` |
| Final candidate（legacy） | 编译后兼容旧展示的整组候选 | `IssueWorkItemPlan.id`、`work_item_ids[]` |

每条历史版本记录应展示：

- artifact kind 与人类可读标题。
- 版本号或 attempt 序号。
- 生成来源 timeline node。
- 当前状态：active / accepted / superseded / copied / compiled / recovery_required。
- reviewer verdict、validator findings 摘要。
- 替代关系：`superseded_by_draft_id`、`copied_from_draft_id`。

结构化 Diff 规则：

| Diff 类型 | 对比内容 |
| --- | --- |
| Outline diff | `work_item_outlines` 新增 / 删除 / 修改、dependency edge 变化、write scope 变化、traceability refs 变化、verification intent 变化 |
| Context resolution diff | 用户补充内容变化、关联 blocker 变化、是否被后续 Outline run 消费 |
| Draft diff | `goal`、`implementation_context`、write scopes、depends_on / required_handoff、`handoff_summary`、verification plan commands / manual checks / required gates 变化 |
| Batch diff | draft 新增 / superseded / copied / retained、queue 状态变化、batch failure summary、batch review findings 变化 |
| Compile report diff | outline_id → work_item_id 映射变化、created work item / verification plan / child session ids、transaction 状态、validator findings 变化 |

Diff 请求与响应约束：

- `ArtifactDiffRequest` 只接受两个 `artifact_version_ref`，后端或前端 diff 层按 ref 解析真实 payload，不接受标题字符串或当前 UI index 作为输入。
- 默认只允许同 `artifact_kind` 版本互相比对；跨类型对比入口置灰并解释原因。若后续确需支持 Draft → Compile report 的追踪视图，应单独建 `traceability view`，不混入普通 diff。
- `ArtifactDiffResult` 使用 typed union：`OutlineDiff`、`ContextResolutionDiff`、`DraftDiff`、`BatchDiff`、`CompileReportDiff`。前端 renderer 按 union tag 渲染字段级变化。
- 任意一侧 payload 缺失时，展示可恢复错误：缺少的 `artifact_version_ref`、期望 payload 来源、允许用户刷新或进入人工处理；不得静默回退到最新 artifact。

实现约束：

- 结构化 Diff 是 WorkItemPlan 的主展示方式；Markdown diff 只能作为 fallback 或原始 JSON/Markdown 查看入口。
- 前端不得只保留最后一个 WorkItemPlan artifact；`workspace-ws-store` 需要保存可查询的 artifact history index，或能从 `SessionState.artifact_versions`、timeline node detail、Draft store payload 重建该 index。
- `superseded` 历史版本只读展示，不提供接受或重写操作；用户若要复用旧 draft，必须走方案中定义的“复制为当前 `generation_round_id` 下的新 `draft_id`”流程。
- Compile report 是正式落盘报告，不参与 Draft 重写；如果 compile 失败进入 `recovery_required`，Artifact 面板展示 transaction report 和允许的人工操作。

## Prompt 设计要求

### Prompt 契约总则

所有 WorkItemPlan 相关 provider prompt 必须遵守以下通用契约：

- provider 可以在最终结构化输出前输出简短可读进度，供 Workbench 流式展示。
- provider 长时间探索、读取代码、分析依赖或准备重写前，必须先输出一句可读状态。
- provider 每完成一组探索或推理，应输出一句当前发现摘要，避免页面长时间无反馈。
- provider 不得修改仓库文件，不得执行实现，不得创建计划文档，不得进入后续执行阶段。
- provider 的机器可解析结果必须放在最后一个 `<ARIA_STRUCTURED_OUTPUT>...</ARIA_STRUCTURED_OUTPUT>` sentinel block 内。
- sentinel block 内只能是完整 JSON object，不允许 Markdown code fence，不允许注释，不允许尾随解释。
- 后端只解析最后一个 sentinel block；sentinel block 之前的可读内容只用于 UI 展示。
- prompt 必须明确当前阶段、允许输出的对象、禁止输出的对象，以及失败时允许的 rewrite 范围。
- **所有 reviewer prompt（Outline reviewer、单 item reviewer、自动模式整组 reviewer）也必须使用 sentinel structured block 输出最终结构化 verdict**，不再使用 markdown JSON fence。当前代码中 reviewer prompt 仍要求 markdown fence（`src/product/workspace_engine.rs`），本方案首个实现批次中需同步改造。
- **迁移路径**：由于 Story / Design / 普通 WorkItem reviewer 也共用 `parse_review_json`，建议本次统一将 reviewer 输出与解析路径迁移到 sentinel block，不再区分 WorkspaceType。旧 reviewer 输出（markdown fence）在新代码中可降级解析一个版本，但新 prompt 必须要求 sentinel block。
- reviewer sentinel block 的 schema 为 `WorkItemPlanReviewComplete`（WorkItemPlan 场景）或通用 `ReviewVerdict`（其他 WorkspaceType），后端按 active workspace type 选择解析目标。

### Outline Author Prompt

Outline author prompt 的任务是生成 WorkItemPlan Outline，即“如何编写 Work Items 的蓝图”。它不得生成完整 Work Item。

必须输入：

- Issue 标题、描述和约束。
- 已确认 Story Spec markdown。
- 已确认 Design Spec markdown。
- repository 路径与仓库结构摘要。
- 用户拆分选项。
- 如果是返修，输入 reviewer findings 与用户补充反馈。

必须输出：

- `strategy_summary`：整体拆分策略。
- `work_item_outlines[]`：每个 work item 的大纲。
- `dependency_graph[]`：outline 之间的依赖边。
- `handoff_strategy`：后续生成 work item 时如何传递上下文。
- `risks[]`：拆分风险与需要用户关注的点。

必须禁止：

- 禁止输出完整 `LifecycleWorkItemRecord`。
- 禁止输出完整 `VerificationPlan`。
- 禁止输出最终 work item id；只能使用稳定 `outline_id`。
- 禁止输出具体验证命令清单；只能输出验证意图。
- 禁止创建 child work item session。
- 禁止生成具体实现步骤或代码编写步骤。
- 禁止输出 `repository_profile`（退出 WorkItemPlan 新流程，仓库结构知识来自 Design spec + CLAUDE.md + 目录探索）。
- 禁止输出 `parallel_groups`（并行生成为后续扩展，当前不做）。

author 探索能力：prompt 允许 author 读取 CLAUDE.md（项目技术栈章节）和目标仓库的目录结构（只读，不得修改文件），作为 Design spec 架构章节的补充。探索所得不作为 Outline 持久化字段，仅用于本次 prompt 上下文。

如果上下文不足以安全生成 Outline，author 必须返回 `context_blockers[]`，并可以省略 `outline` 或返回空 outline。后端收到非空 `context_blockers` 时不得进入 `outline_confirm`。

结构化输出要点：

```json
{
  "context_blockers": [],
  "outline": {
    "strategy_summary": "...",
    "work_item_outlines": [
      {
        "outline_id": "outline_001",
        "title": "...",
        "kind": "backend|frontend|integration|e2e|docs|infra|other",
        "goal": "...",
        "scope": ["..."],
        "non_goals": ["..."],
        "source_story_spec_ids": ["story_spec_0001"],
        "source_design_spec_ids": ["design_spec_0001"],
        "exclusive_write_scopes": ["src/web/..."],
        "forbidden_write_scopes": ["..."],
        "depends_on": ["outline_000"],
        "verification_intent": "...",
        "handoff_notes": "..."
      }
    ],
    "dependency_graph": [
      { "from_outline_id": "outline_001", "to_outline_id": "outline_002" }
    ],
    "handoff_strategy": "...",
    "risks": ["..."]
  }
}
```

### Outline Reviewer Prompt

Outline reviewer prompt 的任务是审核 WorkItemPlan Outline 是否足以作为后续 Work Item 生成蓝图。Reviewer 不得要求完整 Work Item 内容，因为该阶段尚未生成完整 Work Item。

审核范围：

- 是否覆盖 Story/Design 的关键需求和设计约束。
- work item 粒度是否合理，是否过粗、过细、遗漏或重复。
- dependency graph 是否合理、无环、无明显缺口。
- exclusive/forbidden write scopes 是否足以指导后续生成，是否存在明显写入边界冲突。
- handoff_strategy 是否能防止后续 prompt 丢上下文。

禁止 reviewer 在 Outline 阶段要求：

- 完整 verification plan。
- `required_gates`。
- 具体命令 id。
- 完整实现或执行计划。
- `repository_profile`。

Reviewer verdict schema：

```json
{
  "verdict": "pass|revise|needs_human",
  "summary": "...",
  "findings": [
    {
      "severity": "blocking|must_fix|strong_recommend_fix|suggestion|minor|optional",
      "target_outline_id": "outline_001",
      "message": "...",
      "evidence": "...",
      "impact": "...",
      "required_action": "..."
    }
  ]
}
```

### 单个 Work Item Author Prompt

单个 Work Item author prompt 的任务是根据一个已确认 outline 生成一个完整 Work Item draft。每次 provider run 只能生成一个 work item。

必须输入：

- 已确认 WorkItemPlan Outline 完整内容。
- 当前 `WorkItemOutline` 完整内容。
- 当前生成模式：serial 或 batch。
- 用户对当前 item 的反馈。
- reviewer findings，如果本次是重写。
- 已确认前序 work item 的摘要。
- 当前 item 直接依赖的 work item 完整内容。
- 非直接依赖 work item 的压缩摘要。
- 已确认前序 work item 的写入边界、verification plan 摘要和 handoff。

上下文优先级：

1. 用户反馈和 reviewer findings。
2. 当前 WorkItemOutline。
3. 已确认 WorkItemPlan Outline。
4. 当前 item 直接依赖项的完整内容。
5. 其他已确认 work item 的摘要。
6. Story/Design 原文。

必须禁止：

- 禁止生成多个 work item。
- 禁止重写已确认 work item。
- 禁止改变 WorkItemPlan Outline 的依赖图。
- 禁止超出当前 outline 的写入边界。
- 禁止输出 `work_item_id`（阶段 4 编译时由后端分配，Draft 阶段只用 `outline_id`）。
- 禁止输出 `repository_profile`。
- 禁止把自然语言验收条件写入 `required_gates`。
- 禁止输出 `handoff_summary` 之外的预期交付后信息（handoff 是预期交付摘要，不是实际执行后的 diff 摘要；后续执行阶段不消费该 handoff）。

必须输出完整 `WorkItemDraftCandidate`，包括完整 verification plan。`required_gates` 规则：

- 每个 command 必须有稳定 `id`，例如 `cmd_fmt`、`cmd_check`、`cmd_clippy`。
- 每个 manual check 必须有稳定 `id`，例如 `manual_browser_check`。
- `required_gates` 只能引用同一个 verification plan 内已定义的 command/manual_check id。
- `required_gates` 禁止包含自然语言，例如“cargo test 全绿”“手动检查通过”。
- 如果一个 gate 没有对应 command/manual_check，必须先创建 command/manual_check，再引用其 id。

> **现有 validator 覆盖度**：现有 `WorkItemSplitValidator::validate_verification_commands`（`work_item_split_validator.rs:489`）已校验规则 1/2/4（`required_gates` 引用的 id 必须在 `commands`/`manual_checks` 集合内）。规则 3 因 `required_gates: Vec<String>` 数据类型天然不会出现自然语言 gate（任何不匹配 id 的字符串都会被规则 2 拦下）。本节"写死"是对现有校验的确认，不是新增校验逻辑。

结构化输出要点：

```json
{
  "work_item": {
    "outline_id": "outline_001",
    "title": "...",
    "kind": "backend|frontend|integration|e2e|docs|infra|other",
    "goal": "...",
    "implementation_context": "...",
    "exclusive_write_scopes": ["..."],
    "forbidden_write_scopes": ["..."],
    "depends_on_outline_ids": ["outline_000"],
    "required_handoff_from_outline_ids": ["outline_000"],
    "handoff_summary": "...",
    "verification_plan": {
      "scope": "unit|integration|e2e|manual|mixed",
      "commands": [
        {
          "id": "cmd_check",
          "label": "cargo check",
          "command": "cargo check --locked",
          "cwd": ".",
          "purpose": "...",
          "required": true,
          "timeout_seconds": 300,
          "safety": "approved|needs_review"
        }
      ],
      "manual_checks": [
        {
          "id": "manual_ui_check",
          "label": "UI smoke check",
          "instructions": "...",
          "required": false
        }
      ],
      "required_gates": ["cmd_check"],
      "risk_notes": ["..."],
      "confidence": "high|medium|low",
      "fallback_policy": "manual_gate|skip_allowed|block"
    }
  }
}
```

### 单个 Work Item Reviewer Prompt

单个 Work Item reviewer prompt 的任务是审核当前 work item 是否可以作为后续 item 和下游 Workspace 的稳定上下文。

审核范围：

- 当前 work item 是否符合对应 outline。
- 是否正确吸收直接依赖项的 handoff。
- 是否错误修改或覆盖已确认前序 work item。
- 写入边界是否与前序已确认 item 冲突。
- verification plan 是否完整、可执行。
- `required_gates` 是否只引用本 verification plan 内的 command/manual_check id。
- 当前 work item 是否暴露足够 handoff 给后续 item。

允许 verdict：

- `pass`：当前 work item 可锁定并进入下一个。
- `revise`：只重写当前 work item。
- `needs_human`：需要用户判断当前 item。
- `plan_reopen_required`：发现 WorkItemPlan Outline 本身错误，必须回到 Outline 返修或人工决策。

`plan_reopen_required` 只能用于当前 item 无法在不改变整体拆分、依赖或边界的前提下修复的问题；不得用于普通局部质量问题。

Reviewer verdict schema（对应 `WorkItemPlanReviewComplete` 的内容，实际嵌套在 `ReviewComplete.work_item_plan_review` 下）：

```json
{
  "verdict": "pass|revise|needs_human|plan_reopen_required",
  "summary": "...",
  "target_outline_id": "outline_001",
  "findings": [
    {
      "severity": "blocking|must_fix|strong_recommend_fix|suggestion|minor|optional",
      "message": "...",
      "evidence": "...",
      "impact": "...",
      "required_action": "..."
    }
  ]
}
```

### 自动模式 Prompt 与调度

自动模式不是一个大 prompt 生成全部 work item，而是按 WorkItemPlan Outline 的拓扑顺序串行多次调用"单个 Work Item Author Prompt"。

调度规则：

- 按 outline 拓扑顺序逐个生成，每个 provider run 只生成一个 work item。
- 生成 item N+1 时，prompt 必须包含 item N 的完整生成结果（implementation_context、handoff_summary、verification_plan 摘要、写入边界）。
- 自动模式下任何局部失败都会标记 batch 为待处理，但产品操作只允许整组重写、暂停或转人工。
- 整组重写时必须清空本轮全部 draft，用 batch reviewer findings 重新调度。

自动模式的 per-item prompt 与串行单 item prompt 使用同一模板，但上下文命名不同：串行模式携带“前序已确认 item”，自动模式携带“当前 batch 中前序已生成并被调度器接收的 draft records”。两者都必须携带直接依赖完整内容、前序 handoff_summary、写入边界和验证约束。区别只在调度层面：自动模式不等用户逐个确认、不逐个跑 reviewer，全部生成完后整组确认 + 整组 review。

> **后续扩展（不在本方案范围）**：按 dependency layer 分层、同层无写入冲突项并行生成。并行调度需引入 scope lock 与并发 provider 调用资源控制。届时 per-item prompt 将额外输入 `batch_generation_id`、当前 dependency layer、同层并行 item 列表、batch 级反馈。当前自动模式不产出这些字段。

### 自动模式整组 Reviewer Prompt

自动模式整组 reviewer prompt 的任务是审核全部 Work Items 作为一个集合是否符合已确认 WorkItemPlan Outline。它不得要求单个 item 局部重写。

审核范围：

- 所有 outline 是否都有对应 work item。
- 每个 work item 是否覆盖对应 outline。
- dependency graph 是否仍成立。
- verification plans 是否完整且 `required_gates` 引用合法。
- handoff 链是否支持后续 item 生成（handoff 仅服务 WorkItemPlan 流程内，不进入后续执行阶段）。
- 是否存在重复、遗漏、跑偏、过粗或过细。

允许 verdict：

- `pass`：整组可进入最终编译。
- `revise_batch`：整组重写。
- `needs_human`：需要用户人工判断。
- `plan_reopen_required`：发现 Outline 本身错误，必须回到 Outline 返修或人工决策。

Reviewer verdict schema（对应 `WorkItemPlanReviewComplete` 的内容，实际嵌套在 `ReviewComplete.work_item_plan_review` 下）：

```json
{
  "verdict": "pass|revise_batch|needs_human|plan_reopen_required",
  "summary": "...",
  "findings": [
    {
      "severity": "blocking|must_fix|strong_recommend_fix|suggestion|minor|optional",
      "target_outline_id": "outline_001",
      "message": "...",
      "evidence": "...",
      "impact": "...",
      "required_action": "..."
    }
  ]
}
```

### Rewrite Prompt 规则

不同阶段的 rewrite prompt 必须严格限制重写范围：

- Outline 返修：只能重写整个 WorkItemPlan Outline，不生成完整 work item。
- 串行单 item 返修：只能重写当前 work item，不修改已确认前序 item，不修改 Outline。
- 自动模式整组返修：清空本轮全部 work item draft，按原 Outline 或返修后的 Outline 重新调度整组生成。
- `plan_reopen_required`：停止当前 item/batch 生成，进入 Outline 返修或人工决策，不能在当前 item prompt 中自行修改 Outline。

`plan_reopen_required` 触发后的 Draft records 处理规则：

- **已生成但未确认的 Draft records**：不得物理删除。标记 `superseded`、`active=false`，从 active index 移除；timeline 历史通过 `draft_id` 仍可回放。
- **已确认的 Draft records**：默认标记 `superseded`、`active=false`，保留在 Draft store 与 timeline 历史中供回溯，但不参与下一轮生成和阶段 4 编译。
- **downstream invalidation**：若 `plan_reopen_required` 或最终 strict validator 指向某个 outline，后端必须计算 Outline dependency graph 中该 outline 的所有下游 active drafts，并一起标记 `superseded`。旁路且不依赖该 outline 的 draft 可保持 active，但必须在 UI 中标注“未受影响”依据。
- **Outline 返修后的重新生成范围**：Outline 返修可能改变 outline 列表、依赖图或写入边界。返修通过后，所有 `superseded` draft 对应的 outline 默认重新生成；新增或修改的 outline 强制重新生成；未变化且未被 downstream invalidation 命中的 outline 若用户选择复用旧 draft，后端需复制旧 draft 为当前 `generation_round_id` 下的新 `draft_id`，再重新执行局部校验和阶段 4 strict validator。
- **复用未受影响旁路 draft 的 UI 与后端流程**：返修后的 Outline 与旧版差异较小时，后端在 Outline 返修通过后的"重新生成准备阶段"对比新旧 Outline，识别出"未变化且未被 downstream invalidation 命中"的 outline，前端为这些 outline 提供"复用上一版 draft"入口。用户确认复用后，后端复制旧 `WorkItemDraftRecord` 到当前 `generation_round_id` 下的新 `draft_id`，记录 `copied_from_draft_id`，并重新运行 draft 局部严格校验；若 reviewer 开启，仍需通过对应 review 节点。复用不跳过任何校验或 review 环节。

## 可复用代码

可以保留和改造：

- Claude Code provider adapter 与 streaming event 处理（`WorkItemSplitEngine` 的 provider 调用与 sentinel 解析框架可复用）。
- Workspace timeline 持久化。
- WebSocket session state 与 node detail 恢复机制。
- 现有 WorkItemPlan candidate DTO 的部分展示字段。
- `LifecycleStore` 中 work item、verification plan、issue work item plan 的最终落盘方法。
- `LifecycleStore` 的 JSON store 辅助能力可复用于新增 `WorkItemDraftRecord` store，但 Draft records 必须与真实 `work_items/` 分目录存储。
- dependency graph 和 validator 的部分纯函数（`validate_plan_membership` / `validate_dependencies` / `validate_scopes_and_budgets` 可复用于 Outline 轻量校验，需适配签名）。
- 当前 full validator（`WorkItemSplitValidator` 5 函数）作为最终编译后的严格校验器。
- 现有 WorkItemPlan reviewer prompt 的 5 维度审核框架（作为 Outline reviewer 基础）。

需要废弃或重写：

- 当前 `WorkItemSplitEngine` 一次性输出全量 work item 的 prompt/schema（保留 provider 调用与 sentinel 解析框架，废弃"一次性全量"语义）。
- 当前 `complete_work_item_plan_author` 的"生成后立即 full validate + 自动返修 loop"流程（保留 provider 调用与 artifact update 思路，废弃一次性 candidate 落盘语义，改为 Outline/Draft/Compile 三段落盘）。
- 当前 WorkItemPlan 自动返修 loop。
- 当前 candidate panel 只展示整组结果的交互模型。
- 当前校验失败直接进入自动返修的 timeline 行为。
- `repository_profile` 在 WorkItemPlan 新流程中的 provider 输出与必需展示语义；legacy 字段先保留并置空，不在首个实现批次全局删除。
- reviewer prompt 的 markdown JSON fence 输出方式（统一改造为 sentinel structured block，见 Prompt 契约总则）。

## 前端交互要求

Outline 阶段：

- author 生成过程要持续展示探索和拆分进度。
- outline 以可扫描的计划形式展示。
- 用户确认前不得创建真实 child work item session。

串行生成模式：

- 当前 work item 作为独立 author 气泡展示。
- 每个气泡下有接受、重写、暂停操作。
- 已确认项折叠展示摘要，但可展开查看完整内容。
- 生成后续项时，页面应提示正在携带前序 work item 上下文。

自动模式：

- 展示所有 work item 的生成队列、运行中、已完成、失败状态。
- 每个完成项仍有独立气泡。
- 操作区只提供接受全部、整组重写、暂停整组。

Artifact 版本与对比：

- 右侧 Artifact 面板提供 `当前内容`、`历史版本`、`对比` 三个视图。
- 历史版本按 Outline、Context blocker resolution、Draft、Batch snapshot、Compile report 分组。
- 用户从 timeline 选择历史节点时，右侧展示该节点对应的历史 artifact，且明确标记只读 / superseded / active / compiled 状态。
- WorkItemPlan 使用结构化 Diff 展示变动；Outline、Draft、Batch、Compile report 分别按本方案的结构化 Diff 规则渲染。

## 错误处理

Outline 轻量校验失败：

- 轻量校验发生在 author 生成完成后、进入 `outline_confirm` 前。失败时 author 结果不进入 `outline_confirm`，而是创建失败的 outline artifact/version 与 validator findings。
- 若 finding 是可自动修复的结构问题（重复 id、缺失依赖、环形依赖、缺少追踪关系等），后端可在 retry 上限内自动重跑 Outline author，并把校验错误摘要作为 revision feedback 注入下一次 prompt。
- 若 author 返回 `context_blockers[]`，或连续自动返修超过上限，进入 `human_confirm` 阶段的 `work_item_plan_context_blocker` 节点，让用户补充上下文后重跑 Outline 或终止流程。
- 展示结构化错误摘要。

单个 work item 生成失败：

- 串行模式停在当前 item。
- 用户可重试、带反馈重写或暂停。

自动模式任一 item 失败：

- 标记整组失败。
- 允许重试整组或暂停。
- 不提供单 item 重写。

WorkItemPlan Outline review 不通过（reviewer 开启时）：

- 回到 Outline 返修。
- 必须展示 reviewer findings。
- 不允许进入生成模式选择。

串行模式 Work Item review 不通过（reviewer 开启时）：

- 停在当前 work item。
- 只允许重写当前 work item。
- 不能继续生成下一个 work item。

自动模式 Work Items 整组 review（reviewer 开启时）：

- **reviewer 通过**：`work_item_batch_review` 节点自动结束，系统直接进入 `work_item_plan_compile` 运行 final compile，无需用户再次确认。
- **reviewer 不通过**：`work_item_batch_review` 节点自动结束，系统回到 `work_item_batch_confirm` 节点，标记整组待返修，前端展示 reviewer findings 与操作入口：
  - 整组重写（携带 reviewer findings 重新调度 batch run）。
  - 暂停整组。
  - 转人工处理（进入 `human_confirm` 节点）。
  - 若 reviewer 返回 `plan_reopen_required`，先执行 Draft records 失效规则，再进入 Outline 返修或人工决策节点。
- 整组 review 阶段不提供单 item 重写。

`plan_reopen_required` 触发：

- 单 item / 整组 reviewer 返回此 verdict 时，停止当前 item/batch 生成。
- 已生成未确认 Draft records 与已确认 Draft records 均不得物理删除；按 Rewrite Prompt 规则标记 `superseded`、维护 active index，并保留 timeline 历史。
- 进入 Outline 返修或人工决策，不能在当前 item prompt 中自行修改 Outline。
- Outline 返修通过后，受影响 outline 对应 draft 重新生成（详见 Rewrite Prompt 规则章节）。

最终严格校验失败：

- **item 级失败**（某 item 的 verification_plan 内部不合法、该 item 的 scope 与自身依赖冲突等）：
  - 串行模式：定位到具体 work item，执行 downstream invalidation，目标 item 与受影响下游 draft 标记 `superseded` 后从目标 item 重新生成。
  - 自动模式：不做局部失效或局部重生成，整组 draft 标记为待返修后回到 `work_item_batch_confirm`，只允许整组重写、暂停或转人工处理。用户可在失败摘要界面明确选择降级为串行模式重新生成；降级后按串行模式规则从第一个受影响 item 开始处理。
- **plan 级失败**（dependency_graph 不一致、id 映射失败、跨 item id 重复、work_item_set_id 不一致等）：
  - 两种模式都：回 Outline 返修或转人工。
- 不再静默进入内部自动返修。

### Strict Validator 归因映射

阶段 4 strict validator 必须把现有 `WorkItemSplitValidator` finding code 映射到明确 remediation scope，供串行/自动模式选择正确返修入口。

| finding code | 归因 | 处理 |
| --- | --- | --- |
| `work_item_not_in_plan` | plan 级 | 回 Outline 返修或人工处理 |
| `dependency_not_in_plan` | plan 级 | 回 Outline 返修或人工处理 |
| `dependency_graph_mismatch` | plan 级 | 回 Outline 返修或人工处理 |
| `dependency_cycle` | plan 级 | 回 Outline 返修或人工处理 |
| `frontend_backend_split_required` | plan 级 | 回 Outline 返修或人工处理 |
| `integration_work_item_required` | plan 级 | 回 Outline 返修或人工处理 |
| `e2e_work_item_required` | plan 级 | 回 Outline 返修或人工处理 |
| `verification_plan_mismatch` 且无明确 work_item_id | plan 级 | 回 Outline 返修或人工处理 |
| `parallel_scope_overlap` | item 级（涉及两个 item） | 串行模式定位最近生成的相关 item 并执行 downstream invalidation；自动模式整组重写、暂停或转人工处理 |
| `write_scope_required` | item 级 | 串行模式定位目标 item 并执行 downstream invalidation；自动模式整组重写、暂停或转人工处理 |
| `context_budget_over_limit` | item 级 | 串行模式定位目标 item 并执行 downstream invalidation；自动模式整组重写、暂停或转人工处理 |
| `traceability_refs_required` | item 级 | 串行模式定位目标 item 并执行 downstream invalidation；自动模式整组重写、暂停或转人工处理 |
| `verification_plan_missing` | item 级 | 串行模式定位目标 item 并执行 downstream invalidation；自动模式整组重写、暂停或转人工处理 |
| `verification_plan_mismatch` 且有明确 work_item_id | item 级 | 串行模式定位目标 item 并执行 downstream invalidation；自动模式整组重写、暂停或转人工处理 |
| `verification_command_source_invalid` | item 级 | 串行模式定位目标 item 并执行 downstream invalidation；自动模式整组重写、暂停或转人工处理 |
| `verification_command_cwd_outside_repository` | item 级 | 串行模式定位目标 item 并执行 downstream invalidation；自动模式整组重写、暂停或转人工处理 |
| `verification_command_needs_manual_review` | warning | 不阻断编译；展示给用户确认。若用户认为风险不可接受，可选择串行模式返修或人工处理 |
| `verification_command_unsafe` | item 级 | 串行模式定位目标 item 并执行 downstream invalidation 或人工确认；自动模式整组重写、暂停或转人工处理 |
| `verification_gate_missing` | item 级 | 串行模式定位目标 item 并执行 downstream invalidation；自动模式整组重写、暂停或转人工处理 |
| `integration_or_e2e_skipped_risk` | warning | 不阻断编译；展示给用户确认 |

`repository_profile_missing` 与 `repository_profile_low_confidence` 不属于新两阶段 WorkItemPlan 的 remediation scope。新流程调用 validator 时必须确保 `plan.repository_profile_ref = None` 且 `repository_profile = None`，从而不产生这两类 finding；legacy validator 分支可保留给旧数据路径。

## 验证策略

后端测试：

- Design spec heading/section 提取生成 `design_context_capabilities` 与 `design_context_gaps`。
- 旧 Design spec 缺章节时不直接阻断 Outline 生成，但 prompt 必须包含 `design_context_gaps`。
- Design spec + CLAUDE.md + 目录结构均不足时，Outline author 返回 `context_blockers[]` 后进入 `human_confirm` 阶段的 `work_item_plan_context_blocker` 节点。
- Outline parser 接受合法 plan outline。
- Outline validator 拦截重复 id、缺失依赖、环形依赖、缺少追踪关系。
- Outline author prompt 输出合法 sentinel JSON，且不包含完整 Work Item 或 VerificationPlan。
- Outline author prompt 不输出 `repository_profile` 或 `parallel_groups`。
- Outline reviewer prompt 不要求完整 verification plan。
- Outline reviewer prompt 输出走 sentinel structured block（与 author 解析路径统一）。
- 单 item author prompt 只输出一个 WorkItemDraftCandidate。
- 单 item author prompt 不输出 `work_item_id`（阶段 4 编译时分配）。
- provider 输出不得包含 `WorkItemDraftRecord.status`、`generated_from_node_id`、`accepted_at` 等后端状态字段。
- Draft 阶段只写 immutable `WorkItemDraftRecord` 与 active index，不写真实 `LifecycleWorkItemRecord` / `VerificationPlan` / child workspace session。
- 同一 outline 多次重写会生成多个 `draft_id`，旧 draft 标记 `superseded` 且 timeline 历史仍可恢复旧内容。
- Outline 返修后复用旧 draft 时，会复制为当前 `generation_round_id` 下的新 `draft_id`，而不是原地改写旧 record。
- WorkItemPlan artifact history index 能列出 Outline、Context blocker resolution、Draft attempts、Batch snapshots、Compile reports，并能从 SessionState / timeline node detail / Draft store / compile report 恢复。
- 结构化 Diff 后端或 DTO 层至少提供稳定输入：每个 artifact version 必须带 artifact kind、source_node_id、status、generation_round_id 或对应实体 id，前端不依赖标题字符串推断类型。
- 单 item author prompt 生成的 `required_gates` 只能引用本 verification plan 内已定义 id。
- 串行模式生成第二个 work item 时包含第一个已确认 item 上下文与 handoff_summary。
- 串行模式 item accept 前运行 draft 局部严格校验；校验失败停在当前 item 返修。
- 串行模式早期 item 被 strict validator 定位失败时，目标 item 及 downstream active drafts 标记 `superseded` 并从目标 item 重生成。
- 串行模式支持单 item 重写。
- 串行模式当前 work item review 未通过前不能生成下一个。
- 单 item reviewer 支持 `plan_reopen_required` 并能阻断后续 item 生成。
- 自动模式按拓扑顺序串行调度。
- 自动模式生成 item N+1 时携带的是当前 batch 中前序已生成并被调度器接收的 draft records，而不是用户已确认 records。
- 自动模式不允许单 item 重写。
- 自动模式全部生成完成后先进入整组确认；用户接受全部后 reviewer 开启才进入整组 review。
- 自动模式整组 reviewer 不允许返回单 item rewrite 操作。
- `ReviewComplete.work_item_plan_review` / `WorkItemPlanReviewComplete` 支持 `revise_batch`、`plan_reopen_required`、`requires_batch_revision`、`requires_plan_reopen` 等 WorkItemPlan 专属路由。
- `plan_reopen_required` 触发后，已生成未确认 Draft records 与已确认 Draft records 均不物理删除，而是标记 `superseded` 并维护 active index。
- `WorkItemDraftStatus::Superseded` 不影响现有 `WorkItemPlanStatus` 反序列化。
- 阶段 4 编译时分配 work_item_id 并构建 outline_id → work_item_id 映射。
- 阶段 4 strict validator 通过前不创建真实 WorkItem、VerificationPlan 与 child workspace session。
- 阶段 4 使用 `WorkItemPlanCompileTransaction` 记录 compile_id、step_cursor、created ids 和 commit marker；`committing` 状态刷新后可幂等续跑。
- `WorkItemPlanCompileTransaction` 在 `plan_commit_state=not_started` 时支持清理已创建实体后回到确认节点；在 `plan_commit_state=committed` 后不自动清理回退，只能继续幂等补齐或转人工整理。
- 最终编译仍运行严格 validator（现有 5 函数）。
- strict validator item 级失败定位到具体 work item，并在串行模式触发 downstream invalidation。
- strict validator item 级失败发生在自动模式时，不做局部重生成，只允许整组重写、暂停、转人工处理，或由用户明确降级为串行模式重新生成。
- strict validator plan 级失败（如 dependency_graph 不一致）回 Outline 返修。
- strict validator finding code 按归因映射表分流。
- reviewer 关闭时，Outline/单 item/整组均不进入 review 阶段；整组接受后直接进入 final compile。
- provider 中途崩溃后，刷新可恢复 outline、已确认 work item、当前运行 item 或 batch 状态。
- `select_work_item_generation_mode` / `request_outline_revision` / `work_item_draft_decision` / `work_item_batch_decision` 只能在矩阵指定 active node type 下生效，其他阶段拒绝。
- `request_outline_revision` 在 `work_item_generation_mode` 节点下解释为 Outline 返修，不触发旧整组 revision 路径。
- `work_item_plan_compile_recovery_action` 只能在 `work_item_plan_compile_recovery` 节点生效；`abort_and_rollback` 在 `plan_commit_state=committed` 时被拒绝。
- 自动模式降级为串行模式后，未受影响 outline 的 batch drafts 复制为新串行 draft 并重新跑局部校验与 review，受影响 outline 及之后按串行重新生成。
- `outline_context_index.json` 在每次 blocker resolution 后更新，下一次 Outline author run 的 prompt 包含全部历史 resolution。
- Story / Design / 普通 WorkItem reviewer prompt 也迁移到 sentinel structured block；旧 markdown fence 输出可降级解析一个版本。
- `batch_review` 节点 reviewer 通过后自动进入 `final_compile`；不通过后自动回到 `work_item_batch_confirm` 并展示 findings。
- `SessionState` 可从 Draft store + timeline node detail + compile report + outline_context_index 恢复当前 WorkItemPlan artifact payload。

前端测试：

- Outline 确认后展示两个生成按钮。
- WorkItemPlan artifact 使用 discriminated union，能区分 outline / draft / batch / compile report。
- 串行模式每个 work item 独立消息气泡和确认操作。
- 串行模式每个 work item 确认后展示 reviewer 审核状态（reviewer 开启时）。
- 自动模式展示队列状态且只允许整组操作。
- 自动模式整组生成完成后展示接受全部/整组重写/暂停；接受全部后才展示整组 review 状态（reviewer 开启时）。
- 自动模式展示整组 review 结果，不显示单 item 重写入口（reviewer 开启时）。
- `generation_mode_select` 节点展示"逐个生成 / 自动生成 / 返回 Outline 返修"三个入口，不展示通用 author_confirm 的"接受/重写"。
- 串行模式 `work_item_draft_confirm` 节点在局部校验通过前不展示"接受"按钮，校验失败后展示"重写 / 暂停"与 findings。
- `work_item_plan_compile_recovery` 节点根据 `plan_commit_state` 动态展示"继续 / 放弃 / 转人工"操作。
- `batch_review` 通过后自动进入 `final_compile` 展示编译进度；不通过后自动回到 `work_item_batch_confirm` 展示 findings。
- 刷新后可恢复 outline、已确认 work item、当前运行 item 或 batch 状态。
- Artifact 面板展示 `当前内容`、`历史版本`、`对比` 三个视图。
- 用户点击历史 timeline node 时，Artifact 面板能展示该节点的历史 artifact，并标记 active / superseded / copied / compiled / recovery_required。
- Outline diff 展示 outline 增删改、dependency edge、write scope、traceability 与 verification intent 变化。
- Draft diff 展示 goal、implementation_context、write scopes、depends_on / handoff、verification plan 变化。
- Batch diff 展示 draft 新增 / superseded / copied / retained 与 batch review findings 变化。
- Compile report diff 展示 id 映射、created ids、transaction 状态和 validator findings 变化。

回归测试：

- Story Workspace 不受影响。
- Design Workspace 不受影响。
- 普通 Work Item Workspace 不受影响。
- WorkItemPlan 不再在 validator error 后静默进入自动返修 loop。
- WorkItemPlan 的 reviewer 开关与 story/design/workitem 行为一致（默认开启、用户可关闭）。

## Review 规则

本方案确认三条规则：

1. WorkItemPlan Outline review 默认开启。Outline 经人工确认 author 结果后，若 reviewer 开启，由 reviewer 审核通过才能进入生成模式选择；若 reviewer 关闭，直接进入生成模式选择。
2. 串行模式 Work Item review 默认逐项执行。每个 work item 的 author 结果经用户确认后，若 reviewer 开启，reviewer 审核通过才能生成下一个 work item；若 reviewer 关闭，直接生成下一个。
3. 自动模式 Work Item review 默认整组执行。全部 work item 生成完成后先进入整组确认；用户接受全部后，若 reviewer 开启，由 reviewer 审核整组结果，失败时只允许整组重写或转人工处理，不支持单项重写；若 reviewer 关闭，直接进入 final compile。

三条规则的 reviewer 开关语义与 story/design/workitem 完全一致，WorkItemPlan 不做特例。reviewer 开启时走 sentinel structured block 输出与统一解析路径（见 Prompt 契约总则）。

后续实现计划仍需细化 review retry 上限与人工介入入口。review 与最终 strict validator 的错误归因边界以本方案的 Strict Validator 归因映射为准；Reviewer prompt 与 finding schema 的基本契约已在 Prompt 设计章节中定义，实现计划必须以该契约为基础。
