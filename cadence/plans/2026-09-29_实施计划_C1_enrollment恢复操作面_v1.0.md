# C1 Enrollment 恢复操作面实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: 使用 `subagent-driven-development`（推荐）或 `executing-plans` 逐任务实施。任务步骤使用 checkbox（`- [ ]`）跟踪；本计划只定义实现，不代表代码已经完成。

**Goal：** 交付 `enrollment-recovery-surface`：以统一双载体 target、版本化 enrollment binding、候选门恢复、租约三态、Failed advance 显式 retry、existing/create 计划意图、compile child 精确绑定和 Cockpit 通知操作面，形成可观察的“错误→通知→人操作→原链自动续进”闭环，并满足 A07/A09/A12/A13。

**Architecture：** 复用 `IssueAutomationStore` 的 enrollment 文件锁/CAS、`LifecycleStore` 的 session/gate durable 写面、`AdvanceStore` 的 journal、既有 `WorkspaceEngine::handle_advance`/`advance_plan`、`coding_start::start_coding_once`、worktree lock/attempt claim、compile finalizer 和 CockpitInbox/REST human-action 模式。绑定版本只表示用户明确执行重绑/换代后的当前 enrollment 事实，不建设独立 generation 服务；旧 binding、child、失败记录和事件只读保留。通知仅为 durable 状态投影，投递失败不回滚事实；自动路径在安全、已授权的 durable 状态下推进，choice、人工门、未知副作用和 Final Confirm 均通知后停等人。

**Tech Stack：** Rust 2024、Axum、Tokio、serde、既有文件锁与原子 JSON durable store；React、TypeScript、Vitest、Testing Library；不新增外部依赖。

**Spec：** `openspec/changes/enrollment-recovery-surface/{proposal.md,design.md,tasks.md,specs/work-item-group-autopilot/spec.md,specs/web-runtime-repository-routing/spec.md,specs/work-item-group-coding-execution/spec.md,specs/work-item-plan-single-candidate/spec.md,specs/work-item-plan-advance/spec.md,specs/work-item-plan-conversational-gate/spec.md}`；上位合同为 `cadence/designs/2026-09-28_方案设计_真实链缺口修复总方案_v1.2.md` §5.1、§5.5、§6.2、§7；格式参照 `cadence/plans/2026-09-27_实施计划_WIGAutopilot_P2后台coding链_v1.0.md`。

## Global Constraints

- **自动推进＋人到点处理：** plan/coding 只在安全且已授权的 durable 状态、当前 binding/target/lease/attempt/checkpoint 均可证明时自动推进；choice、人工 plan gate、外部副作用未知、租约未知活性、Final Confirm 必须通知并停等人工。系统 MUST NOT 自动批准、反馈、放弃、接管、重绑或重放未知副作用。
- 自动化默认 off；只有 story/design 人工确认后、明确 Enable 且恰一个精确 target 时生效。非 enrolled、Manual、人工 plan gate、Final Confirm 的既有行为保持零回归；多 target、缺 target、跨载体或身份不一致整体 fail-closed，不 fan-out、不猜最新 session/plan。
- 复用既有 CAS、journal、gate、lifecycle、attempt claim、worktree lock、typed `StartCoding` 和 Cockpit 投影；不得新增 durable generation、owner/incarnation/fence、全局 recovery orchestrator、通知数据库或第二套 durable operation 状态机。
- 重新绑定/换代必须由用户明确提供 plan、session、source、target、provider 和 expected 当前 binding/policy version；写入新版本并使后续动作只接受当前版本；旧 binding、旧 child、旧事件、旧 Failed、旧 gate 证据只读可查，迟到旧回执拒绝。
- retry-initialization 是独立产品动作和独立 durable retry 事实，不是重复普通 `advance`；原 Failed 记录和 journal 不改写为成功，安全本地 checkpoint 可续做同一 attempt，未知外部副作用转人工确认。
- 候选 snapshot、source revision、budget、gate identity 和 diagnostics 必须先持久化后通知；snapshot 不完整/不可读时不得显示或接受裸 approve，不扣预算、不启动 provider、不伪造已批准。
- 单仓 target 必须保留真实 physical `RepositoryRecord.id`；逻辑代码库 target 必须同时带 logical codebase id 与 logical repository id；不得把单仓降级为 logical target 或通过路径猜测。
- `existing` 只引用可解析且授权的已有 Work Item；`create` 必须声明 provider Work Item、依赖、exclusive scope、forbidden scope 和 target。区分“未声明”和“不能执行”，不得把 git 基线存在性当 create 授权或扩大 scope。
- 所有操作带稳定 `command_id`、对象身份和 expected version；同 command、同 payload 返回首次 durable 结果；同 command 异 payload、过期 binding/gate/attempt/checkpoint fail-closed。通知/WS 失败不回滚业务事实，Cockpit 通过 durable 补读。
- C1 不实现 C2 coding resilience/验证分诊、C4 LC bootstrap/repair、C5 role-chain gateway/pi recipe；不后台扫描旧 issue，不按最新 plan/session 猜接管，不自动 takeover，不清除/覆盖旧 binding 或 child。**与 C4 的排产合同（文件域交叠串行化）：** C4 中触碰本计划文件域的任务必须排后执行——C4 Task 2（`src/product/coding_attempt_store/{admission.rs,group.rs,group_validation.rs,group_initialization.rs}`）排在 C1 Task 3/6 合入之后；C4 Task 3/6/7/9（`src/web/types.rs`、`src/web/error.rs`、`src/web/app.rs`、`src/web/handlers/mod.rs` 与 Cockpit 投影 `web/src/state/workspace-cockpit-projection.ts`、`web/src/components/chat-workspace/cockpit/CockpitInbox.tsx`）排在 C1 Task 3/6/9/10 合入之后；C4 其余不碰上述交叠文件的任务（Task 1/4/5/8/10 中不涉交叠文件者）可与 C1 并行。
- 计划命令是未来 TDD 步骤，未在本计划编写时执行；实施者每个任务只运行定向测试，集成负责人最后统一项目级验证。

## Review Focus

1. **跨载体 target 和绑定版本漂移：**单仓不得被投影成 logical target，逻辑 target 缺任一级必须拒绝；旧版本动作不启动 provider、不改变当前代（Task 1、Task 2、Task 9）。
2. **候选快照不完整与观察面失效：**无 WS/relay 失败仍能从 Cockpit 恢复完整门；缺 snapshot 禁止 approve，不能扣预算或创建第二候选权威（Task 4、Task 9）。
3. **租约活/死/未知和迟到写入：**活跃只等待，死亡只在用户确认后接管，未知停等；接管后旧 owner 写入被拒，不能制造第二 attempt（Task 6、Task 9）。
4. **Failed advance 重试的审计和副作用边界：**普通 advance 不隐式 retry；retry 保留原 Failed、写独立 retry、续同一 attempt；未知副作用前必须确认（Task 7、Task 9）。
5. **existing/create 与 child 精确绑定：**合法 create 才 compile；未声明/越权分别停等；换代不复用旧 child，同代重放不建第二 child，旧 findings/history 保留（Task 5、Task 8、Task 9）。

---

## 文件责任、既有接口与统一新增契约

### 文件责任（勘察已核验）

