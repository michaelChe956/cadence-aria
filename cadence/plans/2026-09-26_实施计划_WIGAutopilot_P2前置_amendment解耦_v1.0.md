# WIG Autopilot P2 前置（线 B）amendment 解除 live socket 承重 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 交付 tasks.md 3.2 中「解除 amendment 业务应用/恢复对 live coding socket/事件写回执的承重依赖」的前置拆解（REQ-WIGA-06）：业务 amendment 应用/恢复（journal→finalize→resume）在零 coding socket 时照常推进到 durable 检查点；观察投递独立记录真实未送达事实（Unsent/Pending），socket 重连或重复确认后按 durable 事实补投递，只有真实 socket 写 ack 才标 Delivered。本计划**不**实现 3.2 的其余部分（无人打开 Coding Workspace 的首启、已认领 run 恢复的 journal/barrier 中窗）——那些属线 A/C；本计划完成后 `tasks.md` 3.2 **不勾选**。

**Architecture:** `CodingWorkspaceEngine` 是 Clone 的（`src/product/coding_workspace_engine/types.rs:293-305`）。业务路径 `apply_plan_amendment_from_journal` 在 finalize→resume→context 收口后仅**spawn 一个 detached 观察任务**，不再 await 投递；观察单次尝试（register delivery ack → 经 hub 发 `PlanAmendmentUpdated` → 等真实写回执/超时/取消）以 durable delivery 记录收口：ack=Delivered、其余=Unsent，绝不反写业务错误。激活面 `activate_published_plan_amendment` 改用 `CodingSocketRegistry::hub_sender`（零 socket 也建 hub；`broadcast` 零 fan-out 由 `expect_plan_amendment_fan_out_writes(event, 0)` 立即结算失败→Unsent）。补投递触发器两个：coding socket attach（`socket.rs` 初帧后）与重复确认激活的 early-return 分支，共用 `delivery_ack.rs` 的同一 helper。投递去重沿用现有 `delivery_ack` 全局注册表（`register_plan_amendment_socket_write` 的 event_id 互斥即并发抑制）+ 稳定 event_id（`delivery_event_id`，确定性公式）保证同 event 恰一次。

**Tech Stack:** Rust、Tokio（mpsc/oneshot/CancellationToken/timeout）、serde（snake_case enum）、文件持久层（`with_exclusive_lock` + 原子 JSON）；无前端改动。

**Spec:** `openspec/changes/work-item-group-autopilot/specs/work-item-group-autopilot/spec.md` REQ-WIGA-06（L114-126）；`design.md`（L7、L89「无 coding socket 的 amendment/断连恢复」行、Risks #2）；`tasks.md` 3.2（L22）。本计划接口为**拟新增/拟修改契约**，不是声称当前代码已有。

## Global Constraints

- **REQ-WIGA-06 原文关键句**：「业务 amendment 应用/恢复 SHALL 在无 live coding socket 时继续推进；观察事件送达 SHALL 独立记录真实投递事实并允许重连后按 durable 事实补读，MUST NOT 因没有订阅者而假写 Delivered，也不得以假 socket 代替真实交付」；「不以 `plan_amendment_coding_socket_unavailable` 拦截业务推进」；「投递标记反映实际交付，不重复应用 amendment、不因观察端反压改写业务终态」。
- **边界 1（业务/观察分离）**：`apply_plan_amendment_from_journal`（`src/product/coding_workspace_engine/amendment.rs:419-430`）现有顺序 finalize→`reconcile_plan_amendment_delivery`(await，可失败业务)→resume 必须改为 finalize→resume→context 收口→**detached 观察发射**。业务层（journal/apply/finalize/resume）的推进不得被 delivery 状态阻塞或失败；delivery 记录语义改为「已应用、观察投递未完成」（Pending/Unsent）与「真实送达」（Delivered）。
- **边界 2（零 socket 不拒绝）**：删除 `src/web/workspace_ws_handler/plan_repair_activation.rs:22-31` 的 `hub_sender_if_live`→`plan_amendment_coding_socket_unavailable` 拒绝路径，改用 `hub_sender`。零 socket 时写 durable 未送达事实（Unsent），重连后补投递、**真实 ack 才标 Delivered**。禁止伪造 Delivered、禁止安装假 socket 消费者（不创建任何「假装收到的 sink」，投递判定只来自 `delivery_ack` 真实写结算）。
- **边界 3（不复用 ChoiceDeliverySignal）**：P0 的 `ChoiceDeliverySignal` 是 provider choice waiter 回执，语义不同，**禁止复用**。本计划复用既有 amendment 族 durable delivery 标记（`CodingPlanAmendmentDelivery` + `delivery_ack.rs` socket 写结算注册表）。
- **边界 4（fanout=0 与 hub owner/清理）**：`src/web/state/coding_socket_registry.rs:183-214` `broadcast` 的「零 fan-out 立即结算失败」语义保留——但消费方从「业务失败」改为「观察层记 Unsent」。hub 生命周期沿用现有规则（registry 仅在存活 socket 期间持保活引用，`coding_socket_registry.rs:62-75`；runner/engine 的 event_tx clone 维持 hub 存活，`hub_channel` L131-147）；零 socket 激活后若 runner 退出且从未有 socket attach/detach，registry 的 hub entry 与空转路由任务按 attempt 有界滞留（无 CPU 消耗、无正确性影响），在激活点以注释显式记录，不引入新的回收机制。慢 observer 不得反压业务：业务路径不得 await 观察投递（含 hub 容量满场景）。
- **边界 5（文件域互斥）**：只动 amendment 族（`amendment.rs`/`amendment_delivery.rs`/`coding_models/plan_repair.rs` 的 delivery 段/`delivery_ack.rs`/`plan_repair_activation.rs`/`coding_socket_registry.rs`/`socket.rs` attach 钩子/`event_hub.rs`）+ 调用点测试最小适配（`decisions.rs` 不改、`runner*.rs` 不改、`runner_support.rs` 不改）。**不得改** `src/web/{handlers,app.rs,error.rs,types.rs,workspace_session}` 与 `web/src`（其他 worker 域）。引擎层测试放 `src/product/coding_workspace_engine/tests/plan_amendment/` 既有文件；`tests/it_web/web_work_item_plan_repair/part_04.rs` 仅做断言轮询化最小适配。
- 验证从 worktree 根运行：Rust 定向 `cargo test --locked --lib <filter> -- --nocapture`、集成 `cargo test --locked --test it_web web_work_item_plan_repair -- --nocapture`；逐任务红→绿；禁止 `cargo test -j 1`；全量由集成负责人最后统一跑。仅本计划文件在本次派工落盘，**本文未实施任何代码**。

## Review Focus

