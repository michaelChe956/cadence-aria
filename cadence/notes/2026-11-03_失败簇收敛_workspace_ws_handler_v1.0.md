# 失败簇收敛：`web::workspace_ws_handler::tests`（61 条 → 本报告记录态）

- worktree：`.worktrees/bugfix-b-11-02`
- 根因：本轮「单仓入口删除（REQ-ROUTE-01 两态化）」后，`RepositoryRouting::Legacy`
  形态与「无归属 ⇒ Ok(None) 静默放行」路径被删除；web 侧夹具普遍缺
  「LC 权威全链 / issue 记录 / 聚合 index+policy / 成员 language rules」前置，
  路由两态解析在 provider 启动/上下文构建前 fail-closed。
- 原则：只改夹具；断言零弱化（两条已作废语义的断言按 Main 口径迁移并留痕）。

## 逐簇收敛表

| 簇 | 用例数 | 改了哪些夹具 | 窄跑后剩余 |
|---|---|---|---|
| `single_candidate_provider_run::*`（含 `reredrive_and_reopen.rs` 16、`ears_reredrive.rs` 等） | 20 | `ProviderRunFixture::build_routed`：①`repository_root` 统一落 `root/aggregate-root/` 之下（聚合索引布局）；②`FixtureRoute::Legacy` 也经 `register_alias_logical_codebase_with_extras` 登记权威全链 + repos.json 投影升级（复用 `single_candidate_lc_admission`，不另起第二套），**不注入 gateway**（`inject_gateway=false`，保持 legacy 直连 provider） | 见「窄跑结果」节 |
| `campaign_stage3_interactive::*` | 11 | `campaign_stage3_interactive/harness.rs`：`campaign_stage3_fixture` 在基座权威之上补 `ensure_planning_index_and_policy`（active→Degraded 聚合索引 + policy bootstrap + 成员 `.claude/rules/language.md`，不重写权威与投影） | 同上 |
| `campaign_stage3_recovery_matrix::*` | 7 | 同上（矩阵用例全部复用 `campaign_stage3_fixture` / `workspace_human_action_http_fixture`） | 同上 |
| `planning_resume::*` | 5 | ①`PlanningResumeFixture::write_logical_codebase` 头部补 `seed_attributed_issue`（幂等，先于 selection 落盘）；②`planning_resume_fail_closed_when_manifest_without_selection` 的 12 行手写 manifest 块收敛为 `seed_unpaired_manifest_with_attributed_issue`（净 −8 行）；③两条「无归属 ⇒ None」断言按 Main 口径迁移（见下节） | 同上 |
| `provider_run_events::*` | 5 | 未直接改（1257/1257 零余量）；5 条全部经 `ProviderRunFixture::new/new_with_engine_rx`（Legacy 路由）⇒ 主夹具修复覆盖 | 同上 |
| `single_candidate_ir_reredrive` / `single_candidate_structure_reredrive` | 3+3 | 未直接改；全部 `ProviderRunFixture::new`（Legacy）⇒ 主夹具修复覆盖 | 同上 |
| `single_candidate_scope_rejection` | 2 | `scope_test_context`：补 project + repository_0001（成员 checkout 落 `aggregate-root/member`）+ 别名归属 issue + `register_legacy_direct_authority`（SC 入站链先经 routing/上下文解析） | 同上 |
| `plan_repair_activation::*` | 2 | 新增 `seed_lc_authority_for_plan_repair`：经 `register_alias_authority_without_projection`（登记权威但**不**重写 repos.json 投影，保持 coding-runtime legacy 直连）+ `issue_plan_0001` 的 selection 补写 | 同上 |
| `provider_select_refreshes…` / `provider_select_then_user_message…` / `start_generation_refreshes…`（tests.rs） | 3 | 三个内联夹具：repo tempdir 迁至 `root/aggregate-root/` 下 + issue 创建后补 `register_legacy_direct_authority`（不注入 gateway） | 同上 |
| `c1_recovery_surface::c1_a07…` | 1 | A07 链复用 `workspace_human_action_http_fixture` ⇒ campaign harness 修复覆盖 | 同上 |
| `gateway_start::logical_plan_validate_failure_is_reported_by_handler` | 1 | 夹具不再删 manifest（「keep routing legacy」已无对应形态）；改为保留 manifest + 补空成员 selection（anchor 守卫 (manifest,selection) 成对），仍只删 policy artifact 钉 validate-failure 早退 | 同上 |

