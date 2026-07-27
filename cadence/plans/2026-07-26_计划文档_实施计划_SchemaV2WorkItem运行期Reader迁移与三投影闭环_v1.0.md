# Schema v2 Work Item 运行期 Reader 迁移与三投影闭环实施计划

> **面向执行 Agent：** 必须逐任务执行本计划；实施前调用 `superpowers:executing-plans`。每个步骤使用复选框跟踪，严格遵循先失败测试、再最小实现、再验证、再提交的 TDD 顺序。

**目标：** 让新的 Schema v2 Work Item Group 在旧 `work-items/` 目录为空时，从 Final Compile 到子 Workspace、Group Coding、Tester、Gate、Handoff 和生命周期视图全程只通过 Revision Store 运行，并维持 Canonical Contract 与三投影边界。

**架构：** 新增的 `WorkItemRuntimeBinding` 只锚定已经发布的 PlanRevision、WorkItemRevision、ProjectionBundle 和 VerificationPlanRevision，并作为 Work Item 子 Workspace Session 的不可变元数据保存。`WorkItemRuntimeReader` 是 Schema v2 运行期唯一解析入口：它先做双向 ID/Hash/版本完整性校验，再向 Human、Coder、Reviewer 与规范性执行消费者提供各自允许的数据；既有 CodingAttemptPlanBinding、CodingExecutionUnit、CodingUnitRun 已保存的版本凭据用于派生并校验 Coding 侧 Binding，而不是复制一份业务事实。

**技术栈：** Rust 2024、Serde、现有 JSON Product Store、Axum、Tokio、Cargo、Vitest（仅在 API/前端读模型实际变更时）。

## 全局约束

- Canonical Contract 是唯一业务权威；三种 Projection **仅**为 Human、Coder、Reviewer，均为只读不可变派生快照。
- `WorkItemRuntimeBinding` 不是第四投影、缓存或业务状态；不得存放目标、范围、依赖、验证项、Projection 内容或可编辑执行状态。
- Schema v2 严禁旧 `LifecycleWorkItemRecord`、旧 VerificationPlan、旧执行状态的迁移、回填、双写、双读、fallback 和兼容 DTO；缺 Binding 或完整性不符必须失败关闭。
- 不迁移、不删除、不修改历史 `.aria` 业务数据；本计划只修改新 Schema v2 数据的读写代码路径。
- Work Item RuntimeBinding 仅适用于 `WorkspaceType::WorkItem`。Story 与 Design 必须继续走自身既有实体路径，且不得要求或创建该 Binding。
- Coder 只读取绑定的 Coder Projection 与其 Renderer Hash；Reviewer 只读取绑定的 Reviewer Projection 与其 Renderer Hash；Tester/Evaluation/Gate/Handoff 读取 Canonical Contract、VerificationPlanRevision、DependencyGraphRevision、HandoffRevision。
- Plan Repair 只能经既有 Amendment 事务生成后续 Binding/UnitRun；既有 Workspace、completed UnitRun、HandoffRevision 不得因 active PlanRevision 改变而漂移。
- 本变更不修改 Work Item Draft Provider 输入、Canonical Contract 字段语义、投影编译算法或 Provider 输出协议，因此不触发真实 Provider 的 Case A/Case B 验证；若实施中触及这些边界，必须先按 `work-item-draft-prompt-validation.md` 向操作者请求授权。
- Rust 测试使用宿主机工具链与 `cargo test --locked`；定向单测必须为 `cargo test --locked --lib <过滤名>`，禁止 `-j 1`。
- 共享 Workspace 修改必须为 Story、Design、Work Item 三类补齐回归；若某一类不适用，测试名称或断言须明确其原因。

---

## 运行时数据流与文件边界

```text
Final Compile
  → WorkItemPlanRevision / WorkItemRevision / VerificationPlanRevision / ProjectionBundle
  → WorkItemRuntimeBinding（仅引用和 Hash 凭据）
  → WorkItem 子 Workspace Session
  → WorkItemRuntimeReader
      ├─ Human：Human Projection + Presentation / History
      ├─ Coder：Coder Projection + Renderer Hash
      ├─ Reviewer：Reviewer Projection + Renderer Hash
      └─ Tester / Evaluation / Gate / Handoff：Canonical + Verification + Graph + Handoff
```

