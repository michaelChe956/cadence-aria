# WIG Autopilot P2 后台 Coding 链 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 `work-item-group-autopilot` 的 tasks.md §3.1–3.4：经人工确认的唯一单 target plan 在无页面时独立 advance 到 Ready、共用 typed StartCoding 单发首启并恢复，编码执行结束时通知人做 Final Confirm；先解除 GAP-E/G/F/H 的真实链阻断，最后分别关替身与真实链。

**Architecture:** 沿用 P1 的 issue enrollment、唯一 plan/session、非 superseding manager 和有界 durable reconcile；`AdvancePlan` 只是 `WorkspaceEngine::handle_advance` 的共用薄入口，不启动 provider。attempt 冻结 `CodingStartRunPolicy`，人工 WS 和编排器共用一个 typed StartCoding 服务：同一 attempt 临界区内重读、状态/身份/授权/Ready 核验、不可复位的 durable claim 与内存 registry reservation、持久 barrier 后才放行**既有** runner；启动扫描根据既有检查点复用恢复协议，副作用不确定时人工分诊。完成信息从 readiness、等待态和 FinalConfirm 节点派生，不建通知表、不触碰人工 Final Confirm。

**Tech Stack:** Rust 2024、Axum、Tokio、serde、既有文件锁和原子 JSON；React、TypeScript、Vitest、Testing Library。无新增外部依赖。

**Spec:** `openspec/changes/work-item-group-autopilot/{proposal.md,design.md,tasks.md,specs/work-item-group-autopilot/spec.md,specs/work-item-plan-advance/spec.md,specs/multi-target-group-coding/spec.md,specs/work-item-plan-conversational-gate/spec.md}`；本计划**只**展开 tasks.md §3.1–3.4。另以 `cadence/plans/2026-09-26_迭代计划_WIG自动化与引导_v1.0.md` §6.1 和 `cadence/notes/2026-09-27_遗留清单_P1真实链产品缺口_v1.0.md` 的 GAP-E/G/F/H 为 Task 0 前置；P0/P1 与 amendment 解耦已完成，不重做、不改其授权/投递契约。

## Global Constraints

- REQ-WIGA-02/03/04、REQ-ADV-05、REQ-MTG-03：默认 off、必须**持久显式 opt-in**；plan session 永远 `RunPolicy::Interactive`；只对恰一 logical repository、唯一 attempt 生效，零/多 target、冻结 target 与 enrollment 不符时**整体拒绝**，不得循环逐个自动启动。旧 attempt 缺新增字段反序列化为 Manual；非 enrolled 与既有手工逐 target 保持原语义。
- D4 顺序不可颠倒：attempt reload → 首启状态矩阵/origin → enrolled 当前许可和精确 source/plan/revision/target → `advance_is_ready_for_attempt`（SC 未 Ready 固定 `SC_CODING_REQUIRES_ADVANCE`）→ 同 attempt durable 单发 claim + `CodingRunRegistry::try_reserve_attempt` → journal/barrier → 原 runner。手工显式 `RestartCoding`/`RecoverCoding` 不属于首启，不得变成后台隐式 restart；不可证明外部副作用的窗口进入可见人工分诊，**不声称 provider exactly-once**。
- D2 撤销：禁用在线性化点前抢赢则**未消费** AutoStartOnce 不再生效；claim 抢赢后禁用/重开不 Abort 已运行 run、不重置 claim。进程内 registry 不能充当跨进程持久 claim。订阅/observer 只看事实，不是 run 所有者。
- REQ-WIGA-05/06：P0 workspace/coding choice REST 与 HTTP/WS 共用真实回执原样复用；已完成 amendment Unsent/Pending/Delivered、无 socket 激活与 attach 补投递原样复用。慢 observer/零 socket 不可阻断业务，也不得把队列接收当真实 socket 写 ack；唯人工动作解除 choice/plan 门/compile recovery/Final Confirm。
- R5/REQ-WIGA-07：`WaitingForHuman ∧ FinalConfirm ∧ GroupFinalReadinessStatus::Complete` 且快照完整才有「编码执行完成，待最终确认」；`Completed` 是**人工确认后的另一事实**，不把等待态叫「整组已交付」（该用语仅适用现有 `PlanGroupOverall::AllDelivered`）；info 不进 countedInbox/批量操作。稳定时间取 FinalConfirm 节点首次 `started_at`，不是 `completed_at` 或每次重写的 readiness `created_at`。
- GAP 前置只纳入 E/G、F、H：reviewer 失败停等、由人明确重驱；503 分类诊断**必须驾驶舱可见**，复用 E/G 显式恢复入口，**不自动重试**（外部副作用/账号池与重复计费未知）。GAP-A/B/C/D/LC 基建另立项，不能为本计划改变业务边界；Fake 测试运行与真实 gateway 准入分开，不能假改 reviewer 为 Claude。
- 明确非目标：tasks.md §4 P3 的 K=8 窗口外近期补读、TTL、首次 hydration 跨刷新提醒、整链 UI 文案审计；不修改 OpenSpec/P0/P1 勾选。真实链在运行中的 `aria-dev-v48q` **部署同版本实现后**另行实测；计划中的命令/测试均为未来执行步骤，不是已运行证据。worktree 根执行 Rust 定向 `cargo test --locked --lib <filter> -- --nocapture`、前端 `pnpm -C web exec vitest run <file>`；集成负责人最终统一全量验证，不加 `-j 1`。

## Review Focus

1. **enrollment 关闭→重开或 source/plan/revision/target 漂移撞上首启：**关闭先胜不得消费已物化许可；claim 先胜不中止也不再次首启；漂移无 provider 启动（Task 2/3/4/5 的身份/竞态测试）。
2. **未 Ready、manual 与 auto 并发及多 target 的 sibling：**共用错误码、只一个 runner，批量**整体**拒绝且不抢任一 sibling，手工逐 target 可用（Task 1/3/4/5）。
3. **advance journal、claim→registry→barrier 各中窗、进程重启和 Failed/Aborted：**只恢复可信原 run，副作用不明人工分诊，不创建第二个 attempt/provider 首启或隐式 Restart（Task 1/4/5/6）。
4. **零 socket choice/amendment/resume 和满队列/慢 observer：**业务到达 durable 终点，回执只有真 waiter/真 socket 写成功才 Delivered，重开补帧不重复应用（Task 6/7）。
5. **readiness 被重复写、FinalConfirm 等待、人工 Completed：**稳定 key/时间、仅等待态一次「待最终确认」、不计数；Completed 改文案不假报整组交付，503 另有驾驶舱人工恢复卡（Task 0.1/0.3/8/9）。

---

## 文件责任、既有接口、统一拟新增接口

**已核验的责任分工：**`src/web/autopilot_orchestrator.rs::reconcile` 目前只处理 `ensure_enrolled_plan`→`human_stop_point`→`start_plan_generation_once`，Confirmed 因生成准入停在 NeedsHuman；`reconcile_all_once` 扫项目/issue（2 秒、最多 32 issue）。`src/product/workspace_engine/advance.rs::handle_advance` 保有 plan/revision/compile/child/journal 的**唯一**业务实现；`src/web/workspace_ws_handler/decisions/advance.rs::handle_advance_from_handler` 只是人工入口。`src/web/coding_ws_handler/socket.rs` 手工 StartCoding 自己检查 Ready 后裸调 `spawn_coding_runner`，而 `socket/preparation.rs::prepare_coding_message` 只保留 mutation lease、先释放 attempt guard，抽 service 时必须消除锁顺序死锁。`src/web/state/coding_run_registry.rs` 的 reservation 是进程内的；`runner.rs::spawn_coding_runner_reserved` + `runner/task.rs` 的 `start_rx` 提供**已存在**的注册→journal→放行屏障模板，不把 failed-review 专用 journal 当首启 journal。`socket/resumption.rs::ensure_runner_for_resumed_attempt` 现只在 attach 调用；`app.rs::serve_web` 构造 WebAppState 和扫描任务，适合挂无订阅启动补偿。`src/product/coding_attempt_store/group_final_readiness.rs::write_group_final_readiness_snapshot` 每写一次便刷新 `created_at`；`src/product/coding_workspace_engine/timeline.rs::create_pending_final_confirm_timeline_node` 复用旧 pending 节点；`handoffs.rs::handle_final_confirm` 才写 Completed。`src/web/plan_confirmed_info.rs` + `types.rs::IssueLifecycleResponse` + `useWorkspaceSessionObservers.ts` + `workspace-cockpit-projection.ts` 是现有 info/不计数链。以下路径与符号已按源码核对；新文件均在拥有任务中创建。

**既有签名，仅消费、不改名/改语义：**

```rust
// src/product/advance_store.rs / src/product/workspace_engine/advance.rs
pub struct AdvanceInput { pub command_id: String, pub project_id: String,
    pub issue_id: String, pub plan_id: String }
pub enum AdvanceOutcome {
    Completed { record: AdvanceRecord, attempt_id: String,
        workspace_entry: String, target_attempts: Vec<AdvanceTargetAttemptBinding> },
    Replayed { record: AdvanceRecord },
    Rejected { record: Option<AdvanceRecord>, code: String, reason: String },
}
WorkspaceEngine::handle_advance(&mut self, input: AdvanceInput)
    -> impl Future<Output = Result<AdvanceOutcome, String>>;
AdvanceStore::advance_is_ready_for_attempt(&self, project_id: &str, issue_id: &str,
    plan_id: &str, attempt_id: &str) -> Result<bool, ProductStoreError>;
// src/product/models/automation.rs / src/product/issue_automation_store.rs
IssueAutomationStore::get(&self, project_id: &str, issue_id: &str)
    -> Result<Option<IssueAutomationEnrollment>, ProductStoreError>;
// src/web/state/coding_run_registry.rs
CodingRunRegistry::lock_attempt(&self, key: &CodingAttemptRunKey) -> OwnedMutexGuard<()>;
CodingRunRegistry::try_reserve_attempt(&self, key: &CodingAttemptRunKey)
    -> Option<CodingRunReservation>;
CodingRunReservation::activate_cancellable(self, command_tx: mpsc::Sender<CodingRunnerCommand>)
    -> Option<CodingRunRegistration>;
// src/product/coding_attempt_store/attempt.rs / src/web/state/coding_socket_registry.rs
CodingAttemptStore::list_attempts_for_issue(&self, project_id: &str, issue_id: &str)
    -> Result<Vec<CodingExecutionAttempt>, ProductStoreError>;
CodingSocketRegistry::hub_sender(&self, key: &CodingAttemptRunKey)
    -> mpsc::Sender<CodingWsOutMessage>;
// src/web/coding_ws_handler/socket/resumption.rs
ensure_runner_for_resumed_attempt(state: &WebAppState, store: &CodingAttemptStore,
    event_tx: &mpsc::Sender<CodingWsOutMessage>, key: &CodingAttemptRunKey,
    attempt: &CodingExecutionAttempt) -> impl Future<Output = ResumedAttemptRunner>;
// src/product/coding_attempt_store/group_final_readiness.rs
CodingAttemptStore::get_group_final_readiness_snapshot(&self, attempt: &CodingExecutionAttempt)
    -> Result<Option<GroupFinalReadinessSnapshot>, ProductStoreError>;
```

注：`AdvanceOutcome` 各字段按 `src/product/advance_store.rs:107-124` 实际定义抄录；`handle_advance` 的 `impl Future` 只是 async 函数的阅读表示，实施直接调用原函数。现有 `POST /api/workspace-sessions/{session_id}/human-actions` 要求 `expected_gate_id` 必等 active gate，**无门 reviewer 失败不能伪造 gate_id**；P0 coding choice `POST/GET /api/projects/{p}/issues/{i}/coding-attempts/{a}/choices/...` 不增重复路由。`CodingExecutionAttempt` 在 `src/product/coding_models/execution.rs` 有手写 Deserialize，必须同步改 Serde 中转结构、构造与显式 fixture。

