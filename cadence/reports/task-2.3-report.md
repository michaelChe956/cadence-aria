# Task 2.3 (A1b1: 最新 record 纯投影加载与错误合同) 实施报告

- worktree: `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`
- 基线: 49051b3e (2.2 已落 RecoveryDto/OperationResult/LcResumeRecoveryChanged + mapping 最小臂)
- 红绿命令: `cargo test --locked --lib anchor_t04`(cwd=worktree 根)
- 说明: 本文件为证据记录,按 brief 不 git add。

## 写面(brief 显式列表 + Main 两项裁定授权)

1. `src/product/workspace_engine/lifecycle.rs` — 抽出共享纯持久读核心:`PersistentDerivation` 结构 + `derive_persistent_timeline`(载入段:artifact 版本覆盖/timeline/active/stage 派生,纯内存)+ `finalize_persistent_derivation`(收尾段:retired-stage 迁移/最新 review verdict/pending author choice/plan repair snapshot 装配,只读)+ `persistent_engine`(唯一 Self 装配点);`new_persistent` 重构为「写恢复 → 共享核心」完全保持既有行为;新增 `new_persistent_for_projection`(`ProjectionOnly`:无 plan repair journal 修复/outline revision transaction reconcile/text fallback/outline review schema fallback 四类恢复写,durable 读 IO/JSON typed 上抛不降级假态)
2. `src/web/workspace_session/manager/durable_projection.rs` — A1b1 契约类型与纯加载核心:`DURABLE_PROJECTION_UNAVAILABLE_CODE` 常量、`WorkspaceSessionIdentity`(出生 record 冻结五元组,`from_bound_record`)、`DurableWorkspaceProjection{session_state,pending_choice}`(含 `recovery_id()`)、`DurableProjectionError{Store,IdentityMismatch}`(含 Display)、`load_durable_workspace_projection(app_paths,expected_identity)`:每次 `get_workspace_session(session_id)` 读**最新** record → project/issue/entity/type 四元身份核对 → ProjectionOnly 引擎构造纯投影(自动化归属覆盖保留既有语义);`durable_projection_with` 改为「出生 record 仅构造身份 + 消费核心;读取失败输出真实 ProtocolError(workspace_durable_projection_unavailable)＋None」,不回出生 record、不 default 假 state
3. `src/web/workspace_session/manager/mod.rs` — **Main 裁定授权**(同 T2/T5 先例:验收权威于文件清单的最小必需接线):单段 `pub(crate) use durable_projection::{...}` re-export(与该文件 :30/:33 既有同型模式一致),纯可见性零行为——part_07 typed 断言与 2.4 router/attach/socket Result cutover 均必需
4. `src/web/workspace_session/tests/part_07.rs` — 新用例 `anchor_t04_latest_record_pure_projection_while_engine_locked`、`anchor_t04_projection_read_failure_never_uses_birth_record`;fixture:`claim_manager` 补出生 record 真实落盘(store 扫描只认 `workspace_session_*` ID,统一补前缀);新增 `anchored_durable_manager`(真实 store 落盘)/`overwrite_latest_record_with_recovery`/`durable_tree_snapshot`(零写对照)助手
5. `src/web/workspace_session/tests/part_02.rs` — **Main 裁定 Option A 授权**(SDD 纪律实施提交全绿;2.4 未开工无并发写冲突;纯 fixture 合规迁移):`automation_manager_fixture` 内落盘出生 record+`workspace_session_` 前缀(幂等助手 `automation_store_session_id`),manager 测试的 bind_plan/ownership 绑定链对齐同串;断言本体与产品代码不动

### 关键实现决策

