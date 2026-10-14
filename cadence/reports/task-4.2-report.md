# Task 4.2（B1b：fresh 协议 Replay 和真实发送闭包）实施报告

- 实施：ImplAnchorB42（glm53-task worker）
- worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 基线 HEAD：`0be830bc`（fix: B1a native资格与Story Design上下文）；会话中途并行方落 `e2c15c78`（fix(trust): 生成链 trust 指导分类桥下沉产品层共享边界全覆盖，不触碰本包 4 个生产写面文件）
- 状态：实现完成、anchor_t12 红→绿与范围化回归全绿；独立 commit 已落（见 §5）

## 1. 交付内容（写面 = plan Task 4.2 Files 清单 ∩ 本派工独占面）

| 文件 | 变更 |
|---|---|
| `src/web/workspace_ws_types/in_.rs` | 新增 `LcResumeRecoveryIntent { recovery_id, intent: LcResumeRecoveryIntentKind::Fresh, operation_id }`（接口字典 §0.3 wire 项）；`StartGeneration` 增 `#[serde(default)] resume_recovery: Option<LcResumeRecoveryIntent>`——旧载荷缺省读 None（既有客户端 wire 兼容），未知 intent 种类 serde 拒收 |
| `src/web/workspace_ws_handler/socket.rs` | ① `lc_resume_recovery_inbound_gate`：arbitrate（auth/lease/scope）之后、stage 校验/engine mutex 之前的固定判定顺序，全部 store 层纯 durable 读（不等待 engine mutex）：operation_id 空/空白 typed `INVALID_COMMAND_ID`（复用 `validate_command_id`）→ recovery 身份（缺失/被新漂移替换 → `lc_resume_recovery_required` / `lc_resume_recovery_stale`，常量取自 A2.5 `LcFreshClaimOutcome`）→ 同 ID 已受理先 typed `LcResumeRecoveryOperationResult{delivery:replayed}`（零 spawn、不重核配置、resolved 同 ID 只 Replay、engine 持锁期间 1s 内立即返回）→ 新 ID 才核状态机（FreshReserved/FreshRunning → `lc_resume_recovery_busy`；Resolved → `lc_resume_recovery_resolved` 不造第二会话）、无活 provider drive（run registry 计数）、冻结配置（author/reviewer/permission 与 durable record 相等，不等 → `lc_resume_recovery_configuration_changed`）、run_context 冻结 artifact 引用与当前 durable 产物一致（version 定位 + sha256 内容指纹，变化/消失 → `lc_resume_recovery_stale`；未冻结 hash/version 的引用跳过不倒算）→ 全通过返回 None 放行 stage+mutex+CAS claim。② `RequestRevision` 活动恢复（Waiting/FreshReserved/FreshRunning）typed `LC_RESUME_RECOVERY_ACTIVE` 拒收（与 engine 2.6 稳定码同串）；Resolved 放行。③ `lc_artifact_payload_digest`（serde 规范序列化 → sha256）与 12 tag 公共 business artifact 引用提取。④ socket 读循环接线（gate 消费帧即发即 continue） |
| `src/web/workspace_ws_handler/decisions/inbound.rs` | `StartGeneration` 臂分叉：`resume_recovery: Some(intent)` → `handle_lc_resume_recovery_fresh`（A2.2 移交锚：decisions 消费面）——2.5 合同 `claim_lc_fresh` 权威 CAS claim + typed OperationResult sender：Claimed → `Accepted/Reserved` 帧、Replay（门与 claim 间竞态窗口同形收敛）→ `Replayed` 帧、Busy/Stale → typed ProtocolError（码表与 socket 门共用）、store 写失败 → `workspace_durable_projection_unavailable`（与 A1b1 稳定码同值，2.5「写失败整体 Err 不残留半受理」）。零 provider 启动——claim 后 fresh native run 归 B2 tag dispatch（5.1+）。`resume_recovery: None` → 原 plain 路径原样（2.6 engine LC 守卫不变） |
| `src/product/lifecycle_store/mod.rs` | 单行 re-export `pub use workspace::LcFreshClaimOutcome;`（A2.5 移交锚：socket/inbound 以根路径消费 STALE_* 常量与 claim 四态） |
| `src/web/workspace_ws_types/tests.rs` | 新增 `anchor_t12_wire_resume_recovery_intent_serde`（旧载荷缺省 None / 显式 fresh snake_case 往返 / 未知 intent 拒收）+ 既有 `start_generation_roundtrip` 字面量迁移 |
| 迁移（StartGeneration 字面量补 `resume_recovery: None`） | `workspace_ws_handler/tests.rs`(2)、`tests/provider_run_events.rs`(1)、`provider_run_events/sc_delegated_rerun.rs`(3)、`single_candidate_ir_reredrive.rs`(3)、`ears_reredrive.rs`(5)、`reredrive_and_reopen.rs`(9)、`tests/it_core/workspace_ws_integration/part_02.rs`(7)/`part_08.rs`(1)/`part_09.rs`(2) |
| 并行协调备案 | `single_candidate_lc_admission.rs`(19) 与 `single_candidate_provider_run.rs`(8) 的同款机械迁移被并行方 T6R2 提交 `e2c15c78` 一并带走（功能已在库内；T6R2 已在其 task-6-report「并行协调」段记录归属）；本包提交时该两文件无 diff 不重复 stage。`root_cwd_revision.inc.rs` 本包零改动（其 `anchor_t07_superseded_waiting_finishes_provider_node` 等 4.1/2.6 用例随 author_revision 回归组全绿） |