- `src/product/models/automation.rs`：当前 `IssueAutomationEnrollment`、`EnrollmentWriteCommand`、`PreparedPlanIntent`/`PlanGenerationIntent`；本 change 添加双载体 target、版本历史和显式重绑命令的序列化模型，旧缺字段按 off/Manual 解释。
- `src/product/issue_automation_store.rs`、`src/product/issue_automation_store/tests.rs`：enrollment 锁内读/写、CAS、绑定版本追加、command 结果幂等；不覆盖旧 `compare_and_set` 语义。
- `src/web/types.rs`、`src/web/handlers/automation_enrollment.rs`、`src/web/app.rs`、`src/web/handlers/mod.rs`：重绑/换代 REST additive DTO、expected identity/version 校验和统一应用服务入口。
- `src/web/advance_plan.rs`、`src/web/autopilot_orchestrator.rs`、`src/web/coding_start.rs`：消费当前 binding/target；所有 restart/recover/new advance 重新读取 durable enrollment、lease/attempt claim；StartCoding 单入口不复制。
- `src/product/models/workspace.rs`、`src/product/work_item_plan_policy/types.rs`、`src/product/lifecycle_store/workspace.rs`、`src/product/workspace_engine/conversational_gate.rs`、`src/product/workspace_engine/conversational_gate_recovery.rs`、`src/product/work_item_plan_source_store.rs`：candidate/source/budget/gate durable snapshot 和恢复/重建，不新建候选库。
- `src/product/work_item_contract/model.rs`、`src/product/work_item_plan_compiler/{grammar.rs,parse.rs,lower.rs,validate.rs,types.rs}`、`src/product/work_item_revision_store`：existing/create/provider Work Item、依赖/scope 合同及 fail-closed compile 校验。
- `src/product/models/work_item_revision.rs`、`src/product/workspace_engine/compile/finalizer.rs`、`src/product/lifecycle_store/inputs.rs`：child binding identity 和 plan/revision/work item/binding 精确匹配；只增字段，旧 child 只读。
- `src/product/advance_store.rs`、`src/product/workspace_engine/{advance.rs,advance_split.rs}`：retry durable 记录、checkpoint/attempt identity 校验；普通 advance 仍调用现有 handler。
- `src/product/lifecycle_store/worktree.rs`、`src/product/coding_attempt_store/{attempt.rs,group.rs,admission.rs}`、`src/product/coding_workspace_engine/gates.rs`：lease/claim 三态判别、确认接管和旧写拒绝；不加 owner/fence。
- `src/web/autopilot_orchestrator.rs`、`src/web/plan_confirmed_info.rs`、`web/src/state/workspace-cockpit-projection.ts`、`web/src/state/cockpit-action-routing.ts`、`web/src/components/chat-workspace/cockpit/CockpitInbox.tsx`、`web/src/api/{client.ts,types/lifecycle.ts}`：统一等待项、通知内容、动作路由和结果补读。

### 已落地接口（只消费，不改名、不改变既有语义）

```rust
// enrollment / existing automation
IssueAutomationStore::get(&self, project_id: &str, issue_id: &str)
    -> Result<Option<IssueAutomationEnrollment>, ProductStoreError>;
IssueAutomationStore::compare_and_set(
    &self, project_id: &str, issue_id: &str,
    expected_revision: Option<u64>, command: EnrollmentWriteCommand,
) -> Result<IssueAutomationEnrollment, EnrollmentError>;
IssueAutomationStore::with_current_enrollment_locked<T>(
    &self, project_id: &str, issue_id: &str,
    f: impl FnOnce(&IssueAutomationEnrollment) -> Result<T, ProductStoreError>,
) -> Result<T, ProductStoreError>;

// advance / start
WorkspaceEngine::handle_advance(&mut self, input: AdvanceInput)
    -> Result<AdvanceOutcome, String>; // async fn（src/product/workspace_engine/advance.rs:180）
advance_plan(&WebAppState, AdvanceInput, AdvancePlanOrigin)
    -> Result<AdvanceOutcome, String>; // async fn（src/web/advance_plan.rs:31）
AdvanceStore::advance_is_ready_for_attempt(
    &self, project_id: &str, issue_id: &str, plan_id: &str, attempt_id: &str,
) -> Result<bool, ProductStoreError>;
start_coding_once(
    &WebAppState, project_id: &str, issue_id: &str, StartCodingCommand,
) -> Result<StartCodingOutcome, StartCodingError>; // async fn（src/web/coding_start.rs:144）

// durable gate/lifecycle/compile
LifecycleStore::get_workspace_session(&self, session_id: &str)
    -> Result<WorkspaceSessionRecord, ProductStoreError>;
LifecycleStore::ensure_work_item_runtime_binding(
    &self, session_id: &str, &WorkItemRuntimeBinding,
) -> Result<WorkspaceSessionRecord, ProductStoreError>;
WorkItemPlanSourceStore::get_source_revision(&self, &SourceStoreScope, &str)
    -> Result<SourceRevisionRecord, SourceStoreError>;
finalize_initial_plan_compile(existing signature); // 只增加 ChildBindingIdentity 筛选，不改函数签名
```

既有 `AdvanceOutcome::{Completed, Replayed, Rejected}`、`HumanGateRecoveryAction`、`HumanActionRequest`/`POST /api/workspace-sessions/{session_id}/human-actions`、`CockpitActionFacade` 的既有语义全部保留；新增 action 只通过 additive variant/REST handler 接入，不能复制门或 operation 状态机。

### 本计划唯一新增契约（首次定义任务见括号；下游只消费这些定义）

```rust
// Task 1: src/product/logical_codebase/types.rs（固定放置；不新建 target.rs）
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EnrollmentTarget {
    SingleRepository { repository_id: String },
    LogicalCodebase {
        logical_codebase_id: String,
        logical_repository_id: LogicalRepositoryId,
    },
}

// Task 1: automation model/store
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentBindingIdentity {
    pub binding_version: u64,
    pub enrollment_id: String,
    pub plan_id: String,
    pub session_id: String,
    pub source: EnrollmentSource,
    pub target: EnrollmentTarget,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentBindingIdentityInput {
    pub plan_id: String,
    pub session_id: String,
    pub source: EnrollmentSource,
    pub target: EnrollmentTarget,
    pub author_provider: ProviderName,
    pub reviewer_provider: ProviderName,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentBindingHistory {
    pub current: EnrollmentBindingIdentity,
    pub previous: Vec<EnrollmentBindingIdentity>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentCommandResult {
    pub command_id: String,
    pub payload_digest: String,
    pub state: OperationState,
    pub binding_version: u64,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentRebindRequest {
    pub command_id: String,
    pub expected_policy_revision: u64,
    pub expected_binding_version: u64,
    pub binding: EnrollmentBindingIdentityInput,
    pub reason: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum OperationState { Accepted, Replayed, NeedsHuman, Rejected }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnrollmentRebindResult {
    pub command_id: String,
    pub state: OperationState,
    pub enrollment: IssueAutomationEnrollment,
}

// Task 5: plan contract
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkItemIntent { Existing, Create }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkItemIntentContract {
    pub intent: WorkItemIntent,
    pub provider_work_item_id: String,
    pub depends_on: Vec<String>,
    pub exclusive_scopes: Vec<String>,
    pub forbidden_scopes: Vec<String>,
    pub target: EnrollmentTarget,
}

// Task 6: lease disposition
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LeaseDisposition { ActiveWait, DeadNeedsTakeover, UnknownNeedsHuman }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseDecision {
    pub disposition: LeaseDisposition,
    pub lease_id: String,
    pub last_activity_at: Option<String>,
    pub evidence: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseTakeoverRequest {
    pub command_id: String,
    pub expected_binding: EnrollmentBindingIdentity,
    pub expected_lease_id: String,
    pub expected_attempt_id: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LeaseTakeoverResult {
    pub command_id: String,
    pub state: OperationState,
    pub lease: LeaseDecision,
}

// Task 7: advance retry
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryInitializationRequest {
    pub command_id: String,
    pub expected_binding: EnrollmentBindingIdentity,
    pub expected_attempt_id: String,
    pub expected_checkpoint: AdvanceInitializationPhase,
    pub confirm_unknown_side_effect: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryInitializationRecord {
    pub retry_id: String,
    pub command_id: String,
    pub advance_id: String,
    pub attempt_id: String,
    pub state: OperationState,
    pub checkpoint: AdvanceInitializationPhase,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RetryInitializationResult {
    pub command_id: String,
    pub state: OperationState,
    pub retry: RetryInitializationRecord,
    pub outcome: Option<AdvanceOutcome>,
}

// 统一复用 Task 1 定义的 OperationState；此处只定义 C1InboxItem。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct C1InboxItem {
    pub id: String,
    pub kind: String,
    pub reason: String,
    pub completed_steps: Vec<String>,
    pub target: EnrollmentTarget,
    pub plan_id: Option<String>,
    pub session_id: Option<String>,
    pub attempt_id: Option<String>,
    pub gate_id: Option<String>,
    pub possible_side_effect: Option<String>,
    pub actions: Vec<String>,
    pub next_phase: Option<String>,
}
```

