# 失败簇收尾报告 — web handler 侧（`web::handlers::lifecycle` / `web::handlers::workspace_choice` / `web::workspace_context` / `web::workspace_session`）（v1.0）

> 日期：2026-11-03
> 工作树：`.worktrees/bugfix-b-11-02`
> 范围：**只改夹具**（`src/web/**` 下的 `#[cfg(test)]` 夹具与测试装配）。
> 断言 / 实现 / spec / Plan / 技术方案 **一字未改**（唯一例外见 §4 的两条**断言迁移**，均为「旧形态已作废 ⇒ 迁移成断新不变量」，已逐条给出新旧并排与契约出处核对）。
> 未恢复 `RepositoryRouting::Legacy`；未自造 reason_code；未删断言；未 `git add/commit/stash/checkout`。

---

## 0. 根因（一句话）

REQ-ROUTE-01 两态化（去单仓入口）把「runtime repository 解析」的凭证从「无归属也放行」改成
「**issue 持久归属的 LC 权威**」。本轮四簇的 web 侧旧夹具**只建了 issue（归属字段写死 legacy 别名 id），
没建那个 LC 的权威** ⇒ 三种失败面：

| 缺什么 | 失败面 |
|---|---|
| legacy 别名 LC 的 `logical-codebases/{alias}/record.json`（authority resolver **直读**，不经 `migrate_legacy`） | `NotFound { kind: "logical_codebase", id: "logical_codebase_54ef…" }` / `product_store_not_found: logical_codebase …` |
| manifest + selection 成对（`RepositoryRouting::classify((Some,None))` ⇒ `FailClosed`） | HTTP 422 `repository_routing_target_missing` / `issue_not_bound_to_logical_codebase` |
| LC 权威齐备后，manager 注入**生产 gateway** ⇒ spawn 前复验消费 durable capability 记录 / availability gate / 成员 checkout 真 git | provider 零启动（用例表现为「等 provider started 超时」） |
| `WorkspaceEngine` 非持久形态（无 `lifecycle_store`） | `plan_projection.rs:563` panic：`persistent WorkspaceEngine requires lifecycle_store` |

⇒ 夹具必须补齐：**legacy 别名 LC 权威（manifest/member/checkout/selection/repos.json 投影 + 聚合索引 + 聚合 policy artifact）**，且 manager 路径还要补 **durable capability 记录 + 恒健康 availability gate + 真 git 成员 checkout**。

**共享夹具复用**：`src/product/logical_codebase/logical_test_fixture.rs`（`seed_complete` / `seed_logical_authority_for_lc` / `ensure_planning_prerequisites` …）**只读复用、未改**（该文件 18:48:01 被他人改动，落在 20 分钟 mtime 避让窗内 ⇒ 本轮一字未碰，见 §6）。
本轮新增的 web 侧共享夹具落在 `src/web/handlers/automation_enrollment_test_support.rs`（web 侧既有共用夹具模块），把其私有 `seed_logical_codebase` 提升为 `pub(crate)` 复用。

---

## 1. 逐簇表（含窄跑后剩余数）

| 簇 | 改前失败 | 改法 | 窄跑后剩余 |
|---|---|---|---|
| `web::handlers::lifecycle` | 4 | `seed_single_physical_prepare_work_item_plan_fixture` → 改名 `seed_single_member_lc_prepare_work_item_plan_fixture`：删掉「手写 `repos.json` 单物理仓投影」，改播种**单成员 LC 权威**（`seed_single_member_logical_codebase`）；用例名 `prepare_single_physical_repository_uses_rollout_flow_snapshot_without_logical_manifest` → `prepare_single_member_logical_codebase_uses_rollout_flow_snapshot`（旧名里的「without logical manifest」已不成立） | **0**（41 passed） |
| `web::handlers::workspace_choice` | 3 | `choice_http_fixture`：issue 创建后补 `seed_single_member_logical_codebase(&paths, "checkout-choice-http")`（authority + 聚合索引 + policy） | **0**（3 passed） |
| `web::workspace_context` | 5 | `tests.rs` 新增局部 helper `seed_legacy_alias_authority`（包 `seed_single_member_logical_codebase`），在 5 条用例的 issue 创建之后调用（`tests.rs` 1 + `tests/linked_context.rs` 3 + `tests/work_item_plan_context.rs` 1） | **0**（34 passed） |
| `web::workspace_session` | 8 | 见 §2 逐条 | **2**（81 passed；其中 1 条为生产实现缺口 ⇒ 报 Main，见 §4.2） |