1. **zero socket 激活推进**：不连接任何 coding WS，workspace 侧 confirm 后 attempt 到 Running、runner 唯一、delivery=Unsent（`delivered_at=None`）——任务 3 `zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery`。
2. **慢 observer 不改业务终态**：事件通道容量占满/无消费者时 `apply_plan_amendment` 仍在有界时间内返回 Running，marker 停 Pending/Unsent——任务 2 `coding_amendment_delivery_slow_observer_never_backpressures_business`。
3. **重连补投递幂等**：同 `event_id`（`delivery_event_id` 确定性公式）恰一次补投；已 Delivered 再触发返回 0、不重发；并发第二路因注册表互斥跳过——任务 3 `coding_amendment_redelivery_reuses_event_until_real_ack` + 任务 4 零 socket 用例的 late-connect 扩展段。
4. **Pending 不阻塞业务**：投递等待 ack 期间（marker=Pending）业务已 resume Running；通道关闭/socket 写失败同样不产生业务错误——任务 2 `coding_amendment_delivery_pending_marker_does_not_block_business_resume` 等 3 个变体。
5. **伪造 Delivered 反例**：零订阅者/fan-out=0 永不 Delivered 且 `delivered_at=None`（任务 2/3）；Unsent 携带 `delivered_at` 的记录被 store 拒绝、Delivered 不可被 Unsent 降级（任务 1 store 守卫用例）；零 fan-out 由 `expect_plan_amendment_fan_out_writes(event, 0)` 立即结算失败（任务 3 `hub_zero_fanout_settles_amendment_waiter_unsent`）。

---

## 文件责任与统一接口

现有锚点（勘察结论，行号已核对）：

- 引擎：`amendment.rs` — `apply_plan_amendment` L23-37、`apply_plan_amendment_locked` L39-86、`recover_plan_amendment` L88-95、`resume_group_after_plan_amendment` L106-198、`recover_plan_amendment_with_history_session` L234-264、`current_amendment_journal_id` L266-302（其中 `delivered_current` L277-284 **不改**）、`apply_plan_amendment_from_journal` L304-441、`validate_amendment_application_identity` L443-520、`finalize_completed_amendment_application` L621-642、`reconcile_plan_amendment_delivery` L644-677、`record_amendment_application_failure` L784。
- 模型：`src/product/coding_models/plan_repair.rs` — `CodingAmendmentApplicationJournal` L93-103、`CodingPlanAmendmentDeliveryStatus` L105-110、`CodingPlanAmendmentDelivery` L112-122。
- Store：`src/product/coding_attempt_store/amendment_delivery.rs` — `load_or_prepare_plan_amendment_delivery` L31-64、`get_plan_amendment_delivery` L66-82、`mark_plan_amendment_delivery_delivered` L84-117、failpoint 助手 L120-175、`validate_delivery` L177-202、`delivery_id` L204-206、`delivery_event_id` L208-210；路径 `src/product/coding_attempt_store/paths.rs:291-301`（`<attempt_dir>/amendment-event-deliveries/<amendment_id>.json`）。
- Web：`plan_repair_activation.rs` L8-49（拒绝路径 L22-31）；调用方 `src/web/workspace_ws_handler/decisions.rs:26-38`（错误仍映射 `PLAN_AMENDMENT_ACTIVATION_FAILED`，仅真实保留错误如 reservation，**不改**）；runner 恢复入口 `src/web/coding_ws_handler/runner.rs:250-262`→`runner_support.rs:22-38`（**不改**，自动受益）；`src/web/coding_ws_handler/runner/amendment.rs:11-40` `spawn_plan_amendment_runner_reserved`（**不改**）；registry `src/web/state/coding_socket_registry.rs`（`hub_sender` L77-89、`hub_sender_if_live` L91-108、`broadcast` L183-214）；`src/web/coding_ws_handler/delivery_ack.rs`（register L29-54、`wait` L56-69、`wait_or_channel_closed` L71-83、Drop 清理 L86-98、`expect_plan_amendment_fan_out_writes` L108-131）；socket attach `src/web/coding_ws_handler/socket.rs:67-166`（register+hub L97-104、快照 L105-111、choice 补帧 L112-120、主循环 L167-176、清理 L1004）。
- 既有测试锚点：引擎 `src/product/coding_workspace_engine/tests.rs:551`→`tests/plan_amendment.rs:24` `mod review_fix_delivery`；fixture `tests/plan_amendment.rs:507-870`（ack 转发消费者 L845-856）；web 激活 `src/web/workspace_ws_handler/tests/plan_repair_activation.rs:40-160`；registry `src/web/coding_ws_handler/tests/event_hub.rs`（if_live 用例 L74-91）；矩阵 `src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix/amendment_row.rs`（终值断言 L82-86、窗口 4 L565-675、引擎助手 L677-705）；集成 `tests/it_web.rs:191-192`→`web_work_item_plan_repair/part_04.rs`（L113-145、L358-392）。

全任务沿用如下**唯一**新契约，不另造同义类型；带「新增/修改」的符号由相应任务落地：

```rust
// 修改于 src/product/coding_models/plan_repair.rs:105-110（serde snake_case）。
/// Pending=已应用、观察投递未完成（含等待 ack / 尚未结算）；
/// Unsent=一次投递尝试以无存活订阅者/写失败/通道关闭/超时收场，等待补投递；
/// Delivered=真实 socket 写 ack（唯一写 delivered_at 的路径）。
pub enum CodingPlanAmendmentDeliveryStatus { Pending, Unsent, Delivered }
```

```rust
// 新增于 src/product/coding_attempt_store/amendment_delivery.rs（impl CodingAttemptStore 内，
// 与 mark_plan_amendment_delivery_delivered L84-117 同款锁与校验风格）：
/// 投递尝试失败收口：Pending|Unsent -> Unsent（幂等）；已 Delivered 原样返回，
/// 绝不降级、绝不写 delivered_at。event_id 不匹配 => IdentityMismatch。
pub fn mark_plan_amendment_delivery_unsent(
    &self,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
    amendment_id: &str,
    event_id: &str,
) -> Result<CodingPlanAmendmentDelivery, ProductStoreError>

/// 列出该 attempt 目录下全部 delivery 记录（attempt 身份不符的文件 => IdentityMismatch
/// fail-closed；目录缺失 => 空表）；按 created_at、event_id 稳定排序。
pub fn list_plan_amendment_deliveries(
    &self,
    attempt: &crate::product::coding_models::CodingExecutionAttempt,
) -> Result<Vec<CodingPlanAmendmentDelivery>, ProductStoreError>
```