## 迁移断言（新旧并排，Main 口径留痕）

### 1. `planning_resume_decision_is_none_for_legacy_single_repo_path` → `planning_resume_without_lc_authority_fails_closed`

旧（语义已作废：无归属 ⇒ Ok(None) 静默放行）：

```rust
let decision =
    planning_resume_decision_with_fresh_index(&fixture.paths, "project_0001", "issue_0001")
        .await
        .unwrap();
assert!(decision.is_none());
```

新（更具体：无 LC 权威一律 fail-closed，断言稳定码 + 原因串）：

```rust
let fixture = PlanningResumeFixture::new();
super::lc_fixture_support::seed_attributed_issue(&fixture.paths);
let error =
    planning_resume_decision_with_fresh_index(&fixture.paths, "project_0001", "issue_0001")
        .await
        .unwrap_err();
assert!(error.contains("repository_routing_target_missing"), "{error}");
assert!(error.contains("issue_not_bound_to_logical_codebase"), "{error}");
```

### 2. `planning_resume_legacy_when_none_none` → `planning_resume_bare_workspace_without_issue_fails_closed`

旧：

```rust
let result = planning_resume_decision_with_fresh_index(
    &ProductAppPaths::new(root.path().join(".aria")),
    "project_0001",
    "issue_0001",
)
.await;
assert!(result.is_ok());
assert!(result.unwrap().is_none());
```

新（更具体：裸现场 issue 缺失即 fail-closed，断言错误面前缀 + NotFound 串）：

```rust
let error = planning_resume_decision_with_fresh_index(
    &ProductAppPaths::new(root.path().join(".aria")),
    "project_0001",
    "issue_0001",
)
.await
.unwrap_err();
assert!(error.contains("load repository routing failed"), "{error}");
assert!(error.contains("product_store_not_found: issue issue_0001"), "{error}");
```

`planning_resume_fail_closed_when_manifest_without_selection` 断言本就 fail-closed ⇒ 原样保留（仅夹具前置改为共享 helper）。

## 新增本地夹具模块（web 测试树内，非 product 级共享夹具）

`src/web/workspace_ws_handler/tests/lc_fixture_support.rs`（新文件，326 行，不在行数门禁表内）：

- `stub_gateway_author()`：占位 StreamingProviderAdapter（Legacy 登记时必填的
  gateway_author；不注入 engine ⇒ 永不调用，被调即显式报错）。
- `register_alias_authority_and_upgrade_projection(..)`：复用
  `single_candidate_lc_admission::register_alias_logical_codebase_with_extras` +
  repos.json 三层身份投影升级（ProviderRunFixture 两个路由臂共用）。
- `register_alias_authority_without_projection(.., issue_id)`：同源登记但不重写
  repos.json（plan_repair 等 coding-runtime 夹具保持 legacy 直连解析），
  `issue_id ≠ issue_0001` 时补写该 issue 的 selection。
- `register_legacy_direct_authority(..)`：单成员便捷入口（tests.rs 内联夹具用）。
- `ensure_planning_index_and_policy(..)`：既有权威之上补 active(→Degraded)
  聚合索引 + policy bootstrap + 成员 language rules（campaign 基座用；
  不重写权威与投影）。
- `seed_attributed_issue(..)`（幂等）/ `seed_unpaired_manifest_with_attributed_issue(..)`。

## 共享夹具（product 级）是否需扩能力

**需要（报告，未自行修改 `logical_test_fixture.rs`）**：

1. `seed_logical_authority` / `seed_complete` 均不建 **aggregate index**，也不写
   别名位 `record.json`（其布局经 `ProjectStore::get` 懒迁移才落 record）。
   凡走「workspace context / manager create / 计划聚合视野」链路的夹具，
   只用 product 级共享夹具会在 `PlanningContextResolver::build_with_fresh_index
   → index_store.active_required` 处失败（`aggregate_index_unavailable`）。
   ⇒ 建议给共享夹具补「聚合索引（create→activate→Degraded）+ policy bootstrap
   + 别名 record」能力；本轮 web 侧按指示复用了
   `register_alias_logical_codebase_with_extras`（其能力完整）。
2. `seed_logical_authority` 绑定 `repository_0001` 固定物理 id 且 `seed_complete`
   自建 project/issue，无法服务于「issue id ≠ issue_0001」（如 plan_repair 的
   `issue_plan_0001`）或「已存在 project/repository 的现场」——本轮用
   `register_alias_authority_without_projection(issue_id)` 变体在 web 本地模块补位。
