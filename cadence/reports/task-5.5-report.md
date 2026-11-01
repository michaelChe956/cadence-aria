# Task 5.5(B2c:SC author 同 reservation 与成员 compile 回归)实施报告

- 实施:ImplAnchorB55(glm53-task worker)
- worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`(共享树)
- 基线 HEAD:`4620cb5d`(fix: B2b2 legacy outline反馈的fullfresh;含前置 5.4 `7035a193`)
- 状态:**未完成(内部预算耗尽,见 §5)**——TDD Step 1/2 完成且红绿证据留存;Step 3 仅 fresh 返修 prompt builder 落地;主实现(NativeSupersededWaiting/recovery entry mode/precheck 前置/臂与 dispatch 接线)未动工。无 commit。

## 1. 已落地(全部在本包 7 文件写面 + §0 裁定的两个测试子文件)

| 文件 | 变更 |
|---|---|
| `src/product/work_item_split_engine/prompts/sc_revision.rs` | 新 `build_work_item_plan_markdown_fresh_revision_prompt(request,issue,repository,context,review,previous_source)`:SC 显式 fresh 返修轮 prompt——首轮 prompt 前缀 + `[previous_source]`(durable 上一版 source 全文,围栏内嵌,「不依赖任何历史会话」声明)先于既有 `[review_revision]` findings 回灌段 + 共享输出指令居尾唯一;硬预算(65_536B)超限 Err「exceeds hard budget」;正常 resume 修订 prompt 与首轮零变化 |
| `src/product/work_item_split_engine/prompts.rs` | re-export 上述 builder(pub(crate) use,与既有 revision builder 同行惯例) |
| `src/web/workspace_ws_handler/tests/single_candidate_provider_run/anchor_t17.rs`(新,§0 裁定) | ① `anchor_t17_sc_business_material_and_compile_target_do_not_follow_native_anchor`:多成员 LC(focus=FirstMember+聚合根规则在场)full SC 生成——成员 0 checkout language.md 原文进 prompt/聚合根规则不进;working_dir=canonical 成员 checkout(SC 材料锚)、working_directory=canonical 聚合根(native envelope,REQ-PLN-08);IR items[0].target_repository_id==issue.repo_id(成员物理仓)且≠`lc_aggregate_root_view_*`;确认 compile(Approval→Final Compile→Completed/Confirmed/plan.work_item_ids==1/phase Completed)。② `anchor_t17_sc_fresh_reuses_logical_reservation_without_rearm`:seed 漂移形态(Generate+ledger key0+旧 native conversation+tombstone+上一版 source+pending revise verdict)→ Ordinary StartGeneration 应得 `resume_fingerprint_unavailable` typed 等待帧(含「显式」出口)、phase 仍 Generate、ledger 字节不变、recovery Waiting(sc_author:reservation key/source ref/content hash 全对账)→ claim → dispatch → fresh input resume=None+prompt 含 [previous_source]/source 全文/findings/「不依赖任何历史会话」→ Approval 后 ledger 仍字节不变、repairs_used=0、recovery Resolved+fresh_native_session_id=="t17-fresh-native"+fresh_role_run_seq Some、新 candidate source ref |
| `src/web/workspace_ws_handler/tests/single_candidate_provider_run.rs` | 尾部 include 挂载(大文件守卫拆分惯例) |
| `src/product/workspace_engine/tests/single_candidate/anchor_t17.rs`(新,§0 裁定) | `anchor_t17_fresh_revision_prompt_carries_previous_source_and_review_context`(自足性:previous_source 先于 review 段、review 全量、输出指令唯一居尾、**正常 resume 修订 prompt 不携带 [previous_source] 零变化**)+ `anchor_t17_fresh_revision_prompt_keeps_hard_budget`(超限 Err 可诊断) |
| `src/product/workspace_engine/tests/single_candidate.rs` | 尾部 include 挂载 |

## 2. 写面裁定备案(先读)

1. **两个测试子文件不在 tasks.md 5.5 独占清单**:两父测试文件均已超 1200 行且既有「大文件守卫(>1200 行)按 include! 拆分」注释惯例(single_candidate_provider_run.rs:1464、tests/single_candidate.rs:1143);按 5.3 报告 §0.2「清单 letter vs 结构必需、同目录同 face」先例处理,偏离仅此两子文件+两行 include。
2. **gateway_start.rs 的 sc_author 放行臂为验收必需但本包未动**(5.3 §0.1 同型:Main IRC 已裁定「验收权威于文件清单」;briefing 亦注明「gateway_start 面已释放」)——落归下一实施者。
3. **planning_resume.rs 两处负例换载未动**:t13⑤(~:845)与 `anchor_t16_untaken_tag_still_typed_rejected_no_fallback`(~:2044)现以 sc_author 为「未接管 tag」示例;sc_author 放行后须换载 `sc_manual_revision`(5.6 面),5.3 §0.4 同型换载先例。

## 3. 红→绿证据(/tmp/anchor_t17_red_compile.txt、/tmp/anchor_t17_red_behavior.txt 留存)

1. **编译红(测试先行)**:两测试子文件落盘后 `cargo test --locked --lib anchor_t17` → **E0425×2**(fresh 返修 prompt builder 不存在),过滤命中 4 个新用例。
2. **行为红(builder 落地、接线未动,主树)**:`cargo test --locked --lib anchor_t17` → **3 passed; 1 failed**——`anchor_t17_sc_fresh_reuses_logical_reservation_without_rearm` 在 `anchor_t17.rs:82` FAIL("outbound channel must stay open"):现状 mismatch 重入被**静默 Superseded 让位**(reserve 先行:key0 已在 ledger → should_start=false → Superseded 零广播),既无 typed 等待帧也无 sc_author 捕获——正是设计 §5.3 SC 纪律①②的待实现缺口,红点精确。
3. 主锚回归(`..._business_material_and_compile_target_...`)现行为绿(B1/5.2 已修正的链路,本包回归钉住);引擎 fresh 返修 prompt 契约 2 用例绿(builder 已实现)。

## 4. 未实施(设计 §5.3 SC 纪律①② + plan Interfaces,实现要点已定稿见 §6)

- `SingleCandidateProviderRunError::NativeSupersededWaiting`(web run/single_candidate.rs)
- `SingleCandidateAuthorRecoveryEntry{Ordinary, LcFreshContext}`:Ordinary=precheck(`provider_resume_session_candidate`,`FrozenBindingUnavailable`→mismatch 家族,reason=`resume_fingerprint_unavailable`)先于 `reserve_single_candidate_author_start`;LcFreshContext=绕过 reserve/rearm、`build_work_item_plan_streaming_input_fresh`(resume=None)、verdict 在场时走新 fresh 返修 prompt(装载 payload/durable source ref 的上一版全文)
- 引擎捕获面 `persist_single_candidate_author_superseded_wait`(product/workspace_engine/single_candidate.rs):手写 ScAuthor record(被取代 native id 取 conversation 引用,**不可复用** `persist_lc_superseded_wait`——其内部 `provider_resume_session_candidate` 在 mismatch 场景自身必败)→ CAS Waiting → clear_superseded_provider_session + reconcile_lc_resume_waiting(phase/ledger 不动)
- provider_run.rs SC 臂:`lc_fresh_recovery_record_if_running`+ScAuthor tag → LcFreshContext;新错误臂按 AdmissionWaiting 同型收口(无 failed 节点,等待消息上浮)
- gateway_start.rs:sc_author 放行臂 + `verify`(payload phase/reservation key/source ref 对 durable typed 校验,stale→`lc_resume_recovery_stale`);planning_resume.rs 两负例换载 sc_manual_revision
- 绿化+范围回归(anchor_t 全族/planning_resume/gateway_start/workspace_engine 全量)+ 独立审查 + 独立 commit(`fix: B2c SC author同reservation与成员compile回归`,plan Step 5 显式清单)

## 5. 中止原因

内部请求预算(软上限 200)在红阶段收尾时耗尽,系统通知要求立即 wrap up;实现主体未开工,不敢以半成品冒充完成。**未 stage/未 commit 任何文件**;工作树保留 §1 全部改动与他人面未提交改动(131 文件 rustfmt 类,归属分离)。

## 6. 移交(下一实施者直接续作)

- 测试与 builder 已就位且红点精确(§3.2);按 §4 清单实施即可绿化,预期 `cargo test --locked --lib anchor_t17` → 4 passed。
- 关键陷阱三枚:①`persist_lc_superseded_wait` 不可直接复用(§4);②fresh 重入勿再 reserve(phase=Generate 时同 key 重放返回 false 会误判 Superseded);③Test B 的 mismatch 阶段 StartGeneration 用哑 provider 占位(precheck 先于 spawn,断言零启动)。
- B2b1 的 prepared-map/Resolved 收口机制对 fresh 链零改动即生效(begin_plan_split_run_if_logical→persist_prepared_launch_identity→finalize_launch_after_success,已核实 t16 同型)。
- 本报告文件按纪律不 git add(untracked)。

---

## 7. 续作收口（ImplAnchorB55b，2026-10-12，commit `54732928`）

§6 移交清单全量落地，`cargo test --locked --lib anchor_t17` → **4 passed**（红点 anchor_t17.rs:82 → 绿）。

### 7.1 实施明细（§4 清单逐项）

| 面 | 落地 |
|---|---|
| `run/single_candidate.rs`（web） | `SingleCandidateProviderRunError::NativeSupersededWaiting{reason_code,superseded_native_session_id,recovery_id}` 独立 typed 等待；`SingleCandidateAuthorRecoveryEntry{Ordinary,LcFreshContext{source_revision_ref}}` 入口形态；Ordinary：`precheck_single_candidate_author_resume` 先于 reserve（FrozenBindingUnavailable→persist→NativeSupersededWaiting；其余错误原样上浮=既有语义）；LcFreshContext：跳过 precheck/reserve（trap②）、verdict 在场按 payload ref 装载上一版全文走 fresh 返修 builder、`build_work_item_plan_streaming_input_fresh`（resume=None 结构保证） |
| `workspace_engine/single_candidate.rs`（引擎） | `precheck_single_candidate_author_resume`（出生/健康 resume=Ok(None) 零变化）+ `persist_single_candidate_author_superseded_wait`：手写 ScAuthor tagged record（superseded native 取 conversation 引用——不复用 persist_lc_superseded_wait，trap①）、原 key=ledger 末位 SC 键、source hash=durable content hash、CAS Waiting→clear_superseded_provider_session→reconcile_lc_resume_waiting（phase/ledger 不动）；`SC_AUTHOR_NATIVE_MISMATCH_WAITING_REASON`=`RESUME_FINGERPRINT_UNAVAILABLE` |
| `run/provider_run.rs` SC 臂 | FreshRunning+ScAuthor tag→LcFreshContext（读失败按 Message 面收口）；NativeSupersededWaiting 新错误臂=AdmissionWaiting 同型（无 failed 节点、finish_run、消息含判别码+「显式」出口） |
| `run/gateway_start.rs`（延伸面，§0 裁定②） | `lc_fresh_dispatch_kind` 新增 sc_author 放行臂 + `verify_lc_fresh_sc_author_payload`（payload phase/reservation key/source ref 对 durable typed 对照，漂移→`lc_resume_recovery_stale`，读取失败→store_unavailable）；覆盖面 doc 注释更新 |
| `tests/planning_resume.rs`（延伸面，§0 裁定③） | t13⑤ 与 `anchor_t16_untaken_tag_still_typed_rejected_no_fallback` 两负例换载 `sc_manual_revision`（5.6 面未接管，仍 typed 拒零 spawn） |

### 7.2 前任测试的两处夹具修正（合同语义不变）

1. **Test B 种子补齐 repair 形态**：原种子仅设 `work_item_plan_source_revision_ref`（IR/report 缺位），该形态 `{Generate, source Some, IR None, report None}` 无生产可达路径（refs 由上轮 complete/evaluation 原子写入），fresh 完成 CAS（is_repair_generation 需三 refs 齐备）必 Conflict。补 `plan_candidate_ir_ref`/`mechanical_report_ref` 后为生产可达 repair 形态，全部断言原样。
2. **Test B fresh provider 接线改 t15 同型**：原实现把 fresh provider 只注册进 dispatch registry，但 LC 会话物理启动唯一经 gateway（`lookup_real_streaming_adapter` 用 gateway 自有 registry），input/native 观测面必须落在 fixture gateway author（t15 先例）。改为 `new_logical(true, fresh_provider)` 同源注册，断言全保留。

### 7.3 过程事故与恢复（如实记录）

- t13⑤ 换载时我的替换块误吞了原 swap 的 `compare_and_save_lc_resume_state(&before_swap, …)` 持久化调用，导致该用例红（dispatch 读回旧 tag→B2b2 放行臂放行 outline revision→无拒帧）；定位后补回，单测/全族恢复绿。
- 一次 rustfmt 全文件重排波及清单内文件的他人未格式化区域；已 revert 后重放自有编辑——gateway_start.rs/planning_resume.rs 中仍有 ~130/~160 行 rustfmt 机械搭车（清单内文件、纯格式产物，B0.4「fmt 搭车=纯 rustfmt 机械产物」先例报备）。

### 7.4 验收证据（2026-10-12 实测）

- `cargo test --locked --lib anchor_t17` → 4 passed（红→绿闭环；红证据 §3）
- `cargo test --locked --lib anchor_t` → 69 passed（t13⑤/t16 换载后全族含）
- `cargo test --locked --lib single_candidate` → 166 passed
- `cargo test --locked --lib planning_resume` → 17 passed
- SC 真链主锚（Test A：成员 language.md→IR physical→确认 compile→Completed）在 anchor_t17 内保持绿
- commit：`54732928`（单包含前任未提交 prompt builder 成果；11 文件 +1544/−232；报告文件按纪律未 add）

### 7.5 移交 5.6（B2d）

- sc_manual_revision 仍为未接管 tag（t13⑤/t16 两负例现以它为「未接管」示例，5.6 接管后需再换载或转正路覆盖）；
- `SC_AUTHOR_NATIVE_MISMATCH_WAITING_REASON` 常量与 `SingleCandidateAuthorRecoveryEntry` 位于 web/engine 两侧写面文件，5.6 的 conversational_gate_recovery 面可直接消费；
- 疑点无：全部范围回归绿。