**本计划唯一拟新增契约（符号由标注 Task 首次定义；下游只消费）：**

```rust
// Task 0.1: 独立于现有 human gate 的人工 SC 失败重驱，不覆盖 human-actions 门校验。
// POST /api/workspace-sessions/{session_id}/failed-sc-runs/{failed_node_id}/retry
pub struct RetryFailedScRunRequest { pub command_id: String,
    pub expected_phase: SingleCandidatePhase }
pub struct RetryFailedScRunStatus { pub command_id: String, pub failed_node_id: String,
    pub state: String } // "accepted" | "replayed" | "needs_human"，不借 human gate 类型。
// 只接受 Failed 的 WorkItemPlan/SingleCandidate/Interactive、最新失败节点、
// 无活 manager run；从现有 start_generation/manager 非 superseding 入口重驱同 session。

// Task 0.2: 单一静态 gateway reviewer 能力谓词；GET 投影与 Enable 共享。
pub(crate) fn validate_gateway_reviewer_for_enrollment(
    reviewer: &ProviderName, gateway_required: bool, test_provider_enabled: bool,
) -> ApiResult<()>;

// Task 1: 共用 advance 薄服务：不改已有 AdvanceInput/handle_advance 签名。
pub enum AdvancePlanOrigin { Manual, Enrolled { enrollment_id: String, policy_revision: u64 } }
pub async fn advance_plan(state: &WebAppState, input: AdvanceInput,
    origin: AdvancePlanOrigin) -> Result<AdvanceOutcome, String>;

// Task 2: src/product/coding_models/execution.rs，Serde 默认 Manual。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum CodingStartRunPolicy {
    #[default] Manual,
    AutoStartOnce { enrollment_id: String, policy_revision: u64,
        source_plan_revision: String },
}
// 添加到 CodingExecutionAttempt、CodingExecutionAttemptSerde：
// #[serde(default)] pub start_run_policy: CodingStartRunPolicy

// Task 3: src/product/coding_models/execution.rs 先定义 origin 身份，避免 product
// 模型反向引用 web::coding_start；web service 只消费同一类型并提供命令/结果。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CodingStartOrigin { Manual, Enrolled { enrollment_id: String,
    policy_revision: u64 } }

// Task 3: src/web/coding_start.rs。
pub struct StartCodingCommand { pub attempt_id: String, pub command_id: String,
    pub origin: CodingStartOrigin }
pub enum StartCodingOutcome { Started { attempt_id: String },
    AlreadyStarted { attempt_id: String }, NeedsHuman { attempt_id: String, reason: String } }
pub async fn start_coding_once(state: &WebAppState, project_id: &str,
    issue_id: &str, command: StartCodingCommand) -> Result<StartCodingOutcome, StartCodingError>;
// StartCodingError 含稳定 code()/message()，Ready 不足固定
// "SC_CODING_REQUIRES_ADVANCE"；身份/策略不符 -> 显式无启动错误。

// Task 4: attempt 同文件单发事实 + 不可复位的 checkpoint；不以额外 JSON 单据
// 与 attempt 状态相互猜测。外部副作用边界含“不确定人工分诊”终态。
pub struct CodingStartClaim { pub command_id: String, pub origin: CodingStartOrigin,
    pub phase: CodingStartPhase, pub claimed_at: String }
pub enum CodingStartPhase { Claimed, RunnerRegistered, ProviderMayHaveStarted, NeedsHuman }
// #[serde(default)] pub start_claim: Option<CodingStartClaim> on CodingExecutionAttempt/Serde。
CodingAttemptStore::claim_coding_start(&self, attempt: &CodingExecutionAttempt,
    command_id: &str, origin: &CodingStartOrigin,
) -> Result<ClaimCodingStartOutcome, ProductStoreError>;
CodingAttemptStore::advance_coding_start_phase(&self, attempt: &CodingExecutionAttempt,
    command_id: &str, phase: CodingStartPhase)
    -> Result<CodingExecutionAttempt, ProductStoreError>;
pub enum ClaimCodingStartOutcome { Claimed(CodingExecutionAttempt),
    Existing(CodingExecutionAttempt) }
// 每次 attempt 原文件锁中重读/CAS，不接受覆写已认领 command；enrolled 的
// enrollment 精确复核与 disable 竞争必须在同一个 enrollment 文件锁线性化，
// 固定顺序 enrollment lock -> attempt lock；人工同样使用 claim 同一把 attempt 锁。

// Task 8: 只读 info，由当前 attempt/readiness/FinalConfirm node 组成。
pub struct CodingFinalConfirmInfoDto { pub key: String, pub project_id: String,
    pub issue_id: String, pub plan_id: String, pub attempt_id: String,
    pub occurred_at: String, pub title: String, pub final_confirmed: bool }
pub fn issue_coding_final_confirm_info(paths: &ProductAppPaths,
    project_id: &str, issue_id: &str)
    -> Result<Vec<CodingFinalConfirmInfoDto>, ProductStoreError>;
// IssueLifecycleResponse 新增 #[serde(default)] coding_final_confirm_info:
// Vec<CodingFinalConfirmInfoDto>；Task 9 TS 定义同 snake_case DTO。
```

注：`AdvanceOutcome` 的注释代表已有多分支/多字段，不在计划中重新定义该类型；`handle_advance` 的 `impl Future` 是 async 函数的阅读表示，实施直接调用原函数。现有 `POST /api/workspace-sessions/{session_id}/human-actions` 要求 `expected_gate_id` 必等 active gate，**无门 reviewer 失败不能伪造 gate_id**；P0 coding choice `POST/GET /api/projects/{p}/issues/{i}/coding-attempts/{a}/choices/...` 不增重复路由。`CodingExecutionAttempt` 在 `src/product/coding_models/execution.rs` 有手写 Deserialize，必须同时改 Serde 中转结构、构造与显式 fixture。
`WorkspaceInboundContext` 已在 `src/web/workspace_ws_handler/decisions/inbound.rs` 持有 `app_state: WebAppState`，socket 构造点与测试 fixture 均已填充；Task 1 将 `handle_advance_from_handler` 增加 `app_state: WebAppState` 参数，并在 `inbound.rs` 的 Advance 分支传入 destructure 后的 `app_state.clone()`，不从 `ProviderRunContext` 反向重建 state。`AdvanceOutcome::Completed` 的稳定 attempt 读取直接 pattern-match `attempt_id`/`target_attempts`，不假定不存在的 `attempt_id()` 方法。


**取舍：**推荐沿用 advance 引擎/journal、一个薄 web 调用面；另造 advance 实现会分裂 plan/revision/多 target 判据。推荐**attempt 同文件**携单发 claim 而非只用内存 registry；增加少量字段/所有构造 fixture 适配，换取跨进程线性化。runner start barrier 参照原 `spawn_coding_runner_reserved` 的 oneshot，在首启专用函数复用同一 task 结构，**不复用 failed-review recovery journal**。人工失败重驱只提供显式、带失败节点身份的受理入口，重走现有 author generation/评审链：代价是人确认后可能重新生成 author 内容，但不会凭 `Evaluate` 偷重试 reviewer；尝试直接跳到 reviewer 可少一次 author 成本，却要另造预算/评审 scope/ledger 状态机，非本契约。若实施发现需改变 Interactive 门/评审计费或静态 gateway 白名单，先停并更新 OpenSpec，不自行扩范围。

## Task 0.1：前置 GAP-E/G——失败相位收敛与显式人工 SC 恢复

**Files:** Modify `src/product/workspace_engine/{session_state/timeline.rs,review/drive.rs}`、`src/product/lifecycle_store/workspace_single_candidate.rs`（失败节点与 command id 锁内单发认领）、`src/web/{handlers/mod.rs,app.rs,types.rs}`；Create `src/web/handlers/workspace_sc_recovery.rs`；Modify `web/src/{api/client.ts,api/types/workspace.ts,pages/ChatCockpitPage.tsx,components/chat-workspace/cockpit/CockpitInbox.tsx}`（复用既有驾驶舱信息/动作区，不改人工 gate）；Test `src/product/workspace_engine/tests/single_candidate.rs`、新 handler 内 `#[cfg(test)]`、`web/src/pages/ChatCockpitPage.inbox.test.tsx`。

**Interfaces:** Consumes `WorkspaceEngine::start_generation(ProviderConfigSnapshot,bool)`、`LifecycleStore::rearm_failed_single_candidate_for_start_generation`、`WorkspaceSessionManager::create`、`spawn_provider_run_claiming_idle`；produces 上述 `RetryFailedScRunRequest/Status` 和 failed reviewer 节点身份/诊断。仅 latest Failed、无活 run、SingleCandidate+Interactive+当前相位 Failed 受理；现有断连 `retry_interrupted_run`/人工门 `human-actions` 零改动。同 session 文件锁中冻结 `(failed_node_id,command_id)`，异键 409、同键读原结果；manager `ClaimIfIdle` 与 durable ledger 双防二次 provider start，认领已写但派发未知时停人工分诊。

- [ ] **Step 1: 写失败测试。** 引擎测试复用该文件已有 `make_work_item_plan_engine_with_accepted_contract_drafts`、`single_candidate_record`：

  ```rust
  #[tokio::test]
  async fn sc_evaluate_reviewer_failure_is_durably_failed_once() {
      let (_tmp, lifecycle, _plan, mut engine) =
          make_work_item_plan_engine_with_accepted_contract_drafts();
      single_candidate_record(&lifecycle, &mut engine,
          SingleCandidatePhase::Evaluate, RunPolicy::Interactive);
      engine.start_review().await;
      let failed_node_id = engine.active_node_id.clone().unwrap();
      let (_tx, rx) = tokio::sync::mpsc::channel(1);
      engine.drive_reviewer_provider_session(
          Err(crate::cross_cutting::provider_adapter::ProviderAdapterError::provider_unavailable(
              "503 No available accounts")), rx, ProviderName::ClaudeCode).await;
      let durable = lifecycle.get_workspace_session(&engine.session().session_id).unwrap();
      assert_eq!(durable.single_candidate_phase, Some(SingleCandidatePhase::Failed));
      assert_eq!(durable.status, WorkspaceSessionStatus::Failed);
      assert!(lifecycle.load_timeline_nodes_for_issue_session(
          &durable.project_id, &durable.issue_id, &durable.id).unwrap().iter().any(|n|
          n.node_id == failed_node_id && n.status == TimelineNodeStatus::Failed));
  }
  ```

  同任务 HTTP 测试以 `EnrolledGateFixture::new` 创建真实 enrollment/session，在 manager engine 复用 `start_review`→上述 reviewer 失败构造现场；向新增 endpoint 发送 `{command_id,expected_phase:"failed"}`，测错误节点 409、无活 run 前置、正确节点 200，再重复同键不增加 `provider_start_ledger`；前端用 failed node + phase=failed 的当前观察 state 测「人工重新驱动」按钮调用新 REST，而非 auto retry。

- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib sc_evaluate_reviewer_failure_is_durably_failed_once -- --nocapture`；新增 HTTP 测试 `cargo test --locked --lib retry_failed_sc_run_requires_latest_failed_node -- --nocapture`；`pnpm -C web exec vitest run src/pages/ChatCockpitPage.inbox.test.tsx`。预期第一条旧 `Evaluate` 不会变 Failed，其他缺路由/恢复卡。
- [ ] **Step 3: 最小实现。** `finish_review_provider_run_failure` 在 Start/Provider/EmptyOutput 的 ReviewerRun 已 durable Failed 后推进 `persist_single_candidate_terminal_phase(Failed)`；`finish_failed_run` 当前会重置 Open，需在 SC Failed 分支保留 `WorkspaceSessionStatus::Failed`，其他 flow 保持旧 Open。REST handler 先查 session/status/phase/最新失败节点/manager 空闲，再在 session 文件锁内认领 `(failed_node_id,command_id)`，调用现有 `start_generation` 与 `spawn_provider_run_claiming_idle`（不使用 superseding 入口），返回受理/原同键结果；如果派发外部副作用是否发生不明，保留认领并给人工分诊。刷新真实 manager 上下文，失败可见而非日志；不得让同键重复从 Failed 再重臂。相位分支示例：

  ```rust
  if self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
      && self.session.single_candidate_phase == Some(SingleCandidatePhase::Evaluate)
  {
      self.persist_single_candidate_terminal_phase(SingleCandidatePhase::Failed);
  }
  self.finish_failed_run().await; // SC Failed 时保留 durable Failed，其他 flow 仍 Open。
  ```

  该判断只用于**已经标 Failed 的当前 reviewer 节点**，不放到所有 `finish_failed_run` 共用分支；人工入口必须确认 `failed_node_id` 最新且与 durable timeline 一致，生成动作由人显式发起，`human-actions` 的 gate_id 守卫绝不能放松。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2 三条并跑 `cargo test --locked --lib single_candidate_recovery -- --nocapture`；同键重复和未授权/非 Failed 均不增加 provider ledger，测试读取 durable session/节点而非仅 mock 回调。
- [ ] **Step 5: 提交。** `git add src/product/workspace_engine src/web/handlers src/web/app.rs src/web/types.rs web/src/api web/src/pages/ChatCockpitPage.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx web/src/components/chat-workspace/cockpit/CockpitInbox.tsx && git commit -m "fix: expose explicit failed single-candidate recovery"`。

## Task 0.2：前置 GAP-F——enrollment 静态 gateway reviewer 组合预检

**Files:** Modify `src/web/handlers/{automation_target.rs,automation_enrollment.rs,mod.rs}`；Create `src/web/handlers/automation_gateway_preflight.rs`；Test 两 handler 相邻 `#[cfg(test)]`（复用 `automation_enrollment_test_support::seed_fixture/enrollment_body/put_enrollment`）；`src/product/logical_codebase/provider_gateway.rs` 仅当需共享静态阻断谓词时修改，不改原网关行为。

**Interfaces:** Produces 统一 `validate_gateway_reviewer_for_enrollment(&ProviderName,bool,bool) -> ApiResult<()>`；GET `automation-target` 和最终 PUT Enable 共用，均先按现有 routing 确认 logical target、再检查 reviewer gateway 静态映射；Disable 不重验。只判**确定性**静态不支持（Pi、KimiCode、真实 Fake；Codex 当前固定 sandbox 禁令 `CODEX_DANGER_FULL_ACCESS_UNSUPPORTED`），动态账号/版本/网络波动不误判为白名单；测试运行 `test_provider_enabled` 下 Fake 保持 fixture 可用，不假装 Fake 能真实 gateway。

- [ ] **Step 1: 写失败测试。** 在 `automation_target.rs` 现有测试模块加：

  ```rust
  #[tokio::test]
  async fn automation_target_rejects_gateway_unsupported_reviewer_before_enable() {
      let fixture = seed_fixture(1, true);
      let root = fixture._root.path().to_path_buf();
      // provider_workspace_config 的旧可用性门必须先通过，红灯只来自 gateway 缺口。
      let state = WebAppState::with_provider_availability(
          root.clone(), WebRuntime::new_fake(root), |_| true);
      let app = build_web_router(state);
      let response = get_automation_target(&app,
          "?author_provider=pi&reviewer_provider=kimi_code").await;
      assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
      assert_eq!(response_json(response).await["code"],
          "automation_gateway_reviewer_unsupported");
      assert!(!enrollment_file_exists(&fixture));
      let mut enable = enrollment_body(&fixture, 1, 1);
      enable["command"]["options"]["reviewer_provider"] = serde_json::json!("kimi_code");
      let response = put_enrollment(&app, enable).await;
      assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
      assert_eq!(response_json(response).await["code"],
          "automation_gateway_reviewer_unsupported");
      assert!(!enrollment_file_exists(&fixture));
  ```

  同样测试 Codex 配置在当前固定 sandbox 下拒绝，Fake 在 fake runtime fixture 可通过；分别证明投影与最终 Enable 同源、不写错 enrollment。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib automation_target_rejects_gateway_unsupported_reviewer_before_enable -- --nocapture`，预期旧 GET/PUT 允许 KimiCode。
- [ ] **Step 3: 最小实现。** `automation_gateway_preflight.rs` 调用现有 `ProviderRef::from_provider_name`；Codex 额外按 `CODEX_DANGER_FULL_ACCESS_SANDBOX_MODE == CODEX_DEFAULT_SANDBOX_MODE` 做与 `LogicalCodebaseProviderGateway` 私有方法 `enforce_route_policy`（provider_gateway.rs:773-791）同源的静态拒绝。GET 与 PUT 各先按原逻辑核唯一 logical target 再调用同 helper，统一映射稳定错误码 `automation_gateway_reviewer_unsupported`；Disable 不经过 reviewer 检查，不实例化真实 provider、不探活账号、不改用户选项：

  ```rust
  if gateway_required && !(test_provider_enabled && *reviewer == ProviderName::Fake) {
      ProviderRef::from_provider_name(reviewer, "static-preflight")
          .map_err(|error| invalid_gateway_reviewer(error.to_string()))?;
  }
  ```

  复用原 gateway 常量与既有 `provider_gateway_tests.rs::codex_danger_full_access_is_blocked_at_gateway_route_even_outside_ui` 对照；若拆共享判据，只抽出静态谓词，原 gateway launch 检查保持一致。`test_provider_enabled` 只允许 Fake 测试模式，不允许 Pi/KimiCode 绕行。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2 与 `cargo test --locked --lib automation_enrollment -- --nocapture`；Fake fixture 仍建立原同键 enrollment，动态账号 503 不在 GET 静态预检伪报。
- [ ] **Step 5: 提交。** `git add src/web/handlers src/product/logical_codebase/provider_gateway.rs && git commit -m "fix: reject unsupported gateway reviewers at enrollment"`（若复用原 gateway 静态判据而无须改该文件，只提交实际改动）。

## Task 0.3：前置 GAP-H——503 可见诊断，仍由人决定重驱

**Files:** Modify `src/product/workspace_engine/review/drive.rs`、`web/src/{state/workspace-cockpit-projection.ts,pages/ChatCockpitPage.inbox.test.tsx}`；Test `src/product/workspace_engine/tests/single_candidate.rs` 和前端 `ChatCockpitPage.inbox.test.tsx`。错误已由 `drive_reviewer_provider_session_once` 传 `ProviderAdapterError` 至 `finish_review_provider_run_failure`，无需改 web provider runner 的错误传递。

**Interfaces:** Consumes Task 0.1 Failed 节点/显式重驱卡；仅在 `ProviderAdapterError.code == ProviderErrorCode::ProviderUnavailable` 且其有界 `details`/`stderr` 同时含 `503` 与 `No available accounts` 时投影 `provider_gateway_503_no_accounts`，其他错误用通用失败类；落 durable 的仅是固定脱敏摘要，不保存原始 stderr/Authorization。不加重试定时器、不在编排器认领新 provider run。

- [ ] **Step 1: 写失败测试。** 加在 Task 0.1 的 reviewer failure fixture 后：

  ```rust
  #[tokio::test]
  async fn reviewer_gateway_503_has_durable_human_diagnostic_without_retry() {
      let (_tmp, lifecycle, _plan, mut engine) =
          make_work_item_plan_engine_with_accepted_contract_drafts();
      single_candidate_record(&lifecycle, &mut engine,
          SingleCandidatePhase::Evaluate, RunPolicy::Interactive);
      engine.start_review().await;
      let failed_node_id = engine.active_timeline_node_id().unwrap();
      let before = lifecycle.get_workspace_session(&engine.session().session_id)
          .unwrap().provider_start_ledger.len();
      let (_tx, rx) = tokio::sync::mpsc::channel(1);
      engine.drive_reviewer_provider_session(
          Err(crate::cross_cutting::provider_adapter::ProviderAdapterError::provider_unavailable(
              "503 No available accounts; Authorization: Bearer secret")),
          rx, ProviderName::ClaudeCode).await;
      let nodes = lifecycle.load_timeline_nodes_for_issue_session(
          &engine.session().project_id, &engine.session().issue_id,
          &engine.session().session_id).unwrap();
      let summary = nodes.iter().find(|n| n.node_id == failed_node_id)
          .unwrap().summary.as_deref().unwrap();
      assert!(summary.contains("provider_gateway_503_no_accounts"));
      assert!(!summary.contains("secret"));
      assert_eq!(lifecycle.get_workspace_session(&engine.session().session_id)
          .unwrap().provider_start_ledger.len(), before);
  }
  ```

  前端将上述 summary 的 Failed 节点交给 cockpit，测同时显示分类和 Task 0.1 的人工重驱按钮、无自动 invoke。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib reviewer_gateway_503_has_durable_human_diagnostic_without_retry -- --nocapture`；`pnpm -C web exec vitest run src/pages/ChatCockpitPage.inbox.test.tsx`，预期当前节点仅通用「Provider 运行失败」。
- [ ] **Step 3: 最小实现。** reviewer Start/Provider 失败保留 `ProviderAdapterError.code` 至节点摘要分类：仅 `ProviderUnavailable` 且有界 details/stderr 同时表明 `503` 与 `No available accounts` 时落固定安全类，普通动态网络不可用仍写通用失败；通过 `update_timeline_node` 保留原失败节点身份，不写裸 stderr 或凭据。引擎已有字符串 `ReviewProviderRunFailure` 的构造处需携带原 `ProviderErrorCode` 或安全分类枚举，不能从拼接后的错误文案猜 code。示意：

  ```rust
  let diagnostic = if code == ProviderErrorCode::ProviderUnavailable
      && message.contains("503") && message.contains("No available accounts") {
      "provider_gateway_503_no_accounts: 推理网关账号池不可用；检查服务后手动重驱"
  } else {
      "provider_reviewer_failed: 评审运行失败；核对 provider 后手动重驱"
  };
  self.update_timeline_node(&node_id, TimelineNodeStatus::Failed,
      Some(diagnostic.to_string())).await;
  ```

  让错误分类同时覆盖 reviewer start/runtime 两支，不把断连中止/人工取消混成 503；故障不是自动重试信号。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2 与 Task 0.1 Failed/恢复测试；确认只因人工调用 Task 0.1 endpoint ledger 才增长。
- [ ] **Step 5: 提交。** `git add src/product/workspace_engine/review/drive.rs web/src/state/workspace-cockpit-projection.ts web/src/pages/ChatCockpitPage.inbox.test.tsx && git commit -m "fix: surface gateway 503 in failed reviewer recovery"`。

## Task 1：3.1 单一 `AdvancePlan` 共用入口、稳定 replay 与自动多 target 阻断

**Files:** Create `src/web/advance_plan.rs`；Modify `src/web/{mod.rs,wiga_gate_fixture.rs,workspace_ws_handler/decisions/advance.rs,workspace_ws_handler/decisions/inbound.rs}`；Test `src/web/workspace_ws_handler/tests/campaign_stage3_advance.rs`、新 `advance_plan.rs` 的 `#[cfg(test)]`。引擎 `src/product/workspace_engine/advance.rs` 既有 journal 和 per-target 手工逻辑**不重新实现**。

**Interfaces:** Produces `advance_plan(state: &WebAppState, input: AdvanceInput, origin: AdvancePlanOrigin) -> Result<AdvanceOutcome,String>`（`input` 按值）；WS Manual 与 Task 5 自动消费。自动仅在 enabled enrollment 精确 session/plan/source、durable `Confirmed + Completed SC + 成功 publication/compile` 时发固定 `command_id`（同 enrollment+plan，字符符合 `validate_relative_id`）；Ready 不在此调 StartCoding；自动初次请求前检查权威绑定恰一 target、回放后还核 `AdvanceRecord.target_attempts` 不超过一条，不能因为 engine 支持 split 就放行。任何 `AdvanceOutcome::Replayed` Failed/Aborted 都返回人工分诊，不隐式新 command。

