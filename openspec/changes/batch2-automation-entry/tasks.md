# Tasks

本文件按总览 §3/§4 拆分 **16 个编号包（E1 并入 R4，实际派发 15 包，不得写 14）**。每包含：目标、独占文件（只此包写）、禁碰、TDD 验收（先红后绿）、规模（S≤1h/M≤2h/L≤4h，均≤4h）、波次（同波无共享文件才并行；共享文件按总览 §4/§5 与契约 C 移交表串行）。全部包默认 glm53-task。worker 不跑构建/测试/格式化；测试产物只在 `/tmp`；统一验收由 controller 在全部包合入后执行（命令与红测展开见 `cadence/plans/2026-10-16_实施计划_批量2自动化入口_v1.0.md`）。路径均相对 W（本 worktree 根）。

store 串行链（红线）：`models/automation.rs`+`issue_automation_store.rs` 按 P1→N0→R1→P4→T1 顺序移交；`workspace.rs` P2→P4→T2；`workspace-cockpit-projection.ts` R3→P5→T4；`CockpitInbox.tsx` P5→T4；`manager/mod.rs` T2→T3；`manager/runs.rs` T2→N1；`router.rs` P4→N1。

## 波 1（串行链：P1 后端→R3 类型子提交→P1 前端→P2→N0→P3）

### B2-P1 approval 冻结与 binding/rebind 放行（M，波 1；REQ-WIGA-02）

- **目标**：`EnrollmentOptions` 加 `plan_approval` 枚举（缺省 manual、冻结/幂等比较含选项）；P1 一并完成 `automation_enrollment.rs` binding/rebind 的期望 run_policy 校验与 auto 放行（不再交 P2 越独占文件）；FE 入口提交 auto 并禁用旧确认复选框。**包内分段**：P1 后端完成→R3 前置类型子提交（见 R3）→P1 才改前端入口/弹窗。
- **独占文件**：`src/product/models/automation.rs`、`src/product/issue_automation_store.rs`、`src/web/handlers/automation_enrollment.rs`；前端 `web/src/hooks/useIssueLifecycleGeneration.ts`、`web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx`（本包仅 approval 入口段；`lifecycle.ts` 类型归 R3）。
- **禁碰**：`plan_preparation.rs`/`lifecycle_store/workspace.rs`（P2）、块 1 advance/start、块 3 终止；不提交引用未定义类型的前端。
- **TDD 验收**：①新授权冻结 auto、旧缺省 manual、同 payload 幂等（红：现无选项字段）；②binding/rebind 只放行与当前 enabled 精确绑定且冻结策略一致的 AutoIfValid，错绑/降级绑定拒绝；③手动复选框行为不变；④`require_execution_plan_confirm` 在 auto 授权物化 WI 内压 false。

### B2-R3 前端类型与集合 UI（S，波 1+2 分段；REQ-WIGA-01/02）

- **目标**：**波 1 类型前置子提交**——P1 后端合入后、P1 前端之前，仅补 `web/src/api/types/lifecycle.ts::EnrollmentOptions.plan_approval` 类型并立即交回 P1（不新增包、不并写）。**波 2 剩余**（R1 后）：集合 target kind、弹窗「已授权全部 N 个成员仓」文案/原样回传/无逐仓勾选、投影透传。
- **独占文件**：`web/src/api/types/lifecycle.ts`、`web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx`（集合 UI 子提交段）、`web/src/state/workspace-cockpit-projection.ts`（本包只补 approval/集合类型与投影；完成后移交 P5→T4，不交 N1）。
- **禁碰**：`issue_automation_store.rs`/后端（R1）；不越权改 P1 未移交的入口段。
- **TDD 验收**：①类型子提交后 P1 前端可引用 approval 选项（类型先红后绿）；②集合 target 显示 N 仓文案、回传原样集合、无勾选 UI；③手动流程不取 target；④未知 kind 透传不改。

### B2-P2 bound 创建/准入/确认投影贯通（M，波 1，P1 后；REQ-WIGA-02/03）

