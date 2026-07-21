# WorkItemPlan 执行期修订与三投影 Workspace 协同技术方案

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-07-17
- 目标分支：`feat-b-0715`
- 状态：设计已确认，待实施计划拆解
- 适用范围：Work Item Workspace、Work Item Plan、Work Item Group Coding Workspace、Coding/Test/Review、Provider Projection、Workspace Session 恢复
- 历史数据策略：不兼容旧 `.aria` 业务数据，采用一次性 Schema Cutover

## 1. 背景

Cadence Aria 当前主流程为：

```text
Story → Design → Work Item Plan → Work Items → Coding → Testing → Review
```

现有 WorkItemPlan 会在 Work Item Workspace 中生成 Outline、Draft、VerificationPlan 和依赖图，Final Compile 后把真实 Work Item 交给 Work Item Group Coding Workspace 串行执行。该流程存在两个相互关联的问题。

### 1.1 执行期发现 Work Item 规划缺陷时缺少正确恢复路径

Coding 或 Review 可能发现：

- 当前 Work Item 的验收条件、写入范围或验证计划有误。
- 当前 Work Item 依赖的上游 Contract 不完整。
- Work Item 的拆分、合并或依赖关系不合理。
- Work Item 正确映射了 Design，但 Design 或 Story 本身存在问题。

现有 Coding Workspace 主要把 Review 失败路由为 Coder Rework。若问题实际位于上游 Work Item 或依赖图，当前 Coder 可能无权修改对应文件，只能反复报告 Blocker，随后再次进入 Review，形成无效返工循环。

若简单回到 Work Item Workspace 全量重新生成，又会带来：

- 未受影响 Work Item 被迫重写。
- 已完成的 Coding、Review、Commit 和 Handoff 难以复用。
- Work Item ID 和依赖关系整体变化。
- 用户需要频繁在两个 Workspace 页面之间切换。

### 1.2 Work Item Group Outline 对人类不可读

当前 Work Item Draft 中的实现上下文同时承载：

- Group 拆分说明。
- 架构和源码导航。
- Coder 实现任务。
- Reviewer 验证规则。
- Exclusive/Forbidden Write Scope。
- TDD 顺序和验证命令。
- Handoff 约束。
- 风险与 Blocker 升级规则。

单份 Artifact 同时服务人类、Coder、Reviewer 和审计系统，导致人类需要阅读大段实现级材料才能理解一个 Work Item 的目标和依赖关系。若直接把权威契约改写成更复杂的人类自然语言，又可能降低 Codex、Claude Code 等 Provider 对执行边界和审查标准的理解稳定性。

## 2. 相关既有设计与边界调整

本方案建立在以下既有设计之上：

- `2026-06-22_技术方案_WorkItemPlan两阶段生成与逐项WorkItem确认流程_v1.5.0.md`
- `2026-06-27_技术方案_WorkItemGroup级CodingWorkspace串行执行_v1.0.md`
- `2026-07-10_技术方案_共享结构化输出协议与WorkItemPlan返修路由修复_v1.2.md`

本方案调整以下旧边界：

1. Work Item Draft 不再只在 Final Compile 前修订；执行期允许通过 Work Item Repair Session 创建新 Draft Revision。
2. Coding Workspace 仍不得直接修改 Work Item Plan，但可以创建 `PlanRepairRequest`，并通过关联的 Work Item Workspace 子 Session 完成修订。
3. 已确认 Work Item 不原地覆盖；通过 Logical Work Item、WorkItemRevision 和 PlanRevision 保留完整版本链。
4. 下游不再默认整体失效；按 Contract Delta、Dependency Contract Edge 和运行时 Handoff 差异计算最小安全影响集。
5. Work Item 不再使用一份长文本同时服务所有角色；Canonical Contract 编译为 Human、Coder、Reviewer 三种 Projection。

## 3. 目标

### 3.1 必须实现

- Work Item Workspace 同时支持 Initial Planning 和 Plan Repair。
- Coding/Testing/Review 能识别实现缺陷、规划缺陷、设计缺陷和运行故障，并正确路由。
- 单个上游 Work Item 有问题时，只修订必要节点，不全量重写 Work Item Group。
- 拆分、合并或依赖变化时，只重规划受影响子图。
- 保留旧 Draft、WorkItemRevision、PlanRevision、Coding Unit Run、Review、Commit 和 Handoff。
- 人类默认阅读简洁的 Human Projection。
- Coder 和 Reviewer 使用由同一 Canonical Contract 编译出的独立 Projection。
- 不同 Provider 通过 Provider-specific Renderer 获得相同业务语义。
- 常规 Plan Repair 在 Coding Workspace 内嵌完成，不要求用户频繁跳转页面。
- Plan Amendment 发布和 Coding Binding 应用必须可恢复、幂等，不能出现半更新状态。
- Plan Defect 不增加普通 Coder Rework 次数。

### 3.2 非目标

