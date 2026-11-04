# it_product 夹具迁移 · Phase 1（共享 helper + 样板 part_01）· v1.0

> 日期：2026-11-04
> 执行：MigrateItProductFixtures（worker）
> 范围：`tests/it_product/**` + `tests/it_product.rs`。**未改任何 `src/**` / `web/**` / 其他 `tests/it_*`**。

---

## 0. 一句话结论

块3 删除 `RepositoryRouting::Legacy` 之后，**「无 LC 归属的 attempt」一律 fail-closed**，
集成测试夹具必须补齐**三个**前置条件才能恢复原不变量：

| # | 前置条件 | 缺它时的报错 | 本阶段是否已解决 |
|---|---|---|---|
| ① | **LC 权威**（project + LC record + manifest + member + checkout + selection + policy + 聚合索引 + 归属 issue） | `target_snapshot_identity_drifted` / `product_store_not_found: issue issue_0001` | ✅ 本阶段 helper 已覆盖 |
| ② | **冻结 `target_snapshot`**（与权威同源） | `target_snapshot_missing_for_logical` / `coding_run_lease_unknown` | ✅ 本阶段 helper 已覆盖 |
| ③ | **LC provider gateway**（`LogicalCodebaseProviderGateway` + 注册 adapter） | `logical_provider_gateway_required` | ⚠️ 公共 API 足够组装，**但替身侧仍有缺口**（见 §4） |

此外发现**第四个、与夹具无关的产品事实**：块3 同时删掉了
`IssueSharedWorktreeRoute::Legacy` 臂 ⇒ **issue 维 shared worktree 整条路径下线**，
7 个断言 issue 维 worktree 的用例**在新模型下无路可走**（见 §3.3）。

---

## 1. 根因复述（已核对，未重新诊断）

`src/product/coding_attempt_store/admission.rs`：

```
旧：routing = RepositoryRouting::Legacy { repository_id }   // 无归属 ⇒ 仍按物理仓准入
新：routing = RepositoryRouting::unbound_issue()             // = FailClosed(TargetMissing)
```

`RepositoryRouting::load_for_issue` 现在**必须**读到成对的 (manifest, selection) 才返回
`Logical`；否则一律 `FailClosed`。而 `admit_attempt_for_execution_locked` 的分派是：

```rust
(FailClosed { .. }, _) => Err(Io("target_snapshot_identity_drifted")),
(Logical { .. }, None) => Err(Io("target_snapshot_missing_for_logical")),
(Logical { .. }, Some(snapshot)) => { validate_snapshot_fields(...); policy digest 比对 }
```

⇒ **夹具必须建出真实归属 + 冻结快照**，否则测到的是 fail-closed 而不是原不变量。

---

## 2. 交付物

### 2.1 新建：`tests/it_product/context_builder.rs`（727 行）

只用**公开 API**（`cadence_aria::product::logical_codebase::*` 等）重写
`src/product/logical_codebase/logical_test_fixture.rs`（后者是 `pub(crate)`，外部 crate 调不到）。
共享模式照 `tests/it_core/context_builder.rs` + `tests/it_core.rs:14-15` 的 `#[path]` + `mod`。

#### 对外暴露的函数签名

```rust
/// 夹具内唯一成员/物理仓 id
pub(crate) const FIXTURE_PHYSICAL_REPOSITORY_ID: &str = "repository_0001";
/// 夹具内聚合索引固定 id
pub(crate) const FIXTURE_AGGREGATE_INDEX_ID: &str = "aggregate_index_0001";

/// 一条 issue 的完整可准入现场
pub(crate) struct LogicalAuthority {
    pub(crate) project_id: String,
    pub(crate) issue_id: String,
    pub(crate) logical_codebase_id: String,   // 真多仓（非 legacy 别名）
    pub(crate) member_id: LogicalRepositoryId,
    pub(crate) checkout_id: RepositoryCheckoutId,
    pub(crate) member_path: PathBuf,
}

/// 建「project + 真多仓 LC 权威 + 归属该 LC 的 issue」
pub(crate) fn seed_logical_authority(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    root: &Path,
) -> LogicalAuthority;

/// 与权威一致的冻结快照（直接复用生产构造器 build_attempt_target_snapshot）
impl LogicalAuthority {
    pub(crate) fn snapshot(&self, paths: &ProductAppPaths) -> AttemptTargetSnapshot;
}

/// 把冻结 target_snapshot 补挂到「已落盘」的 attempt 记录上
/// （store 没有公开的补挂 API；按 seed_coding_attempt_running 同款 fixture 直写）
pub(crate) fn attach_target_snapshot(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
    snapshot: AttemptTargetSnapshot,
) -> CodingExecutionAttempt;

/// 组装逻辑 target 的 provider gateway（公开 API）
pub(crate) fn build_gateway(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    registry: Arc<ProviderRegistry>,
) -> LogicalCodebaseProviderGateway;

/// 把「只实现 legacy run_streaming」的替身桥接到 gateway 的 validated 启动路径
pub(crate) struct GatewayBridgedAdapter(pub Arc<dyn StreamingProviderAdapter>);
```