- **目标**：`plan_preparation.rs` bound 创建与 intent_request 取冻结策略；`lifecycle_store/workspace.rs::create_workspace_session_bound` 接受明确 AutoIfValid（保留完整身份比较）；`plan_generation.rs::admission_outcome` 接收与绑定核对过的期望策略；`plan_confirmed_info.rs` 按冻结策略互证（保留 compile/provenance 全部条件）；编排器不另建自动 approve 循环。
- **独占文件**：`src/web/handlers/lifecycle/plan_preparation.rs`、`src/product/lifecycle_store/workspace.rs`（用完交 P4→T2）、`src/web/plan_generation.rs`（后交 P4）、`src/web/plan_confirmed_info.rs` 及同名既有测试模块。
- **禁碰**：已归 P1 的 `automation_enrollment.rs`；块 1 advance/start；trust/compile（P3）。
- **TDD 验收**：①auto SC 正常到 Confirmed 且确认投影可 advance（红：现 Interactive-only 拒 AutoIfValid）；②错绑 AutoIfValid 拒绝；③Interactive/manual 回归不变。

### B2-N0 事实模型/store 与 terminated_at 纯字段（S，波 1，P2 后 P3 前；REQ-WIGA-09/12）

- **目标**：`models/automation.rs` 提供 `#[serde(default)] terminated_at: Option<String>`（RFC3339，旧 JSON None）纯模型字段与 `AutomationAttentionFact{key,kind,stage:plan|advance|coding_start|termination,plan_id?,session_id?,attempt_id?,reason_code,occurred_at,choice_id?,gate_id?}`；enrollment `attention_facts` 缺省空、不存已读/推送；`issue_automation_store.rs` 提供 `upsert_automation_attention_fact`/`resolve_automation_attention_fact`（精确绑定校验；同一 terminated 绑定可清旧事实/写清理失败，旧/别的绑定拒绝）。不含终止 CAS、不依赖 R1 集合类型。
- **独占文件**：`src/product/models/automation.rs`、`src/product/issue_automation_store.rs`（P1→N0 接手，后交 R1）。
- **禁碰**：R1 集合变体与七消费点；终止 CAS（T1）；任何调用方接线（P3/P4/R2/T3 各自包）。
- **TDD 验收**：①精确绑定写/清、稳定 key/time（无 checkpoint 首次冻结、重入保留）；②旧 JSON `facts=[]`/`terminated_at=None` 可读；③同一 terminated 绑定可清/写 termination 失败、旧绑定拒绝；④无伪 attempt、错误上抛；⑤纯模型/store 不引入集合类型依赖（编译面独立）。

### B2-P3 trust/compile 停点与 plan_question 写清（M，波 1，N0 后；REQ-WIGA-02/12）

- **目标**：自动 compile 前复用 `lifecycle.rs::ensure_confirmation_trust_ready`；非 Ready 返回 typed `ConfirmationTrustWaiting` 时调 N0 接口 upsert `plan_question(stage=plan)`（保留原 reason/session 目标、零 compile 副作用）；compile 失败打开恢复门时同由本包 upsert（focus 取真实 gate）；trust Ready/恢复门成功关闭时 resolve 对应 key，不重跑已提交 compile、不伪造 gate；`finalizer.rs:244-265` WI 确认快照 auto=false/manual 原值回归不改 finalizer。
- **独占文件**：`src/product/workspace_engine/compile.rs`、`src/product/workspace_engine/lifecycle.rs`、`src/product/workspace_engine/compile/single_candidate.rs` 及 compile 现有测试。
- **禁碰**：N0 store 本体（只调用不修改）；其余 plan 事实（P4）；终止（T 包）。
- **TDD 验收**：①trust 非 Ready 不 compile 且持久 plan_question（未开页面仍可通知）（红：现自动臂无停点事实）；②Ready resolve 后自动 Confirmed；③compile 恢复门打开落 plan_question/真实 gate、恢复成功关闭即 resolve；④稳定 key/time、重复 upsert 不增项、写失败上抛；⑤auto WI 确认 false、manual true 仍拦。

## 波 2

### B2-R1 成员集合 target 全切换（L，波 2，P3 后；REQ-WIGA-01/02、REQ-MTG-03）