`EnrollmentBindingIdentityInput`、`OperationState` 的 wire 名称和 JSON `snake_case` 在 Task 1/9 固定；后续任务不得另造 `Generation`、`RecoveryOperation`、`LeaseState` 或同义 DTO。`binding_version` 是 enrollment 文件内版本历史字段，只有明确 rebind/switch-generation 追加时递增；不是 owner epoch/fence。

> **接口边界说明：** `LeaseDecision` 的 durable 证据来自现有 `IssueSharedWorktree.current_lock_owner_id`、`RepoWorktreeLockLease`、attempt status/version/claim 和既有 lease diagnostics；`retry_initialization` 只能调用现有 `handle_advance` 的安全 continuation，不改变 `AdvanceOutcome`。任何无法从这些证据证明死亡的记录都返回 `UnknownNeedsHuman`。

---

## Phase 1：冻结 target 与版本化 enrollment binding

**Phase 验收映射：** A07（恢复动作身份输入）、A09（lease/attempt 版本前置）、A12（target/scope 传递）、A13（版本化换代与旧回执拒绝）。

### Task 1：双载体 target union 与 enrollment 绑定历史/CAS

**验收映射：** A07、A09、A12、A13；本任务重点覆盖 A13 多次换代与 A09 身份/租约前置，并为 A07/A12 后续通知和 compile 提供同一 target 身份；REQ-WIGA-01、REQ-C1-TARGET-01。
**Files：**
- Modify: `src/product/logical_codebase/types.rs`、`src/product/logical_codebase/mod.rs`（固定在现有 types.rs，不新建 target.rs）。
- Modify: `src/product/models/automation.rs`、`src/product/models/mod.rs`、`src/product/issue_automation_store.rs`、`src/product/issue_automation_store/tests.rs`。
- Modify: `src/product/coding_models/execution.rs` 的 target snapshot 消费点，使 target union 可被 attempt/advance 读取；旧字段 `logical_repository_id` 只作兼容读，不猜 target。
- Test: `src/product/issue_automation_store/tests.rs` 新增 durable CAS/历史测试。

**Interfaces：**
- Consumes: 既有 `IssueAutomationEnrollment`、`EnrollmentSource`、`EnrollmentOptions`、`compare_and_set` 和 `with_current_enrollment_locked`。
- Produces: `EnrollmentTarget`、`EnrollmentBindingIdentity`、`EnrollmentBindingHistory`、`EnrollmentRebindRequest`/`EnrollmentRebindResult` 以及 `IssueAutomationStore::rebind(...)`（参数含 project/issue/request，返回 result）。`IssueAutomationEnrollment` 新增 `#[serde(default)] pub binding_history: Option<EnrollmentBindingHistory>` 与 `#[serde(default)] pub command_ledger: Vec<EnrollmentCommandResult>`；command ledger 与 current/previous 一并写入 issue 下现有 `automation-enrollment.json`，同 `command_id` 同 payload replay、异 payload reject；旧 JSON 缺字段按 `None`/空列表读取。

- [ ] **Step 1：写真实失败测试。** 在真实临时 `.aria` root 写 enrollment，断言：
  - `SingleRepository { repository_id: "repo_physical_1" }` 序列化后仍是 physical id，读取后不生成 `logical_repository_id` 替身；逻辑 target 缺 `logical_codebase_id` 或 `logical_repository_id` 时拒绝。
  - 首次 rebind 以当前 `expected_policy_revision=1`、`expected_binding_version=1` 写入 `binding_version=2`，`previous` 保留版本 1 的完整 plan/session/source/target/provider；旧版本字段逐字相等。
  - 同 `command_id`＋同 payload 重放返回同一 `binding_version=2` 且 durable 文件只有一份当前版本；同 command 异 payload、旧 expected version、跨载体 target 均返回 Conflict/Rejected，`get` 读取的 current/previous 不变。
  - 缺失新字段的旧 enrollment JSON 反序列化为 disabled/off 或 Manual-compatible projection，不自动补 plan/session/binding。

- [ ] **Step 2：运行定向命令确认红灯。**
  `cargo test --locked --lib issue_automation_store_rebind_appends_version_and_is_idempotent -- --nocapture`
  预期：FAIL，缺少 `EnrollmentTarget`/`rebind` 或版本历史字段。

- [ ] **Step 3：最小实现。** 在 enrollment 文件同一 exclusive lock 内重读 current，验证 enabled、expected policy/binding、command ledger（同键同 payload replay、同键异 payload reject）和 target union；append previous、递增 binding version、只写新的 current/command result。所有既有 Enable/Disable caller 保持原 CAS 语义，存量缺字段只按 off/Manual 读侧处理。不要创建 generation service、扫描旧 session 或覆盖旧 binding。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑 Step 2，并运行 `cargo test --locked --lib issue_automation_store_ -- --nocapture`；预期新增版本/CAS 测试 PASS，既有 enable/disable/concurrent tests PASS。

- [ ] **Step 5：提交。**
  ```bash
  git add src/product/logical_codebase src/product/models/automation.rs src/product/models/mod.rs src/product/issue_automation_store.rs src/product/issue_automation_store/tests.rs src/product/coding_models/execution.rs
  git commit -m "feat: add versioned enrollment target bindings"
  ```

### Task 2：重绑/换代 REST 和调用者迁移


**验收映射：** A07、A09、A12、A13；本任务重点覆盖 A13 失败代换代/旧回执拒绝与 A09 过期绑定拒绝，并为 A07/A12 操作面提供统一 identity；REQ-WIGA-01、REQ-C1-TARGET-01、REQ-WIGA-04。
**Files：**
- Modify: `src/web/types.rs`、`src/web/handlers/automation_enrollment.rs`、`src/web/handlers/mod.rs`、`src/web/app.rs`。
- Modify: `src/web/handlers/automation_target.rs`、`src/web/advance_plan.rs`、`src/web/autopilot_orchestrator.rs`、`src/web/plan_confirmed_info.rs`，统一消费 `EnrollmentBindingIdentity`/`EnrollmentTarget`。
- Modify: `web/src/api/types/lifecycle.ts`、`web/src/api/client.ts`、`web/src/components/lifecycle/useIssueLifecycleGeneration.ts`；Test: `src/web/handlers/automation_enrollment_test_support.rs` 及 handler `#[cfg(test)]`、`web/src/api/client.test.ts`。

**Interfaces：**
- Consumes: Task 1 的 `EnrollmentTarget`、`EnrollmentBindingIdentity`、`EnrollmentRebindRequest`/`Result`、`IssueAutomationStore::rebind`。
- Produces: `POST /api/projects/{project_id}/issues/{issue_id}/automation-enrollment/rebind`；前端 `rebindAutomationEnrollment(...)`；所有 enrolled `AdvancePlanOrigin`/StartCoding/通知投影的当前 binding 预检。

- [ ] **Step 1：写真实失败测试。** 通过 Axum router 和 fixture 发送真实 JSON：合法单仓 rebind 返回 200/current version 2；逻辑 target 缺一级、多个候选、plan/session 不属于 issue、source/provider 漂移或旧 expected version 返回 409/422，且读取 enrollment/old binding 不变；同 command 同 payload 重放不触发第二 wake/编排，异 payload reject。增加旧 generation 回执调用 `advance_plan`/typed `start_coding_once` 的 durable no-start 断言。

