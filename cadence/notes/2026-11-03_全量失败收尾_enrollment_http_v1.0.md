# 全量失败收尾：enrollment HTTP 两条（FixEnrollmentHttpTwo）

- 工作树：`.worktrees/bugfix-b-11-02`
- 唯一被改文件：`src/web/handlers/automation_enrollment_tests_http.inc.rs`（629 → 646 行，+17）
- 验证：`flock /tmp/aria-cargo.lock cargo test --locked --lib -- automation_enrollment_http_put_is_idempotent_and_conflicts_on_divergence automation_enrollment_http_put_rejects_zero_target_and_mismatched_member_set`
  → `2 passed; 0 failed; 0 ignored; finished in 0.06s`（编译 1m51s）
- 未改断言、未改生产实现、未碰 reenable/support/fixture/large_file_guard、无 git add/commit。

## ① automation_enrollment_http_put_is_idempotent_and_conflicts_on_divergence（:71 红）

### 根因（实测，非 Main 预判的 capability 缺失）
红点在 :71 而非更早：PUT#1/#2（200/幂等 200）与 :69 的 409 状态断言**全部已过**，
说明角色链预检在当前代码下对 `fake` provider 已放行，不存在
`provider_capability_launch_not_confirmed` 拦截（Main ①的 panic 描述来自其它族的旧状态）。

真实根因：REQ-WIGA-SWAP-02 拆分 ConflictKind 后，`issue_automation_store.rs`
`compare_and_set_inner` 的分支顺序为：

1. terminated 取代 / DefensiveStateMismatch
2. 命令账本重放面（请求带显式 `command_id` 且账本命中：同 payload digest → Unchanged；异 →
   `CommandLedgerPayloadMismatch`）
3. 幂等 Enable（enabled + payload 全等 + revision 匹配/缺省）
4. **PayloadImmutable（enabled + 任一 payload 字段漂移）**
5. RevisionStale（expected_revision ≠ 当前）

旧 wire（不带 `command_id`）的「异 options + expected_revision=99」命中分支 4 →
HTTP 层（`enrollment_api_error`）映射 `ConflictKind::PayloadImmutable` →
`enrollment_payload_immutable` 码，而测试期望的是其余 7 类回落的旧码
`automation_enrollment_conflict`（`details.current_revision=1`）。测试从未更新到
REQ-WIGA-02/09 的新语义，属于夹具仍停留在旧 wire 形态。

### 改法（仅夹具/请求体，断言逐字未动）
给 Enable 请求体补显式稳定 `command_id`（REQ-WIGA-09 人工入口留痕），使幂等/分歧判定
走进**命令账本重放面**（分支 2，先于 PayloadImmutable）：

- PUT#1：`enable["command"]["command_id"] = "enrollment_put_divergence_1"` → 首次落盘，
  账本记 digest（command_id+selection_key+source+options+target+expected_revision）。
- PUT#2：同 id 同 payload（含 expected_revision=null）→ digest 全等 → Unchanged 200，
  同 enrollment_id/revision（幂等语义保持）。
- PUT#3（divergent）：同 id + options.review_rounds=2 + expected_revision=99 → digest 不等
  → `CommandLedgerPayloadMismatch` → 回落旧码 `automation_enrollment_conflict`，
  `details.current_revision=1`（:78-79 断言原样命中）。
- 后续 disable（rev 1→2）与「旧 revision Enable → 409」用例体未动：disable 体无
  command_id；重开用 `enrollment_body` 新体（无 command_id）→ enabled=false → 跳过
  分支 3/4 → 分支 5 RevisionStale → 409（:90 只断言状态）✓。

### 红 → 绿
- 红（全量）：`left: String("automation_enrollment_payload_immutable")  right: "automation_enrollment_conflict"`（:71）
- 绿（窄跑）：`automation_enrollment_http_put_is_idempotent_and_conflicts_on_divergence ... ok`

### concerns（已 Main 复核结案）
- 该用例现在的「幂等」是命令账本重放幂等、「分歧」是账本 payload 不匹配——与用例名
  「is_idempotent_and_conflicts_on_divergence」及注释「异 options + 旧 revision → 409，
  details 携带当前 revision」语义一致，但不再覆盖「无 command_id 的旧 wire 异 payload →
  payload_immutable」分支。
- **Main 复核：无覆盖损失**——该分支仍由 reenable 文件 3 条用例钉住：
  `:614 enrollment_write_with_changed_provider_while_running_reports_payload_immutable`
  （断言 `code == "automation_enrollment_payload_immutable"`）、
  `:651 payload_immutable_and_revision_stale_report_distinct_codes`、
  `:743 conflict_kind_survives_the_store_to_http_hop`（同码断言）。
  ⇒ 结案，无需补测。

## ② automation_enrollment_http_put_rejects_zero_target_and_mismatched_member_set（:121 红）

