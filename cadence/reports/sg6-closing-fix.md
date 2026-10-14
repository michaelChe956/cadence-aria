# SG6 契约迁移收口修复报告（campaign amendment 三败）

- 日期：2026-10-14
- 范围：`campaign_stage3_amendment::returns_to_same_attempt` + `campaign_stage3_recovery_matrix` 两窗（amendment_application_delivery / amendment_binding_window）
- 不在本报告：`sc_delegated_rerun_relay_failure_falls_back_to_human_gate_durable`（另派专人，见主控重划）

## 一、诊断

三败同签名：resume/approve 链后原 plan session 停 `WaitingForHuman`、期望 `Confirmed`（campaign_stage3_amendment.rs:1018、amendment_row.rs:145 共享终态不变量）。

探针取证（修复前）：verdict 收口后 durable 状态为 `status=WaitingForHuman phase=Some(Approval) grant=Resolved turn=Completed`。

归因链：

1. SG6a.3/SG6b 契约（delta spec REQ-CG-03 第 74 行「修订成功回呈」）：合法修订交付后 turn 仍 Running，须原 reviewer 或 synthetic Pass 收口；收口 verdict 经 Evaluate policy route 重建 Approval 门（`EnterHumanGate → phase Approval`，I-1 既有行为，由已通过的真实链测试 `amendment_revision_reviewer_pass_rebuild_continues_gate_budget` 钉死「reviewer Pass 后必须路由回 Approval 门」）。
2. 旧契约下 campaign 测试从不驱动复评收口（fake revision 交付即 turn Completed、无路由），plan session 保持重开原签名 `WaitingForHuman + phase Completed`；SG6b 消费方迁移（c896ff0f）补入 `close_amendment_review_verdict` 后，收口必然路由重建 Approval 门。
3. resume 关回守卫 `close_reopened_amendment_gate_if_idle`（amendment.rs）与关门 CAS `compare_and_close_reopened_amendment_gate`（lifecycle_store/amendment_gate.rs）仍只认重开原签名（phase Completed）——新契约下该签名在 resume 时点不再存在，关回永不触发。

**定性：回归**（SG6 契约迁移后引擎关回守卫 stale），非消费方迁移残余——测试侧收口驱动（c896ff0f）正确，引擎关回面未跟上新收口语义。

## 二、修复（最小修，引擎面）

1. `src/product/lifecycle_store/amendment_gate.rs`：`validate_amendment_gate_record` 参数化 accepted phases；reopen 保持仅 `[Completed]`，close 扩为 `[Completed, Approval]`（收口重建签名）。
2. `src/product/coding_workspace_engine/amendment.rs`：`close_reopened_amendment_gate_if_idle` 守卫相位谓词扩为 `Completed | Approval`。

安全性：关门 CAS 唯一生产调用点持有刚 Applied、经完整 lineage 校验的 `PlanAmendmentContext`（plan_session_id 即目标会话）；首次审批门（从未经 amendment context 重开）不会到达该路径。in-flight turn（Reserved/Running）仍阻断关回；reopen 方向零变化（仍要求 Confirmed+Completed），伪造/初次门不受影响。

## 三、验证

- 三败复绿：`cargo test --lib campaign_stage3_amendment_returns_to_same_attempt`（1/0）、`campaign_stage3_recovery_matrix`（9/0，含两窗）。
- 关回/门面回归：`cascade_d1_3b_amendment_gate`（1/0）、`conversational_gate_amendment_real_chain`（9/0）、`lifecycle_store`（140/0）、`group_amendment_chain`（5/0）。
- 全量 `cargo test --lib`：**4685 通过 / 0 失败 / 7 忽略**（含另派专人已在工作树修复、尚未提交的 `sc_delegated_rerun` 消费方改动；本包三败与关回回归不依赖该改动独立复绿）。

## 四、提交

`fix: SG6 契约迁移收口残余`（仅引擎两面 + 本报告；campaign_stage3_amendment.rs 无净变更）。
