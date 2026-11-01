# Task 5.6（B2d：SC manual 同 turn/hash/gate/预算）实施报告

- 实施：ImplAnchorB56（glm53-task worker）
- worktree：`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`（共享树）
- 基线 HEAD：`54732928`（fix: B2c SC author同reservation与成员compile回归）
- 状态：**完成**——TDD 红→绿证据留存（§3）、范围回归全绿（§4）、commit `f948714c`（§5）

## 0. 写面裁定备案（先读）

1. **gateway_start.rs 为验收必需延伸面**（5.5 §0.2 同型先例）：`lc_fresh_dispatch_kind` 的 sc_manual_revision 放行臂 + `verify_lc_fresh_sc_manual_revision_payload`（payload turn 事实对 durable typed 对照，漂移→`lc_resume_recovery_stale`）。tasks.md 5.6 独占清单未列该文件，但「mismatch→waiting→fresh→原 human gate」的 fresh 步唯一经 dispatch 面（`dispatch_lc_fresh_recovery_run`→`lc_fresh_dispatch_kind`），不放行则接管不成立；controller brief 明示「sc_manual_revision 从未接管示例接管后」即要求本包接管。
2. **planning_resume.rs 两负例换载**（controller brief 明示授权，5.5 §7.5 移交项）：t13⑤ 与 `anchor_t16_untaken_tag_still_typed_rejected_no_fallback` 原以 sc_manual_revision 为「未接管 tag」示例，本包接管后换载为 `LcRecoveryRunContext::Author`（author 族，归 B1 author fresh 合约面，dispatch 仍 typed 拒）。
3. `prompts/human_gate_revision.rs` 与 `conversational_gate_parts/revision_turn.inc.rs` 在独占清单但本次**零改动**：fresh prompt 装配复用既有 `build_sc_manual_revision_prompt`（原预算/transport 契约不变）；同 turn 完成器 `run_sc_manual_revision_turn` 已是同 turn 收口（compile target 经 `workspace_repository_for_session` 取业务仓 physical ID，r30 既有），本包以 anchor_t18 回归钉住而非改动。

## 1. 实施（设计 §5.3 sc_manual_revision 行 + plan 接口字典）

| 面 | 落地 |
|---|---|
| `workspace_engine/conversational_gate_recovery.rs` | ① `HumanGateRecoveryAction::BlockedByLcRecovery{recovery_id}` 新变体；② `recover_human_gate_turns`：active（非 Resolved）sc_manual_recovery 匹配 turn 先做「已真实 Completed 结果」原恢复核对（提取 `complete_turn_from_durable_artifact` 唯一写口，非阻塞路径同源复用），无结果则 BlockedByLcRecovery 跳过（不 ResumeSameTurn/不增 attempt/不 spawn）；Resolved 后既有语义回归（T1 对照臂）；③ `SC_MANUAL_NATIVE_MISMATCH_WAITING_REASON`（=gateway `RESUME_FINGERPRINT_UNAVAILABLE` 同码）；④ `precheck_sc_manual_revision_resume`（`provider_resume_session_candidate`，FrozenBindingUnavailable→Ok(Some(reason))，出生/健康→Ok(None)，其余原样上浮）；⑤ `persist_sc_manual_revision_superseded_wait`：手写 ScManualRevision tagged record（turn_id/command_id/source_hash/turn_status/attempt_no/budget_reserved、human_gate_reservation key（durable reservation 匹配 turn）、source_revision_ref、同 turn feedback 全量；gate_snapshot_ref=None——快照内嵌 session record 无独立 ref；assembly digest/ref=None——inline 交付零持久化，不伪造）→ CAS Waiting → 清被取代旧会话引用 → `reconcile_lc_resume_waiting`（幂等终态化旧 native last 节点；failed_provider_node_id 只取 conversation last_node，**绝不取活动门节点**；turn/gate 快照/预算/ledger 不动） |
| `workspace_engine/conversational_gate_parts/sc_revision_delivery.inc.rs` | `build_sc_manual_revision_prompt_for_fresh_turn(turn)`：显式 fresh 同 turn 重装——重读同 turn feedback 并对 candidate hash 复验（turn.source_hash vs 当前候选 markdown sha256；不一致或空 legacy hash→稳定码 `HUMAN_GATE_REVISION_CANDIDATE_DRIFTED`，不以旧 turn 权限修订漂移后候选）；复验通过走既有 builder（原预算/transport） |
| `web/workspace_ws_handler/run/provider_run.rs` manual 臂 | ① FreshRunning+ScManualRevision（turn_id 匹配）→ `manual_fresh_recovery`；读取失败按 Message 面收口（不落 turn 失败）；② Ordinary 先行 precheck（在 prompt 装配/mark_running/节点创建之前——mismatch 零 durable 业务写）：Ok(Some(reason))→persist→typed 等待收口（无 failed 节点，消息含判别码+「显式」出口+同 turn 语义；persist 失败/precheck 读取失败→原 ProviderErr fail-turn 语义）；③ fresh 模式 prompt 走 fresh builder（漂移→ValidationReject，与恢复面 source-hash 失配同族；其余装配失败仍 ProviderErr）；④ fresh 模式 input 走 `build_work_item_plan_streaming_input_fresh`（resume_provider_session_id=None 结构保证），Ordinary 保持既有 resume input 不变；⑤ 完成器/节点/launch/split-run 收口链零改动（B2b1 prepared-map/Resolved 机制对 LC 生产链零改动生效） |
| `web/workspace_ws_handler/run/gateway_start.rs`（§0.1） | `lc_fresh_dispatch_kind` 新增 HumanGateScManualRevision 放行臂 + `verify_lc_fresh_sc_manual_revision_payload`：turn 存在且非终态、payload command_id/source_hash/attempt_no/budget_reserved==durable turn、reservation key 在 durable ledger、source_revision_ref==record ref；漂移→`lc_resume_recovery_stale` 回 Waiting，store 读取失败→store_unavailable 家族；覆盖面 doc 注释更新（author 族仍 typed 拒） |
| `tests/planning_resume.rs`（§0.2） | t13⑤ 与 anchor_t16 未接管负例换载 Author tag（断言原样：context_unsupported、operation Failed、回 Waiting、零 spawn） |