#### helper 建出的权威现场（形态）

```
{root}/                                  ← 夹具根（= 聚合根 aggregate_root）
  .aria/                                 ← ProductAppPaths 根
    projects/project_0001/
      projects.json                      ← ProjectStore
      issues/issue_0001/issue.json       ← IssueRecord（固定 id，见下）
      logical-codebases/{lc_id}/
        record.json                      ← LogicalCodebaseStore::create
        manifest.json                    ← provider_context_root = {root}
        members/{logical_id}.json
        checkouts/{checkout_id}.json     ← canonical_path = {root}/repository_0001
        selections/issue_0001.json
        aggregate-policy.json
        aggregate-indexes/aggregate_index_0001.json  ← active→degraded LKG
      repos.json                         ← 物理仓兼容投影（identity_schema_version=1）
  repository_0001/                       ← 成员 checkout（真实 git 仓，聚合根直接子目录）
```

**三条关键形态纪律**（踩过就红）：

1. **成员 checkout 必须是聚合根的直接子目录**，且**必须是真实 git 仓**
   （跨目标基线采样按真仓读 HEAD，裸目录 ⇒ `cross_target_store_failure`；
   不在聚合根之下 ⇒ `work_item_scope_domain_unresolved` fail-closed）。
2. **issue 记录按固定 id 直接落盘**，不走 `IssueStore::create`。
   原因：`IssueStore::create` 的 id 按 `issues/` 目录数分配，而 **coding attempt 落盘
   本身就会创建 `issues/{issue_id}/` 目录** ⇒ attempt 先建时再 create issue 会拿到
   `issue_0002`，与夹具里硬编码的 `issue_0001` 全部错位。
   ⇒ 固定 id 落盘让「先建 attempt 还是先建权威」都无所谓。
3. **manifest / policy / index 必须落在同一个 `for_lc(lc_id)` 子树**：
   用 `LogicalCodebaseStore::new`（project 级 legacy 根）会与 per-LC 子树**双份权威**
   ⇒ 触发 `repository_routing_legacy_conflict`。

#### 自检用例（helper 自身）

```rust
#[test]
fn logical_authority_snapshot_and_gateway_are_assemblable_from_public_api()
```
建权威 → 取快照（断言 member/checkout/membership_revision/policy_digest）→ 组装 gateway。
这是「helper 全程只用公开 API」的可执行证据。

### 2.2 改：`tests/it_product.rs`（+2 行）

```rust
#[path = "it_product/context_builder.rs"]
mod context_builder;
```

### 2.3 改：`tests/it_product/product_coding_workspace_engine/part_01.rs`（+100 / −15）

新增两个**本地**薄封装（其余 part 文件可直接复用）：

```rust
fn seed_lc_authority(paths: &ProductAppPaths, root: &Path) -> LogicalAuthority
fn create_input_with_snapshot(paths: &ProductAppPaths, authority: &LogicalAuthority)
    -> CreateCodingAttemptInput   // 复用共享 create_input()，挂上冻结快照
```

---

## 3. part_01 迁移逐用例清单

