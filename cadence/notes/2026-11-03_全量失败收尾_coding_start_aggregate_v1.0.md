# 2026-11-03 全量失败收尾 · coding_start + aggregate_initialization（worker: FixCodingStartAggregate3）v1.0

> **范围**：仅收 `cargo test --locked --all-targets` 全量实测中本 worker 名下的 **2 条**失败（族 C 收尾批）。
> **git 纪律**：全程未 `git add` / `commit` / `stash` / `checkout` / `restore`。
> **生产实现**：**零改动**（本片只补夹具前置事实 + 迁移一条已作废的旧形态断言）。
> **测试删除**：零删除。

---

## 0. 结论速览

| # | 用例 | 根因族 | 改法 | 结果 |
|---|---|---|---|---|
| 1 | `web::coding_start::tests::restart_after_abort_readmits_attempt_in_same_process` | **族 A（无 LC 权威）＋ 族 C（target_snapshot 缺失）** | 补齐缺失前置事实：复用共享夹具 `logical_test_fixture::seed_complete` 建 LC 权威 + 归属 issue，并给 attempt 挂与权威逐字一致的冻结快照 | ✅ 转绿 |
| 2 | `web::handlers::aggregate_initialization::tests::shared_executor_cannot_reach_repository_registration_or_git_finalize` | **族 D（旧形态断言随片4/REQ-BOOT-07 作废，迁移未完成）** | 把 `write_boundary` / `resume` 两格的旧形态断言（`must stay Unknown`）迁到与已迁移的 `launch` 格同形（`== 起跑快照逐格相同`） | ✅ 转绿 |

**需改生产实现的条目：无。** 两条都是「测试现场前置事实缺失」/「断言形态随契约变更作废」，生产语义（REQ-ROUTE-01 两态化、REQ-BOOT-07 隔离契约）**未动**。

**被改文件与行数（供 Main 统一登记 `tests/it_core/large_file_guard.rs`；本次**未碰**该文件）**：

| 文件 | 改后行数 | 改前行数 | 净变化 | 是否已在 large_file_guard 登记 |
|---|---|---|---|---|
| `src/web/coding_start_parts/tests_parts/restart_and_multirepo.inc.rs` | **593** | 596 | −3 | 否（grep 无命中） |
| `src/web/handlers/aggregate_initialization/tests_parts/root_safety.inc.rs` | **659** | 660 | −1 | 否（grep 无命中） |

---

## 1. 用例一：`restart_after_abort_readmits_attempt_in_same_process`

### 1.1 根因族判定：**族 A（无 LC 权威）＋ 族 C（target_snapshot 缺失）**

断言：`assert_eq!(result.state, OperationState::Accepted)` 实得 `NeedsHuman`。

`restart_coding_attempt`（`src/web/coding_start.rs:170`）的前置链顺序为
「命令账本 → 版本/终态校验 → 租约判定 → registry 退役清理 → **durable 重开 admission** → WI 级工作树锁复位 → spawn 新 runner」。
`NeedsHuman` 只有 5 个出口；逐个排除：

| 出口 | 判定 | 依据 |
|---|---|---|
| 租约 `ActiveWait`（他人活跃） | **排除** | `classify_worktree_lease_for_attempt`（`src/product/coding_attempt_store/run_exclusion.rs:353`）：attempt 无 `target_snapshot` ⇒ 走 `RepositoryRouting::load_for_issue` 分支，返回 `UnknownNeedsHuman`（`target_snapshot_missing_for_logical` / `repository routing load failed`），**不是** `ActiveWait` |
| registry `StillStopping` | **排除** | 该分支的 reason 是 `coding_restart_still_stopping: 旧运行尚未退出`；本用例的 abort 只置退役标记、不留活跃 runner |
| **`coding_restart_failed: {error}`** | ✅ **命中** | `restart_terminal_attempt_for_execution` → `admit_attempt_for_execution_locked`（`src/product/coding_attempt_store/admission.rs:526`）返回 `Err` |
| `coding_restart_worktree_lock: …` | 未到达 | 在 admission 之后 |
| `coding_restart_runner_spawn_failed` | 未到达 | 在 admission 之后 |

