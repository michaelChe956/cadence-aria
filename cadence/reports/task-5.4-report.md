# Task 5.4(B2b3:Draft Batch PlanRevision 同业务代恢复)实施报告

- 实施:ImplAnchorB54(glm53-task worker)
- worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 基线 HEAD:`9d30efda`(fix: B2b1 fresh_node绑定纪元写入与Resolved收口)
- 状态:实现完成;anchor_t16 红(编译红+行为红)→绿(8/8×4 连绿);范围化回归全绿;独立 commit `7035a193` 已落(§5)

## 1. 交付内容

| 文件 | 变更 |
|---|---|
| `src/product/workspace_engine/draft_batch/runs.rs` | ① draft/batch 构建重构为 `build_work_item_*_streaming_input_variant(..., explicit_fresh: bool)` 共享体(零行为变化,非 fresh 路径逐字段同原) ② 新 `build_current_work_item_draft_streaming_input_explicit_fresh` / `build_current_work_item_batch_draft_streaming_input_explicit_fresh`(&self;feedback=durable 冻结值原样透传,不读/不清/不 take 内存 `pending_revision_context`;经 `build_work_item_plan_streaming_input_fresh` 全新启动 resume=None) ③ 新 `begin_work_item_batch_fresh_recovery_run`:仅重建 `WorkItemBatchRun` provider 运行节点(不另建 batch、不触碰 repair 预算) |
| `src/web/workspace_ws_handler/run/gateway_start.rs`(Main 契约扩展写面,见 §2) | ① `lc_fresh_dispatch_kind` Kind 臂接管:legacy_draft/legacy_batch/legacy_plan_revision 三 tag 放行(仅此三 tag;5.3 的 outline_author/outline_revision 与 5.5/5.6 的 SC 族保持 catch-all typed 拒零 spawn,错误消息去掉 5.2-5.6 尾注) ② 新 `verify_lc_fresh_business_generation`:Draft/Batch 的 payload 业务代(generation_round_id/active_outline_id/batch_id)spawn 前按 durable active index typed 校验,漂移→`lc_resume_recovery_stale`(=`LcFreshClaimOutcome::STALE_SUPERSEDED`)拒收回等待;index 读取失败按 `lc_resume_store_unavailable` 家族 ③ `lc_fail_fresh_operation_to_waiting` 提权 pub(crate) 供臂内复用 |
| `src/web/workspace_ws_handler/run/provider_run.rs` | ① 新 `lc_fresh_recovery_record_if_running`(pub(super)):仅 `FreshRunning` 时返回恢复记录(臂内 fresh 判别;读取失败 fail-closed 上浮,不吞成正常 run) ② Draft 臂:fresh 时 feedback=payload 冻结 `pending_feedback`(不借内存 pending)、无活动 DraftRun 节点时 `start_serial_work_item_draft_run_for(payload.active_outline_id)` 重建(业务 generation 不重建)、构建全程(含 RetryOnce 轮)走 explicit fresh builder ③ Batch 臂:fresh 时 feedback=payload 冻结值、无活动 BatchRun 节点时 `begin_work_item_batch_fresh_recovery_run`、同 batch 完成器/预算(不改 `complete_work_item_batch_draft_author` 语义)、fresh builder |
| `src/web/workspace_ws_handler/run/followups.rs` | ① 新纯函数 `legacy_plan_revision_payload_matches_recompute`(pub(super)):retained 按序 item_id 对照、redo 按 old_item_id+feedback 精确对照(feedback 为捕获期同一重算的冻结值,不归一) ② PlanRevision 宏(`workspace_ws_work_item_plan_revision_arm!`):fresh 判别+feedback=payload 冻结 `overall_feedback`;重算 retained/redo 与 payload 对照失败→`lc_fail_fresh_operation_to_waiting` 收口(op Failed+recovery 回 Waiting+typed 帧 `lc_resume_recovery_stale`,零 provider start,不转 OutlineRebuild);stale 收口后无活动节点时重建 Revision 运行节点(与 `request_work_item_plan_revision` 节点形态同构,不要求 AuthorConfirm 前置);provider 全新启动走 `build_work_item_plan_streaming_input_fresh`(resume=None);正常 resume 路径原样 |
| `src/product/workspace_engine/tests/part_03/part_05.rs` | 新增 2 个 anchor_t16 engine 用例(§3) |
| `src/web/workspace_ws_handler/tests/planning_resume.rs` | 新增 anchor_t16 业务夹具(`AnchorT16PlanFixture`:T13 基座+draft/batch index/draft 记录/revision work items 落盘;`engine_after_stale_closure` 构造 stale 收口后形态:stage=PrepareContext/无活动节点/旧 Author native conversation 引用在场)+6 个 web 用例(§3) |

## 2. 写面裁定备案

- **gateway_start.rs 不在 tasks.md 5.4 独占清单,但为controller 派工契约明示项**(briefing Contract:"三 tag(Draft/Batch/PlanRevision)Kind 臂接管 full fresh"——Kind 臂即 `gateway_start.rs::lc_fresh_dispatch_kind`;5.1 报告 §7 移交同指此单点)。与 5.3 worker(ImplAnchorB53)双向确认串行:本包先落,其 outline_author/outline_revision 两 tag 放行叠层在我提交之后(git log `7035a193` 为准);本包 catch-all 对其两 tag 保持现拒收态,无交集。
- 共享文件 provider_run.rs/tests/planning_resume.rs:5.3 worker 确认其在两文件零未提交改动后开工;legacy_author.inc.rs/prompts.rs/prompt_contract.rs 未触碰(提交后该文件仍有 5.3 的在途未提交改动,归属正确分离)。

## 3. 新测试(filter `anchor_t16`,8 个)