- [ ] **Step 1: 写失败测试。** 扩展 `confirmed_campaign_harness()` 的真实 Confirm→advance fixture，以 `harness.send(WsInMessage::Advance { ... })` 的既有测试作手工对照；新模块测试：

  ```rust
  #[tokio::test]
  async fn enrolled_advance_replays_one_ready_attempt_without_starting_runner() {
      let fixture = confirmed_enrolled_fixture().await;
      let enrollment = fixture.enrollment();
      let input = AdvanceInput {
          command_id: format!("wiga-advance-{}-{}", enrollment.enrollment_id,
              enrollment.plan_id.as_deref().unwrap()),
          project_id: enrollment.project_id.clone(),
          issue_id: enrollment.issue_id.clone(),
          plan_id: enrollment.plan_id.clone().unwrap(),
      };
      let origin = AdvancePlanOrigin::Enrolled {
          enrollment_id: enrollment.enrollment_id.clone(),
          policy_revision: enrollment.policy_revision,
      };
      let first = advance_plan(&fixture.state, input.clone(), origin.clone()).await.unwrap();
      let second = advance_plan(&fixture.state, input, origin).await.unwrap();
      let first_id = match &first {
          AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.as_str(),
          AdvanceOutcome::Replayed { record } => record.attempt_id.as_deref().unwrap(),
          AdvanceOutcome::Rejected { code, .. } => panic!("unexpected reject: {code}"),
      };
      let second_id = match &second {
          AdvanceOutcome::Completed { attempt_id, .. } => attempt_id.as_str(),
          AdvanceOutcome::Replayed { record } => record.attempt_id.as_deref().unwrap(),
          AdvanceOutcome::Rejected { code, .. } => panic!("unexpected reject: {code}"),
      };
      assert_eq!(first_id, second_id);
      assert_eq!(fixture.coding_attempts().len(), 1);
      assert_eq!(fixture.coding_runner_count(), 0);
  }
  ```

  本任务在现有 `src/web/wiga_gate_fixture.rs` 新增共享 `pub(crate) async fn confirmed_enrolled_fixture() -> EnrolledGateFixture`，调用顺序为 `EnrolledGateFixture::new().await` → `fail_compile_after_human_approve().await` → `recover_and_confirm_compile().await`；只保证 durable Confirmed/已发布 compile，不冒称 Ready。`EnrolledGateFixture.state` 已是 `pub(crate)`，不增同名方法；新增 `pub(crate) fn enrollment(&self) -> IssueAutomationEnrollment`（用 `IssueAutomationStore::new(self.inner.paths.clone()).get(PROJECT_ID,ISSUE_ID)`）、`coding_attempts()/coding_runner_count()` 从 `inner.paths`/`CodingAttemptStore`/`state.coding_runs` 读真实事实。**后续任务用到的其余访问器**——`attempt()`（Task 3/4/8）、`runner_count(&CodingAttemptRunKey)`（Task 3/4）、`attempt_key()/plan_id()`（Task 4/6）、`worker()`（Task 10）、`auto_origin()/restart_state()/store()`（Task 4/6）——均由各自所属任务在 `wiga_gate_fixture.rs` 追加 `pub(crate)` 声明，签名与本计划测试使用逐字一致，禁止各任务分叉出同名异形实现；测试直接以 `confirmed_enrolled_fixture().await` 为起点，advance 完成后才验证 Ready。多 target 自动整体拒绝，人工 split 路径仍可回放，Ready 前 provider 计数为零。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib enrolled_advance_replays_one_ready_attempt_without_starting_runner -- --nocapture`，预期共用符号尚不存在；`cargo test --locked --lib campaign_stage3_advance -- --nocapture` 为人工基线。
- [ ] **Step 3: 最小实现。** `advance_plan` 从 enrollment 和 manager `get_or_create` 读绑定且**再次**验证 enabled/精确身份，人工 origin 不按 enrollment 擅自升级；借 `WorkspaceEngine::handle_advance(AdvanceInput)` 的单一实现及其 record/journal 重放。自动在 engine 前根据 `resolve_authoritative_group_plan_binding_for_revision` 的 unit target 集 fail-closed（无唯一目标也拒），engine 后核 Ready 记录/attempt 精确一致；`advance_completed` 仅状态，不派 coding。人工 `handle_advance_from_handler` 调新服务并继续 `map_advance_outcome`，维持原 WS code/多 target 输出。

  ```rust
  let outcome = advance_plan(&state, AdvanceInput {
      command_id, project_id, issue_id, plan_id,
  }, AdvancePlanOrigin::Manual).await;
  // 这里只映射原 AdvanceOutcome 为人工 WS 帧，不调用 coding runner。
  ```

  `WorkspaceInboundContext` 已持有 `app_state: WebAppState`：将 `handle_advance_from_handler` 增加 `app_state: WebAppState` 参数，在 `src/web/workspace_ws_handler/decisions/inbound.rs` 的 Advance 分支传 `app_state.clone()`，由新 service 通过 `state.workspace_sessions.get_or_create` 取得同一 manager；保留 `map_advance_outcome`，不重建 engine、不启动 coding。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2 两条；补 `cargo test --locked --lib advance_split_recovery_matrix -- --nocapture` 和 `cargo test --locked --lib campaign_stage3_advance -- --nocapture`，持久 journal replay 与手工多 target 对照。
- [ ] **Step 5: 提交。** `git add src/web/advance_plan.rs src/web/mod.rs src/web/workspace_ws_handler src/product/workspace_engine/tests/advance_split_recovery_matrix.rs && git commit -m "feat: share guarded advance plan entry"`（只加实际变更）。

## Task 2：3.1 attempt `CodingStartRunPolicy` 冻结、兼容旧记录

**Files:** Modify `src/product/coding_models/{execution.rs,mod.rs}`、`src/product/coding_attempt_store/{inputs.rs,attempt.rs,group.rs,group_initialization.rs,tests.rs}`、`src/product/workspace_engine/advance.rs`、`src/web/advance_plan.rs`（由已验证的服务传入 policy，不让 product 自行猜 enrollment）；同步所有显式构造 `CreateGroupCodingAttemptInput`/`CodingExecutionAttempt` 的 fixture。

**Interfaces:** Produces `CodingStartRunPolicy` + `attempt.start_run_policy`；`CreateGroupCodingAttemptInput` 新增 `start_run_policy: CodingStartRunPolicy`。普通/手工/legacy group/多 target 输入显式 Manual；Task 1 的自动服务精确核 enabled/source/plan revision/唯一 target 后传 `AutoStartOnce`，产品层仅持久化快照不读 web enrollment。`prepare_group_initialization_with_admission_for_target` 首次准备时经 `build_group_initialization_journal` 写入 `journal.attempt`，已有 journal 保留原 policy，不把重读的新 policy 偷换；旧 JSON 缺字段默认 Manual，`update_attempt_non_status_fields` 从 stored 保留 policy，Task 3/4 认领时重核 enrollment。

- [ ] **Step 1: 写失败测试。** 在现有 `src/product/coding_attempt_store/tests.rs` 使用 `setup()` fixture，测试旧 JSON 缺字段、copy-update 禁止覆盖冻结 policy、手工 group 与已绑定 enrollment 的新 advance journal：

  ```rust
  #[test]
  fn old_attempt_json_defaults_to_manual_and_frozen_policy_survives_updates() {
      let (_tmp, store, created) = setup(); // 已有真实 TempDir 与单 attempt。
      let mut json = serde_json::to_value(&created).unwrap();
      json.as_object_mut().unwrap().remove("start_run_policy");
      let old: CodingExecutionAttempt = serde_json::from_value(json).unwrap();
      assert_eq!(old.start_run_policy, CodingStartRunPolicy::Manual);
      let mut replacement = created.clone();
      replacement.start_run_policy = CodingStartRunPolicy::AutoStartOnce {
          enrollment_id: "enrollment_0001".into(), policy_revision: 1,
          source_plan_revision: "work_item_plan_revision_0001".into(),
      };
      store.update_attempt_non_status_fields(&replacement).unwrap();
      assert_eq!(store.get_attempt(&created.project_id, &created.issue_id, &created.id)
          .unwrap().start_run_policy, CodingStartRunPolicy::Manual);
  ```

  `setup()` 在 `src/product/coding_attempt_store/tests.rs:47-68` 已有；上述测试同时覆盖旧 serde 缺字段及 caller 试图替换冻 policy，另在 Task 1 的 Confirmed→advance fixture 读取新 journal attempt 断言 policy_revision/plan revision/target 冻结，disable 不洗白；普通、历史、多 target 手工路径均 Manual。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib old_attempt_json_defaults_to_manual_and_frozen_policy_survives_updates -- --nocapture`，预期缺字段类型；编译必须覆盖所有显式 attempt fixture。
- [ ] **Step 3: 最小实现。** `CodingExecutionAttempt`、`CodingExecutionAttemptSerde`、手写 `Deserialize` 三处同步 `#[serde(default)] start_run_policy`，普通/group/journal 构造和所有显式 fixture 加新输入字段；`update_attempt_non_status_fields` 保留 `stored.start_run_policy`。`WorkspaceEngine::handle_advance(input)` 保留 Manual 签名与行为，新增 `handle_advance_with_start_policy(input, policy)` 仅给 Task 1 自动路径并只在新建单 target group journal 时穿透 `CreateGroupCodingAttemptInput.start_run_policy`；已有 journal replay 无论当前 enrollment 如何变更都不覆盖。新增字段示意：

  ```rust
  #[serde(default)]
  pub start_run_policy: CodingStartRunPolicy,
  // 反序列化时：start_run_policy: raw.start_run_policy；copy-update 时：
  // start_run_policy: stored.start_run_policy，绝非 attempt.start_run_policy。
  ```

  类型序列化采用已存在 snake_case 约定；`source_plan_revision` 保持 revision **id**，不可用 mutable latest ref 替代；历史 JSON、普通 attempt、手工多 target replay 测试均覆盖。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2，加 `cargo test --locked --lib group_initialization -- --nocapture`；Ready 前自动仍零 provider。
- [ ] **Step 5: 提交。** `git add src/product/coding_models src/product/coding_attempt_store src/product/workspace_engine/advance.rs && git commit -m "feat: freeze coding auto-start policy on new attempt"`。

## Task 3：3.1 typed StartCoding 共用准入：状态、精确授权、Ready、无批量

**Files:** Create `src/web/coding_start.rs`；Modify `src/product/coding_models/{execution.rs,mod.rs}`（共享 `CodingStartOrigin`）、`src/web/mod.rs`；Test 新 `coding_start.rs` 的 `#[cfg(test)]` 和 `src/web/coding_ws_handler/tests/sc_start_guard.rs`。Socket 真正首启迁接在 Task 5，Task 3 不创建生产可调用的裸 runner 替代入口。

**Interfaces:** Produces `StartCodingCommand/Outcome/Error`、`start_coding_once` 和 product `CodingStartOrigin`（不定义 web 同名类型）；本任务先写只读准入：只有 Created+PrepareContext 可首启，现存 claim 只返回原身份状态；Running/WaitingForHuman/Completed/Failed/Aborted/AwaitingManualRecovery 不重新认领。自动核冻结 policy、current enrollment.enabled/id/source/plan/session/target、group journal 的 `plan_id` 与 `plan_binding` revision、`AdvanceRecord` Ready/唯一 attempt/target。手工仍保留 SC Ready 门但不强制 enrollment；非 SC legacy 旧手工判据不变。完整启动在 Task 4 实现，缺/坏 record fail-closed。

- [ ] **Step 1: 写失败测试。** 沿 Task 1 的共享 `wiga_gate_fixture::confirmed_enrolled_fixture()` 建 Confirmed，使用 `advance_handler.rs::advance_initialization_replay_resumes_same_record_attempt_and_units` 的 `JournalPrepared` failpoint 在相同 project/issue/plan 下取得**真实 journal lineage**、尚未 Ready 的 attempt；将该 fixture helper 命名 `enrolled_attempt_before_ready()` 并在本任务 `coding_start.rs` 的 `#[cfg(test)]` 首次定义；现有 `sc_start_guard.rs::start_coding_before_advance_ready_is_rejected_with_sc_coding_requires_advance` 作 WS 守卫对照：

  ```rust
  #[tokio::test]
  async fn start_coding_rejects_sc_before_durable_ready() {
      let fixture = enrolled_attempt_before_ready().await;
      let attempt = fixture.attempt();
      let result = start_coding_once(&fixture.state, &attempt.project_id, &attempt.issue_id,
          StartCodingCommand {
              attempt_id: attempt.id.clone(), command_id: "auto-start-1".into(),
              origin: CodingStartOrigin::Enrolled {
                  enrollment_id: fixture.enrollment().enrollment_id,
                  policy_revision: fixture.enrollment().policy_revision,
              },
          }).await;
      assert_eq!(result.unwrap_err().code(), "SC_CODING_REQUIRES_ADVANCE");
      assert_eq!(fixture.runner_count(&attempt.id), 0);
  }
  ```

  `enrolled_attempt_before_ready` 使用真实 `AdvanceInitializationPhase::JournalPrepared` 中窗、绝不注入无 lineage 的 Created 壳；新增禁用/重开、旧版本/源/target 漂移、`target_attempts.len()>1` 测试，断言全部 sibling claim/run 均空，人工单 target 与显式 Restart 为独立路径。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib start_coding_rejects_sc_before_durable_ready -- --nocapture`；`cargo test --locked --lib start_coding_before_advance_ready_is_rejected_with_sc_coding_requires_advance -- --nocapture`，新服务缺失/无共用调用。
- [ ] **Step 3: 最小实现。** 复用已有 `coding_message_admission` 的状态矩阵与 `socket.rs:304-353` 中 `SC_CODING_REQUIRES_ADVANCE` 的错误码，但不改旧 WS 守卫直到 Task 5 一次迁接。typed service 根据 current enrollment 与 attempt 冻结 policy 实施只读 guard；Ready 查询 `plan_id` 从 `CodingGroupInitializationJournal.plan_id` 读取，同时核其 `plan_binding` 与 `AdvanceRecord.plan_revision_id`、attempt.id，**不假定** `work_item_group_id` 可作 plan_id。Task 4 将在 enrollment 文件锁内重验并消费；多 target 判据同时看权威 binding、`AdvanceRecord.target_attempts`、attempt 快照与 enrollment logical id，不按 sibling 循环。

  ```rust
  let journal = coding_store.get_group_initialization(project_id, issue_id, &bound_plan_id)?;
  if attempt.admission_kind == CodingAdmissionKind::ScAdvance
      && (!matches!(record.status, AdvanceStatus::Ready)
          || journal.attempt.id != attempt.id
          || !advance_store.advance_is_ready_for_attempt(project_id, issue_id,
              &journal.plan_id, &attempt.id)?) {
      return Err(StartCodingError::RequiresAdvance);
  }
  ```

  `RequiresAdvance.code()` 恒 `SC_CODING_REQUIRES_ADVANCE`；无法读取 Ready 也映射该码附真实错误原因，不吞为 OK。Manual 首启不会因为有人 enroll 后被误认成自动 permission；Task 4 负责 claim+runner 接线，不在本任务留下可在生产路径裸调的替代入口。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2 与 `cargo test --locked --lib sc_start_guard -- --nocapture`；测试同时涵盖缺 plan binding、非 Ready 与多 target 的零 claim/零 runner。
- [ ] **Step 5: 提交。** `git add src/web/coding_start.rs src/web/mod.rs src/web/coding_ws_handler && git commit -m "feat: unify typed coding start admission"`。

## Task 4：3.1 durable 单发 claim、registry reservation 与启动 barrier

**Files:** Modify `src/product/coding_models/execution.rs`、`src/product/coding_attempt_store/{attempt.rs,mod.rs}`、`src/product/issue_automation_store.rs`（enrollment 持锁 closure）；Modify `src/web/{coding_start.rs,coding_ws_handler/runner.rs,coding_ws_handler/runner/task.rs,wiga_gate_fixture.rs}`；Test `src/web/coding_ws_handler/tests/{runner_cleanup.rs,runner_recovery.rs}`、`src/product/coding_attempt_store/tests.rs` 新增 claim 测试。

**Interfaces:** Produces `CodingStartClaim/Phase`、`CodingAttemptStore::claim_coding_start/advance_coding_start_phase`、`IssueAutomationStore::with_current_enrollment_locked(project_id,issue_id,f)`（同 enrollment 文件锁重读，`f: FnOnce(&IssueAutomationEnrollment)->Result<T,ProductStoreError>`，闭包内仅同步检查并认领 attempt）、首启专用 `spawn_coding_runner_first_start_reserved(state,store,event_tx,attempt,reservation,command_id)`；原 `spawn_coding_runner` 仅显式 Restart/Recover。锁序 enrollment file → attempt file；entry 前可用 `CodingRunRegistry::lock_attempt` 排串行，但不得持 await 锁跨同步文件锁闭包。任何锁内禁止 await/provider 启动。claim 是 attempt 同文件不可复位身份；reservation 丢失不回滚已消费许可。

- [ ] **Step 1: 写失败测试。** 在新首启测试模块使用 Task 3 的真实 Ready fixture 和两个 `WebAppState` 指向同一 `.aria`（模拟跨进程不同 registry）：

  ```rust
  #[tokio::test]
  async fn manual_and_auto_claim_same_attempt_once_across_registries() {
      let fixture = ready_enrolled_attempt_fixture().await;
      let attempt = fixture.attempt();
      let state_a = fixture.state.clone();
      let state_b = fixture.restart_state();
      let manual = start_coding_once(&state_a, &attempt.project_id, &attempt.issue_id,
          StartCodingCommand { attempt_id: attempt.id.clone(),
              command_id: "manual-1".into(), origin: CodingStartOrigin::Manual });
      let auto = start_coding_once(&state_b, &attempt.project_id, &attempt.issue_id,
          StartCodingCommand { attempt_id: attempt.id.clone(),
              command_id: "auto-1".into(), origin: fixture.auto_origin() });
      let (a, b) = tokio::join!(manual, auto);
      assert_eq!(usize::from(matches!(a.unwrap(), StartCodingOutcome::Started { .. }))
          + usize::from(matches!(b.unwrap(), StartCodingOutcome::Started { .. })), 1);
      let saved = fixture.store().get_attempt(&attempt.project_id,
          &attempt.issue_id, &attempt.id).unwrap();
      assert!(saved.start_claim.is_some());
      assert_eq!(fixture.store().list_role_runs(&attempt.project_id,
          &attempt.issue_id, &attempt.id).unwrap().len(), 1);
  }
  ```

  本任务在 `src/web/wiga_gate_fixture.rs` 新增共享 `ready_enrolled_attempt_fixture()` 与 `auto_origin()/restart_state()/store()`，由该文件现有 `EnrolledGateFixture` 建 Confirmed 并调用 Task 1 服务到 Ready，真实角色 run 只统计首启（如果 stage gate 尚未产生 provider role run，用既有 provider-start durable ledger/claim 与**已启动 runner 总数**双断言；测试须以 provider 真实入口 probe 控制中窗，不用固定 sleep）。另写 barrier failpoint：claim 后/registry 激活前、激活后/放行前、放行后/provider 事实不明三窗；首两窗重启只恢复同 command，最后人工分诊。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib manual_and_auto_claim_same_attempt_once_across_registries -- --nocapture`，预期两个内存 registry 旧路径可各自启动；runner probe 用例按名字同目录定向运行。
- [ ] **Step 3: 最小实现。** Claim 状态写 attempt 文件、CAS/reload 后比对 command 与 origin；读取错误/不匹配 fail-closed。`Claimed` 在未给 runner 放行前可复用相同身份；激活 reservation 后持久 `RunnerRegistered`，写失败即 drop oneshot、撤 registry，不放 provider；持久 `ProviderMayHaveStarted` **先于** start_tx.send(()): 跨进程重启此窗口不保证未触达 provider，只能从可信 Running/role run ledger 复用既有 resumption 或标 `NeedsHuman`。Task 3 守卫还须在锁内再执行一次（自动许可/disable 的单一线性化点）；**不允许**「读 enrollment 后放锁再写 claim」。runner 使用 `CodingRunnerTask {start_rx:Some(...)}` 已有结构，首启专用屏障不复用 recovery journal：

  ```rust
  let reservation = state.coding_runs.try_reserve_attempt(&attempt_key)
      .ok_or(StartCodingError::AlreadyStarted)?;
  let claimed = match store.claim_coding_start(&attempt,
      &command.command_id, &command.origin)? {
      ClaimCodingStartOutcome::Claimed(value) => value,
      ClaimCodingStartOutcome::Existing(_) => return Ok(StartCodingOutcome::AlreadyStarted {
          attempt_id: attempt.id.clone(),
      }),
  };
  let sender = spawn_coding_runner_first_start_reserved(state.clone(), store.clone(),
      state.coding_sockets.hub_sender(&attempt_key), claimed,
      reservation, &command.command_id)?;
  ```

  这是同一 service 的片段，实际 claim/registry 次序在持有同 attempt guard 内实现；拿不到 registry 而无 claim 时不消费许可。`runner/task.rs` 在放行前后持久 checkpoint；禁止 `spawn_coding_runner_first_start_reserved` 对不同 command 二次放行。真实 provider 入口与 claim 中窗的测试 probe 使用已有 `CodingRunnerStartProbe` 机制，指定三点状态及 restart 期望，不以单次 runner 注册等同 provider 启动。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2 与 `cargo test --locked --lib runner_cleanup -- --nocapture`、`cargo test --locked --lib runner_recovery -- --nocapture`；禁用先胜无 claim、claim 先胜不中断且重开不复位、Failed/Aborted 不触发首启均由 durable store 对账。
- [ ] **Step 5: 提交。** `git add src/product/coding_models/execution.rs src/product/coding_attempt_store src/product/issue_automation_store.rs src/web/coding_start.rs src/web/coding_ws_handler/runner.rs src/web/coding_ws_handler/runner/task.rs src/web/coding_ws_handler/tests src/web/wiga_gate_fixture.rs && git commit -m "feat: claim coding first start durably before runner barrier"`。

## Task 5：3.1 人工 WS 与后台 reconcile 共用服务，严格两动作分开

**Files:** Modify `src/web/{coding_ws_handler/socket.rs,coding_ws_handler/socket/preparation.rs,autopilot_orchestrator.rs,advance_plan.rs}`；Test `src/web/autopilot_orchestrator.rs` 内测试及 `src/web/coding_ws_handler/tests/sc_start_guard.rs`。

**Interfaces:** Consumes Task 1 `advance_plan`、Task 3/4 `start_coding_once`；扩展现有 `ReconcileOutcome` 增加 `Advancing`、`Coding`（保留已落地五个变体语义）。`Confirmed` 只在 `plan_confirmed_info` 可派生成功 publication/compile、current enrollment 精确匹配且**无人门/choice/compile recovery**时，请求 stable-id advance；本轮到 Ready 即止。下一轮读取 Ready 唯一 attempt 后才请求 stable-id AutoStartOnce；重复唤醒不重启。WS StartCoding 抽掉原 `spawn_coding_runner` 首启直调，改走同一个 typed service；人工 Restart/Recover 分支仍显式、保留原 socket wire 错误与状态更新。

- [ ] **Step 1: 写失败测试。** 从 Task 1 `confirmed_enrolled_fixture()`（Confirmed 尚未 advance）起步，连续两轮真实 reconcile；Task 4 的 runner probe 负责统计同 attempt 首启，不得先用已 Ready fixture 使第一轮 `Advancing` 断言假失败：

  ```rust
  #[tokio::test]
  async fn confirmed_enrollment_advances_then_starts_without_coding_socket() {
      let fixture = confirmed_enrolled_fixture().await;
      let worker = AutopilotOrchestrator::new(fixture.state.clone(), Default::default());
      assert_eq!(worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
          ReconcileOutcome::Advancing);
      let ready = fixture.coding_attempts().into_iter().next().unwrap();
      assert_eq!(fixture.state.coding_runs.runner_count(
          &CodingAttemptRunKey::from_attempt(&ready)), 0,
          "advance must not sneak provider start into same transition");
      assert_eq!(worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
          ReconcileOutcome::Coding);
      assert_eq!(fixture.state.coding_runs.runner_count(
          &CodingAttemptRunKey::from_attempt(&ready)), 1);
      assert_eq!(worker.reconcile(&fixture.state, PROJECT_ID, ISSUE_ID).await.unwrap(),
          ReconcileOutcome::Coding);
      assert_eq!(fixture.coding_attempts().into_iter().next().unwrap().id, ready.id);
  }
  ```

  Task 1 共享 fixture 在第一轮之前保证 Confirmed、零 attempt；第一次 reconcile 后 assert 新 Ready attempt，第二轮后用 `CodingRunRegistry::runner_count` + Task 4 durable claim 核一份首启；补 disable→重开、源 drift、人工 WS `StartCoding` 并发、Failed/Aborted/旧 attempt 不重启，WS 用 `sc_start_guard.rs` 真实服务验证 Ready 错码。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib confirmed_enrollment_advances_then_starts_without_coding_socket -- --nocapture`，预期 P1 Confirmed 仅 NeedsHuman；`cargo test --locked --lib sc_start_guard -- --nocapture` 为旧手工行为基线。
- [ ] **Step 3: 最小实现。** `AutopilotOrchestrator::reconcile` 中先检查终态 session：仅 Confirmed+Complete 且 compile publication `plan_confirmed_info` 成功时按 durable advance record 判断；先单独调用 `advance_plan`，其返回 Ready 才在后**一次 tick**调用 `start_coding_once`；不能从 `advance_completed` 事件直接 spawn。Stable command `wiga-start-{attempt.id}`（同 claim 意图固定，id 验证合规）；终态 Failed/Aborted/NeedsHuman 一律返回 NeedsHuman。WS `prepare_coding_message` 在 `StartCoding` 上仍完成基本前置，但在进入共用 service 前释放其 mutation lease，让 service 自持 attempt guard 并重读，**不能持 lease 后再 await 同名锁**；保留连接级 runner_started 状态作观察值而非准入唯一事实：

  ```rust
  if inbound == CodingWsInMessage::StartCoding {
      drop(mutation_lease);
      let result = start_coding_once(&state, &current_attempt.project_id,
          &current_attempt.issue_id, StartCodingCommand {
              attempt_id: current_attempt.id.clone(),
              command_id: manual_command_id_for_this_frame(),
              origin: StartCodingOrigin::Manual,
          }).await;
      // Started/AlreadyStarted/NeedsHuman -> 既有 snapshot 或 CodingProtocolError。
  }
  ```

  `manual_command_id_for_this_frame` 由本任务实现为合法随机 ID（仅第一次帧生成，同帧处理不二次生成）；并发失败复用 durable claim 的 attempt 状态而非发第二个 runner。Restart/Recover 仍走原明示独立入口；归属未知的旧 client advance 由现有服务端幂等兜底，不改 P1 `useCockpitAutopilot` 的 server 退位。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2 两条与 `cargo test --locked --lib automation_reconcile -- --nocapture`、`cargo test --locked --lib sc_start_guard -- --nocapture`；不需要任何 coding WS 的测试才算后台首启证据。
- [ ] **Step 5: 提交。** `git add src/web/autopilot_orchestrator.rs src/web/advance_plan.rs src/web/coding_ws_handler src/web/wiga_gate_fixture.rs && git commit -m "feat: drive confirmed plan to ready and coding without sockets"`。

## Task 6：3.2 无 attach 启动扫描与已认领原 run 恢复

**Files:** Modify `src/web/{app.rs,autopilot_orchestrator.rs,coding_ws_handler/socket.rs,coding_ws_handler/socket/resumption.rs,coding_start.rs}`；Test `src/web/coding_ws_handler/tests/runner_recovery.rs` 和 `src/web/autopilot_orchestrator.rs` 测试模块。

**Interfaces:** Consumes `list_attempts_for_issue`、既有 `ensure_runner_for_resumed_attempt`（仅 Running + WorktreePrepare/Coding、SC Ready 仍复核）和 Task 4 `CodingStartClaim`；produces `reconcile_claimed_coding_runs_once(state: &WebAppState) -> Result<usize,String>`，在 `serve_web` 构造 state 后**先**做可观察启动扫描（可分批，不阻塞永久运行），再进入既有 2 秒有界 tick。Created + 没 claim 不启动；`Claimed` 未 barrier 可按同一 command/origin恢复；`RunnerRegistered/ProviderMayHaveStarted` 仅凭足够的 Running/ledger 事实走旧 resumption；副作用不明标 NeedsHuman、attempt `AwaitingManualRecovery`，给 UI durable 诊断；Failed/Aborted/Completed 不恢复。

- [ ] **Step 1: 写失败测试。** 两份 `WebAppState` 指同一 tempdir：先让 Task 4 首启 probe 卡在已持久 `ProviderMayHaveStarted` 且 attempt 已进入可安全恢复的 Running/WorktreePrepare，模拟进程销毁，再不连接任何 WS 执行启动扫描：

  ```rust
  #[tokio::test]
  async fn startup_reconciles_claimed_running_coding_without_attach() {
      let fixture = claimed_running_attempt_fixture().await;
      let fresh = fixture.restart_state();
      assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 0);
      reconcile_claimed_coding_runs_once(&fresh).await.unwrap();
      assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 1);
      assert_eq!(fixture.store().list_attempts_for_work_item_group(
          PROJECT_ID, ISSUE_ID, &fixture.plan_id()).unwrap().len(), 1);
      reconcile_claimed_coding_runs_once(&fresh).await.unwrap();
      assert_eq!(fresh.coding_runs.runner_count(&fixture.attempt_key()), 1);
  }
  ```

  `claimed_running_attempt_fixture` 在 `runner_recovery.rs` 本任务新增：调用 `CodingAttemptStore` 既有 `pub(crate)` 方法 `seed_running_attempt_for_test`（定义于 attempt.rs:203-214，runner_recovery.rs 是其调用方）造 durable admission，加 Task 4 claim 与 pause probe，不直接伪造 Started external 调用；另一用例停在可能已触达 provider 的 Created 无可信 ledger，期望 `AwaitingManualRecovery` 且 0 runner；legacy Running 有既有准入证据也按原规则恢复。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib startup_reconciles_claimed_running_coding_without_attach -- --nocapture`；预期没有 startup reconcile，旧恢复必须 socket attach。