| 文件 | 责任 |
|---|---|
| `src/product/models/work_item_revision.rs` | 定义不可变 `WorkItemRuntimeBinding` 的引用与完整性凭据。 |
| `src/product/models/workspace.rs` | 将可选 Binding 持久化到 Work Item 子 Session；Story/Design 仍为 `None`。 |
| `src/product/work_item_runtime_reader.rs`（新增） | 统一解析、双向校验、角色化返回 Schema v2 运行期数据。 |
| `src/product/lifecycle_store/workspace.rs` | 以文件锁幂等写入 Session Binding，不覆写不同 Binding。 |
| `src/product/workspace_engine/compile/finalizer.rs` | 在 Final Compile 成功对外可见前确保 Binding 与子 Workspace 启动上下文。 |
| `src/web/workspace_context/{builder,entity}.rs`、`src/product/workspace_repository.rs` | Work Item 子 Workspace 的 Human 上下文、标题、关联信息与仓库解析改为 Reader。 |
| `src/web/handlers/coding/{group.rs,rs}`、`src/product/coding_attempt_store/group_validation.rs` | 从 PlanRevision/PlanProjection/DependencyGraph/Issue 元数据物化 Group Coding，不再以旧列表预检。 |
| `src/product/coding_work_item_context.rs`、`src/web/coding_ws_handler/context.rs` | Coder 运行上下文、执行确认和仓库解析只通过绑定 Revision。 |
| `src/product/coding_evaluation_context/{builder.rs,tester_execution.rs}`、`src/product/tester_agent_loop/context_loader.rs` | 评估与测试上下文读取 Canonical/Verification 及真实 Story/Design 引用。 |
| `src/product/coding_workspace_engine/{gates.rs,handoffs.rs}` | Gate、完成状态和 Handoff 使用 UnitRun/Revision/HandoffRevision；不得更新旧 Work Item 执行状态。 |
| `src/web/handlers/lifecycle.rs` | 生命周期 Work Item 列表改为 Revision Store 派生读模型；删除/终止 v2 路径不触碰旧记录。 |

### Task 1：定义 RuntimeBinding 与唯一 Revision Store Reader

**文件：**

- 修改：`src/product/models/work_item_revision.rs`
- 修改：`src/product/models/workspace.rs`
- 修改：`src/product/mod.rs`
- 新建：`src/product/work_item_runtime_reader.rs`
- 新建：`src/product/work_item_runtime_reader/tests.rs`
- 修改：`src/product/models/tests.rs`（或当前 `WorkspaceSessionRecord` Serde 测试所在文件）

**接口：**

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemRuntimeBinding {
    pub plan_id: String,
    pub plan_revision_id: String,
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub projection_bundle_id: String,
    pub verification_plan_revision_id: String,
    pub canonical_contract_hash: String,
    pub projection_compiler_version: String,
    pub human_projection_hash: String,
    pub coder_projection_hash: String,
    pub reviewer_projection_hash: String,
}

pub struct ResolvedWorkItemRuntime {
    pub binding: WorkItemRuntimeBinding,
    pub lineage: WorkItemPlanLineage,
    pub plan_revision: WorkItemPlanRevision,
    pub dependency_graph: DependencyGraphRevision,
    pub work_item_revision: WorkItemRevision,
    pub verification_plan_revision: VerificationPlanRevision,
    pub projection_bundle: WorkItemProjectionBundle,
    pub plan_projection_bundle: PlanProjectionBundle,
    pub human_presentation: Option<HumanPresentationRevision>,
}

pub struct WorkItemRuntimeReader {
    paths: ProductAppPaths,
}

impl WorkItemRuntimeReader {
    pub fn resolve_binding(
        &self,
        project_id: &str,
        issue_id: &str,
        binding: &WorkItemRuntimeBinding,
    ) -> Result<ResolvedWorkItemRuntime, ProductStoreError>;

    pub fn resolve_workspace(
        &self,
        session: &WorkspaceSessionRecord,
    ) -> Result<ResolvedWorkItemRuntime, ProductStoreError>;

    pub fn resolve_coding_unit(
        &self,
        attempt: &CodingExecutionAttempt,
        unit: &CodingExecutionUnit,
        run: Option<&CodingUnitRun>,
    ) -> Result<ResolvedWorkItemRuntime, ProductStoreError>;
}
```

- [x] **步骤 1：为 Session Binding、Coding Unit 派生 Binding 和完整性失败关闭写失败测试。**

  在 `work_item_runtime_reader/tests.rs` 建立只包含 Revision Store Fixture 的测试，不调用 `LifecycleStore::create_work_item`。至少覆盖：合法 Binding 能取得同一逻辑 ID 的 Revision/Bundle/Verification；缺少 Session Binding 返回 `runtime_binding_missing`；PlanRevision 的 `work_item_bindings`、`WorkItemRevision.work_item_projection_bundle_id`、`WorkItemRevision.verification_plan_revision_id`、Bundle `canonical_contract_hash` 任一不符时返回稳定的 `runtime_binding_integrity_mismatch`；带不同 UnitRun Hash 的 Coding 调用失败关闭。

  ```rust
  #[test]
  fn workspace_binding_rejects_projection_bundle_from_another_revision() {
      let fixture = published_runtime_fixture();
      let mut binding = fixture.binding.clone();
      binding.projection_bundle_id = "projection_other".to_string();

      let error = fixture
          .reader()
          .resolve_binding(PROJECT_ID, ISSUE_ID, &binding)
          .unwrap_err();

      assert!(error.to_string().contains("runtime_binding_integrity_mismatch"));
  }
  ```

- [ ] **步骤 2：运行 Reader 红灯测试。**

  运行：`cargo test --locked --lib work_item_runtime_reader`

  预期：失败，原因是 `WorkItemRuntimeBinding`、`WorkItemRuntimeReader` 或目标方法尚不存在；不得通过创建 Legacy Work Item 夹具使测试转绿。

- [x] **步骤 3：实现最小、纯引用的 Binding 与解析器。**

  在 `WorkItemRuntimeBinding` 上实现从已发布对象构造的内部构造器与 ID 非空校验；在 `WorkspaceSessionRecord` 增加 `#[serde(default)] pub work_item_runtime_binding: Option<WorkItemRuntimeBinding>`。Reader 依次读取 lineage → binding 指定的 PlanRevision → DependencyGraph/PlanProjectionBundle → logical work item → WorkItemRevision → VerificationPlanRevision → ProjectionBundle，并校验：

  ```rust
  plan_revision.work_item_bindings[&binding.logical_work_item_id]
      == binding.work_item_revision_id
  && work_item_revision.logical_work_item_id == binding.logical_work_item_id
  && work_item_revision.work_item_projection_bundle_id == binding.projection_bundle_id
  && work_item_revision.verification_plan_revision_id == binding.verification_plan_revision_id
  && projection_bundle.work_item_revision_id == binding.work_item_revision_id
  && projection_bundle.canonical_contract_hash == binding.canonical_contract_hash
  && projection_bundle.compiler_version == binding.projection_compiler_version
  && projection_bundle.human_projection_hash == binding.human_projection_hash
  && projection_bundle.coder_projection_hash == binding.coder_projection_hash
  && projection_bundle.reviewer_projection_hash == binding.reviewer_projection_hash
  ```

  `resolve_workspace` 必须拒绝非 `WorkspaceType::WorkItem` 或缺 Binding；`resolve_coding_unit` 必须先验证 Attempt 的 `CodingAttemptPlanBinding.bound_plan_revision_id`、Unit 的逻辑 ID/Revision ID，再在有 UnitRun 时验证 Contract/Bundle/Compiler/Projection Hash。不得查询 `LifecycleStore::list_work_items`。

