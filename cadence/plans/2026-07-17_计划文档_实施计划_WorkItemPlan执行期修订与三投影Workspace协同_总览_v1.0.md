# WorkItemPlan 执行期修订与三投影 Workspace 协同 Implementation Plan 总览

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 实现版本化 Work Item Plan、Canonical Contract 三投影、执行期 Plan Repair、Coding 自动恢复和同页面 Repair 交互，使单节点规划缺陷不再触发整组重写或无效 Coder 返工。

**Architecture:** Work Item Workspace 作为规划与修订控制平面，Coding Workspace 作为执行与恢复平面。不可变 PlanRevision/WorkItemRevision 绑定 Canonical Contract 和三种 Projection，Plan Defect 通过 PlanRepairRequest、Contract Delta、Impact Analysis 和 PlanAmendmentManifest 在两个 Workspace Session 间流转。

**Tech Stack:** Rust 2024、Serde/serde_json、Tokio、Axum/WebSocket、React 19、TypeScript、Zustand、Vitest、Playwright、Git JSON Store。

## Global Constraints

- 产品数据直接切换到 `product_data_schema_version = 2`，不实现旧 `.aria` 数据迁移、双读或双写。
- Canonical Contract 是唯一规范源；Human、Coder、Reviewer Projection 均为不可变派生 Artifact。
- HumanPresentationRevision 不得改变 Contract、Coder Projection 或 Reviewer Projection Hash。
- Coding Workspace 不得直接修改 Work Item Plan；只能创建 PlanRepairRequest 并应用已确认 Manifest。
- 第一阶段同一 Work Item Plan 同时只允许一个 Active Amendment。
- Breaking Contract 和拓扑变化必须人工确认。
- Plan Defect 不增加 `unit_rework_count`。
- Story、Design、Work Item 共用的 Workspace 协议变更必须覆盖三种 Workspace Type。
- Rust 命令禁止 `-j 1`；每个阶段必须使用对应详细计划中列出的精确 `cargo test --locked --lib ...` 过滤名。
- 前端必须使用 `pnpm`，禁止 npm/yarn。
- 每个任务遵循 TDD：失败测试、最小实现、通过验证、原子提交。

---

## 计划套件

| 阶段 | 计划文档 | 可独立验收结果 |
|---|---|---|
| P1 | `2026-07-17_计划文档_实施计划_WorkItemPlan执行期修订与三投影Workspace协同_P1_Schema与RevisionStore_v1.0.md` | Schema v2、Lineage/Revision/Handoff/Amendment 持久化 |
| P2 | `2026-07-17_计划文档_实施计划_WorkItemPlan执行期修订与三投影Workspace协同_P2_CanonicalContract与Projection_v1.0.md` | Canonical Contract、三投影、Renderer、Validation |
| P3 | `2026-07-17_计划文档_实施计划_WorkItemPlan执行期修订与三投影Workspace协同_P3_WorkItemWorkspace初始规划_v1.0.md` | Initial Planning 生成 Revision/Projection，Human Group UI 与 Presentation Revision |
| P4 | `2026-07-17_计划文档_实施计划_WorkItemPlan执行期修订与三投影Workspace协同_P4_PlanRepair与影响分析_v1.0.md` | Plan Defect、Repair Session、Delta、Impact、Amendment |
| P5 | `2026-07-17_计划文档_实施计划_WorkItemPlan执行期修订与三投影Workspace协同_P5_Coding绑定与恢复_v1.0.md` | UnitRun、Review Router、Manifest 应用、Journal 恢复与运行时 Handoff 传播 |
| P6 | `2026-07-17_计划文档_实施计划_WorkItemPlan执行期修订与三投影Workspace协同_P6_内嵌交互与端到端验收_v1.0.md` | Repair Center、统一 Timeline、Subgraph Replan、共享回归与 E2E |

## 阶段依赖

```text
P1 Schema/Revision Store
       │
       ▼
P2 Canonical Contract/Projection
       │
       ▼
P3 Work Item Initial Planning
       │
       ▼
P4 Plan Repair/Impact Analysis
       │
       ▼
P5 Coding Binding/Recovery
       │
       ▼
P6 Embedded UX/Subgraph/E2E
```

