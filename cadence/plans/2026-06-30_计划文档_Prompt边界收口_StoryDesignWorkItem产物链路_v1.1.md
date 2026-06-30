# Prompt 边界收口与 Workspace 产物链路优化计划

## 文档信息

- 文档类型：计划文档
- 日期：2026-06-30
- 版本：v1.1
- 适用分支：feat-b-0630
- 适用范围：Story Spec、Design Spec、Work Item、Work Item Plan、Work Item Group / Coding prompt 与产物校验链路
- 当前状态：已按 review 优化，待确认
- v1.1 修订说明：基于对 `feat-b-0630` 代码现状的 review，修正了与现状不符的描述（contract 拆分范围、Work Item 语义、artifact gate 落点、WorkItemPlan 双生成路径），补充了共享约束表、结构化匹配口径、OpenSpec/Superpowers 现状判断与执行边界。
- 本轮优化说明：进一步把评审结论固化为可执行决策：Rust 共享约束表先作为真实执行源，OpenSpec config 暂不伪装成已生效约束；artifact gate 从 bool 升级为结构化校验报告；reviewer 覆盖 WorkItemPlan 的 outline/draft/batch 子路径；confirm 前硬拦截不纳入本轮，避免缺少 override UI 时造成死锁；provider 结构化交互能力建模提升为明确任务。

## 一、目标

解决 Story 生成阶段越界生成 Work Item 的问题，并同步检查 Design、Work Item、Work Item Plan、Work Item Group 的 prompt 边界，确保整条 workspace 产物链路职责清晰。

目标产物链路应保持为：

```text
Issue
-> Story Spec
-> Design Spec
-> Work Item Plan
-> Work Item
-> Work Item Group / Coding
```

核心原则：

- 只有 Work Item Plan 阶段允许把 Issue 拆分成多个任务。
- Story Spec 阶段只能生成用户故事与需求规格。
- Design Spec 阶段只能生成技术设计与约束。
- Work Item 阶段只描述单个可执行任务（详见 §6.2 的明确定义）。
- Work Item Group / Coding 阶段只负责真实执行、代码修改与验证。

## 二、当前问题判断（已对照代码核对）

### 2.1 主问题：runtime_contract 未按类型分流

经核对，`src/web/workspace_context/prompts.rs` 中：

- `system_prompt_for`（L30）、`constraint_summary_for`（L47）、`workflow_discipline_for`（L72）、`output_schema_for`（L105）**已经按 `WorkspaceType` 分流**，且 `workflow_discipline_for` 已正确分阶段（Story/Design 用 brainstorming，WorkItem/WorkItemPlan 才用 writing-plans）。
- **唯一没有按类型分流的是 `runtime_contract_for`（L137-157）**。它的 `[superpowers_contract]` 对所有类型（含 Story）统一注入了：
  - “Work Item / Work Item Plan 必须按 writing-plans 风格组织目标、范围、任务、验证、风险与追踪关系。”

这段文案会污染 Story / Design 的生成目标，让模型误以为 Story 也要输出任务拆分。**这是本次主问题的根因，且改动面比 v1.0 描述的小很多——只需拆分一个函数。**

### 2.2 兜底层缺口：gate 只做白名单且不在 confirm 硬拒

代码里存在两个互不相干的校验层，必须区分清楚落点：

- `content_has_complete_workspace_artifact`（`src/product/workspace_engine/parsers/choice.rs:225`）——这是 **workspace 当前实际使用的 gate**，仅做**白名单 heading 检查**（Story 只查 `## 功能需求` + `## 成功标准`）。它仅被 `should_retry_missing_workspace_artifact` 用于**触发重试**，**不会在 confirm 时硬拒越界内容**。`workspace_requires_artifact_gate`（`session_state/timeline.rs:84`）当前只对 Story/Design 返回 true。
- `canonical_validator`（`src/cross_cutting/artifact_validate/canonical.rs`）——有黑/白名单升级潜力，但它属于 `runtime_units`（spec_authoring 等）这条**独立管线**，**未接入 workspace confirm 流程**。`handle_confirm`（`src/product/workspace_engine/controls.rs:31`）对 Story/Design 仅翻转状态，无任何结构校验。

