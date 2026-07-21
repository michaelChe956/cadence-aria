# 设计评审：WorkItemPlan 两阶段生成与逐项 WorkItem 确认流程 v1.4.1

> **评审日期**：2026-06-22
> **评审版本**：v1.4.1
> **评审人**：Claude Code (独立评审)
> **前序评审**：v1.2 评审（4P0 + 5P1 + 5P2，P0/P1 已全部修复）

---

## 评审总结

v1.4.1 相比 v1.2 有显著改善，核心架构（DraftRecord 不可变存储、CompileTransaction 幂等恢复、sentinel block 统一协议）已趋稳定。本次评审聚焦于：**实现可行性缺口**、**边界条件遗漏**、**数据一致性风险**、**工程落地成本**四个维度。

发现 **3 个 P0**（阻塞实现）、**5 个 P1**（影响正确性）、**7 个 P2**（体验/健壮性），以及 **3 个建议项**。

---

## P0 — 阻塞实现

### P0-1：Compile Transaction 步骤 8→9 非原子写入导致的 TOCTOU 风险

**位置**：§3.4 Compile Transaction 步骤 7-9

**问题**：
方案定义的 compile 步骤为：
- 步骤 7：创建 LifecycleWorkItemRecord 文件（替换 depends_on 中的 draft_id 为 real ID）
- 步骤 8：将新 work_item_ids 写入 IssueWorkItemPlan
- 步骤 9：写入 `plan_commit_state = committed`

步骤 8 和 9 是两次独立文件写入。如果步骤 8 成功但步骤 9 之前崩溃：
- Plan 已指向新的 work_item_ids
- 但 transaction 状态仍是 `committing`，`plan_commit_state` 未更新为 `committed`
- 恢复逻辑看到 `committing + plan_commit_state != committed`，按方案描述应"清理回退"
- 但此时 Plan 文件已被修改——回退 Plan 的旧内容从哪里来？

**严重性**：方案的恢复逻辑在此状态下无法保证正确回退。

**优化建议**：
1. 在 transaction 开始时（步骤 1）备份 Plan 的快照（`plan_snapshot_before_commit`）写入 transaction 文件
2. 将步骤 8 和 9 合并为"先写 transaction marker 表示 plan 即将提交 → 写 plan → 写 committed marker"的三阶段，其中每一步失败都有明确的前进/回退语义
3. 或者：先写 `plan_commit_state = committed` 到 transaction（步骤 8'），**然后**再写 Plan 文件（步骤 9'）。恢复时：如果 committed=true 但 plan 未更新，则重新执行 plan 写入（幂等）

### P0-2：batch_id 概念无数据定义

**位置**：§3.3.2 自动模式、§4 WorkItemPlanReviewComplete、§5 Artifact 版本历史

**问题**：
方案在多处引用 `batch_id`：
- WorkItemPlanReviewComplete 中 reviewer 输出包含 `batch_id`
- Artifact 版本历史的 Batch diff 粒度按 `batch_id` 组织
- 自动模式的"组级评审"隐含 batch 概念

但方案中：
- `WorkItemDraftRecord` 没有 `batch_id` 字段
- 没有定义 batch_id 的生成规则（何时产生新 batch？每次自动模式运行一个？）
- 没有定义 batch_id 的存储位置
- 没有定义 batch_id 与 `generation_round_id` 的关系

**严重性**：自动模式的组级评审和 Artifact diff 功能无法实现。

**优化建议**：
1. 在 `WorkItemDraftRecord` 中新增 `batch_id: Option<String>` 字段
2. 明确定义：自动模式每次启动生成 = 一个新 batch；串行模式中 batch_id 为 None
3. 将 batch_id 纳入 `active_index.json` 的 batches 数组中，记录 `{ batch_id, mode, item_draft_ids, status }`
4. 明确 batch_id 与 generation_round_id 的包含关系：一个 round 可以包含多个 batch（用户多次触发自动模式）

### P0-3：Compile 拓扑排序要求未明确

**位置**：§3.4 Compile Transaction 步骤 7

**问题**：
步骤 7 需要"将 draft 的 depends_on 中引用的 draft_id 替换为对应已创建的 real record ID"。这隐含了一个强约束：**compile 必须按依赖拓扑排序处理 item**——先创建被依赖的 item 拿到 real ID，再创建依赖方。

方案未明确：
1. 必须按拓扑排序执行的要求
2. 如果存在循环依赖（虽理论上 Outline 已验证无环，但防御性检查不可少）的处理
3. 如果 depends_on 引用的 draft_id 对应的 draft 状态不是 `accepted` 怎么办