- [ ] **Step 2：运行定向命令确认红灯。** `cargo test --locked --lib automation_enrollment_rebind_requires_current_binding -- --nocapture`；`pnpm -C web exec vitest run src/api/client.test.ts`。预期：FAIL，路由/DTO/客户端函数不存在，旧单仓 target 仍被 routing 拒绝或旧回执未校验 binding。

- [ ] **Step 3：最小实现。** REST 仅调用 Task 1 store 应用服务；先验证 issue/source/plan/session/target/provider 的精确身份，再带 expected policy/binding CAS。将 `automation_target`、`advance_plan`、`autopilot_orchestrator`、`plan_confirmed_info` 和 StartCoding enrolled preflight 改为读取当前 binding；不要从“最新 plan/session”恢复。前端 lifecycle DTO/client/按钮发送稳定 command_id 和完整 target/provider/source；保持 Enable 默认 off 和旧 Manual 路径。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑 Rust handler 测试、`cargo test --locked --lib advance_plan -- --nocapture`（经模块路径 `web::advance_plan::tests::*` 命中该文件全部测试）与 `pnpm -C web exec vitest run src/api/client.test.ts`；预期 REST、过期回执和客户端 payload 测试 PASS。

- [ ] **Step 5：提交。**
  ```bash
  git add src/web/types.rs src/web/handlers/automation_enrollment.rs src/web/handlers/automation_target.rs src/web/handlers/mod.rs src/web/app.rs src/web/advance_plan.rs src/web/autopilot_orchestrator.rs src/web/plan_confirmed_info.rs web/src/api/types/lifecycle.ts web/src/api/client.ts web/src/api/client.test.ts web/src/components/lifecycle/useIssueLifecycleGeneration.ts src/web/handlers/automation_enrollment_test_support.rs
  git commit -m "feat: expose enrollment rebind action"
  ```

### Task 3：当前 binding/target 贯通 prepare、advance、StartCoding 与通知输入


**验收映射：** A07、A09、A12、A13；本任务重点覆盖 A09 lease/attempt 前置与 A13 旧回执，并使 A07/A12 后续恢复、合同和通知消费当前 binding；REQ-WIGA-03、REQ-WIGA-04、REQ-C1-TARGET-01。
**Files：**
- Modify: `src/product/models/automation.rs` 的 `PreparedPlanIntent`/`PlanGenerationIntent` additive binding identity、`src/product/issue_automation_store.rs` 相关 intent 校验。
- Modify: `src/web/handlers/lifecycle/plan_preparation.rs`、`src/web/advance_plan.rs`、`src/web/autopilot_orchestrator.rs`、`src/web/coding_start.rs`。
- Test: `src/web/wiga_gate_fixture.rs`、`src/web/advance_plan.rs`、`src/web/coding_start.rs` 中的现有测试模块；使用真实 durable enrollment/attempt assertion。

**Interfaces：**
- Produces: `load_current_enrollment_binding(...) -> Result<EnrollmentBindingIdentity, String>`（单一读取/校验 helper）；现有 prepare/advance/start/reconcile 只接受该 helper 的结果。

- [ ] **Step 1：写真实失败测试。** fixture 先写当前 binding v1，再制造 v2；验证自动 reconcile/restart/new advance 每次重读 v2，v1 的 generation/advance/start 回执均 fail-closed，`CodingAttemptStore::list_attempts_for_issue` 数量不增加；当前 binding 对应的唯一 plan/session 才可继续。验证人工 `AdvancePlanOrigin::Manual` 不被 enrollment 自动升级。

- [ ] **Step 2：运行定向命令确认红灯。** `cargo test --locked --lib enrolled_advance_reloads_current_binding_and_rejects_old_generation -- --nocapture`；预期：FAIL，当前 advance 仍只核对 policy_revision/plan/session 或按旧 logical 字段。

- [ ] **Step 3：最小实现。** 在每次 prepare/advance/start/reconcile 前经同一 helper 重读 enrollment，核对 binding version、enrollment id、plan/session/source/provider/target；将 binding identity 传给既有 typed StartCoding，不新增 runner/provider 入口。仅在 confirmed+completed+Ready 等已有门满足时继续；Manual、人工门和 Final Confirm 不变。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑 Step 2，运行 `cargo test --locked --lib start_coding -- --nocapture`（`src/web/coding_start.rs` 现有测试名为 `start_coding_*` 前缀）与 `cargo test --locked --lib enrolled_advance_rejects_identity_drift_and_disabled_enrollment -- --nocapture`（`src/web/advance_plan.rs` 既有身份漂移回归；`src/web/wiga_gate_fixture.rs` 是 fixture 模块、无 `#[test]`，不作过滤器）；预期无第二 attempt/provider start，旧回执拒绝。

- [ ] **Step 5：提交。**
  ```bash
  git add src/product/models/automation.rs src/product/issue_automation_store.rs src/web/handlers/lifecycle/plan_preparation.rs src/web/advance_plan.rs src/web/autopilot_orchestrator.rs src/web/coding_start.rs src/web/wiga_gate_fixture.rs
  git commit -m "fix: enforce current enrollment binding across autopilot"
  ```

---

## Phase 2：候选恢复门与 existing/create 计划合同

**Phase 验收映射：** A07（完整候选快照后恢复）、A09（恢复动作不越过 attempt/lease）、A12（existing/create compile 约束）、A13（旧 gate/contract 身份 fail-closed）。

### Task 4：候选 snapshot 先持久化与恢复/重建动作

**验收映射：** A07、A09、A12、A13；本任务重点覆盖 A07 孤儿/降级门和 A13 旧 gate identity reject，同时为 A09 lease/attempt、A12 合同失败通知复用统一 gate/binding；REQ-C1-GATE-01/02、REQ-WIGA-03。
**Files：**
- Modify: `src/web/types.rs`、`src/web/handlers/workspace_human_action.rs`、`src/web/app.rs`：在既有 `POST /api/workspace-sessions/{session_id}/human-actions` 的 `HumanActionRequest` 增加唯一 additive `CandidateRecovery` action variant，不新增第二 candidate recovery route。
- Modify: `src/product/work_item_plan_policy/types.rs`（`HumanGateSnapshot` additive completeness/diagnostics）、`src/product/models/workspace.rs`、`src/product/lifecycle_store/workspace.rs`。
- Modify: `src/product/workspace_engine/conversational_gate.rs`、`src/product/workspace_engine/conversational_gate_recovery.rs`、`src/product/work_item_plan_source_store.rs`；复用 `SourceRevisionRecord` 和现有 snapshot refs。
- Test: `src/product/workspace_engine/tests/conversational_gate_recovery.rs`、`src/product/workspace_engine/tests/conversational_gate_revision/`、`src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix.rs`。

**Interfaces：**
- Consumes: `HumanGateSnapshot`、`HumanGateTurn`、`SourceRevisionRecord`/`WorkItemPlanSourceStore`、`recover_human_gate_turn`、既有 typed feedback/approve/abandon。
- Produces: Task 1 binding 校验下的 `CandidateRecoveryAction`、`CandidateRecoveryRequest/Result`；`persist_candidate_snapshot_before_relay(...)` 和 `recover_candidate_gate(...)`；无完整 snapshot 时 `approve` 固定 fail-closed。

- [ ] **Step 1：写真实失败测试。** 通过真实 LifecycleStore session 写入完整 candidate/source/budget/gate/diagnostics，再模拟 relay/WS consumer 不存在：重开 engine 后能读同一 snapshot，恢复请求返回 accepted，原 gate 仍可用既有 feedback/approve/abandon；同 command 重放不重复扣 budget/provider/turn。另写缺 snapshot/source/budget 的 durable session：Cockpit/REST 不可 approve，recovery/rebuild 可见，session phase/budget/provider ledger 不变；旧 gate/version 返回 conflict。

