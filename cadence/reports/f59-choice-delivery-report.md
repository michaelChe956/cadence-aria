# F-59 choice 投递可靠性修复报告（关键帧兜底 + 等待可见）

- 分支/基线：`feat-b-0808-add-monorepo` @ `461d155e`（BASE 最新）
- 修复人：F59Fix worker（omp 委派）
- 日期：2026-09-25

## 一、现场复盘（issue_0002）

**现象**：story 修订轮 author 发出 AskUserQuestion（choice started）后，900s 内 driver 连接不在（连接日志全部为 observer 角色），choice 卡无处渲染，最终 `provider_choice_wait_timeout`（900s）中止运行。

**链路解剖**：

1. `provider_drive` 驱动循环收到 `ProviderEvent::ChoiceRequest` → 进程级登记簿登记 → 发 `EngineEvent::ChoiceRequest`；此后引擎**静默等待应答**（choice 挂起期间看门狗豁免，无任何后续广播）。
2. router 侧 `broadcast(ChoiceRequest)` 对 degraded（出站队列满）连接只走 `try_send`——**满即丢**；C3 关键帧白名单（`stage_change`/`session_state`/`human_gate_closed`）不含 `choice_request`，丢帧后无有界等待投递、无 `resync_required` 信号。
3. 用户侧等待不可见：卡未送达即对话流无任何「有问题等你」提示；900s 后收到不可行动的中止错误（「可重新开始生成」——修订场景下用户不知要重新提交反馈）。

**两缺陷**：
- **缺陷 1（投递无兜底）**：choice 帧不在关键帧白名单——「丢了它 + 引擎静默 = stale 至超时」与门开帧同级，却无兜底腿。
- **缺陷 2（等待不可见）**：pending choice 存在期间无任何常驻提示；超时文案不可行动。

## 二、修法与实现

### 缺陷 1——choice_request 入 C3 关键帧白名单

**`src/web/workspace_session/router.rs` `is_keyframe_message`**：`ChoiceRequest { source, .. }`（`source != "text_fallback"`）入白名单。效果：

- degraded 连接上 choice 帧溢出（当场降级或已降级）→ 起有界等待投递任务（2×750ms + 250ms 退避，发送前复查 degraded 防旧基线 seq 回退），等待成功即送达**含 `pending_choice_requests` 投影的 session_state 恢复基线**——前端 `reconcilePendingChoiceRequests` 对账自动补弹卡；两次退避仍失败必发 `resync_required`，客户端恢复读取即送达。
- 白名单命中还使基线预建条件放宽（无 degraded 连接也预建），新降级连接同样可恢复。
- `text_fallback` 不入列：该源无 provider 挂起等待界（无 900s 计时），投影（`pending_author_choice`）是其唯一恢复面，由 F-27 广播链覆盖。

**正交性核查（两条腿均在）**：

| 腿 | 机制 | 覆盖场景 | 证据用例 |
|---|---|---|---|
| C3 degraded 投递（本次增强） | 关键帧白名单 → 有界等待 + resync_required | 在线连接被背压降级 | `choice_request_broadcast_on_degraded_attachment_delivers_baseline_during_silence`（part_06，新增） |
| F-43/F-24 刷新补发（既有） | `register_pending_choice_frame` 挂起帧登记；attach 初帧（`activate_attachment_with_initial_frames`）、cursor 重订阅（`send_pending_choices_for_resubscription`）、degraded 恢复（`try_recover_degraded_attachment` Ok 支路）三处补发 choice 帧本身 | 断线后重连/刷新 | `attach_and_cursor_resubscribe_redeliver_pending_provider_choices`、`degraded_attachment_recovery_redelivers_pending_provider_choice`（part_02，既有，回归绿） |

两腿正交：C3 腿送「基线投影」（对账补卡，覆盖降级在线连接）；F-43 腿送「choice 帧本身」（覆盖重连/刷新）。driver 回来无论走哪条路径都能补弹。

### 缺陷 2——等待可见

**后端**：

1. **投影带出时刻与角色**（`pending_choice_requests` 元素增补字段）：
   - `provider_drive/pending_choices.rs`：登记簿条目改 `RegisteredPendingChoice { request, created_at_ms, role }`——`created_at_ms` 为登记时刻（epoch ms），与驱动循环 `choice_wait_timer`「pending 由空转非空起算」同源；`role` 为发问角色（`ProviderConversationRole::wire_label()`，新增于 `models/provider.rs`；`choice_audit::role_label` 收敛为委托）。
   - 三驱动插入点带角色：`provider_drive.rs`（`role.wire_label()`，author/reviewer 随 run）、`review/drive.rs`（`"reviewer"`）、`work_item_plan.rs`（`"author"`）。
   - `WsPendingChoiceRequest`（wire）增 `created_at_ms: Option<u64>`（`skip_serializing_if` None，旧客户端零影响）与 `role: String`（`serde(default)` = author，旧载荷对账兼容）；`session_state.rs` 投影两臂（provider 登记簿 / TextFallback `pending_author_choice`）分别带出 `Some(ms)`+role 与 `None`+`"author"`。
   - **核查结论**：WS 快照/projection 本就带出 `pending_choice_requests`（F-27），本次只补时刻/角色两字段（刷新后已等待时长不失真）。