因此本问题是 **prompt 边界污染 + 校验层兜底不足** 的叠加，且兜底缺口位于 `content_has_complete_workspace_artifact` 而非 `canonical_validator`。

### 2.3 WorkItemPlan 存在两条生成路径

WorkItemPlan 的真实 outline / draft 生成由 `src/product/work_item_split_engine/prompts.rs` 负责，它**已经有按 role 分流的 `[openspec_contract]` / `[superpowers_contract]` / writing-plans 契约**（`work_item_plan_runtime_contract`，L32-46），质量优于 `workspace_context` 路径。本次收口必须同时覆盖这两条路径，避免只改一半。

## 三、改造原则

### 3.1 仅拆分 runtime_contract_for，对齐已分流的其它函数

不重做已经分流的函数，只把 `runtime_contract_for` 按 `WorkspaceType` 拆分，使其 allow/deny 与 `workflow_discipline_for` / `output_schema_for` 的分阶段口径一致。

各类型边界（allow/deny）统一以下表为准，并由 §4 的共享约束表落地：

| Workspace 类型 | 允许输出 | 禁止输出 |
| --- | --- | --- |
| Story Spec | 用户故事、功能需求、成功标准、非功能需求、待确认项 | Work Item、任务拆分、执行计划、实现步骤 |
| Design Spec | 架构、数据流、接口、风险、技术约束、验证策略 | Work Item Plan、开发任务列表、执行 checklist |
| Work Item Plan | 多任务拆解、任务追踪关系、依赖图、验收与验证建议 | 代码实现、Story/Design 重写 |
| Work Item | 单个可执行任务（目标、范围、子步骤、验收、验证、追踪） | 兄弟任务、Issue 级完整计划、其它任务的交叉内容 |
| Work Item Group / Coding | 真实代码修改、测试、验证结果 | 只输出计划、重新生成 Story/Design/Work Item 文档 |

### 3.2 三层收口：Prompt 引导 + Reviewer 判定 + Gate 硬约束

Prompt 只能降低越界概率，不能作为唯一保障。三层职责：

- Prompt：明确每个阶段能生成什么、不能生成什么。
- Reviewer：把跨阶段越界判为 `must_fix`。普通 Story / Design / Work Item 落点是 `build_review_input`（`prompts/review.rs:4`）；WorkItemPlan 必须同时覆盖 `build_work_item_plan_review_input`、`build_work_item_plan_outline_review_input`、`build_work_item_draft_review_input` 与 `build_work_item_batch_review_input`。
- Artifact gate：把禁止项接入 workspace author 输出后的 artifact gate（走重试/返修），阻止明显越界内容进入 reviewer / human confirm 主路径。
- Confirm：本轮不在 `handle_confirm` 加硬拦截。若未来需要 confirm 前二次硬校验，必须先补人工 override 的后端事件、API、UI 和测试，不能直接返回错误把用户卡死。

### 3.3 OpenSpec 与 Superpowers：现状判断与分阶段使用

**OpenSpec 现状（重要）**：`openspec/config.yaml` 当前是空骨架（仅 `schema: spec-driven`，context/rules 全为注释）；本 worktree 执行 `openspec list --json` 仍报 `No OpenSpec changes directory found`，即 `openspec_enabled` 目前**只是文本约束，不是真实写回链路**。因此本方案中不能把 OpenSpec config 说成已生效的运行时约束，也不能依赖尚未落地的 projection 编译。

本轮决策：

- **真实执行源**：Rust 侧共享约束表（见 §4）是 prompt / gate / reviewer / 测试的唯一执行源。
- **OpenSpec 红利**：只取“约束表达与追踪语言”这一层，即 prompt 继续明确 daemon 后续写回 OpenSpec、provider 不直接修改 OpenSpec。
- **不做伪单一事实源**：不把 `openspec/config.yaml` 写成一份没人解析的“影子规则”。若要把 config 变成单一事实源，必须同一轮实现 config 解析、加载失败策略、Rust 表与 config 一致性测试。
- **可选镜像**：可以在后续单独任务中把 `openspec/config.yaml` 作为文档镜像补充，但必须标注“documentation mirror”，不能让实现依赖它。