**优化建议**：
1. 在 compile transaction 启动时（步骤 2）显式执行拓扑排序，并在发现环时标记 transaction 为 `failed` + 具体错误
2. 步骤 7 明确为"按拓扑排序依次处理，每处理一个 draft 后将 draft_id → real_id 映射记入 transaction 的 `id_mapping` 字段"
3. 添加前置校验：所有参与 compile 的 draft 必须处于 `accepted` 状态，否则拒绝启动 compile

---

## P1 — 影响正确性

### P1-1：自动模式下 DraftLocalValidator 失败无处理路径

**位置**：§3.3.2 自动模式、§3.5 Draft Local Validation

**问题**：
方案定义 DraftLocalValidator 在 author 输出后自动运行，通过后用户可见接受按钮。但对于**自动模式**（无人值守生成全部 item），如果某个 item 的 local validation 失败：
- 串行模式：用户可以看到失败信息并手动决定（revise/skip/abort）
- 自动模式：没有人在场，方案未定义系统应如何自动处理

可选策略：自动重试一次？标记为 needs_review 留给组级评审？中断整个 batch？方案缺失此定义。

**优化建议**：
定义自动模式的 validation 失败策略：
- 第一次失败：自动重试生成（重新 invoke author），最多 1 次
- 第二次仍失败：将该 item 标记为 `validation_failed`，继续生成后续 item
- 组级评审时：将 validation_failed 的 item 显式报告给用户，要求人工决策

### P1-2：Outline 返修期间 active_index 与 round_id 的状态歧义

**位置**：§3.2 Outline Confirm、§2.3 active_index.json

**问题**：
流程：Outline 确认 → 生成几个 draft → reviewer 返回 `plan_reopen_required` → 回到 Outline 返修。

返修期间：
- `active_index.json` 的 `current_generation_round_id` 何时更新？
  - 如果是在 Outline **再次确认时**更新：返修期间 active_index 仍指向旧 round
  - 如果此时刷新恢复，后端看到旧 round_id + stage=AuthorConfirm(outline_confirm)，如何区分"正在返修中（未确认）"和"旧 round 仍有效（已确认但还没开始生成）"？

方案没有明确定义"正在返修中"这个中间状态在 active_index 中的表达方式。

**优化建议**：
在 active_index.json 中增加 `outline_state: "confirmed" | "revising"` 字段。当 `plan_reopen_required` 触发时：
- `outline_state` 设为 `"revising"`
- `current_generation_round_id` 不变（保留旧 round 的 draft 记录供后续复用判断）
- 新 round_id 在用户重新确认 Outline 时生成并更新

### P1-3：Reviewer 输出 `affects_items` 引用无效 outline_index 的处理缺失

**位置**：§4 WorkItemPlanReviewComplete

**问题**：
Reviewer 在 `affects_items` 中通过 `outline_index` 引用 work item。但 LLM 输出不可控，可能引用不存在的 index（如 plan 只有 5 个 item 但 reviewer 输出了 `outline_index: 7`）。

方案没有定义"reviewer 幻觉引用"的处理策略——是忽略无效引用？报错？还是将整个 review 结果标记为不可靠？

**优化建议**：
在解析 WorkItemPlanReviewComplete 时增加校验层：
1. 所有 `outline_index` 必须在当前 plan 的有效范围内（0 ~ len-1）
2. 无效引用的处理策略：**忽略该条 affects_items 条目**，但在 review_summary 中记录 warning
3. 如果超过 50% 的引用无效，视为整体解析失败，退回到原始文本展示给用户

### P1-4：串行模式 Reviewer 返回 "revise" 的作用域未定义

**位置**：§3.3.1 串行模式、§4 Review 契约

**问题**：
串行模式下，reviewer 对 item[3] 返回 `revise`。reviewer 的修改建议中是否可以涉及已确认的 item（如 item[1]）？

场景：reviewer 发现 item[3] 和 item[1] 存在范围重叠，建议修改 item[1] 的 scope。此时：
- item[1] 已处于 `accepted` 状态
- 系统是否支持"回溯修改已确认 item"？
- 如果不支持，reviewer prompt 应明确告知"只能建议修改当前 item"
- 如果支持，需要定义已确认 item 的状态回退路径

**优化建议**：
明确规则：串行模式的 revise 仅作用于**当前正在 review 的 item**。如果 reviewer 认为已确认 item 有问题，应返回 `plan_reopen_required` 而非 `revise`。在 reviewer prompt contract 中显式约束此规则。

### P1-5：Downstream Invalidation 粒度过粗

