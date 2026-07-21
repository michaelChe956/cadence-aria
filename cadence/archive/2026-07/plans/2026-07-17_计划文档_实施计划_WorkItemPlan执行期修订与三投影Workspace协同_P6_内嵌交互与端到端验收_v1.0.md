# P6 内嵌 Repair 交互与端到端验收 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 完成 Coding Workspace 内嵌 Plan Repair Center、跨 Session Timeline、局部子图重规划、Story/Design 升级和完整端到端验收。

**Architecture:** 前端 Coding store 同时保存父 Coding Session 和只读 Child Repair Session Projection；Repair UI 复用 Work Item Workspace 的 Overview/Contract/Projection 组件。后端 Subgraph Replanner 保持 Input/Output Boundary；Story/Design 升级复用共享 Session Link 和 Artifact Workspace 协议。

**Tech Stack:** React、TypeScript、Zustand、Vitest、Playwright、Rust Workspace/Coding Engine、Axum WebSocket。

## Global Constraints

- 常规 Level 1/2 Repair 不切换路由。
- 用户通常只在最终 Amendment 阶段确认一次。
- Repair Center 和完整 Work Item Workspace 必须使用同一 DTO 与组件。
- Story/Design/Work Item 共享协议测试必须三类型覆盖。
- Full Replan 只在边界无法闭合或 Story/Design 根本变化时触发。
- 最终必须运行全量 Rust、前端和 E2E 门禁。

---

### Task 1: 前端 Repair Session 状态与 WebSocket 聚合

**Files:**
- Modify: `web/src/api/types/coding.ts`
- Modify: `web/src/api/types/work-item-plan.ts`
- Modify: `web/src/state/coding-workspace-store.ts`
- Modify: `web/src/state/coding-workspace-store.test.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.ts`
- Modify: `web/src/hooks/useCodingWorkspaceWs.test.tsx`
- Modify: `web/src/hooks/useWorkspaceWs.ts`
- Modify: `web/src/state/workspace-ws-store.ts`
- Create: `web/src/state/plan-repair-session.ts`
- Create: `web/src/state/plan-repair-session.test.ts`

**Interfaces:**
- Consumes: P4/P5 WS 消息
- Produces: `activePlanRepair`、Child Session Snapshot、统一 Timeline Entries

- [ ] **Step 1: 写 Store 聚合与刷新恢复测试**

```ts
it("links a plan repair child session without changing the coding address", () => {
  const store = createCodingWorkspaceStore();
  store.getState().setSessionState(codingSessionFixture());
  store.getState().setPlanRepairRequired(planRepairRequiredFixture());

  expect(store.getState().attemptId).toBe("coding_attempt_0001");
  expect(store.getState().activePlanRepair?.childSessionId).toBe(
    "workspace_session_repair_0001",
  );
  expect(store.getState().status).toBe("awaiting_plan_amendment");
});

it("restores the linked repair snapshot after reconnect", () => {
  const store = createCodingWorkspaceStore();
  store.getState().setSessionState(
    codingSessionFixture({
      linked_plan_repair: planRepairSnapshotFixture(),
    }),
  );

  expect(store.getState().activePlanRepair?.stage).toBe(
    "awaiting_confirmation",
  );
  expect(store.getState().timelineNodes).toEqual(
    expect.arrayContaining([
      expect.objectContaining({ id: "plan_repair_node_0001" }),
    ]),
  );
});
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cd web
pnpm test -- coding-workspace-store plan-repair-session useCodingWorkspaceWs
```

Expected: FAIL，Store 没有 Repair 状态和消息处理。

- [ ] **Step 3: 实现类型和 Store Actions**