**Superpowers 现状**：当前 worktree 可见的项目内技能主要在 `.codex/skills`，包含 openspec-*、prepare、ui-ux；未包含 brainstorming / writing-plans / using-superpowers。这些技能可能存在于运行 provider 的外部 Codex/Claude 环境中，但不能假设目标仓内一定可读。分阶段纪律思路正确，且 `workflow_discipline_for` 已实现一半：

- Story / Design 阶段：体现 brainstorming 的需求澄清纪律。
- Work Item Plan 阶段：使用 writing-plans 的任务组织风格。
- Coding 阶段：体现执行、验证、debugging、TDD 等纪律。

本轮 prompt 表述必须避免“技能文件一定存在”的暗示，改成“按 using-superpowers / brainstorming / writing-plans 的纪律执行；若 provider 环境没有对应技能文件，也必须遵守内嵌纪律摘要”。技能是否真实可用与 §9 的 provider 能力问题同源，应一并处理。

## 四、共享约束表（本次新增的核心抓手）

为避免 allow/deny 规则在 prompt builder、gate、reviewer、测试四处漂移，新增一张按类型的共享约束表，四处统一消费。本轮不要只新增散落的字符串常量，应把约束和校验结果都结构化。

建议数据形态：

```text
ArtifactConstraintSpec {
    workspace_type: WorkspaceType,
    required_headings: Vec<HeadingRule>,
    forbidden_headings: Vec<HeadingRule>,
    forbidden_tokens: Vec<TokenRule>,
    required_id_patterns: Vec<IdPatternRule>,
    reviewer_must_fix_rules: Vec<ReviewBoundaryRule>,
}

ArtifactValidationReport {
    passed: bool,
    missing_required_headings: Vec<String>,
    forbidden_headings: Vec<String>,
    forbidden_tokens: Vec<String>,
    missing_required_ids: Vec<String>,
    warnings: Vec<String>,
}
```

落地要求：

- 新增内部 helper，例如 `artifact_constraint_spec_for(workspace_type: &WorkspaceType) -> ArtifactConstraintSpec`。
- 新增结构化校验 helper，例如 `validate_workspace_artifact_constraints(content, workspace_type) -> ArtifactValidationReport`。
- `content_has_complete_workspace_artifact` 保留为兼容 wrapper，但只委托结构化 helper 并返回 `report.passed`，避免调用方一次性大改。
- `runtime_contract_for` / `output_schema_for` 根据该表生成 allow/deny 文案。
- reviewer prompt 根据该表生成类型级越界条款，并把 `reviewer_must_fix_rules` 写入 prompt。
- 表驱动测试直接覆盖共享表、结构化报告与 prompt 消费结果。

匹配口径（避免误伤，复用现有能力）：

- 禁止 heading 必须**只匹配 heading 行**，复用 `normalize_workspace_heading_line`（`parsers/choice.rs:266`），不要裸子串匹配正文。
- 禁止 token（`[TASK-*]`、`WI-*`）必须**跳过代码围栏内文本与“追踪关系/来源引用”区块**，避免 Story 合法引用上游 ID 或正文出现“实施计划”一词被误杀。
- ID 校验先做“至少存在一条目标类型 ID”的轻量检查，不在本轮解析完整投影语义。完整 canonical 语义仍由后续 projection/canonical validator 负责。
- Work Item 的“单任务”校验先覆盖高置信模式：多个兄弟任务 heading、多个并列 Work Item 条目、Issue 级完整计划 heading；不要试图用正则理解所有自然语言。

初始约束表建议：

