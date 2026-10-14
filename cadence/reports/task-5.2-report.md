# Task 5.2(B2b1:Plan full native 启动与 map 绑定)实施报告

- 实施:ImplAnchorB52(glm53-task worker)
- worktree:`/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 基线 HEAD:`15dc65e3`(fix: B2a Plan stale等待与全tagdispatch)
- 状态:实现完成;anchor_t14 红(stub)→绿;范围化回归全绿;独立 commit 已落(见 §5)

## 1. 交付内容

| 文件 | 变更 |
|---|---|
| `src/web/workspace_ws_handler/run/gateway_start.rs` | ① `LogicalPlanLaunch` 删 `target_worktree`/`logical_repository_id`/`checkout_id` 三字段,`planning_request()` target 恒 `PolicyTarget::aggregate_root(canonical root)`(REQ-PLN-08:native anchor 不从 business working_dir/成员身份推导,成员身份不入 envelope 授权面) ② 新 `ensure_plan_native_anchor_resolved`:resolve 前消费 E `workspace_native_launch_anchor_for_session`(durable record + 共同守卫 fail-closed,错误前缀 `logical_plan_native_anchor_unavailable`;无持久 store 同码 fail-closed);`resolve_plan_author_launch(engine)` 单参化 ③ **prepared-map-before-start(§4.2.1,纪元写入落地)**:`PlanSplitRunContext` 增 `(project,issue,provider)` 定位 + `persist_prepared_launch_identity`——Logical+handle 启动在 `prepare_streaming_launch` 前以排它锁 RMW(store 同一文件锁约定,`workspace_sessions_root`+`json_store::read/write_json`+`coding_attempt_store::locking::with_exclusive_lock`)写 `LogicalLaunchFingerprintRecord`(basis=FullLcProjectionV2,role=Author,role_run_seq=handle seq,digest=validated policy 冻结指纹,Prepared 阶段 node/native 留空不伪造);I/O 失败零 start(前缀 `logical_launch_prepared_map_unavailable`);同锁绑定 FreshRunning recovery 的 `fresh_role_run_seq`(仅未绑定时) ④ 成功收口 `finalize_launch_after_success`(close 两出口调用):Prepared→Active(native id/节点取自真实 Completed 落的 conversation ref;Superseded/Active 不覆写)+ FreshRunning→**Resolved** 对账(fresh_native_session_id/fresh_node_id 回填、operation Completed+role_run_seq/fresh_node_id 同 run 绑定);失败收口 `reconcile_launch_failure`(fail_plan_split_run 调用):FreshRunning→Waiting+operation Failed(reason `lc_resume_fresh_run_failed: …`,结果保留;map 保持 Prepared→resume 自然进纪元严格门) ⑤ dispatch:FreshRunning CAS `Conflict`=single-flight 输家静默让位、其余错误才 `store_unavailable` typed 上浮(不吞写失败);fail 路径 CAS `Conflict` 让位不误报 store_unavailable(**5.1 审查观察收窄,Main 裁决附带项**);spawn Ok(true) 后 best-effort 绑 `fresh_node_id`(manager 活动 run,kind 判别式守卫;rebuild 类臂内建节点由收口回补) ⑥ `plan_member_anchor` 删除(唯一用途即旧 native 成员锚,clean-cutover;business 消费面 `workspace_repository_for_session` 原语义不动) |
| `src/web/workspace_ws_handler/run.rs` | 再导出行机械适配:删 `plan_member_anchor`;`begin_plan_split_run`/`logical_plan_launch_for` 现仅测试消费(cfg(test) 收敛/直接模块路径) |
| `src/web/workspace_ws_handler/run/provider_run.rs` | draft/batch 臂 resolve 调用删参;SC manual 臂 r27 成员锚块收敛为 `resolve_plan_author_launch(&engine)`(root 锚下锚漂移机制结构性消失) |
| `src/web/workspace_ws_handler/run/provider_run/work_item_plan_legacy_author.inc.rs` | 首发/AutoRevision 两处 resolve 调用删锚参(business `repository` 材料消费面不动) |
| `src/web/workspace_ws_handler/run/followups.rs` | **仅删实参**(Main 裁决 A:纯机械签名适配,4 处;零其它改动) |
| `src/web/workspace_ws_handler/run/single_candidate.rs` | **仅删实参**(裁决 A,2 处:preflight 谓词 `logical_plan_launch_for(engine)`、SC fresh 轮 resolve;成员锚 let 绑定随实参删除移除) |
| `src/product/workspace_engine/tests/part_09.rs` | 新增 `anchor_t14_plan_resume_without_trusted_map_waits_not_fresh`:纪元激活(gateway+同 role/provider 存在 map 记录)后 conversation-only native → typed `native_resume_candidate_frozen_binding_unavailable`(零 fresh 出生);绑定一致 native 正常续接;无 gateway direct 形态(纪元之前)维持既有 resume(plan None 分支原样) |
| `src/web/workspace_ws_handler/tests/gateway_start.rs` | 新增 `anchor_t14_plan_native_anchor_and_map_are_full_and_durable`(主用例,晚绑定 probe adapter 在 provider start 时点快照 durable prepared map)+ `T14ClaudeOnlyCapabilitySource` + `t14_logical_fixture_with_gateway_extras`/`t14_canonical_root` helper;既有 Logical 用例全部迁 ProviderRunFixture 持久形态(E 消费要求 durable record);`plan_author_launch_anchor_drifts…` 按 §3.3 裁定改写为「两次 resolve target/指纹恒等」;root_cwd 用例改钉 target=root+空成员身份 |

未触碰:`src/product/workspace_engine/provider_drive/work_item_plan.rs`(tasks.md 5.2 列为写面,但子驱动 `record_provider_session` 已满足 Completed→conversation(node/native)绑定的设计消费,纪元机制全部落在 run 身份层,无需改动——不为清单而造改动);`src/product/work_item_split_engine/prompts.rs`(brief 授予面,本包无需求)。

## 2. 设计对照

- **native anchor from E root 不 member business**(plan Step 1/§3.1/§3.2):envelope target 恒 aggregate_root(canonical root),checkout/logical id 空;`resolve_plan_author_launch` 消费 E native field(共同守卫 fail-closed);business 材料(prompt/compile)仍走 `workspace_repository_for_session` 原语义。
- **prepared map before start / native audit 同 run bind**(§4.2.1/§4.2.2):prepared 指纹 spawn 前按 (role,provider,role_run_seq) 落盘,seq 与 run-bound audit sink(`prepare_streaming_launch` 绑定的 `audit_context().role_run_seq`)同源;Active 的 native id 仅从真实 Completed conversation 绑定,不倒算。
- **B4.1 张力裁决的 5.2 收口点**:fullmap 写入落地后,4.1 的「fullmap 纪元激活即严格」立即全量生效——conversation-only resume 自然进纪元严格门(part_09 用例三段式:纪元前宽松/纪元中 unbound 等待/bound 续接)。
- **5.1 移交项**:dispatch 绑 fresh_node_id(spawn 后 best-effort+收口回补)与 fresh_role_run_seq(prepared 写同锁);FreshRunning 的 Resolved 完成收口(§4.3「完整artifact完成」行:map Active+recovery Resolved+operation Completed,原业务完成器不触碰)。
- **5.1 审查观察收窄**(Main 裁决附带项):两处 CAS 的 `Conflict` 与真实存储错误 typed 区分,不再把竞写输家误标 store_unavailable。
- **direct plan None 分支原样**:Legacy(无 gateway)不经 E 消费/map 写面;既有 Legacy 零变化测试保持;`logical_plan_validate_failure_is_reported_by_handler` 既有持久用例原样通过(路由 legacy → E anchor 解析 Ok → validate 失败上报路径不变)。

## 3. 红→绿证据(本 worker 会话实测)

1. **接口切割编译红**(clean-cutover 证明,4.1 同型):测试面先行迁移后 `cargo check` 出 E0061(旧三参调用)/E0560/E0308 族,直至签名与调用面落地才可编译(会话内实测)。
2. **行为红(stub)**:临时 stub `persist_prepared_launch_identity`/`finalize_launch_after_success`(no-op)后 `cargo test --locked --lib anchor_t14` → `1 passed; 1 failed`,判据断言原样 FAIL:`provider start 前 durable 必须已含本 run 的 Prepared 绑定,got: []`(gateway_start.rs:1525);过滤命中 2 个新用例(非 0 tests)。stub 移除(diff 与绿版逐字节 IDENTICAL 校验)。
3. **绿**:`cargo test --locked --lib anchor_t14` → **2 passed; 0 failed**。终态断言覆盖:envelope target=canonical root+空成员身份、prepared-before-start 快照、`persisted_binding.role_run_seq==actual_start_seq`、map Active+native/node 绑定+digest v2 前缀、recovery Resolved+fresh_role_run_seq/fresh_native_session_id/fresh_node_id/operation Completed 同 run。

## 4. 范围化回归(全绿)

`gateway_start` 17 passed;`web::workspace_ws_handler` 全量 201 passed;`product::workspace_engine::tests` 712 passed;`single_candidate` 162;`interrupted_run` 7;`author_revision` 30;`planning_resume` 10(5.1 面);`web::workspace_session` 56;`f60_exit_contract` 8(direct 不回归);`anchor_t` 全族 51(t7–t14);anchor_t11/t12/t13 分项 5/6/4。收尾微调(Kind 拒收消息读 payload、删 2 处死 use、删无用 helper、去 mut)后复验:anchor_t14 2 passed、gateway_start 17 passed、`cargo check --lib --tests` 0 error 且写面零告警。

## 5. 提交

```bash
git add -- src/web/workspace_ws_handler/run/gateway_start.rs src/web/workspace_ws_handler/run.rs src/web/workspace_ws_handler/run/provider_run.rs src/web/workspace_ws_handler/run/provider_run/work_item_plan_legacy_author.inc.rs src/web/workspace_ws_handler/run/followups.rs src/web/workspace_ws_handler/run/single_candidate.rs src/web/workspace_ws_handler/tests/gateway_start.rs src/product/workspace_engine/tests/part_09.rs
git commit -m "fix: B2b1 fresh_node绑定纪元写入与Resolved收口"
```

- 裁决 A 两文件(followups.rs/single_candidate.rs)diff 仅删实参/删实参源绑定与注释,无其它改动;tests/gateway_start.rs 含 1 处先于本包的并行 rustfmt 重排(基线即有,随本包带入)。
- 本报告文件按纪律不 git add。

## 6. 已知边界与移交

- Early-arm 失败(resolve/input 构建失败,begin handle 之前)不触发 fail 收口——FreshRunning 残留按 §4.3「running失败/重启未知→对账」行留待对账(无自动 fresh 重放);provider 级失败已由 `fail_plan_split_run` 收口回 Waiting。
- `LcFreshRunKind::Kind(_)` 各 tag 的 full fresh 臂仍按 5.1 覆盖面纪律 typed 拒零 spawn,归 5.3–5.6 逐包接管(5.1 §7 移交不变)。
- prepared map 的 `planning_access_fingerprint` 暂同 fingerprint_digest(五参域已覆盖 cwd/readable/target);B0 full candidate 消费面接入后由其 digest 覆写(代码注释已标注)。
- 下一包(5.3 B2b2):legacy_author.inc.rs/prompts.rs/provider_run.rs/planning_resume.rs 面;本包后 resolve 单参+map 机制可直接消费。