- [ ] **Step 2：运行定向命令确认红灯。** `cargo test --locked --lib orphaned_candidate_snapshot_requires_recovery_before_approve -- --nocapture`；预期：FAIL，缺少完整性判定/恢复动作，当前裸 approve 或无恢复入口。

- [ ] **Step 3：最小实现。** 在 relay/observer 之前用 session/gate/source store 的同一持久路径原子写 candidate 全文、source revision ref/hash、budget、gate id/version、diagnostics；恢复动作只恢复原 turn/phase或从权威 source/IR/report 重建，不新建候选权威、不扣预算、不启动 provider。REST/WS 均调用同一应用服务，携 command_id、expected gate/binding；通知投递失败仅靠 durable 补读。保留既有 typed feedback/approve/abandon 矩阵。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑 Step 2，运行 `cargo test --locked --lib conversational_gate_ -- --nocapture` 和 `cargo test --locked --lib campaign_stage3_recovery_matrix -- --nocapture`；预期 A07 场景证据完整、无裸 approve/重复 turn。

- [ ] **Step 5：提交。**
  ```bash
  git add src/product/work_item_plan_policy/types.rs src/product/models/workspace.rs src/product/lifecycle_store/workspace.rs src/product/workspace_engine/conversational_gate.rs src/product/workspace_engine/conversational_gate_recovery.rs src/product/work_item_plan_source_store.rs src/web/types.rs src/web/handlers/workspace_human_action.rs src/web/app.rs src/product/workspace_engine/tests src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix.rs
  git commit -m "feat: persist candidate gate recovery facts"
  ```

### Task 5：existing/create/provider Work Item 计划合同与 compile 校验


**验收映射：** A07、A09、A12、A13；本任务重点覆盖 A12 新文件 AC 与 A07 缺失意图停等，并验证 A09 受 target/attempt 约束、A13 跨代 contract 不污染旧历史；REQ-C1-PLAN-01。
**Files：**
- Modify: `src/product/work_item_contract/model.rs`（`CanonicalWorkItemContract` additive intent/target/provider fields）、`src/product/work_item_plan_compiler/{grammar.rs,parse.rs,lower.rs,validate.rs,types.rs}`。
- Modify: `src/product/work_item_revision_store` 的保存/完整性校验和 `src/product/workspace_engine/compile.rs` 入口；保持 `depends_on`、`WorkItemWritePolicy::{exclusive_scopes,forbidden_scopes}` 原义。
- Modify: `web/src/api/types/work-item-plan.ts`、对应 plan DTO serialization；Test: `src/product/work_item_plan_compiler/tests/{full_lowering_validator.rs,blockers.rs}` 及新增 C1 contract test。

**Interfaces：**
- Consumes: `CanonicalWorkItemContract`、`RequiredInputContract.provider_logical_work_item_id`、`depends_on`、`WorkItemWritePolicy`、`validate_plan_candidate_ir`。
- Produces: Task 1 `WorkItemIntent`/`WorkItemIntentContract` 的 canonical contract fields；`validate_work_item_intent_contract(...) -> Result<(), Vec<CompilerDiagnostic>>`；diagnostic reason code `intent_undeclared`（未声明）与 `intent_unexecutable`（不能执行）。

- [ ] **Step 1：写真实失败测试。** 用真实 plan source/IR/contract 编译：
  - explicit `Create` + provider Work Item + dependency + allowed/exclusive scope + forbidden scope + target 可通过，保存后的 revision/binding 可读且保留意图；
  - provider 输出不存在路径但未声明 create 返回 `intent_undeclared`，进入 feedback/revision wait，不产生 work item/compile child；
  - 错 provider Work Item、缺 dependency、exclusive/forbidden scope 冲突、target 不符返回 `intent_unexecutable`，旧 findings/contract durable 不变。

- [ ] **Step 2：运行定向命令确认红灯。** `cargo test --locked --lib work_item_plan_compiler::tests::c1_existing_create -- --nocapture`；预期：FAIL，canonical contract/grammar 尚无 intent 字段和 diagnostics。

- [ ] **Step 3：最小实现。** 用现有 canonical contract/IR/validation 链添加可选但在 C1 compile 路径必须显式的 intent contract；existing 解析授权目标后才通过，create 逐项校验 provider id、依赖闭包、target 和 scope，禁止从 git 基线推导授权。validator 把两类错误分开，写入既有 findings 并停到 feedback/revision，不清除既有 finding、不扩大 scope；旧缺字段数据走 Manual/旧路径但不被自动 enrollment 采用。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑 Step 2，运行 `cargo test --locked --lib full_lowering_validator -- --nocapture` 和 `cargo test --locked --lib blockers -- --nocapture`；预期合法 create 与四类反例 durable 断言 PASS。

- [ ] **Step 5：提交。**
  ```bash
  git add src/product/work_item_contract/model.rs src/product/work_item_plan_compiler src/product/work_item_revision_store src/product/workspace_engine/compile.rs web/src/api/types/work-item-plan.ts
  git commit -m "feat: validate explicit existing and create intents"
  ```

---

## Phase 3：租约三态、advance retry 与 compile child

**Phase 验收映射：** A07（等待项恢复结果可投影）、A09（lease 三态与同 attempt retry）、A12（child contract 可追溯）、A13（跨代 child 与旧历史保留）。

### Task 6：复用 worktree lock/attempt claim 实现租约三态与确认接管


**验收映射：** A07、A09、A12、A13；本任务重点覆盖 A09 lease 冲突/确认接管与 A13 旧 owner/旧代迟到写入，同时为 A07/A12 通知和 scope 校验提供统一判定；REQ-WIGA-03、REQ-C1-TARGET-01。
**Files：**
- Modify: `src/product/lifecycle_store/worktree.rs`、`src/product/coding_attempt_store/{attempt.rs,group.rs,admission.rs}`、`src/product/coding_workspace_engine/gates.rs`。
- Modify: `src/web/autopilot_orchestrator.rs`、`src/web/advance_plan.rs`、`src/web/coding_start.rs`，每次 restart/recover/new advance 重读 lease/claim。
- Modify: `src/web/types.rs`、`src/web/handlers/automation_enrollment.rs`、`src/web/handlers/mod.rs`、`src/web/app.rs`：`LeaseTakeoverRequest/Result` DTO 与新路由 `POST /api/projects/{project_id}/issues/{issue_id}/automation-enrollment/lease/takeover`（不复用既有 `POST /api/workspace-sessions/{session_id}/takeover` 的 stopped_needs_human 语义）。
- Test: `src/product/coding_attempt_store/admission_tests.rs`、`src/product/lifecycle_store/worktree.rs` 相邻 tests、`src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix.rs`。

**Interfaces：**
- Consumes: `IssueWorktreeLockLease`、`RepoWorktreeLockLease`、`try_acquire_issue_worktree_lock`/`try_acquire_repo_worktree_lock`、`CodingAttemptStatus::is_active`、attempt version/claim、`route_issue_shared_worktree`（`src/product/coding_workspace_engine/gates.rs:91-114`，`pub(crate)`）。
- Produces: Task 1 identity 下 `LeaseDisposition::{ActiveWait,DeadNeedsTakeover,UnknownNeedsHuman}`、`LeaseDecision`、`LeaseTakeoverRequest`/`LeaseTakeoverResult`；`classify_worktree_lease(...)`；`confirm_takeover(LeaseTakeoverRequest) -> LeaseTakeoverResult`（expected binding/lease/attempt CAS；应用服务落在 `src/product/coding_workspace_engine/gates.rs`，与 `route_issue_shared_worktree` 同模块，在既有 worktree 文件锁与 attempt claim CAS 内写新 owner）；REST 新路由见 Files；迟到旧 owner 写入统一 IdentityMismatch。