3. 锁维度 API（`upsert_repo_worktree` + `bind_repo_lock_to_attempt`）本簇未用到
   （本簇无 target_snapshot attempt 夹具），无需扩。

## 仍未解项 / 待验证项

- 第一次全量窄跑（编辑批次 1）：61 → **21**（208 绿）。批次 2（campaign 成员
  language rules、plan_repair 权威补齐、scope_rejection 前置、planning_resume
  断言迁移、gateway_start selection 补写）后的最终数字见 IRC/下表。
- 行数门禁实测（本报告时点）：
  - `single_candidate_provider_run.rs` 1493 / 登记 1508 ✓
  - `planning_resume.rs` 2449 / 登记 2449（恰好持平，零余量）
  - `provider_run_events.rs` 1257、`single_candidate_lc_admission.rs` 2638、
    `campaign_stage3_interactive/cases.rs` 1209 —— 未改动，持平
  - `tests.rs` 1176、`lc_fixture_support.rs` 326（新增）、
    `plan_repair_activation.rs` 298、`single_candidate_scope_rejection.rs` 293、
    `gateway_start_parts/author_launch_cases.inc.rs` 614 —— 均低于 1200 默认线
- mtime 避让：编辑窗口内发现 `single_candidate_provider_run.rs` 被并行同胞修改
  （编译阻塞修复：`Option::filter` → `if inject_gateway { Some(..) }`），已保留
  其修复并继续；无其他冲突。
- 佐证（只改夹具、断言与实现未动）：
  `git diff --stat`（本簇文件，含本轮先前批次的改动）：
  `tests.rs +52`、`harness.rs +6`、`author_launch_cases.inc.rs +26/−`、
  `planning_resume.rs +21/−`、`single_candidate_provider_run.rs 121 变更`、
  `single_candidate_scope_rejection.rs +33`、`lc_fixture_support.rs`（新增）。
  实现目录（`src/product/**`、`src/web/workspace_context/**` 等）零改动。

## planning_resume.rs 行数压缩核对（零余量文件）

- 压缩前：2455 行（超登记 2449 共 6 行）；压缩后：**2449 行**（恰好持平）。
- 压缩点仅两处，均为**排版等价**：
  1. `planning_resume_without_lc_authority_fails_closed` 的多行
     `assert!(\n error.contains("issue_not_bound_to_logical_codebase"),\n "{error}"\n)`
     收成单行 `assert!(error.contains("issue_not_bound_to_logical_codebase"), "{error}");`
     —— 谓词与消息字面量**逐字未动**，只删换行/缩进。
  2. `planning_resume_bare_workspace_without_issue_fails_closed` 的复合断言
     `assert!(A && B, "{error}")` 拆为两条 `assert!(A, "{error}"); assert!(B, "{error}");`
     —— 布尔强度等价（两条件仍都必须成立，各自失败时消息更精确），
     字符串字面量（`"load repository routing failed"`、
     `"product_store_not_found: issue issue_0001"`、`"{error}"`）**一字未改**。
- 其余断言（含 `planning_resume_fail_closed_when_manifest_without_selection` 的
  两条原断言）零改动。

---

## 统一跑最终数（Main 的 lib 全量日志 `/tmp/aria-full-lib-test.log`，2026-11-03 17:48）

**61 → 10**（`grep -E '^test web::workspace_ws_handler.*FAILED'` = 10）。

### 剩余 10 条逐条归因（panic 原文取自统一日志）