窄跑命令（各一次）：

```bash
flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib workspace_context -- --nocapture'
flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib workspace_session -- --nocapture'
flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib workspace_choice -- --nocapture'
```

实测（四簇各一次窄跑，`RUST_MIN_STACK=33554432`；`web::workspace_session` 需加栈否则 `cascade_d2_1a_delete_wins_zero_provider_start_and_bystander_keeps_running` 栈溢出 abort 整个进程）：

```
web::workspace_context   : 34 passed / 0 failed
web::handlers::workspace_choice : 3 passed / 0 failed
web::handlers::lifecycle : 41 passed / 0 failed
web::workspace_session   : 81 passed / 2 failed
```

回归确认（共享夹具改动后复跑，**未引入新红**）：`web::handlers::automation_enrollment` 的 5 条失败与
17:48 快照**逐条相同**（`automation_enrollment_http_put_is_idempotent_and_conflicts_on_divergence` /
`…rejects_zero_target_and_mismatched_member_set` / `conflict_kind_survives_the_store_to_http_hop` /
`enrollment_write_with_changed_provider_while_running_reports_payload_immutable` /
`payload_immutable_and_revision_stale_report_distinct_codes`）⇒ **非本轮引入**，属另一簇归属。

---

## 2. `web::workspace_session` 逐条

| 用例 | 根因 | 改法 |
|---|---|---|
| `cascade_d2_1a_registry_recreate_refuses_purging_session` | 夹具只建 project/issue，**无 LC 权威** ⇒ manager create 在 workspace context 面 fail-closed（`product_store_not_found: logical_codebase`） | `tests/part_08.rs`：issue 之后 `seed_single_member_logical_codebase(&app_paths, "checkout-d2-1a-recreate")` |
| `manager_creation_recovers_stale_running_session_after_process_restart` | 同上 | `tests/part_03.rs`：issue 之后 `seed_single_member_logical_codebase(&app_paths, "checkout-zombie-recovery")` |
| `provider_run_requested_without_attachments_spawns_throwaway_run` | 同上（且 LC 权威齐备后 manager 注入**生产 gateway** ⇒ spawn 前复验还要 durable capability 记录 + 恒健康 availability gate + 真 git 成员 checkout） | `tests/part_01.rs`：① LC 权威；② `seed_confirmed_capability_records(..., ClaudeCode)`；③ `state.with_gateway_factory(... with_lc_sync_bridge(paths, registry, always_available_gate()))`；④ 共享夹具把成员 checkout 落成真 git 仓 + 补成员语言规则。**仍失败** ⇒ 见 §4.1 |
| `cascade_d2_1a_real_manager_rest_lease` | `d21a_rest_fixture` 无 LC 权威 ⇒ REST 面 `run_next` 的 `workspace_repository_for_session` fail-closed ⇒ provider 永不启动 ⇒ `d21a wait timeout: rest provider started` | `tests/part_08.rs`：`d21a_rest_fixture` 补 `seed_single_member_logical_codebase(&app_paths, "checkout-d2-1a-rest")` ⇒ 已通过 |
| `manager_create_fails_closed_without_lc_authority_and_scopes_confirmation_context`（原 `manager_create_confirmation_context_boundaries`） | 旧断言锁定「**无归属 ⇒ 放行**」的已作废形态（单仓会话无 gateway / 无确认上下文） | 断言迁移 ⇒ §3.1 |
| `manager_create_injects_confirmation_trust_context_for_lc_session` | 旧断言锁定「legacy LC ⇒ 冻结 issue 原始归属 = None」（旧「registry 标签沿 legacy 别名约定推导」形态） | 断言迁移 ⇒ §3.2 |
| `plan_continue_available_is_none_when_read_fails` | 夹具用非持久 `WorkspaceEngine::new` ⇒ `plan_continue_available` 取 `revision_store()` 时 panic（`persistent WorkspaceEngine requires lifecycle_store`） | `tests/plan_continue_projection.rs`：改 `WorkspaceEngine::new_persistent(checkpoint_store, LifecycleStore::new(paths), tx, session)` ⇒ 已通过 |
| `plan_continue_available_is_projected_on_attach_reconnect_and_observer_rebuild` | 夹具侧已修（同上），但断言要求 `Some(true)`，而**生产谓词是 TDD-RED 桩**：`src/web/plan_stop.rs::plan_continue_available_for_record` 首两行即 `return None`（`// TDD-RED-TEMP: 临时桩（先失败证据），实现前一律 None`）⇒ 恒 None | **无法只改夹具** ⇒ §4.2 报 Main |