```rust
// 新增于 src/product/coding_workspace_engine/amendment.rs（替换 L644-677 的
// reconcile_plan_amendment_delivery；原函数删除，不保留旧名）：
pub(crate) const PLAN_AMENDMENT_DELIVERY_ACK_TIMEOUT: std::time::Duration =
    std::time::Duration::from_secs(30);

impl CodingWorkspaceEngine {
    /// 观察层单次投递尝试（REQ-WIGA-06）：load_or_prepare -> 已 Delivered 早退 ->
    /// register（他人持有注册 => 跳过发送返回 durable 现状）-> 经 hub 发
    /// PlanAmendmentUpdated -> 等真实写回执（timeout=PLAN_AMENDMENT_DELIVERY_ACK_TIMEOUT，
    /// 经 hub 发送的 send 阶段本身也 MUST 受同一超时约束（k3 审查 F1：
    /// types.rs:250-263 的 reserve() 会无限等待且 transient 引擎取消永不触发，
    /// 注册互斥会让后续补投递被永久跳过）——send 与 ack 等待包在同一
    /// tokio::time::timeout(PLAN_AMENDMENT_DELIVERY_ACK_TIMEOUT) 内，超时 => 释放注册并 mark Unsent。
    /// 超时/写失败/通道关闭 => 释放注册并 mark Unsent；ack Ok => mark Delivered。
    /// 引擎取消 => 不改 marker 返回现状。绝不返回业务错误、绝不伪造 Delivered。
    pub(crate) async fn deliver_plan_amendment_observation_once(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    ) -> Result<CodingPlanAmendmentDeliveryStatus, ProductStoreError>

    /// 业务路径唯一的观察发射点：detached tokio::spawn 包 once；失败仅 tracing::warn。
    pub(crate) fn spawn_plan_amendment_delivery_observation(
        &self,
        attempt: &CodingExecutionAttempt,
        manifest: &PlanAmendmentManifest,
    )

    /// 重连/重复确认补投递：list deliveries -> 跳过 Delivered -> 取 manifest
    /// （无 manifest 的孤儿记录跳过；plan.active_revision_id != manifest.
    /// new_plan_revision_id 的非当前修订跳过）-> once。返回本次真实达成 Delivered 的条数。
    pub(crate) async fn redeliver_undelivered_plan_amendments(
        &self,
        attempt: &CodingExecutionAttempt,
    ) -> Result<usize, ProductStoreError>
}
```

```rust
// 新增于 src/web/coding_ws_handler/delivery_ack.rs（amendment 族统一补投递触发器；
// socket attach 与激活 early-return 共用，禁止各自内联第二份）：
pub(crate) fn spawn_undelivered_amendment_redelivery(
    coding_store: crate::product::coding_attempt_store::CodingAttemptStore,
    attempt: crate::product::coding_models::CodingExecutionAttempt,
    event_tx: mpsc::Sender<CodingWsOutMessage>,
)
// 体内：以 CodingWorkspaceEngine::new(coding_store, GitWorkspaceService::new(), event_tx)
// 构造 transient 引擎并 tokio::spawn 调 redeliver_undelivered_plan_amendments，
// Err 仅 tracing::warn!(%error, "plan_amendment_delivery_redelivery_failed")。
```

并发与幂等总则：投递互斥由 `register_plan_amendment_socket_write` 的 event_id 全局注册表承担（重复注册 => `IdentityMismatch` => 调用方视为「他路在投」跳过）；`wait_or_channel_closed` 的 waiter 被 timeout 取消时经 `Drop`（`delivery_ack.rs:86-98`）清理注册，后续可重试；`mark_*_delivered/unsent` 均在 `with_exclusive_lock` 内重读校验，Delivered 终态单向。

## Task 1：delivery 状态模型与 store 语义扩展（Unsent/list/mark_unsent）

**Files:** Modify `src/product/coding_models/plan_repair.rs:105-110`（enum + 注释）；`src/product/coding_attempt_store/amendment_delivery.rs`（`mark_plan_amendment_delivery_delivered` L84-117 保持允许任意非 Delivered 源态→Delivered；新增 `mark_plan_amendment_delivery_unsent`/`list_plan_amendment_deliveries` 于 impl L30-118；`validate_delivery` L177-202 扩展）。Test `src/product/coding_workspace_engine/tests/plan_amendment/review_fix_delivery.rs`（文件尾部新增 store 用例段）。

**Interfaces:** Consumes: `amendment_fixture()`（`tests/plan_amendment.rs:507-870`）、`load_or_prepare_plan_amendment_delivery`、`mark_plan_amendment_delivery_delivered`、`with_exclusive_lock`、`attempt_dir`。Produces: 统一接口块第一、二段符号。旧调用路径本轮不变更语义（现有写入仍只有 Pending/Delivered 两态）。

- [ ] **Step 1: 写失败测试。** 在 `review_fix_delivery.rs` 尾部新增（沿用该文件既有的 marker_path 直读模式 L115-127）：
  ```rust
  #[tokio::test]
  async fn coding_amendment_delivery_store_unsent_never_fakes_delivered() {
      let fixture = amendment_fixture().await;
      let attempt = fixture.attempt.clone();
      let seeded = fixture
          .store
          .load_or_prepare_plan_amendment_delivery(&attempt, &fixture.manifest.id)
          .unwrap();
      assert_eq!(
          seeded.status,
          crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending
      );

      let unsent = fixture
          .store
          .mark_plan_amendment_delivery_unsent(&attempt, &fixture.manifest.id, &seeded.event_id)
          .unwrap();
      assert_eq!(
          unsent.status,
          crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent
      );
      assert_eq!(unsent.delivered_at, None);
      // 幂等：重复 Unsent 不改写。
      let again = fixture
          .store
          .mark_plan_amendment_delivery_unsent(&attempt, &fixture.manifest.id, &seeded.event_id)
          .unwrap();
      assert_eq!(
          again.status,
          crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent
      );

      // 真实 ack 后不可降级：Delivered 之上 Unsent 必须原样返回。
      let delivered = fixture
          .store
          .mark_plan_amendment_delivery_delivered(&attempt, &fixture.manifest.id, &seeded.event_id)
          .unwrap();
      assert_eq!(
          delivered.status,
          crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
      );
      let guarded = fixture
          .store
          .mark_plan_amendment_delivery_unsent(&attempt, &fixture.manifest.id, &seeded.event_id)
          .unwrap();
      assert_eq!(
          guarded.status,
          crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
      );
      assert!(guarded.delivered_at.is_some());

      // 异 event_id => IdentityMismatch。
      let error = fixture
          .store
          .mark_plan_amendment_delivery_unsent(&attempt, &fixture.manifest.id, "other_event")
          .unwrap_err();
      assert!(matches!(
          error,
          crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
      ));
  }

  #[tokio::test]
  async fn coding_amendment_delivery_store_lists_own_deliveries_and_rejects_foreign() {
      let fixture = amendment_fixture().await;
      let attempt = fixture.attempt.clone();
      fixture
          .store
          .load_or_prepare_plan_amendment_delivery(&attempt, &fixture.manifest.id)
          .unwrap();
      let listed = fixture.store.list_plan_amendment_deliveries(&attempt).unwrap();
      assert_eq!(listed.len(), 1);
      assert_eq!(listed[0].amendment_id, fixture.manifest.id);
      assert_eq!(
          listed[0].event_id,
          format!(
              "coding_plan_amendment_updated_{}_{}",
              attempt.id, fixture.manifest.id
          )
      );

      // 异 attempt 身份的文件必须 fail-closed（不静默跳过）。
      let delivery_dir = fixture
          .store
          .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
          .join("amendment-event-deliveries");
      let foreign = serde_json::json!({
          "id": "coding_plan_amendment_delivery_foreign_amendment_x",
          "event_id": "coding_plan_amendment_updated_foreign_amendment_x",
          "attempt_id": "attempt_other",
          "amendment_id": "amendment_x",
          "status": "pending",
          "delivered_at": null,
          "created_at": "2026-09-26T00:00:00Z",
          "updated_at": "2026-09-26T00:00:00Z",
      });
      std::fs::write(
          delivery_dir.join("amendment_x.json"),
          serde_json::to_vec(&foreign).unwrap(),
      )
      .unwrap();
      assert!(matches!(
          fixture.store.list_plan_amendment_deliveries(&attempt).unwrap_err(),
          crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
      ));
  }

  #[tokio::test]
  async fn coding_amendment_delivery_store_rejects_unsent_with_delivered_at() {
      let fixture = amendment_fixture().await;
      let attempt = fixture.attempt.clone();
      fixture
          .store
          .load_or_prepare_plan_amendment_delivery(&attempt, &fixture.manifest.id)
          .unwrap();
      let marker_path = fixture
          .store
          .attempt_dir(&attempt.project_id, &attempt.issue_id, &attempt.id)
          .join("amendment-event-deliveries")
          .join(format!("{}.json", fixture.manifest.id));
      let mut marker: serde_json::Value =
          serde_json::from_slice(&std::fs::read(&marker_path).unwrap()).unwrap();
      marker["status"] = "unsent".into();
      marker["delivered_at"] = "2026-09-26T00:00:00Z".into();
      std::fs::write(&marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
      assert!(matches!(
          fixture
              .store
              .get_plan_amendment_delivery(&attempt, &fixture.manifest.id)
              .unwrap_err(),
          crate::product::json_store::ProductStoreError::IdentityMismatch { .. }
      ));
  }
  ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib coding_amendment_delivery_store -- --nocapture`；预期 `Unsent`/`mark_plan_amendment_delivery_unsent`/`list_plan_amendment_deliveries` 未定义、断言失败。