- **目标**：`LogicalCodebaseMembers` 变体＋`types.rs` 统一反序列化规范化（原始 JSON/磁盘、乱序/重复归一、空集/非法 UUID 拒、单元素折叠旧 kind）；GET/PUT/rebind/gateway 集合支持；七消费点逐项处理（lower 拒集合 WI、validator 授权上界+`member_targets` 映射、finalizer attribution 拒集合+child 复制完整授权、projection 不造归属、gateway 单次 LC 预检、coding.rs 集合 child 测试、rebind 规范化后全等）；三生产面（single_candidate×2、revision_turn）传真实 binding+映射；advance/start 准入以 P⊆A 互证替代「恰一桶」（Ready/Replay 均 record target_attempts 与 P 一一对应、attempt 唯一）；split 传 start_policy、`AdvanceRecord.start_run_policy` 冻结（`persist_advance_record_if_absent` 显式接策略参数、迁移全部调用不留 shim；retry 从 record 取冻结值）。
- **独占文件**：`src/product/logical_codebase/types.rs`、`src/product/issue_automation_store.rs`（N0 接手）、`src/product/work_item_plan_compiler/{lower,validate,types}.rs`、`src/product/workspace_engine/{compile/finalizer,plan_projection,single_candidate,advance_split}.rs`、`src/product/workspace_engine/{conversational_gate_parts/revision_turn,advance_parts/initialize_inner}.inc.rs`、`src/web/workspace_ws_handler/run/single_candidate.rs`、`src/web/handlers/{automation_target,automation_enrollment,automation_gateway_preflight}.rs`、`src/web/{advance_plan,coding_start}.rs`；另独占 `src/product/advance_store.rs`、`src/product/workspace_engine/advance.rs`。fixture 同步面（仅补 Manual 参数/字段）：`src/product/{coding_workspace_engine/advance_ready_only.rs,lifecycle_store/tests/cascade_d1_1.rs,workspace_engine/tests/advance_handler.rs,workspace_engine/tests/advance_initialization.rs}`、`src/web/{plan_confirmed_info_c1_tests.rs,coding_ws_handler/socket/resumption.rs,handlers/lifecycle_tests_d2_2.inc.rs,handlers/coding/advance_tests.rs}`。测试独占：`work_item_plan_compiler/tests/{member_targets,c1_existing_create,full_lowering_validator,preflight_baseline_paths,preflight_options}.rs`、`issue_automation_store/target_union_rebind_tests.inc.rs`、`workspace_engine/tests/{single_candidate,conversational_gate_recovery,advance_split_targets,part_03/park1_work_item_records}.rs`、`work_item_split_engine/tests/prompt_contract_weak_model.rs`、`web/workspace_ws_handler/tests/c1_recovery_surface.rs`、`web/coding_start_parts/tests.inc.rs`。
- **禁碰**：`plan_start.rs`/`autopilot_orchestrator.rs`（R2）、前端（R3 余量子提交在本包后）、`.aria`/4317/provider 实现/批 1 门与最终确认。
- **TDD 验收**：①原始 JSON 反序列化覆盖乱序/重复归一、空集/非法 UUID 拒、单元素折叠、磁盘旧 wire（红测从 `serde_json::from_value` 进入，不只测构造函数）；②digest/selection_key/bound.target 规范化后相等；③三生产 binding+映射装载失败走诊断不以 None 吞错；④单 WI 集合/越权拒；⑤GET/PUT 四成员匹配、单次 LC 预检；⑥4 桶 Ready/Replay＋AutoStartOnce＋各自 journal；⑦初始化失败继续/replay 保持 AutoStartOnce、旧 Manual 不升格；⑧LC 单桶 Ready 有真实一条 binding。

### B2-R3（剩余集合 UI 子提交，波 2，R1 后；REQ-WIGA-01）

见波 1 条目：同一 R3 包的第二个子提交，在 R1 合入后执行（集合 target UI/投影），与 R2 并行（无共享文件）。

## 波 3

### B2-R2 共享驱动与全图 fan-out（M，波 3，R1 后；REQ-WIGA-03/C5-CHAIN、REQ-MTG-03、REQ-WIGA-12）