### 1.2 具体断点（族 A / 族 C 的确切落点）

`admit_attempt_for_execution_locked`（`admission.rs:571-620`）按 **authority resolver → routing → 快照/policy** 顺序 fail-closed：

1. **族 A：无 LC 权威** — `RepositoryAuthorityResolver::resolve_for_issue`（`src/product/logical_codebase/repository_routing_resolver.inc.rs:23`）
   - 本用例原现场**只有 issue + attempt，没有 project 记录**，也没有 manifest/member/checkout/selection/policy；
   - resolver 先调 `ProjectStore::get(project_id)` → `NotFound` → **直接 `return Ok(None)`**（`repository_routing_resolver.inc.rs:52-56`），权威解析「落空」；
   - 调用方据此 `RepositoryRouting::unbound_issue()` ⇒ `FailClosed(TargetMissing)`。
2. **族 C：无冻结快照** — 原 attempt 传 `target_snapshot: None`；
   `match (routing, attempt.target_snapshot)` 命中 `(FailClosed{..}, _) => Err(TARGET_SNAPSHOT_IDENTITY_DRIFTED)`（`admission.rs:610`）；
   若路由为 `Logical` 且无快照，则命中 `TARGET_SNAPSHOT_MISSING_FOR_LOGICAL`（`admission.rs:590`）——两条分支都在 admission 处 `Err` ⇒
   `restart_coding_attempt` 把 `Err` 映射成 `NeedsHuman("coding_restart_failed: …")`（`src/web/coding_start.rs:326-341`）。

⇒ **期望值 `Accepted` 本身没错，错的是测试现场**：REQ-ROUTE-01 两态化（去单仓入口）之后，durable 重开 admission 的唯一凭证是
**「LC 权威 + 冻结 `target_snapshot`」**，本用例两样都没有，于是测到的是 fail-closed，而不是「重开 → spawn 新 runner」这条不变量。

### 1.3 改法（补前置事实，**不动断言、不动生产实现**）

复用**共享夹具**（不重造）：`src/product/logical_codebase/logical_test_fixture.rs`（只读，本次未改）

- `seed_complete(&paths, &root, "project_0001", "issue_0001")`：建 project 记录 + legacy 布局的 LC 权威
  （manifest / member / checkout / selection / policy artifact / `repos.json` 投影）**＋ 归属该 LC 的 issue**；
- `logical.snapshot(&paths)`：按当前权威事实构造的冻结 `AttemptTargetSnapshot`
  （policy digest / membership revision 逐字取自 store ⇒ `validate_snapshot_fields` 必然通过）。

**改前 → 改后**（`src/web/coding_start_parts/tests_parts/restart_and_multirepo.inc.rs`）：

| 位置 | 改前 | 改后 |
|---|---|---|
| `:12` | `use crate::product::issue_store::{CreateProductIssueInput, IssueStore};` | （删除，改用共享夹具建 issue） |
| `:24-35` | 手搓 `IssueStore::new(paths).create(CreateProductIssueInput{ … logical_codebase_id: legacy_logical_codebase_id("project_0001"), … })`（**只建 issue，无 project、无 LC 权威**） | `let logical = crate::product::logical_codebase::logical_test_fixture::seed_complete(&paths, &root, "project_0001", "issue_0001");`（**建 project + 完整 LC 权威 + 归属 issue**） |
| `:51` | `target_snapshot: None,` | `target_snapshot: Some(logical.snapshot(&paths)),` |

**未改**：断言全部原样保留（`Accepted` / `Replayed` / `Rejected` / `runner_count == 1` / `status == Running`）。

### 1.4 红 → 绿输出

**红**（Main 转交的全量实测）：