- [ ] **Step 3: 最小实现。** ① `plan_repair.rs:105-110` 加 `Unsent` 变体与语义注释（统一接口块逐字）；② `amendment_delivery.rs` 在 `mark_plan_amendment_delivery_delivered` 后新增 `mark_plan_amendment_delivery_unsent`：`with_exclusive_lock(&path, ...)` 内 `read_json`→`validate_delivery`→event_id 比对→已 `Delivered` 原样返回→否则置 `Unsent`、`updated_at=now`（`delivered_at` 保持 `None`）、`write_json`；③ 新增 `list_plan_amendment_deliveries`：`validate_attempt_lineage`→目录不存在返回空 `Vec`、存在则逐文件 `read_json`+`validate_delivery`（任一不符即 `IdentityMismatch`）→按 `created_at`/`event_id` 排序返回；④ `validate_delivery` L194-197 状态守卫改为：`delivered_at.is_some() ⟺ status == Delivered`（即 `Pending|Unsent` 携带 `delivered_at` 或 `Delivered` 缺 `delivered_at` 均拒）。示意核心：
  ```rust
  if /* ...既有身份比对不变... */ ||
      (delivery.delivered_at.is_some()
          != (delivery.status == CodingPlanAmendmentDeliveryStatus::Delivered))
  {
      return Err(identity_mismatch(amendment_id));
  }
  ```
- [ ] **Step 4: 绿灯。** 同一过滤命令；再跑 `cargo test --locked --lib coding_amendment_delivery -- --nocapture` 确认既有 8 个用例零回归（旧路径仍只写 Pending/Delivered）。
- [ ] **Step 5: 提交。** `git add src/product/coding_models/plan_repair.rs src/product/coding_attempt_store/amendment_delivery.rs src/product/coding_workspace_engine/tests/plan_amendment/review_fix_delivery.rs && git commit -m "feat: extend amendment delivery marker with unsent facts"`。

## Task 2：引擎业务/观察解耦（apply 顺序翻转 + detached 观察任务）

**Files:** Modify `src/product/coding_workspace_engine/amendment.rs:419-441`（顺序翻转+spawn）、`644-677`（`reconcile_plan_amendment_delivery` 重写为 `deliver_plan_amendment_observation_once` + 新增 `spawn_plan_amendment_delivery_observation` 与 `PLAN_AMENDMENT_DELIVERY_ACK_TIMEOUT`）。Test `src/product/coding_workspace_engine/tests/plan_amendment/review_fix_delivery.rs:51-567`（既有 8 用例全量 re-pin + 新 helper + 慢观察者用例）。

**Interfaces:** Consumes: Task 1 全部符号、`register_plan_amendment_socket_write`/`wait_or_channel_closed`（`delivery_ack.rs:29-83`）、`CancellableCodingEventSender::{send, raw_sender}`（`types.rs:231-273`）、`self.cancellation`。Produces: 统一接口块第三段的 `deliver_plan_amendment_observation_once`/`spawn_plan_amendment_delivery_observation`/常量。删除 `reconcile_plan_amendment_delivery`（唯一调用点即本任务改写）。`current_amendment_journal_id` L266-302 与 `record_amendment_application_failure` L784 **不动**（前者仅在读到 Delivered 时把 Completed journal 视作可幂等重放，语义不变；后者从不再收到投递类错误，自然只记真实业务失败）。