- **目标**：`plan_start.rs` 提取 crate 内共享服务（prepare→Enrolled advance→bind_ready→mark_start_requested→Enrolled start→真实 outcome）；`autopilot_orchestrator.rs` LC 改调共享服务、沿 `deterministic_start_command_id`、删 `wiga-start-{attempt}` 旁路；首项 outcome 落盘即调 `drive_ready_units_for_plan` fan-out 其余就绪桶，tick/完成挂点复用；Created 按 attempt 冻结 policy 选 origin、Running parked 物化前复核授权；`coding_chain_stage` 改聚合 run 冻结 targets 新鲜状态；每 tick 先读 N0 未解除失败事实停点；错误调 N0 写 `automation_failed(stage=advance|coding_start)`（key 取 durable command/checkpoint、time 取 record.updated_at/claim.claimed_at，无 checkpoint 首次冻结）；终止围栏由 T 包后置。
- **独占文件**：`src/web/handlers/coding/plan_start.rs`、`src/web/autopilot_orchestrator.rs`、`src/web/handlers/{coding,mod}.rs`（后两者仅导出/既有 child 测试）；测试 `web/handlers/coding/plan_start_tests.rs`。
- **禁碰**：store 本体（N0/R1/P4 串行链）；`advance_plan.rs`/`coding_start.rs`（R1 产物只调用）；块 2/3 未移交文件。
- **TDD 验收**：①自动 LC 先落 run 再开所有独立根（A→B）（红：现只读唯一 attempt）；②首桶及非首桶 AutoStartOnce 均 Enrolled origin，逐桶核 policy/enrollment/binding；③撤销/篡改零 provider、禁止降级 Manual；④tick/完成同 claim 重复零启动；⑤物理单仓/Manual 原链回归；⑥advance/coding_start 失败写 N0 事实（稳定 key、无 checkpoint 冻结）。

### B2-P4 plan 继续/control/接管拒绝/其余 plan 事实（M，波 3，与 R2 并行仅限无交集面；REQ-WIGA-10/12、REQ-WIGA-02/08）

- **目标**：`workspace_sc_recovery.rs` 新增 continue-plan handler 与 control GET（DTO/路由在 `types.rs`/`app.rs`）；`workspace_single_candidate.rs` 绑定专用 StoppedNeedsHuman 原地重开、`conversational_gate.rs`/`decisions.rs` 复用候选修订；Failed 重臂把旧 generation intent 重置 Claimed 再推进；`workspace_session.rs::workspace_session_takeover` 与 `lifecycle_store/workspace.rs` 落实 enabled 精确绑定 takeover 拒绝（既有码 `workspace_session_takeover_not_allowed`）；接手 `workspace_session/router.rs` 写其余 plan 事实（choice/HumanGateOpened/durable 停点 upsert `plan_question`/`automation_failed`/`review_limit`）与 `plan_generation.rs:99-110` 派发前失败（intent NeedsHuman、EngineStarted/ProviderDispatched 无活 run 转 NeedsHuman）写 `automation_failed`；成功继续/关门 resolve；读 N0 `terminated_at` 拒绝非空，不依赖 T 包。
- **独占文件**：`src/web/handlers/workspace_sc_recovery.rs`、`src/product/lifecycle_store/workspace_single_candidate.rs`、`src/product/workspace_engine/conversational_gate.rs`、`src/web/workspace_ws_handler/decisions.rs`、`src/web/{app.rs,types.rs}`（继续/control 段）、`src/web/handlers/workspace_session.rs`、`src/web/workspace_session/router.rs`（后交 N1）、`src/web/plan_generation.rs`（P2 接手）、`src/product/lifecycle_store/workspace.rs`（P2 接手，用完交 T2）。
- **禁碰**：P3 的 trust/compile 事实（不重复写）；N0/R1 store 本体（只调用）；终止路由（T3）；`ChatCockpitPage` 等 P5 文件。
- **TDD 验收**：①Failed/超限说明同 session/auto 继续（红：现 retry 只收 Interactive）；②同 session CAS WaitingForHuman＋恢复 HumanConfirm＋TurnOpened；③预算耗尽显式继续只开一个新说明回合；④control 真实 stop_key/stage/allowed_actions（无 takeover）；⑤同键不重复 note/派发、异负载拒；⑥terminated（N0 字段非空）拒绝继续；⑦精确绑定 takeover 既有码拒绝、非托管原样；⑧choice/人工门/provider/超限写事实；⑨`plan_generation.rs:99-110` 两分支写 automation_failed、无活窗口仍可通知、稳定 key 零重复 provider；⑩继续成功 resolve、写失败不假成功。

