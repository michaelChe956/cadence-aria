## 背景

两套同名机制并存，职责重叠但地位悬殊。

| | `HandoffRevision` | `WorkItemHandoff` |
|---|---|---|
| 定义 | `src/product/models/work_item_revision.rs:187` | `src/product/coding_models/plan.rs:49` |
| 落盘 | issue 级 lineage，跨 attempt 存活 | attempt 目录 `units/<unit>/work-item-handoff.json` |
| 内容 | 结构化契约与能力清单 | 自然语言摘要 |
| 生成 | group completion 从契约编译产出 | provider 调用生成 |
| 校验 | `runtime_handoff_authority.rs` 比对 commit / revision / status，失败关闭 | 无 |
| 消费 | 下游 unit、运行时校验、reviewer / tester 上下文 | reviewer 读摘要、验收 `ac_handoff_published` |

真正承担交接职责的是前者。后者的六类字段中五类可由 diff 或 `HandoffRevision` 得到，唯一独有的是自然语言意图与下游提示。

## 根因

`generate_work_item_handoff_from_provider`（`handoffs.rs:401`）的 prompt 要求 provider 汇报 `files_changed`、`diff_summary`、`tests_run`，却不在 prompt 中提供这些信息；provider 只能自行查 git。而 `default_compatibility_matrix`（`src/cross_cutting/adapter_compatibility.rs`）给 claude 的 run_command 带 `--tools ""`。

实测（真实 worktree，相同命令与 prompt 形态）：provider 返回 exit 0，输出 267 字节，内容为一次工具调用尝试，不含 `<ARIA_STRUCTURED_OUTPUT>` sentinel。`cli_adapter.rs:119` 的 `parse_last_structured_output` 因此报 parse error。

该路径是结构性必然失败，而非偶发。

## 决策

### 决策一：移除而非修复

修复方案（在 prompt 中直接注入 diff，或为 handoff 开放工具权限）技术上可行，但产出仍是冗余数据。既然五类字段已有更准确的来源、且唯一独有字段不足以支撑一套生成机制与验收依据，移除比修复更彻底。

### 决策二：`HandoffRevision` 的 `tests` 与 `artifacts` 一并移除

`build_group_handoff_revision`（`group_completion.rs:576-602`）中，`tests` 来自交接摘要的 `tests_run`，`artifacts` 来自 `files_changed`。移除交接摘要后二者无数据源。

- `tests` 依赖 testing 阶段，该阶段已决定废弃。
- `artifacts` 是 git 事实的二手拷贝，且存在一个必须迁移的消费者（见决策二之二）。

保留恒空字段会给后来者留下误导。因此移除，`HandoffRevision` 收敛为纯契约凭证。

被否决的替代：保留字段恒空（改动更小但留下失效语义）。

### 决策二之二：组完成门禁的 changed_files 改用 git 事实，不接受门禁失效

`artifacts` 并非只有生产者没有消费者。`gates.rs:281-287`：schema v2 路径的组完成门禁把 `facts.handoff.artifacts` 作为 changed_files 送入 `validate_changed_files_for_runtime`（`gates/schema_v2.rs:110-140`），按 `write_policy.forbidden_scopes` / `exclusive_scopes` 判 `WorkItemDiffScopeViolation`。legacy 路径（`gates.rs:289-305`）用 `handoff.files_changed`，同样在移除范围内。

也就是说两条路径的写入范围校验数据源都会随本变更消失。`validate_changed_files_for_runtime` 的循环体 `for relative_path in changed_files` 会退化为零次迭代，函数恒返回 `Ok(())`——**任何越界写入静默放行**。

这是安全相关的行为退化，不接受。写入范围门禁必须改用 git 事实作为数据源。

可用来源已存在：`changed_files_for_attempt`（`gates.rs:351-369`，走 `git_status --porcelain`），单 WorkItem 完成门禁（`gates.rs:230-236`）正是这么取的。schema v2 组路径需要的是 per-unit 粒度，而 `HandoffRevision.commit_sha`（`work_item_revision.rs:195`）与 unit run 的 `completion_commit` 已由 `runtime_handoff_authority.rs:38` 校验一致，因此每个 unit 的 changed files 可由其 completion commit 得到。

