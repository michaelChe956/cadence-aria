# 块3 片1 · Web 侧 `RepositoryRouting::Legacy` 12 处改写报告 v1.0

- **worker**: LegacyFixWeb（glm53-task）
- **日期**: 2026-11-03
- **worktree**: `/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- **任务**: 改写 `src/web/**` 12 处 `RepositoryRouting::Legacy` 使用点（7 文件），使 `Legacy` 变体可整体移除（枚举定义已由 B3S1 删为 `Logical`/`FailClosed` 两态）。

## 0. 结论

- `grep -rn 'RepositoryRouting::Legacy' src/` → **0 命中**（全仓，含 product 侧另一 worker 的成果）。
- 改动文件（9 个）：原清单 7 个 + Main/brief 授权追加的 2 个（`support_task11_tests.rs` 删孤立测试/修夹具、`lifecycle_aggregate_scope.inc.rs` 删死函数）。
- 行数门禁：全部文件 ≤ 登记上限，**未动登记表**。
- TDD 说明：**本片无 RED 取证**（crate 编译阻塞 + 纯机械改写），以编译通过 + 现有测试不回归作证据。

## 1. 逐处改写（12 处）

口径：`Legacy` 臂删除；原逻辑**合并进 Logical** 或（旧数据/异常语义）**由 resolver 层 FailClosed 承接**。本片所有 12 处均为 **match 消费臂**（1 处构造点），无一处需要新造错误码——旧数据的 FailClosed 形态由 `RepositoryRouting::load_for_issue`/`classify` 在上游统一产出（`(None,None)→unbound_issue()`＝`FailClosed{TargetMissing,"issue_not_bound_to_logical_codebase:…"}`），web 侧只需删臂让既有 FailClosed 臂穷尽。

### 1.1 `src/web/handlers/coding.rs:118`（原 :118）

- 改前：`RepositoryRouting::Legacy { .. } => Ok(None),`（`attempt_target_snapshot`，单仓无逻辑快照）
- 改后：**整臂删除**（Logical 臂产快照、FailClosed 臂报错的既有逻辑不动）
- 选型：**删臂＝改 FailClosed 承接**。单仓/无归属旧数据在新两态下由 `load_for_issue` 产出 `FailClosed(TargetMissing)`，走既有 FailClosed 臂报稳定码；不再有「静默无快照」第三态。
- 连带：`:111` 起的 `lc_id` 现为 `String`（resolver 签名 `Result<String,_>`），`:130` `lc_id.as_deref()` → `Some(lc_id.as_str())`；注释去掉「回退 legacy project 级路径」措辞。

### 1.2 `src/web/handlers/coding/group.rs:318`（`group_target_snapshot`）

- 改前：`RepositoryRouting::Legacy { .. } => Ok(None),`
- 改后：整臂删除。理由同 1.1（FailClosed 承接，稳定码上抛）。
- 连带：`:335` `lc_id.as_deref()` → `Some(lc_id.as_str())`。

### 1.3 `src/web/handlers/coding/group.rs:406`（`resolve_group_repository`）

- 改前：Legacy 臂取 `issue.repo_id`（缺失则 `NotFound{kind:"repository"}`）→ `resolve_legacy_group_repository`（物理仓 dual 解析 + 列表回退）。
- 改后：**整臂删除**（Logical 臂经 selection 校验 + `resolve_logical_repository_for_issue_codebase`；FailClosed 臂既有）。
- 选型：**改 FailClosed 承接**。单仓物理解析路径是 Legacy 的存在理由本身，随变体一并移除；无归属旧数据在上游即 `FailClosed(TargetMissing)`。
- 连带（死代码清除）：唯一被该臂调用的私有函数 `resolve_legacy_group_repository`（原 :662-690）整体删除；`IssueStore` 导入随之删除；`:435`/`:481` `lc_id.as_deref()` → `Some(lc_id.as_str())`；`match lc_id.as_deref() { Some(_)=>new, None=>for_project }`（原 :469-477）收敛为 `RepositoryStore::new(app_paths.clone())`（project 级回退随 Legacy 删除，与 B3S1「project 级 store 的 None 回退已删」一致）。

### 1.4 `src/web/handlers/coding/group.rs:518`（`split_group_targets`）

- 改前：`RepositoryRouting::Legacy { .. } => Ok(SplitGroupTargets::Legacy),`
- 改后：整臂删除；`SplitGroupTargets` 枚举只剩单变体 `Logical`，**失去存在意义，整体移除**——`split_group_targets` 改返回 `ApiResult<(String, BTreeSet<LogicalRepositoryId>, Vec<LogicalRepositoryId>)>`，字段 `lc_id` 由 `Option<String>` 收敛为 `String`。
- 选型：**改 FailClosed 承接**（无归属→上游 FailClosed）；两个 `#[allow(dead_code)]` 多值消费函数（`group_target_snapshots`/`resolve_group_repositories`）的 let-else（原 :585-592/:624-631，else 分支即 Legacy→`Ok(None)`）改为普通元组解构，返回类型保留 `Option<…>`（恒 `Some`，最小机械，未来接线时再定形）；各自的 `match lc_id.as_deref()` 同 1.3 收敛为 `RepositoryStore::new`。
- doc 注释同步：三态→两态措辞、「Legacy 路由 → None」句删除。

### 1.5 `src/web/handlers/lifecycle.rs:124`（issue 详情/list 面）

- 改前：`Legacy` 臂 `issue.repo_id.clone().ok_or_else(NotFound{kind:"issue_repository"})?`；`Logical | FailClosed` 臂 `issue.repo_id.clone().unwrap_or_default()`。
- 改后：**只删 Legacy 臂**，保留既有 `Logical { .. } | FailClosed { .. }` 合并臂（本就同体）。
- 选型：**合并进 Logical**——该处 Legacy 语义（无条件要求 repo_id）与 Logical 语义（repo_id 非必填、权威在 manifest/selection）在新两态下只保留后者；FailClosed 在下游解析报稳定码（原注释声明的阶段A行为，不变）。

### 1.6 `src/web/handlers/lifecycle.rs:576`（`generate_story_specs`）

- 改前：Legacy 臂 `reject_aggregate_scope_on_legacy_project(involved.is_some(), false)?` + `issue.repo_id` 必填校验 + `find_repository` + `(repository_id, None)`。
- 改后：**整臂删除**（Logical 臂产聚合视野 scope、FailClosed 臂既有报错不变）。
- 选型：**改 FailClosed 承接**——「legacy 单仓项目拒绝 aggregate scope」守卫的存在前提（单仓路径）随 Legacy 消失；无归属旧数据→上游 `FailClosed(TargetMissing)`。
- 连带：`issue` 绑定仅该臂使用，改无绑定调用（保留 issue 存在性校验）；`find_repository` 在本文件无剩余调用（经 glob 导入，无孤立 import）。

### 1.7 `src/web/handlers/lifecycle.rs:686`（`generate_design_specs`）

- 改前：Legacy 臂 `reject_aggregate_scope_on_legacy_project(involved.is_some(), change_order.is_some())?` → `None`。
- 改后：整臂删除。理由同 1.6。

### 1.8 `src/web/handlers/lifecycle/plan_preparation.rs:82`（`prepare_plan_records` preflight）

- 改前：Legacy 臂 `issue.repo_id` 必填（422 `repository_required`）→ `find_repository` → 单仓 `preflight_plan_repository_candidates(&[repository.id], &[])`。
- 改后：**整臂删除**（Logical 臂的上界子集 preflight、FailClosed 臂既有 `routing_api_error` 不变）。
- 选型：**改 FailClosed 承接**——单仓 preflight 语义随 Legacy 移除。
- 连带：`issue` 绑定仅该臂使用→改无绑定调用；`use super::super::support::{find_repository, …}` 删除 `find_repository`。

### 1.9 `src/web/handlers/support_parts/coding_cleanup.inc.rs:171`（`#[cfg(test)] fn cleanup_shared_worktree_by_routing` 内）⚠️ 测试夹具处置

- 改前：`Legacy` 臂 `lifecycle.delete_issue_shared_worktree(project_id, issue_id)?`（单仓走老清理，红线）。
- 改后：**整臂删除**（Logical 臂枚举 `shared-worktrees/` 全清、FailClosed 臂既有上抛不动）；doc 注释改两态口径。
- **该函数的配套测试处置**（brief 授权二选一，选了「删测试」）：
  - `cleanup_shared_worktree_by_routing_keeps_legacy_cleanup_for_legacy_routing`（support_task11_tests.rs 原 :226-253）**删除**——它钉死的契约（「单仓 issue 删除必须继续走老 delete_issue_shared_worktree 逻辑」）随单仓路径整体消失；且其夹具不建 issue 记录，在新两态下 `load_for_issue` 直接 `Err(NotFound)`，测试无可改写的等价语义（伪造新断言需设计决策，不属于机械改写）。
  - 保留的 `…_enumerates_all_repo_worktrees_for_logical` 夹具 `t11_logical_routing_fixture` 原本**不建 issue 记录**（旧解析靠「无 issue→project 级 store 回退」命中 Logical）——两态下必红。按 Main 公布的 (a) 口径补 issue 记录：`logical_codebase_id = legacy_logical_codebase_id("project_0001")`，manifest/selection 仍在项目级根（`LogicalCodebaseStore::new`/`IssueCodebaseSelectionStore::new`），`lc_scope_root` 对该 id 恰落回项目级根 ⇒ routing 判定 Logical。与 `coding_ws_handler/tests/runner_recovery.rs:929` 既有前例同款。

### 1.10 `src/web/handlers/support_parts/coding_cleanup.inc.rs:222`（`sweep_issue_worktree_residue`）

- 改前：`Legacy { repository_id } => repositories.filter(|r| !repository_id.is_empty() && r.id == repository_id).map(path).collect()`（单仓 issue 物理仓为候选根）。
- 改后：**整臂删除**（Logical 臂 LC 子树 checkouts + repos.json 回退既有；FailClosed 臂既有「告警跳过，不阻断删除结果」不变）。
- 选型：**改 FailClosed 承接**（该函数对 FailClosed 显式宽宥为 warn+skip，删除准入已在前序步骤 fail-closed——原注释红线，不变）。
- 连带：臂内 `resolve_issue_logical_codebase_id` 调用点 `lc_id` 现为 `String`，`match lc_id.as_deref() { Some=>for_lc, None=>LogicalCodebaseStore::new }`（原 :239-242）收敛为 `LogicalCodebaseStore::for_lc(app_paths.clone(), &lc_id)`。

### 1.11 `src/web/workspace_context/builder.rs:136`（唯一构造点）

- 改前：`None => RepositoryRouting::Legacy { repository_id: String::new() },`（resolver `Ok(None)`＝无归属且无 legacy 别名 record 的旧数据，伪 Legacy 空 id 兼容分支）
- 改后：`None => RepositoryRouting::unbound_issue(),`（B3S1 新增的统一无归属 fail-closed 形态＝`FailClosed{TargetMissing,"issue_not_bound_to_logical_codebase:…"}`，B3S1 IRC 指定改法）
- 选型：**改 FailClosed**——`Ok(None)` 是「旧数据/无法解析」信号，落 `unbound_issue()` 稳定码 `repository_routing_target_missing`；`:141-144` 的 `match &routing`/`*code` 既有 FailClosed 处置保持（E0614 为本构造点类型错误级联，构造点修复后消解）。
- 连带：注释块去掉「回退 Legacy 兼容分支」措辞，改为 unbound_issue 两态口径。

### 1.12 `src/web/workspace_ws_handler/socket.rs:240`（`planning_resume_decision_with_fresh_index`）

- 改前：`RepositoryRouting::Legacy { .. } => Ok(None),`（单仓不受 resume 校验影响）
- 改后：**整臂删除**（Logical 臂校验指纹、FailClosed 臂 `planning_resume_routing_error` 既有）；doc 注释首条改「无归属旧数据 → routing 层即 FailClosed，拒绝恢复」。
- 选型：**改 FailClosed 承接**——「传统单仓不受影响」的静默通道随 Legacy 关闭。

## 2. Main 追加授权的两处处置

### 2.1 `builder.rs:143` `*code` E0614（Main 追加）

`routing_error_for_builder` 签名为 `code: RepositoryRoutingErrorCode`（按值）；`match &routing` 下 `code: &…`、`*code` 解引用拷贝本合法——E0614 系 `:136` Legacy 构造点（E0599）类型错误级联。`:136` 修复后无需改 `:143`（选 Main 给的 (a) 形态：保持按引用 match + `*code`），以编译自查 0 错误佐证。

### 2.2 死代码删除：`lifecycle_aggregate_scope.inc.rs:36` `reject_aggregate_scope_on_legacy_project`（Main 裁决「删」）

两个调用方（1.6/1.7 的 Legacy 臂）删除后无引用。**只删该函数**（原 :36-48）：文件其余 `validate_requested_aggregate_scope`/`validate_confirmed_story_specs`/`validate_confirmed_design_specs`/`confirm_gate_api_error`/`confirm_workspace_entity` 等均为活代码，文件保留；`lifecycle.rs:1030` 的 `include!` 挂载继续有效。未顺手改其它逻辑。

## 3. 行数门禁表

| 文件 | 改前 | 登记上限 | 改后 | 是否调整登记 |
|---|---|---|---|---|
| `src/web/handlers/coding.rs` | 1321 | 1321 | 1321 | 否（持平，门禁为 `>` 才红） |
| `src/web/handlers/coding/group.rs` | 818 | 未登记（<1200） | 734 | 否 |
| `src/web/handlers/lifecycle.rs` | 1041 | 未登记 | 1016 | 否 |
| `src/web/handlers/lifecycle/plan_preparation.rs` | 447 | 未登记 | 434 | 否 |
| `src/web/handlers/support_parts/coding_cleanup.inc.rs` | 1353 | 1572 | 1340 | 否 |
| `src/web/workspace_context/builder.rs` | 365 | 未登记 | 364 | 否 |
| `src/web/workspace_ws_handler/socket.rs` | 1893 | 1893 | 1892 | 否 |
| `src/web/handlers/support_task11_tests.rs`（授权追加） | 423 | 未登记（<1200） | 410 | 否 |
| `src/web/handlers/lifecycle_aggregate_scope.inc.rs`（授权追加） | 174 | 未登记 | 161 | 否 |

全部净减或持平，`large_file_guard.rs` 未动（且按 Main 指令登记数字由 Main 统一改）。

## 4. 验收 grep

```
$ grep -rn 'RepositoryRouting::Legacy' src/
（0 命中，exit=1）   # 全仓，不只 src/web
$ grep -rn 'logical_codebase_0001|logical_codebase_id: None' <我的 9 个文件>
（0 命中）           # 本片未引入任何填值字面量（Main 填值口径四类均不涉及）
```

## 5. 编译自查（窄过滤，仅一次）

- 10:15 统一全量日志（controller 跑，`/tmp/aria-full-lib-test.log`，10:17 死于 lib test 目标 28 错）：**本片 9 文件 0 错误 0 警告**（28 错均在他人片区：single_candidate、artifact_content、manager/mod.rs、entity.rs、workspace_choice、provider_installation、verification_surface、coding_ws_handler/runner、coordinator tests、p2_campaign、repository_resolution、aggregate_initialization 等）。
- 自排窄查（10:28 排队、312s，`cargo check --lib --profile test --message-format=short | grep -E 'coding_cleanup|lifecycle|socket|coding/group|handlers/coding\.rs|aggregate_scope|task11|workspace_context/builder|error|Finished|could not compile'`）：
  ```
  src/product/coding_workspace_engine/gates.rs:1424:3: error: this file contains an unclosed delimiter
  error: could not compile `cadence-aria` (lib test) due to 1 previous error
  ```
  **唯一错误在 product 侧 `gates.rs`**（LegacyFixProduct 的零余量文件，其在途编辑所致）——parse 级错误使全 crate 解析失败、任何文件都未进入类型检查；**本片 9 文件零诊断输出**。本片类型级证据以前一条（10:15 统一日志）+ 下一轮统一全量为准；已 IRC Main 该阻塞归属。

## 6. 收尾自证

```
$ ls -la src/web/handlers/coding.rs src/web/handlers/coding/group.rs src/web/handlers/lifecycle.rs src/web/handlers/lifecycle/plan_preparation.rs src/web/handlers/support_parts/coding_cleanup.inc.rs src/web/handlers/lifecycle_aggregate_scope.inc.rs src/web/workspace_context/builder.rs src/web/workspace_ws_handler/socket.rs src/web/handlers/support_task11_tests.rs
-rw-r--r-- 1 michaelche michaelche  53821 Nov  3 10:03 src/web/handlers/coding.rs
-rw-r--r-- 1 michaelche michaelche  31067 Nov  3 10:16 src/web/handlers/coding/group.rs
-rw-r--r-- 1 michaelche michaelche  42960 Nov  3 10:12 src/web/handlers/lifecycle.rs
-rw-r--r-- 1 michaelche michaelche  20725 Nov  3 10:16 src/web/handlers/lifecycle/plan_preparation.rs
-rw-r--r-- 1 michaelche michaelche  6230 Nov  3 10:23 src/web/handlers/lifecycle_aggregate_scope.inc.rs
-rw-r--r-- 1 michaelche michaelche  55589 Nov  3 10:12 src/web/handlers/support_parts/coding_cleanup.inc.rs
-rw-r--r-- 1 michaelche michaelche  16961 Nov  3 10:20 src/web/handlers/support_task11_tests.rs
-rw-r--r-- 1 michaelche michaelche  15344 Nov  3 10:03 src/web/workspace_context/builder.rs
-rw-r--r-- 1 michaelche michaelche  80804 Nov  3 10:12 src/web/workspace_ws_handler/socket.rs
$ wc -l（同上 9 文件）
  1321 / 734 / 1016 / 434 / 1340 / 161 / 364 / 1892 / 410
```

## 7. Concerns

1. **两处陈旧红**（仅存在于 10:15 那份修复前二进制日志，下轮全量应绿；已 IRC Main）：
   - `cleanup_shared_worktree_by_routing_keeps_legacy_cleanup_for_legacy_routing`：跑的旧二进制仍含该测试且必红——测试本体已删（见 1.9）。
   - `cleanup_shared_worktree_by_routing_enumerates_all_repo_worktrees_for_logical`：旧二进制的夹具无 issue 记录，两态下 `Err(NotFound)` 必红——夹具已按 (a) 补（见 1.9）。若下轮仍红即回滚排查。
2. `group_target_snapshots`/`resolve_group_repositories`（group.rs）保留 `Option<BTreeMap<…>>` 返回型（现恒 `Some`）——两者是 `#[allow(dead_code)]` 的 web 多值解析库存，最小机械改写不动签名；后续接线时再定形。
3. `attempt_target_snapshot`（coding.rs）、`group_target_snapshot`（group.rs）、`planning_resume_decision_with_fresh_index`（socket.rs）同理保留 `Option<…>` 返回型（`Ok(None)` 通道已不可达）——调用方按 Some 处理，无行为差异。
4. 工具缺陷报告：edit 工具对 `.worktrees/…` 绝对路径错误解析到主仓同名文件（read 正确），已写 `xd://report_issue`；本片以 `.worktrees/bugfix-b-11-02/…` 相对路径形式绕过完成全部编辑。
5. 本片无 RED 取证：改写期间 crate 处于编译阻塞态（Legacy 变体已删、全仓引用未清），且为纯机械改写——以「编译 0 错 + 现有测试不回归」为证据，未伪造 RED。
