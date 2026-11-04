# it_product 夹具迁移 · Phase 2（part_03..part_06）· v1.0

> 日期：2026-11-04
> 执行：MigrateItProductP2A（worker）
> 范围：`tests/it_product/product_coding_workspace_engine/part_03.rs` / `part_04.rs` / `part_05.rs` / `part_06.rs`
> **未改任何 `src/**` / `web/**` / 其他 `tests/it_*` / `openspec/**` / `context_builder.rs`。**

---

## 0. 一句话结论

**我负责的 4 个文件共 28 个用例，全部迁绿（28 绿 / 0 红）。**
全量 `it_product`：**211 passed / 0 failed / 0 ignored**（本组开工时为 22 passed / 51 failed 的模块基线）。

P1（provider-run 族）的判定前提——「`ValidatedStreamingProviderInput::into_parts` 是 `pub(crate)`」——
**在本组施工期间被产品侧解除（A1 落地，`into_parts` 改 `pub`）**，原先判为 P1 的 3 条
随之迁绿；P2（issue 维 shared worktree 族）在本组文件里**一条都没有**。

---

## 1. 迁绿清单（28/28）

### part_03.rs（4 例，全绿）

| # | 用例 | 夹具改动 | 状态 |
|---|---|---|---|
| 1 | `retry_code_review_prompt_includes_previous_role_run_diagnostic` | LC 权威 + 冻结快照 + gateway（`GatewayValidatedStartAdapter`） | ✅ |
| 2 | `coding_code_reviewer_run_records_session_without_resume` | LC 权威 + 冻结快照 + gateway（`GatewayExactInputAdapter`，取**真实** input） | ✅ |
| 3 | `coding_internal_reviewer_uses_fresh_provider_session` | 同上（internal reviewer 角色，Pi） | ✅ |
| 4 | `execute_coding_includes_work_item_context_in_provider_prompt` | LC 权威 + 冻结快照 + gateway（`GatewayBridgedAdapter`） | ✅ |

### part_04.rs（13 例，全绿）

| # | 用例 | 夹具改动 | 状态 |
|---|---|---|---|
| 1 | `pi_role_with_supervised_mode_normalized_to_auto` | LC 权威 + 冻结快照 + gateway（`GatewayExactInputAdapter`） | ✅ |
| 2 | `pi_failure_does_not_trigger_fresh_retry` | 同上（coder=Pi，替身改挂 Pi） | ✅ |
| 3 | `coding_prompt_includes_rework_fix_hints` | LC 权威 + 冻结快照 + gateway（`GatewayBridgedAdapter`） | ✅ |
| 4 | `execute_coding_includes_unconsumed_context_notes_and_consumes_them` | 同上 | ✅ |
| 5 | `execute_coding_emits_prompt_for_coder_provider` | 同上（coder=Codex，替身 provider_type=Codex） | ✅ |
| 6 | `execute_coding_forwards_provider_execution_and_tool_events` | 同上（`GatewayValidatedStartAdapter` 保住 Execution/ToolCall 事件） | ✅ |
| 7 | `execute_coding_persists_coder_role_run_and_full_output_chat_entry` | 同上（Fake → ClaudeCode，见 §4.1 唯一一条期望值改动） | ✅ |
| 8 | `execute_coding_forwards_provider_permission_choice_and_status_events` | LC 权威 + 冻结快照 + gateway（start 桥，Permission/Choice 事件不丢） | ✅ |
| 9 | `execute_coding_forwards_permission_responses_to_provider_session` | 同上 | ✅ |
| 10 | `execute_coding_persists_provider_choice_and_resumes_after_response` | 同上 | ✅ |
| 11 | `execute_coding_blocks_when_provider_completes_before_choice_response` | 同上 | ✅ |
| 12 | `execute_coding_blocks_later_permission_before_choice_response` | 同上 | ✅ |
| 13 | `execute_coding_stops_forwarding_provider_events_after_abort_command` | 同上 | ✅ |

### part_05.rs（8 例，全绿）