web(planning_resume.rs):
1. `anchor_t16_legacy_draft_batch_revision_fresh_preserves_business_generation`(主用例):draft fresh→provider 经唯一启动口启动,resume=None(旧 native 引用在场证不回捞)、prompt 含 durable 冻结 feedback+outline 全量、generation/outline/批次零重建、FreshRunning+op Running、无重复 spawn。
2. `anchor_t16_legacy_batch_fresh_keeps_batch_and_budget`:batch 同上;batch_id/条目/状态同原、不另建 batch。
3. `anchor_t16_legacy_plan_revision_fresh_recomputes_matching_payload`:payload(组合 feedback=revert_feedback+"\n\n整体反馈: "+overall)与重算一致→启动;resume=None、prompt 含整体反馈。
4. `anchor_t16_fresh_draft_business_mismatch_waits_typed_stale`:payload round_9999 vs durable round_0001→typed 帧 `lc_resume_recovery_stale`、回 Waiting、op Failed、零 spawn。
5. `anchor_t16_fresh_plan_revision_payload_mismatch_waits_typed_stale`:payload redo feedback 漂移→臂内对照失败同收口(零 provider start)。
6. `anchor_t16_untaken_tag_still_typed_rejected_no_fallback`:sc_author(归 5.5)仍 `lc_resume_recovery_context_unsupported` 拒零 spawn不回落。
   (4/5/6 的零 spawn 判定为「窗口内无输入或通道无输入关闭」,排除 registry 随 run_context 释放时 recv=None 的误判。)

engine(part_03/part_05.rs):
7. `anchor_t16_draft_fresh_builder_and_node_rebuild_keep_generation`:无活动节点重建 DraftRun;fresh builder resume=None、prompt 含冻结 feedback 且**不含**内存残留 pending(payload=None 时同样不回落)、pending 未被 take;generation/outline/批次 before==after;`work_item_draft_repair_states` 保留(fresh 不重置修复预算)。
8. `anchor_t16_batch_fresh_builder_and_node_rebuild_keep_batch`:同型;仅重建节点,batch 列表/状态/条目同原。

## 4. 红→绿证据(/tmp/anchor_t16_red_compile.txt、/tmp/anchor_t16_green.txt、/tmp/t54_red_behavior.txt 留存)

1. **红①编译红(clean-cutover,4.1/5.2 同型)**:测试先行落盘后 `cargo check --lib --tests` → 4×E0599(三个新引擎接口不存在)。
2. **红②行为红(桩版,主树)**:签名真实+行为错桩(直委派非 fresh builder+no-op 节点+dispatch 未 flip)→`cargo test --locked --lib anchor_t16` → **0 passed; 8 failed**(全部新用例判别断言 FAIL:正向用例 provider input 超时、负向用例码不符、engine 用例 resume=Some/节点缺失)。
3. **绿(实现落地,主树)**:`cargo test --locked --lib anchor_t16` → **8 passed; 0 failed**,四连绿。排障记录:revision 正例首红根因=测试 payload 未按 build 侧「revert_feedback+整体反馈」组合串构造,臂内对照正确拒收(错误帧实证),修正 payload 后绿;零 spawn 三连曾flaky,根因为断言把「通道无输入关闭(Ok(None))」误判,修正判定形式后稳定。

## 5. 范围化回归(全绿,主树)

- `planning_resume` 16 passed(t13 族+t16 共存);`gateway_start` 18 passed(t14 map/纪元面不回归);`anchor_t` 全族 64 passed(t7–t16);`product::workspace_engine::tests` 全量 **1249 passed**;`cargo check --lib --tests` 0 error 且本包写面零告警。
- 隔离树 /tmp/t54-verify(HEAD=9d30efda+本包文件叠加)按 5.1 先例建立用于规避并行 5.3 prompts.rs 中间态;主树恢复可编译后全部验证回主树完成。

## 6. 提交

```bash
git add -- src/web/workspace_ws_handler/run/followups.rs src/product/workspace_engine/draft_batch/runs.rs src/web/workspace_ws_handler/run/provider_run.rs src/product/workspace_engine/tests/part_03/part_05.rs src/web/workspace_ws_handler/tests/planning_resume.rs src/web/workspace_ws_handler/run/gateway_start.rs
git commit -m "fix: B2b3 Draft Batch PlanRevision同业务代恢复"
```

- Commit:`7035a193`(6 文件 +1565/-41;gateway_start.rs 为 §2 备案的契约扩展项)。
- 本报告文件按纪律不 git add(untracked)。

## 7. 已知边界与移交

- 三 tag 的**捕获面**(draft/batch/revision 节点的 run_context 落盘)不在本包(5.1 捕获面仅 outline author/revision;`persist_planning_stale_wait_for_node` 扩臂归 manager/C1 面,测试以直接落 Waiting 记录验证消费侧,同 anchor_t13 先例)。
- fresh run 在 Logical 启动时的 prepared map/Resolved 对账由臂内既有 `begin_plan_split_run_if_logical`/`close_plan_split_run` 链自动消费(B2b1 机制,本包零改动);Legacy(direct)启动无 split run,map 面不适用(与 5.1/5.2 一致)。
- PlanRevision 臂内 mismatch 拒收路径不 drop provider_drive_guard/不 finish_run——与该宏既有错误路径同惯例(缺陷 #10 修复仅落在 SC 臂);如审查裁定需对齐,归 fixloop。
- 下一包(5.5 B2c):sc_author/sc_manual_revision 两 tag;`lc_fresh_recovery_record_if_running` 可直接复用;5.3 叠层 outline 两 tag 放行时,`verify_lc_fresh_business_generation` 的 `_ => Ok(())` 默认臂无需改(其两 tag 业务校验归 5.3 自理)。