| # | 用例 | 夹具改动 | 状态 |
|---|---|---|---|
| 1 | `start_attempt_moves_to_worktree_prepare_and_creates_timeline_node` | 建 LC 权威 + `create_input_with_snapshot` | ✅ 绿 |
| 2 | `execute_worktree_prepare_creates_git_worktree_and_completes_timeline_node` | 同上 | ✅ 绿 |
| 3 | `execute_coding_runs_provider_in_worktree_and_streams_timeline_events` | LC 权威 + 快照 + author 改挂 ClaudeCode（gateway 不支持 Fake）+ 注册 gateway + `GatewayBridgedAdapter` | ✅ 绿 |
| 4 | `final_confirm_releases_issue_shared_worktree_lock` | LC 权威 + `attach_target_snapshot` | ❌ 仍红（§3.3） |
| 5 | `failed_attempt_releases_issue_shared_worktree_lock` | LC 权威 + `attach_target_snapshot` | ❌ 仍红（§3.3） |
| 6 | `dirty_shared_worktree_blocks_lock_release_and_next_work_item` | LC 权威 + `attach_target_snapshot` | ❌ 仍红（§3.3） |
| 7 | `failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner` | 组夹具 `running_group_engine_with_two_units_for_terminal_lock_tests` 建 LC 权威 + 快照 | ❌ 仍红（§3.3） |
| 8 | `aborted_group_attempt_releases_transferred_shared_worktree_lock_by_owner` | 同上 | ❌ 仍红（§3.3） |
| 9 | `dirty_group_delete_keeps_transferred_shared_worktree_lock` | 同上 | ❌ 仍红（§3.3） |
| 10 | `group_final_confirm_releases_transferred_shared_worktree_lock_by_owner` | （夹具在 part_12，本阶段未碰） | ❌ 仍红（Phase 2 的 part_12） |
| 11 | `coding_coder_run_resumes_previous_coder_provider_session` | LC 权威 + 快照 | ❌ 仍红（§3.2） |
| 12 | `coding_coder_rework_with_resume_uses_delta_prompt` | LC 权威 + 快照 | ❌ 仍红（§3.2） |
| 13 | `group_next_work_item_coder_run_does_not_resume_previous_unit_session` | LC 权威 + `attach_target_snapshot`（夹具在 part_12，attempt 先建） | ❌ 仍红（§3.2） |
| 14 | `deleted_group_attempt_releases_transferred_shared_worktree_lock_by_owner` | 同 #7 | ✅ 绿 |
| 15 | `role_permission_modes_are_persisted_with_role_provider_config` | 未动（不触 admission） | ✅ 绿 |
| 16 | `worktree_prepare_uses_issue_shared_worktree_path_for_issue_branch` | 未动（不触 admission） | ✅ 绿 |
| 17-19 | `single_work_item_{abort,failure,delete}_rejects_same_owner_lock_for_another_work_item` | 未动（断言即报错路径） | ✅ 绿 |

**未弱化任何断言**：全程零 `assert_*` 改动、零期望值改动、零测试名改动、零 `#[ignore]`。

### 3.1 「无归属 ⇒ fail-closed」这条不变量的**正面证据**

#1 / #2 两例现在走的是**真 admission**：
`resolve_for_issue` → (manifest, selection) 成对 ⇒ `Logical` ⇒
`validate_snapshot_fields`（member/checkout/canonical_path/git_dir_identity/membership_revision 逐字比对）
⇒ policy digest 比对 ⇒ 签票 ⇒ CAS 进 Running。
⇒ 夹具形态**对**了：只要权威与快照有任何一个字段漂移，这两例会立刻红。

### 3.2 剩余 provider-run 用例的缺口（§4 的 `into_parts`）

#11 / #12 / #13 的替身是 `SessionInputCapturingProvider`（part_09 定义），
它断言 `StreamingProviderInput` 的 **`permission_mode` / `timeout_secs` /
`resume_provider_session_id`**。validated 启动路径下替身只能拿到公开只读投影
`launch_view()`（**role / spawn cwd / prompt** 三件）⇒ 这三个字段拿不到 ⇒
**不能桥接**（it_web 的 `GatewayDialectStreamingStub` 手法只对「行为与完整
`StreamingProviderInput` 无关」的替身成立）。

⇒ 需产品侧二选一（见 §4 裁定项 A）。

### 3.3 issue 维 shared worktree 用例：**产品路径已下线**（与夹具无关）