| # | 用例 | 夹具改动 | 状态 |
|---|---|---|---|
| 1 | `execute_code_review_forwards_provider_execution_events` | LC 权威 + 冻结快照 + gateway（start 桥） | ✅ |
| 2 | `execute_code_review_persists_role_run_events_while_forwarding_realtime_events` | 同上 | ✅ |
| 3 | `execute_code_review_persists_provider_control_and_tool_event_payloads` | 同上 | ✅ |
| 4 | `execute_code_review_records_permission_timeout_as_timeout_event` | 同上 | ✅ |
| 5 | `execute_internal_pr_review_forwards_provider_execution_events` | 同上（internal reviewer 角色） | ✅ |
| 6 | `execute_code_review_persists_report_and_emits_review_events` | LC 权威 + 冻结快照 + gateway（run_streaming 桥） | ✅ |
| 7 | `parses_real_provider_review_finding_aliases` | 同上 | ✅ |
| 8 | `review_payload_parse_failure_records_blocked_reviewer_evidence` | 同上 | ✅ |

### part_06.rs（3 例，全绿）

| # | 用例 | 夹具改动 | 状态 |
|---|---|---|---|
| 1 | `execute_code_review_blocked_creates_retry_gate` | LC 权威 + 冻结快照 + gateway（run_streaming 桥） | ✅ |
| 2 | `code_review_provider_start_failure_marks_attempt_blocked_and_node_failed` | 同上（start 桥，替身启动失败原样透出） | ✅ |
| 3 | `execute_code_review_prompt_includes_diff_work_item_rules_and_role_provider` | 同上（code_reviewer=Codex） | ✅ |

**未弱化任何断言**：全程零 `assert_*` 删除、零测试名改动、零 `#[ignore]`；
**唯一的期望值改动**见 §4.1（Main 授权 ①）。

---

## 2. 全量 it_product 数字

```bash
flock /tmp/aria-cargo.lock cargo test --locked --test it_product
```

```
test result: ok. 211 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.95s
```

module 级：

```bash
flock /tmp/aria-cargo.lock cargo test --locked --test it_product -- product_coding_workspace_engine::
```

```
test result: ok. 73 passed; 0 failed; 0 ignored; 0 measured; 138 filtered out; finished in 0.85s
```

编译：

```bash
flock /tmp/aria-cargo.lock cargo build --locked --tests
```

⇒ `Finished dev profile … `，**0 warning / 0 error**。

`cargo fmt --check` ⇒ 无输出（干净）。

---

## 3. P1 / P2 判定与阻塞签名

### 3.1 最终态：本组 **0 红**，无 P1、无 P2

### 3.2 施工过程中的中间态（供 Main 参考）

**P1（provider-run 族）——已被 A1 解除，不再是阻塞**

施工开始时判为 P1 的 4 条，原始错误签名全部是同一条：

```
ProviderStream("coding_run_lease_unknown: 租约状态未知，已停等：repository routing load failed: product_store_not_found: issue issue_0001")
```

| 用例 | 原阻塞签名 | 解除方式 |
|---|---|---|
| `coding_code_reviewer_run_records_session_without_resume` | 同上 | A1：`into_parts` 改 `pub` ⇒ 用 `GatewayExactInputAdapter` 取**真实** `StreamingProviderInput` |
| `coding_internal_reviewer_uses_fresh_provider_session` | 同上 | 同上 |
| `pi_role_with_supervised_mode_normalized_to_auto` | 同上 | 同上（**这条必须用真值**：它断言引擎把 Pi 的 `Supervised` 归一成 `Auto`，用 `launch_view()` 重建会是**假绿**） |
| `execute_coding_persists_coder_role_run_and_full_output_chat_entry` | 同上 | Main 授权 ①（Fake → ClaudeCode）+ `GatewayBridgedAdapter` |

> **A1 的产品侧缺口本身已不在**：`src/cross_cutting/session_launch.rs` 的
> `ValidatedStreamingProviderInput::into_parts` 自 2026-11-04 19:16 起是 `pub`
> （裸 input 的**构造**仍无 public 入口，validated 载体依旧是唯一启动凭据）。
> ⇒ **它_product 里剩下的 provider-run 断言不必再因 A1 判红**。

