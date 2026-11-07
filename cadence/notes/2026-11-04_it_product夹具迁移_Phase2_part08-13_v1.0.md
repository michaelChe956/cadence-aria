# it_product 夹具迁移 · Phase 2（part_08 / part_09 / part_12 / part_13）· v1.0

> 日期：2026-11-04
> 执行：MigrateItProductP2B（worker）
> 范围：`tests/it_product/product_coding_workspace_engine/part_08.rs` / `part_09.rs` / `part_12.rs` / `part_13.rs`
> **未改任何 `src/**` / `web/**` / `context_builder.rs` / 其他 `tests/it_*` / `openspec/**`。**

---

## 0. 一句话结论

本组 4 个文件里共 **12 个失败用例**（part_08 六条 + part_13 五条 + part_12 夹具承载的一条），
判定为：

- **P3（LC 权威 + 快照即可解）= 9 个** → 全部迁绿；
- **P2（issue 维 shared worktree 族，块3 已下线该路径）= 3 个** → 本阶段只登记；
  其中 part_13 的两条在「P2 改指仓维」并行推进后，已由本组补齐**夹具侧的仓维锁 seeding**
  转绿 1 条、剩 1 条只差取消注释（由该 worker 收口），part_12 承载的那条已按 Main 口径
  派给 `UnblockIntoPartsAndRepointP2`；
- **P1 = 0 个** → Phase 1 判定的 P1「provider-run 族」在本组**已被共享 helper 的桥接能力解掉**
  （`GatewayBridgedAdapter` 升级为具名 struct，`provider_type` / `output_schema` 均可复原），
  原本被归为 P1 的 `execute_group_final_review_prompt_includes_request_commit_diff_and_function_context`
  也能迁绿。

`part_09.rs` **无失败用例**（纯夹具文件，零失败用例）⇒ **零改动**。
`part_12.rs` 的唯一改动 = 删除全仓零引用的 `group_engine_with_two_units`（死代码，见 §4），
其余夹具一行未改。

---

## 1. 逐用例判定与迁移动作

### 1.1 part_08.rs（6 个失败用例 → 6 个迁绿）

| # | 用例 | 原错误签名 | 类别 | 迁移动作 | 结果 |
|---|---|---|---|---|---|
| 1 | `review_request_blocks_when_only_runtime_artifacts_changed` | `Store(Io("target_snapshot_identity_drifted"))` | P3 | `seed_lc_authority` + `create_input_with_snapshot` | ✅ 绿 |
| 2 | `execute_review_request_keeps_push_started_when_failed_push_cannot_query_remote` | `Store(Io("target_snapshot_identity_drifted"))` | P3 | 同上 | ✅ 绿 |
| 3 | `execute_group_final_review_persists_review_and_waits_for_final_confirm` | `ProviderStream("coding_run_lease_unknown: 租约状态未知，已停等：repository routing load failed: product_store_not_found: issue issue_0001")` | P3 | LC 权威 + group attempt 冻结快照 + `internal_reviewer` 改挂 ClaudeCode + `GatewayBridgedAdapter` + registry + `with_logical_provider_gateway` | ✅ 绿 |
| 4 | `execute_group_final_review_blocked_opens_human_gate` | 同上 | P3 | 同上 | ✅ 绿 |
| 5 | `execute_group_final_review_prompt_includes_request_commit_diff_and_function_context` | 同上 | **P1→P3**（helper 升级后解锁） | 同上，且 bridge 侧 `provider_type = Codex`、`output_schema` 按 prompt 复原 | ✅ 绿 |
| 6 | `handle_final_confirm_completes_when_required_plan_is_satisfied` | `Store(NotFound { kind: "issue", id: "issue_0001" })` | P3 | `seed_lc_authority`（建 issue 记录）+ `create_input_with_snapshot` | ✅ 绿 |

### 1.2 part_09.rs（0 个失败用例）

纯夹具文件（`coding_store_with_attempt` / `final_confirm_attempt` / `failed_attempt` /
`dirty_failed_attempt` / `sample_review_request` / 6 个 provider 替身），**不含任何
`#[test]` / `#[tokio::test]`** ⇒ 本组零失败用例、**零改动**。

