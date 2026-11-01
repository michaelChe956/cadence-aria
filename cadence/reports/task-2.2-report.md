# Task 2.2 (A1c: typed 恢复 snapshot 与 operation 结果) 实施报告

- worktree: `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 基线: af26f0e9 (2.1 已落 LcResumeRecoveryRecord/LcFreshOperationResult/run_context 全 tag)
- 红绿命令: `cargo test --locked --lib anchor_t03`(cwd=worktree 根)
- 说明: 本文件为证据记录,按 brief 不 git add。

## 写面(brief 显式列表,无越界)

1. `src/product/workspace_engine/session_state.rs` — build_session_state 新增 `lc_resume_recovery` durable 纯投影(`self.session.lc_resume_recovery` 经 2.1 from_record 透传);新增 `#[cfg(test)] mod anchor_t03_tests`(引擎级红绿用例,隔离 TempDir 的 new_persistent 构造)
2. `src/product/workspace_engine/types.rs` — `EngineEvent::LcResumeRecoveryChanged` 单元变体(内部 authoritative 全量同步触发,不经 WS 序列化)
3. `src/web/workspace_ws_types/out.rs` — `LcResumeRecoveryDelivery`(Accepted|Replayed)/`LcFreshOperationDto`(operation_id/state/reason?/fresh_node?)/`LcResumeRecoveryDto`(recovery_id/state/reason_code/provider/failed_node_id/run_kind/detected_at/updated_at/allowed_actions/last_operation,含 `from_record` 纯投影);`lc_recovery_run_kind` 与 durable serde tag 严格一致;`WsOutMessage::LcResumeRecoveryOperationResult` 变体;`SessionState.lc_resume_recovery` 字段(default+skip_none);内部 mod tests 字面量闭包
4. `src/web/workspace_ws_types/tests.rs` — 新用例 `anchor_t03_recovery_snapshot_and_operation_result_wire`;既有 SessionState 字面量迁移
5. `src/web/workspace_ws_handler/mapping.rs` — BP1 最小 `return None` 忽略臂(无 catch-all,真实广播串行移交 2.4)
6. `tests/it_web/web_workspace_recovery_consistency/part_02.rs` — SessionState 字面量迁移+无恢复时 wire 省略断言

### 关键实现决策

- **allowed_actions 恒空集**:设计 §4.5 A/B 发布纪律「未接齐 B0/covered-run fresh handler 时 allowed_actions=[]」+Plan Task 2.6 判据「waiting allowed 为空直到 B 齐」;waiting/fresh/resolved 四态均空,后端入口真实拒收兜底。
- **run_kind**:与 durable serde tag(snake_case)严格一致,不引入第二套字符串协议。
- **last_operation 派生**:record.operation_id 锚定 fresh_operations 内同 ID 结果(缺锚回退最新一条,证据不丢);映射 reason_code→reason、fresh_node_id→fresh_node(接口字典命名)。
- **秘密不下发**:superseded_native_session_id/previous_fingerprint_digest/fresh_native_session_id 不入 DTO;测试以真实 fixture 内嵌秘密串断言序列化不含。
- **mapping.rs BP1**:仅把 `EngineEvent::LcResumeRecoveryChanged` 加入现有 `return None` 忽略组,无 catch-all、不序列化内部事件、不碰 router 真实广播(串行移交 2.4)。

## TDD 证据

### 红(Step 2)

命令: `cargo test --locked --lib anchor_t03`(2026-10-11 16:0x,实现前)

新接口编译红(计划 §0.2 允许:新接口初次可编译红)。本包测试文件错误:

```
error[E0432]: unresolved imports `crate::web::workspace_ws_types::LcFreshOperationDto`, `crate::web::workspace_ws_types::LcResumeRecoveryDelivery`, `crate::web::workspace_ws_types::LcResumeRecoveryDto`
   --> src/web/workspace_ws_types/tests.rs:815:9
```

完整原始输出留存 `/tmp/anchor_t03_red_raw.txt`。同期输出含并行 worker 半成品文件的无关错误(workspace_human_action.rs/conversational_gate_protocol.rs 等,非本任务写面),不影响本包红判定。

### 绿(Step 4)

命令与结果(2026-10-11 16:2x,实现后):

```
$ cargo test --locked --lib anchor_t03
running 2 tests
test web::workspace_ws_types::tests::anchor_t03_recovery_snapshot_and_operation_result_wire ... ok
test product::workspace_engine::session_state::anchor_t03_tests::anchor_t03_build_session_state_projects_durable_recovery ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 4308 filtered out
```

红→绿间的一次真实行为迭代:引擎级用例最初用 `WorkspaceEngine::new`(无 lifecycle_store)构造 WorkItemPlan 会话,触发既有 `plan_projection.rs:559` expect panic;按引擎测试惯例改隔离 TempDir 的 `new_persistent` 后通过(不改产品代码)。

范围化回归组(Plan 8.1 表 2.1/2.2 行):

```
$ cargo test --locked --lib workspace_ws_types
running 38 tests
test result: ok. 38 passed; 0 failed; 0 ignored; 0 measured; 4272 filtered out
```

2.1 用例回归(同写面 types.rs 重叠):

```
$ cargo test --locked --lib anchor_t02
running 2 tests
test product::models::tests::anchor_t02_legacy_json_and_runtime_restore_defaults ... ok
test product::models::tests::anchor_t02_recovery_schema_roundtrips_all_run_contexts ... ok
test result: ok. 2 passed; 0 failed
```

集成编译闭包:`cargo check --locked --test it_web` 本包写面收口后验证 0 错误(16:52 时点);后续复跑出现的 1 个 E0004 位于 `src/web/handlers/workspace_human_action.rs:237`,为并行 trust T5 worker 半成品,非本包文件。

绿证据原文留存 `/tmp/anchor_t03_green_raw.txt`。

## 完成判据对照

- snapshot 必要字段不含 native/digest 秘密 ✓(四态断言 serialized 不含 native-secret-session/sha256:secret-full,wire 无 native_session_id 键)
- waiting/reserved/running/resolved 与 last_operation 序列化 ✓(四态×对应 op 状态 failed/reserved/running/completed)
- resolved 证据保留但 allowed 为空 ✓(detected_at/updated_at/last_operation 在,allowed_actions==[])
- 内部 RecoveryChanged 不是 wire delta ✓(`map_engine_event(EngineEvent::LcResumeRecoveryChanged).is_none()`)
- mapping.rs 同提交最小 None 忽略臂、无 catch-all ✓;router 通用转发不碰(router catch-all 保持可编译,已核全库 WsOutMessage/EngineEvent match 无第二穷举点)
- build_session_state 取 durable 字段 ✓(引擎级用例:含恢复 record→Some(dto) 全字段断言;无恢复→None)

## 提交(Step 5)

按 brief Step 5 显式列表 stage 六文件提交:

```
git add -- src/product/workspace_engine/session_state.rs src/product/workspace_engine/types.rs src/web/workspace_ws_types/out.rs src/web/workspace_ws_types/tests.rs src/web/workspace_ws_handler/mapping.rs tests/it_web/web_workspace_recovery_consistency/part_02.rs
git commit -m "fix: A1c typed恢复snapshot与operation结果"
```

commit hash: `36843e7e`

## 移交

- mapping.rs 的 `LcResumeRecoveryChanged` 真实广播接线 → Task 2.4(A1b2)router 串行接管
- `LcResumeRecoveryOperationResult` 的 outbound 发送点(B1b fresh 协议)→ 4.2
- waiting 态 allowed_actions 放开(StartGeneration)→ B 波接齐 B0/covered-run handler 后