**P2（issue 维 shared worktree 族）——本组文件里一条都没有**

本组 28 条全部是单 Work Item 的 provider run / code review 用例，
不触碰 `IssueSharedWorktreeRoute`；该族剩余用例全在 part_01 / part_13 / part_17
（其他 worker 负责，已在本组收尾期间转绿）。

### 3.3 施工中撞到的另外三条产品/夹具前置（均已就地解决，记录在案）

| # | 现象 | 处理 |
|---|---|---|
| 1 | gateway 只支持 ClaudeCode / Codex / Pi / KimiCode 四家真实 provider；替身是 `Fake` ⇒ `provider_unsupported_for_gateway_launch` | Main 授权 ①：把 role provider 改挂真实 provider（夹具改动） |
| 2 | `ProviderAvailabilityGate::ensure_available` 对**健康快照里没有的 provider** 报 `provider health entry is missing` ⇒ Pi 用例的替身根本没被 `start` | helper 作者把 `always_available_gate()` 的健康快照扩到四家 |
| 3 | gateway 对 Codex/KimiCode 的「用户级 workspace trust」门读真实 HOME | **实测不触发**：helper 的 `build_gateway` 走 `LogicalCodebaseProviderGateway::with_audit(...)`，该构造里 `trust: None` ⇒ 集成夹具下 trust 门短路。Codex 用例实测全绿 |

---

## 4. 断言改动说明（唯一一条）

### 4.1 `execute_coding_persists_coder_role_run_and_full_output_chat_entry`

```diff
-    assert_eq!(metadata["provider"], "fake");
+    assert_eq!(metadata["provider"], "claude_code");
```

**为什么必须改**：该断言恰好钉住 provider 身份，而 gateway 不支持 Fake（§3.3 #1）。
Main 授权把替身改挂真实 provider（ClaudeCode）；chat entry 的 `metadata["provider"]`
取的是**实际跑的那个 provider**（`coding.rs` 的 `provider_name: &coder_provider`）
⇒ 期望值随夹具同步。

**语义不变**：断言仍是「chat entry 记录了实际执行的那个 provider」的精确相等，
不是放宽或删除。**若 Main 认为该字面量不可动 ⇒ 回退本条即可**（该用例会回到
`coding_run_lease_unknown …` 的已知红，见 §3.2 表末行）。

其余 27 条用例：**零** `assert_*` 删除、零期望值改动、零测试名改动、零 `#[ignore]`。

---

## 5. 对 helper（`tests/it_product/context_builder.rs`）的依赖

本组**未改** `context_builder.rs`（只读）。用到的公开件：

| API | 用途 |
|---|---|
| `seed_logical_authority(paths, project_id, issue_id, root)` | 建 LC 权威（project + 真多仓 LC + 归属 issue + member/checkout/selection/policy/聚合索引） |
| `LogicalAuthority::snapshot(paths)` | 造与权威同源的冻结 `target_snapshot` |
| `build_gateway(paths, project_id, lc_id, registry)` | 组装 LC provider gateway |
| `GatewayBridgedAdapter::new(inner, provider_type)` | run_streaming 桥（替身只实现 `run_streaming`） |
| `GatewayValidatedStartAdapter { inner, provider_type }` | start 桥（替身只实现 `start`；`ProviderEvent` 原样透传） |
| `GatewayExactInputAdapter { inner }` | exact 桥（A1 后取**真实** `StreamingProviderInput`） |

**我向 helper 作者提的需求（均已实现）**：

1. `always_available_gate()` 的健康快照扩到 ClaudeCode / Codex / Pi / KimiCode
   （否则 Pi 用例在 availability 门就被拒，替身根本没被 `start`）——**已完成**。
2. provider_type 可配的 validated 桥 ——**已完成**（`GatewayValidatedStartAdapter`）。
3. exact input 桥（`into_parts` 公开后取真实 input）——**已完成**（`GatewayExactInputAdapter`）。

**helper API 面保持冻结**：本组全程只新增使用，未改任何既有签名。

---

## 6. 三条形态纪律（Phase 1 总结，本组实测全部成立）