**位置**：§3.3 Downstream Invalidation 机制

**问题**：
当某个 item 被 rewrite 时，方案要求"计算 dependency graph 中所有下游 active drafts，全部标记 superseded"。对于链式依赖（A→B→C→D→E），修改 A 会导致 B/C/D/E 全部 superseded。

但实际场景中，A 的修改可能只影响 B（比如只改了 A 的 scope 边界），C/D/E 完全不受影响。方案没有提供"判断下游是否真正受影响"的机制，一律全部作废会导致大量不必要的重新生成。

**优化建议**：
提供两层策略：
1. **安全默认**：当前方案的全部 superseded 行为（保证正确性）
2. **可选优化**（标记为 P2 后续实现）：在 supersede 时同时标记 `supersede_reason: "ancestor_rewritten"`，前端在"组级确认"UI 中允许用户逐个勾选"此 item 仍然有效，跳过重新生成"

---

## P2 — 体验/健壮性

### P2-1：前端刷新恢复时 stage/node_type 时序竞态

**位置**：§2.1 Stage/Node 路由规则

**问题**：
前端按 `active_node.node_type` 路由 UI，而 `WorkspaceStage` 存在复用（如 `AuthorConfirm` 同时用于 `generation_mode_select` 和 `outline_confirm`）。

刷新恢复流程中，如果 WebSocket 先推送了 stage 变更但 active_node 数据稍后才到达（或 REST 初始化加载 timeline 有延迟），前端可能在极短窗口内渲染错误 UI。

**优化建议**：
在 WebSocket 初始化消息（workspace_state_snapshot）中保证 `stage` 和 `active_node` 作为原子单元一起推送。前端在 active_node 未就绪时展示 loading 状态而非按 stage 猜测 UI。

### P2-2：Sentinel Block 转义规则缺失

**位置**：§4 Sentinel Structured Block 协议

**问题**：
如果 author 生成的 work item 内容中恰好包含 `<ARIA_STRUCTURED_OUTPUT>` 文本（如文档中引用了系统协议），解析器可能误判为结构化输出边界。

**优化建议**：
定义转义规则：
1. sentinel 开始标记使用唯一 nonce：`<ARIA_STRUCTURED_OUTPUT nonce="随机8字符">`
2. 只有开始和结束标记的 nonce 匹配时才视为有效边界
3. 或：要求 sentinel 独占一行，且前后有空行分隔，减少误匹配概率

### P2-3：outline_context_index.json 并发写入无保护

**位置**：§2.4 OutlineContextIndex

**问题**：
如果用户快速连续提交两次"上下文补充"（如在 blocker resolution 对话框中快速敲了两次 Enter），两个写入可能并发操作同一个 JSON 文件，导致数据损坏或丢失。

**优化建议**：
使用 write-to-temp + atomic-rename 模式：
1. 写入时先写到 `outline_context_index.json.tmp`
2. 写入完成后 rename 覆盖（文件系统级原子操作）
3. 后端对同一 workspace 的上下文写入加 Mutex（进程内串行化）

### P2-4：outline_context_index.json 无大小限制

**位置**：§2.4 OutlineContextIndex

**问题**：
随着用户多轮提供上下文补充，index 文件可能无限增长。积累 10+ 轮后，读取和传递给 provider 的 token 消耗会显著增加，可能超出上下文窗口限制。

**优化建议**：
1. 设定条目上限（如最多 20 条 context resolution）
2. 超出时：合并最早的条目为摘要（summarize），或提示用户"上下文已充足，请直接确认 Outline"
3. 每条 resolution 记录 token 估算值，总 token 超过阈值时触发警告

### P2-5：前端 Unknown Node Type 降级策略缺失

**位置**：§2.1 Stage/Node 路由规则

**问题**：
方案引入 8 个新 timeline node type。如果前后端版本不匹配（老前端 + 新后端），前端收到未知 node_type 时无兜底处理。

**优化建议**：
前端实现 fallback UI：
- 未知 node_type → 展示通用的"系统处理中..."卡片 + node_type 原始值
- 附带"请刷新页面获取最新版本"提示
- 后端在 WebSocket 握手时交换 protocol_version，版本不匹配时主动推送升级提示

### P2-6：Sentinel Block 全 WorkspaceType 统一改造的影响面

**位置**：§4 Sentinel Block 统一协议

**问题**：
方案要求"本次统一将所有 WorkspaceType 的 reviewer 输出迁移到 sentinel block"。这影响 Story/Design/普通 WorkItem 等所有类型，改造范围远超 WorkItemPlan。