- [ ] **Step 3: 最小实现。** 启动扫描沿 `ProjectStore::list`→`IssueStore::list`→`CodingAttemptStore::list_attempts_for_issue`，对 attempt 带 Task 4 claim 或旧 Running+可信 admission，建立 `coding_sockets.hub_sender`（**不登记 fake socket**），用相同 attempt lock 调已有 `ensure_runner_for_resumed_attempt`；当前 attach 不再担当*唯一*恢复触发，保留其补快照/choice/amendment 功能，attach 与 startup/tick 同 claim/registry 去重。`ensure_runner_for_resumed_attempt` 进入新失败分诊要区别“已有 runner/预约＝NotNeeded”和“确实检查失败＝manual recovery”，不把抢输者标失败。启动修复必须不会等待 socket 事件 ack。

  ```rust
  let event_tx = state.coding_sockets.hub_sender(&attempt_key);
  match ensure_runner_for_resumed_attempt(state, &store, &event_tx,
      &attempt_key, &attempt).await {
      ResumedAttemptRunner::NotNeeded => {}
      ResumedAttemptRunner::Restarted { .. } => { resumed += 1; }
      ResumedAttemptRunner::ManualRecovery { .. } => { /* durable 故障在原 helper 写入 */ }
  }
  ```

  对 `Claimed` 未跨 barrier 的安全窗用 Task 4 单一恢复方法，不能把所有 `Created` 都交上述只收 Running 的 helper；对外部是否收到 provider 无法证明的窗直接 NeedsHuman 而非再跑 start_attempt。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2，并跑 `cargo test --locked --lib runner_recovery -- --nocapture`、`cargo test --locked --lib automation_reconcile -- --nocapture`。关闭所有订阅后依旧有 1 runner；Failed/Aborted/no claim 各 0。