- 不迁移或兼容旧 `.aria` Work Item、Coding Attempt 或 Workspace Session 数据。
- 不允许 Coding Workspace 直接编辑 Canonical Contract 或 Dependency Graph。
- 不允许直接编辑生成后的 Coder/Reviewer Prompt。
- 不在第一阶段支持同一 Work Item Plan 多个并发 Amendment。
- 不自动应用未经人工确认的 Breaking Contract 或拓扑变化。
- 不在本方案中实现 Work Item Group 并行 Coding。
- 不自动修改 Story 或 Design；升级后仍进入对应 Artifact Workspace 的确认流程。

## 4. 设计原则

### 4.1 不可变历史

已执行或已确认的 Revision 不允许覆盖。修订必须创建新版本，并通过 `supersedes`、`replaces` 或 `amends` 关系关联旧版本。

### 4.2 权威契约与展示分离

Canonical Contract 是唯一权威数据源。Human、Coder、Reviewer Projection 均为只读派生结果，不能反向覆盖 Contract。

### 4.3 控制平面与执行平面分离

- Work Item Workspace 是规划、修订、Plan Review 和发布控制平面。
- Coding Workspace 是执行、问题发现、暂停、Amendment 应用和恢复平面。

逻辑边界分离不等于页面跳转。常规 Repair 通过关联子 Session 内嵌展示。

### 4.4 Contract 驱动影响分析

影响范围根据发生变化的 Contract 和实际消费者计算，不按 Work Item 序号或“后续全部重做”粗暴判断。

### 4.5 Defect 先分类再路由

Provider 输出必须区分实现、验证、规划、设计和运行故障。只有实现问题进入 Coder Rework。

## 5. 总体架构

```text
Story / Design
       │
       ▼
┌──────────────────────────────────────────────┐
│ Work Item Workspace：规划与修订控制平面       │
│                                              │
│ Outline → Draft Revision → Plan Review       │
│      → Canonical Contract → Compile          │
│      → Human / Coder / Reviewer Projection   │
└──────────────────────┬───────────────────────┘
                       │ Confirm & Publish
                       ▼
┌──────────────────────────────────────────────┐
│ Coding Workspace：执行平面                    │
│                                              │
│ Plan Binding → Unit Run                      │
│      → Coding → Testing → Review             │
│      → Handoff Revision                      │
└──────────────────────┬───────────────────────┘
                       │ Plan Defect
                       ▼
              PlanRepairRequest
                       │
                       ▼
┌──────────────────────────────────────────────┐
│ Work Item Repair Session                     │
│                                              │
│ 修订当前节点 / 上游节点 / 受影响子图          │
│      → Contract Delta → Impact Analysis      │
│      → Plan Review → Amendment Manifest      │
└──────────────────────┬───────────────────────┘
                       │ Apply Amendment
                       ▼
┌──────────────────────────────────────────────┐
│ Coding Workspace                             │
│                                              │
│ 更新 Plan Binding 与 Unit Runs               │
│      → revalidate / stale / resume           │
└──────────────────────────────────────────────┘
```

## 6. Work Item Workspace 双模式

### 6.1 Initial Planning Mode

```text
Story / Design
  → Group Outline
  → Group Contract Validation
  → Work Item Draft Revisions
  → 三种 Projection Preview
  → Plan Review
  → Final Compile
  → Confirm Plan Revision 1
```

### 6.2 Plan Repair Mode

Plan Repair 可以由 Coder、Tester、Code Reviewer 或人工触发：

```text
PlanRepairRequest
  → 定位 Current / Upstream / Subgraph
  → 打开既有 Canonical Contract
  → 创建 Draft Revision
  → Contract Delta
  → 三种 Projection Preview
  → Impact Analysis
  → Plan Review
  → Confirm Amendment
```

Repair Mode 必须显示：

- 触发的 Coding Attempt、Unit Run、Review 和 Finding。
- 当前绑定的 PlanRevision 和 WorkItemRevision。
- 建议修订目标及证据。
- 新旧 Contract 的语义差异。
- 受影响 Work Item 与执行状态。
- 三种 Projection 的变化。

## 7. 版本数据模型

### 7.1 WorkItemPlan

代表同一拆分方案的长期身份：

```text
plan_id
project_id
issue_id
story_spec_refs
design_spec_refs
```

### 7.2 WorkItemPlanRevision

代表某次已确认、不可变的计划快照：

```text
plan_revision_id
plan_id
revision_no
supersedes
reason
work_item_bindings
dependency_graph_revision_id
validation_report_ref
plan_projection_bundle_id
```

仅修订 WI-01 时：

```text
Plan Revision 1
- WI-01 → WorkItemRevision 1
- WI-02 → WorkItemRevision 1

Plan Revision 2
- WI-01 → WorkItemRevision 2
- WI-02 → WorkItemRevision 1
```

### 7.3 LogicalWorkItem

表示依赖图中的稳定业务节点：

```text
logical_work_item_id
plan_id
title
lineage_created_at
```

依赖关系绑定 Logical Work Item，而不是带时间戳的物理 Artifact ID。

### 7.4 WorkItemDraftRevision

Work Item Workspace 中的可审查候选：

```text
draft_revision_id
logical_work_item_id
revision_no
supersedes
canonical_contract_candidate
revision_reason
trigger_repair_request_id
status:
  drafting
  reviewing
  changes_requested
  approved
  rejected
  compiled
```