### 404 层级定位结论（grep 路由与校验顺序实证）
PUT Enable 的校验顺序（`automation_enrollment.rs::validate_enrollment_scope`）：
`ensure_issue_exists`（issue 缺 → 404）→ `validate_confirmed_source_refs`（422）→
`resolve_automation_carrier`（载体判定）→ manifest/selection 缺失检查（422）→
canonical target 全等校验（422）→ 角色链预检。

`seed_fixture(0, true)` 自 REQ-ROUTE-01 起给 issue 写了必填归属
`logical_codebase_id = legacy_logical_codebase_id(PROJECT_ID)`，但 0 成员时
`seed_logical_codebase` 不会被调用 → 别名 LC **record.json 不存在**。于是：

`resolve_automation_carrier` → `RepositoryAuthorityResolver::resolve_for_issue`
（issue 带归属 → lc_id=Some(别名)）→ `resolve_logical`：`record_exists=false` →
`ProductStoreError::NotFound { kind: "logical_codebase" }` →
`product_store_api_error` → `logical_codebase_not_found` →
**error.rs:293 → 404**。即 404 出自**载体判定层（repository routing resolver 的 LC
record 直读）**，先于任何 422 校验——测试以为的「0 成员 → 校验层 422」根本没被执行到。
（另证：若把 issue 记录改成缺归属字段的「无归属老记录」，`read_issue_record` 会报
`InvalidRecord(issue_not_bound_to_logical_codebase)` → error.rs:178 映射 **400**，也不是
422——该路不通，弃用。）

### 改法（仅夹具，断言逐字未动）
`seed_fixture` 自己的注释已写明出路：「0 成员 fixture 由用例自行改写为无归属老记录
形态」已过时——正确形态是**补 0 成员权威布局**：在用例内调用共享支持模块（只读复用，
全限定路径，tests mod 的 use 语句在禁改文件内）：

```rust
super::super::automation_enrollment_test_support::seed_logical_codebase(&fixture.paths, &[]);
```

效果：project 级 manifest（0 成员）/selection（Explicit 空集）/repos.json/aggregate
index/policy 全落盘；PUT 时 `ProjectStore::get` 内置幂等 `migrate_legacy` 生成别名 LC
record 并镜像子树 → 载体解析成功（manifest=Some、selection=Some）→
`logical_repository_ids_for_preflight` 候选 = manifest∩selection = **0** →
`canonical_lc_enrollment_target` →
`invalid_scope("automation target requires at least one logical repository")` →
**422**、零写入（:128-129 原样命中）。第二段（2 成员 + 旧单成员 target → 422
`automation_enrollment_invalid_scope`）本来就在 canonical 全等校验层，红跑时因第一段
提前失败未执行，本次一并转绿。

### 红 → 绿
- 红（全量）：`left: 404  right: 422`（:121）
- 绿（窄跑）：`automation_enrollment_http_put_rejects_zero_target_and_mismatched_member_set ... ok`

### concerns
- `seed_logical_codebase(paths, &[])` 是空集参数的既有共享夹具（所有构造器均无
  非空校验，实测落盘成功）；若后续有人给 manifest/selection 加非空校验，此用例会
  panic 而非 422，届时需换「冷启动 LC（有 record 无 manifest）」形态（handler :391
  的 422 路径）。
- 用例注释已从过时的「Legacy 路由 → 422」改为当前语义（canonical 候选为空 → 422）；
  Legacy 变体未恢复、未自造 reason_code。

## 验证输出（唯一一次窄跑，flock 排队）

```
running 2 tests
test web::handlers::automation_enrollment::tests::automation_enrollment_http_put_is_idempotent_and_conflicts_on_divergence ... ok
test web::handlers::automation_enrollment::tests::automation_enrollment_http_put_rejects_zero_target_and_mismatched_member_set ... ok

test result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 5969 filtered out; finished in 0.06s
```

- 被改文件行数：`automation_enrollment_tests_http.inc.rs` 646 行（改前 629）。
- 未跑本族之外的全量（全量复核归 Main）。

## 交付说明留痕（Main 复核结论）

**行为变化点名（对老客户端）**：ConflictKind 拆分（REQ-WIGA-SWAP-02）改的是**判定
顺序**而非码值本身——store CAS 中 `PayloadImmutable` 分支
（`issue_automation_store.rs:277`）位于 revision 检查（`:280`）**之前**，因此旧 wire
（不带 `command_id`）的「异 options + 旧 revision」请求会先命中 payload 不可变分支：
响应码从 `automation_enrollment_conflict` 变为 `automation_enrollment_payload_immutable`
（状态码仍 409，details 均携带 current_revision，但稳定码与「该终止重开 / 该刷新重试」
的分流提示不同）。携带稳定 `command_id` 的新 wire 走命令账本重放面（异 payload →
`CommandLedgerPayloadMismatch` → 回落旧码），断言语义不变。⇒ 这是**码分流改变了判定
顺序**，不是码写错；老客户端如依赖旧码做重试分流，需按 REQ-WIGA-02 语义先 Disable
重开而非原地换 payload 重试。