| # | 用例 | panic 原文 / 归因 | 归属 |
|---|---|---|---|
| 1 | `c1_recovery_surface::c1_a07_a09_a12_a13_recovery_surface` | `assertion left == right failed: body: {"code":"single_candidate_approval_compile_failed",…}` | **product 层共享基座**（approval 链编译失败面） |
| 2 | `campaign_stage3_interactive::workspace_human_action_http_feedback_replay_and_approve_without_driver` | HTTP **422**（同一 approval 编译失败面） | 同上 |
| 3 | `campaign_stage3_recovery_matrix::campaign_stage3_recovery_matrix_amendment_application_delivery_recovers_same_event` | panic 在 `src/product/coding_workspace_engine/tests/campaign_stage3_amendment.rs:165`「real approval chain must confirm: single-candidate approval compile failed」 | **product 层共享基座** |
| 4 | `…_amendment_binding_window_resumes_from_journal_prefix` | 同上 | 同上 |
| 5 | `…_amendment_context_window_reconnect_resumes_same_command` | 同上 | 同上 |
| 6 | `…_amendment_publication_checkpoints_resume_same_command` | 同上 | 同上 |
| 7 | `campaign_stage3_recovery_matrix::campaign_stage3_recovery_matrix_candidate_row_orphaned_snapshot_recovery` | body `single_candidate_approval_compile_failed` | 同上 |
| 8 | `gateway_start::logical_plan_validate_failure_is_reported_by_handler` | `assertion failed: matches!(message, WsOutMessage::Error { ref message } if message.starts_with("logical plan launch failed:"))` | web 侧夹具（validate 失败面被更早的 fail-closed 抢先） |
| 9 | `plan_repair_activation::repeated_confirmation_recovers_delivery_mark_failure_without_duplicate_application` | `save manifest: IdentityMismatch { kind: "logical_codebase_manifest", id: "project_0001" }` | **本轮已修**（见下） |
| 10 | `plan_repair_activation::zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery` | 同上 | **本轮已修**（见下） |

### 本轮追加修复：#9 / #10（`plan_repair_activation`，2 条）✅ 已改，待统一跑复验

根因：`plan_repair_activation.rs::seed_lc_authority_for_plan_repair` **重复登记** LC 权威 ——
基座 `src/web/test_controls/plan_repair/seed.rs:151` **已经**落了 manifest + member +
主 checkout + selection + policy + repos.json 逻辑归属回填；夹具又调别名登记器
⇒ 新 manifest 的 `logical_codebase_id` 与基座的不一致 ⇒
`save_manifest` 返回 `IdentityMismatch { kind: "logical_codebase_manifest" }`。

改法（**只改夹具，不动断言**）：**不重写权威**，改为补基座真正缺的两段：
1. `ProjectStore::get("project_0001")` —— 幂等触发 `migrate_legacy`，把项目级 legacy
   布局落成别名位 `record.json`（authority resolver 的 LC 存在性凭证）；
2. `ensure_planning_index_and_policy(&app_paths, "project_0001")` —— 补
   active(→Degraded) 聚合索引（planning context 链路的 `active_required` 前置）。

### 仍未解项（1-8）

**#1-#7（7 条）的根因在 product 层共享基座夹具**
（`src/product/coding_workspace_engine/tests/campaign_stage3_amendment.rs:165` 依赖
`workspace_engine::tests::make_work_item_plan_engine_with_accepted_contract_drafts`），
不在 web 侧授权路径内，且与 `coding_workspace_engine` / `workspace_engine` 两簇的
worker 改动面重叠 ⇒ 本簇**不重复动**，已在 IRC 报 Main 归并。

**#8（1 条）**：`gateway_start` 的 validate-failure 面被更早的 fail-closed 抢先
（消息前缀不是 `logical plan launch failed:`）——需进一步确认触发点；
本轮未改（改它要动 `author_launch_cases.inc.rs` 的夹具构造，属同一簇但在
门禁表内零余量文件隔壁 ⇒ 留待下一轮）。


### 本轮追加清理

- `lc_fixture_support.rs::register_alias_authority_without_projection` —— 随 #9/#10
  的修法（改为「不重写权威」）而失去唯一调用点 ⇒ **删除该函数**（不留死代码）。
- 编译复验：`cargo check --lib --tests` = **0 error**（仅剩既有 warning）。

### 共享夹具扩能力建议（product 级 `logical_test_fixture.rs`，本轮未改）

1. **不建 aggregate index**、**不写别名位 `record.json`** ⇒ 走 planning context /
   manager create / 聚合视野链路的夹具，只用 product 级共享夹具会在
   `PlanningContextResolver::build_with_fresh_index → index_store.active_required`
   处失败（`aggregate_index_unavailable`）。
   ⇒ 建议共享夹具补「聚合索引（create→activate→Degraded）+ policy bootstrap +
   别名 record」三项能力。
2. **固定 `repository_0001` 物理 id、自建 project/issue** ⇒ 无法服务
   「issue id ≠ issue_0001」或「已存在 project/repository 的现场」。
   ⇒ 建议共享夹具接受调用方传入既有 project/issue/repository。
3. **锁维度 API**（`upsert_repo_worktree` + `bind_repo_lock_to_attempt`）本簇未用到
   （web 侧无带 `target_snapshot` 的 coding attempt 夹具）⇒ 无需扩。


---

