# Task 5.1(B2a:Plan stale 等待与全 tag dispatch + gateway_start native 启动接线)实施报告

- 实施:ImplAnchorB51(glm53-task worker)
- worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 基线 HEAD:`ef587e8d`(fix: C2 fresh sender与typed结果对账)
- 状态:实现完成;anchor_t13 红(隔离树)→绿(隔离树 16 连稳 + 主树复跑)与范围化回归全绿;独立 commit `15dc65e3`
- 独立 commit:已落(见 §5)

## 0. 隔离取证说明

主树 `cargo test --lib` 于本包会话中期被并行文件破坏(27 个编译错误均归属 `src/web/workspace_ws_handler/tests/register_trust_entry.rs` 未跟踪文件 + `tests.rs` 的 mod 声明,非本包写面)。按 Main 裁决建 `/tmp/t51-verify`(HEAD=ef587e8d detach,web/dist 软链主树产物),仅叠加本包 6 文件跑红/绿与回归;主树 `cargo check --lib` 零错误。

## 1. 交付内容(写面 = tasks.md 5.1 独占 4 文件 + Main 契约扩展的 gateway_start.rs/run 面)

| 文件 | 变更 |
|---|---|
| `src/web/workspace_ws_handler/socket.rs` | ① `planning_resume_run_kind` clean-cutover 为 `PlanningResumeLaunchDecision{ResumeExisting(ProviderRunKind),SupersededWaiting}` + `planning_resume_launch_decision`:StaleContext 不再自动转 `WorkItemPlanOutlineRebuild` spawn(B3 旧行为删除),漂移 0 自动 start;SameContext/None 沿用原 kind 续跑;SC author 透传保留(B2c/5.5 接管前原语义) ② 新稳定码 `LC_RESUME_RECOVERY_CONTEXT_UNSUPPORTED_CODE`="lc_resume_recovery_context_unsupported"(§5.3 未知/非法 tag 拒收;与 4.2 码表同族) ③ 接收循环:`handle_workspace_inbound_message` 之后对 fresh-intent `StartGeneration` 调 `dispatch_lc_fresh_recovery_run`(claim 后 tag dispatch 接线;同 ID Replay/Busy/Stale 仍由 4.2 入站门先收口) |
| `src/web/workspace_session/manager/durable_projection.rs` | ① `recover_on_creation` 在既有 lc_waiting_active 块后、human gate/outline/孤儿/僵尸自动链之前插入 `persist_planning_stale_wait_if_drifted` 预检(读侧判据与 recover_outline_run should_resume 同源:WorkItemPlan∧非 SC∧Running∧活动 OutlineRun 节点∧无活 run):Stale→0 spawn,捕获 run_context 落 `planning_context_stale` 等待+reconcile 收口(旧节点 Failed/active 清/run 阶段回 PrepareContext),此后 lc_waiting_active 同语义阻断一切自动接力(manual turn 不自动 resume/increment);捕获写失败 fail-closed(阻断+recovery_error 可观测,不 spawn 漂移 run) ② 新增 `persist_planning_stale_wait_for_node`:revision 节点→`LegacyOutlineRevision`(normalized_feedback 取 durable node detail 冻结值,任何 take 前;outline_ref/version 同源),首发/重建中断节点→`LegacyOutlineAuthor`;business 事实从 durable session/plan/repository 现值捕获,未知身份留 None 不倒算(original_role_run_seq=None——纪元前态,map/run 绑定归 5.2 B2b1) ③ `recover_outline_run` 消费 launch decision:SupersededWaiting→不 spawn(TOCTOU 安全臂,预检兜残余窗口) ④ `recover_on_creation` 可见性 pub(super)→pub(crate)(测试消费) |
| `src/web/workspace_ws_handler/run/gateway_start.rs` | ① `lc_fresh_run_kind_for_recovery_context`:§5.3 全 12 tag→`ProviderRunKind` 精确对照纯映射(feedback 冻结值原样透传;LegacyOutlineRebuild→`NeedsFreshPlanningContext`;非法 flow typed 拒,不回落 `work_item_plan_author_for_durable_flow` 通用兜底) ② `dispatch_lc_fresh_recovery_run`:claim(FreshReserved)后的 dispatch 入口——幂等仅推进 FreshReserved;整记录 CAS single-flight FreshReserved→FreshRunning(op Running);spawn 用 ClaimIfIdle(fresh 语义不 supersede 无关 run,§4.3 原业务 run/turn 保持);认领失败/spawn 失败按 §4.3 prepare 失败行收口(op failed+recovery 回 Waiting+typed 帧,不残留无 run 的 FreshRunning) ③ 覆盖面纪律(§4.5/§10.1):本波仅覆盖 `legacy_outline_rebuild`(重建仅显式 fresh 授权后——kind 由 dispatch 时新解析 resolved planning context 组装,native 启动经臂内 `start_work_item_plan_author` Logical 分支唯一启动口/B0 full prepare 消费/fresh 输入 resume_provider_session_id=None 旧 audit 不回捞);其余 tag full fresh 臂归 5.2–5.6,typed 拒零 spawn |
| `src/web/workspace_ws_handler/run.rs` | re-export:`dispatch_lc_fresh_recovery_run`(生产面,socket 消费)+ `LcFreshRunKind`/`LcFreshDispatchRejection`/`lc_fresh_run_kind_for_recovery_context`(cfg(test),测试消费) |
| `src/web/workspace_ws_handler/mod.rs` | socket 导出行更新:出 `planning_resume_run_kind`,入 `PlanningResumeLaunchDecision`/`planning_resume_launch_decision` |
| `src/web/workspace_ws_handler/tests/planning_resume.rs` | 旧 B3 行为两测试改写+新增 4 个 anchor_t13 用例(见 §2) |