> 注：part_09 的 `SessionInputCapturingProvider` 断言 `permission_mode` /
> `timeout_secs` / `resume_provider_session_id` —— 这三个字段在 validated 路径下
> 拿不到（`ValidatedStreamingProviderInput::into_parts()` 是 `pub(crate)`），
> 属产品侧 A1 缺口，**不是本组用例**，未动。

### 1.3 part_12.rs（1 个失败用例 → P2，本阶段不动）

| 用例 | 归属 | 类别 | 阻塞签名 |
|---|---|---|---|
| `group_final_confirm_releases_transferred_shared_worktree_lock_by_owner` | 测试本体在 part_01.rs:499，**夹具**在 part_12.rs 的 `group_attempt_waiting_for_final_confirm()` | **P2** | `Store(NotFound { kind: "issue", id: "issue_0001" })`（补齐 LC 权威后转 `LegacySharedWorktreePresent("project_0001/issue_0001")` —— 块3 删掉 `IssueSharedWorktreeRoute::Legacy` 臂后 issue 维记录不再被引擎读写） |

任务书明确标注该用例属 P2「不要动」⇒ 本组**不改该夹具**。
（后续按 Main 口径，该用例连同 part_12 的 `group_attempt_waiting_for_final_confirm()`
仓维化，已统一派给 `UnblockIntoPartsAndRepointP2` 一并改 part_01 + part_12；
本组对 part_12 的唯一改动是删除零引用死代码，见 §4。）

### 1.4 part_13.rs（5 个失败用例 → 3 绿 + 2 P2）

| # | 用例 | 原错误签名 | 类别 | 迁移动作 | 结果 |
|---|---|---|---|---|---|
| 1 | `completing_group_unit_marks_current_unit_completed_and_next_running` | `Store(NotFound { kind: "issue", id: "issue_0001" })` | P3 | 新增 `group_engine_with_two_units_under_lc_authority()`（LC 权威 + 冻结快照补挂）+ 仓维 shared worktree 锁 | ✅ 绿 |
| 2 | `completing_last_group_unit_enters_review_request_stage` | `Store(NotFound { kind: "issue", id: "issue_0001" })` | P3 | 新增 `group_engine_with_last_running_unit_under_lc_authority()` | ✅ 绿 |
| 3 | `completing_group_units_publishes_revisions_without_legacy_handoff_artifacts` | `Store(NotFound { kind: "issue", id: "issue_0001" })` | P3 | 同 #1 + 仓维锁 | ✅ 绿 |
| 4 | `completing_group_unit_moves_issue_shared_lock_to_next_unit` | `Store(NotFound { kind: "issue", id: "issue_0001" })` | **P2** | 断言读 **issue 维** `get_issue_shared_worktree`，而引擎只写仓维记录 ⇒ 断言永远不成立 | ❌ 仍红（登记） |
| 5 | `group_final_confirm_completes_attempt_after_all_units_completed` | `Store(NotFound { kind: "issue", id: "issue_0001" })` | **P2** | 同 #4：断言 `shared.current_active_work_item_id == None` / `last_completed_work_item_id == Some("work_item_0002")` 读的是 issue 维记录 | ❌ 仍红（登记） |

> **后续进展（见 §3.3）**：#4 / #5 的**读取维度**已由 `RepointP2ToRepoDimPart13` 改到仓维
> （`get_repo_shared_worktree(..., authority.member_id)`，不弱化断言、只改读取维度）；
> 本组随后补上**夹具侧的仓维锁 seeding** ⇒ #5 实测转绿 ✅；#4 只差把同一处 seeding 取消注释
> （该 worker 在实时编辑同一段，本组避免抢写，精确行号与实证已 IRC 同步）。

---

## 2. 关键迁移形态（本组新学到的三条）