- [x] **步骤 4：完成 Serde 与边界测试。**

  补充 Session JSON 缺少可选字段可解析、Work Item Session 有 Binding 可 roundtrip、Story/Design Session 保持 `None` 的测试；同时断言 Binding JSON 不含 `canonical_contract`、`human_projection`、`coder_projection`、`reviewer_projection`、`verification_checks` 或执行状态字段。

- [ ] **步骤 5：运行 Task 1 验证并提交。**

  运行：`cargo test --locked --lib work_item_runtime_reader`

  预期：全部通过，且测试 Fixture 的旧 Work Item 目录为空。

  ```bash
  git add src/product/models/work_item_revision.rs src/product/models/workspace.rs src/product/mod.rs src/product/work_item_runtime_reader.rs src/product/work_item_runtime_reader/tests.rs src/product/models/tests.rs
  git commit -m "feat: add schema v2 work item runtime reader"
  ```

### Task 2：使 Final Compile 的成功边界包含子 Session Binding 与启动上下文

**文件：**

- 修改：`src/product/lifecycle_store/workspace.rs`
- 修改：`src/product/lifecycle_store/tests.rs`
- 修改：`src/product/workspace_engine/compile.rs`
- 修改：`src/product/workspace_engine/compile/finalizer.rs`
- 修改：`src/web/workspace_context/builder.rs`
- 修改：`src/web/workspace_ws_handler/decisions.rs`
- 修改：`tests/it_web/web_work_item_plan_compile/part_01.rs`
- 修改：`tests/it_web/web_work_item_plan_compile/part_02.rs`

**接口：**

```rust
impl LifecycleStore {
    pub fn ensure_work_item_runtime_binding(
        &self,
        session_id: &str,
        binding: &WorkItemRuntimeBinding,
    ) -> Result<WorkspaceSessionRecord, ProductStoreError>;
}
```

- [x] **步骤 1：先写 Binding 幂等性与 Compile 成功边界的失败测试。**

  为 `ensure_work_item_runtime_binding` 写三个测试：首次写入成功；重放同值不改变 Binding；同一 Session 重放不同 Binding 返回 `IdentityMismatch`。在 `web_work_item_plan_compile/part_01.rs` 扩展 `batch_accept_all_runs_final_compile_and_publishes_revision_entities`：断言三份 Legacy 列表仍为空、每个 Work Item 子 Session 含 Binding、首条 system context 已持久化且包含 Human Projection 标题/摘要。添加 failpoint 用例，在第一个子 Session Binding 或 Context 准备后中断并恢复，断言不重复 Session、不更换 Revision/Binding、成功前不产生 committed compile report。

  ```rust
  assert!(legacy_work_items.is_empty());
  assert!(legacy_verification_plans.is_empty());
  assert!(work_item_sessions.iter().all(|s| s.work_item_runtime_binding.is_some()));
  assert!(work_item_sessions.iter().all(|s| {
      s.messages.first().is_some_and(|m| m.content.contains("[work_item_context]"))
  }));
  ```

- [ ] **步骤 2：运行 Final Compile 红灯测试。**

  运行：`cargo test --locked --test it_web batch_accept_all_runs_final_compile_and_publishes_revision_entities`

  预期：失败，因为 Final Compile 尚未保存 Binding/上下文，或恢复 checkpoint 尚不存在。

