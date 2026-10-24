# Tasks

本文件只列高层工作包。精确实施步骤、独占文件、测试命令及串行写面窗口见 `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo/cadence/plans/2026-10-13_实施计划_级联删除_v1.1.md`（保留v1.0原稿）。本轮吸收接地与一致性审查，不实施、不commit；复选框待真实实施/独立审查/验证后勾。

## 1. R：报告身份先行移交（外部依赖，不重复实现）

- [ ] 1.1 R 消费 FixReportId 的同 payload 幂等、异报告新身份、两 writer 同规则、旧 immutable 严格校验的提交/独立审查与 Main 验证证据；明确 SG3a.0/source store/两 writer/fixture 串行移交窗口（保护 REQ-GRP-01、REQ-DEL-06，R 本身不属删除新功能）。

## 2. D1：拥有闭包、耐久事实与存储准入

- [ ] 2.1 D1.0 单条无内容purging/deleted记录、非阻塞try-shared write/try-exclusive delete、稳定repo resource准入、身份list-by-owner；pending manifest支持重入且成功移除，真实错误不吞（REQ-DEL-02/06/07）。
- [ ] 2.2 D1.1 实现真实 WI/session/review/run/专属配置 manifest 与子树先删/根最后的 store 接口，纳入全局session split-run/tool-policy审计分区，精确识别 incoming refs/共享归属；source整Plan purge在D1.2a落地（REQ-GRP-01、REQ-DEL-01、REQ-DEL-03、REQ-DEL-07）。
  - 进度注记（2026-10-14，ImplD11）：store 原语已实现并红→绿→回归（`cascade_d1_1_real_owner_manifest`/`cascade_d1_1_audit_dependency_order`，`lifecycle_store::tests` 109/0，全量 4610/0）；manifest 已消费 D1.0 owned-index 与 D1.2c list/purge_plan_artifacts；repository_profile 归属按 D1.3a 裁定入 manifest；D1.3b 关切②（save_work_item_split_provider_run 无归属字段）按 §0.5 refs 归属处置并登记。source/revision/engine-checkpoint purge 分别留待 D1.2a/D1.2b/D2.1d+D2.2 接线（报告 `cadence/reports/D1.1-report.md`）。复选框按本文件规约待独立审查后勾。
- [ ] 2.3 D1.2a/b/c 接入source/draft/compile/revision/publication/advance owner准入及完整purge；advance record/journal/retry精确plan归属，删除身份优先于旧幂等与Ready放行，不变未删对象hash/join/CAS（REQ-GRP-01、REQ-DEL-06/07）。
- [ ] 2.4 D1.3a/b/c 生命周期/session/版本/review/run/门操作/checkpoint/审计准入与shared-worktree中央writer guard；按attempt/WI/journal反推Plan，不改原确认CAS/lock/helper，接线遵scope先于旧session/object锁（REQ-GRP-01、REQ-DEL-03/06/07）。
- [ ] 2.5 D1.4 单调保存 ever_confirmed，生产/测试构造闭包同提交迁移；新路径可证未确认零提示，旧历史未知保守一次确认，不改变确认 CAS（REQ-DEL-04）。
  - 进度注记（2026-10-14，ImplD14）：三类 record `ever_confirmed:Option<bool>`（serde default/skip_none）＋`LifecycleStore::classify_entity_deletion_confirmation` 三分类（NotRequired／Required{PreviouslyConfirmed}〔current Confirmed｜ever=true｜Plan 发布日志在档｜Story/Design confirmed_by 版本〕／Required{LegacyHistoryUnknown}）已红→绿→回归（`cascade_d1_4_confirmation_history_classification`；`lifecycle_store` 140/0、`conversational_gate` 146/0、全量 lib 4616/1/7i——唯一失败 `campaign_stage3_turn_reservation_crash_recovers_exactly_once` 归因 SG6a.1 契约迁移消费方回归〔基线 9596b68e 单跑 PASS、主树 FAIL，对方已认领登记并直报 Main〕；it_product 209/1 唯一失败 ambiguous_push 为基线既有）。确认/commit/request_change/restore/append_version(confirmed_by) 原写单调置 true，确认 CAS 零改动；16 个构造点机械闭包同提交（含 Task Files 清单外 `src/web/handlers/workspace_human_action.rs` 与 `tests/it_product` 三 fixture——Step 3 机械 literal 闭包要求，报告注明）；候选 parser（parse/revision/validate）内存 record 保持 None。模型/分类交 D2.2，原 SG fixture 回交。报告 `cadence/reports/D1.4-report.md`。复选框按本文件规约待独立审查后勾。

## 3. D2：协调、运行隔离与一次确认