1. **成员 checkout 必须是聚合根的直接子目录且是真实 git 仓**
   ⇒ helper 的 `{root}/aggregate-root/repository_0001`（真 git init + 首次提交）满足。
   逻辑 target 的 provider run 会采跨仓基线（`capture_cross_target_baseline`），
   裸目录会 `cross_target_store_failure`。
2. **LC 权威必须先于 attempt 建立**
   ⇒ 本组统一顺序：`seed_lc_authority` → `create_attempt(create_input_with_snapshot)`。
   反序（先 attempt 后权威）会把 project id 分配成 `project_0002`
   （attempt 落盘自建 `projects/project_0001/…`），admission 读不到归属
   ⇒ `target_snapshot_identity_drifted`（P2B 在 part_13 实测踩到，本组直接照此顺序写）。
3. **manifest / policy / index 必须落在同一个 `for_lc(lc_id)` 子树**（helper 已保证）。

---

## 7. 被改文件行数

| 文件 | + | − |
|---|---|---|
| `tests/it_product/product_coding_workspace_engine/part_03.rs` | 129 | 27 |
| `tests/it_product/product_coding_workspace_engine/part_04.rs` | 330 | 81 |
| `tests/it_product/product_coding_workspace_engine/part_05.rs` | 204 | 52 |
| `tests/it_product/product_coding_workspace_engine/part_06.rs` | 77 | 21 |
| **合计** | **740** | **181** |

新增的公共块（part_03.rs 顶部，本组 4 个文件共用）：
`p3_engine_with_logical_gateway(...)` —— 替身同时交给引擎方法（断言捕获面）与
gateway registry（真实 spawn 面，两处同一实例），并把 LC provider gateway 注入引擎。

`src/**`、`web/**`、其他 `tests/it_*`、`tests/it_product/context_builder.rs`、
`tests/it_product/product_coding_workspace_engine/part_01.rs`、`openspec/**`：
**零改动**（context_builder.rs 与 part_01.rs 的改动是 helper 作者做的）。

---

## 8. Concerns

1. **唯一的期望值改动**（§4.1）需要 Main 追认；若不可动，回退该条即可，
   代价是 1 条用例回到已知红。
2. **`GatewayBridgedAdapter` 的 `output_schema` 是「按 prompt 复原」**（helper 的
   `gateway_bridged_output_schema`），不是引擎真值。`part_06` 的
   `execute_code_review_prompt_includes_diff_work_item_rules_and_role_provider`
   断言的 `input.output_schema == "coding_workspace_code_review_json"` 因此是
   **复原值**，不是引擎下发值 —— 该断言的强度弱于同组的 prompt 内容断言。
   （它_web 是同款口径，Main 已认可。）
3. **`GatewayValidatedStartAdapter` 重建的 `StreamingProviderInput`** 里
   `permission_mode` / `timeout_secs` / `resume_provider_session_id` 是**中性默认**，
   不是引擎真值 —— 断言这三个字段的用例**必须**改用 `GatewayExactInputAdapter`
   （A1 已公开 `into_parts`）。本组已按此分流，无「假绿」。
4. **`Fake` provider 在 gateway 侧永久不可用** ⇒ 任何断言 provider 身份为 `fake`
   的用例都必须改挂真实 provider（Main 授权 ①）。it_product 里可能还有同类
   （part_01 的 group final review 一族），建议 Main 统一收口。
5. **中途的一次误操作已修复**：我在批量改写 validated 桥时脚本的括号定位差一位，
   把 4 个文件写坏（`Arc::new((stub),`）。已用只读 `git show HEAD:<path>` 取回
   HEAD 版本后重跑全部迁移脚本修好（**未做任何 git 写操作**：无 add/commit/stash/
   checkout/reset），并复核了最终 diff（除 §4.1 外无断言删除）。
6. **并发**：本组施工期间 part_08 / part_12 / part_13 / part_17 等文件由其他 worker
   并行改动（part_13 一度编译不过 `E0508`，part_01 一度编译不过 `E0609`，
   均由其归属者修复）；本组只碰 part_03..06，未触碰他人文件。