- [x] **步骤 3：实现 Session Binding 的锁内幂等写入。**

  `LifecycleStore::ensure_work_item_runtime_binding` 必须在 `find_workspace_session_path` 的独占锁内读取 Session，先验证该 Session 为 WorkItem，再按以下规则处理：`None` 写入 Binding；`Some(existing) == binding` 原样返回；不同则返回 `IdentityMismatch { kind: "work_item_runtime_binding", ... }`。写入时更新 `updated_at`，不得改写 messages、状态、Provider 配置，也不得创建任何 Legacy Record。

- [x] **步骤 4：在 Finalizer 中把“子 Workspace 可运行”放到 committed 前。**

  在 `WorkspaceEngine::finalize_initial_plan_compile` 的每个 logical Work Item 循环内，从 `InitialPlanCompileOutcome` 的已发布 Revision/Bundle/Verification 对象构建 Binding，随后顺序执行：确保或复用子 Session → `ensure_work_item_runtime_binding` → 调用既有 `ensure_workspace_context_message` 生成 Revision Store 驱动的 system context → 更新 Compile Transaction cursor。新增 `WorkItemPlanCompileFinalizerCheckpoint::FirstChildBindingEnsured` 与 `FirstChildContextPrepared`，恢复时必须复用同一 `compile_id`、Session ID 和 Binding。

  `WorkItemPlanCompileReportPayload { status: Committed }` 及 `persist_compile_report` 必须位于所有子 Session Context 成功之后；失败仅保留可重放 Journal/cursor，不发送 human-confirm 成功。

- [x] **步骤 5：移除确认成功后的补救式 Context 初始化。**

  `handle_human_confirm_from_handler` 的 `ConfirmedWithChildSessions` 分支不得再在成功响应之后执行可能失败的 Context 构建。保留读取并发送已经就绪的 child session 信息即可；若存储中缺少 Binding/Context，返回明确 `runtime_binding_missing` 错误，而不是访问旧 Work Item 或发送先成功后失败的 WebSocket 序列。

- [ ] **步骤 6：运行恢复与 Compile 测试并提交。**

  运行：

  ```bash
  cargo test --locked --test it_web batch_accept_all_runs_final_compile_and_publishes_revision_entities
  cargo test --locked --test it_web work_item_plan_compile
  ```

  预期：新 Group 的旧目录为空仍能完成 Compile；任一子 Context 准备失败时没有 `Committed` 成功事件；恢复重放不产生重复 Session。

  ```bash
  git add src/product/lifecycle_store/workspace.rs src/product/lifecycle_store/tests.rs src/product/workspace_engine/compile.rs src/product/workspace_engine/compile/finalizer.rs src/web/workspace_context/builder.rs src/web/workspace_ws_handler/decisions.rs tests/it_web/web_work_item_plan_compile
  git commit -m "fix: bind work item runtime before compile confirmation"
  ```

### Task 3：迁移 Work Item 子 Workspace 的 Human 读取、仓库解析与三类型共享协议

**文件：**

- 修改：`src/web/workspace_context/entity.rs`
- 修改：`src/web/workspace_context/tests.rs`
- 修改：`src/web/workspace_context/tests/linked_context.rs`
- 修改：`src/product/workspace_repository.rs`
- 修改：`src/product/workspace_engine/tests/part_*.rs` 中现有 Workspace Session 恢复表驱动测试

**接口：**

```rust
pub(super) fn work_item_runtime_context_summary(
    runtime: &ResolvedWorkItemRuntime,
) -> String;

pub fn workspace_repository_for_session(
    app_paths: &ProductAppPaths,
    lifecycle: &LifecycleStore,
    session: &WorkspaceSessionRecord,
) -> Result<RepositoryRecord, ProductStoreError>;
```

- [x] **步骤 1：为 Work Item Human Context 写无旧目录失败测试，并保留 Story/Design 对照。**

  在 `workspace_context` 测试中创建只含 Revision Store、Issue、Story/Design Spec 与带 Binding Session 的 Fixture；不创建 `LifecycleWorkItemRecord`。断言生成 context 成功并包含 Human Projection 的标题、目标、依赖与范围摘要，以及存在时的 HumanPresentationRevision 风险/依赖解释和 Revision 标识，但不包含 Coder/Reviewer Projection、Canonical Contract JSON 或旧 `[work_item_plan_source]` 段。再以表驱动方式断言 Story、Design、WorkItem 三种 Session 都可恢复，只有 WorkItem 需要 Binding。

  ```rust
  for case in [WorkspaceType::Story, WorkspaceType::Design, WorkspaceType::WorkItem] {
      let session = fixture.session_for(case);
      let result = ensure_workspace_context_message(&fixture.paths, &fixture.lifecycle, session);
      assert!(result.is_ok(), "{case:?} must keep its own context path");
  }
  ```

- [ ] **步骤 2：运行 Workspace Context 红灯测试。**

  运行：`cargo test --locked --lib workspace_context`

  预期：Work Item 仍因 `find_work_item` 读取旧目录失败；Story/Design 现有断言保持通过。