禁止跳过前置阶段后伪造接口。每阶段合并前必须运行该阶段计划列出的定向验证；P6 执行全量 Rust、前端和 E2E 门禁。

## Design 覆盖索引

| Design 能力 | 实施落点 |
|---|---|
| Schema Cutover、Lineage、不可变 Revision、Handoff、Store | P1 Task 1-4 |
| Canonical Contract、Dependency Contract Edge、三 Projection、Validation、Renderer | P2 Task 1-4 |
| 静态 Projection + 运行时 Envelope | P2 Task 4 定义 Envelope；P5 Task 1 保存 Hash/Version |
| Work Item Workspace Initial Planning、Group Overview、Contract Flow、Projection Tabs | P3 Task 1-4 |
| HumanPresentationRevision、Informative Edit、No-Invention | P2 Task 3 Validator；P3 Task 5 保存与 UI |
| Plan Defect 分类、统一路由、Fingerprint、防重复 | P4 Task 1；P5 Task 2-3 |
| Contract Delta、静态最小影响集、PlanRepairRequest、Child Session、一次确认 | P4 Task 2-4 |
| Active Amendment Lock、Publication/Application Journal、CAS 与恢复 | P1 Task 3-4；P4 Task 3-4；P5 Task 4 |
| Coding Plan Binding、UnitRun、Coder/Tester/Reviewer Defect、Manifest Apply | P5 Task 1-4 |
| Handoff Revision 差异和运行时条件传播 | P5 Task 5 |
| 内嵌 Repair Center、统一 Timeline、无频繁跳转 | P6 Task 1-2 |
| Subgraph Split/Merge、Story/Design Amendment 升级 | P6 Task 3 |
| Provider Matrix、三 Workspace 共享协议、故障恢复、固定案例 E2E | P3 Task 3；P6 Task 3-4 |

## 跨阶段稳定接口

### P1 产出

```rust
pub struct WorkItemRevisionStore;
pub struct WorkItemPlanLineage;
pub struct WorkItemPlanRevision;
pub struct LogicalWorkItem;
pub struct WorkItemDraftRevision;
pub struct WorkItemDraftRevisionState;
pub struct WorkItemRevision;
pub struct VerificationPlanRevision;
pub struct PlanValidationReportArtifact;
pub struct WorkItemProjectionBundle;
pub struct PlanProjectionBundle;
pub struct HumanPresentationRevision;
pub struct HandoffRevision;
pub struct DependencyGraphRevision;
pub struct PlanRepairRequest;
pub struct PlanAmendmentManifest;
pub struct PlanAmendmentPublicationJournal;
```

### P2 产出

```rust
pub struct CanonicalWorkItemContract;
pub struct DependencyContractEdge;
pub struct WorkItemProjectionCompiler;
pub struct HumanGroupProjection;
pub struct CoderGroupContext;
pub struct ReviewerGroupMatrix;
pub struct CoderExecutionEnvelope;
pub struct ReviewerExecutionEnvelope;
pub struct ProjectionValidationReport;
pub trait ProviderProjectionRenderer;
```

### P3 产出

```rust
pub struct InitialPlanCompileOutcome;

impl WorkspaceEngine {
    pub fn compile_initial_plan_revision(
        &mut self,
        accepted_drafts: &[WorkItemDraftRevision],
    ) -> Result<InitialPlanCompileOutcome, WorkspaceEngineError>;
}

pub fn save_human_presentation_revision(
    store: &WorkItemRevisionStore,
    plan: &WorkItemPlanLineage,
    revision: HumanPresentationRevision,
) -> Result<HumanPresentationRevision, WorkspaceEngineError>;
```

WebSocket Artifact 新增：

```rust
ArtifactPayload::WorkItemPlanProjection { projection: PlanProjectionBundleDto }
ArtifactPayload::WorkItemProjection { projection: WorkItemProjectionBundleDto }
```

### P4 产出