- [ ] **Step 1: 重写失败测试。** 对 `review_fix_delivery.rs` 做如下 re-pin（文件头部 import 增补 `use crate::product::coding_attempt_store::CodingAttemptStore;`，如缺）：
  - 新 helper（放 L512 `plan_amendment_event_id` 旁）：
    ```rust
    async fn poll_delivery_status(
        store: &CodingAttemptStore,
        attempt: &crate::product::coding_models::CodingExecutionAttempt,
        amendment_id: &str,
        expected: crate::product::coding_models::CodingPlanAmendmentDeliveryStatus,
    ) -> crate::product::coding_models::CodingPlanAmendmentDelivery {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                if let Ok(delivery) = store.get_plan_amendment_delivery(attempt, amendment_id)
                    && delivery.status == expected
                {
                    return delivery;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap_or_else(|_| panic!("delivery status did not reach {expected:?}"))
    }
    ```
  - L51-87 → `coding_amendment_delivery_pending_marker_does_not_block_business_resume`：`mpsc::channel(8)` **直接 await** `apply_plan_amendment`（无 spawn 也不回执），断言返回 `Ok` 且 `resumed.status == Running`；随后 `timeout(2s, event_rx.recv())` 收到 `PlanAmendmentUpdated` 证明观察事件照常入队；最后 `poll_delivery_status(..., Pending)`（本用例无人回执，marker 停 Pending）。
  - L89-128 → `coding_amendment_delivery_no_subscriber_marks_unsent_and_resumes`：`mpsc::channel(1)` 后 `drop(event_rx)`；`apply_plan_amendment` 断言 `Ok`+`Running`（替换原 `expect_err` 与 `AmendmentApplyFailed` 断言）；`poll_delivery_status(..., Unsent)` 且 `delivered_at == None`；marker 文件直读断言 `marker["status"] == "unsent"`（保留原 L115-127 模式）。
  - L130-181 → `coding_amendment_delivery_socket_write_failure_marks_unsent_and_resumes`：spawn apply→recv 事件→`fail_plan_amendment_socket_write(&event)`→`apply.await` 断言 `Ok`+`Running`（替换原 `expect_err("plan_amendment_socket_write_failed")`/`AmendmentApplyFailed`）→`poll_delivery_status(..., Unsent)`。
  - L183-231（writer abort）、L233-274（receiver drop）、L276-329（outstanding permit）三个用例合并 re-pin 为 `coding_amendment_delivery_observation_once_reuses_event_and_cleans_registration`：
    ```rust
    #[tokio::test]
    async fn coding_amendment_delivery_observation_once_reuses_event_and_cleans_registration() {
        let fixture = amendment_fixture().await;
        let (event_tx, event_rx) = mpsc::channel(8);
        drop(event_rx); // 模拟驱动 socket 断开：订阅者消失
        let engine =
            CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
        // 业务先行收口：不被投递通道关闭阻塞或失败。
        let resumed = engine
            .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
            .await
            .expect("receiver drop must not fail business");
        assert_eq!(resumed.status, CodingAttemptStatus::Running);
        let unsent = poll_delivery_status(
            &fixture.store,
            &resumed,
            &fixture.manifest.id,
            crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent,
        )
        .await;

        // 失败收口必须清理 ack 注册：同 event 可再次注册（供补投递重试）。
        let retry = register_plan_amendment_socket_write(&unsent.event_id)
            .expect("failed channel wait must remove the stale ACK registration");
        drop(retry);

        // 补投递（once 直调）：真实回执才 Delivered，且重发同一 event_id。
        let (reconnect_tx, mut reconnect_rx) = mpsc::channel(8);
        let confirm_loop = tokio::spawn(async move {
            while let Some(event) = reconnect_rx.recv().await {
                crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                    &event,
                );
            }
        });
        let reconnect_engine = CodingWorkspaceEngine::new(
            fixture.store.clone(),
            GitWorkspaceService::new(),
            reconnect_tx,
        );
        let status = reconnect_engine
            .deliver_plan_amendment_observation_once(&resumed, &fixture.manifest)
            .await
            .unwrap();
        assert_eq!(
            status,
            crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered
        );
        confirm_loop.abort();
        let delivered = fixture
            .store
            .get_plan_amendment_delivery(&resumed, &fixture.manifest.id)
            .unwrap();
        assert_eq!(delivered.event_id, unsent.event_id);
        assert!(delivered.delivered_at.is_some());
    }
    ```
  - L331-426（mark failpoint）→ `coding_amendment_delivery_retries_same_event_after_send_before_mark_failure` re-pin：首轮 apply 断言 `Ok`+`Running`（替换 `expect_err("delivery_mark_failpoint")` 与 `AmendmentApplyFailed`）；确认事件后 marker 停 `Pending`（failpoint 拦截 Delivered 落盘）；`drop(failpoint)` 后改调 `reconnect_engine.deliver_plan_amendment_observation_once(...)`（带 confirm 消费循环）断言 `Delivered` 且 `second_event_id == first_event_id`（保留原事件身份断言 L352-361/L399-417 结构）。
  - L428-510（并发恢复）→ `coding_amendment_concurrent_recovery_reconciles_one_durable_delivery` re-pin：并发两个 `apply_plan_amendment`（替换原 `recover_plan_amendment`；种子步骤改用 apply+failpoint 造 Pending）双双 `Ok`+`Running`；`select!` 收到**恰一路**事件后 confirm；断言最终 marker `Delivered`、`left/right_event_rx.try_recv().is_err()`（注册互斥使败者不发送）保留。
  - 新增（Review Focus 2）：
    ```rust
    #[tokio::test]
    async fn coding_amendment_delivery_slow_observer_never_backpressures_business() {
        let fixture = amendment_fixture().await;
        let (event_tx, _event_rx) = mpsc::channel(1);
        // 占满唯一槽位：观察投递的 send 将长时间阻塞在容量上。
        let _outstanding_permit = event_tx.clone().reserve_owned().await.unwrap();
        let engine =
            CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
        let resumed = tokio::time::timeout(std::time::Duration::from_millis(500), async {
            engine
                .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
                .await
        })
        .await
        .expect("slow observer must not backpressure business")
        .expect("apply must succeed");
        assert_eq!(resumed.status, CodingAttemptStatus::Running);
        poll_delivery_status(
            &fixture.store,
            &resumed,
            &fixture.manifest.id,
            crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Pending,
        )
        .await;
    }
    ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib coding_amendment_delivery -- --nocapture`；预期新语义断言（apply Ok/Running、Unsent、`deliver_plan_amendment_observation_once` 未定义）失败。
- [ ] **Step 3: 最小实现。** ① `amendment.rs` L644-677 重写为统一接口块的 `PLAN_AMENDMENT_DELIVERY_ACK_TIMEOUT` + `deliver_plan_amendment_observation_once` + `spawn_plan_amendment_delivery_observation`（骨架逐字按接口块；`Ok(Some(Ok(())))→mark_delivered`、`Ok(Some(Err(_)))|Err(_)(超时)→mark_unsent`、`Ok(None)(取消)→返回 load 时的现状`；`register` 失败按「他路在投」跳过发送返回现状）；② L419-441 顺序翻转：
  ```rust
  self.finalize_completed_amendment_application(attempt, manifest, &authority.plan, &authority.request)?;
  let resumed = self
      .store
      .resume_attempt_after_amendment(attempt, manifest)
      .map_err(CodingWorkspaceEngineError::from)?;
  if let Some(context) = amendment_context {
      let completed = self.store.complete_plan_amendment_context(
          &resumed, &context.id, &manifest.new_plan_revision_id, &manifest.resume_target,
      )?;
      self.close_reopened_amendment_gate_if_idle(&completed.plan_session_id)?;
  }
  // REQ-WIGA-06：投递是观察面——业务先达 durable 检查点，真实未送达事实
  // 留给重连/重复确认补投递；不得以投递状态阻塞或失败业务。
  self.spawn_plan_amendment_delivery_observation(attempt, manifest);
  Ok(resumed)
  ```
- [ ] **Step 4: 绿灯。** 同一过滤命令（含 Task 1 store 用例）；注意本任务提交后 `src/web/workspace_ws_handler/tests/plan_repair_activation.rs` 与 campaign 矩阵/it_web 的旧语义用例预期转红，由 Task 3/5 收口——**不得**为使其变绿回退本任务语义。
- [ ] **Step 5: 提交。** `git add src/product/coding_workspace_engine/amendment.rs src/product/coding_workspace_engine/tests/plan_amendment/review_fix_delivery.rs && git commit -m "feat: decouple amendment business resume from delivery acknowledgement"`。

## Task 3：激活解除 live socket 承重 + engine 补投递 + 重复确认触发

**Files:** Modify `src/web/workspace_ws_handler/plan_repair_activation.rs:8-49`（`hub_sender` 替换 L22-31 + early-return 补投递 L19-21 + 注释）；`src/web/state/coding_socket_registry.rs:91-108`（删除 `hub_sender_if_live`）；`src/web/coding_ws_handler/delivery_ack.rs`（新增 `spawn_undelivered_amendment_redelivery`）；`src/product/coding_workspace_engine/amendment.rs`（新增 `redeliver_undelivered_plan_amendments`）。Test `src/product/coding_workspace_engine/tests/plan_amendment/review_fix_delivery.rs`（尾部 redelivery 用例）；`src/web/workspace_ws_handler/tests/plan_repair_activation.rs`（新增零 socket 用例 + 既有用例最小适配）；`src/web/coding_ws_handler/tests/event_hub.rs:74-91`（if_live 用例改写为零 fan-out 用例）。