- [ ] **Step 1：写真实失败测试。** durable fixture 分别写：active owner/active attempt（返回 ActiveWait，通知等待且第二 `try_acquire` 不改变 owner/attempt）；terminal attempt 或已明确释放 owner（返回 DeadNeedsTakeover，未确认不能继续，确认后原子写新 owner）；缺 last activity/owner 证据或读失败（UnknownNeedsHuman，绝不抢占）。确认后用旧 lease/旧 binding 写 lock/attempt，断言 `IdentityMismatch` 且 current owner/attempt 不变；并发确认只有一个成功。

- [ ] **Step 2：运行定向命令确认红灯。** `cargo test --locked --lib lease_disposition_active_dead_unknown_is_fail_closed -- --nocapture`；预期：FAIL，尚无三态分类和确认接管应用服务。

- [ ] **Step 3：最小实现。** 只从现有 lock record、attempt terminal/activity/status 和 journal 证据分类：活跃→通知等待；死亡→只产生 needs-confirm，确认时在既有文件锁/CAS 内写新的合法 owner；未知→停等/重新绑定提示。不得引入 epoch/fence/owner incarnation；禁止自动 takeover/重启 provider。所有 restart/recover/new advance 先重新分类再调用原 advance/StartCoding。新 REST takeover 路由仅是 `confirm_takeover` 应用服务的薄入口，不承载分类/判定逻辑。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑 Step 2，运行 `cargo test --locked --lib admission_ -- --nocapture` 与 `cargo test --locked --lib campaign_stage3_recovery_matrix -- --nocapture`；预期三态、并发、旧写拒绝 PASS。

- [ ] **Step 5：提交。**
  ```bash
  git add src/product/lifecycle_store/worktree.rs src/product/coding_attempt_store/attempt.rs src/product/coding_attempt_store/group.rs src/product/coding_attempt_store/admission.rs src/product/coding_workspace_engine/gates.rs src/web/autopilot_orchestrator.rs src/web/advance_plan.rs src/web/coding_start.rs src/product/coding_attempt_store/admission_tests.rs src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix.rs
  git commit -m "feat: classify enrollment lease recovery safely"
  ```

### Task 7：Failed advance 显式 retry-initialization 状态机

**验收映射：** A07、A09、A12、A13；本任务重点覆盖 A09 Failed advance retry 与 A13 过期 binding/旧回执，并验证 A07 恢复结果及 A12 合同编译不会绕过 retry 身份；REQ-ADV-C1-RETRY、REQ-WIGA-03。

**Files：**
- Modify: `src/product/advance_store.rs`（additive retry record/index/lock methods）、`src/product/workspace_engine/advance.rs`/`advance_split.rs`（checkpoint-safe continuation）、`src/web/advance_plan.rs`。
- Modify: `src/web/types.rs`、`src/web/handlers/mod.rs`、`src/web/app.rs`，新增 retry REST action；Test: `src/product/advance_store.rs` unit tests、`src/product/workspace_engine/tests/advance_handler.rs`、`src/web/workspace_ws_handler/tests/campaign_stage3_advance.rs`。

**Interfaces：**
- Consumes: existing `AdvanceRecord`/`AdvanceInitializationJournal`、`AdvanceInitializationPhase`、`mark_advance_initialization_error`、`advance_initialization_phase`、Task 1 binding and Task 6 `LeaseDecision`。
- Produces: Task 7 `RetryInitializationRequest`/`RetryInitializationRecord`/`RetryInitializationResult`；`AdvanceStore::create_retry_initialization(...)`（same command replay/异 payload conflict）；`retry_initialization(RetryInitializationRequest) -> Result<RetryInitializationResult, String>`——`RetryInitializationResult` 固定为 retry REST 路由的响应体，`NeedsHuman` 由 `state: OperationState` 承载（此时 `outcome=None`）；REST `POST /api/projects/{project_id}/issues/{issue_id}/work-item-plans/{plan_id}/advance/retry-initialization`。

- [ ] **Step 1：写真实失败测试。** 注入现有 failpoint 使真实 advance record/journal 在 `JournalPrepared`、`AttemptPersisted` 或 `UnitsMaterialized` 后 Failed，保存 attempt id/checkpoint/error；调用 retry endpoint/service：
  - 同 binding/plan revision/target/attempt/checkpoint 写一条独立 retry record，原 `AdvanceRecord.status=Failed`、error、journal failure、created_at 保持不变，安全本地 prefix 续做同一 attempt 到 Ready；attempt 文件数量和 id 不变；
  - 普通 `advance` 同/异 command 返回原 Failed 和 error，不创建 retry/attempt/provider；
  - checkpoint/target/binding 过期拒绝且 durable 全不变；未知副作用未携 `confirm_unknown_side_effect` 时返回 `RetryInitializationResult { state: NeedsHuman, outcome: None }`，不重跑步骤；同 retry command 重放返回原 retry result。

- [ ] **Step 2：运行定向命令确认红灯。** `cargo test --locked --lib retry_initialization_preserves_failed_record_and_attempt -- --nocapture`；预期：FAIL，只有原 record/journal，没有独立 retry action/record。

- [ ] **Step 3：最小实现。** 在 `AdvanceStore` 同一 issue lock 下 append retry record，以 advance id、plan/revision、target、attempt、checkpoint、binding identity 作为不可变键；retry 仅调用现有 checkpoint continuation，不调用“重新创建 record/attempt”的普通入口。未知副作用 phase 必须先返回 NeedsHuman；用户确认后才进入既有安全恢复/重新绑定路径。普通 `handle_advance` 遇 Failed 保持 `Rejected/Replayed` 原事实，绝不 reset journal 或改 Failed 为 Ready。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑 Step 2，运行 `cargo test --locked --lib advance_handler -- --nocapture` 和 `cargo test --locked --lib campaign_stage3_advance -- --nocapture`；预期原 Failed、retry record、同 attempt Ready 和普通 advance no-op 均 PASS。

- [ ] **Step 5：提交。**
  ```bash
  git add src/product/advance_store.rs src/product/workspace_engine/advance.rs src/product/workspace_engine/advance_split.rs src/web/advance_plan.rs src/web/types.rs src/web/handlers/mod.rs src/web/app.rs src/product/workspace_engine/tests/advance_handler.rs src/web/workspace_ws_handler/tests/campaign_stage3_advance.rs
  git commit -m "feat: add explicit advance initialization retry"
  ```

### Task 8：compile child/session 精确绑定与跨代新建

**验收映射：** A07、A09、A12、A13；本任务重点覆盖 A13 跨代 child/旧历史与 A12 合同可追溯，并验证 A07 恢复候选、A09 attempt/target 身份不能复用旧 child；REQ-C1-CHILD-01、REQ-C1-TARGET-01。

**Files：**
- Modify: `src/product/models/work_item_revision.rs`、`src/product/lifecycle_store/inputs.rs`、`src/product/models/workspace.rs`/`src/product/lifecycle_store/workspace.rs`，为 `WorkItemRuntimeBinding`/session 添加 current binding identity/version（旧 JSON `None` 只读）。
- Modify: `src/product/workspace_engine/compile/finalizer.rs`，替换当前仅 `workspace_type == WorkItem && entity_id == logical_id` 的筛选。
- Test: `src/product/workspace_engine/tests/part_03/part_11.rs`、`src/product/workspace_engine/tests/part_03/part_12.rs`（在现有 focused tests 中增加 finalizer binding cases）。

**Interfaces：**
- Consumes: Task 1 `EnrollmentBindingIdentity`、Task 5 validated contract/provider intent、现有 `WorkItemRuntimeBinding`、`LifecycleStore::ensure_work_item_runtime_binding`。
- Produces: `ChildBindingIdentity { plan_id, plan_revision_id, logical_work_item_id, work_item_revision_id, binding_version, enrollment_id, target }`；`match_compile_child(session, expected) -> bool`；`create_workspace_session` 的 additive binding identity 输入。