## 10. 终态（跨簇协作后的收敛链）

### 10.1 逐簇收敛轨迹

| 阶段 | 剩余 | 说明 |
|---|---|---|
| 基线（`/tmp/aria-full-tests.log`） | 61 | 本轮「单仓入口删除」引入 |
| 统一 lib 全量跑（Main 的 `/tmp/aria-full-lib-test.log`，17:48） | 10 | 本簇夹具迁移后 |
| 追加修复（本簇）+ 跨簇协作（product 侧共享基座）后 | **2** | 见下 |

### 10.2 剩余 2 条的最终归因（**都不是本簇夹具能修的**）

| # | 用例 | 归因 | 归属 |
|---|---|---|---|
| 1 | `gateway_start::logical_plan_validate_failure_is_reported_by_handler` | 断言 `message.starts_with("logical plan launch failed:")` 不成立 —— validate-failure 面被更早的 fail-closed 抢先；该用例的夹具 `gateway_fixture()` 被同文件 4+ 条**已绿**用例共用，改它有连带打红风险 | web 侧（本簇），下一轮 |
| 2 | `plan_repair_activation::{repeated_confirmation…, zero_socket…}` | **根因已推进三层**：① 重复登记 LC 权威 ⇒ `IdentityMismatch`（已修：不重写权威）；② 修掉 ① 后残余症状变为 **websocket 超时** ⇒ 指向**基座 attempt 以 `target_snapshot: None` 落盘**（路由既已是 Logical ⇒ `classify_worktree_lease_for_attempt` 要求快照 ⇒ evidence `target_snapshot_missing_for_logical` ⇒ `LeaseUnknown` fail-closed ⇒ amendment 投递面起不来）；③ **已补第三层**（与权威一致的冻结快照 `snapshot_for` + 仓维 worktree 记录 + 锁绑到该 attempt）⇒ **`cargo check` 0 error，但复跑仍红 ⇒ 第三层未命中根因**（方向大概对、细节没对） | **已移交 `FixPlanRepairActivation`**（该文件归他；树上已含我的第三层改法，他可选保留/回退/改内容） |

### 10.3 本轮在 web 侧之外促成的收敛（跨簇协作，非我改动）

- **7 条**（`campaign_stage3_interactive` 1 + `campaign_stage3_recovery_matrix` 4 + `c1_recovery_surface` 1 + `campaign_stage3_advance` 1）
  的根因都在 **product 层共享基座夹具**
  （`src/product/coding_workspace_engine/tests/campaign_stage3_amendment.rs` 依赖
  `workspace_engine::tests::make_work_item_plan_engine_with_accepted_contract_drafts`）：
  - 聚合 policy artifact 缺失 ⇒ `FinalClusterClose2` 在共享夹具加
    `ensure_planning_prerequisites` + 基座夹具加一行调用 ⇒ **6 条转绿**；
  - accepted draft 的 `target_repository_id: None` ⇒ 派生不出冻结快照 ⇒
    `FinishWSEngine2` 落权威成员 logical id ⇒ **2 条转绿**；
  - 断言读取维度（issue 维 → 仓维）⇒ `FinalClusterClose2` 的**混合实现**
    （仓维优先 + issue 维兜底）⇒ 该条转绿（实测：旧实现 34/2 failed、
    新实现 35/1 failed ⇒ **混合实现必要且已贴回**；断言主体
    `current_lock_owner_id == attempt_id` 一字未动 ⇒ 不弱化）。
- **4 条** `web::coding_ws_handler::runner::tests` ⇒ 共享夹具新增
  `seed_logical_authority_for_lc`（真 per-LC 子树权威播种，含该子树自己的
  `record.json` —— `resolve_logical` 直读该路径，legacy 别名布局才有
  `migrate_legacy` 懒生成那条捷径）+ `snapshot_for_in_scope` ⇒ 转绿。

### 10.4 本簇在 harness 侧补的两段 gateway 前置（夹具缺前置条件，非实现缺陷）

`campaign_stage3_interactive/harness.rs::campaign_stage3_fixture`：
1. **durable capability 记录** —— gateway 启动 provider 的准入凭证
   （`provider_selection.rs` 的 `launch_cell_state`：缺记录 ⇒
   `capability record missing`；launch 分格非 `Confirmed` ⇒ `launch cell is unknown`）
   ⇒ 播种 ClaudeCode 的 `Confirmed` 记录（`capability_snapshot_ref` 与 gateway
   的 `ProviderRef` 对得上）。