```rust
pub struct PlanDefectRouter;
pub struct PlanRepairEngine;
pub struct ContractImpactAnalyzer;

pub fn compute_contract_delta(
    previous_revision_id: &str,
    previous: &CanonicalWorkItemContract,
    next_revision_id: &str,
    next: &CanonicalWorkItemContract,
) -> ContractDelta;

impl PlanRepairEngine {
    pub fn prepare_amendment(
        &self,
        request: &PlanRepairRequest,
    ) -> Result<PreparedPlanAmendment, PlanRepairError>;
}
```

### P5 产出

```rust
pub struct CodingAttemptPlanBinding;
pub struct CodingUnitRun;
pub struct CodingAmendmentApplicationJournal;
pub struct RuntimeHandoffImpactPropagator;

impl CodingWorkspaceEngine {
    pub async fn apply_plan_amendment(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError>;
}
```

### P6 产出

```text
WorkItemPlanOverview
WorkItemContractFlow
WorkItemProjectionTabs
PlanRepairCenter
PlanRepairTimelineGroup
ImpactPreview
SemanticContractDiff
```

## 全局文件边界

### 新增领域目录

```text
src/product/work_item_revision_store/
src/product/work_item_contract/
src/product/work_item_projection/
src/product/plan_repair/
```

### 既有后端接入点

```text
src/product/models/
src/product/work_item_plan_store.rs
src/product/work_item_split_engine/
src/product/work_item_split_validator/
src/product/workspace_engine/
src/product/coding_models/
src/product/coding_attempt_store/
src/product/coding_workspace_engine/
src/web/workspace_ws_types/
src/web/workspace_ws_handler/
src/web/coding_ws_handler/
src/web/handlers/
src/web/types.rs
```

### 既有前端接入点

```text
web/src/api/types/work-item-plan.ts
web/src/api/types/coding.ts
web/src/components/workspace/
web/src/components/coding-workspace/
web/src/pages/ChatWorkspacePage.tsx
web/src/pages/CodingWorkspacePage.tsx
web/src/state/workspace-ws-store.ts
web/src/state/coding-workspace-store.ts
web/src/hooks/useWorkspaceWs.ts
web/src/hooks/useCodingWorkspaceWs.ts
```

## 阶段完成门禁

P1 完成时：

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib product_data_schema_
cargo test --locked --lib work_item_revision_
```

P2 完成时：

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib canonical_work_item_
cargo test --locked --lib work_item_projection_
cargo test --locked --lib provider_projection_renderer_
```

P3-P5 使用各自详细计划 Step 4 中的精确 Rust/前端定向命令；P6 执行下面的全量门禁。任何定向测试必须确认实际运行数大于 0。

P6 最终执行：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
cd web
pnpm tsc -b
pnpm test
pnpm test:e2e
```

## 提交策略

每个详细计划中的 Task 是一个独立 Review 边界。禁止把六阶段压成一个提交。推荐提交前缀：

```text
feat(work-item-revision):
feat(work-item-contract):
feat(work-item-workspace):
feat(plan-repair):
feat(coding-repair):
feat(workspace-ui):
test(plan-repair):
```

## 最终验收场景

固定端到端 Fixture：

```text
WI-01 Revision 1 缺少：
- workflow_explicit_completion
- finalization_failure
- failure_message

WI-02 正确要求这些 Capability，且禁止修改 WI-01 核心文件。
```

完成标准：

1. WI-02 Reviewer 返回 `upstream_contract_invalid`。
2. WI-02 暂停且 `unit_rework_count` 不增加。
3. 内嵌 Repair Session 创建 WI-01 Draft/WorkItem Revision 2。
4. WI-02 Draft/WorkItem Revision 保持不变。
5. Amendment 只重跑 WI-01，并让 WI-02 保留原 WorkItemRevision、创建新 UnitRun、使用 Handoff Revision 2 恢复。
6. WI-03/WI-04 不被无条件重写。
7. 页面刷新、服务重启和重复 Finding 不产生重复 Amendment 或 UnitRun。