- [ ] **Step 1：写真实失败测试。** 通过真实 finalizer fixture：
  - 旧 child 仅 entity_id 相同但 plan/revision/work-item revision/binding v1 不同，当前 v2 compile 不复用，创建一个 v2 child；旧 child/session binding JSON 逐字段不变；
  - 当前四元身份＋binding identity 完全相同的 compile/replay 命中同一 child，不创建第二 session；
  - 任一 plan/revision/work-item/target/binding 不匹配时 fail-closed（不能静默复用，也不能清旧历史）。

- [ ] **Step 2：运行定向命令确认红灯。** `cargo test --locked --lib finalizer_rejects_old_binding_child -- --nocapture`；预期：FAIL，当前 finalizer 按 entity_id 复用旧 child。

- [ ] **Step 3：最小实现。** 先把 `ChildBindingIdentity` 作为 session/runtime binding 的 additive durable 字段写入，再在 `finalize_initial_plan_compile` 中按 plan id、plan revision id、logical work item id、work item revision id、当前 enrollment binding version/enrollment id/target 全匹配。零或仅旧 child 时创建新 child；多个精确匹配仍报 identity conflict；同代 ensure 保持幂等。旧 child 不迁移、不覆盖、不删除。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑 Step 2，运行 `cargo test --locked --lib work_item_plan_reviewer_prompt -- --nocapture`、`cargo test --locked --lib plan_repair_reviewer_orchestration -- --nocapture` 与 `cargo test --locked --lib plan_repair_real_prepare_review_publish -- --nocapture`（`part_03/part_11.rs`、`part_03/part_12.rs` 经 `include!` 展开进 `part_03` 模块，`part_03::part_11` 不是有效过滤器，须用实际测试函数名）；预期跨代新 child、同代 replay 和旧历史保留 PASS。

- [ ] **Step 5：提交。**
  ```bash
  git add src/product/models/work_item_revision.rs src/product/lifecycle_store/inputs.rs src/product/models/workspace.rs src/product/lifecycle_store/workspace.rs src/product/workspace_engine/compile/finalizer.rs src/product/workspace_engine/tests/part_03/part_11.rs src/product/workspace_engine/tests/part_03/part_12.rs
  git commit -m "fix: bind compile children to current enrollment generation"
  ```

---

## Phase 4：统一通知、驾驶舱操作和端到端回归

**Phase 验收映射：** A07/A09/A12/A13（每类故障均验证错误→通知→用户动作→原链自动续进）。

### Task 9：统一 C1 inbox/REST action 投影与 Cockpit 操作闭环

**验收映射：** A07、A09、A12、A13；本任务综合覆盖四类故障的通知→用户动作→durable 结果→原链续进，并落实 REQ-C1-GATE-02、REQ-WIGA-03/04、tasks.md §3.2/§4.1。

**Files：**
- Modify: `src/web/types.rs`、`src/web/error.rs`、`src/web/handlers/automation_enrollment.rs`、retry/lease/candidate handlers；所有 action 结果使用 Task 1/4/6/7 的 `OperationState`。
- Modify: `src/web/autopilot_orchestrator.rs`、`src/web/plan_confirmed_info.rs`，将 durable waiting facts 投影为统一系统通知/inbox，不以事件当权威。
- Modify: `web/src/state/workspace-cockpit-projection.ts`、`web/src/state/cockpit-action-routing.ts`、`web/src/components/chat-workspace/cockpit/CockpitInbox.tsx`、`web/src/pages/ChatCockpitPage.tsx`、`web/src/api/client.ts`、`web/src/api/types/lifecycle.ts`。
- Test: `src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix.rs`、enrollment/advance handler tests、`web/src/components/chat-workspace/cockpit/CockpitInbox.test.tsx`、`web/src/pages/ChatCockpitPage.inbox.test.tsx`；若 `cockpit-action-routing` 测试文件尚不存在，则在 `CockpitInbox.test.tsx` 或既有页面 action test 中覆盖 facade 的真实 URL/body 接线。

**Interfaces：**
- Consumes: `C1InboxItem`/`OperationState`、candidate recovery、`LeaseTakeoverRequest`/`LeaseTakeoverResult`（Task 6 新 takeover 路由）、`RetryInitializationResult`（Task 7 retry 路由固定响应体，`NeedsHuman` 由 `state` 承载）、rebind REST contracts、既有 `CockpitInboxItem`、`CockpitActionFacade`、`postWorkspaceHumanAction`/`takeoverWorkspaceSession` patterns。
- Produces: durable waiting projection `list_c1_waiting_items(...)`、C1 action result projection；Cockpit actions `rebind/switchGeneration`、`recoverCandidate`、`retryInitialization`、`confirmTakeover`，每个只调用对应 REST/application service（`confirmTakeover`→Task 6 新 lease takeover 路由；`retryInitialization`→Task 7 retry 路由，按 `RetryInitializationResult.state` 投影等待/结果），不在前端判定业务成功。

- [ ] **Step 1：写真实失败测试。** 使用真实 fixture 先制造四类 durable waiting facts：孤儿 candidate、active/dead/unknown lease、Failed advance、intent undeclared/unexecutable、旧代 child；重开 router/页面后 inbox 必展示 reason、completed steps、target、plan/session/attempt/gate、possible side effect、可用按钮和下一阶段。点击按钮发送稳定 command_id；返回未知后同 command 重试只得到同一 durable result，旧/错 version 显示刷新/重新绑定，UI 不自行推进状态。前端测试断言按钮文本和 `requestJson` 的真实 URL/body，不只测 mock callback 次数。

- [ ] **Step 2：运行定向命令确认红灯。** `cargo test --locked --lib c1_waiting_items_include_identity_and_actions -- --nocapture`；`pnpm -C web exec vitest run src/components/chat-workspace/cockpit/CockpitInbox.test.tsx src/pages/ChatCockpitPage.inbox.test.tsx`。预期：FAIL，尚无统一 C1 projection/action props/DTO。

- [ ] **Step 3：最小实现。** 从各既有 durable store 派生 waiting item，写入既有 inbox/system notification 投影；通知失败不影响业务事实，GET/页面 hydration 可补读。CockpitInbox 仅展示/触发 REST，`CockpitActionFacade` 添加 additive actions 并保留 `confirm/feedback/terminate/advance/confirmBatch/recoverCompile`；active gate、Final Confirm、Manual 仍按原矩阵拦截。动作成功后由 autopilot wake/reconcile 继续原链，失败/needs_human 显示真实状态和下一按钮。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑 Step 2，运行 `pnpm -C web exec vitest run src/state/cockpit-action-routing.test.ts src/components/chat-workspace/cockpit/CockpitInbox.test.tsx`（若 action routing 测试并入既有页面测试，则运行实际承载该测试的文件）和 `cargo test --locked --lib campaign_stage3_recovery_matrix -- --nocapture`；预期 A07/A09/A12/A13 的通知→动作→durable 结果闭环 PASS。

- [ ] **Step 5：提交。**
  ```bash
  git add src/web/types.rs src/web/error.rs src/web/handlers src/web/autopilot_orchestrator.rs src/web/plan_confirmed_info.rs src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix.rs web/src/state/workspace-cockpit-projection.ts web/src/state/cockpit-action-routing.ts web/src/components/chat-workspace/cockpit/CockpitInbox.tsx web/src/pages/ChatCockpitPage.tsx web/src/api/client.ts web/src/api/types/lifecycle.ts web/src/components/chat-workspace/cockpit/CockpitInbox.test.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx
  git commit -m "feat: close C1 recovery actions in cockpit"
  ```

### Task 10：C1 契约集成、兼容边界与 A07/A09/A12/A13 定向回归

**验收映射：** A07、A09、A12、A13 全部；tasks.md §4.1–4.2；v1.2 §5.1、§5.5、§5.6、§7。

