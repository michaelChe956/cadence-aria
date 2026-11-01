# Wave4 F-25b + F-26b 修复报告

- 缺陷来源：2026-09-21 全流程 E2E 验证（F-25/F-26 登记，0497 现场锚）
- Commit：`61b4ee8d fix(web): F-25b HTTP confirm 后广播 confirmed session_state + F-26b 生命周期面投影 review 证据`

## F-25b：确认后 cockpit 页面投影不刷新

### 根因
- story/design AuthorConfirm 的确认通路是 HTTP confirm 端点（`POST /api/workspace-sessions/{id}/confirm`，WS confirm 帧在该阶段被矩阵拒收，见 d2d67b39/F-18 接线）。
- 该端点只写 durable（gate → status Confirmed → 追加审计消息），不广播任何 WS 帧。
- 前端乐观 `setSessionStatus("confirmed")` 只覆盖发请求的 tab；其他 tab / takeover 视图 / 重连连接读到的仍是旧投影——会话页动作条与门卡停在「等待确认产物」。

### 修法（对照 F-09 HumanGateOpened 广播先例 a1a479a1）
1. `WorkspaceEngine::apply_external_confirm_record`（session_state.rs）：以 durable record 同步引擎内存投影——`session_status`、`stage`（fix round 1，k3 审 1×P2：经 `workspace_stage_for_status` 与 from_record 同口径收敛，Confirmed→Completed，否则已连接 tab 收到 confirmed+author_confirm 组合、重连 tab 却看到 completed，两投影互相矛盾）、`human_gate_snapshot`（Confirmed 时 durable 已清空）、尾部新增消息（确认审计行，按 `msg_{:03}` 连续编号补登）。
2. `WorkspaceSessionManager::broadcast_http_confirm`（router.rs）：锁 engine 同步后调用既有 `broadcast_current_session_state()`——广播全量 session_state 给全部 attachment，同时入 journal（后续 cursor 回放同样取到 confirmed 态）。
3. `workspace_session_confirm` handler：durable 写完成后经 `state.workspace_sessions.get()` 取运行期 manager（无 manager 时零副作用跳过），广播后返回 DTO。

### TDD
- 红测：`it_core/workspace_ws_integration/part_07.rs::http_confirm_broadcasts_confirmed_session_state_to_connected_ws`——修复前 confirm 后 WS 零帧（post-confirm frame timeout）。
- 绿测断言：真实 WS 连接（TCP server + tokio-tungstenite）吸收初帧（非 confirmed 基线）→ 同 state `app.oneshot` 发 HTTP confirm → WS 收到 `session_state` 帧且 `session_status == "confirmed"`、`stage == "completed"`（fix round 1 补），且 `messages` 含「确认当前 Workspace 产物」审计行。

## F-26b：review 生命周期不可见

### 根因
- design 审核与 plan Review Round 实际执行（workspace timeline 有 reviewer_run / plan 系 review 节点），但生命周期工作台（issue 面）零展示——用户无法从 issue 面确认「review 已做」。

### 修法
- 后端投影（additive，旧响应/旧客户端兼容）：
  - `StorySpecDto` / `DesignSpecDto` / `IssueWorkItemPlanDetailDto` 新增 `review_status: Option<String>`（serde default + skip_serializing_if None）。
  - `dto.rs::session_review_status`：读 `workspace-timelines/{session}/timeline_nodes.json`，reviewer 节点集与 engine reviewer 角色分类同集合（`ReviewerRun | WorkItemPlanOutlineReview | WorkItemDraftReview | WorkItemBatchReview`）；Active/Paused → `"running"`（进行中优先），否则任一 Completed → `"completed"`；零证据/读失败 → None（只读投影降级，不阻断生命周期响应）。
  - `issue_lifecycle` 对 story/design/plan 三族 DTO 接线；`prepare_work_item_plan` 响应同款投影。
- 前端展示（零证据零展示）：
  - `LifecycleCard`：story_spec/design_spec/work_item_group 卡渲染 `lifecycle-card-review-status` 徽标（running=sky「Review 进行中」/completed=primary「Review 已完成」）。
  - `LifecycleCardDrawer`：`DrawerEntity.reviewStatus` → header `drawer-review-status` 徽标（`toDrawerEntity` 从 card.raw 透传）。
  - 类型：`WorkspaceReviewStatus`（lifecycle.ts 导出，work-item-plan.ts 复用）。

### TDD
- 红测（后端）：`it_web/web_lifecycle_api/part_02.rs::lifecycle_projects_review_status_from_workspace_timeline_reviewer_runs`——三态钉死：无节点 → null；仅 completed → `"completed"`；completed+active 并存 → `"running"`（修复前 review_status 恒 Null）。
- 绿测（前端）：`LifecycleCard.test.tsx::projects workspace review evidence onto spec and plan cards`（story completed / design running / plan completed / 零证据不渲染）+ `LifecycleCardDrawer.test.tsx::projects workspace review evidence in the drawer header`（running/completed/无证据三态）。

## 验证
- Rust：`cargo test --lib` 3444/3444；`it_web -- web_lifecycle_api` 25/25；`it_core -- workspace_ws_integration` 46/46（首轮 1 例并发 flaky，复跑两轮全绿，与本改动无逻辑交集）。
- 前端：vitest `LifecycleCard` + `LifecycleCardDrawer` 14/14；`tsc --noEmit` 0 错。
- Lint：`cargo clippy --lib --tests` 改动文件 0 警告（预存 sandbox.rs 警告非本批文件）；`cargo fmt` 已跑。

## 改动文件
- 后端：`session_state.rs`（engine 同步面）、`router.rs`（manager 广播面）、`handlers/workspace_session.rs`（confirm 接线）、`dto.rs`（review_status 投影）、`handlers/lifecycle.rs`（三族 DTO 接线）、`types.rs`（DTO 字段）。
- 测试：`part_07.rs`、`part_02.rs`。
- 前端：`lifecycle.ts`、`work-item-plan.ts`（类型）、`LifecycleCard.tsx`、`LifecycleCardDrawer.tsx`、`IssueLifecycleWorkbenchParts.tsx`（展示/接线）+ 两测试文件。