**Interfaces:** Consumes: Task 1/2 全部符号、`state.coding_sockets.hub_sender`（`coding_socket_registry.rs:77-89`）、`state.coding_runs.{lock_attempt,runner_count,try_reserve_attempt}`、`spawn_plan_amendment_runner_reserved`、`WorkItemRevisionStore::{get_plan_lineage,get_amendment_manifest}`、`get_plan_binding`。Produces: `spawn_undelivered_amendment_redelivery` 与 `redeliver_undelivered_plan_amendments`（统一接口块）。删除 `hub_sender_if_live` 及其 doc（生产唯一调用方即本任务改写点）。

- [ ] **Step 1: 写失败测试。**
  - 引擎层（`review_fix_delivery.rs` 尾部）：
    ```rust
    #[tokio::test]
    async fn coding_amendment_redelivery_reuses_event_until_real_ack() {
        let fixture = amendment_fixture().await;
        // 种子：零订阅者 -> 业务 Running + durable Unsent。
        let (event_tx, event_rx) = mpsc::channel(8);
        drop(event_rx);
        let seeded_engine =
            CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx);
        let resumed = seeded_engine
            .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
            .await
            .unwrap();
        let unsent = poll_delivery_status(
            &fixture.store,
            &resumed,
            &fixture.manifest.id,
            crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent,
        )
        .await;

        // 补投递：新订阅者 + redeliver -> 同 event_id 恰一帧 + Delivered。
        let (tx2, mut rx2) = mpsc::channel(8);
        let confirm_loop = tokio::spawn(async move {
            while let Some(event) = rx2.recv().await {
                crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                    &event,
                );
            }
        });
        let engine2 = CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), tx2);
        assert_eq!(
            engine2
                .redeliver_undelivered_plan_amendments(&resumed)
                .await
                .unwrap(),
            1
        );
        let delivered = poll_delivery_status(
            &fixture.store,
            &resumed,
            &fixture.manifest.id,
            crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Delivered,
        )
        .await;
        assert_eq!(delivered.event_id, unsent.event_id);
        confirm_loop.abort();

        // 幂等：已 Delivered 再触发 => 0 条、无新事件。
        let (tx3, mut rx3) = mpsc::channel(8);
        let engine3 = CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), tx3);
        assert_eq!(
            engine3.redeliver_undelivered_plan_amendments(&resumed).await.unwrap(),
            0
        );
        assert!(rx3.try_recv().is_err());
        drop(rx3);
    }

    #[tokio::test]
    async fn coding_amendment_redelivery_skips_orphan_records_without_manifest() {
        let fixture = amendment_fixture().await;
        let (event_tx, event_rx) = mpsc::channel(8);
        drop(event_rx);
        let resumed = CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), event_tx)
            .apply_plan_amendment(&fixture.attempt, &fixture.manifest)
            .await
            .unwrap();
        poll_delivery_status(
            &fixture.store,
            &resumed,
            &fixture.manifest.id,
            crate::product::coding_models::CodingPlanAmendmentDeliveryStatus::Unsent,
        )
        .await;
        // 孤儿记录：有 delivery 文件、无 manifest（不阻塞、不计数）。
        let orphan = serde_json::json!({
            "id": format!("coding_plan_amendment_delivery_{}_amendment_orphan", resumed.id),
            "event_id": format!("coding_plan_amendment_updated_{}_amendment_orphan", resumed.id),
            "attempt_id": resumed.id,
            "amendment_id": "amendment_orphan",
            "status": "unsent",
            "delivered_at": null,
            "created_at": "2026-09-26T00:00:00Z",
            "updated_at": "2026-09-26T00:00:00Z",
        });
        let orphan_path = fixture
            .store
            .attempt_dir(&resumed.project_id, &resumed.issue_id, &resumed.id)
            .join("amendment-event-deliveries")
            .join("amendment_orphan.json");
        std::fs::write(&orphan_path, serde_json::to_vec(&orphan).unwrap()).unwrap();

        let (tx2, mut rx2) = mpsc::channel(8);
        let confirm_loop = tokio::spawn(async move {
            while let Some(event) = rx2.recv().await {
                crate::web::coding_ws_handler::delivery_ack::confirm_plan_amendment_socket_write(
                    &event,
                );
            }
        });
        let engine2 = CodingWorkspaceEngine::new(fixture.store.clone(), GitWorkspaceService::new(), tx2);
        assert_eq!(
            engine2.redeliver_undelivered_plan_amendments(&resumed).await.unwrap(),
            1
        );
        confirm_loop.abort();
    }
    ```
  - web 层（`tests/plan_repair_activation.rs` 新增；URL 构造与既有用例 L76-77 / `tests/it_web/web_work_item_plan_repair/part_04.rs:81-84` 同款 `/ws/projects/{p}/issues/{i}/coding-attempts/{id}`）：
    ```rust
    #[tokio::test]
    async fn zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery() {
        let root = tempfile::tempdir().unwrap();
        let runtime = crate::web::test_controls::PlanRepairFixtureRuntime::seed(
            root.path(),
            crate::web::test_controls::PlanRepairFixtureControl::default(),
        )
        .await
        .unwrap();
        let identity = runtime.drive_until_awaiting_confirmation().await.unwrap();
        let state = WebAppState::new(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
        );
        let app = build_web_router(state.clone());
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap(); });
        let store = CodingAttemptStore::new(ProductAppPaths::new(root.path().join(".aria")));
        let attempt = store
            .get_attempt_for_work_item_group(
                "project_0001",
                "issue_plan_0001",
                "work_item_plan_0001",
                None,
            )
            .unwrap()
            .unwrap();
        let attempt_key = CodingAttemptRunKey::from_attempt(&attempt);

        // 全程不连接 coding WS：只从 workspace 子会话确认。
        let child_url = format!("ws://{addr}/api/ws/workspace/{}", identity.child_session_id);
        let (mut child_ws, _) = connect_async(child_url).await.unwrap();
        let initial_child = receive_json(&mut child_ws, "initial plan repair child state").await;
        assert_eq!(initial_child["type"], "session_state");
        child_ws
            .send(Message::Text(
                json!({
                    "type": "confirm_plan_amendment",
                    "amendment_id": identity.amendment_id,
                })
                .to_string()
                .into(),
            ))
            .await
            .unwrap();

        let resumed = timeout(Duration::from_secs(3), async {
            loop {
                let current = store
                    .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
                    .unwrap();
                if current.status == CodingAttemptStatus::Running
                    && state.coding_runs.runner_count(&attempt_key) == 1
                {
                    return current;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("zero-socket activation must resume the attempt (REQ-WIGA-06)");

        let delivery = timeout(Duration::from_secs(3), async {
            loop {
                let delivery = store
                    .get_plan_amendment_delivery(&resumed, &identity.amendment_id)
                    .unwrap();
                if delivery.status == CodingPlanAmendmentDeliveryStatus::Unsent {
                    return delivery;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("no live socket must leave a durable unsent fact, never Delivered");
        assert_eq!(delivery.delivered_at, None);

        child_ws.close(None).await.ok();
        server.abort();
    }
    ```
    （fixture fake provider 挂起保持 runner 存活，与既有 `repeated_confirmation_...` 用例同构，runner_count==1 轮询可达。）
  - registry（`event_hub.rs` L74-91 改写）：
    ```rust
    /// REQ-WIGA-06：plan_amendment 激活不再要求存活 socket；hub 可在零订阅者时
    /// 建立，零 fan-out 由 delivery ack 立即结算失败（观察层记 Unsent）。
    #[tokio::test]
    async fn hub_zero_fanout_settles_amendment_waiter_unsent() {
        let registry = CodingSocketRegistry::default();
        let key = CodingAttemptRunKey::new("project_0001", "issue_0001", "attempt_e");
        let hub = registry.hub_sender(&key);
        let waiter = register_plan_amendment_socket_write("event_zero_fanout").unwrap();
        hub.send(plan_amendment_event("event_zero_fanout"))
            .await
            .unwrap();
        let error = tokio::time::timeout(Duration::from_millis(250), waiter.wait())
            .await
            .expect("zero fan-out must settle immediately")
            .expect_err("zero fan-out must not acknowledge delivery");
        assert!(
            error.to_string().contains("plan_amendment_socket_write_failed"),
            "unexpected settlement error: {error}"
        );
    }
    ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib coding_amendment_redelivery -- --nocapture`；`cargo test --locked --lib zero_socket_plan_amendment -- --nocapture`；`cargo test --locked --lib hub_zero_fanout -- --nocapture`。预期 `redeliver_undelivered_plan_amendments` 未定义、零 socket 激活被 `plan_amendment_coding_socket_unavailable` 拦截（resumed 轮询超时）、if_live 用例待删。