2. **超时文案可行动**（`provider_drive.rs` 与 `review/drive.rs` 同口径）：
   - 旧：`等待用户选择应答超过 N 秒（choice 卡未达用户或无人应答，pending=[..]），运行已中止；可重新开始生成`
   - 新：`等待回答超时——N 秒内未收到选择应答，运行已中止；choice 卡可能未送达（页面断线/连接降级期间丢失，刷新页面可补卡）或已送达但无人应答（pending=[..]）。请重新提交反馈重新发起本轮`
   - 原因码前缀 `provider_choice_wait_timeout` 与 pending 清单保留（既有断言/日志口径不变）。

**前端**：

1. **store**：`WorkspaceWsState.pendingChoiceRequests: PendingChoiceRequestProjection[]`（id/prompt/role/created_at_ms/first_seen_at_ms）；`setSessionState` 经 `pendingChoiceRequestsFromSession`（helpers 新增）归一——畸形条目跳过、同会话同 id 首见时刻跨帧稳定、旧载荷 `created_at_ms` 缺省回退首见。观测态（`observerStateFromSessionState`）同样带出——**驾驶连接缺席、驾驶舱 observer 视图也能看到提示条**（现场「日志全 observer」的对症面）。
2. **提示条**（`PendingChoiceNotice` 增 `requests` prop，两模式）：
   - 投影驱动（新）：`⏳ {author|reviewer} 有问题等你回答（N 个问题）：{摘要}` + `已等待 m:ss · m:ss 后超时`（窗口 901s = 后端 900s + 1s 展示余量；锚点 = min(created_at_ms ?? first_seen_at_ms)，与后端计时语义一致）；卡在场时给「定位选择卡」，卡缺席时给「选择卡未显示？刷新页面可补卡」。每秒刷新（无挂起不留定时器）。
   - 卡驱动（既有 F-43 路径）：文案不变（回归绿）。
3. **接线**：`ChatCockpitPage`（当前会话 + takeover/观测态）门条件改为 `pendingChoices.length > 0 || pendingChoiceRequests.length > 0`。`CodingWorkspacePage` 维持既有 F-43 卡驱动路径（coding 面有独立机制，不在本次现场）。

## 三、红绿证据

| 用例 | 红 | 绿 |
|---|---|---|
| `keyframe_whitelist_covers_pending_choice_frames`（router 白名单单测） | ✅ `source=ask_user_question 的 choice_request 必须按关键帧投递` | ✅ |
| `choice_request_broadcast_on_degraded_attachment_delivers_baseline_during_silence`（part_06：choice 首溢出降级 + 引擎静默，有界等待必达基线，event_seq=2） | ✅ `Elapsed(())`（3s 无帧送达） | ✅ |
| `workspace_ws_session_state_projects_pending_provider_choice_during_active_run`（part_08 增补：provider 投影带 created_at_ms∈[now-60s,now] + role=author） | ✅ `must carry created_at_ms` / `Null != "author"` | ✅ |
| `workspace_ws_session_state_projects_text_fallback_pending_choice`（part_08 增补：role=author、created_at_ms 缺席） | ✅ | ✅ |
| `choice_wait_timeout_converts_lost_choice_into_diagnosable_rerunnable_failure`（增补文案断言） | ✅ `超时文案必须指路重新提交反馈` | ✅ |
| `review_choice_wait_timeout_converts_lost_reviewer_choice_into_diagnosable_failure`（增补同口径断言） | ✅ `超时文案必须可行动` | ✅ |
| `workspace-ws-message-handler.test.ts` pending choice wait state projection（4 用例：直通/首见回退稳定/清空/role 缺省） | ✅ 9 失败之一族 | ✅ |
| `PendingChoiceNotice.test.tsx` wait hint（5 用例：等待+倒计时/reviewer 角色/卡在场跳转/首见回退/计数） | ✅ `Unable to find pending-choice-notice` | ✅ |

## 四、门禁

- `cargo fmt --check` ✅；`cargo clippy --all-targets --all-features --locked -- -D warnings` ✅；`cd web && pnpm tsc -b` ✅
- `cargo test --locked --no-fail-fast`（两轮一致）：lib 3643 ✅、it_core 187 ✅（首轮全量中
  `workspace_ws_idle_timeout_records_server_idle_connection_diagnostic` 为 30ms idle-timeout
  时序用例负载抖动，两轮 no-fail-fast 与单跑均通过）、it_product 43 ✅、it_web 210 ✅、
  351 ✅、54 ✅、31 ✅、43 ✅、2 ✅；唯一失败
  `guards::single_repo_rejects_logical_codebase_routes_without_persisting_artifacts`
  （PIB 基准分支 git 探测面，422≠200）经基线 461d155e detach 复测**同样失败**——预先存在，
  与本改动无关，另行定责
- `cd web && pnpm test` 全量 **1863/1863** ✅
- 行数红线：`provider_drive.rs` 1195 / `review/drive.rs` 1200（净零增行）✅

## 五、提交