## 2. 新测试(filter `anchor_t13`,4 个)

1. `anchor_t13_planning_resume_launch_decision_supersedes_waiting_on_stale`:SameContext/None→ResumeExisting 原 kind;StaleContext→SupersededWaiting(不得自动 spawn rebuild);SC author 漂移透传不改写。
2. `anchor_t13_plan_stale_waits_for_explicit_fresh`:物理仓+issue+plan+WorkItemPlan 会话+中断 outline revision 节点(durable detail 固化 feedback)+逻辑 codebase 漂移 → manager `recover_on_creation`:①stale 重连 0 自动 start(recording provider 通道零输入) ②durable 等待 state=Waiting/reason_code=planning_context_stale/捕获 `LegacyOutlineRevision.normalized_feedback`==原 revision feedback(take 前) ③旧节点收口(active 清空、stage 回 PrepareContext、等待 active) ④同存活 manager 二次恢复幂等仍 0 start、fresh_operations 空 ⑤显式 fresh claim(FreshReserved)后 dispatch 对未接管 tag typed 拒(protocol_error/lc_resume_recovery_context_unsupported),op failed(reason 同码)、recovery 回 Waiting、0 spawn。
3. `anchor_t13_plan_fresh_authorized_rebuild_starts_after_claim`:直接落 `LegacyOutlineRebuild`(origin=outline_revision 不丢 feedback)等待→claim→dispatch:经新 resolved planning context 组装 rebuild spawn,provider input 断言 worktree==聚合根、`resume_provider_session_id==None`(旧 audit 不回捞)、prompt 注入聚合 inventory;durable FreshRunning/op Running;幂等重放不二次 spawn。
4. `anchor_t13_plan_run_context_dispatch_never_falls_back`:§5.3 全 12 tag 精确映射断言(含 feedback 透传与空 feedback→None、ScManualRevision prompt 留空由臂内重装配);非法 flow(LegacyOutlineAuthor 携 SingleCandidate flow)typed 拒(code==lc_resume_recovery_context_unsupported),不产出通用 plan_author 兜底。

改写:原 `planning_resume_run_kind_rebuilds_on_stale_context_and_resumes_on_same`、`stale_context_rebuild_starts_new_outline_run_with_rebuilt_context` 断言的即本次裁定的删除对象(B3 自动 spawn),按新合同改写;`rebuilt_snapshot_committed_only_after_provider_start_success`(provider 启动成功后才 commit snapshot)与 commit 语义保留不动。

## 3. 设计对照与范围裁定

- Stale 0 自动 start/等待/收口:设计 §4.3 mismatch 行、§4.4 manager 唯一写方;plan Task 5.1 Step 3 全项。
- run_context 捕获(take 前):§5.3 legacy_outline_revision 行——normalized_feedback 即 durable detail 冻结值。
- tag 精确 dispatch/unknown 拒/不 plan_author 兜底:§5.3 表+范围外拒绝段;plan 验收"typedtagknown精确dispatch、unknown拒"。
- fresh 启动接线(Main 契约):native 经 `start_work_item_plan_author` Logical 分支唯一启动口(臂内既有,T6R2 分类桥同口)、B0 full prepare 唯一消费(dispatch 不新造第二条启动路径)、旧 audit 不回捞(rebuild 臂 fresh 输入 resume=None;旧 audit 仅由 4.1 `provider_resume_session_candidate` 资格过滤消费,即 B4.1 裁决消费面)。
- prepared-map-before-start 纪元前态:本包不写/不读 prepared 指纹 map,`original_role_run_seq=None`、dispatch 不绑 fresh_node_id/role_run_seq(map/native audit 同 run 绑定归 5.2 B2b1 anchor_t14);与 B4.1 张力裁决一致。
- A/B 发布纪律(§4.5/§10.1):未接管 tag typed 拒零 spawn;allowed_actions 恒空面(4.2)未动。