- [ ] **Step 3: 最小实现。** ① `amendment.rs` 新增 `redeliver_undelivered_plan_amendments`（统一接口块逐字：list→跳过 Delivered→`get_amendment_manifest` 失败跳过→`plan.active_revision_id != manifest.new_plan_revision_id` 跳过→`deliver_plan_amendment_observation_once`，计数 Delivered）；② `delivery_ack.rs` 新增 `spawn_undelivered_amendment_redelivery`（接口块逐字）；③ `plan_repair_activation.rs`：L22-31 的 `hub_sender_if_live`+`ok_or_else(...)` 改为 `let event_tx = state.coding_sockets.hub_sender(&attempt_key);`（注释改述：REQ-WIGA-06 零 socket 也激活；hub 保活与清理规则见 `coding_socket_registry.rs:62-75`——零 socket 激活后若 runner 退出且无 socket 周期，hub entry 有界滞留、无正确性影响）；L19-21 early-return 分支（runner_count>0）在 `return Ok(())` 前调用 `spawn_undelivered_amendment_redelivery(coding_store, attempt, event_tx)`——`event_tx` 获取移到分支判断之前（对两分支共用）；④ `coding_socket_registry.rs` 删除 `hub_sender_if_live` L91-108。
- [ ] **Step 4: 绿灯。** 上述三条过滤命令；复跑 `cargo test --locked --lib repeated_confirmation -- --nocapture`（既有用例经 early-return 补投递后应保持绿；首轮不再出现 `PLAN_AMENDMENT_ACTIVATION_FAILED`，如断言该错误则按新语义删改该断言）与 `cargo test --locked --lib coding_amendment -- --nocapture`。
- [ ] **Step 5: 提交。** `git add src/product/coding_workspace_engine/amendment.rs src/web/coding_ws_handler/delivery_ack.rs src/web/workspace_ws_handler/plan_repair_activation.rs src/web/state/coding_socket_registry.rs src/product/coding_workspace_engine/tests/plan_amendment/review_fix_delivery.rs src/web/workspace_ws_handler/tests/plan_repair_activation.rs src/web/coding_ws_handler/tests/event_hub.rs && git commit -m "feat: activate published amendment without live coding socket"`。

## Task 4：socket 重连补投递（attach 钩子）

**Files:** Modify `src/web/coding_ws_handler/socket.rs:112-121`（`send_pending_choice_frames` 成功后、`ensure_runner_for_resumed_attempt` 之前插入 attach 补投递 spawn）；`src/web/state/coding_socket_registry.rs:183-214`（`broadcast` doc 注释更新：零 fan-out 立即结算失败的消费方改为「观察层记 Unsent（REQ-WIGA-06）」，行为零变化）；`src/web/coding_ws_handler/delivery_ack.rs:108-111`（`expect_plan_amendment_fan_out_writes` doc 注释同步 k3-P1→REQ-WIGA-06 语义，行为零变化）。Test `src/web/workspace_ws_handler/tests/plan_repair_activation.rs`（零 socket 用例尾部扩展 late-connect 段）。

**Interfaces:** Consumes: Task 3 `spawn_undelivered_amendment_redelivery`。Produces: attach 路径补投递接线；无新符号。

- [ ] **Step 1: 写失败测试。** 在 Task 3 的 `zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery` 的 Unsent 断言之后、`child_ws.close()` 之前插入：
  ```rust
  // 重连补投递（REQ-WIGA-06）：迟到的观察者收到同一 event，真实写 ack 后才 Delivered。
  let coding_url = format!(
      "ws://{addr}/ws/projects/{}/issues/{}/coding-attempts/{}",
      attempt.project_id, attempt.issue_id, resumed.id
  );
  let (mut coding_ws, _) = connect_async(coding_url).await.unwrap();
  assert_eq!(
      receive_json(&mut coding_ws, "attach snapshot").await["type"],
      "coding_session_state"
  );
  let amendment_event = receive_type(&mut coding_ws, "plan_amendment_updated").await;
  assert_eq!(amendment_event["event_id"], serde_json::json!(delivery.event_id));
  let delivered = timeout(Duration::from_secs(3), async {
      loop {
          let current = store
              .get_plan_amendment_delivery(&resumed, &identity.amendment_id)
              .unwrap();
          if current.status == CodingPlanAmendmentDeliveryStatus::Delivered {
              return current;
          }
          tokio::task::yield_now().await;
      }
  })
  .await
  .expect("real socket write acknowledgement must mark Delivered");
  assert!(delivered.delivered_at.is_some());
  coding_ws.close(None).await.ok();
  ```
- [ ] **Step 2: 红灯。** `cargo test --locked --lib zero_socket_plan_amendment -- --nocapture`；预期 late-connect 段在 `receive_type(...)` 超时（无补投递触发）。
- [ ] **Step 3: 最小实现。** `socket.rs` 在 L117-120 的 choice 补帧成功后插入（不内联 await，避免与主循环写结算互相等待）：
  ```rust
  // REQ-WIGA-06：attach 补投递 durable 未送达的 plan amendment（快照/choice
  // 初帧先行，保持 wire 顺序；真实写 ack 才标 Delivered，失败仍 Unsent）。
  super::delivery_ack::spawn_undelivered_amendment_redelivery(
      coding_store.clone(),
      resumed_attempt.clone(),
      event_tx.clone(),
  );
  ```
  （`resumed_attempt` 即 L105 既有克隆；helper 内部 spawn，不阻塞 attach 主循环；快照与 choice 帧已于 L106-120 直写 socket，事件经 hub 在其后到达，顺序不变。）同步更新两处 doc 注释（Files 所列）。