### 7.5 WorkItemRevision

Draft Review 和 Compile 通过后产生的不可变执行产物：

```text
work_item_revision_id
logical_work_item_id
source_draft_revision_id
canonical_contract
canonical_contract_hash
work_item_projection_bundle_id
verification_plan_revision_id
status:
  active
  superseded
```

`superseded` 表示有新版本，不代表删除或执行失败。

### 7.6 WorkItemProjectionBundle

```text
work_item_projection_bundle_id
work_item_revision_id
canonical_contract_hash
projection_schema_version
compiler_version
human_projection
coder_projection
reviewer_projection
human_projection_hash
coder_projection_hash
reviewer_projection_hash
```

Projection 不是权威数据，但发布后保存不可变快照，以便复现当时人类、Coder 和 Reviewer 实际看到的内容。

### 7.7 PlanProjectionBundle

Group Outline 和 Group Final Review 需要 Plan 级三投影聚合：

```text
plan_projection_bundle_id
plan_revision_id
dependency_graph_revision_id
work_item_projection_bundle_refs
human_group_projection
coder_group_context
reviewer_group_matrix
human_group_projection_hash
coder_group_context_hash
reviewer_group_matrix_hash
compiler_version
```

- `human_group_projection` 展示 Group 目标、拆分理由、Contract Flow 和风险。
- `coder_group_context` 提供当前 Work Item 在 Group 中的位置、前后置节点和 Group 级边界，不替代单 Work Item Coder Projection。
- `reviewer_group_matrix` 用于 Group Final Review，验证跨 Work Item Handoff、整体 Design 覆盖和最终集成结果。

PlanRevision 绑定不可变 PlanProjectionBundle。单 Work Item Projection 与 Group Projection 分开保存，避免修改 Group 展示时重写所有 WorkItemRevision。

### 7.8 HumanPresentationRevision

已发布 ProjectionBundle 不允许原地修改。纯人类可读性优化通过独立 Presentation Revision 保存：

```text
human_presentation_revision_id
source_plan_projection_bundle_id
source_work_item_projection_bundle_id
supersedes
human_summary
why_split
dependency_explanation
risk_explanation
source_refs
```

Human UI 使用“不可变 Base Human Projection + 最新 HumanPresentationRevision”渲染。Presentation Revision：

- 不修改 Canonical Contract Hash。
- 不修改 Coder/Reviewer Projection。
- 不创建 WorkItemRevision 或 PlanRevision。
- 不进入 Provider 输入。
- 必须保留来源引用并通过 No-Invention Validation。

### 7.9 HandoffRevision

```text
handoff_revision_id
logical_work_item_id
work_item_revision_id
coding_unit_run_id
provided_contracts
provided_capabilities
contract_hash
commit_sha
tests
artifacts
```

下游 Unit Run 记录实际消费的 Handoff Revision，不使用未绑定的“最新 Handoff”。

## 8. Canonical Work Item Contract

Canonical Contract 保存机器可判断的规范语义：

```text
schema_version
identity
goal
non_goals
input_contracts
output_contracts
tasks
write_policy
acceptance_criteria
verification_checks
handoff_contract
blocker_rules
design_traceability
```

关键能力使用稳定 ID：

```json
{
  "contract_id": "repository_initialization_finalization",
  "capabilities": [
    "workflow_explicit_completion",
    "finalization_failure",
    "failure_message"
  ]
}
```

每个 Task、Acceptance Criterion、Verification Check 和 Blocker Rule 都必须具有稳定 ID 和来源引用。自然语言负责解释，稳定 ID 和结构字段负责执行与验证。

## 9. 三种 Projection

### 9.1 Human Projection

面向产品人员、开发者和人工 Reviewer，默认回答：

- Group 为什么这样拆。
- 每个 Work Item 做什么和不做什么。
- 谁依赖谁，依赖的具体 Contract 是什么。
- 每个 Work Item 向下游提供什么。
- 负责范围、风险、状态和版本。
- Repair 前后改了什么，影响哪些节点。

人类说明字段必须标记为非规范性：

```text
normative = false
used_by_provider = false
source_refs = [...]
```

Human Projection 分为 Plan 级和 Work Item 级。Plan 级解释 Group 拆分、Contract Flow 和整体风险；Work Item 级解释单节点目标、输入、输出和边界。纯展示优化通过 HumanPresentationRevision 叠加，不修改已发布 ProjectionBundle。

### 9.2 Coder Projection

Provider-neutral Coder Projection 包含：

```text
identity
objective
required_input_contracts
resolved_handoffs
implementation_tasks
write_policy
acceptance_criteria
verification_checks
blocker_rules
previous_actionable_review
handoff_requirements
```

固定章节顺序为：

1. Work Item 身份和 Revision。
2. 强制目标。
3. 已解析上游输入。
4. 实现任务。
5. 写入边界。
6. 验收条件。
7. 验证命令。
8. Blocker 路由。
9. Handoff 输出。
10. 当前返修意见。

强制章节不得因 Token Budget 被截断。

### 9.3 Reviewer Projection

Reviewer Projection 是验证矩阵，不复用 Coder 的长篇实现说明：

