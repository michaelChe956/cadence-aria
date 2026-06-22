# WorkItemPlan 两阶段生成与逐项 Work Item 确认流程技术方案

## 文档信息

- 文档类型：技术方案
- 版本：v1.2
- 日期：2026-06-22
- 分支：feat-b-0616
- 状态：方案草案，已强化 prompt 契约，待实现计划拆解

## v1.2 变更摘要

- 继承 v1.1 已确认的 review 粒度规则：WorkItemPlan Outline 强制 review、串行逐项 review、自动模式整组 review。
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

- WorkItemPlan 第一阶段只生成“如何编写 work item 的计划”，不生成完整 work item。
- 用户先确认整体拆分方案，再选择生成模式。
- 严格串行模式下，每个 work item 独立生成、独立展示、独立确认、可独立重写。
- 自动连续/并行模式下，系统按计划自动生成全部 work item，但只支持整组确认或整组重写。
- 生成后续 work item 时必须携带已确认 work item 上下文，避免 prompt 丢失。
- 最终全部确认后，再编译成现有真实数据结构并执行严格 validator。
- WorkItemPlan Outline review 强制开启；串行模式逐项强制 review；自动模式整组强制 review。

## 非目标

- 本方案不定义 Coding Workspace 执行策略。
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
- 推荐执行顺序和可并行分组。
- 每个 work item 的验证意图概要。
- 风险、handoff 信息和上下文传递要求。

此阶段只执行轻量校验：

- outline id 唯一且稳定。
- dependency graph 引用存在。
- dependency graph 无环。
- 每个 work item outline 有 Story/Design 追踪关系。
- 每个 work item outline 有基本目标、范围和写入边界。
- 并行分组中不存在明显写入边界冲突。

此阶段不生成完整 `LifecycleWorkItemRecord`，不生成完整 `VerificationPlan`，不运行当前 full `WorkItemSplitValidator`。

### 阶段 2：用户确认 Outline

Outline 生成后进入确认节点。用户可以：

- 确认该 plan。
- 要求重写整个 plan。
- 带反馈重写整个 plan。

用户确认后必须进入 WorkItemPlan Outline review。该 review 强制开启，不受当前 reviewer 开关影响，不允许跳过。

Reviewer 审核对象是 WorkItemPlan Outline，而不是完整 Work Item。审核范围包括：

- 拆分策略是否合理。
- work item 大纲是否覆盖 Story/Design。
- dependency graph 是否合理且无明显缺口。
- 串行/并行分组是否安全。
- 写入边界是否存在明显冲突。
- work item 是否过粗、过细、遗漏或顺序错误。

Reviewer 通过后，页面显示两个生成入口：

- 逐个生成 Work Item。
- 自动连续/并行生成全部。

Reviewer 不通过时，流程回到 WorkItemPlan Outline 返修；如果 reviewer 判定需要人工判断，则停在人工决策节点，不进入 Work Item 生成。

### 阶段 3A：逐个生成 Work Item

严格串行模式。系统按 outline 的拓扑顺序逐个生成 work item。

每个 work item 生成时，author prompt 必须包含：

- 已确认的 WorkItemPlan Outline。
- 当前 work item outline。
- 所有已确认 work item 的摘要。
- 当前 work item 直接依赖项的完整内容。
- 之前已确认 work item 的写入边界、验证约束和 handoff 信息。
- 当前生成模式和用户反馈。

每个 work item 生成完成后创建独立消息气泡和确认节点。用户可以：

- 接受当前 work item。
- 带反馈重写当前 work item。
- 暂停流程。
- 继续生成下一个 work item。

用户接受当前 work item author 结果后，必须进入该 work item 的 reviewer 审核。串行模式下 review 粒度是单个 work item，当前 work item review 通过前不能继续生成下一个 work item。

Reviewer 审核对象包括：

- 当前 work item 是否符合对应 outline。
- 当前 work item 是否正确引用前序已确认 work item 的上下文和 handoff。
- 写入边界是否和已确认 work item 冲突。
- verification plan 是否完整、可执行，且 required gates 引用合法。
- 当前 work item 是否足以支撑后续 Coding Workspace。

Reviewer 不通过时，只重写当前 work item；重写 prompt 必须携带 reviewer finding、当前 outline、已确认前序 work item 上下文，以及用户补充反馈。

如果当前 work item 被重写，后续未生成项应使用重写后的版本作为上下文。已确认的后续项如果依赖被重写项，需标记为可能过期；是否自动要求重写，留到实现计划中细化。

### 阶段 3B：自动连续/并行生成全部