- [ ] **Step 5: 提交。** `git add src/web/app.rs src/web/autopilot_orchestrator.rs src/web/coding_start.rs src/web/coding_ws_handler && git commit -m "feat: recover claimed coding runs at startup without socket"`。

## Task 7：3.2 零 socket choice/amendment、慢观察者的业务隔离回归

**Files:** Modify `src/web/state/coding_socket_registry.rs`（仅慢订阅队列满时非阻塞 fan-out；不碰已落地 amendment 业务 journal）、`src/web/coding_ws_handler/tests/event_hub.rs`、`src/web/workspace_ws_handler/tests/plan_repair_activation.rs`、`src/web/handlers/coding_choice.rs` 的测试模块；如 Task 6 暴露连接态 race，仅修 `src/web/coding_ws_handler/socket.rs` 的观察回帧，不能改 P0 choice wire/真实回执。

**Interfaces:** 复用已有 `CodingRunRegistry::claim_choice/submit_claimed_choice/wait_choice_receipt`、`coding_choice` REST 200/202/410、`CodingPlanAmendmentDeliveryStatus::{Pending,Unsent,Delivered}` 和 zero-socket amendment 既有用例 `zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery`；不新增投递协议。Registry fan-out 满队列不 await 慢 socket：跳过当前 socket 的本帧写入机会并向 amendment ack 注册表报告这份写失败，强制该观察者重订阅补 durable；其他 socket 可继续，零订阅者 Unsent。

