# 失败簇收尾报告 — `web::coding_ws_handler`（v1.0）

> 日期：2026-11-03
> 工作树：`.worktrees/bugfix-b-11-02`
> 范围：**只改夹具**（`src/web/coding_ws_handler/tests/plan_repair/**` +
> `src/web/coding_ws_handler/tests/runner_recovery/startup_window_compensation.rs`）。
> 断言 / 实现（`src/product/**`、`src/web/coding_ws_handler/runner/**`）/ spec / Plan / 技术方案
> **一字未改**；未恢复 `RepositoryRouting::Legacy`；未自造 reason_code；未删断言。

---

## 0. 结论

```
cargo test --locked --lib coding_ws_handler
⇒ 176 passed / 0 failed
```

接手时 **172 passed / 4 failed** ⇒ 本轮**收掉全部 4 条**（簇基线 58 failed ⇒ 累计收 58 条）。
**未改任何生产实现**（无「需改实现」的条目需要 Main 授权）。

---

## 1. 4 条用例名 + panic 原文 + 根因族

窄跑命令（单次，`flock /tmp/aria-cargo.lock`）：

```
flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib coding_ws_handler -- --nocapture 2>&1 | tail -120'
```

| # | 用例（`web::coding_ws_handler::tests::…`） | panic 原文（接手时实测） | 根因族 |
|---|---|---|---|
| 1 | `plan_repair::failed_review_recovery::coding_plan_repair_recovered_reviewer_retry_can_start_plan_repair` | `src/web/coding_ws_handler/tests/plan_repair/failed_review_recovery.rs:230:5:`<br>`assertion \`left == right\` failed: retry reviewer report must persist; journal=Some(FailedCodeReviewRecoveryJournal { … phase: Completed, runner_started_at: Some("2026-11-03T11:50:26.218106372+00:00") … })`<br>`  left: 0` / `  right: 1` | **B**（provider 准入事实缺失） |
| 2 | `plan_repair::provider_start_failure_recovery::coding_plan_repair_provider_start_failure_recovery_legacy_can_retry_again` | `src/web/coding_ws_handler/tests/plan_repair/provider_start_failure_recovery.rs:223:5:`<br>`assertion \`left == right\` failed`<br>`  left: Some("coding_runner_failed_while_running")` / `  right: Some("code_review_provider_interrupted")` | **B** |
| 3 | `plan_repair::provider_start_failure_recovery::coding_plan_repair_provider_start_failure_recovery_modern_can_retry_again` | 同 #2（同文件同行） | **B** |
| 4 | `runner_recovery::startup_window_compensation::bad_tree_fail_closed_while_conflict_free_tree_resumes` | `src/web/coding_ws_handler/tests/runner_recovery/startup_window_compensation.rs:1135:10:`<br>`分诊后 runner 建立 provider 会话(role run)` | **A + C + B** |

### 归因

**#1/#2/#3（根因族 B：provider 准入事实缺失）**

这三条走 **runner 路径**（`spawn_coding_runner_reserved` ⇒ `run_coding_runner_task_body`），
provider 只能经 `LogicalCodebaseProviderGateway` 启动（REQ-ROUTE-01 两态化后
`is_logical_target = attempt.target_snapshot.is_some()` ⇒ legacy 直连与 legacy
`run_streaming` 回退被强制关闭）。夹具缺三段前置中的两段：

1. **gateway factory 与 provider registry 不同源**：夹具直改 `state.provider_registry`
   （`Arc::new(registry)`），而 `LogicalCodebaseGatewayFactory` 在**构造期**持 registry
   快照 ⇒ factory 仍持默认 registry ⇒ gateway 解析不到夹具的 fake provider。
2. **durable capability 记录缺失**：gateway 的能力源是
   `StoreBackedProviderCapabilitySource`（读 durable `capabilities.json`），
   launch/resume/write_boundary 三格须 `Confirmed` ⇒ 缺记录即
   `provider_capability_launch_not_confirmed` fail-closed（provider 零启动）。