- [x] **步骤 3：以 Reader 替换 Work Item 分支。**

  `workspace_entity_context` 与 `work_item_context_summary` 的 `WorkspaceType::WorkItem` 分支改为 `WorkItemRuntimeReader::resolve_workspace`：标题取 `projection_bundle.human_projection.title`，人类摘要取该 Projection 的 goal/dependencies/scope 与可选 `human_presentation` 的风险说明，关联 Story/Design 取 `runtime.lineage.story_spec_refs` / `design_spec_refs`，验证摘要取 `verification_plan_revision.verification_checks`。`workspace_repository_id` 的 WorkItem 分支改为 Issue 的 `repo_id`，并在缺失时失败关闭；不得从旧 Work Item 取 `repository_id`。

  保留 Story、Design、WorkItemPlan 的原有分支与传入的 LifecycleStore；不得给 Story/Design 创建 Binding，也不得在 Reader 失败后回退 `find_work_item`。

- [x] **步骤 4：删除旧 Work Item Context 构造与验证旧字段依赖。**

  删除或收窄 `find_work_item`、`needs_source_draft_supplement` 等只为 WorkItem 子 Workspace Context 服务的调用路径；`[work_item_context]` 必须明确列出 `plan_revision_id`、`work_item_revision_id`、`verification_plan_revision_id` 与 Human Projection Hash，供人工审计而不让其成为 Provider 执行输入。

- [ ] **步骤 5：运行三类型回归并提交。**

  运行：

  ```bash
  cargo test --locked --lib workspace_context
  cargo test --locked --lib workspace_engine
  ```

  预期：Story/Design 不依赖 Binding；WorkItem 在旧目录不存在时仅通过 Human Projection 成功初始化和恢复。

  ```bash
  git add src/web/workspace_context src/product/workspace_repository.rs src/product/workspace_engine/tests
  git commit -m "fix: read work item workspace context from revisions"
  ```

### Task 4：从 PlanRevision 物化 Group Coding，消除创建前的 Legacy 预检

**文件：**

- 修改：`src/product/coding_attempt_store/group_validation.rs`
- 修改：`src/product/coding_attempt_store/group_initialization.rs`
- 修改：`src/product/coding_attempt_store/tests.rs` 及其 group 初始化测试模块
- 修改：`src/web/handlers/coding/group.rs`
- 修改：`src/web/handlers/coding.rs`
- 修改：`src/web/handlers/coding/tests/*group*`（按现有测试目录）

**接口：**

```rust
pub struct AuthoritativeCodingUnitBinding {
    pub logical_work_item_id: String,
    pub work_item_revision_id: String,
    pub verification_plan_revision_id: String,
    pub projection_bundle_id: String,
    pub dependency_logical_work_item_ids: Vec<String>,
}

pub struct AuthoritativeGroupPlanBinding {
    pub plan_revision_id: String,
    pub dependency_graph_revision_id: String,
    pub plan_projection_bundle_id: String,
    pub units: Vec<AuthoritativeCodingUnitBinding>,
}
```

- [x] **步骤 1：为 Group 创建写无 Legacy 记录的失败测试。**

  复用 Final Compile Fixture，明确断言 `LifecycleStore::list_work_items(...).is_empty()` 后调用 `POST /projects/{project}/issues/{issue}/work-item-plans/{plan}/coding-attempts`。断言 Attempt 的 `bound_plan_revision_id` 为 active Revision、Units 顺序与 `PlanProjectionBundle.coder_group_context.ordered_logical_work_item_ids` 相同、每个 Unit 指向 PlanRevision 指定 Revision，且 Group 初始化 Journal 可重放。

- [ ] **步骤 2：运行 Group 创建红灯测试。**

  运行：`cargo test --locked --test it_web create_group_coding_attempt`

  预期：当前实现因 `list_work_items` 或 `group_work_item_execution_order` 报 `work_item_not_found`/空组失败。

- [x] **步骤 3：重写权威 Group Binding 的解析顺序。**

  `CodingAttemptStore::resolve_authoritative_group_plan_binding` 必须删除第一个 `LifecycleStore::list_work_items` 分支，改为：读取 lineage 的 active PlanRevision → 读取该 PlanRevision 的 PlanProjectionBundle → 使用 `ordered_logical_work_item_ids` 固定顺序 → 验证无重复且和 `work_item_bindings` 集合精确一致 → 读取 DependencyGraphRevision → 对每个逻辑 ID 读取 Revision、Verification、ProjectionBundle 并构造 Unit Binding。若 logical ID 的 active Revision 与 **该 PlanRevision Binding** 不同，不得拒绝历史 PlanBinding；只校验被读取 Revision 属于正确 logical item，避免 Plan Repair 后错误漂移。

- [x] **步骤 4：迁移 Handler 的仓库、Provider 与入口策略。**

  `create_group_coding_attempt` 先取 confirmed plan（仅确认状态）与 `AuthoritativeGroupPlanBinding`，再从 `IssueStore::get(...).repo_id` 解析仓库。Provider 配置从具有相同 logical ID 和相同 `work_item_runtime_binding` 的已绑定 WorkItem Session 获取；无 Session 时使用现有 repository 默认配置。不得把 `LifecycleWorkItemRecord` 重新引回 `coding_provider_config_snapshot`。

  `create_coding_attempt`、`save_work_item_execution_plan_for_attempt`、`group_work_item_execution_order`、`work_item_by_id` 的 v2 可达调用也必须改为 Reader/AuthoritativeBinding；若单 Work Item API 不属于 Schema v2 支持表面，则对具有有效 v2 PlanRevision 的请求返回明确 `schema_v2_group_coding_required`，不可悄悄使用旧路径。