```
1) web::coding_start::tests::restart_after_abort_readmits_attempt_in_same_process
   src/web/coding_start_parts/tests_parts/restart_and_multirepo.inc.rs:87
   assertion `left == right` failed
     left: NeedsHuman
    right: Accepted
```

**绿**（窄跑，`flock /tmp/aria-cargo.lock` + `cargo test --locked --lib -- <两个用例名> --nocapture`）：

```
running 2 tests
test web::coding_start::tests::restart_after_abort_readmits_attempt_in_same_process ... ok
test web::handlers::aggregate_initialization::tests::shared_executor_cannot_reach_repository_registration_or_git_finalize ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 5969 filtered out; finished in 0.40s
```

### 1.5 concerns

- 本用例的**断言一字未动**，改的是现场：原现场只建了 issue（还带一个**指向不存在的 LC** 的 `logical_codebase_id`），与 REQ-ROUTE-01 两态化的准入前提不符；
  现在走共享夹具建出**真实归属**，测的仍是「abort → 同进程重开 admission → spawn 新 runner → 同 command 重放 → 错版本 Rejected」这条不变量。
- 共享夹具 `logical_test_fixture::seed_complete` 会 `git init` 一个成员仓（真实 git 仓是 `build_attempt_target_snapshot` / cross-target 采样的前提）；
  本用例不需要成员仓内容，只借它的**权威与快照**形状。
- `logical_test_fixture.rs` 本次**只读引用，未改**（该文件 mtime 20:42:03，属他人改动中）。

---

## 2. 用例二：`shared_executor_cannot_reach_repository_registration_or_git_finalize`

### 2.1 根因族判定：**族 D（旧形态断言随片4 / REQ-BOOT-07 作废，迁移未完成）**

断言：`"normal write_boundary cell for PlanningReadOnly must stay Unknown"`（`root_safety.inc.rs:644`）。

### 2.2 「谁写了 `PlanningReadOnly` 的 `write_boundary` 单元格」——定位结论

**答：是本用例夹具的「选择器前置」写的，发生在 recipe 起跑之前；recipe 运行路径没有写它。**

证据链：

1. **写入方**：`src/web/handlers/aggregate_initialization/tests.inc.rs:97` 的 `with_selector_defaulting_to_claude` →
   `crate::web::handlers::automation_enrollment_test_support::seed_confirmed_capability_records(paths, "project_0001", lc_id, &[ProviderRefType::ClaudeCode])`
   （`automation_enrollment_test_support.rs:850`）→ 写出的记录形状是 `confirmed_capability_record`（`automation_enrollment_test_support.rs:785`）：
   ```rust
   let row = |action: SessionPolicyAction| ProviderActionCapability {
       action,
       launch: ProviderCapabilityEvidence::Confirmed,
       resume: ProviderCapabilityEvidence::Confirmed,
       write_boundary: ProviderCapabilityEvidence::Confirmed,   // ← 就是这一格
       …
   };
   ```
   即 **三个 canonical action 的 launch/resume/write_boundary 全被写成 `Confirmed`**（不是只写 `PlanningReadOnly.launch`）。
2. **recipe 运行路径没有写它**：聚合初始化执行期对 capability store 的唯一触碰是
   `src/product/logical_codebase/coordinator_provider_turn.inc.rs:955` 的 `ProviderCapabilityStore::for_lc(…).ensure_bootstrap(&project.id)`，
   而 `ensure_bootstrap` **文件已存在即跳过、不覆盖**（`provider_capability_store.rs:867-871`：「文件已存在(含用户 upsert 后)直接 `Ok` 跳过,不覆盖」）。
3. **运行前后同值**：失败发生在 `launch` 格断言**之后**（同一循环体、同一 action），而 `launch` 断言 `row.launch == before.launch` **已通过**
   ⇒ 起跑前后记录未被重写 ⇒ `write_boundary` 格的 `Confirmed` **来自夹具前置，不是 recipe 铸造的**。
   （迁移后新增的 `row.write_boundary == before.write_boundary` 断言把这一点固化为可执行不变量。）