## 4. 红→绿证据

1. **红**(隔离树 `/tmp/t51-verify` = `git worktree add HEAD(detach=ef587e8d)` + 仅叠加本包文件,web/dist 软链主树产物;stub 骨架:launch decision 恒 ResumeExisting、mapping 恒 Err、dispatcher no-op、恢复链保持旧行为):
   `cargo test --locked --lib anchor_t13` → `0 passed; 4 failed; 4366 filtered out`——四用例全部按判别断言 FAIL:`StaleContext 必须等待显式 fresh(SupersededWaiting),不得自动 spawn rebuild`(stub 回 ResumeExisting)、`stale 重连不得自动 spawn,必须等待显式 fresh 授权`(旧链实际 spawn,日志可见 `[aria-cancellation] handler_run_supersede ... kind=WorkItemPlanOutlineRebuild{rebuilt:ResolvedPlanningContext{...}}`)、fresh rebuild 3s Elapsed(dispatcher no-op 零启动)、dispatch 映射全拒。过滤命中 4 新用例(非 0 tests/非 unsupported 绕过)。
2. **绿**(真实实现叠加,隔离树):`cargo test --locked --lib anchor_t13` → `4 passed; 0 failed`。稳定性:单用例 5/5、全 suite 6/6、插桩定位 1/1(修 worktree 断言为与 dispatch 同源自解析后),合计 16 连绿;期间 bg_404 出现过一次幂等窗口断言失败、其后 16 次未复现,已将该断言强化为双窗口 try_recv 形式(首轮后无残留 + 重放后 300ms 无新增),终版再全绿。
3. **主树复跑绿**(trust 方 `4c8b393f` 落地后):`cargo test --locked --lib anchor_t13` → `4 passed; 0 failed; 4378 filtered out`。

## 5. 提交

- Commit:`15dc65e3`(fix: B2a Plan stale等待与全tagdispatch;独立提交,5 文件 +1417/-263,显式列表 stage:socket.rs/durable_projection.rs/gateway_start.rs/run.rs/planning_resume.rs)
- 并行协调备案:`mod.rs` 的 socket 导出行变更(出 `planning_resume_run_kind`,入 `PlanningResumeLaunchDecision`/`planning_resume_launch_decision`)被并行 trust 提交 `e726e80d` 一并带走(共享文件,同 4.2 §1 并行协调先例);本包提交时该文件无 diff 不重复 stage。
- 本报告文件按纪律不 git add(untracked)。

## 6. 范围化回归(全绿)

- 隔离树(叠加本包文件):`web::workspace_ws_handler::tests::planning_resume` 10 passed;`anchor_t12` 6 passed(4.2 socket 门回归)。
- 主树(trust 落地后):`web::workspace_ws_handler::tests::gateway_start` 16 passed;`web::workspace_session` 56 passed;`interrupted_run` 7 passed;`author_revision` 30 passed;`single_candidate` 162 passed;`web::workspace_ws_handler` 全量 200 passed(4.2 时点 191+本包 4+并行 trust 增量)。

## 7. 下包移交(5.2 B2b1 / 5.3 B2b2+)

- dispatch 覆盖面开关在 `gateway_start.rs::lc_fresh_dispatch_kind`(单点):5.2–5.6 各包接管对应 tag 的 full fresh 臂后把 `LcFreshRunKind::Kind(_)` 拒收分支翻为 spawn(或按 §5.3 专用输入 mode);5.2 接 prepared map/native audit 同 run 绑定后补 `fresh_node_id`/`role_run_seq`/Resolved 终态对账(本包 FreshRunning 的完成收口未接——纪元前态,归 5.2+)。
- 捕获面 `persist_planning_stale_wait_for_node`(durable_projection.rs):outline revision/author 两 tag;其余族捕获随 5.3–5.6 各臂补(捕获判据与 §5.3 表对齐)。
- `PlanningResumeLaunchDecision`/`planning_resume_launch_decision`(socket.rs)为 resume 决策唯一归类口;5.5(B2c)接管 SC stale 等待时改 SC 透传分支。
- 稳定码 `lc_resume_recovery_context_unsupported` 已入 socket 码表(与 4.2 码族同列),前后端(C2/D)消费时同串。