## 2. 测试（Test 文件：part_02.rs / thin_rows.rs）

- **engine（part_02.rs）**：`anchor_t18_manual_mismatch_wait_preserves_turn_gate_budget_and_blocks_recovery`（mismatch→Waiting record 全字段对账；turn Reserved/attempt/budget/failure_class 不变；gate 快照预算 1 不二次扣；ledger 不追加；BlockedByLcRecovery 跳过且 attempt 不增；Resolved 对照→ResumeSameTurn{1} 回归）；`anchor_t18_manual_same_turn_completer_after_wait_keeps_turn_gate_budget`（等待后同 turn 完成器：同 turn_id/command/source_hash、Completed、attempt/budget 不变、门预算接续、回 WaitingForHuman）。
- **web e2e（thin_rows.rs）**：`anchor_t18_manual_turn_fresh_preserves_turn_gate_and_budget`（campaign 真链）：①开 turn（预算 2→1）→②seed 旧 native+tombstone→③重连#1 ResumeSameTurn{1}（漂移检出前语义不变）→spawn 修订臂→**mismatch typed 等待**（消息含 `resume_fingerprint_unavailable`+「显式」；turn Running 保持不 fail；Waiting recovery 全字段==turn 事实；零 provider start；ledger/预算不变；无新节点身份、旧 native last 节点幂等终态化携带 LC waiting 摘要）→④重连#2 BlockedByLcRecovery（attempt 不增、零 spawn）→⑤claim→dispatch（放行臂对照通过）→同 turn fresh（恰一次 start、**resume=None**、**prompt 含同 turn feedback**）→同 turn Completed（同 turn_id/command/source_hash、attempt/budget_reserved 不变）→账本/预算不重臂、回 WaitingForHuman（原 human gate 预算接续）、新 candidate source ref（业务 compile 收口）。

## 3. 红→绿证据（/tmp/anchor_t18_red_compile.txt、/tmp/anchor_t18_red_behavior.txt 留存）

1. **编译红（测试先行）**：两测试文件落盘后 `cargo test --locked --lib anchor_t18` → **E0599×4**（`precheck_sc_manual_revision_resume`×2、`persist_sc_manual_revision_superseded_wait`、`BlockedByLcRecovery` 变体不存在）+E0433（ProviderCompletion 作用域，测试侧修复）；错误类型级联的 E0277/E0614 随方法落地消失。
2. **行为红（引擎面落地、web 接线未动）**：`cargo test --locked --lib anchor_t18` → 2 passed；**1 failed**——e2e 在「mismatch 等待消息必须携带稳定判别码」FAIL：现臂以 generic `native_resume_candidate_frozen_binding_unavailable` 落 HumanGateTurnFailed（turn 被 fail），既无 typed 等待帧也无等待 record——正是设计 §5.3 SC 纪律的待实现缺口，红点精确。
3. **绿**：web 接线后 `cargo test --locked --lib anchor_t18` → **3 passed**（红→绿闭环）。

## 4. 范围回归（2026-10-12 实测，全部 `--locked --lib`）

- `anchor_t` → 72 passed（t13⑤/t16 换载后全族含 t15/t16/t17）
- `conversational_gate_revision` → 39 passed（plan §回归表 5.6 行）
- `conversational_gate` → 128 passed（恢复面原用例含 CompletedRevision/事件前缀）
- `campaign_stage3_recovery_matrix` → 9 passed（thin_rows 三薄行原断言不变）
- `planning_resume` → 17 passed
- `gateway_start` → 18 passed
- `single_candidate` → 166 passed（5.5 面不回归）
- `human_gate` → 34 passed；`provider_run` → 52 passed
- 写面文件 rustfmt 清洁（HEAD 基线干净，全部偏差为本次引入并已格式化；复跑绿）

## 5. commit

- `git add` 显式 9 文件（7 独占清单 − 2 未触碰(human_gate_revision.rs/revision_turn.inc.rs 零改动) + 2 §0 延伸面）+ 消息 `fix: B2d SC manual同turn/hash/gate/预算`；本报告按纪律不 add。

## 6. 移交（8.x 收口面）

- anchor_t18 断言「recovery 离开 Waiting」采用 `assert_ne!(Waiting)`（Legacy campaign 夹具无 split-run Resolved 收口；LC 生产链由 `close_plan_split_run_with_markdown→finalize_launch_after_success` 收口 Resolved——与 5.5 同构，8.1 真链验收时可用 t17 同型 `new_logical` 夹具补 Resolved 断言）。
- fresh 臂内 candidate 漂移（等待期候选被换代）按 ValidationReject 收口（与恢复面 source-hash 失配同族）；dispatch 面漂移为 `lc_resume_recovery_stale` 回 Waiting——两口径均已实现，未单列负例测试（验收未列；8.1 若需可按 verify 函数对称补）。
- 未知 tag dispatch 拒面现仅剩 author 族四 tag（Author/AuthorChoiceFollowup/AuthorFeedback/ReviewerRevision，归 B1 author fresh 合约面）；换载负例已用 Author 钉住。