## 2. 新测试（filter `anchor_t12`，6 个；socket.rs tests mod + ws_types tests.rs）

1. `anchor_t12_wire_replay_running_resolved_and_failed_waiting`：同 ID op-1 三态（FreshRunning/Running、Resolved/Completed、Waiting/NotStarted-prepare失败回等待）全部 typed Replay（`delivery=="replayed"`、`operation_id=="op-1"`、`operation_state` 分别 `running`/`completed`/`not_started`）；FreshRunning 期间 **engine 锁被测试任务持有时 1s 内立即 Replay**（不等 engine mutex）；新 ID 撞 FreshRunning → `lc_resume_recovery_busy`；Resolved 新 ID → `lc_resume_recovery_resolved`；Resolved 后普通 generation（resume_recovery=None）门放行（resolved 只保留证据，后续新 native 正常）；prepare 失败回 Waiting 同 ID 只 Replay 不删 ID；普通 `RequestRevision` → `LC_RESUME_RECOVERY_ACTIVE` typed 拒收；全程 0 provider drive、0 新 timeline 节点、durable recovery 原样。
2. `anchor_t12_explicit_fresh_keeps_workspace_and_ignores_old_audit`：Waiting + 显式 fresh 经 decisions handler 真实 claim → 出站帧 `lc_resume_recovery_operation_result`/`accepted`/`reserved`/`fresh_node_id:null`；durable：FreshReserved、恰一次 claim、operation Reserved 且 fresh_node_id/role_run_seq None、fresh_native_session_id None、superseded_native_session_id 原样；workspace 保留（entity/type/status/author/reviewer/permission/messages/旧 conversation 逐项相等——忽略旧 audit 不复活）；零 spawn（0 provider drive、engine 锁即刻可取、0 新节点）；同 ID 重发 → `replayed` 且 fresh_operations 仍 len==1（单 claim）。
3. `anchor_t12_empty_operation_id_rejected_before_claim`：`""`/`"   "` → typed `INVALID_COMMAND_ID`；durable 仍 Waiting 零受理（claim 前）。
4. `anchor_t12_provider_config_change_rejected_before_claim`：author（ClaudeCode→Codex）/reviewer（Some(Codex)→None）/permission（author Auto→Supervised）三负例 → `lc_resume_recovery_configuration_changed`；durable 零受理。
5. `anchor_t12_stale_recovery_or_changed_source_rejected_before_claim`：recovery_id 被替换 → `lc_resume_recovery_stale`；正控（身份/配置/source hash 一致）→ 门放行 None 进 claim 路径；artifact v1 内容改写后 → `lc_resume_recovery_stale`；durable 零受理且冻结 hash 原样。
6. `anchor_t12_wire_resume_recovery_intent_serde`（ws_types）：wire 合同三项（见 §1）。

## 3. A/B 发布检查点（B1+B0 齐）

`allowed_actions` 维持恒空（out.rs `LcResumeRecoveryDto` B 波纪律注释不变，anchor_t07 既有断言继续覆盖）：B1（4.1+4.2）与 B0（3.1–3.7）均已落库，首次放开的前置条件就绪；实际放开归 B2（5.1+ tag dispatch/fullnative 启动）与 C（6.x sender/observer）。

## 4. 红→绿证据（本 worker 会话实测，cwd=worktree 根）