块3 删掉了 `IssueSharedWorktreeRoute::Legacy` 臂（`git show fd0b9a8 --
src/product/coding_workspace_engine/gates.rs`）⇒ `IssueSharedWorktreeRoute`
现在**只剩 `Repository { repository_id }` 一个变体**，引擎**不再读写 issue 维
`issue-shared-worktree.json`**。

而这 7 个用例的夹具建的就是 issue 维记录，断言也读 issue 维记录：

```rust
let shared = lifecycle.get_issue_shared_worktree("project_0001", "issue_0001")
    .expect("load shared").expect("shared exists");
assert_eq!(shared.current_active_work_item_id, None);
```

带快照的 attempt 走仓维路径时，`preflight_repo_shared_worktree_absent`
（gates.rs）会看到旧 issue 维文件 ⇒ 直接 fail-closed
`LegacySharedWorktreePresent("project_0001/issue_0001")`；
不带快照则 `route_issue_shared_worktree` 报 `target_snapshot_missing_for_logical`。
⇒ **两条路都红，且不是夹具能修的**。

⇒ 需裁定（§4 裁定项 B）：改夹具维度 + 改读 API（等于改断言，被本阶段铁律禁止），
或按「已知红」登记，或产品侧恢复兼容臂。

---

## 4. 「不是 pub」的 API 清单（Phase 2 关键输入）

| # | API | 现状 | 影响 |
|---|---|---|---|
| **A1** | `cadence_aria::cross_cutting::session_launch::ValidatedStreamingProviderInput::into_parts` | **`pub(crate)`** | ★ **最关键**：外部集成测试**无法**实现 `StreamingProviderAdapter::start_validated` 的「把 validated 载体拆回 `StreamingProviderInput`」。⇒ 所有断言 `permission_mode` / `timeout_secs` / `resume_provider_session_id` 的替身**无法**接入 validated 路径。it_web 已确认同一结论（`tests/it_web/web_lc_bootstrap_repair.rs:293`：「库外（it_web）无法拆 `ValidatedStreamingProviderInput`（`into_parts` 为 pub(crate)）」）。<br>**裁定建议**：把 `into_parts` 提为 `pub`（或加一个公开的 `ValidatedPassthroughAdapter`），即可解锁 it_product 的 provider-run 全族。 |
| A2 | `crate::product::logical_codebase::store::legacy_logical_codebase_id` | `pub(crate) use` | 外部测试拿不到「legacy 别名 LC id」。本阶段用**真多仓 LC**（`LogicalCodebaseStore::create` 返回的 `record.id`）规避 ⇒ **不阻塞**，但「必须用 legacy 布局」的用例需要它。 |
| A3 | `crate::product::logical_codebase::store::lc_scope_root` | `pub(crate)` | 外部测试拿不到「某 lc_id 的权威根」。本阶段用 `for_lc(lc_id)` store 反推 ⇒ **不阻塞**。 |
| A4 | `crate::product::logical_codebase::logical_test_fixture::*`（`seed_logical_authority` / `snapshot_for` / `seed_complete` / `ensure_planning_prerequisites` / `upsert_repo_worktree` / `bind_repo_lock_to_attempt`） | 整个模块 `pub(crate) mod` | 这就是本阶段新建 `tests/it_product/context_builder.rs` 的原因（公开 API 重写等价物）。**已解决**。 |
| A5 | `crate::product::coding_attempt_store::write_coding_attempt_for_test` | `#[cfg(test)]` | 外部测试不能用 ⇒ 补挂快照改走 `json_store::write_json` 直写（`attach_target_snapshot`）。**已绕过**。 |
| A6 | `crate::product::coding_attempt_store::seed_running_attempt_for_test` | `#[cfg(test)]` | 同上 ⇒ 复用 `tests/it_product.rs` 里既有的 `seed_coding_attempt_running`（公开 JSON 直写）。**已绕过**。 |
| A7 | `crate::product::coding_workspace_engine::tests::group_terminal::bind_group_drafts_to_snapshot_target` | `#[cfg(test)]`（`pub(super)`） | group attempt 的 draft 归属补齐（`validate_group_single_target` 对 Logical 路由要求每个权威 unit 带 `target_repository_id`）在外部**调不到**。本阶段用手写 `WorkItemPlanStore::put_draft_record`（公开 API）可等价实现 ⇒ **可绕过**，但 Phase 2 的 group 用例需要。 |
| A8 | `crate::product::logical_codebase::provider_gateway::PROVIDER_ROOT_RECIPE_REQUIRES_FIXED_CLAUDE` | `pub` ✓ | 无缺口（helper 已用）。 |