- [ ] **Step 4: 绿灯。** `cargo test --locked --lib zero_socket_plan_amendment -- --nocapture`；复跑 `cargo test --locked --lib plan_repair -- --nocapture`（覆盖 `coding_ws_handler/tests/plan_repair/*` attach/恢复/身份用例与 workspace 激活用例）与 `cargo test --locked --lib hub_ -- --nocapture`。
- [ ] **Step 5: 提交。** `git add src/web/coding_ws_handler/socket.rs src/web/state/coding_socket_registry.rs src/web/coding_ws_handler/delivery_ack.rs src/web/workspace_ws_handler/tests/plan_repair_activation.rs && git commit -m "feat: redeliver undelivered amendments on coding socket attach"`。

## Task 5：矩阵/集成旧语义用例收口与全组复跑

**Files:** Modify `src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix/amendment_row.rs`（终值断言 L82-86、窗口 4 L565-675）；`tests/it_web/web_work_item_plan_repair/part_04.rs`（L142-145、L386-392）。无生产代码改动。

**Interfaces:** Consumes: Task 1-4 全部语义。Produces: 旧「delivery 收口失败=业务失败」用例按 REQ-WIGA-06 re-pin；无新符号。

- [ ] **Step 1: 写失败/重定向测试。**
  - `amendment_row.rs` 窗口 4（L565-675）re-pin：首轮（L601-608）由 `expect_err` 改为 `expect`（socket 写失败与 mark failpoint 两模式都**不再产生业务错误**）；L609-615 断言 `AmendmentApplyFailed` → 改 `Running`；L616-624 marker：`socket_write_failed` 模式期望 `Unsent`（`delivery_mark_crash` 模式仍 `Pending`，按 `socket_write_succeeds` 分支断言）；L643-647 `context == Open` → 改 `Applied`（业务已完整收口）；L648-653 `first_event_ids.len()==1` 保留。第二轮（L656-673）由 `recovered_engine.recover_plan_amendment(&failed)` 改为 `recovered_engine.redeliver_undelivered_plan_amendments(&failed)`（Running+Completed+未送达不再能经 `current_amendment_journal_id` 重选，见 `amendment.rs:277-293`——`delivered_current` 仅认 Delivered）；`recovered_event_ids == first_event_ids` 保留（同 event_id 恰一次）。
  - `assert_amendment_row_final_invariants`（L82-86 delivery 断言）与其调用点（L557、L673）：Delivered 断言改为有界轮询（2s timeout + `yield_now` 循环，同 `poll_delivery_status` 模式；该文件 fixture 自带 confirm 消费者，观察任务必达 Delivered）。
  - `part_04.rs` L142-145 与 L386-392：`Delivered` 直接断言改为 3s 有界轮询（两处 coding WS 均已连接并收到 `plan_amendment_updated`，写结算与 Delivered 落盘之间存在异步窗口）。
- [ ] **Step 2: 红灯。** `cargo test --locked --lib campaign_stage3_recovery_matrix_amendment -- --nocapture`（若 Step 1 已先行改断言则先红后实现无生产改动，红因 Task 2 起语义翻转——本任务以「改完断言即绿」为准）；`cargo test --locked --test it_web web_work_item_plan_repair -- --nocapture`。
- [ ] **Step 3: 最小实现。** 无生产代码改动；仅当 Step 1 断言与实际语义有出入时修断言本身（如 failpoint 模式下 marker 停 `Pending` 而非 `Unsent`——mark 在真实 ack 后被 failpoint 拦截）。**禁止**为实现绿灯改生产语义。
- [ ] **Step 4: 绿灯（本计划关闸复跑清单）。** 依次：`cargo test --locked --lib coding_amendment -- --nocapture`；`cargo test --locked --lib plan_repair -- --nocapture`；`cargo test --locked --lib hub_ -- --nocapture`；`cargo test --locked --lib campaign_stage3 -- --nocapture`；`cargo test --locked --lib group_amendment -- --nocapture`（`tests/group_amendment_chain.rs:891-895` 恢复链零回归）；`cargo test --locked --test it_web web_work_item_plan_repair -- --nocapture`。全量套件仍由集成负责人统一执行。
- [ ] **Step 5: 提交。** `git add src/web/workspace_ws_handler/tests/campaign_stage3_recovery_matrix/amendment_row.rs tests/it_web/web_work_item_plan_repair/part_04.rs && git commit -m "test: repin amendment recovery matrix for unsent delivery semantics"`。

## 自审

- **契约覆盖**：REQ-WIGA-06 两 Scenario 逐句落位——「没有 coding 页面仍可应用 amendment」→ Task 2（业务解耦）+Task 3（激活面）；「不以 `plan_amendment_coding_socket_unavailable` 拦截」→ Task 3 删除该路径（`decisions.rs` 的 `PLAN_AMENDMENT_ACTIVATION_FAILED` 仅剩真实保留类错误）；「真实事件未投递则保持未送达并供重连补读」→ Unsent/Pending durable 事实 + Task 4 attach 补投递 + Task 3 重复确认触发；「不重复应用 amendment」→ 业务幂等路径未动（journal/finalize/context 收口原序保留，仅投递移出关键路径），事件幂等由稳定 `event_id`+注册表互斥保证；「不因观察端反压改写业务终态」→ detached 观察 + 慢观察者用例；「MUST NOT 假写 Delivered / 假 socket」→ 边界 2/3 与 Review Focus 5 反例用例。tasks.md 3.2 其余部分（无人首启、barrier 中窗）明确排除并声明不勾选。
- **占位符**：无 TBD/「适当处理」；所有测试为可运行代码（含断言与超时），所有实现步骤给出逐字签名/骨架与精确行号；唯一依赖同文件既有 helper（`receive_json`/`receive_type`/`amendment_fixture`/`plan_amendment_event`）均已勘察存在。
- **类型一致**：统一接口块三段签名与各任务 Step 3/测试代码逐字一致（`mark_plan_amendment_delivery_unsent`/`list_plan_amendment_deliveries`/`deliver_plan_amendment_observation_once`/`spawn_plan_amendment_delivery_observation`/`redeliver_undelivered_plan_amendments`/`spawn_undelivered_amendment_redelivery`/`PLAN_AMENDMENT_DELIVERY_ACK_TIMEOUT`/`CodingPlanAmendmentDeliveryStatus::Unsent`）；`validate_delivery` 守卫与 Task 1 Step 3 公式一致；serde snake_case `"unsent"` 与 Task 1/3 的 JSON 直读断言一致。
- **Review Focus 落实**：5 条各有 owning task 与具名用例（见 Review Focus 列表）；边界 4 的 hub owner/清理以注释+零行为变化处理并说明理由（避免 `wait_until_hub_drained` 因果序回归风险）。
- **文件域**：全部 Files 均在许可域内（amendment 族+socket.rs attach 单点+两处测试最小适配）；未触碰 `src/web/{handlers,app.rs,error.rs,types.rs,workspace_session}`、`web/src`、`decisions.rs`、`runner*.rs`。

---

> 本计划不是运行记录；落盘时没有声称任何测试已执行或代码已实施。P2.2 的 tasks.md 勾选由后续线 A/C 完成后统一处理。