3. **availability gate 读真实健康快照**：单测环境无真实 probe ⇒ 生产门恒拒 ⇒
   `provider_gateway_unavailable`。

⇒ 症状统一表现为「runner 起来了但 provider 没起来」：
#1 的 code review 报告没落盘（`reports.len() == 0`），
#2/#3 的 retry role run 被判 `coding_runner_failed_while_running`（而不是用例期望的
`code_review_provider_interrupted`——后者要求 provider **真的起来**再在 ProviderStart
处注入故障）。

**#4（根因族 A + C + B）**

夹具 `WindowStartupFixture` 建的是「无 LC 权威 + `target_snapshot: None`」的 attempt：

* **族 A**：issue 归属字段写死 legacy 别名 id，但该 LC 的权威（manifest/member/
  checkout/selection/policy/repos.json）**没建** ⇒ authority resolver 直读
  `logical-codebases/{alias}/record.json` 落空 ⇒ `NotFound { kind: "logical_codebase" }`。
* **族 C**：路由切到 Logical 后 `classify_worktree_lease_for_attempt` 要求快照 ⇒
  无快照 ⇒ 证据 `target_snapshot_missing_for_logical` ⇒ `LeaseUnknown` fail-closed ⇒
  provider run 停等（`coding_run_lease_unknown`）⇒ **零 provider 会话**（用例末尾
  `expect("分诊后 runner 建立 provider 会话(role run)")` 失败）。
* **族 B**：即便补了快照与权威，provider 仍需 durable capability 记录 + 恒健康门 +
  registry 可解析的**真实 provider 名**（`Fake` 不进 gateway 映射，
  `ProviderRef::from_provider_name(Fake)` 恒失败）。

---

## 2. 逐条改法（改前 → 改后）

### 2.1 `plan_repair` 夹具（`src/web/coding_ws_handler/tests/plan_repair/support.rs`）

| 改点 | 改前 | 改后 |
|---|---|---|
| **durable capability 记录**（族 B） | 无 ⇒ gateway 能力源 `UnsupportedCapability(launch not confirmed)` | `seed_confirmed_capability_records(&paths, "project_0001", None, &[ClaudeCode, Codex])`（复用 **web 侧共享支持模块** `src/web/handlers/automation_enrollment_test_support.rs` 的现成 helper，未另造一份） |
| **coding worktree 身份** | `tmp/worktree`（独立 git 仓）⇒ 生产 target resolver 复验 git-dir identity 时 `TargetMismatch { field: "git_dir" }` | worktree 恒取 LC 成员主 checkout `logical.member_path`（生产形态是 checkout 下的嵌套 worktree，同一 git-dir 身份）⇒ 复验通过 |
| **runner 状态接线**（新增 `runner_state_with_provider`） | `WebAppState::new(...)` + 直改 `state.provider_registry` ⇒ factory 持旧 registry + 生产健康门恒拒 | `WebAppState::with_provider_registry(...)`（重建 factory）+ 用**共享** `always_available_gate()` 重建 factory（registry 与 factory 同源、门恒健康） |

### 2.2 `plan_repair` 两条用例（`failed_review_recovery.rs` / `provider_start_failure_recovery.rs`）

| 改点 | 改前 | 改后 |
|---|---|---|
| runner 状态 | `WebAppState::new(...)` + `state.provider_registry = Arc::new(registry)` | `super::support::runner_state_with_provider(fixture._tmp.path(), registry)` |
| **provider 替身的启动面**（族 B） | 替身只实现裸 `start` / legacy `run_streaming` ⇒ gateway 只能调 `start_validated`（默认实现恒 `streaming provider validated start is not supported by this adapter`） | 替身补 `start_validated`：拆出 input 后与各自产物面同路径（modern ⇒ 原 `start`；legacy ⇒ 原 `run_streaming` 的产物与 ProviderStart 故障注入点折成 validated 会话事件） |

### 2.3 `startup_window_compensation`（`src/web/coding_ws_handler/tests/runner_recovery/startup_window_compensation.rs`）

**只在本用例内生效**（同文件其余 20+ 条用例的夹具路径**未改**）：