```text
requirements_matrix
scope_policy
input_contract_checks
output_contract_checks
acceptance_evidence_rules
verification_evidence_rules
plan_defect_routing
handoff_validation
```

每个 Check 必须指向 Canonical Contract 中的 Criterion、Contract 或 Capability，并声明失败路由。

本节 Reviewer Projection 面向 Coding Workspace 的 Code Reviewer。Work Item Workspace 的 Plan Reviewer 使用独立的 Plan Review Context，检查 Canonical Contract、Dependency Graph、Plan/Work Item 三投影覆盖和影响分析；它不会复用 Code Reviewer 的运行时 Diff 审查 Prompt。

### 9.4 Work Item Authoring Context

Work Item Author Provider 使用 Story、Design、Repository Context、现有 Plan Revision 和 Repair Evidence 生成 Canonical Contract Candidate。它不是第四种发布 Projection，而是权威契约的 Authoring Context。

Work Item Author Provider 不直接编辑 Human、Coder 或 Reviewer Projection。

### 9.5 Plan Review Context

Plan Reviewer 使用：

```text
Story / Design Traceability
Canonical Contract Candidates
Dependency Contract Graph
PlanProjectionBundle Candidate
WorkItemProjectionBundle Candidates
Projection Validation Report
Contract Delta
Impact Analysis
Repair Evidence
```

Plan Review Context 属于 Work Item Workspace 的 Authoring/Review 输入，不是第四种面向 Coding 消费者的发布 Projection。

## 10. Projection Compiler 与 Provider Renderer

```text
Canonical Contract
        │
        ▼
Provider-neutral Projection
        │
        ▼
Provider-specific Renderer
```

Renderer 可以调整章节格式、Structured Output 指令、Provider 权限提示和 Resume Session 表达，但不得：

- 删除强制规范项。
- 改变依赖、写入范围或 Acceptance Criteria。
- 引入 Canonical Contract 中不存在的要求。
- 改变 Blocker 路由。
- 把 informative 内容升级为 normative。

每次 Unit Run 保存：

```text
canonical_contract_hash
projection_bundle_id
projection_compiler_version
provider_renderer_version
coder_execution_context_hash
reviewer_execution_context_hash
```

### 10.1 静态 Projection 与运行时 Envelope

发布 WorkItemRevision 时保存静态 Projection Bundle。执行时增加动态证据：

```text
Published Coder Projection
+ Repository 状态
+ Handoff Revision
+ Unit Run
+ Review Findings
+ Git Commit
= Coder Execution Context
```

```text
Published Reviewer Projection
+ 实际 Diff
+ 测试证据
+ Handoff
+ Contract Delta
= Reviewer Execution Context
```

运行时内容不得改变已发布规划语义。

## 11. Projection Validation

发布前必须通过：

1. Schema Validation：结构、字段和枚举合法。
2. Coverage Validation：Canonical 规范项没有遗漏。
3. No-Invention Validation：Projection 没有新增规范。
4. Cross-Projection Validation：Coder 和 Reviewer 引用相同 Contract ID。
5. Token Budget Validation：Mandatory Section 不可截断。
6. Renderer Snapshot Validation：不同 Provider 输出可复现。
7. Structured Output Validation：Reviewer 返回统一 Defect Class。
8. Hash Validation：持久化 Hash 与实际内容一致。

Work Item Workspace 必须同时提供 Human、Coder、Reviewer Preview。Plan Reviewer 不仅审查 Canonical Contract，还要检查三种 Projection 的覆盖和一致性。

## 12. Work Item Workspace 信息架构

### 12.1 Group Overview

默认只展示：

- Group 一句话目标。
- Plan Revision 和状态。
- Work Item 数量和依赖关系。
- Blocking 风险。
- 是否可以发布。

Work Item 卡片默认展示：

```text
可读名称
一句话目标
依赖谁
向下游提供什么
负责范围摘要
当前状态
```

内部 ID、完整路径、详细验证命令和 Provider Prompt 默认折叠。

### 12.2 Contract Flow

依赖边必须显示 Contract：

```text
WI-01
  provides:
  - repository_initialization_finalization
          │
          ▼
WI-02
  requires:
  - workflow_explicit_completion
  - finalization_failure
  - failure_message
```

上下游不匹配时直接显示缺失 Capability。

### 12.3 Work Item 详情页签

- `Overview`：Human Projection。
- `Contract`：Canonical Contract 格式化视图。
- `Coder View`：Provider-neutral 和 Provider-specific 预览。
- `Reviewer View`：验证矩阵和失败路由。
- `History`：Revision、Delta、Review、Unit Run 和 Handoff。

### 12.4 编辑规则

Informative 编辑创建新的 HumanPresentationRevision，不修改已发布 ProjectionBundle，不创建执行 Revision，也不改变 Provider 输入。

Normative 编辑必须通过结构化表单修改 Canonical Contract，并自动执行：

1. 创建 Draft Revision。
2. 生成三种 Projection。
3. Projection Validation。
4. Contract Delta。
5. Impact Analysis。
6. Plan Review。

禁止直接编辑生成后的 Coder 或 Reviewer Prompt。

## 13. 无跳转 Plan Repair 交互