- [ ] 3.1 D2.1a/b/c/d manager/REST/provider/compile/advance/retry恢复真实入口持共享run lease；coding创建与删除互斥，Deleted advance/retry拒绝且旧Ready false，旧Arc/晚写无权；写面点名双向SG移交（REQ-GRP-01、REQ-DEL-05/06/07）。
  - 进度注记（2026-10-14，ImplD21c）：D2.1c coding 创建/删除互斥已红→绿→回归（`cascade_d2_1c_coding_create_delete_race`——红=实现回退 HEAD 后 (a) 删除独占未被创建准入阻挡；绿=guard 于 creation 锁外层持 owning Plan 共享准入随生命周期到 attempt 持久化；回归 attempt_creation_concurrency 3/0、coding_attempt_store 全族 203/0、group_split_target 11/0、group_uniqueness 2/0、advance_attempt_isolation 4/0）。接线：guard（sync/async）scope→creation 锁固定序＋WI→Plan 归属解析（source_work_item_plan_id→plan refs 反查，无归属 legacy 保持原行为）；`create_group_attempt`/`prepare_group_initialization_with_admission_for_target`（含重放读）/`mark_group_initialization_error`/`advance_group_initialization_phase` 套 `with_plan_live_write`；`ensure_group_initialization_attempt` 消费 guard 已持准入、legacy guard 由 journal.plan_id 兜底。purging/deleted 一律 `Conflict{entity_deleting}`（N2 P2 同族）；guard identity/active uniqueness/admission/不自动 abort 零变化；group_validation/advance_split 零修改。D2.1a/b/d 未含（并行/后行），复选框按规约待独立审查后勾。
  - 进度注记（2026-10-14，ImplD21b）：compile/恢复真实入口 run lease 已红→绿→回归（`cascade_d2_1b` 四用例——钝化基线红 0/4→绿 4/0：legacy/SC preparing→commit 全 checkpoint〔execute 3＋SC initial 3＋finalizer 6＋committed〕删除独占 Busy 且零墓碑〔零 attempts 前提〕；恢复 Continue 于真实 resume 位点同样 Busy；purging 后 Continue/HumanTriage/Abort 三动作 `entity_deleting` 稳定拒且 transaction/WorkItems/source/provenance 零写〔journal＋快照逐项〕；正常链路 provenance content_hash/plan_revision refs 同代。接线＝`WorkItemPlanCompileAdmission`〔acquire_run try-shared＋锁内墓碑复核〕于 run_work_item_plan_compile／enter 状态机帧〔含失败 mark_recovery＋恢复节点窗口〕／恢复动作入口三顶层持有，final 提交随帧释放；外层已持同 scope lease 时共享 flock 跨 OFD 幂等重入＝等价消费；内层 store 写沿用 D1.2/D1.3 准入。回归 compile 204/single_candidate 183/finalizer 5/cascade_d2_1 15 全绿；隔离树全量 4695/0/7i〔唯一抖动＝D2.1d 已登记 sc_confirm_cancelled 存量，裸 HEAD 对照树同败佐证〕；check --lib --tests 零新增告警。web 恢复入口 engine.lock 前预取迁移＋mod.rs 导出留终审统一登记。）
- [ ] 3.2 D2.2 三类DELETE统一门禁/manifest及服务端分级确认：coding/ref/busy/未确认拒删零副作用，两Plan路径含advance完整purge、target专属清理只读消费现worktree helper；失败重入与单条事实（REQ-GRP-01、REQ-DEL-01/02/03/04/05/07）。
  - 进度注记（2026-10-14，ImplD22）：三类 DELETE 统一编排 `cascade_delete_entity` 已红→绿→回归（`cascade_d2_2_full_closure`/`cascade_d2_2_late_child_attempt_zero_mutation`/`cascade_d2_2_failure_reentry` 3/0；N7 4/0、D1.1/D1.4 2/2、finalize/cleanup 3/2、it_web `delete_work_item_plan` 8/0）。墓碑/try-delete/资源 cleanup lease 排序/关系短锁内三道门禁（coding→entity_referenced→deletion_confirmation_required 409 稳定码，error.rs Main 授权三行）→pending+owned-index+purging→manager 失效→子依赖 purge（D1.1 children＋`purge_plan_sources` 新原语〔Main 裁定 source_store 独占，仅新增该原语〕＋plan-store/revision 既有 helper＋attempt 残留锁＋`cleanup_plan_shared_worktrees`〔D1.3c resolve 归属复核，他 Plan 保留/半残锁 owner 孤儿清〕＋`purge_plan_with_lease`＋advance checkpoint 按 session 纯索引）→根最后→残留复核→remove pending→complete；Deleted 幂等 200、根缺不 404、failpoint 重入一次收口。清单外 4 测试文件按新契约迁移（N7 helper confirmed=true/3A 幂等/3C 安全窗清锁、it_web part_13/14 confirmed、part_01 design confirmed、part_05 部分失败语义→零破坏更名迁移）登记待复核；404 映射缺与 D1.1 引用判据窄两缺口登记（报告 `cadence/reports/D2.2-report.md`）。复选框按本文件规约待独立审查后勾。