- **共享纯读核心,不建第二权威 model**:`new_persistent`(真实恢复)与 `new_persistent_for_projection`(纯投影)共用 `derive_persistent_timeline`/`finalize_persistent_derivation`/`persistent_engine`;恢复写(plan repair journal/outline revision transaction/text fallback/outline review schema fallback)只在 `new_persistent`,ProjectionOnly 零写——§4.4「不能在 fallback 沿会写的 new_persistent 偷偷恢复」。
- **typed 错误合同**:latest record 读 NotFound/IO/JSON → `DurableProjectionError::Store`;身份漂移 → `IdentityMismatch{session_id}`;两者都不回出生 record/空 recovery/Open 假态。tuple 出口(2.4 删除对象)本包错误仅真实 ProtocolError(`workspace_durable_projection_unavailable`)+None。
- **plan repair snapshot 在纯投影保留 fail-closed 读**:`load_plan_repair_snapshot_fail_closed` 以 `transition_recovery_error=None` 参与 E 段(读既有 durable snapshot 事实,不执行 journal 恢复),plan-amendment 会话投影 stage 与真实路径同源。
- **纯投影 timeline/artifact 读 typed 上抛**:`new_persistent_for_projection` 对 `load_timeline_nodes_for_issue_session`/`list_artifact_versions_for_issue_session` 用 `?` 传播(区别于 `new_persistent` 既有 `unwrap_or_default` fail-open,真实恢复语义不变);文件缺失本身是 `Ok(vec![])` 合法空态,仅 IO/JSON 上抛,不把损坏 timeline 假装成初始 timeline。
- **LC 恢复等待 reconcile 不在纯投影**:本包只交付读取层;`reconcile_lc_resume_waiting`/真实 `recover_on_creation` 写入口归 A2b(Task 2.6)。
- **store 扫描 ID 前缀事实**:`lifecycle_store/utils.rs:90 workspace_session_file_stem` 只认 `workspace_session_*`——非该前缀 ID 落盘后 `find_workspace_session_path` 不可见。已记 ledger 供 2.4/终审知悉(未改产品代码);part_07/part_02 fixture 按此前缀落盘。

## TDD 证据

### 红(Step 2)

命令: `cargo test --locked --lib anchor_t04`(2026-10-11,实现前,基线 49051b3e,worktree 编译面当时干净)

新接口编译红(计划 §0.2「新接口初次可编译红但绿须全契约」允许,progress.md「红测编译红采信」先例同型 A2.2 E0432):

```
error[E0432]: unresolved imports `crate::web::workspace_session::manager::DurableProjectionError`, `crate::web::workspace_session::manager::WorkspaceSessionIdentity`, `crate::web::workspace_session::manager::load_durable_workspace_projection`
   --> src/web/workspace_session/tests/part_07.rs:468:5
```

完整原始输出留存 `/tmp/anchor_t04_red_raw.txt`。

### 绿(Step 4)

绿证据取自隔离 worktree `/tmp/anchor_t23_green`(= 基线 49051b3e + 仅本包 diff,硬链接复用主 target);主 worktree 共享编译面同期被并行 trust worker 半成品(provider_trust.rs/gateway_factory.rs 等 13 个 E0277/E0308/E0599 错误,全部非本包文件)阻断,按环境纪律不在并行半成品窗口跑绿。

```
$ CARGO_TARGET_DIR=/tmp/anchor_t23_target cargo test --locked --lib anchor_t04
running 2 tests
test web::workspace_session::tests::anchor_t04_projection_read_failure_never_uses_birth_record ... ok
test web::workspace_session::tests::anchor_t04_latest_record_pure_projection_while_engine_locked ... ok
test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 4310 filtered out
```

红→绿间的真实行为迭代(均 fixture 层,不改产品代码):
1. `manager.engine().lock().await` 临时值生命周期(E0716)→ 改用既有 `lock_engine_for_test()`;
2. 发现 store 扫描只认 `workspace_session_*` ID(探针实证:非前缀 ID 落盘后 `get_workspace_session` NotFound)→ part_07/part_02 fixture 统一前缀;
3. part_02 automation 绑定链(bind_plan/ownership)与落盘 record.id 对齐(Main 裁定 Option A)。

### 范围化回归组(Plan 8.1 表 2.3/2.4 行,同隔离 worktree)