2. `campaign_stage3_interactive/human_action_http.rs`（**REST 侧**）：注入
   **恒健康**的 provider 可用性门（`always_available_gate()`）—— REST 侧经
   manager 重建 engine ⇒ 走**生产 gateway** ⇒ 撞 `provider_availability_gate.rs`
   的 `degraded()` 门（夹具无真实 probe ⇒ 默认健康服务恒 degraded ⇒
   `provider health state is degraded`）；WS 侧孪生用例走直连 registry 路径
   不经此门 ⇒ 两面行为原本分叉。**只改夹具装配，不改实现。**
   ⚠️ 修法踩了个 `E0382`（`with_gateway_factory` 按值 move `state`，实参又借
   `state`）⇒ 修法：先把 `state.provider_registry.clone()` 出来再 move。

### 10.5 共享夹具扩能力汇总（product 级 `logical_test_fixture.rs`）

| 能力 | 状态 |
|---|---|
| `seed_logical_authority_for_lc(paths, project_id, issue_id, root, lc_id)` —— 真 per-LC 子树权威（manifest/member/checkout/selection/policy/**子树 record.json**/repos.json 投影） | ✅ 已加 |
| `snapshot_for_in_scope(paths, project_id, logical_id, checkout_id, lc_id)` —— 按 `for_lc(lc_id)` 读权威事实（`snapshot_for` 签名与行为未变） | ✅ 已加 |
| `ensure_planning_prerequisites(paths, project_id, lc_id)` —— active(→Degraded) 聚合索引 + 聚合 policy artifact 播种 | ✅ 由 `FinalClusterClose2` 加 |
| **仍缺**：接受调用方传入**既有** project/issue/repository（`seed_complete` 自建 project/issue ⇒ 无法服务「issue id ≠ issue_0001」或「已有 repository 的现场」） | ⬜ 未加（报告给 Main） |

## 最终窄跑结果（2026-11-03 深夜收口）

`cargo test --locked --lib web::workspace_ws_handler` ⇒
**227 passed / 2 failed**（起点 61 failed → 21 → 10 → 4 → 3 → 2）。

仅剩 2 条，均为 `plan_repair_activation::*`（归属 coding 侧收尾 worker 的
#4-5，其 `seed_lc_authority_for_plan_repair`/`test_controls/plan_repair/seed.rs`
仍在迭代中，非本簇文件）：

| 仍未解项 | 现象 | 归因 |
|---|---|---|
| `plan_repair_activation::repeated_confirmation_recovers_delivery_mark_failure_without_duplicate_application` | panic 点随其夹具迭代移动（最近一次：plan_repair_activation.rs:157 unwrap None） | coding 侧在途 |
| `plan_repair_activation::zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery` | "zero-socket activation must resume the attempt: Elapsed"（amendment 投递 runner 未恢复） | coding 侧在途 |

本簇范围（single_candidate_provider_run 20、interactive 11、matrix 7、
planning_resume 5、provider_run_events 5、ir/structure reredrive 3+3、
scope_rejection 2、plan 修复外的 tests.rs 3、c1 1、gateway_start 1）
**全部转绿**。期间由本簇一并处理的关联修复：campaign harness 的 capability
行播种 / 成员语言规则 / Degraded 聚合索引 / 恒健康 availability 工厂注入
（后两项与并行同胞协作完成）、gateway_start validate-failure 夹具迁移为
「保 manifest + 登记成员 + selection + 投影升级 + 删 capabilities.json」、
断言失败面增强（等强度，附带真实错误消息）。

### 最终行数核对（对登记上限）

| 文件 | 实测 | 登记 | 结论 |
|---|---|---|---|
| single_candidate_provider_run.rs | 1493 | 1508 | ✓ |
| planning_resume.rs | 2449 | 2449 | 持平 ✓ |
| provider_run_events.rs | 1257 | 1257 | 未改持平 ✓ |
| single_candidate_lc_admission.rs | 2638 | 2638 | 未改持平 ✓ |
| campaign_stage3_interactive/cases.rs | 1209 | 1209 | 未改持平 ✓ |
| tests.rs | 1176 | （默认线 1200） | ✓ |
| lc_fixture_support.rs（新增） | 339 | 不在表内 | ✓ |
| campaign_stage3_interactive/harness.rs | 600 | 不在表内 | ✓ |
| plan_repair_activation.rs | 367 | 不在表内 | ✓（coding 侧迭代中） |
| single_candidate_scope_rejection.rs | 293 | 不在表内 | ✓ |
| gateway_start_parts/author_launch_cases.inc.rs | 710 | 不在表内 | ✓ |