## 波 4

### B2-T1 终止 CAS 与准入（M，波 4，P4 后；REQ-WIGA-09）

- **目标**：N0/P4 移交后加终止结果与 `issue_automation_store.rs` 终止 CAS/命令账本及 Enable/rebind/continue 拒绝（不再首次引入 terminated_at）；`coding_plan_run.rs` 按 plan 失效入口复用现有核体；`coding_attempt_store/{admission.rs,gate.rs}` 对精确绑定已 terminated 计划拒绝执行。
- **独占文件**：`src/product/coding_plan_run.rs`、`src/product/coding_attempt_store/admission.rs`、`src/product/coding_attempt_store/gate.rs`（T1 后交 T2）及现有测试；store 终止段（P4→T1 接手）。
- **禁碰**：runner 取消/收口（T2）；REST/WS 路由（T3）；手动无绑定路径。
- **TDD 验收**：①terminated 不可 Enable/rebind/继续（红：现 Disable 后可 Enable 重开）；②无 run 也关闸；③manual 不拦；④Invalidated 拒 provider 驱动。

### B2-T2 completion/专用收口/关全部门（M，波 4，T1 后；REQ-WIGA-09）

- **目标**：`manager/{mod,runs}.rs` 补真实 completion；`provider_run.rs` spawn 前重验、任务结束发 completion；`timeline.rs` 与 `workspace.rs` 防迟到覆盖 Terminated；`handoffs.rs` 提取专用收口（runner 全停后跳过 clean、保留文件、Completed 保留/其余 Aborted）；接 T1 `gate.rs` 关闭全部 open blocked/choice/stage 门（不伪造答案、不续跑）；**独占追加** `coding_run_registry.rs` 与 `coding_ws_handler/{runner_support.rs,gates.rs}`：终止发起或已 terminated 时 `registry.abort_attempt` 投递的内部 AbortAttempt 在 `runner_support:67-78`、stage 门 `gates.rs:202-211` 只干净退出/发 completion，不走旧 handle_abort/clean、不递归终止；取消等待不持 engine 锁。
- **独占文件**：`src/web/workspace_session/manager/{mod.rs,runs.rs}`（mod 完成后交 T3、runs 交 N1）、`src/web/workspace_ws_handler/run/provider_run.rs`、`src/product/workspace_engine/session_state/timeline.rs`、`src/product/lifecycle_store/workspace.rs`（P4 接手，终段）、`src/product/coding_workspace_engine/{handoffs.rs,gates_parts/blocked_gate.inc.rs}`、`src/web/state/coding_run_registry.rs`、`src/web/coding_ws_handler/{runner_support.rs,gates.rs}`。
- **禁碰**：REST/WS/收件箱路由（T3/T4）；手动 Abort/其他门动作语义。
- **TDD 验收**：①completion 真等待（非 `is_active_run()==false` 冒充）、迟到 Failed/Open 不覆盖 Terminated、已 Confirmed 不改；②**脏工作树下终止**：registry 内 AbortAttempt 不走旧 clean/handle_abort，全部 runner 干净退出后专用收口 Aborted、文件留存；③**stage 门等待中终止**：gates.rs AbortAttempt 干净退出、不旧 abort/不倒计时续跑/无 provider、专用收口取消门；④全部 open blocked/choice/stage 门关闭、手动 clean 原样；⑤发起终止与已 terminated 两状态均测、重复清理幂等。

### B2-P5 plan 对话恢复 UI（M，波 4，P4+R3 后，可与 T1/T2 并行；REQ-WIGA-10、REQ-WIGA-08）