| 改点 | 改前 | 改后 |
|---|---|---|
| 好树 worktree 路径 | `new_in(root, "treeB")` ⇒ `root/treeB/worktree`（与成员 checkout 不同仓） | 新增 `new_in_with_worktree(root, label, worktree)` ⇒ 好树 worktree 恒取 LC 成员主 checkout（git-dir identity 复验通过） |
| **族 A：LC 权威** | 无 | `logical_test_fixture::seed_logical_authority(&paths, project, issue, root)`（复用**产品级共享夹具**，未自造）⇒ manifest/member/checkout/selection/policy/repos.json 齐备 |
| **族 C：冻结快照** | `target_snapshot: None` | `logical_test_fixture::snapshot_for(...)` + `attach_target_snapshot(...)`（与权威逐字一致） |
| **族 C：仓维租约** | 无仓维记录/锁 | `upsert_repo_worktree` + `bind_repo_lock_to_attempt`（带快照的 attempt 只按仓维分类租约；自持活跃租约放行） |
| **族 B：provider 准入** | provider = `Fake`（不进 gateway 映射）+ 无 capability 记录 + 生产健康门 | `switch_to_gateway_provider()`（切 `ClaudeCode`）+ `seed_confirmed_capability_records(..., &[ClaudeCode])` + 新增 `restart_state_with_provider()`（注入 registry 的构造路径 + 共享 `always_available_gate()`）+ 夹具替身 `FixtureClaudeProvider`（实现 `start_validated`） |

---

## 3. 验证（红 → 绿）

```
# 接手时（本次窄跑实测）
cargo test --locked --lib coding_ws_handler
  ⇒ 172 passed / 4 failed
  ⇒ 失败清单（4 条）：
     plan_repair::failed_review_recovery::coding_plan_repair_recovered_reviewer_retry_can_start_plan_repair
     plan_repair::provider_start_failure_recovery::coding_plan_repair_provider_start_failure_recovery_legacy_can_retry_again
     plan_repair::provider_start_failure_recovery::coding_plan_repair_provider_start_failure_recovery_modern_can_retry_again
     runner_recovery::startup_window_compensation::bad_tree_fail_closed_while_conflict_free_tree_resumes

# 改完后（同一窄跑命令，最终树再跑一次复核）
cargo test --locked --lib coding_ws_handler
  ⇒ test result: ok. 176 passed; 0 failed; 0 ignored; 0 measured; 5795 filtered out
```

同文件其余用例（`plan_repair::runner_amendment_recovery` 2 条、`typed_sources` 5 条、
`startup_window_compensation` 其余 20+ 条）**保持绿**（同一窄跑内全部 `ok`）。

---

## 4. 改动清单（`git diff --stat`，本轮只改夹具）

```
 src/web/coding_ws_handler/tests/plan_repair/failed_review_recovery.rs       |  25 ++-
 src/web/coding_ws_handler/tests/plan_repair/provider_start_failure_recovery.rs |  52 ++-
 src/web/coding_ws_handler/tests/plan_repair/support.rs                     | 131 +++++++---
 src/web/coding_ws_handler/tests/runner_recovery/startup_window_compensation.rs | 251 +++++++++++++++--
```

（`git diff --stat` 里另有 `context.rs` / `runner.rs` / `tests.rs` / `plan_repair.rs` /
`typed_sources.rs` / `runner_cleanup.rs` / `runner_recovery.rs` /
`tests_parts/manual_recovery.inc.rs` —— 那是**前一 worker** 的改动，本轮未动。）

### 行数实测（**登记由 Main 统一改，我没碰 `tests/it_core/large_file_guard.rs`**）

