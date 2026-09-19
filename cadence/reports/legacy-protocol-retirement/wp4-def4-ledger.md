# WP4 DEF-4 契约显式化——逐句锚定表与台账销账（retire-legacy-workitem-protocol Task 3）

> 计划锚：`cadence/plans/2026-09-19_计划文档_阶段4-C3_旧协议退役与DEF4_v1.0.md` Task 3（WP4.1-4.4，REQ-ADV-05/06）。
> 决议锚：三方决议 Q4 DEF-4 → (b) 契约显式化结项（`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/stage4-tripartite-decision.md` §Q4）。
> 独立面声明：本 WP 不依赖 T1 门重测结论（挂起分支下可先行，计划约束 6）。
> 实读基线：worktree `feat-b-0808-add-monorepo` HEAD=`3b5260c7`（2026-09-19 07:22:53 +0800，晚于计划锚定基线 `4d097c1d`；本表行号为该 HEAD 实读复测，锚点全部复核成立）。

## §1 三句契约+红线逐句锚定表（4.1）

spec 句摘自 delta `openspec/changes/retire-legacy-workitem-protocol/specs/work-item-plan-advance/spec.md:28-50`（REQ-ADV-05「advance 到 Ready 即止与 coding 启动唯一入口」）。

| REQ-ADV-05 契约句（delta 原文摘录） | 代码/测试证据（`3b5260c7` 实读） | 一致性 |
|---|---|---|
| ①「`advance` 的完成态 SHALL 为 `Ready`（group workspace 就绪），advance 本身及 `advance_completed` 事件 SHALL NOT 启动任何 coding provider」 | 测试 `src/product/coding_workspace_engine/advance_ready_only.rs:71-96`：`advance_ready_response_does_not_start_coding_provider`——durable Ready 记录在案后 engine 侧启动仅产出一条 `CodingStageChange{Coding}` 事件且 `event_rx.try_recv().is_err()`（:95）钉死无任何 provider 后续事件 | 一致（独立测试锚在案，`cargo test --locked advance_ready` 绿） |
| ②「coding provider 启动的唯一入口 SHALL 为显式 `StartCoding` 入站命令（人工触发）；advance 完成、事件编排、恢复重放或任何其他服务端动作 SHALL NOT 隐式启动、批量启动或随附启动 coding provider」 | 首发入口：`src/web/coding_ws_handler/socket.rs:284`（`if inbound == CodingWsInMessage::StartCoding`）→ `spawn_coding_runner`（:340，首发唯一 spawn 点）。产码 spawn 面全量清点（grep `spawn` 实测，其余四处均非首发）：socket.rs:254 `spawn_coding_runner_reserved`（FailedReviewRecovery 预约恢复，attempt 已入失败审查恢复流）；socket.rs:563 `spawn_coding_runner`（GateResponse 后续跑，前置 `updated.status == Running` :562）；`socket/resumption.rs:90`（attach 半启动重启：Running+WorktreePrepare/Coding+注册表无 runner 判定 :106-119，SC 重启同款 durable-ready fail-closed 门 :121-153）；`runner/amendment.rs:11` `spawn_plan_amendment_runner_reserved`（plan amendment 链预留 runner，唯一调用点 `workspace_ws_handler/plan_repair_activation.rs:36`，SC 基座保留项，非 coding attempt 启动）。恢复/重放不隐式启动测试锚：resumption.rs `other_statuses_stages_or_live_runners_stay_snapshot_only` :248-301（**本次补 Created+PrepareContext 不动作断言 :274-282**——未启动 attempt 的恢复路径零动作）+ `sc_advance_gate_only_applies_to_sc_admission_with_binding` :303-326（SC 重启未 durable-ready 即 fail-closed） | 一致（首发唯一=StartCoding 分支；其余 spawn 全部限定「已启动 attempt 的恢复/续跑」或「amendment 链独立 runner」，不存在 Created/未启动 attempt 的服务端隐式启动路径； Created 负向断言本次补锚转直接覆盖） |
| ③「SC admission 的 coding attempt 在经 advance 置 `Ready` 之前收到 `StartCoding` SHALL fail-closed 拒绝并返回错误码 `SC_CODING_REQUIRES_ADVANCE`，attempt 与 session 状态不变」 | socket 层守卫：`socket.rs:302`（durable 校验 Err 维度）/`:321`（未 Ready 维度）两处错误码发射，拒后 drop 租约+continue（无 spawn 无状态迁移）；引擎层同语义 `src/product/coding_workspace_engine/lifecycle.rs:207-213`（"SC advance attempt must be Ready before StartCoding"）。测试锚：引擎层 `advance_ready_only.rs:98-123` `sc_coding_start_without_advance_is_rejected`（错误文本+attempt 停留 Created+零事件）；**本次补 socket 层锚** `src/web/coding_ws_handler/tests/sc_start_guard.rs:41-42` `start_coding_before_advance_ready_is_rejected_with_sc_coding_requires_advance`——真实 ws 入口（`/ws/projects/{p}/issues/{i}/coding-attempts/{a}`）发送 `{"type":"start_coding"}`，断言错误码 `SC_CODING_REQUIRES_ADVANCE`+message 逐字、attempt status/stage 不变、拒后无后续 ws 事件 | 一致（双层测试锚+产码双点守卫在案） |
| ④（红线条件 defer）「显式 opt-in auto 启动通道为条件 defer 项（触发条件=autopilot/驾驶舱真实 auto 需求）……MUST 满足全部红线：opt-in 持久化 run_policy、默认 off、per-attempt 单发、绝不批量、不动唯一人工门，且 MUST 另行 change 显式定义，MUST NOT 以隐式或随 advance 形态落地」 | 本 change 零 auto 启动路径落地：grep `auto_start` 于 `src/` 零命中（2026-09-19 实测）；§1 表 spawn 面清点（句②）佐证无隐式/批量/随附启动路径 | 一致（现状=「在此之前任何 auto 启动路径不存在」成立） |