1. **⚠️ LC 权威必须先于 coding attempt 建立**（本组实测踩到，最隐蔽的一条）：
   `ProjectStore::create` 的 project id 按 `projects/` 目录数分配，而 **coding attempt
   落盘本身就会建 `projects/project_0001/issues/issue_0001/`** ⇒ 先建 attempt 再建权威
   会把 project 分配成 `project_0002`，权威与 attempt 的 project 身份错位 ⇒
   admission 的 `RepositoryAuthorityResolver::resolve_for_issue` 走
   `ProjectStore::get("project_0001")` → `NotFound` → 返回 `Ok(None)` ⇒ routing 恒
   `FailClosed` ⇒ **`target_snapshot_identity_drifted`**（与 Phase 1 报告里
   「issue id 漂到 issue_0002」同源的目录序分配陷阱，只是发生在 project 维度）。
   实测证据（临时探针，已删除）：
   `resolve_for_issue(...) == Ok(None)`，而 `RepositoryRouting::load_for_issue(...)` 同刻
   返回 `Logical` —— 两者走的是**不同的 project 身份解析路径**，只有 admission 用前者。
   ⇒ 正确顺序：**LC 权威 → work item + plan → attempt（带冻结快照）→ coding units**。

2. **仓维 shared worktree 锁是 group 推进链的硬前置**：
   `advance_to_next_group_unit` 的锁转移走
   `LifecycleStore::transfer_repo_worktree_lock(project, issue, logical_repository_id, …)`，
   该函数 `read_repo_worktree(...)` 对缺失记录直接
   `NotFound { kind: "repo_shared_worktree" }`（实测落到 IO 面上是
   `Store(Io("open …/shared-worktrees/<uuid>.json: No such file or directory (os error 2)"))`）
   ⇒ 夹具必须先 `upsert_repo_shared_worktree` + `try_acquire_repo_worktree_lock`
   （owner = attempt id、active work item = 当前 unit）。
   与此相对，`validate_attempt_issue_shared_worktree_lock_if_present` 对**缺记录**是
   `Ok(())`（"if present" 语义）⇒ 不建锁记录不会在入口 fail-closed，而是**到锁转移那一步才炸**。
   ⇒ 这条断言是**锚在真实仓维锁记录上**的（缺记录 ⇒ 直接 IO 失败），不是被 fail-closed 蒙混。

   **负控实证**（`RepointP2ToRepoDimPart13` 带 md5 校验重跑，本组复核结论一致）：
   - 负控 A（去掉 test1 的仓维 seeding）⇒ **红**：
     `complete unit: Store(Io("open …/issues/issue_0001/shared-worktrees/<uuid>.json: No such file or directory (os error 2)"))`
   - 负控 B（去掉终局夹具里的仓维 seeding）⇒ **红**：panic `existing shared worktree`
     （`get_repo_shared_worktree` 返回 `None`）
   ⇒ 两侧都证明「仓维 shared worktree 记录是 group 推进锁转移的**必要条件**」，
   缺记录时既不是 fail-closed、也不是断言被弱化蒙混。
   （注：首次负控曾报「2 passed」，那是并发写覆盖导致编译到了别人版本，结论已作废；
   上述 A/B 是带 md5 校验重跑的结果。）

3. **gateway 只认真实 provider**：`ProviderRef::from_provider_name(Fake)` 直接
   `provider_unsupported_for_gateway_launch` ⇒ 逻辑 target 的 provider 角色必须挂
   ClaudeCode / Codex / Pi / KimiCode。本组把 **internal reviewer 角色**改挂
   ClaudeCode（断言不涉及 provider 身份，setup 变更不影响任何期望值）。
   另：helper 的 `build_gateway` 走 `with_audit(...)` ⇒ `trust: None`
   ⇒ Codex/KimiCode 的「用户级 workspace trust」门在集成夹具里**不触发**
   （生产构造器注入了 trust source，那里才成立）。

---

## 3. 验证输出

### 3.1 本组窄测（9 个 P3 用例）

```
flock /tmp/aria-cargo.lock cargo test --locked --test it_product -- \
  product_coding_workspace_engine::execute_group_final_review_persists_review_and_waits_for_final_confirm \
  product_coding_workspace_engine::execute_group_final_review_blocked_opens_human_gate \
  product_coding_workspace_engine::execute_group_final_review_prompt_includes_request_commit_diff_and_function_context \
  product_coding_workspace_engine::completing_group_unit_marks_current_unit_completed_and_next_running \
  product_coding_workspace_engine::completing_last_group_unit_enters_review_request_stage \
  product_coding_workspace_engine::completing_group_units_publishes_revisions_without_legacy_handoff_artifacts
```
⇒ **6 passed / 0 failed**。
另一批 3 个 part_08 用例（`review_request_blocks_when_only_runtime_artifacts_changed` /
`execute_review_request_keeps_push_started_when_failed_push_cannot_query_remote` /
`handle_final_confirm_completes_when_required_plan_is_satisfied`）早前一轮单独跑
⇒ **3 passed / 0 failed**。