- [x] **步骤 5：验证 Group Journal 重放与 Plan Repair 稳定性。**

  为同一 Group 重试、`AttemptPersisted`/`PlanBindingSaved`/首 Unit 中断恢复、以及 Amendment 发布后已完成 UnitRun 的测试添加断言：当前创建只绑定其 `bound_plan_revision_id`，旧 UnitRun 与 Handoff 不变，新 Revision 只能经已有 Amendment Journal 生效。

- [ ] **步骤 6：运行 Group 测试并提交。**

  运行：

  ```bash
  cargo test --locked --lib coding_attempt_store
  cargo test --locked --test it_web create_group_coding_attempt
  cargo test --locked --lib coding_plan_repair_
  ```

  预期：新 v2 Group 不创建/读取 Legacy Work Item；Plan Repair 的既有 Binding 不漂移。

  ```bash
  git add src/product/coding_attempt_store/group_validation.rs src/product/coding_attempt_store/group_initialization.rs src/product/coding_attempt_store/tests.rs src/web/handlers/coding/group.rs src/web/handlers/coding.rs src/web/handlers/coding/tests
  git commit -m "fix: materialize coding groups from plan revisions"
  ```

### Task 5：迁移 Coder、Reviewer、Tester、Gate、Handoff 与生命周期运行期消费者

**文件：**

- 修改：`src/product/coding_work_item_context.rs`
- 修改：`src/web/coding_ws_handler/context.rs`
- 修改：`src/product/coding_evaluation_context/builder.rs`
- 修改：`src/product/coding_evaluation_context/tester_execution.rs`
- 修改：`src/product/tester_agent_loop/context_loader.rs`
- 修改：`src/product/coding_workspace_engine/gates.rs`
- 修改：`src/product/coding_workspace_engine/handoffs.rs`
- 修改：`src/web/handlers/lifecycle.rs`
- 修改：对应 `src/product/*/tests.rs`、`src/web/coding_ws_handler/tests/*` 与 `tests/it_web/*` 回归文件

**接口：**

```rust
impl WorkItemRuntimeReader {
    pub fn coder_projection_for_unit(/* attempt, unit, run */)
        -> Result<(CoderWorkItemProjection, String), ProductStoreError>;
    pub fn reviewer_projection_for_unit(/* attempt, unit, run */)
        -> Result<(ReviewerWorkItemProjection, String), ProductStoreError>;
    pub fn normative_context_for_unit(/* attempt, unit, run */)
        -> Result<ResolvedWorkItemRuntime, ProductStoreError>;
}
```

- [x] **步骤 1：为每种角色的正确读取与错误 Hash 写失败测试。**

  新增/修改测试，使用只含 Revision Store 和 Group Attempt/Unit/Run 的 Fixture：

  - Coder 只取得 `coder_projection`，其 Renderer 输出 hash 必须与 UnitRun 的 `coder_projection_hash` 和 renderer version 一致；
  - Reviewer 只取得 `reviewer_projection`，不能使用 Human Projection；
  - Tester/Evaluation 的 required checks 来自 `VerificationPlanRevision.verification_checks`，范围/依赖来自 Canonical Contract/DependencyGraph；
  - Gate/Handoff 的 HandoffRevision 必须匹配 UnitRun 的 `work_item_revision_id`、`canonical_contract_hash`、`projection_bundle_id`；
  - 任一错配返回 binding 错误，测试中断言没有调用或写入 `LifecycleStore::list_work_items` / `update_work_item_execution_status`。

- [ ] **步骤 2：运行角色与 Gate 红灯测试。**

  运行：

  ```bash
  cargo test --locked --lib coding_workspace_engine
  cargo test --locked --lib coding_evaluation_context
  cargo test --locked --lib tester_agent_loop
  ```

  预期：至少一个测试因旧 Work Item/VerificationPlan 读取或 Human Projection 被当作执行数据而失败。

- [x] **步骤 3：替换 Coder/Reviewer 运行上下文。**

  `load_coding_work_item_context` 不再构造 `LifecycleWorkItemRecord` Markdown、读取旧 VerificationPlan、查 Draft 或回退 Workspace Artifact。它通过当前 Unit/Run 的 Reader 得到 Coder Projection 与 `VerificationPlanRevision`，仅生成传给现有 renderer 的最小结构化输入。`coding_execution_context` 使用该结果；现有 `CodingWorkspaceEngine::render_coder_unit_run_context` 与 `render_reviewer_unit_run_context` 保持 Renderer/Hash 校验逻辑，并改为共同使用 Reader 的解析结果，禁止重复按 active revision 查询。

  `ensure_work_item_execution_plan_confirmed` 仅对仍有 Canonical Contract 明确等价语义的 v2 Unit 执行确认；当前 Contract 没有该语义时，不得从旧字段猜测，直接取消该 Legacy 前置检查。`repository_path_for_attempt` 从 Issue `repo_id` 解析仓库。