- [ ] 3.3 D2.3 所有既有 Story/Design/Plan 删除入口消费服务端确认分类，Draft 零弹窗、Confirmed/未知旧历史恰一次，取消零删除、错误提示先删后置对象（REQ-DEL-04、REQ-DEL-05）。
  - 进度注记（2026-10-14，ImplD23）：前端单次确认流已红→绿→回归（`cascade_d2_3_single_confirmation`＋client `cascade_d2_3:*` 两件 2/0/2；crud+client 全量 33/0；前端全量 213 文件 2163/0；`npm run build`+tsc EXIT=0）。三 delete 函数第 4 参 `options?:{confirmed?:boolean}`（默认 URL 无 query、true 经 URLSearchParams 编码）；Workbench 删除改显式二次调用（非预创建 Promise），`deletion_confirmation_required` 为唯一弹窗分支（完整中文警告「永久清除内容、工作项、会话和 review，且不可恢复」），取消零出站第二请求/无刷新/退场/清选；group drawer 旧无条件确认移除（不双弹）；独立 WI 保原确认行为、文案不再声称自动删除 Coding Workspace；coding/reference/busy 零确认、服务端 message 先删后置直达错误面；普通错误不自动重试。web/errors 面（Main 授权）：error.rs 族分派补全 `entity_deleting`/`entity_deleted` 409＋N2 冻结族（`coding_plan_run_*` 19 码 409、4 码 422；`plan_coding_paused`/`invalidated`/`first_unit_completed` 409）；product_store_error 直通臂保留 details.kind/id（N3 断言兼容）；D2.2 登记 1 裁定：无墓碑无 root 的 Story/Design DELETE 经 NotFound{spec}→`spec_not_found` 404（Plan 分支为 `get_issue_work_item_plan` Io 形态、需 lifecycle_store 面 NotFound 语义改造，留 D2.4 终审登记）。清单外触碰登记：`IssueLifecycleWorkbench.drawer.test.tsx` 两处旧 drawer 确认断言机械迁移（新契约下 confirm 0 次）。
  - 修复轮注记（2026-10-14，ImplD23，Main 审查派回 P1＋P2）：P1 族分派补变体——
    `IdentityMismatch`/`InvalidRecord` 前缀臂（revision/graph/replay_drift/selection
    族/attempt_binding 族/target_attempts/purge_scope 走 IdentityMismatch、
    plan_revision/selection_unit 走 InvalidRecord，原先只匹配 Conflict 致 14 码落
    other 兜底 500 不可达；单测改真实变体），error.rs 加族前缀兜底（未登记 kind
    默认 409）；提交 `40f889e9`。P2 完整性码保留——`coding_plan_run_target`
    （repo_lease_state 的 target 非法 UUID InvalidRecord）从前缀臂排除回 500
    （owner 设计 §B「真实 IO/JSON/损坏身份不是 409」）；提交 `3c619c62`。两轮均
    红→绿＋`plan_coding_n3_revision_deleting_and_needs_human_fail_closed`（真实
    POST 路径）1/0 族复跑。观察项：族前缀兜底对后续新增 `coding_plan_run_*` 码
    默认 409，新增完整性语义码时须像 target 一样显式排除；主树彼时被兄弟在途
    编辑两度打断（E0596/E0061 等，非本包面），P1/P2 红绿证据在隔离树 HEAD＋
    本包文件取证（D2.2 先例）。
- [ ] 3.4 D2.4 Main 统一执行端到端与无误伤回归、独立终审、owner 审阅；仅在全部契约满足后更新主规格/归档，不把 strict 格式通过当实现已完成（REQ-GRP-01、REQ-DEL-01 至 REQ-DEL-07）。

## 4. W：单 plan 不变量与代际取代（owner 2026-10-24 裁定）

- [ ] 4.1 W1 建立路径先删旧：在 issue 级 plan 作用域互斥内把「旧 plan 完整删除」接到 prepare/编译确认的建立入口，删除成功后才允许建立新 plan；补存活 plan ≤1 的不变量断言与 owner-key 精确定位，禁止摘摘要/解绑/改引用代替删除（REQ-WPS-01）。
- [ ] 4.2 W2 三个触发点接线：re-enable 轮换 `prepare_intent_id`、重新 prepare、重新编译确认 MUST 复用同一取代删除编排；同 `command_id` 重放不重复删、不递增代际，并记录触发原因与旧 plan 身份（REQ-WPS-02、REQ-WPS-07）。
- [ ] 4.3 W3 闭包清单落地：把 REQ-WPS-03 逐项清单接入 owner manifest，先子树后根；补齐当前缺删除原语的项（attempt 的 runs/units/events 目录、reviewer windows/snapshots/leases、coding-plan-workspace-lifecycles、group-initializations、work-item-attempt-locks）（REQ-WPS-03）。
- [ ] 4.4 W4 fail-closed、needs_human 与幂等：任一子树失败保留 plan 根为可达的 purging/needs_human 入口并持久记录失败清单，禁「根已删产物还在」；完成/进行中/失败并发重试收敛到同一事实，只落 1 条无内容墓碑（REQ-WPS-04、REQ-WPS-05、REQ-WPS-07）。
- [ ] 4.5 W5 terminated 交互：re-enable 先走既有整 plan 终止收口语义，再级联删 attempts/worktree 实体/shared-worktree 记录与锁/attempt locks/reviewer leases，不复用旧 identity；普通 DELETE 维持既有 coding 拒删，不把该能力开放给用户面（REQ-WPS-06）。