风险：
- 正在运行中的 workspace（旧格式 reviewer 输出）如何处理？
- "降级解析一个版本"的兼容期多长？何时删除旧解析逻辑？
- 如果分阶段上线（先 WorkItemPlan 再其他），中间态如何管理？

**优化建议**：
1. 将 sentinel 改造拆为独立 WP，不与 WorkItemPlan 功能绑定
2. 定义明确的兼容期：旧格式解析保留 2 个版本（~4 周）
3. 在 provider adapter 层做 format detection：先尝试 sentinel 解析，失败后 fallback 到旧 markdown fence 解析
4. 添加 telemetry：记录 fallback 触发次数，归零后安全删除旧逻辑

### P2-7：Artifact 版本历史与结构化 Diff 的 MVP 边界不清

**位置**：§5 Artifact 版本历史

**问题**：
v1.2 评审 P2-5 已指出工作量大建议拆分。v1.4.1 仍然将完整的 Artifact 版本方案（per-kind diff、ArtifactDiffRequest/Response、revision diff 视图）写入主方案，但没有标注哪些是 MVP 必须、哪些可后续。

如果按方案全量实现，仅 Artifact 部分的前后端工作量预估 ≥ 1 人周。

**优化建议**：
明确分层：
- **MVP（P0）**：compile_report 的简单 before/after 文本展示（无结构化 diff）
- **增强（P1）**：Outline diff（逐 item 变更高亮）
- **体验（P2）**：Draft diff、Batch review diff、完整 ArtifactDiffRequest/Response 协议

---

## 建议项（非问题，纯优化建议）

### 建议-1：DraftLocalValidator 子状态通知

当前方案中"draft 生成完成"和"local validation 完成"在同一个 node（`work_item_draft_run`）内，前端无法区分这两个子阶段。建议在 timeline event stream 中增加一个轻量信号（如 `{ type: "validation_progress", status: "running" | "passed" | "failed" }`），让前端可以展示"正在校验..."的中间状态。

### 建议-2：Compile Transaction 的进度粒度

对于 10+ item 的 plan，compile 可能耗时数秒。建议 transaction 每处理完一个 item 时推送进度（`{ compiled: 3, total: 10 }`），前端展示 progress bar。当前方案只有 transaction 开始和结束两个状态通知。

### 建议-3：Generation Round 的显式生命周期图

方案中 `generation_round_id` 在多处被引用，但没有一张统一的状态图展示 round 的完整生命周期（创建 → active → superseded / compiled）。建议补充一张状态转换图，明确 round 的所有合法状态转换及触发条件。

---

## 与现有代码的实现 Gap 分析

| 领域 | 现状 | 方案要求 | Gap |
|------|------|---------|-----|
| WorkspaceStage 转换 | 8 变体，无显式转换图校验 | Running → HumanConfirm（compile_recovery）跳转 | 需确认现有代码是否有转换白名单校验 |
| TimelineNodeType | 12 变体 | 新增 8 变体 | 需扩展 enum + 前端路由 |
| Validator | `validate()` 接收完整 plan + items | 方案要求 per-item local validation | 需新建 `WorkItemDraftLocalValidator`，API 签名不同 |
| Engine invoke | 单次 invoke 返回完整 `WorkItemSplitProviderOutput` | 方案要求多次 invoke（Outline、per-item Draft） | 需新建 `WorkItemPlanEngine`，invoke 粒度完全不同 |
| ReviewComplete | 统一的 `ReviewGate` 枚举 | 新增 `plan_reopen_required` 等 WorkItemPlan 特有 verdict | 需扩展 `ReviewVerdictType` 或子类型 |
| File I/O | Plan/Record 直接读写 JSON | Compile Transaction 需要多步事务性写入 | 需新建 transaction 层，现有代码无此模式 |

---

## 总结与优先级建议

| 优先级 | 数量 | 建议处理时机 |
|--------|------|-------------|
| P0 | 3 | 方案修订时必须解决，否则实现将卡在数据一致性问题 |
| P1 | 5 | 实现前明确规则即可，多数需要在方案中补充 1-2 段文字 |
| P2 | 7 | 可在实现过程中逐步补充，不阻塞开发启动 |
| 建议 | 3 | 体验优化，可作为 polish 阶段处理 |

**最关键的修订优先级**：
1. P0-1（Compile 原子性）→ 直接影响数据安全
2. P0-2（batch_id 定义）→ 自动模式无法实现
3. P0-3（拓扑排序）→ compile 正确性
4. P1-4（revise 作用域）→ 影响 reviewer prompt contract 设计