逻辑上创建 Work Item Repair Session，交互上默认停留在 Coding Workspace：

```text
Coding Workspace
      │ Plan Defect
      ▼
内嵌 Plan Repair Center
      │
      ▼
Work Item Repair Child Session
      │
      ▼
生成修订 → Plan Review → Impact Analysis
      │
      ▼
一次最终确认
      │
      ▼
Apply Amendment → 自动恢复 Coding
```

### 13.1 分级交互

- Level 0：Implementation Defect，普通 Coder Rework。
- Level 1：单 Work Item Revision，完全内嵌处理。
- Level 2：局部子图重规划，使用内嵌扩展 Repair Canvas。
- Level 3：Story/Design Amendment，显式升级到对应 Artifact Workspace。

只有用户主动选择“在完整 Work Item Workspace 中打开”或进入 Story/Design Amendment 时才跳转页面。

### 13.2 一次确认原则

Plan Repair 默认自动完成：

1. Defect 分类和目标定位。
2. Canonical Contract Revision 生成。
3. 三种 Projection 生成。
4. Plan Review。
5. Contract Delta 和影响分析。

最后只展示一次确认页。Breaking Contract 和拓扑变化必须人工确认，informative-only 变化不要求单独确认。

### 13.3 统一 Timeline

```text
Coding WI-02
  └─ Code Review：Blocked by Plan Defect
      └─ Plan Repair WI-01
          ├─ Revision Generated
          ├─ Projection Validation
          ├─ Plan Review Passed
          └─ Amendment Confirmed
              └─ Coding WI-01 Revision 2
```

子 Session 独立持久化，但事件通过父 Coding Timeline 连续展示。

## 14. Coding Attempt 与 Unit Run

### 14.1 CodingAttemptPlanBinding

```text
attempt_id
bound_plan_revision_id
applied_amendment_ids
```

Plan Revision 不能静默替换，必须通过 `ApplyPlanAmendment` 更新绑定。

### 14.2 CodingUnit 与 CodingUnitRun

```text
CodingUnit
- logical_work_item_id
- work_item_revision_id
- dependency information
```

```text
CodingUnitRun
- unit_run_id
- execution_no
- resolved_handoff_revisions
- coder_projection_hash
- reviewer_projection_hash
- start_commit
- completion_commit
- rework_count
- status
```

旧 completed Unit Run 不原地改回 running。修订后创建新 Revision 或新 Unit Run：

```text
WI-01 Revision 1
  └─ UnitRun 1：completed，后被发现 Contract 缺陷

WI-01 Revision 2
  └─ UnitRun 1：pending

WI-02 Revision 1
  ├─ UnitRun 1：blocked_by_plan_defect
  └─ UnitRun 2：等待 Handoff Revision 2
```

### 14.3 Unit Run 状态

```text
pending
running
completed
failed
blocked
blocked_by_plan_defect
awaiting_amendment
needs_revalidation
stale
superseded
```

## 15. Plan Defect 分类与路由

统一分类：

| Defect Class | 含义 | 默认路由 |
|---|---|---|
| `implementation_defect` | Contract 正确，实现错误 | Coder Rework |
| `verification_incomplete` | 验证证据不足 | Tester/Coder 或 Operational Gate |
| `current_work_item_invalid` | 当前 Work Item Contract 错误 | Repair Current Work Item |
| `upstream_contract_invalid` | 上游交付不满足当前依赖 | Repair Upstream Work Item |
| `dependency_graph_invalid` | 拆分或依赖图错误 | Subgraph Replan |
| `design_amendment_required` | Design 本身需要修改 | Design Workspace |
| `story_amendment_required` | Story 本身需要修改 | Story Workspace |
| `operational_blocker` | Provider、权限、Git、环境问题 | Retry/Operational Gate |

Coder 和 Tester 也可以返回结构化 Plan Defect，不需要伪造完成结果。

### 15.1 Reviewer Structured Output

```json
{
  "verdict": "blocked",
  "findings": [
    {
      "finding_id": "finding_finalization_contract",
      "severity": "error",
      "defect_class": "upstream_contract_invalid",
      "reason_code": "upstream_contract_capability_missing",
      "message": "上游状态机无法表达 finalization 失败",
      "contract_refs": [
        "repository_initialization_finalization"
      ],
      "capability_refs": [
        "workflow_explicit_completion",
        "finalization_failure"
      ],
      "repair_target": {
        "kind": "upstream_work_item",
        "logical_work_item_id": "wi_initialization_core",
        "work_item_revision_id": "work_item_revision_0001"
      },
      "recommended_route": "plan_repair",
      "confidence": "high",
      "evidence": []
    }
  ]
}
```

Reviewer 只能建议路由，Plan Defect Router 必须验证 Target、Contract、Capability、Evidence 和当前 Reviewer Projection。

### 15.2 自动路由

- 高置信度且证据完整：自动暂停 Unit，创建 PlanRepairRequest，打开 Repair Center。
- 中置信度：只询问一次 Repair Target 或问题层级。
- 低置信度：进入 Human Triage，不自动修改 Plan。

混合 Findings 先处理 Story/Design 和 Plan Defect，再处理实现与验证问题。Plan 修订后重新生成 Reviewer Projection，再判断旧 Findings 是否仍有效。