自动模式。系统按 outline 的依赖图调度生成：

- 无依赖或同层无冲突项可以并行。
- 有依赖项必须等待依赖项生成完成。
- 每个 work item 仍生成独立消息气泡和进度状态。

自动模式的用户确认粒度是整组：

- 接受全部。
- 整组重写。
- 暂停整组。

自动模式下，全部 work item 生成完成后必须进入整组 reviewer 审核。Reviewer 审核对象是整组 Work Items，而不是单个 item 的暂停确认点。

Reviewer 审核范围包括：

- 所有 work item 是否整体符合 WorkItemPlan Outline。
- 每个 work item 是否覆盖对应 outline。
- work item 之间的依赖关系是否仍成立。
- 并行生成的 work item 是否出现写入边界冲突。
- verification plans 是否完整且 required gates 合法。
- handoff 信息是否能支撑后续 Coding Workspace。
- 是否有 work item 明显缺失、跑偏、重复或过粗/过细。

自动模式不支持单个 work item 重写。Reviewer 不通过时，只允许整组重写、带 reviewer finding 整组重写、暂停整组或转人工处理。这样避免“自动生成但局部返修”的半自动状态复杂化。

### 阶段 4：最终编译与严格校验

所有 work item 确认后，后端再把结果编译为现有真实结构：

- `IssueWorkItemPlan`
- `LifecycleWorkItemRecord[]`
- `VerificationPlan[]`
- repository profile
- dependency graph
- child workspace sessions

此时运行严格 validator。失败处理按生成模式区分：

- 串行模式：定位到具体 work item，返回对应 work item 的重写入口。
- 自动模式：返回整组失败摘要，只支持整组重写或转人工处理。

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
| `batch_running` | 自动模式下批量或并行生成 work item |
| `batch_confirm` | 自动模式下等待确认整组 work item |
| `batch_review` | 自动模式下 reviewer 审核整组 work items |
| `final_compile` | 编译为真实 WorkItemPlan 并运行严格校验 |
| `human_confirm` | 等待最终人工确认 |
| `completed` | WorkItemPlan 确认完成 |

Timeline node 建议：

| 节点类型 | 用途 |
| --- | --- |
| `work_item_plan_outline_run` | 生成整体拆分方案 |
| `work_item_plan_outline_confirm` | 确认或重写整体方案 |
| `work_item_plan_outline_review` | 强制审核整体拆分方案 |
| `work_item_generation_mode` | 选择生成模式 |
| `work_item_draft_run` | 生成单个 work item |
| `work_item_draft_confirm` | 确认或重写单个 work item |
| `work_item_draft_review` | 串行模式下审核单个 work item |
| `work_item_batch_run` | 自动连续/并行生成整组 |
| `work_item_batch_confirm` | 确认或整组重写 |
| `work_item_batch_review` | 自动模式下审核整组 work items |
| `work_item_plan_compile` | 最终编译和严格校验 |

实际实现时可复用现有 `author_run`、`author_confirm`、`revision` 等通用节点类型，也可以新增更语义化的 node type。若复用现有节点类型，需要在 metadata 中明确 `work_item_plan_phase`，避免前端无法区分 outline 与 item draft。

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
- `parallel_groups`
- `risks`
- `handoff_strategy`
- `status`

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
- `expected_write_scopes`
- `forbidden_write_scopes`
- `depends_on`
- `verification_intent`
- `handoff_notes`

### WorkItemDraftCandidate

第二阶段生成的单个 work item 候选。

核心字段：

- `outline_id`
- `work_item_id`
- `title`
- `kind`
- `goal`
- `implementation_context`
- `exclusive_write_scopes`
- `forbidden_write_scopes`
- `depends_on`
- `required_handoff_from`
- `verification_plan`
- `status`
- `generated_from_node_id`
- `accepted_at`

## Prompt 设计要求

### Prompt 契约总则

所有 WorkItemPlan 相关 provider prompt 必须遵守以下通用契约：

- provider 可以在最终结构化输出前输出简短可读进度，供 Workbench 流式展示。
- provider 长时间探索、读取代码、分析依赖或准备重写前，必须先输出一句可读状态。
- provider 每完成一组探索或推理，应输出一句当前发现摘要，避免页面长时间无反馈。
- provider 不得修改仓库文件，不得执行实现，不得创建计划文档，不得进入 Coding Workspace。
- provider 的机器可解析结果必须放在最后一个 `<ARIA_STRUCTURED_OUTPUT>...</ARIA_STRUCTURED_OUTPUT>` sentinel block 内。
- sentinel block 内只能是完整 JSON object，不允许 Markdown code fence，不允许注释，不允许尾随解释。
- 后端只解析最后一个 sentinel block；sentinel block 之前的可读内容只用于 UI 展示。
- prompt 必须明确当前阶段、允许输出的对象、禁止输出的对象，以及失败时允许的 rewrite 范围。

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
- `parallel_groups[]`：可并行生成的 outline 分组。
- `handoff_strategy`：后续生成 work item 时如何传递上下文。
- `risks[]`：拆分风险与需要用户关注的点。