| Workspace 类型 | Required | Forbidden heading | Forbidden token / pattern |
| --- | --- | --- | --- |
| Story Spec | `范围`、`用户故事`、`功能需求`、`成功标准`、`待确认项`、`非功能需求`；`[REQ-*]`、`[AC-*]` | `Work Items`、`任务拆分`、`实施计划`、`执行计划`、`开发任务` | `[TASK-*]`、独立 `WI-*` |
| Design Spec | `设计范围`、`设计决策`、`公共组件`、`API 契约`、`数据模型`、`风险`、`追踪关系`；`[DEC-*]`/`[CMP-*]`/`[API-*]` 至少一类 | `Work Item Plan`、`任务拆分`、`开发任务`、`执行 checklist` | `[TASK-*]`、独立 `WI-*` |
| Work Item Plan | `计划范围`、`任务拆分`、`依赖图`、`验证计划`、`执行顺序`、`风险`、`追踪关系`；`[TASK-*]` | `代码实现`、`完整 Story Spec`、`完整 Design Spec` | 大段代码 fence 中的实现补丁 |
| Work Item | `目标`、`范围`、`实现步骤`、`依赖`、`验证命令`、`风险`、`追踪关系` | `兄弟任务`、`整体实施计划`、`Issue 级计划`、`Work Items` | 多个顶层 `[TASK-*]` 作为兄弟任务；跨任务 `WI-*` 章节 |

## 五、Story Prompt 优化方案

Story prompt 需要明确：

- 本阶段只生成 Story Spec。
- 必须覆盖 Issue 的用户价值、范围、功能需求、成功标准、非功能需求。
- 必须保留稳定 requirement IDs（`[REQ-001]`）与 acceptance criteria IDs（`[AC-001]`）。
- 必须追踪 source ids 与 proposal constraints。
- 禁止输出 Work Item、Work Item Plan、任务拆分、执行步骤。
- 禁止出现 `## Work Items`、`## 任务拆分`、`## 实施计划`、`[TASK-*]`、`WI-*`。
- `## 待确认项` 只用于真正无法交互解决的问题，不允许把应通过交互提问解决的问题塞进去。

落点：`runtime_contract_for(Story)` 移除 writing-plans 文案；`output_schema_for(Story)` 维持现有六个 heading，并由共享表补 forbidden_headings / forbidden_tokens。

## 六、Work Item 与 Work Item Plan 优化方案

### 6.1 Work Item Plan

Work Item Plan 是唯一允许把 Issue 拆成多任务的阶段。该阶段应：

- 基于 Story、Design 与 OpenSpec constraints 拆分任务。
- 每个任务都追踪到 requirement IDs。
- 每个任务包含目标、范围、验收、验证建议、风险。
- 不直接输出代码实现，不重写 Story 或 Design 的需求语义。

两条路径都要对齐：`workspace_context/prompts.rs` 与 `work_item_split_engine/prompts.rs`（后者已有较完整契约，本次只需对齐 allow/deny 口径与共享表）。

### 6.2 Work Item（语义已明确）

Work Item 描述**单个可执行任务**。明确定义（本次确认）：

- 是一个单个可执行的任务说明，**可以包含子步骤**。
- **大小受控**：内容控制在约 20k 以内，确保单个会话可完成。
- **零交叉**：只包含当前任务的内容，不得包含任何其它任务的交叉内容。
- 不生成兄弟任务、不重新规划整个 Issue、不改写上游 Story / Design 语义。
- 保留与 Story / Design / Work Item Plan 的追踪关系。

**与现有代码的冲突（必须同步处理）**：`output_schema_for(WorkItem)`（`prompts.rs:129`）当前要求 Work Item 内必须含“任务拆分”并使用 `[TASK-001]`。按上述定义需改写为：

- 允许“子步骤 / 实现步骤”表述，但语义是**单任务内部步骤**，不是多任务拆分。
- 措辞从“任务拆分”改为“实现步骤”或“子步骤清单”，避免与 Work Item Plan 的多任务拆分混义。
- 明确加入“大小约束（≤20k，单会话可完成）”与“禁止跨任务交叉内容”两条。
- 保留目标、范围、验收、验证命令、风险、追踪关系。
- 共享表对 Work Item 的 forbidden 项应包含“兄弟任务 / 多个并列任务条目 / Issue 级完整计划”。