- [ ] **Step 1: 写失败测试。** `event_hub.rs` 用已有 socket registry fixture 与容量 1 的慢接收端；`plan_repair_activation.rs` 在零 socket confirm 后核 status 已 Running + Unsent，再 attach 仅真实写 ack 能转 Delivered。`coding_choice.rs` 已有 `coding_choice_reply_http_202_then_retry_200_with_full_answers` 的真实 `coding_choice_http_fixture`、`request_body/post_choice/paused_coding_runner/get_status`，扩成显式「zero socket」回归而非新造 waiter：

  ```rust
  #[tokio::test]
  async fn no_socket_coding_choice_is_delivered_only_to_live_waiter() {
      let fixture = coding_choice_http_fixture().await;
      let (release, seen, answers) = paused_coding_runner(fixture.command_rx);
      // fixture 只登记 coding_runs command，**不**注册 coding_sockets。
      let body = request_body("choice-zero-socket", &fixture.incarnation);
      let (pending, value) = post_choice(&fixture.router, &fixture.attempt_id,
          "choice-http-1", &body).await;
      assert_eq!(pending, StatusCode::ACCEPTED, "{value}");
      assert_eq!(value["state"], "resolving");
      assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 1);
      assert_eq!(answers.lock().unwrap().as_ref().unwrap(), &answers_payload());
      release.notify_one(); // 真实 provider waiter receipt，而非 mpsc 入队。
      let (delivered, value) = post_choice(&fixture.router, &fixture.attempt_id,
          "choice-http-1", &body).await;
      assert_eq!(delivered, StatusCode::OK, "{value}");
      assert_eq!(value["state"], "delivered");
      assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 1);
  }
  ```

  已有 fixture 可直接完成零 socket 多问题 202→receipt→200 验证；同模块加无 waiter 反例仍为 202/410，不以 mpsc 入队冒充 Delivered。慢队列测试 `timeout(Duration::from_millis(250), hub_tx.send(event))` 必须同时核 durable amendment Unsent，不以断开 socket 规避反压。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib no_socket_coding_choice_is_delivered_only_to_live_waiter -- --nocapture` 与 `cargo test --locked --lib slow_observer_does_not_block_coding_business -- --nocapture`，预期第二条在旧 `broadcast` 的 `target.send(...).await` 卡满队列；首条若已由 P0 满足仍为保护性回归。
- [ ] **Step 3: 最小实现。** 仅修改 `CodingSocketRegistry::broadcast`：保留现有 `expect_plan_amendment_fan_out_writes(event, targets.len())` **先登记**，遍历每目标 `try_send(event.clone())`；Full/Closed 同步调用现有 `fail_plan_amendment_socket_write(event)` 各结算一份，已成功入队者由真实 socket writer 成功或失败 ack 再结算。慢 socket 不阻塞业务，落后订阅须重订阅 durable snapshot/Unsent；不能以队列入队视为 Delivered，也不重复调用期待份额。实现片段：

  ```rust
  expect_plan_amendment_fan_out_writes(event, targets.len());
  for target in targets {
      if target.try_send(event.clone()).is_err() {
          fail_plan_amendment_socket_write(event);
      }
  }
  ```

  注意 `expect_plan_amendment_fan_out_writes(event, targets.len())` 先登记份额，失败每份恰一次，已入队但写失败仍由 socket writer 的既有 ack 机制结算。业务 amendment 解耦已完成，不重复改引擎或回执模型。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2，再跑 `cargo test --locked --lib zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery -- --nocapture`、`cargo test --locked --lib coding_choice -- --nocapture`、`cargo test --locked --lib event_hub -- --nocapture`；核重连见同一业务终态、只在真写成功后 Delivered、未重复 application。
- [ ] **Step 5: 提交。** `git add src/web/state/coding_socket_registry.rs src/web/coding_ws_handler/tests/event_hub.rs src/web/workspace_ws_handler/tests/plan_repair_activation.rs src/web/handlers/coding_choice.rs && git commit -m "fix: keep slow coding observers off business path"`。

## Task 8：3.3 durable FinalConfirm 等待信息后端投影

**Files:** Create `src/web/coding_final_confirm_info.rs`；Modify `src/web/{mod.rs,types.rs,handlers/lifecycle.rs}`；Test 新文件 `#[cfg(test)]`，扩展 `src/product/coding_workspace_engine/tests/group_final_readiness.rs` 的重复准备测试（不改 handoffs 人工动作）。

**Interfaces:** Produces `CodingFinalConfirmInfoDto` 与 `issue_coding_final_confirm_info`，增量 `IssueLifecycleResponse.coding_final_confirm_info`。只读从 enrolled 且**已认领**的单 target group attempt 的持久 `WaitingForHuman+FinalConfirm`、`GroupFinalReadinessStatus::Complete`、无 diagnostics/完整 units、同 attempt 的 FinalConfirm pending 节点推导；`Completed` 后查同节点已完成，复用**同 key/原 `started_at`** 把标题改「已最终确认」且 `final_confirmed=true`，不产生新提醒。禁用 enrollment 后，**已经成功认领**的 run 仍能显示结果；未认领的旧 Manual attempt 不冒充自动完成。缺节点/identity 不匹配/坏 snapshot fail-closed，读取错误传播 HTTP，不吞作空。

- [ ] **Step 1: 写失败测试。** 生产 `src/web/coding_final_confirm_info.rs` 内仅测试模块使用 Task 4 的 `ready_enrolled_attempt_fixture()`、`EnrolledGateFixture`、原 `CodingWorkspaceEngine::prepare_group_final_confirm_from_readiness` 与 `handle_final_confirm`。`src/product/coding_workspace_engine/tests/group_final_readiness_support.rs::readiness_fixture/seed_complete_group_readiness` 作用于**独立临时 repo**，不可跨模块调用也不可把其 snapshot 复制到 enrolled attempt；在同一个 enrolled group attempt 上通过 Fake 真实 runner 产生 unit/handoff/review/readiness，再断言投影与重复准备。正反例：

  ```rust
  #[tokio::test]
  async fn coding_info_stays_stable_when_readiness_is_rewritten() {
      let fixture = complete_enrolled_group_waiting_for_final_confirm().await;
      let first = issue_coding_final_confirm_info(&fixture.inner.paths,
          PROJECT_ID, ISSUE_ID).unwrap().remove(0);
      fixture.prepare_group_final_confirm_again().await;
      let second = issue_coding_final_confirm_info(&fixture.inner.paths,
          PROJECT_ID, ISSUE_ID).unwrap().remove(0);
      assert_eq!((first.key, first.occurred_at), (second.key, second.occurred_at));
      assert!(!second.final_confirmed);
      assert_eq!(fixture.attempt().status, CodingAttemptStatus::WaitingForHuman);
      fixture.confirm_final_by_human().await;
      let completed = issue_coding_final_confirm_info(&fixture.inner.paths,
          PROJECT_ID, ISSUE_ID).unwrap().remove(0);
      assert_eq!(completed.key, second.key);
      assert_eq!(completed.occurred_at, second.occurred_at);
      assert!(completed.final_confirmed);
  }
  ```

  Task 8 在 `src/web/wiga_gate_fixture.rs` 新增共享 `pub(crate) async fn complete_enrolled_group_waiting_for_final_confirm() -> EnrolledGateFixture`：接 Task 4 已 Ready/claimed Fake attempt，令 Fake runner 沿实际 unit→handoff→readiness 的业务入口运行至 `WaitingForHuman+FinalConfirm`，从同一 attempt 的 `CodingAttemptStore` 读取完整 snapshot/节点；`prepare_group_final_confirm_again()` 复用原 `prepare_group_final_confirm_from_readiness`，`confirm_final_by_human()` 使用原 `handle_final_confirm`。失败即暴露真实缺失的 group 事实，不通过另一个 fixture 拷贝 snapshot 或手改 attempt status。缺 readiness/Incomplete/Completed 但无人工节点/非 enrolled/错 plan 各为反例。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib coding_info_stays_stable_when_readiness_is_rewritten -- --nocapture`，预期尚无投影；`cargo test --locked --lib preparing_group_final_confirm_twice_reuses_pending_timeline_node -- --nocapture` 为既有事实基线。
- [ ] **Step 3: 最小实现。** `issue_coding_final_confirm_info` 枚举同 issue 有 durable claim 的 group attempts，检查 claim.origin=Enrolled、冻结 plan/单 target、readiness snapshot identity/status/diagnostics/units，找到匹配 `attempt.id` 的当前 FinalConfirm Pending 节点；`Completed` 分支只认同一个节点 `Completed` 和 attempt 已人工 Completed。稳定 `key=format!("coding_final_confirm:{}:{}",attempt.id,node.id)`、`occurred_at=node.started_at.clone()`；不得取 snapshot.created_at。lifecycle.rs 跟 `plan_confirmed_info` 同源挂只读字段，错误直传。

  ```rust
  let ready = store.get_group_final_readiness_snapshot(&attempt)?;
  if !matches!(ready, Some(ref snap) if snap.status == GroupFinalReadinessStatus::Complete
      && snap.diagnostics.is_empty() && !snap.units.is_empty()) {
      continue;
  }
  // 再核等待态/人工已确认态及 FinalConfirm 节点，二者同 id/time。
  ```

- [ ] **Step 4: 验证绿灯。** 重跑 Step 2，另跑 `cargo test --locked --lib group_final_readiness -- --nocapture` 和 `cargo test --locked --lib plan_confirmed_info -- --nocapture`，保证 P1 plan info 不被 coding 事实覆盖。
- [ ] **Step 5: 提交。** `git add src/web/coding_final_confirm_info.rs src/web/mod.rs src/web/types.rs src/web/handlers/lifecycle.rs src/product/coding_workspace_engine/tests/group_final_readiness.rs src/web/wiga_gate_fixture.rs && git commit -m "feat: derive final-confirm coding info from durable readiness"`。

## Task 9：3.3 驾驶舱信息去重、不可计数、正确 Coding Workspace 下钻

**Files:** Modify `web/src/{api/types/lifecycle.ts,hooks/useWorkspaceSessionObservers.ts,state/workspace-cockpit-projection.ts,components/chat-workspace/cockpit/CockpitInbox.tsx,pages/ChatCockpitPage.tsx,pages/ChatWorkspacePage.tsx,router.tsx}`；Test `web/src/{hooks/useWorkspaceSessionObservers.test.tsx,components/cockpit/CockpitShell.test.tsx,pages/ChatCockpitPage.inbox.test.tsx,router.test.tsx}`。

**Interfaces:** Consumes Task 8 `coding_final_confirm_info?: CodingFinalConfirmInfoItem[]`（TS snake_case 与 Rust DTO 同字段）；produces `codingFinalConfirmInfoItem` 的 `CockpitInboxItem {kind:"info", source:"coding_final_confirm_info", codingInfo:{projectId,issueId,planId,attemptId,key,occurredAt,finalConfirmed}}`；`CockpitInbox` 增 `onOpenInfoCoding?: (address: CodingAttemptAddress)=>void`，`ChatCockpitPage`/`ChatWorkspacePage`/`router.tsx` 传递真正 coding route 导航；Plan info 继续用 `onOpenInfoSession`。只投影当前 watched plan session 所属 issue 的完成事实、按稳定 key 去重；不把 attempt id 当 session id，不进 countedInbox/选择/批量。

- [ ] **Step 1: 写失败测试。** `useWorkspaceSessionObservers.test.tsx` 已有 dependency injection `getIssueLifecycle`，注入同 key 两轮与等待→Completed；断言：

  ```tsx
  it("shows one final-confirm info without increasing actionable count", async () => {
    const info = {
      key: "coding_final_confirm:coding_attempt_001:coding_node_0004",
      project_id: "project_1", issue_id: "issue_1",
      plan_id: "work_item_plan_0001", attempt_id: "coding_attempt_001",
      occurred_at: "2026-09-27T03:20:00Z",
      title: "编码执行完成，待最终确认", final_confirmed: false,
    } satisfies CodingFinalConfirmInfoItem;
    const view = renderObserverHook(observerOptions({
      getIssueLifecycle: async () => ({
        workspace_sessions: [summary("s1")], coding_attempts: [],
        coding_final_confirm_info: [info, info],
      }),
    }));
    await waitFor(() => expect(view.result.inbox.filter(
      (item) => item.source === "coding_final_confirm_info")).toHaveLength(1));
    expect(view.result.countedInbox).toHaveLength(0);
  });
  ```

  `renderObserverHook(observerOptions({getIssueLifecycle: async () => (...) }))` 为该测试文件现有 helper；`summary("s1")` 的 issue_id=`issue_1` 与内建 listProductIssues 的 project_id=`project_1` 匹配，不另造不存在的 render helper。路由测试点击 info 按钮后断言 `/workbench/projects/project_1/issues/issue_1/coding/coding_attempt_001`（`web/src/router.test.tsx:259-260` 与 `IssueLifecycleWorkbenchParts.tsx:563-564` 已有此 scoped 路由），Completed 后同 key 显示「已最终确认」、不出现第二 toast，Plan info 仍走原会话目标。
- [ ] **Step 2: 验证红灯。** `pnpm -C web exec vitest run src/hooks/useWorkspaceSessionObservers.test.tsx src/pages/ChatCockpitPage.inbox.test.tsx src/router.test.tsx`，预期不存在 coding 信息投影/下钻。
- [ ] **Step 3: 最小实现。** TS 生命周期类型 additive optional，目录刷新沿现有 `planConfirmedInfos` 收集 codingInfos；用 watched session 的 `projectId/issueId` 索引过滤（从对应 issue lifecycle 的 workspace_sessions 获取，不按 coding attempt id 伪造 `sessionId`），dedup key；只往 `inbox` 加，`countedInbox` 不加。新 row 类型 `codingInfo` 并用独立 `onOpenInfoCoding`，`CockpitShell` 现有 `knownInfoKeysRef` 按 `item.id` 只提醒新增 key、同 key Completed 改文案不再 toast；导航回调来自现有 `router.tsx` coding route，而非当前只传 `onOpenSession` 的 plan callback：

  ```tsx
  {item.kind === "info" && item.codingInfo && onOpenInfoCoding ? (
    <button type="button" onClick={() => onOpenInfoCoding({
      projectId: item.codingInfo!.projectId,
      issueId: item.codingInfo!.issueId,
      attemptId: item.codingInfo!.attemptId,
    })}>查看 Coding Workspace</button>
  ) : null}
  ```

  当用户尚未人工确认，标题不得称「已完成全部交付」；真正整组 all delivered 若需额外展示，只用后端现有 `PlanGroupOverall::AllDelivered`，本任务不新增第二通知。
- [ ] **Step 4: 验证绿灯。** 重跑 Step 2，加 `pnpm -C web exec vitest run src/components/cockpit/CockpitShell.test.tsx`；保证 info 不改角标、toast 同键一次、Coding Workspace 点击实际 navigate。
- [ ] **Step 5: 提交。** `git add web/src/api/types/lifecycle.ts web/src/hooks/useWorkspaceSessionObservers.ts web/src/hooks/useWorkspaceSessionObservers.test.tsx web/src/state/workspace-cockpit-projection.ts web/src/components/chat-workspace/cockpit/CockpitInbox.tsx web/src/pages/ChatCockpitPage.tsx web/src/pages/ChatWorkspacePage.tsx web/src/pages/ChatCockpitPage.inbox.test.tsx web/src/components/cockpit/CockpitShell.test.tsx web/src/router.tsx web/src/router.test.tsx && git commit -m "feat: show coding final-confirm info with attempt navigation"`。

## Task 10：3.4 P2 双清单关闸、部署真实链实证

**Files:** Modify `src/web/autopilot_orchestrator.rs` 的 `#[cfg(test)]` campaign、`src/web/coding_ws_handler/tests/{runner_recovery.rs,sc_start_guard.rs}`、`src/web/workspace_ws_handler/tests/plan_repair_activation.rs`、`web/src/pages/ChatCockpitPage.inbox.test.tsx`（仅缺的行为对照）；按实际验证结果回填 `openspec/changes/work-item-group-autopilot/tasks.md` §3.1–3.4 的验收证据及 `cadence/plans/2026-09-27_实施计划_WIGAutopilot_P2后台coding链_v1.0.md` 的关闸记录；未过不得勾。不得伪造真实 provider 结果。