- [x] **步骤 4：替换 Tester/Evaluation 的规范性输入。**

  `build_evaluation_context_pack`、`tester_execution` 与 `TestContextLoader` 通过 `normative_context_for_unit` 取绑定 Revision：Story/Design 引用来自 PlanLineage，Work Item Markdown 由 Canonical Contract 的稳定 formatter 生成，验证命令由 `VerificationPlanRevision` 生成。缺失 artifact ref 只能产生针对用户显式可选展示的 warning；缺 Binding、Plan/Unit/Hash 不一致必须返回错误，不能为空上下文继续执行。

- [x] **步骤 5：替换 Gate、Handoff 与生命周期派生读模型。**

  `gates.rs` 以当前 Unit/Run、VerificationPlanRevision、DependencyGraphRevision、已完成 HandoffRevision 完成 required checks 与依赖门禁；`handoffs.rs` 只更新 Coding Unit、UnitRun、HandoffRevision、共享 worktree lock，不再探测/写入旧 Work Item completion。`issue_lifecycle` 从 active PlanRevision 的 Human Group Projection + CodingAttempt/Unit 状态生成 Work Item DTO；删除/终止针对 v2 仅删除允许删除的 Session/Attempt 元数据，缺 Binding 失败关闭，绝不删除或回填历史 Legacy 文件。

- [x] **步骤 6：逐项清点并关闭 v2 可达旧 Reader。**

  运行 `rg -n 'list_work_items\\(' src`，为每个命中写分类：仅 Legacy 历史入口/测试可保留但不得从 v2 路由进入；v2 可达的 `workspace_context/entity.rs`、`workspace_repository.rs`、`handlers/coding/{group.rs,rs}`、`coding_ws_handler/context.rs`、`coding_work_item_context.rs`、`coding_evaluation_context/{builder.rs,tester_execution.rs}`、`tester_agent_loop/context_loader.rs`、`coding_workspace_engine/{gates.rs,handoffs.rs}`、`handlers/lifecycle.rs` 必须迁移或明确下线。将该清单作为 PR 描述的一部分，并以测试证明没有 fallback。

- [ ] **步骤 7：运行角色、生命周期与 Handoff 测试并提交。**

  运行：

  ```bash
  cargo test --locked --lib coding_workspace_engine
  cargo test --locked --lib coding_evaluation_context
  cargo test --locked --lib tester_agent_loop
  cargo test --locked --test it_web web_coding_ws_handler
  cargo test --locked --test it_web lifecycle
  ```

  预期：Coder/Reviewer 使用不同绑定 Projection；Tester/Gate/Handoff 仅使用规范性 Revision；生命周期在旧 Work Item 目录为空时仍能展示 Schema v2 Group。

  ```bash
  git add src/product/coding_work_item_context.rs src/web/coding_ws_handler/context.rs src/product/coding_evaluation_context src/product/tester_agent_loop/context_loader.rs src/product/coding_workspace_engine/gates.rs src/product/coding_workspace_engine/handoffs.rs src/web/handlers/lifecycle.rs
  git commit -m "fix: route schema v2 runtime consumers through revisions"
  ```

### Task 6：端到端 Cutover 回归、三 Workspace 防回归与交付验证

**文件：**

- 修改：`tests/it_web/web_work_item_plan_compile/part_01.rs`
- 修改：`tests/it_web/web_work_item_plan_compile/part_02.rs`
- 新建或修改：`tests/it_web/web_work_item_runtime_projection.rs`
- 修改：`src/web/workspace_context/tests/linked_context.rs`
- 修改：`src/product/work_item_revision_store/tests/initial_publication.rs`
- 修改：`openspec/changes/fix-work-item-runtime-projection/tasks.md`（实施完成后逐项勾选；本计划编写阶段不勾选）

**验收场景：**

```text
Story + Design 已确认
  → Work Item Plan Outline / Draft / Final Compile
  → Legacy work-items 与 verification-plans 均为空
  → 每个 Work Item 子 Session 有 Binding 和 Human Context
  → 创建 Group Coding Attempt
  → Coder / Tester / Reviewer / Gate / Handoff 使用绑定 Revision
  → Plan Repair 只作用于后续 Binding，历史 UnitRun/Handoff 不漂移
```

- [x] **步骤 1：写全链路红灯集成测试。**

  用同一测试覆盖三种真实逻辑 Work Item（例如 library export、service、UI）：Final Compile 后断言旧目录为空；连接子 Workspace 不报 `product_store_not_found: work_item ...`；创建 Group Coding 成功；将一个 UnitRun 的 Bundle/Hash 篡改后 Coder/Reviewer/Gate 明确失败关闭；恢复未篡改 Fixture 后继续到 Handoff。测试不得手工 `create_work_item`。

  ```rust
  assert!(LifecycleStore::new(paths.clone())
      .list_work_items(PROJECT_ID, ISSUE_ID)
      .unwrap()
      .is_empty());
  assert!(create_group_attempt(&app, PLAN_ID).await.is_ok());
  assert_runtime_binding_error(tampered_unit_run_result.unwrap_err());
  ```

- [ ] **步骤 2：运行端到端红灯测试。**

  运行：`cargo test --locked --test it_web web_work_item_runtime_projection`

  预期：在实现所有 Reader 替换前失败于旧读取点；不得为让测试通过写入 Legacy 记录。