### 2.3 契约出处核实（族 D 的前置条件）

- **用例自身的注释（随片4 / REQ-BOOT-07 同步更新）**（`root_safety.inc.rs:615-620`）已把判定口径改写为：
  「片4（REQ-BOOT-07）起 fixture 起跑前由选择器前置种下 ClaudeCode 的 PlanningReadOnly Confirmed（选择期 L2 事实），
  **故判定改为「跑完后 normal 矩阵与起跑快照逐格相同」**：五步 recipe 完整 Completed(上方断言)证明 root-recipe 相位只消费固定 Claude recipe 事实,不被 normal 分格形态阻断。」
  ——但**只迁移了 `launch` 一格**（`assert_eq!(row.launch, before.launch)`），`write_boundary` / `resume` 两格仍留在旧形态（`must stay Unknown`）⇒ 迁移**半途而废**，这就是本条失败的直接来源。
- **规格出处**：`openspec/specs/logical-codebase-registration/spec.md`
  - 「#### Scenario: 普通会话不得复用 root recipe 能力证据」：
    「系统 SHALL 仍按普通会话的 capability/gateway 条件拒绝，**不把 recipe 证据升级为普通会话 Confirmed**，不借成员规则豁免绕过 capability 门」；
  - 同一 requirement 的 bootstrap 场景：「合法 recipe 不先被普通会话的 Normal capability 矩阵拒绝，**不伪造 Confirmed**、不写成员规则」。
  ⇒ 规格约束的是**相位隔离**（recipe 不得铸造/降级 normal 分格），**并未**要求 normal 分格恒为 `Unknown`；
  「恒 Unknown」只是片4 之前「fixture 起跑时无任何 probe 行」这一现场事实的副产品，夹具一变即作废。
  ⇒ **旧形态断言钉的是已作废的现场形态，契约出处已核实 ⇒ 属族 D 迁移。**

### 2.4 改法（完成迁移，**不改语义方向**：仍断「recipe 既不铸造、也不降级 normal 分格」）

`src/web/handlers/aggregate_initialization/tests_parts/root_safety.inc.rs`：

| 位置 | 改前 | 改后 |
|---|---|---|
| `:644-647` | `assert!(matches!(row.write_boundary, ProviderCapabilityEvidence::Unknown), "normal write_boundary cell for {action:?} must stay Unknown");` | `assert_eq!(row.write_boundary, before.write_boundary, "normal write_boundary cell for {action:?} must be unchanged by the recipe run");` |
| `:648-651` | `assert!(matches!(row.resume, ProviderCapabilityEvidence::Unknown), "normal resume cell for {action:?} must stay Unknown");` | `assert_eq!(row.resume, before.resume, "normal resume cell for {action:?} must be unchanged by the recipe run");` |
| `:621` | `use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;` | （删除；迁移后不再引用该枚举） |

`before` 即用例里已有的 `pre_run_claude.action_matrix.row(&action)`（起跑快照，`root_safety.inc.rs:638`），与已迁移的 `launch` 格**同形同源**。

**未改**：`launch` 格的既有迁移（块3 之前落地）、其余全部断言（root 清单对照、零 Repository 记录、零单仓 operation、3 次 gateway 启动、policy digest、receipt 冻结等）一律原样。

### 2.5 红 → 绿输出

**红**（Main 转交的全量实测）：

```
2) web::handlers::aggregate_initialization::tests::shared_executor_cannot_reach_repository_registration_or_git_finalize
   src/web/handlers/aggregate_initialization/tests_parts/root_safety.inc.rs:644
   "normal write_boundary cell for PlanningReadOnly must stay Unknown"
```

**绿**：见 §1.4 的窄跑输出（`… shared_executor_cannot_reach_repository_registration_or_git_finalize ... ok`）。

### 2.6 concerns