**A1 是唯一的硬阻塞**（A2/A3/A5/A6/A7 都有公开 API 等价物）。

---

## 5. Phase 2 操作模板（逐条可执行）

> 目标文件：`tests/it_product/product_coding_workspace_engine/part_XX.rs`（其余 18 个）。

### 步骤 0：确认改动边界

只改 `tests/it_product/**`；不改 `src/**`、不改其他 `tests/it_*`。
每个 part 文件里 `create_input()` / `create_group_attempt` 等共享 helper **可能在别的 part 文件**
（如 part_08 / part_09 / part_12）⇒ **不要改它们**，改用步骤 2 的 `attach_target_snapshot` 补挂。

### 步骤 1：建 LC 权威（每个用例一次）

```rust
let root = tempdir().expect("root");
let paths = ProductAppPaths::new(root.path().join(".aria"));
let authority = crate::context_builder::seed_logical_authority(&paths, "project_0001", "issue_0001", root.path());
```

> ⚠️ `paths` 必须是该用例真实在用的 `ProductAppPaths`；`root` 必须是该 `.aria` 的父目录。
> ⚠️ 若用例的夹具在别的 part 文件里创建了 `paths`（拿不到 `root`），见步骤 5。

### 步骤 2：给 attempt 挂冻结快照

**2a. attempt 由本文件创建**（最常见）：

```rust
let attempt = store.create_attempt(CreateCodingAttemptInput {
    ..create_input_with_snapshot(&paths, &authority)   // 复用共享 create_input()，只加快照
}).expect("create attempt");
```

**2b. attempt 由别的 part 文件的共享 helper 创建**（`coding_store_with_attempt` /
`failed_attempt` / `dirty_failed_attempt` / `final_confirm_attempt` / `group_engine_with_*`）：

```rust
let (store, attempt) = failed_attempt(paths.clone(), "work_item_0001");
let attempt = crate::context_builder::attach_target_snapshot(&store, &attempt, authority.snapshot(&paths));
```

**2c. group attempt**（`CreateGroupCodingAttemptInput`）：

```rust
target_snapshot: Some(authority.snapshot(&paths)),
```

> ⚠️ group attempt 另有 **draft 归属**前置：`validate_group_single_target` 对 Logical 路由
> 要求每个权威 unit 的 `target_repository_id` 与 attempt 快照一致 ⇒ 需按 A7 用
> `WorkItemPlanStore::put_draft_record` 把 plan 的 draft 补上 `candidate.target_repository_id`。

### 步骤 3：provider run 用例（要跑 provider 的才做）

```rust
// 3.1 gateway 不支持 Fake ⇒ role provider 改挂 ClaudeCode
let attempt = store.create_attempt(CreateCodingAttemptInput {
    provider_config_snapshot: ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: Some(ProviderName::ClaudeCode),
        review_rounds: 1,
        permission_modes: Default::default(),
    },
    ..create_input_with_snapshot(&paths, &authority)
}).expect("create attempt");

// 3.2 替身同时交给引擎方法与 gateway registry（两处同一实例）
let provider = Arc::new(crate::context_builder::GatewayBridgedAdapter(Arc::new(SomeAdapter)));
let mut registry = ProviderRegistry::new();
registry.register(ProviderName::ClaudeCode, provider.clone());
let gateway = crate::context_builder::build_gateway(
    &paths, "project_0001", &authority.logical_codebase_id, Arc::new(registry));

// 3.3 引擎注入 gateway
let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx)
    .with_logical_provider_gateway(Arc::new(gateway));

engine.execute_coding(&attempt, provider.as_ref(), &context).await.expect("execute coding");
```

> ⚠️ `GatewayBridgedAdapter` **只对「行为与完整 `StreamingProviderInput` 无关」的替身成立**
> （`launch_view()` 只给 role / working_dir / prompt）。断言
> `permission_mode` / `timeout_secs` / `resume_provider_session_id` 的替身**桥接不了**
> ⇒ 需 A1 裁定后由产品侧提供拆分能力。