| 文件 | 改前行数 | 改后行数 | 是否在门禁表内 |
|---|---|---|---|
| `src/web/coding_ws_handler/tests/runner_recovery/startup_window_compensation.rs` | 1834 | **2047** | **在**（登记 1834 ⇒ **需上调到 2047**） |
| `src/web/coding_ws_handler/tests/plan_repair/support.rs` | 705 | 758 | 否 |
| `src/web/coding_ws_handler/tests/plan_repair/failed_review_recovery.rs` | 340 | 349 | 否 |
| `src/web/coding_ws_handler/tests/plan_repair/provider_start_failure_recovery.rs` | 547 | 583 | 否 |
| `src/web/coding_ws_handler/runner.rs` | 2924 | 2924（本轮**未改**） | 在（登记 **2880** ⇒ 仍需 Main 上调到 2924） |

---

## 5. mtime 避让上报

本轮开工（19:41）前对 `src/web/coding_ws_handler/**`、
`src/web/handlers/automation_enrollment_test_support.rs`、
`src/product/logical_codebase/logical_test_fixture.rs` 逐个
`ls -la --time-style=+%H:%M:%S`：**开工时无 20 分钟内被改动的目标文件**
（最近的是 `automation_enrollment_test_support.rs` 19:32——**只读复用，未改**；
`logical_test_fixture.rs` 18:48——**只读复用，未改**）。
本轮**未触碰** `src/product/**` 任何文件、**未触碰**
`src/web/workspace_session/**`、`src/web/workspace_ws_handler/**`、
`tests/it_core/large_file_guard.rs`。

⚠️ 期间撞到两处**别人在途**的编译错，本轮**未改**（按 mtime 纪律等 owner）：
`src/web/workspace_session/tests/part_01.rs`（E0716，owner `CloseLastTwo`，已修）、
`src/web/coding_ws_handler/tests/plan_repair/support.rs:535`（E0382，Main 代修）。

---

## 6. Concerns（须 Main 知悉）

1. **#2（legacy 变体）的语义位移 —— Main 已裁决：不改名、不合并**。
   REQ-ROUTE-01 之后，逻辑 target 的 legacy `run_streaming` **回退**已被强制关闭
   （`allow_legacy_stream_fallback` 对 `is_logical_target` 恒 false）⇒ 两条用例走同
   一条 validated 启动面。但 Main 查证后确认：`ProviderStreamMode::Modern | Legacy`
   指的是 **provider 的流式输出模式**（现代流 vs 旧文本流），**不是仓库路由**
   （`RepositoryRouting::Legacy` 枚举已删，只剩 Logical / FailClosed 两态）。
   ⇒ 本轮做法（替身 `start_validated` 按 mode 分派：Modern 面与裸 `start` 同路径、
   Legacy 面沿用原 `run_streaming` 的产物与 ProviderStart 故障注入点）**保住了两条
   用例各自的 provider 流式观测点**，断言与用例名**一字未改**，名字仍然准确。
2. **#4 的夹具形态**：好树的 coding worktree 被迫与 LC 成员主 checkout **同目录**
   （生产形态是 checkout 下的嵌套 worktree）。共享夹具
   `logical_test_fixture` 目前**没有**「接受调用方传入既有 repository 路径」的能力
   （Main 已知并已上报），本轮用「worktree 路径 = 成员 checkout 路径」的形态绕开，
   **未自造新夹具**。
3. **`WindowStartupFixture` 的改动面**：本轮给该夹具加的是**增量方法**
   （`new_in_with_worktree` / `restart_state_with_provider` /
   `switch_to_gateway_provider` / `attach_target_snapshot`），**既有构造路径
   （`new` / `new_in` / `restart_state`）行为未变** ⇒ 同文件其余 20+ 条用例零影响
   （窄跑实测全绿，Main 已确认此口径正确：共享夹具**只加不改**）。但
   `startup_window_compensation.rs` 已 **2047 行**（登记 1834）⇒ **Main 统一上调
   门禁登记**（已进 Main 台账 A 组；`runner.rs` 登记 2880 → 2924 一并由 Main 改）。
4. **未验证面**：本轮只跑了 `cargo test --locked --lib coding_ws_handler` 窄跑；
   **未跑** `cargo test --all-targets` / `it_core`（含 `large_file_guard`——
   该门禁在 Main 上调 `startup_window_compensation.rs` 登记前**会红**）。
