# WorkItemPlan 两阶段生成与逐项 Work Item 确认流程 设计评审

> 版本：v1.0 | 日期：2026-06-22 | 目标分支：`feat-b-0616`
> 被评审方案：`cadence/designs/2026-06-22_技术方案_WorkItemPlan两阶段生成与逐项WorkItem确认流程_v1.2.md`
> 评审方式：只读调研 + 与方案负责人逐项确认

## 一、评审结论

方案方向成立，两阶段生成 + 逐项确认 + 强制 review 的设计直接命中现有流程的四个痛点：黑盒一次性生成、用户无法介入拆分策略、单点错误导致整组失败、自动返修黑盒。prompt 契约（sentinel structured block、`required_gates` 禁止自然语言）是对现有实现的正确强化。

经只读调研与逐项确认，方案有 22 处需修订（见第五节 R1-R22），其中 3 处影响范围界定、5 处影响数据模型、4 处影响状态机、其余为文本精确化。修订完成后即可进入实现计划拆解。

**核心范围调整**：
- 自动模式简化为"串行自动生成全部 + 整组 review"，并行/DAG 调度为后续扩展。
- `repository_profile` 从数据模型中移除，author 仓库知识改由 Design spec + CLAUDE.md + 目录探索提供。
- handoff 范围明确限定在 WorkItemPlan 生成流程内，不进 Coding。
- reviewer 开关与 story/design/workitem 一致（默认开启、用户可关闭），不做 WorkItemPlan 特例。

## 二、与现有实现的关键差距

### 2.1 author 已用 sentinel block，reviewer 未用