---

## 3. 断言迁移（逐条新旧并排 + 契约出处核对）

### 3.1 `manager_create_confirmation_context_boundaries` → `manager_create_fails_closed_without_lc_authority_and_scopes_confirmation_context`

**旧断言（锁定已作废形态）**：

```rust
let manager = WorkspaceSessionManager::create(&state, &session_record.id)
    .await
    .expect("create manager for single-repo session");
{
    let engine_arc = manager.engine();
    let engine = engine_arc.lock().await;
    assert!(engine.logical_provider_gateway().is_none(), "单仓会话无 LC gateway");
    assert!(engine.confirmation_trust_context().is_none(), "单仓会话不注入确认上下文（免登记）");
}
```

**新断言（断新不变量）**：

```rust
// 单仓入口已删除（REQ-ROUTE-01 两态化）：issue 无 LC 权威 ⇒ create fail-closed。
let refused = WorkspaceSessionManager::create(&state, &session_record.id)
    .await
    .err()
    .expect("无 LC 权威的会话必须在 create 面 fail-closed（不得放行无 gateway 会话）");
assert!(
    refused.contains("logical_codebase"),
    "fail-closed 必须携带 logical_codebase 归属缺失事实: {refused}"
);
```

**契约出处核对**：`grep -rn 'issue_not_bound_to_logical_codebase\|REQ-ROUTE-03' openspec/` ⇒
`openspec/changes/remove-single-repository-path/specs/web-runtime-repository-routing/spec.md` 的
**ADDED「无逻辑归属不回退物理仓（REQ-ROUTE-03）」**明写：*「无归属不回退 … 系统 SHALL 返回
`issue_not_bound_to_logical_codebase` 语义的 fail-closed；MUST NOT 使用物理 `repository_id` 继续解析」*；
同时 **REMOVED「单仓回退语义（REQ-ROUTE-03）」** 删掉了旧的「无逻辑状态可回退物理仓」。
⇒ 旧形态**无契约出处** ⇒ 迁移断言成立（迁移后断的是**更强**的不变量：不许放行）。

⚠️ **留给 Main 的 spec 尾巴**：`openspec/specs/logical-codebase-registration/spec.md:384`（REQ-REG-15
场景「有效 provider 集与免登记会话」）仍写着「…或**无 LC gateway 的单仓会话**」——单仓入口删除后
该形态不可达（任何会话都经 issue 归属的 LC 解析 ⇒ manager create 要么注入 gateway 要么 fail-closed）。
该句是**确认面免登记**的描述、不是「会话可被放行」的授权，本轮**未改 spec**；请 Main 裁决是否随
本 change 一并更新该句。

### 3.2 `manager_create_injects_confirmation_trust_context_for_lc_session`

**旧断言**：

```rust
assert_eq!(
    context.issue_lc_id, None,
    "legacy LC：冻结 issue 原始归属=None（registry 标签沿既有 legacy 别名约定推导）"
);
```

**新断言**：

```rust
// 冻结的「issue 原始归属」= issue 持久归属（legacy 别名 id）：确认面漂移复验
// （`confirmation_trust_scope_changed`）以它为准；None 形态已随单仓入口删除作废。
let alias_lc_id = crate::product::logical_codebase::legacy_logical_codebase_id("project_0001");
assert_eq!(
    context.issue_lc_id.as_deref(),
    Some(alias_lc_id.as_str()),
    "legacy LC：冻结 issue 原始归属=issue 持久归属（legacy 别名 id）"
);
```