- **目标**：`client.ts` continue-plan/control 调用；`ChatCockpitPage.tsx` Failed/停点的 completed 投影仍可输入；`ChatInputBar.tsx` 恢复输入/继续动作；接 R3 `workspace-cockpit-projection.ts` 管理态；`CockpitInbox.tsx` 按同源绑定隐藏托管会话接管按钮（不以仅隐藏代替服务端拒绝）。
- **独占文件**：`web/src/api/client.ts`、`web/src/pages/ChatCockpitPage.tsx`、`web/src/components/chat-workspace/ChatInputBar.tsx`、`web/src/state/workspace-cockpit-projection.ts`（R3 接手，用完交 T4）、`web/src/components/chat-workspace/cockpit/CockpitInbox.tsx`（完成后交 T4）及现有同名测试。
- **禁碰**：coding 页面/store/WS hook（T4）；后端。
- **TDD 验收**：①Failed 的 completed 投影仍可输入（红：现 Completed 不渲染输入）；②继续成功收敛到自动推进；③enabled 精确绑定的 stopped 卡无接管按钮、非托管仍保留；④不出现 coder 卡/暂停/转手动入口。

## 波 5

### B2-T3 统一终止路由/投影/continue-chain（M，波 5，T2+R2 后；REQ-WIGA-09/11、REQ-WIGA-08/12）

- **目标**：`automation_enrollment.rs` 抽取 crate 内统一 plan 终止服务供新 terminate REST、原 abort REST、gate-responses REST 与 WS 共用；未确认返回 `automation_plan_termination_required`（details 含 confirm_required/plan/enrollment/binding 身份、零副作用），确认后沿原 command_id 重放；`app.rs/types.rs` 路由/DTO；`autopilot_orchestrator.rs` 跳 terminated/未解除失败；接 T2 `manager/mod.rs::project_automation_ownership` 同源管理投影；REST `coding.rs::{abort_coding_attempt,post_coding_gate_response}` 与 WS `coding_ws_handler/{protocol,state,socket}.rs` Abort 路由；终止批量清当前绑定 plan 三类旧事实、清理失败写 `automation_failed(stage=termination)`；continue-chain REST（advance 用 R1 冻结 policy、start 仅重放原 claim、成功 resolve+wake）；借 R1 `advance_plan.rs`/`coding_start.rs` 与 P4 `workspace_sc_recovery.rs`；仅调用 T2 专用取消契约，不并写其文件。
- **独占文件**：`src/web/handlers/automation_enrollment.rs`（R1 接手）、`src/web/{app.rs,types.rs}`（终止/确认段）、`src/web/handlers/coding.rs`、`src/web/coding_ws_handler/{protocol.rs,state.rs,socket.rs}`、`src/web/autopilot_orchestrator.rs`（R2 接手）。
- **禁碰**：T2 独占的 registry/runner_support/gates；R1 `advance_plan.rs`/`coding_start.rs`（只调用）；P4 `workspace_sc_recovery.rs`（只调用）；A2 `plan_final_confirm.rs` 不改。
- **TDD 验收**：①原 abort/gate REST/WS 在 auto 单/多仓同 plan 二次确认，未确认零副作用（红：现直接单 attempt abort）；②步骤④复用 T2 干净退出分支，脏树/stage 等门均终止成功、无旧 handle_abort；③全停/同源投影（REST/WS 一致、terminated 仍 managed）；④清旧事实＋termination 失败重清后消失；⑤continue-chain 保留 auto/claim、重启不复活；⑥repeated terminate 幂等。

### B2-T4 前端统一终止与隐藏（M，波 5，T3+P5 后；REQ-WIGA-09、REQ-WIGA-08）

- **目标**：沿原 coding 管理接线补 terminate 调用与统一二次确认弹层（任意入口→同一确认→`confirmed:true`）；隐藏局部 abort/接管/restart（自动绑定）；门动作集合不变但 auto Abort 执行语义改整 plan；接 P5 `workspace-cockpit-projection.ts`/`CockpitInbox.tsx`，补 `CockpitInboxWaitingCards.tsx` 收件箱统一终止/二次确认；管理/终态投影进 store/WS hook。
- **独占文件**：`web/src/api/{client.ts,types/coding.ts}`（P5/T4 串行段）、`web/src/state/coding-workspace-store.ts`、`web/src/hooks/useCodingWorkspaceWs.ts`、`web/src/pages/{CodingWorkspacePage,CodingWorkspaceControls,CoderTriageCard,ChatCockpitPage}.tsx`（ChatCockpitPage 为 P5 接手段）、`web/src/components/coding-workspace/StageGateEntry.tsx`、`web/src/components/chat-workspace/cockpit/CockpitInboxWaitingCards.tsx` 及现有测试。
- **禁碰**：非 Abort 门动作；后端；P4/P5 已收口文件段。
- **TDD 验收**：①页面/收件箱共用二次确认、任意入口整 plan 动作（红：现局部 abort 直接发）；②未确认零请求副作用；③精确 enabled 绑定 stopped 卡不显示接管，终止后无接管/局部 abort/restart；④门动作为原集合但 auto Abort 走统一终止；⑤手动单 attempt abort 不变。