**Interfaces:** 只消费 Task 0.1–9；不新增生产契约。替身 checklist 与真实链 checklist **分别**列样本 issue/plan/attempt、状态前后、provider start ledger、journal/claim/runner 数、REST 回执与 FinalConfirm 人工动作；前置 GAP-F/H 失败时不可拿 legacy/manual/Fake 替代 enrolled 真链。

- [ ] **Step 1: 写最后一条跨层行为测试。** 使用已有 `EnrolledGateFixture` + Task 4 Ready fixture（测试 helper 在 `src/web/wiga_gate_fixture.rs` 完整定义），无页面运行两轮 reconcile，停于 coding choice 和人工 FinalConfirm，对照非 enrolled issue：

  ```rust
  #[tokio::test]
  async fn p2_campaign_requires_human_final_confirm_after_socketless_run() {
      let fixture = p2_enrolled_campaign_fixture().await;
      fixture.confirm_plan_by_human().await;
      fixture.reconcile_until_coding_waiting_for_human().await;
      let attempt = fixture.attempt();
      assert_eq!(attempt.stage, CodingExecutionStage::FinalConfirm);
      assert_eq!(attempt.status, CodingAttemptStatus::WaitingForHuman);
      assert_eq!(fixture.runner_start_claims(&attempt.id), 1);
      assert_eq!(fixture.manual_issue_runner_start_claims(), 0);
      fixture.confirm_final_by_human().await;
      assert_eq!(fixture.attempt().status, CodingAttemptStatus::Completed);
      assert_eq!(fixture.runner_start_claims(&attempt.id), 1);
  }
  ```

  `p2_enrolled_campaign_fixture` 在此测试任务**完整实现**，串起现有 `EnrolledGateFixture` 人工 approve、Task 1 Ready、Task 4 StartCoding、fake provider 真实 runner、P0 coding choice waiter 与原 `handle_final_confirm`；控制器只能人手调用 confirm，不以直接文件 status mutation 冒充完成；非 enrolled issue 使用 `OrchestratorFixture` 的手工 issue fixture 对照。补 503 失败分诊与 Fake 完成两份不同证据，避免一条 happy path 覆盖多行同义断言。
- [ ] **Step 2: 验证红灯。** `cargo test --locked --lib p2_campaign_requires_human_final_confirm_after_socketless_run -- --nocapture`，先观察真实业务失败/缺条件（不能只因 helper 不存在称红灯）。
- [ ] **Step 3: 修最小跨层断点。** 只修本测试揭露的已有接口错接/顺序竞态；如揭露新范围或要修改 OpenSpec 的准入/人工门语义，停止并请主控先更新契约，再更新本计划。测试场景写入 campaign fixture 的真正 helper，例如：

  ```rust
  async fn reconcile_until_coding_waiting_for_human(&self) {
      tokio::time::timeout(std::time::Duration::from_secs(10), async {
          while self.attempt().status != CodingAttemptStatus::WaitingForHuman {
              self.worker().reconcile(&self.state, PROJECT_ID, ISSUE_ID).await.unwrap();
              tokio::task::yield_now().await;
          }
      }).await.expect("fake campaign reaches human FinalConfirm");
  }
  ```

  不从 Step 3 新增生产兜底模式，也不回退 GAP-F 静态白名单；真实链 server 部署用现有项目部署流程/`aria-dev-v48q`，记录发布产物 revision 与服务进程版本一致。
- [ ] **Step 4: 验证绿灯与双清单。** **替身**：定向运行本测试及 Tasks 0–9 所列测试；逐项截图/记录 Ready+精确授权、手工/自动同 attempt 并发、journal/barrier 中窗、disable/reopen、Failed/Aborted 和旧 attempt、多 target 无任何 sibling 自动 claim、零 socket choice 完整答案/真实 waiter 回执、amendment Unsent→真写 Delivered、零 socket resume、慢 observer 不反压、FinalConfirm 前一次通知与 0 计数/人工 Completed。**真实 provider/人工链（不可用 Fake 替代）**：部署本 worktree 同版产物至运行的 `aria-dev-v48q`，人工批准绑定 plan 后*不打开 coding 页*等后台独立 advance+首启；关闭全部 workspace/coding 订阅，实际覆盖一次 coding choice 的 REST 200/202→Delivered、一次人工确认 amendment→零 socket Unsent→重开真写 Delivered、一次重启后的已认领 run 续接；观察 FinalConfirm 等待时 info 出现/0 待处理增量，人手最终确认后 durable Completed；对照默认 off 和手工多 target；所有 provider 帐号/gateway 503 仅诊断+人手重驱。若环境仍 503 或 GAP-A/B/C/D/LC 阻断，逐条记阻断和已完成的替身证据，**不得勾 3.4 或声称真实链通过**。
- [ ] **Step 5: 提交关闸证据。** 只在上述双清单实证后勾 §3.1–3.4、记录精确命令/服务器版本/实际状态与日志引用：`git add openspec/changes/work-item-group-autopilot/tasks.md cadence/plans/2026-09-27_实施计划_WIGAutopilot_P2后台coding链_v1.0.md src/web/autopilot_orchestrator.rs src/web/coding_ws_handler/tests src/web/workspace_ws_handler/tests/plan_repair_activation.rs web/src/pages/ChatCockpitPage.inbox.test.tsx && git commit -m "test: gate WIG autopilot P2 fake and real chains"`；真实链未通过时仅提交仍未勾选的测试/失败证据，不伪造关闸状态。

---

## 需求追溯与自审（计划阶段）

| 权威条目 | 负责任务与可观察证据 |
|---|---|
| §3.1 / REQ-WIGA-02、REQ-ADV-05 / D4 | Task 1 独立 advance、Task 2 持久 opt-in、Task 3/4 共用首启准入/单发、Task 5 WS+编排器同一服务；Ready 前同码、disable 两种竞争、Failed/Aborted 不隐式 restart。 |
| §3.1 / REQ-WIGA-03、REQ-WIGA-04 / REQ-MTG-03 | Task 1/3/4/5 精确 plan/source/revision/target/唯一 attempt，不批量 fan-out；Task 10 与手工多 target/默认 off 对照。 |
| §3.2 / REQ-WIGA-03、REQ-WIGA-04、REQ-WIGA-06 | Task 4 journal/barrier 中窗、Task 6 startup 无 attach 恢复、Task 7 无 socket choice+已完成 amendment 解耦、慢 observer 真实 ack；Task 10 重启真实链。 |
| §3.3 / REQ-WIGA-07、REQ-WIGA-08 / R5 / D5/D6 | Task 8 完整 readiness+稳定 pending 节点事件、Task 9 info 去重/计数隔离/Coding Workspace 下钻；人工 `handle_final_confirm` 才 Completed，Task 10 真链人手按键。 |
| §3.4 双清单 | Task 10 不把替身冒充真实 provider；明确部署同版至 `aria-dev-v48q`，环境失败不得关闸。 |
| GAP-E/G/F/H 前置 | Task 0.1 reviewer Evaluate→Failed 与人工明确重驱，Task 0.2 GET+PUT 同源静态 gateway 检查，Task 0.3 503 durable 诊断+驾驶舱可见且无自动重试。 |
| 不可提前实现 P3 | 当前 watched info 可读/提醒，不规划 K 外补读/TTL/跨刷新策略；§4 工作包保持原样。 |

**自审：**合计 **13 个**任务（Task 0.1–0.3 + Task 1–10），每项均有精确文件/邻接接口、真实行为断言的 Step 1、定向红灯命令、实现的 Step 3、定向绿灯命令与独立 commit；上面五项 Review Focus 分别落具体行为测试。已核对源码的复用点：`advance.rs::handle_advance`、`coding_run_registry.rs::try_reserve_attempt/lock_attempt`、`runner.rs::spawn_coding_runner_reserved` 的既有 start barrier、`socket/resumption.rs::ensure_runner_for_resumed_attempt`、`group_final_readiness.rs::write_group_final_readiness_snapshot` 与 `timeline.rs::create_pending_final_confirm_timeline_node`、P1 `plan_confirmed_info` 投影链及 P0 choice/amendment 契约。新符号只在负责 Task 首次定义；任务间测试 helper 明确要求由所属任务新增，计划中的片段是未来 TDD 目标，**未运行测试、未实施源码、未部署真实链**。设计开放点：如果现有 manager 不能安全承载 Task 0.1 所需的显式恢复认领，或 attempt/enrollment 双锁现有锁序与 plan advance 有不可消解循环，必须先按 OpenSpec 流程调整设计，不能用裸调 runner/自动重试代替。