```ts
import type {
  DependencyContractEdge,
  PlanProjectionBundle,
} from "../api/types/work-item-plan";

export type RepairTargetKind =
  | "current_work_item"
  | "upstream_work_item"
  | "subgraph";

export type PlanDefectClass =
  | "implementation_defect"
  | "verification_incomplete"
  | "current_work_item_invalid"
  | "upstream_contract_invalid"
  | "dependency_graph_invalid"
  | "design_amendment_required"
  | "story_amendment_required"
  | "operational_blocker";

export type RepairTarget = {
  kind: RepairTargetKind;
  logical_work_item_ids: string[];
  work_item_revision_ids: string[];
};

export type PlanRepairRequest = {
  id: string;
  plan_id: string;
  base_plan_revision_id: string;
  trigger_attempt_id: string;
  trigger_unit_run_id: string;
  trigger_review_id: string | null;
  trigger_finding_id: string;
  amendment_id: string | null;
  defect_class: PlanDefectClass;
  reason_code: string;
  repair_target: RepairTarget;
  contract_refs: string[];
  capability_refs: string[];
  evidence: Array<{
    kind: string;
    source_ref: string;
    message: string;
  }>;
  fingerprint: string;
  status:
    | "open"
    | "in_progress"
    | "awaiting_confirmation"
    | "published"
    | "applied"
    | "cancelled"
    | "failed";
  created_at: string;
  updated_at: string;
};

export type WorkspaceSessionLink = {
  id: string;
  relation: "plan_repair" | "story_amendment" | "design_amendment";
  parent_session_id: string;
  child_session_id: string;
  trigger: {
    attempt_id: string;
    unit_run_id: string;
    review_id: string | null;
    finding_id: string;
  };
  return_context: {
    original_attempt_id: string;
    original_unit_run_id: string;
    timeline_anchor_id: string;
    original_route: string;
  };
  created_at: string;
};

export type ContractDeltaKind =
  | "informative_only"
  | "implementation_guidance"
  | "compatible_contract_extension"
  | "breaking_contract_change"
  | "topology_change";

export type PlanAmendmentManifest = {
  id: string;
  repair_request_id: string;
  previous_plan_revision_id: string;
  new_plan_revision_id: string;
  revised_work_items: Record<string, {
    previous_revision_id: string;
    next_revision_id: string;
    delta_kind: ContractDeltaKind;
  }>;
  superseded_revisions: string[];
  dependency_graph_changes: Array<{
    kind: "edge_added" | "edge_removed" | "edge_replaced";
    previous: DependencyContractEdge | null;
    next: DependencyContractEdge | null;
  }>;
  contract_deltas: Array<{
    logical_work_item_id: string;
    previous_revision_id: string;
    next_revision_id: string;
    kind: ContractDeltaKind;
    added_contracts: string[];
    removed_contracts: string[];
    added_capabilities: string[];
    removed_capabilities: string[];
    changed_capabilities: string[];
    acceptance_changed: boolean;
    verification_changed: boolean;
    write_policy_changed: boolean;
  }>;
  unaffected_units: string[];
  revalidation_required_units: string[];
  stale_units: string[];
  replacement_units: Record<string, string[]>;
  resume_target: {
    logical_work_item_id: string;
    mode: "reexecute" | "revalidate" | "await_handoff";
  };
  created_at: string;
};

export type PlanRepairSessionState = {
  request: PlanRepairRequest;
  link: WorkspaceSessionLink;
  stage:
    | "triaging"
    | "authoring_revision"
    | "validating_contract"
    | "generating_projections"
    | "plan_review"
    | "awaiting_confirmation"
    | "published"
    | "amendment_conflict"
    | "applying_amendment"
    | "amendment_apply_failed"
    | "completed"
    | "failed";
  projection: PlanProjectionBundle | null;
  amendment: PlanAmendmentManifest | null;
  timelineNodes: CodingTimelineNode[];
  error: string | null;
};

export interface CodingWorkspaceActions {
  setPlanRepairRequired(message: PlanRepairRequiredMessage): void;
  updatePlanRepairSession(snapshot: PlanRepairSessionSnapshot): void;
  setPlanAmendment(amendment: PlanAmendmentManifest): void;
  clearPlanRepairAfterResume(amendmentId: string): void;
}
```

`useCodingWorkspaceWs` 处理：

```text
plan_repair_required
plan_repair_session_state
plan_repair_timeline_node_created
plan_repair_timeline_node_updated
plan_amendment_updated
plan_amendment_applied
```

不得把 Child Session ID 写入当前 Coding Route Address。

- [ ] **Step 4: 运行 Store/Hook 测试**

Run:

```bash
cd web
pnpm tsc -b
pnpm test -- coding-workspace-store plan-repair-session useCodingWorkspaceWs
```