## 七、Design Prompt 优化方案

Design prompt 需要明确：

- Design 是技术方案，不是任务计划。
- 可说明组件边界、数据模型、API、状态流、错误处理、扩展性。
- 必须引用 Story requirement IDs，保持追踪关系。
- 可说明验证策略，但不能写成开发任务清单。
- 禁止生成 Work Item、Work Item Group、任务拆分、执行 checklist。

落点：`runtime_contract_for(Design)` 移除 writing-plans 文案；`output_schema_for(Design)` 维持现有 heading，由共享表补 forbidden 项。

## 八、Reviewer 与 Artifact Gate 增强方案

### 8.1 Reviewer 越界检查

落点必须覆盖普通 workspace review 与 WorkItemPlan 子 review：

- 普通 Story / Design / Work Item：`build_review_input`（`prompts/review.rs:4`）。
- WorkItemPlan 最终整组候选：`build_work_item_plan_review_input`。
- WorkItemPlan Outline：`build_work_item_plan_outline_review_input`。
- WorkItemPlan 单项 Draft：`build_work_item_draft_review_input`。
- WorkItemPlan 自动 Batch Draft：`build_work_item_batch_review_input`。

以上 prompt 都消费 §4 的 `reviewer_must_fix_rules`，按类型注入 `must_fix` 越界规则：

- Story 中出现 Work Item heading、任务拆分、`[TASK-*]`，判 `must_fix`。
- Design 中出现 Work Item Plan、开发任务清单，判 `must_fix`。
- Work Item 中出现兄弟任务或跨任务交叉内容，判 `must_fix`。
- Work Item Plan 中直接输出代码实现，判 `must_fix`。

验证要求：

- `workspace_engine` reviewer prompt 单测至少断言 Story、Design、Work Item 三类普通路径都包含对应越界条款。
- WorkItemPlan outline / draft / batch review 至少各有一条 prompt 断言，防止只改最终整组 review。

### 8.2 Artifact Gate 硬校验（明确落点）

将现用 gate `content_has_complete_workspace_artifact`（`parsers/choice.rs:225`）从“必要 heading 检查”升级为“结构化校验 wrapper”，底层调用 `validate_workspace_artifact_constraints` 并返回报告。

Story gate 建议检查：

- 必须包含 Story Spec 要求的六个 heading。
- 必须包含 `[REQ-*]` 与 `[AC-*]`。
- 必须拒绝任务类 heading（`## 任务拆分` / `## Work Items` / `## 实施计划`）。
- 必须拒绝 `[TASK-*]`、`WI-*` 等任务 token。

Design / Work Item / Work Item Plan 各自配置白名单与黑名单（统一来自 §4 共享表）。

覆盖范围决策：

- Story / Design：继续由 `workspace_requires_artifact_gate` 强制执行，因为它们当前已经走 gate。
- Work Item：本轮纳入 gate，但只启用高置信 required/forbidden 规则，避免模型自然语言被过度拦截。
- Work Item Plan：`workspace_context` 入口纳入 gate；`work_item_split_engine` outline/draft 的结构化 JSON 仍优先由现有 parser/validator 负责，本轮只对 reviewer prompt 对齐 allow/deny，不重复写 Markdown gate。
- Work Item Group / Coding：不纳入该 Markdown artifact gate，Coding 的真实验证继续由 coding workspace / tester / reviewer 链路负责。

### 8.3 防死锁约束

- 主路径仍以“author 输出阶段检测 → 重试 + reviewer must_fix”为主（沿用现有 `should_retry_missing_workspace_artifact` 机制）。
- 本轮不在 `handle_confirm` 加硬校验；只保证明显越界内容在进入 human confirm 前被 gate/reviewer 拦下。
- 若未来加 confirm 前二次校验，必须新增 `artifact_validation_override_required` 这类人工门禁事件，UI 显示结构化报告，并提供“退回返修 / 人工 override 确认”两条动作。
- 黑名单先覆盖高置信越界模式（任务 heading、`[TASK-*]`），边界模糊内容交给 reviewer 判 `must_fix`。