```
$ CARGO_TARGET_DIR=/tmp/anchor_t23_target cargo test --locked --lib workspace_session
test result: ok. 66 passed; 0 failed; 0 ignored; 0 measured; 4246 filtered out
```

`new_persistent` 重构行为保持的安全网(engine 全量):

```
$ CARGO_TARGET_DIR=/tmp/anchor_t23_target cargo test --locked --lib workspace_engine
test result: ok. 1399 passed; 0 failed; 0 ignored; 0 measured; 2913 filtered out
```

相邻 anchor 包(E1.1/2.1/2.2/2.3):

```
$ CARGO_TARGET_DIR=/tmp/anchor_t23_target cargo test --locked --lib anchor_t0
test result: ok. 8 passed; 0 failed; 0 ignored; 0 measured; 4304 filtered out
```

集成编译闭包: `cargo check --locked --test it_web` 0 错误(隔离 worktree)。

绿证据原文留存 `/tmp/anchor_t04_green_raw.txt`。

## 完成判据对照

- 最新 session_id record identity 校验 ✓(latest.project/issue/entity/type 四元 vs `WorkspaceSessionIdentity`;漂移 → `IdentityMismatch`,用例 1 断言)
- 旧 manager 出生 record 不作正文 ✓(出生无恢复、最新带 recovery_id="latest-rec" → `projection.recovery_id()==Some("latest-rec")`;tuple 出口 SessionState.lc_resume_recovery 同源最新)
- ProjectionOnly 前后 session/timeline/turn 零写 ✓(`durable_tree_snapshot` projects 子树逐文件字节 before==after)
- NotFound/IO/JSON 错误 Result 不旧 fallback ✓(损坏 JSON/删除 → `Err(Store(_))`;tuple 出口 ProtocolError+None,无出生 record 假 SessionState)
- 本包独立可编译/测试提交 ✓;2.4 以同核心一次性 public Result cutover 并删除 tuple 封装(未留 alias/shim,`durable_projection`/`_offload` 签名未动)

## 提交(Step 5)

按 brief Step 5 显式列表 + Main 两项裁定授权文件 stage 提交:

```
git add -- src/product/workspace_engine/lifecycle.rs src/web/workspace_session/manager/durable_projection.rs src/web/workspace_session/manager/mod.rs src/web/workspace_session/tests/part_07.rs src/web/workspace_session/tests/part_02.rs
git commit -m "fix: A1b1 最新record纯投影加载与错误合同"
```

commit hash: `c8570b4c`

### 并发写面隔离(提交时点实录)

提交时主 worktree 有 45 个文件被并行 trust 波次在写(其中对本包 lifecycle.rs/durable_projection.rs/manager/mod.rs 三个文件的改动经逐 hunk 核对**全部为 cargo fmt 格式化重排**,既有 trust 代码重排+本包新行重排,零语义变化)。为不吞并行方未提交改动,提交采用索引级隔离:`git apply --cached` 本包 /tmp 已验证补丁(与绿证据逐字节一致)后 commit;格式化重排保留在 worktree 未暂存区,由运行 fmt 的 worker 随其波次提交。

## 移交

- `durable_projection`/`durable_projection_offload`/`current_session_state`/`attached_session_state` 的 public Result cutover 与旧 tuple 封装删除 → Task 2.4(A1b2,紧接,同 integration owner)
- router `LcResumeRecoveryChanged` try_lock 忙走最新 pure 读 + `workspace_durable_projection_unavailable` 全 caller 闭包 → Task 2.4
- `DurableWorkspaceProjection`/`DurableProjectionError`/`load_durable_workspace_projection`/`DURABLE_PROJECTION_UNAVAILABLE_CODE` 经 manager/mod.rs re-export 供 2.4 消费
- LC 恢复等待 reconcile(真实 `recover_on_creation`/live mismatch 收口)→ Task 2.6(A2b)
- store 扫描 ID 前缀限制事实(lifecycle_store/utils.rs:90)已报 Main 记 ledger,2.4/终审知悉