Expected: 类型检查和聚合/重连测试通过。

- [ ] **Step 5: 提交**

```bash
git add web/src/api/types web/src/state web/src/hooks
git commit -m "feat(workspace-ui): aggregate linked plan repair sessions"
```

### Task 2: Plan Repair Center、Semantic Diff 与统一 Timeline

**Files:**
- Create: `web/src/components/coding-workspace/PlanRepairCenter.tsx`
- Create: `web/src/components/coding-workspace/PlanRepairCenter.test.tsx`
- Create: `web/src/components/coding-workspace/SemanticContractDiff.tsx`
- Create: `web/src/components/coding-workspace/SemanticContractDiff.test.tsx`
- Create: `web/src/components/coding-workspace/ImpactPreview.tsx`
- Create: `web/src/components/coding-workspace/ImpactPreview.test.tsx`
- Create: `web/src/components/coding-workspace/PlanRepairTimelineGroup.tsx`
- Create: `web/src/components/coding-workspace/PlanRepairTimelineGroup.test.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.gates.test.tsx`
- Modify: `web/src/pages/CodingWorkspacePage.reports.test.tsx`
- Modify: `web/src/components/coding-workspace/CodingTimeline.tsx`

**Interfaces:**
- Consumes: Task 1 Repair state、P3 Work Item Projection 组件
- Produces: 内嵌 Repair UI、一次确认 Action

- [ ] **Step 1: 写无跳转和一次确认测试**

```tsx
it("renders plan repair inline and preserves the coding workspace route", async () => {
  renderCodingWorkspace({
    session: codingSessionAwaitingPlanRepairFixture(),
  });

  expect(screen.getByRole("heading", { name: "Plan Repair" })).toBeInTheDocument();
  expect(screen.getByText("WI-01 初始化领域模型")).toBeInTheDocument();
  expect(screen.getByText("新增 failure_message")).toBeInTheDocument();
  expect(window.location.pathname).toContain("/coding-attempts/coding_attempt_0001");
  expect(window.location.pathname).not.toContain("workspace_session_repair_0001");
});

it("shows one amendment confirmation with impact and projection diffs", () => {
  render(<PlanRepairCenter state={repairAwaitingConfirmationFixture()} />);

  expect(screen.getByRole("button", {
    name: "确认修订并恢复执行",
  })).toBeInTheDocument();
  expect(screen.getByText("WI-01：重新执行")).toBeInTheDocument();
  expect(screen.getByText("WI-02：重新验证")).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "确认 Coder Projection" }))
    .not.toBeInTheDocument();
});
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cd web
pnpm test -- PlanRepairCenter SemanticContractDiff ImpactPreview PlanRepairTimelineGroup CodingWorkspacePage.gates
```

Expected: FAIL，新组件不存在。

- [ ] **Step 3: 实现内嵌组件**

`PlanRepairCenter` 页签：

```ts
type PlanRepairTab =
  | "summary"
  | "contract_diff"
  | "coder_diff"
  | "reviewer_diff"
  | "impact"
  | "evidence";
```

默认 `summary` 使用 P3 的 Human Projection 组件。最终操作只包含：

```text
确认修订并恢复执行
要求重新生成
调整修订范围
取消修订
在完整 Work Item Workspace 中打开
```

`CodingTimeline` 把 Child Session 节点包在 `PlanRepairTimelineGroup` 中，不复制 Provider Stream 内容到父 Store，只保存可见 Projection。

- [ ] **Step 4: 运行组件测试**

Run:

```bash
cd web
pnpm tsc -b
pnpm test -- PlanRepairCenter SemanticContractDiff ImpactPreview PlanRepairTimelineGroup CodingWorkspacePage
```

Expected: 无跳转、一次确认、Timeline 分组和刷新恢复测试通过。

- [ ] **Step 5: 提交**

```bash
git add web/src/components/coding-workspace web/src/pages/CodingWorkspacePage.tsx web/src/pages/CodingWorkspacePage.gates.test.tsx web/src/pages/CodingWorkspacePage.reports.test.tsx
git commit -m "feat(workspace-ui): embed plan repair in coding workspace"
```

### Task 3: Subgraph Replan 与 Story/Design 升级