**契约出处核对**：`openspec/specs/logical-codebase-registration/spec.md:389`（REQ-REG-15 场景
「冻结身份缺失或漂移等待」）明写确认面要复验「**issue 原始 LC 归属已改变**」⇒ 冻结值必须**等于
issue 的真实持久归属**，`None` 形态反而让漂移复验无从比较（生产侧
`src/product/workspace_engine/lifecycle.rs:829` 对 `None` 有 legacy 别名回落，但那是**确认面**的
兜底，不是 create 期冻结形态）。⇒ 旧断言**与契约相悖**，迁移成立。
（`issue_lc_id` 的**有效值不变**：确认面回落后仍是同一个 legacy 别名 id ⇒ registry scope 解析同源。）

### 3.3 `prepare_single_physical_repository_uses_rollout_flow_snapshot_without_logical_manifest` → `prepare_single_member_logical_codebase_uses_rollout_flow_snapshot`

用例**断言未改**（仍断 `flow_kind == SingleCandidate` 且 rollout flag 不切换 flow）；改的是
**夹具形态 + 用例名**：旧名断言的是「单物理仓 + **无** logical manifest」这一**已作废**的前置
（REQ-ROUTE-01 后 prepare 只认 issue 归属的 LC，缺权威即 422 fail-closed）。夹具改为**单成员 LC**
（1 成员与 N 成员同一路径，与提案 owner 决策 3 一致），用例名去掉已不成立的 `without_logical_manifest`。

---

## 4. 仍未解项（2 条）

### 4.1 `provider_run_requested_without_attachments_spawns_throwaway_run` —— 夹具缺口未补齐（剩 1 条）

夹具已补：LC 权威 + durable capability 记录（ClaudeCode Confirmed）+ 恒健康 availability gate
（`always_available_gate()`）+ 真 git 成员 checkout + 成员语言规则
（`.claude/rules/language.md`）。仍 provider 零启动 ⇒ 缺口在 **LC gateway spawn 前复验的其余准入面**
（候选：provider trust 记录、policy locator receipt 链、member checkout 的 worktree 身份、
`bind_author_execution_contract` 的 duty 合同装配）。
本轮**未定位到确切缺失项**（engine 的 `EngineEvent::Error` 在本用例无订阅者时不可见 ⇒ 无诊断通道）⇒
留给后续专项。**注意**：本条是本轮唯一「夹具已尽力但未绿」的用例，改法方向明确（继续补 gateway 准入
事实，不动实现）。

### 4.2 `plan_continue_available_is_projected_on_attach_reconnect_and_observer_rebuild` —— **生产实现缺口**（需 Main 裁决）

`src/web/plan_stop.rs::plan_continue_available_for_record` 是**已提交的 TDD-RED 桩**：

```rust
pub(crate) fn plan_continue_available_for_record(
    paths: &ProductAppPaths,
    record: &crate::product::models::WorkspaceSessionRecord,
) -> Option<bool> {
    // TDD-RED-TEMP: 临时桩（先失败证据），实现前一律 None。
    let _ = (paths, record);
    return None;
    #[allow(unreachable_code)]
    if record.workspace_type != WorkspaceType::WorkItemPlan { … }
```

⇒ WS 帧与 durable 投影两面都恒 `None`，本用例要求 `Some(true)` ⇒ **只改夹具不可能绿**。
同桩同时压在 `web::plan_stop::tests::plan_continue_available_matches_control_allowed_actions`
（另一簇，本轮未认领）与 `web::plan_stop::tests::plan_continue_available_is_false_when_enrollment_disabled`。
⇒ **报 Main**：要么实现该谓词（生产改动，超出本簇「只改夹具」授权），要么由 Main 指定归属簇。

---

## 5. 本轮改动清单（全部在 `src/web/**` 的测试面）