必须禁止：

- 禁止输出完整 `LifecycleWorkItemRecord`。
- 禁止输出完整 `VerificationPlan`。
- 禁止输出最终 work item id；只能使用稳定 `outline_id`。
- 禁止输出具体验证命令清单；只能输出验证意图。
- 禁止创建 child work item session。
- 禁止生成 implementation plan 或 coding 步骤。

结构化输出要点：

```json
{
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
        "expected_write_scopes": ["src/web/..."],
        "forbidden_write_scopes": ["..."],
        "depends_on": ["outline_000"],
        "verification_intent": "...",
        "handoff_notes": "..."
      }
    ],
    "dependency_graph": [
      { "from_outline_id": "outline_001", "to_outline_id": "outline_002" }
    ],
    "parallel_groups": [
      { "group_id": "group_001", "outline_ids": ["outline_001", "outline_002"], "rationale": "..." }
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
- parallel groups 是否安全，是否存在明显写入边界冲突。
- expected/forbidden write scopes 是否足以指导后续生成。
- handoff_strategy 是否能防止后续 prompt 丢上下文。

禁止 reviewer 在 Outline 阶段要求：

- 完整 verification plan。
- `required_gates`。
- 具体命令 id。
- 完整 Coding Workspace 执行计划。

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
- 禁止把自然语言验收条件写入 `required_gates`。

必须输出完整 `WorkItemDraftCandidate`，包括完整 verification plan。`required_gates` 规则必须写死：

- 每个 command 必须有稳定 `id`，例如 `cmd_fmt`、`cmd_check`、`cmd_clippy`。
- 每个 manual check 必须有稳定 `id`，例如 `manual_browser_check`。
- `required_gates` 只能引用同一个 verification plan 内已定义的 command/manual_check id。
- `required_gates` 禁止包含自然语言，例如“cargo test 全绿”“手动检查通过”。
- 如果一个 gate 没有对应 command/manual_check，必须先创建 command/manual_check，再引用其 id。

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

单个 Work Item reviewer prompt 的任务是审核当前 work item 是否可以作为后续 item 和 Coding Workspace 的稳定上下文。

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

Reviewer verdict schema：

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

自动连续/并行模式不是一个大 prompt 生成全部 work item。它由调度器按 WorkItemPlan Outline dependency graph 分层执行，多次调用“单个 Work Item Author Prompt”。

调度规则：

- 第 0 层为无依赖 outline。
- 下一层必须等待直接依赖层生成完成。
- 同层 outline 只有在 expected_write_scopes 无明显冲突时才允许并行。
- 每个 provider run 仍只生成一个 work item。
- 每个下一层 item prompt 必须包含其直接依赖项的完整生成结果。
- 自动模式下任何局部失败都会标记 batch 为待处理，但产品操作只允许整组重写、暂停或转人工。
- 整组重写时必须清空本轮全部 draft，用 batch reviewer findings 重新调度。

自动模式的 per-item prompt 与串行单 item prompt 相同，但必须额外输入：

- `batch_generation_id`。
- 当前 dependency layer。
- 同层并行 item 列表。
- batch 级用户反馈或 reviewer findings。

### 自动模式整组 Reviewer Prompt

自动模式整组 reviewer prompt 的任务是审核全部 Work Items 作为一个集合是否符合已确认 WorkItemPlan Outline。它不得要求单个 item 局部重写。

审核范围：

- 所有 outline 是否都有对应 work item。
- 每个 work item 是否覆盖对应 outline。
- dependency graph 是否仍成立。
- 并行生成项之间是否产生写入边界冲突。
- verification plans 是否完整且 `required_gates` 引用合法。
- handoff 链是否支持后续 Coding Workspace。
- 是否存在重复、遗漏、跑偏、过粗或过细。

允许 verdict：

- `pass`：整组可进入最终编译。
- `revise_batch`：整组重写。
- `needs_human`：需要用户人工判断。
- `plan_reopen_required`：发现 Outline 本身错误，必须回到 Outline 返修或人工决策。

Reviewer verdict schema：

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

## 可复用代码

可以保留和改造：

- Claude Code provider adapter 与 streaming event 处理。
- Workspace timeline 持久化。
- WebSocket session state 与 node detail 恢复机制。
- 现有 WorkItemPlan candidate DTO 的部分展示字段。
- `LifecycleStore` 中 work item、verification plan、issue work item plan 的最终落盘方法。
- dependency graph 和 validator 的部分纯函数。
- 当前 full validator 作为最终编译后的严格校验器。

需要废弃或重写：

- 当前 `WorkItemSplitEngine` 一次性输出全量 work item 的 prompt/schema。
- 当前 `complete_work_item_plan_author` 的“生成后立即 full validate”流程。
- 当前 WorkItemPlan 自动返修 loop。
- 当前 candidate panel 只展示整组结果的交互模型。
- 当前校验失败直接进入自动返修的 timeline 行为。

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

## 错误处理

Outline 轻量校验失败：

- 停在 outline confirm 或 outline revision。
- 展示结构化错误摘要。
- 用户选择带反馈重写。

单个 work item 生成失败：

- 串行模式停在当前 item。
- 用户可重试、带反馈重写或暂停。

自动模式任一 item 失败：

- 标记整组失败。
- 允许重试整组或暂停。
- 不提供单 item 重写。

WorkItemPlan Outline review 不通过：

- 回到 Outline 返修。
- 必须展示 reviewer findings。
- 不允许进入生成模式选择。

串行模式 Work Item review 不通过：

- 停在当前 work item。
- 只允许重写当前 work item。
- 不能继续生成下一个 work item。

自动模式 Work Items 整组 review 不通过：

- 标记整组待返修。
- 只允许整组重写、暂停或转人工处理。
- 不提供单 item 重写。

最终严格校验失败：

- 串行模式尽量定位到具体 item。
- 自动模式按整组重写处理。
- 不再静默进入内部自动返修。

## 验证策略

后端测试：

- Outline parser 接受合法 plan outline。
- Outline validator 拦截重复 id、缺失依赖、环形依赖、缺少追踪关系。
- Outline author prompt 输出合法 sentinel JSON，且不包含完整 Work Item 或 VerificationPlan。
- Outline reviewer prompt 不要求完整 verification plan。
- 单 item author prompt 只输出一个 WorkItemDraftCandidate。
- 单 item author prompt 生成的 `required_gates` 只能引用本 verification plan 内已定义 id。
- 串行模式生成第二个 work item 时包含第一个已确认 item 上下文。
- 串行模式支持单 item 重写。
- 串行模式当前 work item review 未通过前不能生成下一个。
- 单 item reviewer 支持 `plan_reopen_required` 并能阻断后续 item 生成。
- 自动模式按 dependency layer 调度。
- 自动模式不允许单 item 重写。
- 自动模式全部生成完成后进入整组 review。
- 自动模式整组 reviewer 不允许返回单 item rewrite 操作。
- 最终编译后仍运行严格 validator。

前端测试：

- Outline 确认后展示两个生成按钮。
- 串行模式每个 work item 独立消息气泡和确认操作。
- 串行模式每个 work item 确认后展示 reviewer 审核状态。
- 自动模式展示队列状态且只允许整组操作。
- 自动模式展示整组 review 结果，不显示单 item 重写入口。
- 刷新后可恢复 outline、已确认 work item、当前运行 item 或 batch 状态。

回归测试：

- Story Workspace 不受影响。
- Design Workspace 不受影响。
- 普通 Work Item Workspace 不受影响。
- WorkItemPlan 不再在 validator error 后静默进入自动返修 loop。
- WorkItemPlan Outline review 强制开启，即使 reviewer 开关关闭也必须审核。

## Review 规则

本方案确认三条强规则：

1. WorkItemPlan Outline review 强制开启。Outline 经人工确认 author 结果后，必须由 reviewer 审核通过，才能进入生成模式选择。
2. 串行模式 Work Item review 强制逐项执行。每个 work item 的 author 结果经用户确认后，必须 reviewer 审核通过，才能生成下一个 work item。
3. 自动连续/并行模式 Work Item review 强制整组执行。全部 work item 生成完成后，由 reviewer 审核整组结果；失败时只允许整组重写或转人工处理，不支持单项重写。

后续实现计划仍需细化 review retry 上限、人工介入入口，以及 review 与最终 strict validator 的错误归因边界。Reviewer prompt 与 finding schema 的基本契约已在 v1.2 的 Prompt 设计章节中定义，实现计划必须以该契约为基础。