### 步骤 4：worktree 类用例

- **仓维（LC 模型）**：夹具建 `shared-worktrees/{logical_id}.json`（`upsert_repo_shared_worktree`），
  断言也读 `get_repo_shared_worktree`。
- **issue 维（legacy 模型）**：块3 已下线 ⇒ 见 §3.3，先向 Main 取裁定，**不要自行改断言**。

### 步骤 5：夹具在别的 part 文件里、拿不到 `root` 时

`group_engine_with_last_running_unit()` 等返回 `(TempDir, ProductAppPaths, ...)` ⇒
用返回的 `root.path()` 当聚合根：

```rust
let (root, paths, store, _engine, attempt) = group_engine_with_last_running_unit();
let authority = crate::context_builder::seed_logical_authority(&paths, "project_0001", "issue_0001", root.path());
```

### 步骤 6：自检

```bash
flock /tmp/aria-cargo.lock cargo test --locked --test it_product -- product_coding_workspace_engine
flock /tmp/aria-cargo.lock cargo build --locked --tests
```

---

## 6. 验证输出

> 阻塞已解除（并行 worker `FixBootstrapAdvanceStall2` 补齐了
> `src/web/handlers/aggregate_initialization/handlers.inc.rs` 的 `detail` 字段，
> lib 恢复 exit 0）。本阶段**全程未改任何 src/**。

### 6.1 全量 it_product（迁移后）

```bash
flock /tmp/aria-cargo.lock cargo test --locked --test it_product
```

```
test result: FAILED. 153 passed; 58 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.98s
```

**迁移前基线**（本阶段开工时）：`148 passed; 62 failed`
⇒ **净 +5 绿 / −4 红**（另新增 1 个 helper 自检用例 ⇒ 总用例数 210 → 211）。

### 6.2 module 级

```bash
flock /tmp/aria-cargo.lock cargo test --locked --test it_product -- product_coding_workspace_engine::
```

| | 迁移前 | 迁移后 |
|---|---|---|
| passed | 11 | **15** |
| failed | 62 | **58** |

### 6.3 part_01 自身用例状态（19 例）

| 状态 | 数量 | 用例 |
|---|---|---|
| ✅ 绿 | **4**（+3 由本阶段转绿，1 原就绿） | `start_attempt_moves_to_worktree_prepare_and_creates_timeline_node` ★新绿<br>`execute_worktree_prepare_creates_git_worktree_and_completes_timeline_node` ★新绿<br>`execute_coding_runs_provider_in_worktree_and_streams_timeline_events` ★新绿（gateway 桥接）<br>`deleted_group_attempt_releases_transferred_shared_worktree_lock_by_owner` ★新绿 |
| ✅ 绿（原就绿，未动） | 4 | `role_permission_modes_are_persisted_with_role_provider_config`、`worktree_prepare_uses_issue_shared_worktree_path_for_issue_branch`、`single_work_item_abort_rejects_same_owner_lock_for_another_work_item`、`single_work_item_failure_rejects_same_owner_lock_for_another_work_item`、`single_work_item_delete_rejects_same_owner_lock_for_another_work_item` |
| ❌ 仍红 | 11 | 见 §3.2 / §3.3（**10 个是产品路径缺口，非夹具问题**；1 个夹具在 part_12） |

**part_01 下已无任何用例因 `target_snapshot_identity_drifted` / `coding_run_lease_unknown` 失败**
（这两类签名在本阶段已全部消除）。

### 6.4 剩余 58 个失败用例名（Phase 2 的迁移清单）

全部落在 `product_coding_workspace_engine::` 模块（part_03/04/05/06/08/09/12/13/17/18/19/20）：

```
aborted_group_attempt_releases_transferred_shared_worktree_lock_by_owner      ← part_01（§3.3）
ambiguous_push_with_initial_old_ref_converges_after_remote_updates_later
code_review_provider_start_failure_marks_attempt_blocked_and_node_failed
coding_code_reviewer_run_records_session_without_resume
coding_coder_rework_with_resume_uses_delta_prompt                            ← part_01（§3.2）
coding_coder_run_resumes_previous_coder_provider_session                      ← part_01（§3.2）
coding_internal_reviewer_uses_fresh_provider_session
coding_prompt_includes_rework_fix_hints
completing_group_unit_marks_current_unit_completed_and_next_running
completing_group_unit_moves_issue_shared_lock_to_next_unit
completing_group_units_publishes_revisions_without_legacy_handoff_artifacts
completing_last_group_unit_enters_review_request_stage
dirty_group_delete_keeps_transferred_shared_worktree_lock                     ← part_01（§3.3）
dirty_shared_worktree_blocks_lock_release_and_next_work_item                  ← part_01（§3.3）
execute_code_review_blocked_creates_retry_gate
execute_code_review_forwards_provider_execution_events
execute_code_review_persists_provider_control_and_tool_event_payloads
execute_code_review_persists_report_and_emits_review_events
execute_code_review_persists_role_run_events_while_forwarding_realtime_events
execute_code_review_prompt_includes_diff_work_item_rules_and_role_provider
execute_code_review_records_permission_timeout_as_timeout_event
execute_coding_blocks_later_permission_before_choice_response
execute_coding_blocks_when_provider_completes_before_choice_response
execute_coding_emits_prompt_for_coder_provider
execute_coding_forwards_permission_responses_to_provider_session
execute_coding_forwards_provider_execution_and_tool_events
execute_coding_forwards_provider_permission_choice_and_status_events
execute_coding_includes_unconsumed_context_notes_and_consumes_them
execute_coding_includes_work_item_context_in_provider_prompt
execute_coding_persists_coder_role_run_and_full_output_chat_entry
execute_coding_persists_provider_choice_and_resumes_after_response
execute_coding_stops_forwarding_provider_events_after_abort_command
execute_group_final_review_blocked_opens_human_gate
execute_group_final_review_persists_review_and_waits_for_final_confirm
execute_group_final_review_prompt_includes_request_commit_diff_and_function_context
execute_internal_pr_review_forwards_provider_execution_events
execute_review_request_keeps_push_started_when_failed_push_cannot_query_remote
failed_attempt_releases_issue_shared_worktree_lock                            ← part_01（§3.3）
failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner       ← part_01（§3.3）
final_confirm_owner_conflict_does_not_complete_attempt
final_confirm_releases_issue_shared_worktree_lock                             ← part_01（§3.3）
group_completion_gate_allows_changed_files_within_exclusive_scope_from_git
group_completion_gate_fails_closed_when_worktree_is_missing
group_completion_gate_rejects_changed_files_in_forbidden_scope_from_git
group_final_confirm_completes_attempt_after_all_units_completed
group_final_confirm_releases_transferred_shared_worktree_lock_by_owner        ← part_12 夹具
group_next_work_item_coder_run_does_not_resume_previous_unit_session          ← part_01（§3.2）
group_unit_owner_conflict_does_not_advance_unit_or_attempt
handle_abort_marks_attempt_aborted_and_closes_active_timeline_node
handle_final_confirm_completes_when_required_plan_is_satisfied
nonzero_push_with_authoritatively_updated_remote_records_pushed_review_request
parses_real_provider_review_finding_aliases
pi_failure_does_not_trigger_fresh_retry
pi_role_with_supervised_mode_normalized_to_auto
rejected_push_with_verified_missing_remote_ref_records_failed_review_request
retry_code_review_prompt_includes_previous_role_run_diagnostic
review_payload_parse_failure_records_blocked_reviewer_evidence
review_request_blocks_when_only_runtime_artifacts_changed
```

**按阻塞原因归类（Phase 2 派工依据）**：

| 类别 | 数量 | 阻塞原因 | Phase 2 可否自解 |
|---|---|---|---|
| **P1 provider-run 族** | ~28 | 替身未实现 `start_validated`（A1：`into_parts` 是 `pub(crate)`） | ❌ 需产品侧先解 A1 |
| **P2 issue 维 shared worktree 族** | ~8 | 块3 删了 `IssueSharedWorktreeRoute::Legacy` 臂（§3.3） | ❌ 需裁定（改断言/已知红/恢复兼容臂） |
| **P3 LC 权威 + 快照即可解** | ~22 | 其余（group 单元推进/完成门禁、code review 族、push 族、final confirm 族等） | ✅ 按 §5 模板逐个迁 |

> P3 才是 Phase 2 主体（**22 个可自解**）；P1/P2 共 ~36 个**不是夹具能修的**，
> 建议 Main 先就 A1 与 §3.3 两条做产品侧裁定，再决定 P1/P2 的归属。

### 6.5 编译零警告

```bash
flock /tmp/aria-cargo.lock cargo build --locked --tests
```

⇒ **0 warning / 0 error**（`grep -E "^warning|^error"` 无输出）。

### 6.6 cargo fmt --check

```bash
flock /tmp/aria-cargo.lock cargo fmt --check
```

⇒ 本阶段改动的文件**全部干净**（`tests/it_product/context_builder.rs`、
`tests/it_product.rs`）。
唯一残留 diff 在 **`src/web/types/logical_codebase_bootstrap.rs:98`**（并行 worker 的在途文件，
非本阶段产物，**未代改**）。

> 注：`tests/it_product/product_coding_workspace_engine/part_01.rs` 是 `include!` 片段，
> `cargo fmt` **不覆盖**它（只覆盖 `#[path] mod` 可达的 `context_builder.rs` / `it_product.rs`）；
> 故未对它单独跑 rustfmt（避免整文件去缩进的噪声）。

---

## 7. 被改文件行数

| 文件 | 变化 |
|---|---|
| `tests/it_product/context_builder.rs` | **新建，727 行** |
| `tests/it_product.rs` | **+2 / −0**（`#[path]` + `mod` 声明） |
| `tests/it_product/product_coding_workspace_engine/part_01.rs` | **+100 / −15** |
| 合计 | 新建 727 + 改 102 − 15 |

`src/**`、`web/**`、其他 `tests/it_*`、`openspec/**`、`cadence/**`（除本报告）、`scripts/**`、
`Cargo.toml`、`Cargo.lock`：**零改动**。

---

## 8. Concerns

1. **helper 的公共 API 面**：模块只挂 `#[path] mod context_builder;` 于 `tests/it_product.rs`，
   作用域限于 it_product 集成测试 crate，**不污染产品 crate 的公共 API**。
   模块内 `#![allow(dead_code)]`（Phase 2 才会用到全部导出项）。
2. **`#![allow(dead_code)]` 的取舍**：本阶段只迁了 part_01，helper 的部分导出项
   （`build_gateway` / `GatewayBridgedAdapter` / `attach_target_snapshot`）在 part_01 里
   引用面不全 ⇒ 用模块级 allow 避免 Phase 2 铺开前的死代码告警。Phase 2 全铺开后
   可去掉该 allow 并逐项确认无 dead code。
3. **A1 是硬阻塞**（§4）：`ValidatedStreamingProviderInput::into_parts` 是 `pub(crate)`
   ⇒ it_product 的 provider-run 全族（本次 62 红里的绝大多数）**不是夹具问题**，
   需要产品侧二选一：把 `into_parts` 提为 `pub`，或加一个公开的
   `ValidatedPassthroughAdapter`。
   **在此之前，Phase 2 迁 provider-run 用例会得到
   `logical_provider_gateway_required` 或
   `streaming provider validated start is not supported by this adapter`。**
4. **§3.3 的 7 个 issue 维 worktree 用例**：断言的是块3 已下线的产品路径。
   本阶段**未改断言**（铁律），按「补了前置条件后仍红 ⇒ 记下来报告」处理。
   需要 Main 裁定：(a) 改夹具维度 + 改读 API；(b) 按已知红登记；(c) 产品侧恢复兼容臂。
5. **issue 记录固定 id 直写**：helper 不走 `IssueStore::create`，直接写
   `issues/issue_0001/issue.json`。这是为了消除「attempt 先建 ⇒ issue id 漂到
   issue_0002」的顺序陷阱。副作用：绕过了 `IssueStore::create` 的
   `change_id` slug 化（固定写成 `change_issue_0001`）——与夹具语义无关。
6. **并发冲突**：本阶段工作期间有并行 worker 在改 `src/product/logical_codebase/bootstrap_*.inc.rs`
   （mtime 18:24~18:26），一度把 lib 编译打坏。本阶段**未触碰任何 mtime 在 20 分钟内的文件**，
   也**未代补 src/**（已获 Main 明确授权「不要代补」）。