### §1.1 锚定测试实跑记录（2026-09-19，HEAD=`3b5260c7`）

```text
cargo test --locked advance_ready            → 10 passed; 0 failed（句①×1+句③引擎层×1 及同文件既有 8 条）
cargo test --locked sc_coding_requires_advance → 1 passed; 0 failed（句③ socket 层新锚）
cargo test --locked other_statuses_stages    → 1 passed; 0 failed（句② Created 负向断言所在测试）
```

注：cargo 测试过滤大小写敏感，计划 Step 1 示例命令的大写过滤串（`SC_CODING_REQUIRES_ADVANCE`）零命中（错误码非测试名命名段），实测以小写 `sc_coding_requires_advance` 过滤执行——锚定语义不受影响，如实记录口径差。

### §1.2 补锚说明（红→绿口径）

两处补锚均为「钉既有正确行为」性质（产码守卫在案、行为无缺口），故新增测试首跑即绿、无红相位（红相位只会在守卫被移除的回归场景出现——socket 锚测试在守卫缺失时将因收不到 `coding_protocol_error` 而超时红，防护性成立）。本 WP 零产码改动，见 §4。

## §2 auto 通道红线条件 defer 登记（4.2）

- **登记时间**：2026-09-19（change ③ Task 3 落地时点）。
- **登记位置**：`cadence/reports/workitem-conversational-gate-advance/defer-ledger.md` Deferred 表末新增 `DEF-4A` 行（内容=红线五条+触发条件+MUST/MUST NOT 边界，逐字按计划 Step 2）。
- **依据**：三方决议 Q4 裁决 (b) 之子项 (a) 降级形态——「显式 opt-in auto 通道降为红线条件项 defer（触发=autopilot/驾驶舱真实 auto 需求；若做须满足：opt-in 持久化 run_policy/默认 off/per-attempt 单发/绝不批量/不动唯一人工门）」（stage4-tripartite-decision.md §Q4）；行权边界=3.3 设计先例「拒绝 advance 批量拉起全部 coding——不另造自动启动协议；将来需要时单独显式定义 auto_start_coding；**单独显式定义未被排除**」（design.md 事实基线 #12 转述）。
- **spec 承载**：delta REQ-ADV-05 末句+Scenario「auto 通道红线条件 defer 登记」（spec.md:47-50）。

## §3 DEF-4 销账记录（4.3）

- **原台账行**：`cadence/reports/workitem-conversational-gate-advance/defer-ledger.md` Deferred 表 `DEF-4`（原 :23，销账前=「不在阶段 3 范围/owner 后续 change」）。
- **销账处置**：状态列推进为「已销账（change ③ WP4：契约显式化入 `work-item-plan-advance` REQ-ADV-05；auto 通道降级为红线条件 defer 项 DEF-4A）」，owner 列=「change ③ WP4（已闭环）」——形态循 DEF-8「已核销/确认，登记备查」先例（约束 11：行状态推进，不改历史 notes、不篡改事件）。
- **spec 指针**：delta `openspec/changes/retire-legacy-workitem-protocol/specs/work-item-plan-advance/spec.md` REQ-ADV-05（:28-50，三句+红线五条条件 defer）+ REQ-ADV-06（:9-26，SC 准入唯一入口——REQ-ADV-03 REMOVED 的承接项）。
- **销账依据**：本表 §1 三句逐句实测一致+§1.1 锚定测试全绿+§2 红线 defer 登记在案。

## §4 自查（4.4）

- **零产码行为变更**：本 WP 仅两处测试文件新增/扩展（`src/web/coding_ws_handler/tests/sc_start_guard.rs` 新建+接线 `tests.rs:56`；`socket/resumption.rs` 既有测试内补一条 Created 负向断言），零 `src/` 产码逻辑改动。
- **auto 启动路径零落地**：`grep -rn "auto_start" src/` 零命中（否定句/守卫面亦无命中）；§1 句② spawn 面清点无隐式启动路径。
- **不依赖 T1**：本报告全部证据为测试面+代码实读，未引用 REQ-WSC-07 门重测任何结论（挂起分支下先行合法，计划约束 6）。
- **spec 零改动**：REQ-ADV-05/06 delta 措辞在计划起草轮已锁（v1.0 计划 Spec 节），本 WP 仅核验一致，未改 spec 任何字符。