1. **红**（gate stub 返回 None + handler 忽略 intent 的最小骨架先行落盘后）：`cargo test --locked --lib anchor_t12` → `1 passed; 5 failed`——五处行为断言全部按预期 FAIL：`同 ID 已受理必须 Replay`（stub 无 Replay）、`空/空白 operation_id 必须被拒收`、`author 配置漂移必须被拒收`、`recovery 替换后旧 ID 必须被拒收`、explicit_fresh 得到的是 plain 路径 `Error` 帧（`left: "error" right: "lc_resume_recovery_operation_result"`，即 2.6 LC 守卫错误）。过滤命中 6 个新用例（4362 filtered out）。
2. **绿**：`cargo test --locked --lib anchor_t12` → `6 passed; 0 failed`。
3. **范围化回归**（全部本 worktree 实测）：
   - `cargo test --locked --lib author_revision` → 30 passed（4.1/4.2 §8 回归行；含 root_cwd 全部 2.3/2.6 用例）
   - `cargo test --locked --lib workspace_ws_types` → 39 passed
   - `cargo test --locked --lib interrupted_run` → 7 passed（2.5/2.6 等待语义）
   - `cargo test --locked --lib workspace_session` → 71 passed（2.3/2.4 投影/manager）
   - `cargo test --locked --lib web::workspace_ws_handler` → 191 passed（含全部迁移字面量所在用例）
   - `cargo test --locked --lib single_candidate` → 162 passed
   - `cargo test --locked --test it_core workspace_ws` → 70 passed / 3 failed（见 §6 归属）

## 5. 提交

- Commit：`6acdd4fc`（fix: B1b fresh协议Replay和真实发送闭包；独立提交，14 文件 +1244/-1，显式列表 stage——仅含本包归属 diff）。`lifecycle.rs`/`review/drive.rs` 本包零改动未 stage（lifecycle.rs 树上现存 diff 为并行方全树 rustfmt churn，归属非本包）；`single_candidate_lc_admission.rs`/`single_candidate_provider_run.rs`/`root_cwd_revision.inc.rs` 无本包未提交 diff（见 §1 并行协调备案）。
- 独立审查（ReviewAnchorB42 / k3-reviewer，2026-10-12）：**APPROVED / spec PASS，零阻断与非阻断 finding**——八项核验全过（门位置与纯读性、持锁 Replay 1s/零 spawn、稳定码表逐字核对、claim 五态全映射、allowed_actions 恒空、serde 兼容与 73 处字面量迁移逐文件计数、三项 concerns 核验、红 5→绿 6 discriminating）；两条观察均非本 commit 缺陷（e2c15c78 卷迁移接缝已由本 commit 闭环；it_core abort 族 flake 转 Main/8.1）。无 fixloop 项。
- 本报告文件按纪律不 git add（untracked）。

## 6. 范围外事实备案：it_core part_03/04 三个 abort 用例失败归属

`workspace_ws_secondary_connection_takes_lease_and_can_abort_active_run_started_by_primary`、`workspace_ws_abort_with_pi_reaches_cancelled_state_and_stops_output`、`workspace_ws_abort_after_choice_response_returns_prepare_context`（均在**非本包写面**的 part_03/part_04，超时于 recv_json 3s）。本包 socket 门对这些消息路径结构性 inert（gate 仅作用于 StartGeneration-with-intent 与 RequestRevision；UserMessage/Hello/Ping/Abort 一律 None 直通，diff 可证）。

定界结论（初判单跑对照已被重复测量修正，终版如下）：
- **三例均为负载敏感存量 flake 家族**（abort 取消后 select_cancelled 与 3s recv 超时竞态、失败成员每轮轮换），非 6acdd4fc、亦非 e2c15c78 的确定性回归。修正证据：①T6R2 干净树双基线 ≥5 轮重复测量（task-6-report.md 末节）：takes_lease 在 0be830bc 为 3P/5F（差于 e2c15c78 的 4P/1F），失败率同量级且成员轮换；②本 worker 独立复测（当前树含 6acdd4fc，各 5 轮单跑）：takes_lease P/P/F/P/P、abort_after_choice P/F/F/P/P——同树轮换通过/失败，flake 实锤。初判「e2c15c78 引入 2 例回归」系单跑噪声样本，作废并致歉。
- abort_with_pi 同族（0be830bc 即有失败记录），三例一并转 Main/8.1 按 flake 家族分诊（稳定性收口或超时加固，不属于任何 B/T 提交回归）。
- 本包写面在 it_core workspace_ws 的 70 个通过用例含 part_02/08/09 全部迁移用例；唯三失败均如上归属，且单跑轮换可复现通过。

## 7. 下包移交（B2a / 5.1）

- socket 门已就位：新 ID 前置核验通过后放行 stage+mutex 路径，5.1 在 decisions 侧接管 claim 后的 typed tag 精确 dispatch（`lc_recovery_business_artifact_refs`/`LcRecoveryRunContext` 12 tag 提取器可直接复用）；Busy/配置/身份/source 拒收码表见 socket.rs 常量（`LC_RESUME_RECOVERY_*`）。
- `claim_lc_fresh` 根路径 `crate::product::lifecycle_store::LcFreshClaimOutcome`（re-export 单行已落）。
- wire：`StartGeneration.resume_recovery`（serde default None）+ `WsOutMessage::LcResumeRecoveryOperationResult` 双向已验证；前端 sender（C2/6.2）按接口字典接 `sendLcResumeFresh`。