实施时二者取其一，判据是能否保住 per-unit 粒度：
- 若能按 unit completion commit 取 changed files，保持现有 per-unit 校验语义不变（首选）；
- 若不能，退为 attempt 级 `changed_files_for_attempt` 对每个 unit runtime 校验一次。这会放宽 per-unit 归属判定（A unit 的越界写入可能记到 B unit 头上），必须在实施说明中记录该退化，且不得静默采用。

代价是每次组完成门禁多若干次 git 调用。相对于越界写入放行，这个代价可以接受——这也是决策二里「为冗余信息增加 git 调用」不再成立的原因：`artifacts` 对这个门禁不是冗余信息，而是它唯一的数据源。

### 决策三：不为既有持久化记录保留兼容

`HandoffRevision` 持久化于 issue 级 lineage，既有记录含 `tests` 与 `artifacts`。用户明确按全新系统处置，不保留历史数据兼容：直接移除字段，不添加迁移、不添加 `#[serde(alias)]` 或忽略未知字段的兼容层，也不为历史记录编写兼容测试。

影响面不止 lineage 记录。`LifecycleWorkItemRecord` 的三个待移除字段 `planned_handoff_summary`、`required_handoff_from`、`handoff_summary_ref`（`models/lifecycle.rs:152,166,174`）均**无** `#[serde(default)]` 保护，移除后既有 work item 记录同样面临反序列化失败，且这类记录数量远大于 handoff revision。

代价是本变更前写入的 lineage 记录与 work item 记录都可能不可读。这是用户已接受的取舍，换取代码不携带一次性兼容分支。

### 决策四：legacy 前置校验一并移除

`src/web/handlers/coding.rs:117-135` 在启动 coding 前校验 `required_handoff_from` 中的上游是否都有 `handoff_summary_ref`，缺失则拒绝。而 `handoff_summary_ref` 仅在非 schema v2 路径写入（`handoffs.rs:269` 判断 `schema_v2_group_plan_lineage` 为 `None`）。

移除交接摘要后该字段永不写入，校验对所有流程恒定放行，属失效逻辑。保留它比移除更具误导性，故一并移除 `handoff_summary_ref`、`required_handoff_from`、`planned_handoff_summary` 与该校验。

### 决策五：严格区分 legacy 交接摘要与 schema v2 契约

`work_item_split_engine` 同时含两类 handoff 命名，必须区分：

| 标识 | 归属 | 处置 |
|---|---|---|
| `required_handoff_from` | legacy 交接摘要前置 | 移除 |
| `max_handoff_chars`、`max_dependency_handoffs` | legacy 摘要注入预算 | 移除 |
| `handoff_contract`（`schema.rs:282`） | schema v2 契约体系 | **保留** |
| `handoff_field`（`schema.rs:243`，`EvidenceKind` 成员） | schema v2 证据类型 | **保留** |

误删后两者会破坏 `HandoffRevision` 的编译来源。

### 决策五之二：reviewer 提示词中以交接摘要为审查对象的指令一并移除

移除数据结构而保留提示词指令，会让本变更的核心动机落空。

`prompts.rs:346-353` 的 `group_final_review_material_protocol` 有多条硬性指令以 unit handoff 为审查对象，其中 `:347`「必须确认每个 completed unit 的 handoff 承诺是否体现在最终 diff 或最终报告中」、`:350`「handoff 未闭环…必须 request_changes 或 blocked」、`:353`「不得用平台默认技术栈假设替代 unit handoff」。`prompts.rs:325` 的 `code_review_material_protocol` 直接点名「handoff tests_run/test_result_summary」。

字段移除后这些指令仍在，reviewer 会拿到「必须确认 handoff 承诺闭环」的要求配一个没有承诺字段的上下文段落——假阳性只是换了形式，proposal 声称要消除的机制原样保留。

因此本变更必须同批改写这些指令：以 `HandoffRevision` 的契约与能力语义为审查对象，不再以自然语言承诺为审查对象。改写限于把审查对象从摘要切换到契约，不改变 reviewer 的否决权限边界与 verdict 取值口径。

### 决策六：前端与协议一并移除