### 15.3 防重复循环

Finding Fingerprint 由以下字段生成：

```text
plan_revision_id
defect_class
repair_target
contract_refs
capability_refs
normalized_reason_code
```

相同问题重复出现时追加 Evidence，不创建新的 Repair Request，也不增加 Coder Rework 次数。

计数器分离：

```text
unit_rework_count
verification_retry_count
operational_retry_count
plan_repair_count
```

## 16. Contract Delta 与影响分析

### 16.1 Dependency Contract Edge

依赖边必须携带具体 Contract：

```json
{
  "from": "wi_initialization_core",
  "to": "wi_registration_api",
  "required_contracts": [
    {
      "contract_id": "repository_initialization_finalization",
      "required_capabilities": [
        "workflow_explicit_completion",
        "finalization_failure",
        "failure_message"
      ],
      "compatibility_policy": "require_all"
    }
  ]
}
```

### 16.2 Delta 分类

- `informative_only`：只影响人类展示，不影响执行。
- `implementation_guidance`：当前 Work Item 重新执行或验证，下游默认不受影响。
- `compatible_contract_extension`：当前 Work Item 重新执行，明确需要新增能力的下游重新绑定并验证。
- `breaking_contract_change`：直接消费者 stale 或 needs_revalidation。
- `topology_change`：进入 Subgraph Replan。

### 16.3 两阶段影响分析

静态阶段根据 Contract Delta 和 Dependency Edge 计算：

```text
unaffected
direct_revalidation
direct_stale
conditional_downstream
```

运行时阶段在新 Handoff 生成后比较 old/new Handoff Contract Hash：

- 输出未变化：停止传播。
- 兼容扩展：下游重新验证输入。
- 破坏性变化：下游标记 stale。
- 下游重新执行后输出也变化：继续向后传播。

用户可以扩大重做范围，但不能无理由缩小系统计算出的最小安全影响集。缩小范围必须记录风险接受说明并通过 Plan Review。

## 17. 局部子图重规划

只有拆分、合并或依赖变化时重规划子图。

```text
不受影响前置节点
        │ Input Boundary
        ▼
┌────────────────────┐
│ 需要重规划的子图    │
└────────────────────┘
        │ Output Boundary
        ▼
不受影响后续节点
```

新子图必须继续满足：

- Input Boundary：能消费未受影响上游提供的 Contract。
- Output Boundary：能为未受影响下游提供其所需 Contract。

无法满足边界时逐层扩大影响范围。只有扩展到整张图或 Story/Design 根本变化时才执行 Full Replan。

拆分、合并必须保存一对多或多对一的替代映射，旧节点不删除。

## 18. PlanRepairRequest 与 Amendment Manifest

### 18.1 PlanRepairRequest

```text
repair_request_id
trigger_attempt_id
trigger_unit_run_id
trigger_review_id
trigger_finding_id
defect_class
suspected_logical_work_item_id
suspected_revision_id
contract_refs
capability_refs
evidence
recommended_repair_scope
fingerprint
```

### 18.2 PlanAmendmentManifest

```text
amendment_id
previous_plan_revision_id
new_plan_revision_id
revised_work_items
superseded_revisions
dependency_graph_changes
contract_deltas
unaffected_units
revalidation_required_units
stale_units
replacement_units
resume_target
```

Coding Workspace 只消费已确认 Manifest，不自行推断回退范围。

## 19. 跨 Workspace Session 协议

```text
CodingSession
  └─ relation: plan_repair
       └─ WorkItemRepairSession
```

关联记录必须保存：

```text
link_id
parent_session
child_session
trigger_attempt/unit/review/finding
return_context
timeline_anchor_id
```

子 Session 事件保留来源 Workspace 和 Session ID，但通过父 Coding Timeline 展示。

### 19.1 幂等请求

同一 Plan Revision、Defect Class、Repair Target、Contract 和 Capability 生成稳定 Fingerprint。已有 Open Request 时追加 Evidence，不重复创建 Workspace。

### 19.2 锁

第一阶段同一个 Work Item Plan 同时只允许一个 Active Amendment。Coding Attempt 在 `awaiting_plan_amendment` 时禁止启动新的 Coder、Tester 或 Reviewer Run。

Work Item Repair Session 只修改规划数据，不持有 Git Worktree 写锁。Amendment 应用后，Issue Worktree 执行权转移给新的 Unit Run。

未提交 Diff 存在时禁止自动 reset，必须先创建安全 Checkpoint 并按 Rollback 规则处理。

## 20. Amendment 事务与恢复

Amendment 状态：

```text
drafting
→ reviewed
→ awaiting_confirmation
→ amendment_prepared
→ plan_published
→ coding_binding_applied
→ completed
```

发布顺序：

1. 保存 Draft、Contract 和 Projection。
2. 完成全部 Validation。
3. 保存 Impact Report。
4. 保存 Amendment Manifest。
5. 标记 `amendment_prepared`。
6. 人工确认。
7. 发布新 Plan Revision。
8. Coding Workspace 幂等应用 Manifest。
9. 标记 `completed`。