调研事实：
- WorkItemPlan author prompt 已显式使用 `<ARIA_STRUCTURED_OUTPUT>` sentinel block，后端 `parse_last_structured_output`（`provider_adapter.rs:208`）用 `rfind(STRUCTURED_OUTPUT_START)` 取最后一个 block 解析。
- reviewer prompt（`build_review_input` `workspace_engine.rs:3133` / `build_work_item_plan_review_input` `workspace_engine.rs:3204`）仍要求"末尾附加 JSON 代码块"（markdown ```json fence），解析走 `extract_tail_json`（`workspace_engine.rs:4781`），与 author 解析路径不同。

**结论**：v1.2"所有 author/reviewer 输出必须使用 sentinel structured block"是对 reviewer 的改造要求。实现时需统一 reviewer prompt 指令与 `parse_review_verdict` 解析路径，二者都走 sentinel。详见 R18。

### 2.2 一次性生成 + 自动返修 loop 的废弃边界

调研事实：
- `complete_work_item_plan_author`（`workspace_engine.rs:3714`）现状：生成 → `WorkItemSplitValidator::validate` → 失败则 `work_item_plan_author_retry_count += 1`，3 次封顶转 `HumanConfirm`，否则 `AutoRevision`（`engine.rs:3772`）。
- reviewer 是 AuthorConfirm 之后的独立阶段（`start_review_or_skip` `engine.rs:4398`），不参与 author 自动返修 loop。
- 现有 reviewer prompt 已有 5 维度审核（拆分粒度 / 依赖完整性 / 写入范围互斥 / 跨端拆分 / 验证计划覆盖度）。

**结论**：
- 废弃"一次性生成全量 + 立即 full validate + 自动返修 loop"主流程。
- 保留 `WorkItemSplitValidator` 作为阶段 4 最终 strict validator。
- 保留现有 reviewer 机制与 5 维度 prompt 作为 Outline reviewer 基础。
- 现有 `WorkItemSplitEngine` 的 provider 调用与 sentinel 解析框架可复用，废弃的是"一次性全量 prompt/schema"。详见 R8、R14（可复用代码章节修订）。

### 2.3 状态机与 timeline node 对四种 WorkspaceType 完全通用

调研事实：
- `WorkspaceStage` enum（`workspace_engine.rs:296`）8 个变体，4 种 WorkspaceType 共用，无 WorkItemPlan 专属状态。
- `TimelineNodeType` enum（`workspace_ws_types.rs:513`）12 个变体，同样通用。
- WorkItemPlan 仅有专用 timeline node title（"Work Item Plan 生成" / "Work Item Plan 自动返修 Round {n}"），node_type 复用 `AuthorRun` / `Revision`。

**结论**：v1.2 新增的 12 个状态 + 11 个 node type 与现有架构落差大。鉴于前端需要区分 outline / item draft / batch 三种完全不同的 UI 形态，建议新增语义化 node type（至少 `work_item_draft_run` / `work_item_batch_run`），而非纯 metadata。详见 R14。

### 2.4 前端无逐项确认能力

调研事实：
- `WorkItemPlanCandidatePanel.tsx` 有逐项 revert + feedback（`tsx:31-44`），但只在 `stage === "author_confirm"` 时显示。
- accept 是整组操作（`request-revision-button` + `accept-plan-button`，`tsx:153-173`），无逐项确认。

**结论**：v1.2 串行模式的"每个 work item 独立消息气泡和确认节点"是新增前端能力，需新增逐项确认组件。详见 R13（`generation_mode_select` 接入点）与前端交互要求章节。

### 2.5 `required_gates` 规则现有覆盖度

调研事实：
- `validate_verification_commands`（`work_item_split_validator.rs:489`）已校验 `required_gates` 引用的 id 在 `commands` / `manual_checks` 集合内（通过 `available_gate_ids` 交集判定）。
- `required_gates: Vec<String>`，若 provider 输出自然语言字符串，会被"引用必须存在"校验拦下。
- 现有校验已覆盖 v1.2 的 4 条规则中规则 1（稳定 id）、规则 2（引用本 plan 内 id）、规则 4（无对应项则先创建）。规则 3（禁止自然语言）因数据类型天然不会被当作合法 gate。

**结论**：v1.2"`required_gates` 规则必须写死"基本是对现状的确认，不是新增校验。方案文本应说明现有 validator 已覆盖，避免实现时重复建设。详见 R19。

## 三、流程决策（已与方案负责人确认）

| 决策项 | 结论 | 说明 |
|---|---|---|
| **v1.2 与 v1.0/v1.1 关系** | 以 v1.2 为准 | v1.0/v1.1 已删除，不继承其决策 |
| **reviewer 开关** | 默认开启、用户可关闭 | 与 story/design/workitem 一致，不做 WorkItemPlan 特例。v1.2 三条强规则中"强制开启，不允许跳过"改写 |
| **自动模式** | 串行自动生成全部 + 整组 review | 并行/DAG/dependency layer 为后续扩展；per-item prompt 与串行模式一致，力度不减 |
| **handoff 范围** | 只在 WorkItemPlan 生成流程内 | Outline 规划 + Draft 生成消费；不进 Coding；是"预期 handoff" |
| **repository_profile** | 去掉 | 从数据模型移除，author 仓库知识来自 Design spec + CLAUDE.md + 目录探索 |
| **Design spec 模板** | 补强 | 新增架构/模块/技术选型章节，作为 v1.2 实施前置 |
| **author 探索能力** | 允许轻量探索 | 可读 CLAUDE.md + 仓库目录结构，只读不改，不作为 plan 持久化字段 |
| **work_item_id 分配** | 阶段 4 编译时分配 | Draft 阶段以 outline_id 为主键；映射表在阶段 4 构建 |
| **strict validator** | 复用现有 5 函数 | 失败分 item 级 / plan 级 |
| **Draft 持久化** | 复用 LifecycleWorkItemRecord | 新增 `outline_id: Option<String>` 字段，编译时创建 child session |
| **编译失败处理** | item 级 / plan 级分流 | item 级定位重写；plan 级回 Outline 返修或转人工 |

## 四、handoff 完整契约（已确认）

handoff 是 work item 之间的衔接手段，只在 WorkItemPlan 生成流程内使用。

| 阶段 | 字段 | 作用 |
|---|---|---|
| Outline（阶段 1） | `WorkItemPlanOutline.handoff_strategy` | 元策略：定义"后续生成 work item 时如何传递上下文" |
| Outline（阶段 1） | `WorkItemOutline.handoff_notes` | 每个 item 的预期交接说明 |
| Draft（阶段 3） | `WorkItemDraftCandidate.handoff_summary` | author 生成 item N draft 时产出的"预期交付摘要" |
| Draft（阶段 3） | `required_handoff_from_outline_ids` | 声明 item N+1 依赖哪些前序 item 的 handoff |

**生成与消费链**：
- 生成：Draft 阶段，author 生成 item N draft 时，基于 `implementation_context` 产出 `handoff_summary`（预期交付什么）。
- 消费：Draft 阶段，生成 item N+1 draft 时，author prompt 携带 item N 的 `handoff_summary` 作为上下文。
- 不进 Coding：Coding 阶段输入只有 work item + work item plan + design spec + story spec，不含 handoff。

**语义**：handoff_summary 是"预期 handoff"——Draft 阶段生成 item N 的 handoff 时，item N 尚未 Coding 执行。局限是 handoff 不准只会影响 item N+1 的 Draft 生成质量，不会影响 Coding 执行（Coding 不消费 handoff）。

**与 spec 驱动的关系**：handoff_summary 是 author 基于 draft（衍生自 spec）生成的衔接信息，不是 spec 原文。author 产物本身是 spec 的衍生（implementation_context、verification_plan 同理），handoff_summary 也是衍生，符合 spec 驱动。

## 五、修订点清单（R1-R22）

### 范围界定

| # | 修订点 | 说明 |
|---|---|---|
| R1 | reviewer 开关改写 | 三条强规则中"强制开启，不允许跳过"改为"默认开启，与 story/design/workitem 共用 reviewer 开关，用户可关闭" |
| R2 | 自动模式简化 | 删除并行/DAG/dependency layer/同层并行等描述；明确"串行自动生成 + 整组 review"；per-item prompt 与串行模式一致；并行相关字段标注"后续扩展" |
| R3 | handoff 范围明确 | 只在 WorkItemPlan 流程内（Outline 规划 + Draft 生成消费），不进 Coding；明确"预期 handoff"语义 |
| R4 | repository_profile 去掉 | 从 Outline/Draft 数据模型移除；author 仓库知识来自 Design spec + CLAUDE.md + 目录探索；现有 validator 两条相关校验随之失效 |
| R5 | 新增前置工作 | Design spec 模板补强（架构/模块/技术选型章节），作为 v1.2 实施前置 |
| R6 | author 探索能力 | prompt 允许读 CLAUDE.md + 仓库目录结构（只读，不作为 plan 字段） |

### 数据模型

| # | 修订点 | 说明 |
|---|---|---|
| R7 | work_item_id 分配 | 阶段 4 编译时分配；Draft 阶段以 outline_id 为主键；映射表在阶段 4 构建 |
| R8 | strict validator 职责 | 复用现有 `WorkItemSplitValidator` 5 函数；入参换成编译后真实结构；失败分 item 级 / plan 级 |
| R9 | Draft 持久化 | 复用 `LifecycleWorkItemRecord` + 新增 `outline_id: Option<String>` 字段；编译时创建 child workspace session；review_verdict 存 timeline node metadata |
| R10 | 编译失败处理 | item 级 → 定位重写；plan 级 → 回 Outline 返修或转人工 |
| R15 | 补数据流转图 | Outline → Draft → 编译后 IssueWorkItemPlan 的字段映射表 |
| R16 | 命名统一 | `expected_write_scopes`（Outline）vs `exclusive_write_scopes`（Draft）统一或说明语义递进 |
| R17 | review 状态字段来源 | `WorkItemDraftCandidate` 的 review_verdict 存在 timeline node metadata，不污染 Record |

### 状态机

| # | 修订点 | 说明 |
|---|---|---|
| R13 | `generation_mode_select` 接入点 | 明确是 AuthorConfirm 扩展（加分支）还是独立新状态 |
| R14 | 状态机倾向 | 新增语义化 node type（至少 `work_item_draft_run` / `work_item_batch_run`），非纯 metadata |

### Prompt 契约

| # | 修订点 | 说明 |
|---|---|---|
| R18 | reviewer sentinel 改造 | 现状 reviewer 用 markdown JSON fence + `extract_tail_json`，与 author 的 sentinel 不同；需统一为 sentinel block + `parse_last_structured_output` |
| R19 | `required_gates` 现有覆盖度 | 现有 validator 已覆盖规则 1/2/4，规则 3 因数据类型天然不合法；方案应说明是对现状的确认 |

### 错误处理

| # | 修订点 | 说明 |
|---|---|---|
| R20 | `plan_reopen_required` 后处理 | 触发后已生成 drafts 清空、已确认 drafts 保留与否、Outline 返修后重新生成范围 |
| R21 | Outline 轻量校验失败状态归属 | 明确停在 `outline_running` 返修还是 `outline_confirm` 让用户看到错误 |
| R11 | "已确认的后续项"笔误 | 修正为"前序已确认项作为后序 prompt 上下文" |

### 其他

| # | 修订点 | 说明 |
|---|---|---|
| R12 | Outline 轻量校验与现有 validator 复用关系 | 复用部分函数（`validate_plan_membership` / `validate_dependencies` / `validate_scopes_and_budgets`），适配签名（现有入参是 `IssueWorkItemPlan` + `LifecycleWorkItemRecord[]`，Outline 阶段无这些数据） |
| R22 | 补测试 | provider 中途崩溃恢复、自动模式串行上下文传递正确性、reviewer 关闭时的 fallback 行为 |

## 六、建议的下一步

1. 根据本评审 R1-R22 修订 v1.2 方案，产出 v1.3。
2. v1.3 定稿后进入实现计划拆解（方案状态标注的"待实现计划拆解"）。
3. 实现计划应包含：Design spec 模板补强（前置）、Outline 阶段（数据模型 + 轻量校验 + author/reviewer prompt）、Draft 阶段（串行/自动两种模式 + 逐项确认 UI）、阶段 4 编译（id 分配 + strict validator + child session 创建）、reviewer sentinel 统一改造。

## 七、参考：关键现有实现位置

| 主题 | 位置 |
|---|---|
| `IssueWorkItemPlan` | `src/product/models.rs:632` |
| `LifecycleWorkItemRecord` | `src/product/models.rs:364` |
| `VerificationPlan` | `src/product/models.rs:612` |
| `WorkItemSplitValidator` | `src/product/work_item_split_validator.rs:23` |
| 一次性生成主入口 | `src/product/workspace_engine.rs:3714` `complete_work_item_plan_author` |
| author prompt + sentinel | `src/product/work_item_split_engine.rs:590` `build_split_prompt` |
| author sentinel 解析 | `src/cross_cutting/provider_adapter.rs:208` `parse_last_structured_output` |
| WorkItemPlan reviewer prompt | `src/product/workspace_engine.rs:3204` `build_work_item_plan_review_input` |
| reviewer verdict 解析 | `src/product/workspace_engine.rs:4779` `parse_review_verdict`（走 `extract_tail_json`） |
| reviewer 开关 | `src/product/workspace_engine.rs:1030` `start_generation(reviewer_enabled)` |
| `start_review_or_skip` | `src/product/workspace_engine.rs:4398` |
| 前端 candidate panel | `web/src/components/workspace/WorkItemPlanCandidatePanel.tsx` |
| 前端 reviewer 开关 | `web/src/components/workspace/ProviderConfigPanel.tsx:8` |
| `required_gates` 校验 | `src/product/work_item_split_validator.rs:489` `validate_verification_commands` |
