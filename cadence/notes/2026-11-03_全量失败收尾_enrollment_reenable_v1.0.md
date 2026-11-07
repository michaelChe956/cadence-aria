# 全量失败收尾：automation_enrollment_tests_reenable.inc.rs 三条（v1.0）

- 日期：2026-11-03
- Worker：FixEnrollmentreenable3（glm53-task）
- Worktree：`.worktrees/bugfix-b-11-02`
- 被改文件（唯一）：`src/web/handlers/automation_enrollment_tests_reenable.inc.rs`，751 → **783 行**（+32：helper 27 行 + 3 条用例各 1 行调用 + 注释归属）
- 结论：**3/3 转绿，0 条需改实现**；断言逐字未动；未改生产代码 / spec / Plan。

## 共同根因（三条同源）

三条用例都经 `seed_enabled_enrollment()`（= `seed_fixture(1, true)` + 首次 enable）后发起「换 `author_provider=claude_code`」或其组合的写入请求。夹具默认 options 全为 `fake`（`enrollment_body`，test_support.rs:554-555），Fake 走测试运行豁免；一旦换成 `claude_code`，Enable 前的角色链同源预检要求 durable capability 证据，而夹具从未播种 ⇒ `automation_role_chain_unsupported`（violations[].reason_code=`provider_capability_launch_not_confirmed`，roles plan_author/coder）经 `ApiError::validation_with_details` 返回 **422**，请求被拦在 store CAS 冲突判定之前——被测的 409 分流逻辑根本没执行。

旁证：同 tests mod 的 `automation_reenable_supersede_tests.inc.rs:639-655`（`reenable_with_new_provider_is_accepted_in_one_step`）对同场景已用「resolve LC → ensure_bootstrap → seed Confirmed(ClaudeCode)」三步物化能力证据。本次改动即把该已验证模式函数化后接入本文件。

## 改法（一处 helper + 三行调用，逐字照抄已验证模式）

新增私有 helper `seed_claude_code_launch_capability(&fixture)`（reenable.inc.rs:610-636）：

1. `RepositoryAuthorityResolver::resolve_for_issue` 解析 issue 归属 LC 的权威 id；
2. `ProviderCapabilityStore::for_lc(&paths, &lc_id).ensure_bootstrap(PROJECT_ID)`；
3. `seed_confirmed_capability_records(&paths, PROJECT_ID, Some(&lc_id), [ProviderRefType::ClaudeCode])`（共享支持模块，pub(crate)）。

三条用例在 `seed_enabled_enrollment().await` 之后各加一行调用（:645 / :683 / :762）。

## 逐条记录

### 1) enrollment_write_with_changed_provider_while_running_reports_payload_immutable（原 :624）

- 根因：见上（capability 缺 → 422 拦截）。
- 改法：加 helper + :645 一行调用。
- 改前→改后：`left: 422 right: 409`（panic 消息 code=`automation_role_chain_unsupported`）→ 请求落到 CAS 层，409 + `automation_enrollment_payload_immutable`，断言原样通过。
- 红→绿输出：改前 `assert_eq!` failed (422 vs 409)；改后 `test ...reports_payload_immutable ... ok`（`--lib` 窄跑，1 passed）。
- concerns：无。

### 2) payload_immutable_and_revision_stale_report_distinct_codes（原 :667）

- 根因：同上；第一个请求（改 provider）先被 422 拦，`first_status` 断言即红。
- 改法：:683 一行调用。第二个请求（stale revision，未换 provider）本就不需要 capability。
- 改前→改后：`left: 422 right: 409` → 两请求分别 409 `automation_enrollment_payload_immutable` / 409 `automation_enrollment_revision_stale`，`assert_ne!` 通过。
- 红→绿输出：改后 `test ...distinct_codes ... ok`。
- concerns：无。

### 3) conflict_kind_survives_the_store_to_http_hop（原 :743）

- 根因：同上；且本条断言判别位经 store→HTTP 传递不丢，第一个请求被 422 拦后 payload code=`automation_role_chain_unsupported` 与期望 `automation_enrollment_payload_immutable` 不符。
- 改法：:762 一行调用（未用 command_id 绕账本重放面——按任务约束必须走到 CAS 的 PayloadImmutable 分支）。
- 改前→改后：`left: String("automation_role_chain_unsupported") right: "automation_enrollment_payload_immutable"` → 两断言分别命中 `automation_enrollment_payload_immutable` / `automation_enrollment_revision_stale`。
- 红→绿输出：改后 `test ...conflict_kind_survives_the_store_to_http_hop ... ok`。
- concerns：无。

## 验证

- 窄跑命令（flock /tmp/aria-cargo.lock，未并行第二个 cargo）：
  1. `cargo test --locked --lib enrollment_write_with_changed_provider_while_running_reports_payload_immutable` → `1 passed; 0 failed`
  2. `cargo test --locked --lib -- payload_immutable_and_revision_stale_report_distinct_codes conflict_kind_survives_the_store_to_http_hop enrollment_write_with_stale_revision_reports_revision_stale binding_write_conflict_keeps_legacy_conflict_code terminate_confirm_with_stale_revision_keeps_legacy_conflict_code` → `5 passed; 0 failed`（目标③⑥ + 同 helper 家族②④⑤回归全绿）
- 未跑本族之外的全量（按约束留给 Main 统一）。

## 约束遵守

- 只改 `automation_enrollment_tests_reenable.inc.rs`；未碰 `automation_enrollment_tests_http.inc.rs`（22:37 被 FixEnrollmentHttpTwo 改过，已按 mtime 避让只读参考）、共享支持模块、`logical_test_fixture.rs`、`large_file_guard.rs`。
- 断言逐字未改（diff 全部为新增行，零删改）；未改生产实现、未造 reason_code、未动 Legacy 变体、未改 spec/Plan、未删测试。
- 未执行 git add/commit/stash/checkout。
- mtime 检查：目标文件 10:34:32（>12h 前）后开工；`automation_enrollment_test_support.rs` 20:42:04（~2h 前）仅读未写。

## 遗留 / concerns

- 无需改实现的条目：0。
- `large_file_guard.rs` 登记数字未动（按约束由 Main 统一调整；本文件 783 行）。