**Files:**
- Create: `src/product/plan_repair/subgraph.rs`
- Modify: `src/product/plan_repair/mod.rs`
- Modify: `src/product/plan_repair/engine.rs`
- Modify: `src/product/plan_repair/tests.rs`
- Modify: `src/product/workspace_engine/plan_repair.rs`
- Modify: `src/product/workspace_engine/types.rs`
- Modify: `src/web/workspace_ws_types/in_.rs`
- Modify: `src/web/workspace_ws_types/out.rs`
- Create: `src/product/workspace_engine/tests/part_22.rs`
- Modify: `src/product/workspace_engine/tests.rs`
- Modify: `src/web/workspace_ws_handler/tests.rs`

**Interfaces:**
- Produces: `SubgraphReplanResult`、Story/Design Amendment Session Link
- Consumed by: Plan Repair UI and Amendment Publisher

- [ ] **Step 1: 写边界扩张、拆分和三 Workspace 类型测试**

```rust
#[test]
fn plan_repair_subgraph_replan_preserves_unchanged_boundaries() {
    let graph = subgraph_fixture("wi_a -> wi_b -> wi_c -> wi_d");
    let request = split_request("wi_b", ["wi_b1", "wi_b2"]);

    let result = SubgraphReplanner::default()
        .replan(&graph, &request)
        .unwrap();

    assert_eq!(result.input_boundary, vec!["wi_a"]);
    assert_eq!(result.output_boundary, vec!["wi_c"]);
    assert_eq!(
        result.replacement_mapping["wi_b"],
        vec!["wi_b1", "wi_b2"]
    );
    assert!(!result.affected_logical_work_items.contains(&"wi_d".to_string()));
}

#[test]
fn plan_repair_subgraph_expands_when_output_boundary_is_not_satisfied() {
    let result = SubgraphReplanner::default()
        .replan(
            &subgraph_fixture("wi_a -> wi_b -> wi_c"),
            &replacement_missing_output_contract("wi_b"),
        )
        .unwrap();

    assert!(result.affected_logical_work_items.contains(&"wi_c".to_string()));
}

#[test]
fn workspace_session_link_roundtrips_for_story_design_and_work_item_relations() {
    for relation in [
        WorkspaceSessionRelation::StoryAmendment,
        WorkspaceSessionRelation::DesignAmendment,
        WorkspaceSessionRelation::PlanRepair,
    ] {
        let value = workspace_session_link_fixture(relation);
        assert_eq!(
            serde_json::from_value::<WorkspaceSessionLink>(
                serde_json::to_value(&value).unwrap()
            )
            .unwrap(),
            value
        );
    }
}

#[test]
fn linked_workspace_timeline_and_artifact_binding_restore_for_all_artifact_types() {
    for (workspace_type, relation) in [
        (WorkspaceType::Story, WorkspaceSessionRelation::StoryAmendment),
        (WorkspaceType::Design, WorkspaceSessionRelation::DesignAmendment),
        (WorkspaceType::WorkItem, WorkspaceSessionRelation::PlanRepair),
    ] {
        let before = linked_workspace_snapshot_fixture(workspace_type.clone(), relation);
        let after = restart_and_restore_linked_workspace(before.clone());

        assert_eq!(after.workspace_type, workspace_type);
        assert_eq!(after.artifact_version_id, before.artifact_version_id);
        assert_eq!(after.timeline_nodes, before.timeline_nodes);
        assert_eq!(after.selected_timeline_node_id, before.selected_timeline_node_id);
        assert_eq!(after.human_confirm_state, before.human_confirm_state);
    }
}
```

- [ ] **Step 2: 运行测试确认失败**

Run:

```bash
cargo test --locked --lib plan_repair_subgraph_
cargo test --locked --lib workspace_session_link_roundtrips_
```

Expected: FAIL，Subgraph Replanner 和升级路由未实现。

- [ ] **Step 3: 实现 Subgraph Replanner 和升级路由**