确认发布时使用 `base_plan_revision_id` 做乐观并发检查。Base 已变化时进入 `amendment_conflict`，必须 Rebase、重新生成 Delta/Projection/Impact 并重新 Review，禁止 Last Write Wins。

Coding Binding 应用必须使用 Journal。任一步失败时 Attempt 进入 `amendment_apply_failed`，不能在部分绑定状态继续执行。

服务重启时：

- 未发布 Repair Session 从最近 Checkpoint 恢复。
- Plan 已发布但 Coding 未应用时，若用户已确认则自动继续幂等应用。
- Coding 应用中断时按 Journal 继续未完成步骤。
- 页面断开不删除 Repair Session。

## 21. 当前案例的预期行为

场景：

```text
WI-01 Revision 1 缺少：
- workflow_explicit_completion
- finalization_failure
- failure_message

WI-02 正确要求这些 Capability，且禁止修改 WI-01 核心文件。
```

预期：

1. WI-02 Coder 或 Reviewer 返回 `upstream_contract_invalid`。
2. Plan Defect Router 暂停 WI-02，不增加 Coder Rework 次数。
3. Coding Workspace 内嵌打开 Work Item Repair Session。
4. 创建 WI-01 Draft Revision 2，WI-02 Draft 不重写。
5. WI-01 Revision 2 补齐缺失 Capability。
6. Dependency Graph 不变化。
7. Impact Analysis 判定 WI-01 重新执行，WI-02 重新绑定并验证，其他节点暂不受影响。
8. 用户一次确认 Amendment。
9. Coding Attempt 绑定 Plan Revision 2，创建 WI-01 新 Unit Run。
10. WI-01 完成并生成 Handoff Revision 2。
11. WI-02 使用原 WorkItemRevision、新 Unit Run 和 Handoff Revision 2 恢复。
12. WI-03/WI-04 不重写；是否受影响由后续 Handoff 差异决定。

## 22. 新数据目录

```text
issues/{issue_id}/
├── work-item-plans/
│   └── {plan_id}/
│       ├── plan.json
│       ├── revisions/{plan_revision_id}/
│       ├── human-presentation-revisions/
│       ├── repair-requests/
│       └── amendments/
├── logical-work-items/
│   └── {logical_work_item_id}/
│       ├── lineage.json
│       ├── draft-revisions/
│       ├── work-item-revisions/
│       ├── human-presentation-revisions/
│       └── handoff-revisions/
├── workspace-sessions/
│   ├── work-item/
│   └── coding/
└── coding-attempts/
    └── {attempt_id}/
        ├── attempt.json
        ├── plan-binding.json
        ├── amendment-applications/
        └── units/{unit_id}/runs/
```

## 23. Schema Cutover

新增：

```json
{
  "product_data_schema_version": 2
}
```

启动规则：

- 无 `.aria` 业务数据：创建 Schema v2。
- Schema v2：正常加载。
- 旧数据：返回 `product_data_schema_unsupported`，要求用户清理或归档。

不实现旧 Artifact Reader、迁移脚本、双读、双写或兼容 DTO。

## 24. Story、Design、Work Item 共享链路

本方案的三 Projection 仅适用于 Work Item，但以下能力属于共享 Workspace 基础设施：

- Reviewer Structured Output 语法与诊断。
- Parent/Child Session Link。
- Timeline 恢复与 Artifact Version Binding。
- Revision、Human Confirm 和 Provider Run 恢复。
- Story/Design Amendment 升级入口。

实现共享协议时必须同时验证 Story、Design、Work Item Workspace。若某项行为只适用于 Work Item，例如 Contract Projection 或 Plan Amendment，测试必须明确说明 Story/Design 不适用的原因。

## 25. 测试策略

### 25.1 Canonical Contract

- Input/Output Contract、Task、AC、Write Policy、Verification、Handoff 和 Blocker Schema。
- Stable ID 和引用完整性。
- Contract Hash 稳定性。
- Informative 字段不影响 Contract Hash。

### 25.2 Projection Compiler

- Coverage、No-Invention、Cross-Projection。
- Mandatory Section 不被截断。
- 相同 Contract 生成稳定 Projection Hash。
- Human Summary 只引用真实 Contract。
- PlanProjectionBundle 与 WorkItemProjectionBundle 的 Contract 引用一致。
- HumanPresentationRevision 不改变 Contract、Coder Projection 或 Reviewer Projection Hash。

### 25.3 Provider Matrix

对 Codex、Claude Code、Fake 覆盖 Work Item Author、Plan Reviewer、Coder 和 Code Reviewer：

- Renderer 不丢失 Contract ID。
- Structured Output Schema 一致。
- 相同缺陷得到相同 Defect Class。
- Provider 输出异常安全进入修复或人工 Gate。

### 25.4 Initial Planning

- Group Contract 覆盖 Story/Design。
- Dependency Edge 有明确 Contract。
- Required Capability 有提供者。
- Scope 不冲突、依赖图无环。
- 三种 Projection 通过后才允许 Compile。

### 25.5 Plan Repair