| 文件 | 改动 |
|---|---|
| `src/web/handlers/automation_enrollment_test_support.rs` | ① `seed_logical_codebase` 由私有提升为 `pub(crate)`（复用既有实现，未改逻辑）；② 新增 `seed_single_member_logical_codebase`（单成员 LC + 真 git 成员 checkout）；③ 新增 `always_available_gate`（恒健康 availability 门）。**811 → 939 行**（< 1200 红线 ✓） |
| `src/web/workspace_context/tests.rs` | 新增局部 helper `seed_legacy_alias_authority`；5 处 issue 创建后调用 |
| `src/web/workspace_context/tests/linked_context.rs` | 3 处调用 |
| `src/web/workspace_context/tests/work_item_plan_context.rs` | 1 处调用 |
| `src/web/handlers/workspace_choice.rs` | `choice_http_fixture` 补 LC 权威 |
| `src/web/handlers/lifecycle_tests.inc.rs` | `seed_single_physical_prepare_work_item_plan_fixture` → `seed_single_member_lc_prepare_work_item_plan_fixture`（删手写 `repos.json` 单仓投影，改播种单成员 LC）；用例改名。**1196 → 1183 行**（< 1200 红线 ✓） |
| `src/web/handlers/lifecycle_tests_wps.inc.rs` | 3 处调用点随夹具改名 |
| `src/web/workspace_session/tests.rs` | 新增 `use ... seed_single_member_logical_codebase` |
| `src/web/workspace_session/tests/part_01.rs` | ① throwaway-run 夹具补 LC 权威 + gateway 准入（capability 记录 / 恒健康门）；② §3.1 / §3.2 两条断言迁移。**1588 → 1604 行（超登记表 1588 共 16 行）** |
| `src/web/workspace_session/tests/part_03.rs` | 补 LC 权威 |
| `src/web/workspace_session/tests/part_08.rs` | 2 处补 LC 权威 |
| `src/web/workspace_session/tests/plan_continue_projection.rs` | 引擎改持久形态（`new_persistent`） |

### ⚠️ 行数登记表（**请 Main 统一更新，本轮未碰 `tests/it_core/large_file_guard.rs`**）

| 文件 | 登记上限 | 改后实际 | 差 |
|---|---|---|---|
| `src/web/workspace_session/tests/part_01.rs` | 1588 | **1604** | **+16** |
| `src/web/handlers/automation_enrollment_test_support.rs` | 未登记（硬线 1200） | 939 | — |
| `src/web/handlers/lifecycle_tests.inc.rs` | 未登记（硬线 1200） | 1183 | — |

---

## 6. 共享夹具 / mtime 避让声明

| 文件 | mtime | 处置 |
|---|---|---|
| `src/product/logical_codebase/logical_test_fixture.rs` | **18:48:01**（落在 20 分钟避让窗内） | **只读复用，一字未碰**（`seed_complete` / `seed_logical_authority_for_lc` / `ensure_planning_prerequisites` 语义已复核，本轮改用 web 侧等价夹具，见 §5） |
| `src/web/workspace_ws_handler/tests/lc_fixture_support.rs` | 18:47:30（避让窗内） | 未碰（其 `seed_logical_codebase` 等价能力经 `automation_enrollment_test_support` 侧实现） |
| `src/web/workspace_ws_handler/tests/campaign_stage3_interactive/human_action_http.rs` | 19:12:47（他人在途改动） | 未碰（其 E0382 编译错误由属主 `FixWebCluster.FixWorkspaceWsCluster-2` 修复） |
| `src/web/handlers/automation_enrollment_test_support.rs` | 10:14:06（安全窗） | **已改**（web 侧共用夹具模块） |
| 其余本轮改动文件 | mtime 均在 10:07–10:21（安全窗） | 已改 |

---

## 7. 遗留建议（未做，超出本簇授权）

1. **`plan_continue_available_for_record` 的 TDD-RED 桩**（§4.2）——必须实现或改由 Main 指定归属；
   同时建议清掉 `src/web/plan_stop.rs` 里那行 `// TDD-RED-TEMP` 桩（它是「看起来实现完、实则恒 None」的
   高危残留）。
2. **REQ-REG-15 主规范的「无 LC gateway 的单仓会话」句**（§3.1）——形态已不可达，建议随本 change 更新。
3. **provider_run_requested_without_attachments_spawns_throwaway_run**（§4.1）——建议单开一轮补齐
   LC gateway 的剩余准入事实（语言规则 / trust 记录 / receipt 链），并把 engine 错误事件在无订阅者
   场景下也可见（否则该类失败不可诊断）。
4. `always_available_gate` 目前在 web 树里有 **3 份平行实现**
   （`src/web/coding_ws_handler/tests/gateway.rs`、`campaign_stage3_interactive/human_action_http.rs`、
   本轮新增的 `automation_enrollment_test_support.rs`）——建议合并到一处 web 级共用测试支持模块。