## 波 6

### B2-N1 通知 mapper 合并与失效题清理（M，波 6，T4 后；REQ-WIGA-12）

- **目标**：新 `src/web/automation_attention_info.rs` 只读 mapper（plan_question/automation_failed/review_limit→A1 DTO/workspace_session target）；`handlers/lifecycle.rs` 在既有 coding 事项后合并不递归调 GET；接 P4 `router.rs` ChoicePendingChanged 答后清题；接 T2 `runs.rs::finish_run` 及 `choices.rs` run 结束按 incarnation 清失效题（不误清 durable Failed/review_limit）；仅后端，不改前端透传。
- **独占文件**：新 `src/web/automation_attention_info.rs`、`src/web/mod.rs`、`src/web/handlers/lifecycle.rs`、`src/web/workspace_session/manager/choices.rs`（独占追加）、`src/web/workspace_session/manager/runs.rs`（T2 接手）、`src/web/workspace_session/router.rs`（P4 接手）。
- **禁碰**：`coding_attention_info.rs`（只 import DTO）；`lifecycle.ts`/projection 前端；不承担隐式写者。
- **TDD 验收**：①未 watched 也通知、稳定 key/正确 session target、原 7 类不变（红：现无三类投影）；②答后及 finish_run 后失效 choice 消失、不误清 Failed/review_limit；③terminate 后无幽灵门通知；④GET 只读不写事实。

### B2-R4/E1 四仓整链夹具验收（M，波 6，全包+块 2/3 后；REQ-WIGA-13、REQ-WIGA-09 负例；E1 并入本包同一实施/验收单元）

- **目标**：`wiga_gate_fixture.rs` 扩四仓夹具（含 automation_enrollment_test_support、`autopilot_orchestrator_parts/test_mods.inc.rs` 及新测试 include、表列注册点）；复用 `handlers/coding/{plan_final_confirm_tests.rs,plan_start_tests.rs}`；按块 3 §8 步骤构造零人工正例/故障/终止/手动回归用例（真实 provider 整链由 Main 执行，本包交付夹具与集成用例）。
- **独占文件**：`src/web/wiga_gate_fixture.rs`、`src/web/handlers/automation_enrollment_test_support.rs`、`src/web/autopilot_orchestrator_parts/test_mods.inc.rs` 及其新测试 include；`src/web/handlers/coding/{plan_final_confirm_tests.rs,plan_start_tests.rs}`（复用/加用例不改生产）。
- **禁碰**：一切生产调度/路由代码；A2 handler 本体。
- **TDD 验收**：①新 issue→自动生成/批准→各实际桶首启→跨仓依赖→批 1 唯一最终确认（四仓零人工正例，红：现仅单仓集成）；②选中无 WI 仓不造空 attempt；③终止后无新启；④terminated＋部分 Completed 最终确认不可点（A2 不改）；⑤故障用例（choice/provider/超限/挂起）与零人工正例分开；⑥Main 正式证据入 `cadence/reports/`。

## 收口（controller 执行，不派工）

- [ ] 统一验收：四条 cargo 标准命令＋web 三绿（禁 `-j`）；定向红/绿证据留存。
- [ ] 单仓与手动冒烟：单仓显式 auto 自动链＋统一终止；手动单/多仓确认/中止/门回归。
- [ ] 整链实测（块 3 §8 七步，真实 provider，独立 `/tmp` 夹具）：零人工正例与故障/终止用例分开。
- [ ] `openspec validate batch2-automation-entry --strict` 通过。