- Implementation Defect 只进入 Coder Rework。
- Current Work Item Revision。
- Upstream Contract Revision。
- Compatible、Breaking 和 Topology Delta。
- Handoff 未变化时影响传播停止。
- Work Item 拆分、合并、依赖调整。
- Story/Design Amendment 升级。
- 重复 Finding 合并。
- Plan Defect 不增加 Coder Rework 次数。

### 25.6 事务与恢复

在 Draft 保存、Projection 生成、Plan Review、Amendment Prepared、Plan Published、Coding Binding 和 Unit Run 创建等边界模拟进程退出，验证：

- 不产生半发布 Plan。
- Attempt 不绑定不存在的 Revision。
- 幂等恢复不创建重复 Unit。
- 已确认 Amendment 可以继续应用。
- 未确认 Amendment 不自动发布。

### 25.7 并发

- 同一 Plan 同时只有一个 Active Amendment。
- 重复 Request 合并。
- Base Revision 变化触发 Conflict/Rebase。
- Awaiting Amendment 时不能启动新 Provider Run。
- 未提交 Diff 时不能自动回退。

### 25.8 UI

- Group 页面默认不展示长篇实现上下文。
- Work Item 卡片能展示目标、输入、输出和依赖。
- Contract Edge 能显示缺失 Capability。
- 三 Projection 使用同一 Contract Hash。
- Informative 编辑只创建 HumanPresentationRevision，不改变任何 Provider 输入。
- Repair Center 与完整 Work Item Workspace 展示一致。
- Semantic Diff 和 Impact Preview 正确。
- 页面刷新后恢复 Repair Session。

### 25.9 当前案例回归

固定覆盖“WI-01 缺少 finalization Capability，WI-02 正确阻塞”的完整链路，验证只修订 WI-01、恢复 WI-02、其他节点不被无条件重写。

## 26. 验收标准

### 26.1 人类可理解

用户在 Group Overview 中无需阅读 Coder Prompt，即可回答：

- Group 为什么这样拆。
- 每个 Work Item 做什么。
- 谁依赖谁。
- 依赖的具体 Contract 是什么。
- 哪个节点存在风险或缺口。

### 26.2 Agent 可执行

- Coder Projection 的 Task 全部来自 Canonical Contract。
- Reviewer Projection 覆盖所有 Acceptance Criteria。
- Provider Renderer 只调整表达，不改变业务规范。
- 任意执行都能复现当时的 Contract、Projection 和运行时 Envelope。

### 26.3 Plan 可局部修复

- 单节点问题不重写整个 Group。
- 上游问题自动暂停下游。
- 影响分析基于 Contract，而不是顺序。
- 未受影响节点和执行结果继续复用。
- 拆分、合并和依赖变化只重规划必要子图。
- Full Replan 仅用于 Story/Design 根本变化或边界无法闭合。

### 26.4 交互连续

- 常规 Plan Repair 不跳出 Coding Workspace。
- Work Item Repair 使用关联 Child Session。
- 用户通常只做一次 Amendment 确认。
- Amendment 完成后自动恢复 Coding。
- Timeline 能解释执行指针为何回到上游。

### 26.5 状态可靠

- 所有 Revision 不可变。
- 发布和应用可恢复、可重试、幂等。
- Plan Defect、代码返工、验证重试和运行故障分别计数。
- 不出现 Reviewer/Coder 无限循环。
- 不出现 Plan 已更新而 Coding 仍使用旧 Projection 的状态。

## 27. 实施拆分建议

正式实施计划建议按以下能力边界拆分，但具体 Work Item 应在本 Design 评审通过后另行生成：

1. Schema v2、Lineage、Revision 和新 Store。
2. Canonical Contract 与 Dependency Contract Edge。
3. 三 Projection Compiler、Validator 和 Provider Renderer。
4. Work Item Workspace Initial Planning 新流程与 Human Projection UI。
5. Plan Defect Structured Output 与 Router。
6. Plan Repair Session、Contract Delta 和 Impact Analysis。
7. Coding Unit Run、Plan Binding 和 Amendment Application。
8. 内嵌 Repair Center、跨 Session Timeline 和恢复。
9. Subgraph Replan 与 Story/Design Amendment 升级。
10. Provider Matrix、故障恢复、端到端回归和验收。

## 28. 决策摘要

1. 使用不可变 PlanRevision、WorkItemRevision 和 HandoffRevision，不原地覆盖历史。
2. Logical Work Item 作为依赖图稳定身份，PlanRevision 绑定具体 WorkItemRevision。
3. Canonical Contract 是唯一权威数据，编译 Human、Coder、Reviewer 三种 Projection。
4. Work Item Workspace 同时负责首次规划和执行期 Plan Repair。
5. Coding Workspace 只发现问题、暂停、创建 Request、应用 Manifest 和恢复执行。
6. 常规 Repair 使用内嵌 Child Session，避免频繁页面切换。
7. Reviewer、Coder、Tester 必须返回统一 Defect Class，Plan Defect 不进入普通 Coder Rework。
8. 影响范围按 Contract 和 Handoff 差异计算；拓扑变化只重规划受影响子图。
9. 第一阶段同一 Plan 只允许一个 Active Amendment。
10. 不兼容历史 `.aria` 数据，直接切换 Schema v2。