交接摘要经 WebSocket 协议（`protocol.rs:58`、`types.rs:559`）暴露给前端并被状态层消费。后端移除而前端保留会留下永不赋值的字段与死组件，故同批移除。

### 决策七：`update_work_item_handoff_summary` 不能整体删除

`lifecycle_store/work_item.rs:177-192` 同时写两个字段：`handoff_summary_ref`（本变更移除）与 `completion_commit`（**必须保留**）。该函数是 `completion_commit` 在全仓的唯一写入点（`:188`），而 `completion_commit` 有独立消费者：`web/handlers/coding.rs:318` 填 `WorkItemDependencyHandoffRef.commit_sha`，前端 `IssueLifecycleWorkbenchParts.tsx:536` 与 `LifecycleCardDrawer.tsx:631` 渲染。

因此不是删函数，而是把 `handoff_summary_ref` 参数与赋值摘掉、保留 `completion_commit` 写入，并按新职责重命名。

### 决策八：`WorkItemHandoffMissing` 的九个触发点分属两个体系

该错误变体不能整体移除。属交接摘要、随本变更消失的四处：`group_completion.rs:101`、`gates.rs:241`、`reports.rs:51`、`handoffs.rs:244`。属 `HandoffRevision` 体系、语义必须保留的四处：`runtime_impact.rs:479`、`plan_defect.rs:474`、`plan_defect.rs:487`、`group.rs:20`。

其中 `group.rs:20` 是既有的语义错用——`get_active_coding_unit` 返回 `None` 时报「handoff missing」，与交接无关。本变更需为这四处保留可用变体；`group.rs:20` 应换用语义正确的变体，属顺带修正而非范围扩张。

另注意 `web/error.rs:97` 的 `"work_item_handoff_missing"` 是 API 错误码字符串（对应 `web/handlers/coding.rs:130` 的 `ApiError`），与引擎侧枚举是两套东西，处置需分别判断。

### 决策九：`handoff_notes` 与 `handoff_strategy` 属 outline 层，保留

决策五的区分表不完整。以下两项名字带 handoff 但与交接摘要无关，且都是 splitter schema 的 `required` 项，误删会让 splitter 输出全部校验失败：

| 标识 | 位置 | 归属 |
|---|---|---|
| `handoff_notes` | `work_item_split_engine/schema.rs:496,515`、`models/outline.rs:97` | outline 层，**保留** |
| `handoff_strategy` | `schema.rs:524,536`、`outline.rs:51`、`workspace_engine/plan_projection.rs:222` 用作 `split_reason` | outline 层，**保留** |

## 边界

- 不改 `HandoffRevision` 的契约字段语义与运行时权威校验判定口径。
- 不改组完成门禁的写入范围**判定口径**（`forbidden_scopes` / `exclusive_scopes` 语义不变），只改其 changed_files 数据源（见决策二之二）。
- 不改 reviewer 的否决权限边界与 verdict 取值口径，只改审查对象（见决策五之二）。
- 不改 `completion_commit` 的写入时机与消费路径（见决策七）。
- 本变更只消除 `handoffs.rs:360` 这一个 `list_testing_reports` 消费者（删除占位生成器的副产品）；`reports.rs:11`、`reviewer_context.rs:18`、`plan_defect.rs:427`、`web/coding_ws_handler/state.rs:30`、`web/handlers/coding.rs:552` 五处归 `remove-testing-stage`，本变更不触碰。
- `coding_evaluation_context` 的 `handoff_tests_run` / `handoff_test_result_summary`（`builder.rs:383-389`、`mod.rs:59-60`）归本变更移除；`remove-testing-stage` 不重复处理。
- 不改 schema v2 的 `handoff_contract` 与 `handoff_field`。
- 不改 group completion 的 unit 完成判定、commit 绑定与 `HandoffRevision` 发布路径（除不再读取交接摘要）。
- 不改 testing 阶段本身：其移除由 `remove-testing-stage` 负责。本 change 只移除交接摘要中依赖 testing 的字段。
- 不自动清理历史遗留的 `work-item-handoff.json`。
- 不为历史持久化数据提供迁移或兼容层（见决策三）。