**Files：**
- Modify: `openspec/changes/enrollment-recovery-surface/specs/*/spec.md` 交叉引用/实现状态（只同步 approved delta，不增加要求）、必要时 `openspec/changes/enrollment-recovery-surface/tasks.md` 勾选实现状态；不改 proposal/design 契约。
- Modify: 受影响 Rust/TS 测试 fixture 以覆盖旧 enrollment JSON、Manual/off、人工 plan gate、Final Confirm 和单仓/逻辑双载体。
- Test: 新增或扩展 `src/web/wiga_gate_fixture.rs` 的 C1 campaign/真实 durable tests；`src/web/handlers/automation_enrollment_test_support.rs`；前端 Cockpit/inbox tests。

**Interfaces：**
- Consumes: Tasks 1–9 全部契约；既有 `HumanActionRequest`、`StartCoding`、advance journal、gate/lease/child stores。
- Produces: C1 定向验收测试/记录和兼容边界断言；不产生新生产接口。

- [ ] **Step 1：写失败测试。** 用真实产品 fixture（不直接编辑权威 JSON 作为解法）覆盖：
  1. A07：无 WS consumer/relay 失败→完整 snapshot durable→inbox 恢复/重建→既有 feedback/approve→自动继续，issue 不 abandon；
  2. A09：活 lease 等待、死 lease 用户确认 takeover、未知 lease 停等；Failed advance 显式 retry 到 Ready；原 Failed/attempt/审计保留、无第二 attempt/provider；
  3. A12：合法 create compile；未声明、错拼、缺依赖、forbidden/越权 create 分别停等/失败且不扩大 scope；
  4. A13：失败代→明确换代/换 provider→再次失败→再次 adopt；旧 binding/child/history 可查，同代重放无第二链，旧回执不改当前代；
  5. 兼容：缺新字段的 enrollment/child/session 按 off/Manual 解释；非 enrolled、Manual、人工 plan gate、Final Confirm 的旧测试仍观察原结果。
  每个断言读取真实 durable enrollment/history/session/source/gate/advance retry/attempt/lease/child/通知结果，而非只读 mock 回调或字段非空。

- [ ] **Step 2：运行定向命令确认红灯。** `cargo test --locked --lib c1_a07_a09_a12_a13_recovery_surface -- --nocapture`；`pnpm -C web exec vitest run src/pages/ChatCockpitPage.inbox.test.tsx src/components/chat-workspace/cockpit/CockpitInbox.test.tsx`。预期：FAIL，综合链尚未覆盖全部 C1 action。

- [ ] **Step 3：最小实现。** 只补齐任务间 wiring、DTO/route 交叉引用和测试 fixture；若测试揭示接口冲突，回到负责 Task 的唯一契约修正并迁移所有 caller，不新增同义类型/旁路。保持 capability delta 与 C2/C4/C5 边界；不以修改测试期望、删除旧记录、重启服务或直接改 JSON 通过。

- [ ] **Step 4：运行定向命令确认绿灯。** 重跑两条定向命令，逐行记录 A07/A09/A12/A13 的 durable IDs、错误→通知→command_id→结果→下一阶段证据；另运行受影响现有 scoped regression（各任务已有命令）和前端 Cockpit action tests。此任务只做定向 C1 复验，不在任务内运行项目级全量 build/lint/formatter。

- [ ] **Step 5：提交。**
  ```bash
  # 仅添加本任务实际修改的 openspec 目录与 fixture/测试文件；禁止 git add src/product、src/web、web/src 整目录
  git add openspec/changes/enrollment-recovery-surface/specs openspec/changes/enrollment-recovery-surface/tasks.md src/web/wiga_gate_fixture.rs src/web/handlers/automation_enrollment_test_support.rs src/product/issue_automation_store/tests.rs src/product/coding_attempt_store/admission_tests.rs src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix.rs src/web/workspace_ws_handler/tests/campaign_stage3_advance.rs web/src/components/chat-workspace/cockpit/CockpitInbox.test.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx
  git commit -m "test: verify C1 enrollment recovery surface"
  ```

---

## 自审

- **OpenSpec requirement 覆盖：**
  - `REQ-WIGA-01`/`03`/`04`：Task 1–3 冻结双载体/version/CAS，Task 6 lease 三态，Task 9 StartCoding/通知，Task 10 兼容回归；人工 design、choice、plan gate、Final Confirm 均保留。
  - `REQ-C1-TARGET-01`：Task 1 target union；Task 2 REST/DTO；Task 3 prepare/advance/start/reconcile；Task 6 lease；Task 8 child；Task 9 notification identity。
  - `REQ-C1-CHILD-01`：Task 8 精确匹配、跨代新建、同代 replay、旧 child 只读；Task 10 A13。
  - `REQ-C1-GATE-01`/`02`：Task 4 先持久化 snapshot、恢复/重建/无 snapshot 禁 approve、command/version；Task 9 Cockpit/inbox；Task 10 A07。
  - `REQ-C1-PLAN-01`：Task 5 existing/create/provider/depends/scope、未声明与不能执行分开停等、finding 不清；Task 9/10 操作通知。
  - `REQ-ADV-C1-RETRY`：Task 7 独立 retry record、原 Failed 保留、同 attempt、安全 checkpoint、未知副作用确认、普通 advance 不隐式 retry；Task 9/10 A09。
  - `tasks.md` §1.1/1.2、§2.1/2.2、§3.1/3.2、§4.1/4.2：依次由 Task 1–3、4–5、6–7/9、10 落地；所有阶段/任务均明确 A07/A09/A12/A13 映射。
- **A07/A09/A12/A13 逐任务映射：** Task 1–10 的每条“验收映射”均显式列出 A07、A09、A12、A13，并在后文注明主覆盖与依赖贡献；每个 Phase 标题同样列出四类验收，跨任务依赖不隐藏。
- **类型/签名一致性：** Task 1 首次定义 `EnrollmentTarget`/binding version/rebind；Task 2/3/4/5/6/7/8/9 只消费；Task 7 retry record 使用既有 `AdvanceInitializationPhase`；Task 8 child identity 使用同一 binding identity；Task 6 `LeaseTakeoverRequest`/`LeaseTakeoverResult`、Task 7 `RetryInitializationResult` 与 Task 9/10 均复用统一 `OperationState`，无第二 generation/operation/notification 状态机。`EnrollmentBindingIdentityInput` 在 Task 1 作为 `EnrollmentBindingIdentity` 的输入投影一并定义，避免未定义类型。
- **Review Focus 测试归属：** Focus 1→Task 1/2/3/9；Focus 2→Task 4/9；Focus 3→Task 6/9；Focus 4→Task 7/9；Focus 5→Task 5/8/9；每项都包含真实 durable 行为断言和定向红/绿命令。
- **Non-Goals 与零回归：** Global Constraints 明确不建设 durable generation/owner/fence/orchestrator/notification DB，不自动 takeover/批准/反馈/放弃/重绑/未知副作用，不实现 C2/C4/C5；Task 3/4/9/10 明确非 enrolled、Manual、人工 gate、Final Confirm 零回归。
- **任务数量与粒度：** 共 **10** 个任务，分四 Phase；每个任务均有精确 Files、Interfaces、真实失败测试、红灯命令、最小实现、绿灯命令、独立 `git add`/`git commit`，可按依赖顺序逐任务审查。
- **勘察边界：** 计划中的新增符号、字段、route 和测试名是待实现契约，不冒称已存在；现有已落地符号来自读取的源码。未运行 cargo/pnpm/build/lint/formatter，未修改实现源码；项目级验证由主控在所有实现落地后统一执行。

**状态：** DONE_WITH_CONCERNS（计划已完成；实施前需由执行者按 Task 1 冻结 target/binding wire 具体 JSON，并在发现现有 lock/advance 事实无法证明死亡时保持 UnknownNeedsHuman，不得自行放宽）。