### 3.2 全量 it_product

```
flock /tmp/aria-cargo.lock cargo test --locked --test it_product
```
⇒ 迁移完成时：**test result: FAILED. 193 passed; 18 failed**
⇒ 并行 worker（P2A / P2 改指组）收口后复跑：**test result: FAILED. 202 passed; 9 failed**
⇒ **全部收口后（本组独立复跑，20:36）：**
```
test result: ok. 211 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.94s
```

阶段基线对照：

| 时点 | passed | failed |
|---|---|---|
| Phase 1 起始基线 | 153 | 58 |
| 本组迁移完成 | 193 | 18 |
| 并行 worker 收口后 | 202 | 9 |
| **全部收口（本组复跑）** | **211** | **0** |

`cargo fmt --check` ⇒ clean。

### 3.3 全量剩余失败用例名：**0 条**（211 / 0）

全部收口后**无剩余失败用例**。下表留档「202 / 9 口径下最后 9 条」的归属与原始阻塞签名，
以及各条的收口方（便于追溯「谁把哪条从红转绿」）：

| # | 用例（202/9 口径） | 归属 | 类别 | 原始阻塞签名 | 收口方 |
|---|---|---|---|---|---|
| 1 | `completing_group_unit_moves_issue_shared_lock_to_next_unit` | **part_13（本组）** | P2 | `complete unit: Store(Io("open …/issues/issue_0001/shared-worktrees/8ce5948f-….json: No such file or directory (os error 2)"))` | 读取维度：`RepointP2ToRepoDimPart13`；夹具侧仓维锁 seeding：**本组** |
| 2 | `group_final_confirm_completes_attempt_after_all_units_completed` | **part_13（本组）** | P2 | `existing shared worktree`（仓维记录不存在） | 读取维度：`RepointP2ToRepoDimPart13`；夹具侧仓维锁 seeding：**本组** |
| 3 | `group_final_confirm_releases_transferred_shared_worktree_lock_by_owner` | part_01（夹具在 part_12） | P2 | `Store(NotFound { kind: "issue", id: "issue_0001" })` | `UnblockIntoPartsAndRepointP2`（part_01 + part_12 一并改仓维） |
| 4 | `aborted_group_attempt_releases_transferred_shared_worktree_lock_by_owner` | part_01 | P2 | `LegacySharedWorktreePresent("project_0001/issue_0001")` | `UnblockIntoPartsAndRepointP2` |
| 5 | `failed_group_attempt_releases_transferred_shared_worktree_lock_by_owner` | part_01 | P2 | `LegacySharedWorktreePresent("project_0001/issue_0001")` | `UnblockIntoPartsAndRepointP2` |
| 6 | `dirty_group_delete_keeps_transferred_shared_worktree_lock` | part_01 | P2 | `dirty group delete keeps the transferred lock: ()` | `UnblockIntoPartsAndRepointP2` |
| 7 | `failed_attempt_releases_issue_shared_worktree_lock` | part_01 | P2 | `handle failed: LegacySharedWorktreePresent("project_0001/issue_0001")` | `UnblockIntoPartsAndRepointP2` |
| 8 | `final_confirm_releases_issue_shared_worktree_lock` | part_01 | P2 | `final confirm: LegacySharedWorktreePresent("project_0001/issue_0001")` | `UnblockIntoPartsAndRepointP2` |
| 9 | `dirty_shared_worktree_blocks_lock_release_and_next_work_item` | part_01 | P2 | `dirty worktree diagnostics must not replace terminal failure: LegacySharedWorktreePresent("project_0001/issue_0001")` | `UnblockIntoPartsAndRepointP2` |