- 迁移后的 `write_boundary` / `resume` 断言判的是「**与起跑快照逐格相同**」，不再是「恒 Unknown」。
  在本夹具下起跑快照三格皆 `Confirmed`，因此**「recipe 铸造 normal Confirmed」这一旧检测点在起跑快照已 Confirmed 时不再由这两格独立承担**；
  相位隔离的完整防线仍由 (a) `launch` 格同形断言、(b) 规格「recipe 证据不得升级为普通会话 Confirmed」的生产侧门（`root_recipe_evidence` 独立格）承担。
  若后续要把「起跑为 Unknown 的格不得被 recipe 铸造」也钉死，需要另立一个**不预种 Confirmed** 的夹具用例（不在本片范围内）。
- 未改夹具侧 `seed_confirmed_capability_records` / `confirmed_capability_record`：该 helper 是**多个** provider 选择用例共用（选择器需要三格 Confirmed），
  且其文件 mtime 20:42:04（他人改动中）——按 mtime 避让纪律**未碰**。

---

## 3. 纪律自证

| 约束 | 落实 |
|---|---|
| 不改断言凑绿 | 用例一**零断言改动**；用例二仅迁移**已作废旧形态**（族 D），且**核实了契约出处**（用例自身注释 + `openspec/specs/logical-codebase-registration/spec.md` 的相位隔离场景） |
| 不改生产实现 | 生产代码**零改动**；两条均在测试文件内解决 |
| 不恢复 `RepositoryRouting::Legacy` | 未触碰（该变体本就已在块3 删除） |
| 不自造 reason_code | 未新增任何 reason_code / 稳定码 |
| 不改 spec / Plan / 技术方案 | 未改 `openspec/**`、未改任何计划/方案文档 |
| 不删测试 | 零删除 |
| 不 `git add/commit/stash/checkout` | 全程未执行 |
| 不碰 `tests/it_core/large_file_guard.rs` | 未碰；被改文件行数已在 §0 上报（593 / 659，且两文件均未在该门禁登记） |
| mtime 避让 | 改前 `ls -la --time-style=+%H:%M:%S` 核对：两个目标文件 mtime 为 10:44:39 / 11:25:08（早于 20 分钟窗口）；`logical_test_fixture.rs`(20:42:03) 与 `automation_enrollment_test_support.rs`(20:42:04) 属他人改动中，**只读引用、未修改** |
| 窄跑只排一次 / 不起第二个 cargo | 全程用 `flock /tmp/aria-cargo.lock` 串行；无并发 cargo 任务；无挂死，无需 kill |
| 未跑本族之外的全量 | 只跑了两个用例名的窄过滤 + 本族两个模块的回归窄跑（见 §3.1） |

### 3.1 本族模块回归窄跑（同 `flock` 串行，未跑全量）

```
cargo test --locked --lib -- web::coding_start::tests web::handlers::aggregate_initialization::tests
→ test result: ok. 53 passed; 0 failed; 0 ignored; 0 measured; 5918 filtered out; finished in 4.36s
```

（两个被改文件所在模块的全部 53 条用例通过 ⇒ 同文件兄弟用例无连带回归。）

---

## 4. 遗留 / 待 Main 统一处理

1. **`tests/it_core/large_file_guard.rs` 登记数字**：本片两个被改文件**均未**在该门禁登记（grep 无命中），无需登记数字；若 Main 的门禁按目录/文件白名单另行统计，请按 §0 表中的行数（593 / 659）核对。
2. **全量回归**：本片只做了窄跑（2/2 绿）；`cargo test --locked --all-targets` 全量由 Main 统一跑（另 11 条失败属其他 worker）。
3. **共享夹具 mtime**：`src/product/logical_codebase/logical_test_fixture.rs`（20:42:03）与 `src/web/handlers/automation_enrollment_test_support.rs`（20:42:04）在本片开工前 20 分钟内被他人改动，本片**只读引用**；若那些改动引入新语义，本片结论需复核。