## 九、交互机制优化方案

现状：`workflow_discipline_for`（`prompts.rs:90-102`）已根据 `author_provider` 区分 Claude Code（AskUserQuestion）与 Codex（requestUserInput），且 `text_fallback` 兜底链路已存在。

本节从“建议进一步”提升为本轮明确任务，因为 Story / Design 的 brainstorming 纪律依赖结构化交互能力；如果 prompt 要求 provider 做不到的工具调用，就会把未决问题错误写进 artifact 正文。

当前 `ProviderCapability`（`src/cross_cutting/provider_capabilities.rs`）只有 `supported_output_modes`、`supports_session`、`supports_resume`，未显式建模“结构化用户输入能力”。本轮采用最小改造：

- 新增内部交互能力 helper，例如 `structured_interaction_guidance_for(provider, capability) -> StructuredInteractionGuidance`。
- 初始规则可先按 provider 名称给默认能力：Claude Code -> `AskUserQuestion`，Codex -> `requestUserInput`，Fake/未知 -> daemon pause signal / text_fallback。
- prompt 只在能力明确时要求原生结构化工具；能力未知时要求输出 daemon 可识别的暂停信号，不要求模型伪造工具调用。
- 禁止把 A/B/C 文本选择题当作最终 artifact 正文。
- 后续若 `ProviderCapability` 扩字段，应把 helper 改为优先读 capability，provider 名称只作为 fallback。

## 十、验证方案

本计划不要求执行 E2E，端到端验证由用户手动进行。

遵循项目 TDD 规则（先写失败测试再改实现）。代码侧补充单元测试与表驱动测试（直接覆盖 §4 共享表）：

- `artifact_constraint_spec_for` 对 Story、Design、Work Item、Work Item Plan 返回完整 required/forbidden/id/reviewer 规则。
- `validate_workspace_artifact_constraints` 返回结构化报告，能区分缺少 required heading、出现 forbidden heading、出现 forbidden token、缺少 required ID。
- Story / Design 的 `runtime_contract` 不再包含 writing-plans 任务拆分文案。
- Story 输出含 `## Work Items` / `[TASK-001]` 时应被 gate 拒绝或 reviewer 返修。
- Design 输出任务拆分时应被拒绝或返修。
- Work Item Plan 仍允许多任务拆分。
- Work Item 只允许单个任务、禁止兄弟任务与跨任务交叉内容。
- Work Item 大小约束（≤20k，单会话可完成）的文案存在性断言。
- 表驱动覆盖 Story、Design、Work Item、Work Item Plan、Coding。
- reviewer prompt 覆盖普通路径与 WorkItemPlan outline/draft/batch 子路径。
- provider 结构化交互 helper 覆盖 Claude Code、Codex、Fake/未知 provider 的文案分流。
- 复用 `workspace_context/tests.rs` 现有表驱动样例（如 `all_workspace_artifact_outputs_require_artifact_fence`）。

如涉及后端 Workspace Engine 或共享 workspace contract 变更，验证说明必须明确 Story Spec、Design Spec、Work Item 三种产物类型是否已覆盖，且两条 WorkItemPlan 生成路径是否都已对齐。

## 十一、实施顺序（按 TDD 调整）