`git diff --stat`（src/web/workspace_ws_handler/ 全域，含并行同胞的改动）：
17 files changed, 585 insertions(+), 136 deletions(-)。实现目录零改动。


---

## 11. 交接给下一手的「树上基线」（`plan_repair_activation.rs`）

该文件（367 行，mtime 19:29:32）**当前已含**我 19:29 落的**未验证**第三层改法：

```rust
// seed_lc_authority_for_plan_repair(root) 里：
//   ① ProjectStore::get("project_0001")     → 幂等触发 migrate_legacy（别名位 record.json）
//   ② ensure_planning_index_and_policy(...) → active(→Degraded) 聚合索引 + policy
//   ③ attempt.target_snapshot = Some(snapshot_for(...))          ← 第三层（未验证）
//      + upsert_repo_worktree(...) + bind_repo_lock_to_attempt(...) ← 第三层（未验证）
```

⚠️ **③ 加了之后那 2 条仍红** ⇒ 它**未命中根因**。下一手可选 (a) 保留 ③ 继续查 /
(b) 回退 ③（回到「`target_snapshot: None` + 路由 Logical ⇒ `target_snapshot_missing_for_logical`」）/ 
(c) 改 ③ 的内容（快照 `canonical_path` 与 `attempt.worktree_path` 对齐、锁的 `work_item_id` 换值）。

**最后三站（按此顺序查最快）**：
1. **租约分类的 evidence 原文** —— 跑一次窄跑从 `failures:` 段取 panic 原文；evidence
   变了 ⇒ ③ 生效、卡点前移；没变 ⇒ ③ 没写进 attempt 的**最终落盘事实**（基座
   `seed.rs` 之后可能还有别处回写 attempt 覆盖了它）⇒ 查 attempt 的落盘 JSON。
2. **仓维锁的 `work_item_id`** —— 基座为 `"wi_registration"`；
   `bind_repo_worktree_lock_to_attempt` 要求 `record.current_active_work_item_id == work_item_id`
   且 owner 是 `repo_worktree_lease_*` 前缀或已是 attempt id ⇒ 确认 bind **真的成功**。
3. **amendment 投递面本身** —— `plan_amendment_updated` 由
   `src/product/coding_workspace_engine/amendment.rs:681` 的
   `deliver_plan_amendment_observation_once` 发出（需 socket write 注册 + ack）；
   若租约已放行却仍超时 ⇒ 看该投递面（`register_plan_amendment_socket_write` /
   `load_or_prepare_plan_amendment_delivery`）。


---

## 12. 自我更正记录（第三层改法**有效**；破口在配套查找键）

**留痕目的**：本报告 §10/§11 记录的「第三层补快照 + 仓维锁」期间，我曾**误判**它为
「净负、建议回退」。`FixPlanRepairActivation` 用诊断通道实测推翻了我的误判 —— **③ 是
有效的修法**，破口只在配套的查找键。此处保留完整的判断与更正过程（收口评审需看到这一层）。

### 12.1 破口机制（`FixPlanRepairActivation` 定位，我复核确认）

`CodingAttemptStore::get_attempt_for_work_item_group`
（`src/product/coding_attempt_store/group.rs:77-101`）按
```rust
attempt.work_item_group_id.as_deref() == Some(plan_id)
  && Self::attempt_target_bucket(attempt) == target      // ← 桶
```
过滤，而 `attempt_target_bucket`（:132-140）= **有 `target_snapshot` ⇒
`Some(logical_repository_id)`，无 ⇒ `None`**；`work_item_group_id` 恒为
`Some(input.plan_id)`（:287）⇒ 那半个条件恒真，**桶是唯一变量**。

⇒ 我加快照 ⇒ 桶从 `None` 变 `Some(member_id)` ⇒ 用例正文写死 `None` 的查找不中
⇒ `.unwrap().unwrap()` 在**第二层 unwrap** panic ⇒ **破口由我引入**。

### 12.2 我的误判与更正（**我错了**：③ 有效）

我曾据「③ 之前同样 fail-closed」推断「③ 没把链子打通 ⇒ 净负 ⇒ 建议回退」——
**后半句是错的**。`FixPlanRepairActivation` 的诊断通道实测原文（**非推断**）：