```rust
pub struct SubgraphReplanResult {
    pub input_boundary: Vec<String>,
    pub output_boundary: Vec<String>,
    pub affected_logical_work_items: Vec<String>,
    pub replacement_mapping: BTreeMap<String, Vec<String>>,
    pub dependency_graph_revision: DependencyGraphRevision,
    pub full_replan_required: bool,
}

impl SubgraphReplanner {
    pub fn replan(
        &self,
        graph: &DependencyContractGraph,
        request: &SubgraphReplanRequest,
    ) -> Result<SubgraphReplanResult, PlanRepairError>;
}
```

规则：

1. 从直接变更节点建立初始子图。
2. 验证所有 Input Boundary Contract。
3. 验证所有 Output Boundary Contract。
4. Boundary 不满足时按依赖方向扩张。
5. 扩张覆盖整图或 Story/Design Ref 改变时设置 `full_replan_required`。
6. 一拆多/多合一保存 Replacement Mapping。
7. Story/Design 升级创建对应 Child Session，不自动发布修改。

- [ ] **Step 4: 运行共享后端测试**

Run:

```bash
cargo fmt --check
cargo check --locked
cargo test --locked --lib plan_repair_subgraph_
cargo test --locked --lib workspace_session_link_
cargo test --locked --lib workspace_artifact_
```

Expected: Subgraph、Story、Design、Work Item 三类型共享协议测试通过。

- [ ] **Step 5: 提交**

```bash
git add src/product/plan_repair src/product/workspace_engine src/web/workspace_ws_types src/web/workspace_ws_handler
git commit -m "feat(plan-repair): replan affected subgraphs and escalate specs"
```

### Task 4: 端到端 Fixture、故障恢复与最终门禁

**Files:**
- Create: `tests/it_web/web_work_item_plan_repair.rs`
- Create: `tests/it_web/web_work_item_plan_repair/part_01.rs`
- Create: `tests/it_web/web_work_item_plan_repair/part_02.rs`
- Create: `tests/it_web/web_work_item_plan_repair/part_03.rs`
- Create: `tests/it_web/web_work_item_plan_repair/provider_matrix.rs`
- Modify: `tests/it_web.rs`
- Create: `web/e2e/work-item-plan-repair.spec.ts`
- Create: `web/e2e/helpers/plan-repair.ts`
- Modify: `src/web/test_controls/fixtures.rs`
- Modify: `web/src/components/lifecycle/IssueLifecycleWorkbench.test-data.ts`
- Modify: `tests/it_web/web_workspace_recovery_consistency.rs`

**Interfaces:**
- Validates: Design 全部验收标准
- Produces: 当前案例稳定回归和重启/并发证据

- [ ] **Step 1: 写端到端当前案例测试**

```rust
#[tokio::test]
async fn web_work_item_plan_repair_rewrites_only_upstream_and_resumes_consumer() {
    let app = WebRuntime::new_fake_with_plan_repair_fixture().await.unwrap();
    let attempt = app.start_group_coding("issue_plan_0001").await.unwrap();

    app.drive_until_review_finds_upstream_contract_invalid(&attempt)
        .await
        .unwrap();

    let waiting = app.get_coding_attempt(&attempt).await.unwrap();
    assert_eq!(waiting.status, "awaiting_plan_amendment");
    assert_eq!(waiting.active_unit.logical_work_item_id, "wi_registration");
    assert_eq!(waiting.active_unit.unit_rework_count, 0);

    let amendment = app.confirm_plan_repair(&attempt).await.unwrap();
    assert_eq!(amendment.revised_logical_work_items, vec!["wi_core"]);

    app.drive_until_consumer_resumes(&attempt).await.unwrap();

    let resumed = app.get_coding_attempt(&attempt).await.unwrap();
    assert_eq!(resumed.bound_plan_revision_id, "plan_revision_0002");
    assert_eq!(
        resumed.current_unit.work_item_revision_id,
        "work_item_revision_wi02_v1"
    );
    assert_eq!(
        resumed.current_unit.resolved_handoff_revision_ids,
        vec!["handoff_revision_0002"]
    );
    assert!(!resumed.rewritten_logical_work_items.contains(&"wi_unrelated".to_string()));
}
```

Playwright：