1. 新增共享约束表与结构化报告的失败测试：覆盖 Story、Design、Work Item、Work Item Plan 的 required/forbidden/id/reviewer 规则。
2. 落地 `ArtifactConstraintSpec` / `ArtifactValidationReport` 与 `artifact_constraint_spec_for`，先不改调用方。
3. 新增并落地 `validate_workspace_artifact_constraints`，让 `content_has_complete_workspace_artifact` 作为兼容 wrapper 调用它。
4. 补失败测试：Story/Design `runtime_contract` 不含 writing-plans，Story/Design/WorkItem/WorkItemPlan allow/deny 文案来自共享表。
5. 拆分 `runtime_contract_for`，移除 Story/Design 的 writing-plans 文案，并按类型注入 allow/deny。
6. 改写 `output_schema_for(WorkItem)`（单任务 + 子步骤 + 大小约束 + 禁交叉），避免“任务拆分”混义。
7. 扩展 gate 覆盖：Story/Design 全量结构化校验，Work Item 高置信结构化校验；WorkItemPlan Markdown 入口按需纳入，split engine JSON 路径继续用现有 parser/validator。
8. 在 reviewer prompt 注入类型级 `must_fix` 越界规则，并覆盖普通 review 与 WorkItemPlan outline/draft/batch 子路径。
9. 对齐 `work_item_split_engine/prompts.rs` 的 allow/deny 口径与共享表；不重复实现它已有的 runtime contract。
10. 落地 provider 结构化交互 helper，替换 `workflow_discipline_for` 内硬编码 provider 文案；能力未知时走 daemon pause signal / text_fallback。
11. 本轮不改 `handle_confirm` 硬校验；只补测试确认 human confirm 路径不会因为新增 gate 缺 override 而直接卡死。
12. 可选后续任务：若决定让 OpenSpec config 成为事实源，再实现 config 解析与一致性测试；本轮不把 `openspec/config.yaml` 作为执行源。
13. 跑 `cargo test --locked --lib workspace_context`、`cargo test --locked --lib workspace_engine` 相关过滤、`cargo test --locked --lib work_item_split_engine`，再跑 `cargo check --locked` / `cargo clippy --all-targets --all-features --locked -- -D warnings`（遵守 `cadence/project-rules/build-test-commands.md`，禁止 `-j 1`）。
14. 启动服务，由用户进行端到端手测。

## 十二、风险与取舍

### 12.1 风险

- Prompt 收紧过度可能导致模型拒绝输出合理的验证策略。
- Artifact gate 黑名单过宽可能误伤合法内容（用 §4 的结构化匹配口径缓解）。
- Provider 交互能力差异可能导致 prompt 规则与实际 runtime 能力不一致。
- 两条 WorkItemPlan 路径口径不一致会造成行为漂移（用共享表统一）。
- confirm 前硬 gate 若无 override 会卡死用户（本轮不做 confirm 硬拦截；后续必须先补 override 流程）。
- OpenSpec config 若只写不读会形成第二份漂移规则（本轮 Rust 表为执行源，config 仅作为后续可选镜像）。
- 结构化报告引入后，如果只保留 bool wrapper 而不在日志/测试中暴露原因，排障价值有限（测试必须断言具体 issue kind）。

### 12.2 取舍

- 优先保证产物类型边界正确，再逐步优化生成质量。
- Gate 规则先覆盖高置信越界模式（任务 heading、`[TASK-*]`）。
- 对边界模糊内容先交给 reviewer 判 `must_fix`，避免 gate 一开始过度严格。
- OpenSpec 红利本次只取“约束语言与可追踪性”，不把未解析的 config 当作事实源。
- provider 能力先用 helper/fallback 收口，不在本轮重做完整 capability probe。

## 十三、完成标准

完成后应满足：

- Story 生成结果不再包含 Work Item 或任务拆分。
- Design 生成结果不再提前生成开发任务。
- Work Item Plan 是唯一多任务拆分阶段。
- Work Item 只描述单个可执行任务（可含子步骤、≤20k、单会话可完成、无跨任务交叉）。
- Reviewer 能识别跨阶段越界并要求返修。
- Artifact gate 能阻止明显越界内容进入 reviewer / human confirm 主路径，且不改动 confirm 阶段造成死锁。
- Story、Design、Work Item 三类 workspace 与两条 WorkItemPlan 生成路径的共享链路影响已被统一评估。
- allow/deny 规则由单一共享约束表驱动，prompt / gate / reviewer / 测试四处一致。
- OpenSpec config 不被误声明为运行时事实源；若未来要启用，必须有解析与一致性测试。
- provider 结构化交互文案由能力 helper 分流，能力未知时不要求模型伪造工具调用。