### 3.3.1 本组迁移完成时（193 / 18）的 18 条快照 —— 其后全部收口

| 用例 | 归属 | 类别 | 原始阻塞签名 | 收口方 |
|---|---|---|---|---|
| `final_confirm_owner_conflict_does_not_complete_attempt` | part_17 | P2 | `assertion failed: matches!(error, …Conflict { kind: "issue_worktree_lock_owner", .. })` | `RepointP2ToRepoDimPart17` |
| `group_unit_owner_conflict_does_not_advance_unit_or_attempt` | part_17 | P2 | 同上 | `RepointP2ToRepoDimPart17` |
| `coding_coder_run_resumes_previous_coder_provider_session` | part_01 | P1 | `first coding run: ProviderStream("logical_provider_gateway_required")` | helper 作者（Fake → 真实 provider + gateway） |
| `coding_coder_rework_with_resume_uses_delta_prompt` | part_01 | P1 | 同上 | 同上 |
| `group_next_work_item_coder_run_does_not_resume_previous_unit_session` | part_01 | P1 | `execute second unit coding: ProviderStream("logical_provider_gateway_required")` | 同上 |
| `coding_code_reviewer_run_records_session_without_resume` | part_03 | P1 | `code review provider run: ProviderStream("coding_run_lease_unknown: 租约状态未知，已停等：repository routing load failed: product_store_not_found: issue issue_0001")` | P2A |
| `coding_internal_reviewer_uses_fresh_provider_session` | part_03 | P1 | `internal reviewer provider run: ProviderStream("coding_run_lease_unknown: … product_store_not_found: issue issue_0001")` | P2A |
| `execute_coding_persists_coder_role_run_and_full_output_chat_entry` | part_04 | P1 | `execute coding: ProviderStream("coding_run_lease_unknown: … product_store_not_found: issue issue_0001")` | P2A |
| `pi_role_with_supervised_mode_normalized_to_auto` | part_04 | P1 | `execute coding: ProviderStream("coding_run_lease_unknown: … product_store_not_found: issue issue_0001")` | P2A |
| 本组 9 条 P3 + 3 条 P2 | part_08 / part_13 / part_12 | P3/P2 | 见 §1 逐用例表 | **本组** |

> 本组**从未**产生 P1 红灯：Phase 1 判定的 part_08 P1
> （`execute_group_final_review_prompt_includes_request_commit_diff_and_function_context`）
> 已被 helper 的 `GatewayBridgedAdapter`（可复原 `provider_type` + `output_schema`）解锁并转绿。

### 3.4 编译与格式

```
flock /tmp/aria-cargo.lock cargo fmt --check     ⇒ exit 0，无输出
```
（`part_08.rs` / `part_13.rs` 是 `include!` 片段，`cargo fmt` 不覆盖它们；
按 Phase 1 的纪律**未**对 `.inc.rs`/片段单独跑 rustfmt。）

全量 it_product 测试二进制**编译通过**（本组无编译错误/警告）。

---

## 4. 被改文件行数

| 文件 | 变化 | 状态 |
|---|---|---|
| `tests/it_product/product_coding_workspace_engine/part_08.rs` | **+140 / −28** | 已随 `8fa977b` 入库 |
| `tests/it_product/product_coding_workspace_engine/part_13.rs` | **+162 / −3**（本组）<br>另叠加 `RepointP2ToRepoDimPart13` 的 P2 改指仓维改动 | 已随 `8fa977b` 入库 |
| `tests/it_product/product_coding_workspace_engine/part_09.rs` | **0** | 无失败用例，未动 |
| `tests/it_product/product_coding_workspace_engine/part_12.rs` | **+5 / −62** | 未入库（工作区） |
| 本组合计 | **+307 / −93** | |

> part_12 的净变化是**删除**已无引用的 `group_engine_with_two_units` 夹具
> （原定义 62 行 → 换成 5 行去向注释）：part_13 / part_17 已各自改用自带
> 「LC 权威 + 冻结快照」的仓维夹具 ⇒ 该函数全仓零引用 ⇒ `dead_code` 告警会阻断
> `cargo clippy --all-targets -- -D warnings`。
> `group_engine_with_last_running_unit` 仍被 part_01 / part_13 / part_20 使用，**保留未动**。
> 验证：`flock /tmp/aria-cargo.lock cargo build --locked --tests` ⇒ **零 warning / 零 error**。