- [x] **步骤 3：完成 Compile 恢复、Binding 不变性与三类型矩阵。**

  增加 table-driven Workspace 测试：Story/Design 无 Binding 正常恢复；WorkItem 有正确 Binding 正常恢复；WorkItem 无 Binding、错 PlanRevision、错 Bundle、错 Hash 均失败关闭。添加 Final Compile Journal 在发布后、首 Binding 后、首 Context 后的三种恢复位置；每次重放均断言 Revision IDs、Binding、Session IDs、Artifact Version IDs 不变。

- [ ] **步骤 4：执行定向与全量验证。**

  运行：

  ```bash
  cargo fmt --check
  cargo check --locked
  cargo test --locked --lib work_item_runtime_reader
  cargo test --locked --lib workspace_context
  cargo test --locked --lib coding_attempt_store
  cargo test --locked --lib coding_workspace_engine
  cargo test --locked --lib coding_evaluation_context
  cargo test --locked --lib tester_agent_loop
  cargo test --locked --test it_web web_work_item_runtime_projection
  cargo test --locked
  cargo clippy --all-targets --all-features --locked -- -D warnings
  ```

  预期：全部通过；任何 `list_work_items` 剩余命中均由代码评审标记为 v2 不可达的 Legacy-only 路径，且没有 Schema v2 fallback。

- [ ] **步骤 5：完成 OpenSpec 和提交。**

  在所有验收命令有新鲜通过证据后勾选 Change tasks、运行严格校验并提交：

  ```bash
  openspec validate fix-work-item-runtime-projection --type change --strict --no-interactive
  git add tests/it_web/web_work_item_plan_compile tests/it_web/web_work_item_runtime_projection.rs src/web/workspace_context/tests src/product/work_item_revision_store/tests openspec/changes/fix-work-item-runtime-projection/tasks.md
  git commit -m "test: cover schema v2 work item runtime cutover"
  ```

---

## 实施完成前的审查清单

- [ ] `WorkItemRuntimeBinding` 只有 ID/Hash/version 凭据，没有 Contract、Projection 或执行业务字段。
- [ ] 新 Group 的 `.aria/.../work-items/` 和旧 verification plans 保持为空；测试没有调用 `create_work_item` 作为补偿。
- [ ] Final Compile 在任一 Binding/Context 失败时没有给客户端发送成功确认，且恢复幂等。
- [ ] Human、Coder、Reviewer 分别取得三个不同且被 Hash 校验的 Projection；Tester/Evaluation/Gate/Handoff 没有把 Human Projection 用作规范。
- [ ] Plan Repair 后历史 Session、completed UnitRun、HandoffRevision 不漂移；只有 Amendment 事务创建的后续 Unit 使用新 Binding。
- [ ] Story、Design、WorkItem 三类 Workspace 的创建和恢复均覆盖，且仅 WorkItem 要求 Binding。
- [ ] `rg -n 'list_work_items\\(' src` 的每个 v2 可达调用已消除或下线，无任何 fallback。

## 执行复核记录（2026-07-27）

- 已按提交与当前源码逐项复核并勾选已落地的测试编写、实现、恢复/不变性和 Legacy Reader 清点步骤；对应实现提交为 `0600bf65`、`cb01219c`、`6df449db`、`4de89880`、`972bbb33`、`45c00677`、`4e4f7113`、`83541d29`、`d047acd5`、`e6f160ac`。
- 所有“运行红灯测试”及“运行验证/全量验证”步骤仍保留未勾选：历史红灯输出未作为可复核证据持久化，本次最终验收测试按操作者安排由操作者执行。相应的提交存在，但提交不能替代新鲜测试证据。
- 上方审查清单同样保留为未勾选，待最终验收测试与代码审查完成后统一关闭。

## 计划自检记录（2026-07-26）

- **OpenSpec 覆盖：** RuntimeBinding/完整性（Task 1）；Final Compile 成功边界和恢复（Task 2）；Work Item Runtime Reader 与三投影职责（Task 3、5）；Group Coding Revision 物化和 Repair 不漂移（Task 4）；Schema Cutover 与三类 Workspace、端到端验证（Task 5、6）。无遗漏需求。
- **占位符扫描：** 未发现未决占位标记或依赖其他任务才能理解的步骤；每个实现任务均指定文件、接口、失败测试、验证命令和提交边界。
- **类型一致性：** `WorkItemRuntimeBinding` 的字段与现有 `WorkItemPlanRevision`、`WorkItemRevision`、`VerificationPlanRevision`、`WorkItemProjectionBundle`、`CodingAttemptPlanBinding`、`CodingExecutionUnit`、`CodingUnitRun` 的现有字段一一对应；新增 Reader 的所有消费者使用相同 `ResolvedWorkItemRuntime`。

## 执行交接

计划完成后，实施应使用 `superpowers:executing-plans` 在当前 worktree 按 Task 1 → Task 6 顺序执行；每个 Task 的红绿测试和提交都是独立审查门。不得在 Plan 获得审阅确认前启动 `/opsx:apply` 或修改产品代码。