```ts
test("repairs an upstream work item without leaving coding workspace", async ({ page }) => {
  await openPlanRepairFixture(page);

  await expect(page.getByRole("heading", { name: "Plan Repair" })).toBeVisible();
  await expect(page).toHaveURL(/coding-attempts\/coding_attempt_0001/);
  await expect(page.getByText("WI-01：重新执行")).toBeVisible();
  await expect(page.getByText("WI-02：重新验证")).toBeVisible();

  await page.getByRole("button", {
    name: "确认修订并恢复执行",
  }).click();

  await expect(page.getByText("已使用 WI-01 Revision 2 恢复")).toBeVisible();
  await expect(page).toHaveURL(/coding-attempts\/coding_attempt_0001/);
});
```

- [ ] **Step 2: 运行定向 E2E 确认失败**

Run:

```bash
cargo test --locked --test it_web web_work_item_plan_repair
cd web
pnpm test:e2e -- work-item-plan-repair.spec.ts
```

Expected: FAIL，Fixture/端到端链路尚未全部接入。

- [ ] **Step 3: 完成 Fixture、故障注入和恢复矩阵**

在 `src/web/test_controls/fixtures.rs` 增加：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlanRepairFaultPoint {
    AfterDraftSaved,
    AfterProjectionGenerated,
    AfterPlanReview,
    AfterAmendmentPrepared,
    AfterPlanPublished,
    AfterPlanBindingWritten,
    AfterUnitRunsWritten,
    AfterResumeTargetWritten,
    AfterHandoffPublished,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlanRepairFixtureControl {
    pub fault_point: Option<PlanRepairFaultPoint>,
    pub duplicate_plan_defect_finding: bool,
    pub concurrent_amendment_request: bool,
    pub dirty_worktree_before_apply: bool,
}
```

在 `provider_matrix.rs` 增加四角色、三 Provider 的语义矩阵：

```rust
#[tokio::test]
async fn work_item_plan_repair_provider_matrix_preserves_contract_and_defect_semantics() {
    for provider in [
        ProviderName::Codex,
        ProviderName::ClaudeCode,
        ProviderName::Fake,
    ] {
        let fixture = provider_matrix_fixture(provider.clone());

        let authored = fixture.run_work_item_author().await.unwrap();
        assert!(authored.contract_ids().contains(
            &"repository_initialization_finalization".to_string()
        ));

        let plan_review = fixture.run_plan_reviewer(&authored).await.unwrap();
        assert!(plan_review.projection_validation.is_valid());

        let coder_context = fixture.render_coder_context().unwrap();
        assert!(coder_context.text.contains("workflow_explicit_completion"));

        let code_review = fixture
            .run_code_reviewer_with_missing_upstream_capability()
            .await
            .unwrap();
        assert_eq!(
            code_review.findings[0].defect_class,
            PlanDefectClass::UpstreamContractInvalid
        );
        assert_eq!(
            code_review.findings[0].recommended_route,
            PlanDefectRoute::PlanRepair
        );
    }
}
```

在 `tests/it_web/web_work_item_plan_repair/part_02.rs` 对每个 `PlanRepairFaultPoint` 启动 Runtime、触发故障、重启 Runtime、继续执行，并断言 Amendment/UnitRun/Handoff ID 唯一。`duplicate_plan_defect_finding`、`concurrent_amendment_request` 和 `dirty_worktree_before_apply` 分别断言 Request 去重、`amendment_conflict` 和 Worktree Gate。`provider_matrix.rs` 必须实际运行 Work Item Author、Plan Reviewer、Coder、Code Reviewer 四角色，而不是只检查 Renderer 字符串。旧数据 Fixture 全部改为 Schema v2，不保留兼容分支。

- [ ] **Step 4: 运行完整门禁**

Run:

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

Expected:

```text
所有命令 exit 0
Rust/Frontend/E2E 实际测试运行数均大于 0
无 ignored failure
无旧 Schema 兼容分支
```

- [ ] **Step 5: 提交最终验收**

```bash
git add tests/it_web.rs tests/it_web/web_work_item_plan_repair.rs tests/it_web/web_work_item_plan_repair web/e2e/work-item-plan-repair.spec.ts web/e2e/helpers/plan-repair.ts src/web/test_controls/fixtures.rs web/src/components/lifecycle/IssueLifecycleWorkbench.test-data.ts tests/it_web/web_workspace_recovery_consistency.rs
git commit -m "test(plan-repair): cover end-to-end repair and recovery"
```