---

## 5. Concerns

1. **未弱化任何断言**：全程零 `assert_*` 改动、零期望值改动、零测试名改动、零 `#[ignore]`。
   所有改动都是「setup 补齐」（LC 权威 / 冻结快照 / 仓维锁 / gateway 注入 / 角色 provider 改挂）。
2. **共享夹具未动**：part_12 的 `group_engine_with_last_running_unit` /
   `completed_group_attempt_with_handoff_revisions` /
   `group_attempt_waiting_for_final_confirm` / `seed_authoritative_group_*_fixture`
   **一行未改**（它们同时被 part_17 / part_20 / part_01 使用，跨 owner 改动会打架）。
   本组改为在 part_13 内部新建 `group_engine_under_lc_authority` 夹具本体 +
   `*_under_lc_authority` 两个变体 + `seed_repo_shared_worktree_lock` 助手。
   **唯一的 part_12 改动**：删除全仓零引用的 `group_engine_with_two_units`（见 §4）。
3. **与并行 worker 的交叉**：
   - `context_builder.rs` 的 `GatewayBridgedAdapter` 在本组施工期间由 helper 作者
     （`MigrateItProductFixtures`）从 tuple struct 升级为具名 struct（签名变更）；
     本组 part_08 已按新签名（`::new(inner, provider_type)` / 具名字段 + 显式
     `output_schema`）落地。
   - `part_03.rs` / `part_05.rs`（P2A 负责）在施工期间多次处于「编译不过」状态
     （括号不配对 / `store` move 后再用），**整个 it_product 测试二进制因此编译失败**，
     本组窄测被迫等待；已两次通过 IRC 同步 P2A。
   - `UnblockIntoPartsAndRepointP2`（P2 改指多仓维度）要改 part_12 的
     `group_attempt_waiting_for_final_confirm`；已确认与本组 part_12 零重叠，
     并把 part_13 的读取维度需求同步给对方。
4. **helper 能力缺口（本组遗留）**：`GatewayBridgedAdapter` 复原的
   `StreamingProviderInput` 里 `permission_mode` / `timeout_secs` /
   `resume_provider_session_id` 仍是中性默认（validated 公开投影只有 role/cwd/prompt）
   ⇒ 断言这三个字段的替身（如 part_09 的 `SessionInputCapturingProvider`）仍不可用。
   需产品侧把 `ValidatedStreamingProviderInput::into_parts` 提为 `pub`（Phase 1 报告 A1）。
5. **helper 的 `gateway_bridged_output_schema(prompt)` 判别顺序有歧义**（不影响本组断言，
   但对后续 worker 是个坑）：group final review 的 prompt 里**同时**含
   「 Reviewer Work Item Projection」段 ⇒ 该函数的**第一个分支**先命中，
   `output_schema` 复原成 `coding_workspace_code_review_json`（而不是
   `coding_workspace_internal_pr_review_json`）。
   本组的处理：**在 part_08 显式钉住 `output_schema: Some("coding_workspace_internal_pr_review_json")`**，
   不改任何期望值、不改 helper。若后续 worker 也撞到，建议 helper 作者把
   group final review 的判别提到第一位。
6. **给 helper 作者的两条新增需求（若要进一步减少 P1）**：
   - `GatewayBridgedAdapter` 若能同时复原 `permission_mode` / `timeout_secs` /
     `resume_provider_session_id`（需要产品侧 A1 `into_parts` 提为 `pub`），
     part_01 的 `coding_coder_run_resumes_previous_coder_provider_session` /
     `coding_coder_rework_with_resume_uses_delta_prompt` /
     `group_next_work_item_coder_run_does_not_resume_previous_unit_session`
     三条（现签名 `ProviderStream("logical_provider_gateway_required")`）才能转绿。
   - 本组**不需要** helper 新增函数：现有 `seed_logical_authority` /
     `LogicalAuthority::snapshot` / `attach_target_snapshot` / `build_gateway` /
     `GatewayBridgedAdapter` 已足够。