```
DIAG attempt.target_snapshot.is_some = true
DIAG lease.disposition = ActiveWait / lease_id = 本 attempt 自己（自持 ⇒ 不拦截）
DIAG repo worktree record = Some(current_active_work_item_id: "wi_registration",
                                  current_lock_owner_id: 本 attempt)
DIAG issue shared worktree record = None
DIAG coding frame = {"type":"plan_amendment_updated", …}
DIAG delivery = Ok((Delivered, …)) / journals = Completed
```

⇒ **③ 确实落盘生效、租约确实从 Unknown 变自持 `ActiveWait`、投递面确实通了。**
③ 落盘后那次红的**真实原因**不是「租约没通」，而是 **③ 改了 attempt 的 target 检索桶**
（`attempt_target_bucket`：快照在场 ⇒ `Some(logical_repository_id)`），**而用例正文
写死 `get_attempt_for_work_item_group(..., None)` 按桶过滤 ⇒ 查找落空 ⇒ 第二层
`unwrap()` panic（`:157` / `:284`）⇒ **根本没走到 websocket**。
⇒ **症状从「websocket 超时」前移到「unwrap None」，是配套查找键没跟上，不是链子没通。**

### 12.3 最终修法（Main 裁决「丙」，`FixPlanRepairActivation` 落地）

**保留 ③** + 把取 attempt 的口径换成 **public 且不看桶** 的
`list_attempts_for_work_item_group`（`group.rs:106`，只按 `work_item_group_id` 过滤）
⇒ 与「attempt 有没有快照」彻底解耦，不脆；新增本地 helper `plan_repair_attempt`，
3 处调用点统一走它。**断言零改动、实现零改动。**

### 12.4 当时的未解层（**已由 12.3 解掉，非实现面问题**）

我曾把这一层报给 Main 判「实现面需裁定」（当时误以为 ③ 之后仍 fail-closed）：

```
routing = Logical（权威已建）但 attempt 无 target_snapshot
⇒ classify_worktree_lease_for_attempt ⇒ evidence "target_snapshot_missing_for_logical"
⇒ lease_exclusion_option 不放行 ⇒ LeaseUnknown ⇒ fail-closed
⇒ amendment 投递面起不来
```
而该用例链上**不能**给 attempt 加快照（撞桶语义 ⇒ 检索不中）⇒ **夹具层无解**。
两条出路（已报 Main 裁定）：**(甲)** 该用例链上就不该建 LC 权威（但会撞
`resolve_for_issue` 的 `NotFound logical_codebase` ⇒ 两难）；**(乙)** 产品侧为
「routing=Logical 且 attempt 无快照」给一个非 fail-closed 判定（**实现面改动 ⇒
需 Main 批准 + 单独立项**）。

### 12.5 一个可复用的夹具层口径（供后续同类用例）

若确需「带桶的 attempt」又要按 plan 查，**不必依赖桶**：
`CodingAttemptStore::list_attempts_for_work_item_group(project_id, issue_id, plan_id)`
（`group.rs:106`，**public**）只按 `work_item_group_id` 过滤、**不看桶** ⇒
夹具带不带快照都能查到。


---

## 13. 本簇终态（更新，覆盖 §10 的「剩余 2 条」）

`FixPlanRepairActivation` 收掉最后 2 条后：

```
cargo test --locked --lib plan_repair_activation → 2 passed; 0 failed
cargo test --locked --lib web::workspace_ws_handler → 229 passed; 0 failed
```

⇒ **本簇 61 → 0**（收敛轨迹：61 → 21 → 10 → 4 → 3 → 2 → **0**）。

**收口状态**：
- 该 worker 的 TEMP DIAGNOSTIC 整段已删净（`grep -rn 'TEMP DIAGNOSTIC' src/` = 0 命中）；
- `plan_repair_activation.rs` 372 行；**只改 1 个文件**；断言零改动、实现零改动；
- 未 git add/commit/stash/checkout；
- 报告：`cadence/notes/2026-11-03_失败簇收尾_plan_repair_activation_v1.0.md`。

**姊妹簇**：`web::coding_ws_handler` 58 → **4**（剩余 4 条归 `CloseCodingWsHandler4`：
3 条 `plan_repair::*` 走**生产 gateway 工厂**（durable capability / 真实健康门 / 真实
trust store，web 测试面无法就地满足）+ 1 条 `runner_recovery::startup_window_compensation`
（夹具被同文件 20+ 条已绿用例共用，改动面大本轮未冒险动））。
