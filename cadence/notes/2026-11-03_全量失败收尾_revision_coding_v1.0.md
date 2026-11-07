# 全量失败收尾 · 族 D（work_item_revision_store + coding handlers）v1.0

- 日期：2026-11-03
- 工作目录：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 范围：族 D 的 3 条失败（11 / 12 / 13）
- 硬约束遵守：未改生产实现、未恢复 `RepositoryRouting::Legacy`、未自造 reason_code、未改 spec/Plan/技术方案、未删测试、未 `git add/commit/stash/checkout`、未碰 `tests/it_core/large_file_guard.rs`（登记数字见文末「被改文件行数」，由 Main 统一改）

---

## 11) `product::work_item_revision_store::tests::initial_publication::runtime_reader_derives_coding_unit_binding_and_rejects_run_hash_mismatch`

- **用例名**：`runtime_reader_derives_coding_unit_binding_and_rejects_run_hash_mismatch`
- **证据位置**：`src/product/work_item_revision_store/tests/initial_publication.rs`（夹具 `bound_group_attempt_fixture`，原 928–1028 行）
- **根因族**：**族 A（无 LC 权威 / 缺归属字段）** —— 两处缺前置，**都不是断言问题**：
  1. 夹具的 issue 虽写了 `logical_codebase_id`，但该 LC 侧**没有 manifest/member/checkout/selection**（两态 routing 的 `(None, None)`）⇒ `RepositoryRouting::unbound_issue()` ⇒ 评估链 `build_evaluation_context_pack` fail-closed：
     `InvalidRecord { kind: "repository_routing", reason: "repository_routing_target_missing: issue_not_bound_to_logical_codebase: work item has no logical codebase attribution" }`（原红输出，第 765 行 unwrap 落在此处）。
  2. 补上 LC 权威后暴露第二处：group attempt 的**仓维归属**来自 work item 的**源 draft**（`resolve_authoritative_group_plan_binding` 按 `draft.candidate.target_repository_id` 解析成员）⇒ 夹具缺该 draft ⇒
     `InvalidRecord { kind: "repository_routing", reason: "repository_routing_inconsistent: work item revision source draft is missing" }`（第 793 行 `repository_path_for_attempt`）。
- **改法（只改夹具，不改断言、不改实现）**：
  1. 夹具改用**共享夹具** `crate::product::logical_codebase::logical_test_fixture::seed_complete`（`src/product/logical_codebase/logical_test_fixture.rs`，只读未改）建出完整现场：project + 归属 issue + LC 权威（manifest/member/checkout/selection/policy）+ `repos.json` 投影；并断言 issue 的 `logical_codebase_id` 与夹具 LC 一致（归属字段钉死）。
  2. 补**源 draft 归属**：新增常量 `SOURCE_DRAFT_REVISION_ID = "draft_revision_0001"`（与发布 journal 的 `work_item_revision.source_draft_revision_id` 同源，journal 两处硬编码改为引用该常量），并 `WorkItemPlanStore::put_draft_record` 落一条 `WorkItemDraftRecord`：`draft_id = SOURCE_DRAFT_REVISION_ID`、`candidate.logical_work_item_id = WORK_ITEM_ID`、`candidate.target_repository_id = Some(member_id)`。
  3. 随之把夹具的 `repository_id` / `repository_path` 改为**权威侧**的真实值（`FIXTURE_PHYSICAL_REPOSITORY_ID` / `logical.member_path`）——原值来自被共享夹具覆盖掉的 `RepositoryStore::create` 记录（`repos.json` 投影已被权威记录取代），断言的**语义不变**（仍是「该 attempt 解析到的物理仓 == 夹具声明的物理仓」）。
  4. 删除因此失效的 import（`IssueStore/CreateProductIssueInput`、`ProjectStore/CreateProjectInput`、`RepositoryStore/CreateRepositoryInput`）。
- **改前 → 改后**：
  - 改前：夹具 = project + `RepositoryStore::create`(path=temp 根) + issue(归属 legacy 别名 LC，**无任何 LC 权威**)，无 draft。
  - 改后：夹具 = `seed_complete`（project + issue + manifest/member/checkout/selection/policy + repos.json 投影，成员 checkout 是**真 git 仓**且位于聚合根之下）+ 同名源 draft（带 `target_repository_id`）。
- **红 → 绿输出**：
  - 红（改前）：`called Result::unwrap() on an Err value: Store(InvalidRecord { kind: "repository_routing", reason: "repository_routing_target_missing: issue_not_bound_to_logical_codebase: work item has no logical codebase attribution" })`
  - 红（补 LC 权威后、尚缺 draft）：`... reason: "repository_routing_inconsistent: work item revision source draft is missing" }`
  - 绿：（见文末「窄跑结果」）
- **concerns**：
  - 夹具的 `repository_path` 现在指向共享夹具建的成员 checkout（`<temp>/aggregate-root/repository_0001`），不再指向 temp 根；断言语义（attempt 解析到的仓 == 夹具声明的仓）保持不变。
  - 本条**不需要改生产实现**。

---

## 12) `web::handlers::coding::plan_final_confirm_tests::plan_final_confirm_post_confirms_chinese_evidence_commit_end_to_end`

- **用例名**：`plan_final_confirm_post_confirms_chinese_evidence_commit_end_to_end`
- **证据位置**：`src/web/handlers/coding/plan_final_confirm_tests.rs:519`（断言 `result.outcome == "all_confirmed"`）
- **根因族**：**族 A（LC 权威与成员 checkout 的几何关系不自洽 ⇒ 范围门禁 fail-closed）**
  - 实际失败目标：`target_repository_id = ba9fb562-…`（busi），`error_code = coding_final_confirm_failed`，
    `message = work_item_scope_domain_unresolved: coding_attempt_b3a8eb10…: member checkout /tmp/.tmp0CpkbP/n3-busi is outside provider_context_root /tmp/.tmp0CpkbP/worktree`
  - 链路：`handle_final_confirm` → `run_group_completion_gates` → `schema_v2_group_completion_gate_facts` → `changed_files_for_unit_completion_range`（中文证据提交 ⇒ changed_files 非空）→ `validate_changed_files_for_runtime` → `scope_member_prefix`（`src/product/coding_workspace_engine/gates.rs:675` 一带）。
    `scope_member_prefix` 的契约：**成员 checkout 必须在 manifest 的 `provider_context_root` 之下**，否则一律 fail-closed（不退化无前缀比较）。
  - 夹具不自洽处：`n3_fixture`（`src/web/handlers/coding/plan_start_tests.rs`）的四个多仓成员 checkout 建在 `<temp>/n3-*`，而基座夹具 `PlanRepairFixtureRuntime::seed`（`src/web/test_controls/plan_repair/seed.rs:63,149`）把 LC 聚合根取成其单仓 `<temp>/worktree` ⇒ 成员在聚合根之外 ⇒ 任何**有 changed_files** 的完成门禁都 fail-closed。
    （无证据提交的同族用例之所以绿：changed_files 为空 ⇒ `validate_changed_files_for_runtime` 提前返回，根本不解析前缀。）
  - 权威口径佐证：`src/product/coding_workspace_engine/tests/gates_scope_domain.rs:53-58` 的专用夹具就是「聚合根 `<root>/aggregate` + 成员 `<root>/aggregate/busi`（直接子目录）」；`src/product/logical_codebase/aggregate_index/exclude.rs:46` 亦要求成员是聚合根的**直接子目录**。
- **改法（只改夹具，不改断言、不改实现）**：
  - 在 `n3_fixture` 登记完四个仓之后，把该 LC 的 manifest `provider_context_root` 上提到**夹具根目录**（`<temp>`），使全部成员（基座单仓 `<temp>/worktree` + 四个 `<temp>/n3-*`）成为聚合根的直接子目录。
  - 只改 `provider_context_root` 一个字段：**成员集合 / `membership_revision` / `logical_codebase_id` / `layout` 一律不动**（冻结快照按 `membership_revision` 校验，改动会引发快照漂移）。
- **改前 → 改后**：
  - 改前：`provider_context_root = <temp>/worktree`，成员 `<temp>/n3-busi` 在其外 ⇒ `work_item_scope_domain_unresolved` ⇒ 目标 failed ⇒ `outcome = partially_confirmed`。
  - 改后：`provider_context_root = <temp>`，成员 `<temp>/n3-busi` 是其直接子目录 ⇒ 前缀解析为 `n3-busi` ⇒ 范围门禁按契约空 scope 放行 ⇒ 四目标全 confirmed ⇒ `outcome = all_confirmed`。
- **红 → 绿输出**：
  - 红（改前）：`assertion left == right failed: per-target confirmation results: [... status: "failed", error_code: Some("coding_final_confirm_failed"), message: Some("work_item_scope_domain_unresolved: …: member checkout /tmp/.tmp0CpkbP/n3-busi is outside provider_context_root /tmp/.tmp0CpkbP/worktree") …]  left: "partially_confirmed"  right: "all_confirmed"`
  - 绿：（见文末「窄跑结果」）
- **concerns**：
  - 该修复动的是**共享夹具** `n3_fixture`（plan_start / plan_final_confirm 两族共用），因此一并跑了 `plan_coding_` + `plan_final_confirm_` 过滤做回归（见文末）。
  - 附带把该用例的两处断言补了失败上下文（`assert_eq!(result.outcome, "all_confirmed", "per-target confirmation results: {:?}", result.results)` 与逐项 `"failed target: {item:?}"`）——**只加诊断信息，不放宽断言**。
  - 本条**不需要改生产实现**。

---

## 13) `web::handlers::coding::plan_start_tests::plan_coding_sweep_is_benign_noop_on_legacy_single_repo_layout`

- **原用例名**：`plan_coding_sweep_is_benign_noop_on_legacy_single_repo_layout`
- **证据位置**：`src/web/handlers/coding/plan_start_tests.rs:3419`（原 3385–3425）
- **根因族**：**族 D（旧形态断言已随块3 作废 ⇒ 迁移为新不变量）**
  - **契约出处（原文）**：`openspec/changes/archive/2026-11-03-remove-single-repository-path/specs/web-runtime-repository-routing/spec.md`
    - REMOVED「统一分流状态机（REQ-ROUTE-01）」：「本条定义的三态（`Legacy` / `Logical` / `FailClosed`）中的 `Legacy` 即单仓语义（issue 未归属 LC → 回退物理仓）。技术方案 §3.1.1 要求 `Legacy` 变体整个移除、`(None, None)` 落 `FailClosed`，本条随之被两态版本取代。」
    - ADDED「两态分流状态机（REQ-ROUTE-01）」：「`RepositoryRouting` SHALL 只包含这两个变体，MUST NOT 保留单仓语义的 `Legacy` 变体」「`(None, None)`（issue 未持久归属任何逻辑代码库）SHALL 落 `FailClosed` 并返回稳定错误码，MUST NOT 回退物理仓。」
    - REMOVED「单仓回退语义（REQ-ROUTE-03）」：「本条的核心是『无逻辑状态时可回退物理 `repository_id`』，这正是要被删除的单仓语义……老记录返回 `issue_not_bound_to_logical_codebase` 而非静默降级。」
    - ADDED「全入口对齐（无单仓分支）（REQ-ROUTE-02）」：「创建 issue 不再走单仓校验……系统 SHALL 要求携带逻辑代码库归属；MUST NOT 提供『无 LC、仅带物理 `repo_id`』的创建路径。」
  - ⇒ 原用例钉的形态（「单仓(Legacy)布局上 sweep 是 benign no-op」）**已无出处**：块3 删掉了 `RepositoryRouting::Legacy`，issue 无归属一律 FailClosed。原用例之所以还能绿，靠的是「夹具根本没有 issue 记录 ⇒ 什么都没做」的巧合，而非契约。
  - 当前实测（改前）：`ApiError { code: "issue_not_found", message: "issue not found", details: Object {} }` —— 旧形态**已不可达**，sweep 在缺 issue 时 fail-closed（稳定码 `issue_not_found`；`src/web/handlers/support_parts/product_store_error.inc.rs:289`）。
- **改法（改名 + 断新不变量，不删测试、不弱化断言）**：
  - 拆成两条用例，**旧形态的覆盖面一条不丢**：
    1. `plan_coding_sweep_fails_closed_when_issue_record_is_absent`（旧用例的**原现场**：project 目录 + 物理仓目录 + 残留目录，**没有 issue 记录**）⇒ 断新不变量「**旧单仓形态已不可达 ⇒ sweep 必须 fail-closed**」：断言错误码恰为稳定码 `issue_not_found`，且残留目录**一个字节都没动**。
    2. `plan_coding_sweep_is_benign_noop_when_issue_has_no_logical_codebase_authority`（**新形态**：issue 存在、归属一个**没有 manifest/selection** 的 LC ⇒ 两态 routing 落 `FailClosed(TargetMissing)`）⇒ 断新不变量「**无 LC 权威 ⇒ 不回退物理仓**」：
       - 显式断言 `RepositoryRouting::load_for_issue` 落 `FailClosed`（单仓形态不再有 routing 变体）；
       - 物理仓**已登记且可解析**（`repos.json` 在场）时，sweep 依「FailClosed 仅告警跳过」契约零动作收尾：其 issue worktree 残留目录必须原封不动。
  - 「benign no-op / 零扰动」这一原不变量**被保留并加强**：原用例的零扰动来自「解析不到仓根」的巧合，新用例的零扰动来自「routing fail-closed ⇒ 绝不回退物理仓」的**语义**。
- **改前 → 改后**：
  - 改前：旧形态（无 issue 记录）→ sweep 静默成功（benign no-op），靠巧合成立。
  - 改后：旧形态 → `Err(issue_not_found)`（fail-closed，稳定码）；新形态（issue 无 LC 权威）→ `Ok` 且物理仓残留零扰动（routing FailClosed 语义）。
- **红 → 绿输出**：
  - 红（改前）：`sweep on a legacy single-repo layout is a benign no-op: ApiError { code: "issue_not_found", message: "issue not found", details: Object {} }`
  - 绿：（见文末「窄跑结果」）
- **concerns**：
  - 族 D 迁移按要求**先核实了契约出处**（上面逐条引用 spec 原文），确认旧形态已作废后才迁移；**未删测试、未弱化断言**。
  - `sweep_issue_worktree_residue` 对 routing FailClosed 的既有契约（「仅告警跳过，不阻断已完成的删除结果」）落在 `src/web/handlers/support_parts/coding_cleanup.inc.rs:200` 一带的实现注释里，本条直接沿用，**未改实现**。

---

## 窄跑结果

窄跑命令（单批次，`flock /tmp/aria-cargo.lock` 串行，未起第二个 cargo 任务）：

```
flock /tmp/aria-cargo.lock cargo test --locked --lib -- \
  runtime_reader_derives_coding_unit_binding_and_rejects_run_hash_mismatch \
  plan_coding_ plan_final_confirm_
```

- 本族 3 条（改后）：

```
test product::work_item_revision_store::tests::initial_publication::runtime_reader_derives_coding_unit_binding_and_rejects_run_hash_mismatch ... ok
test web::handlers::coding::plan_final_confirm_tests::plan_final_confirm_post_confirms_chinese_evidence_commit_end_to_end ... ok
test web::handlers::coding::plan_start_tests::plan_coding_sweep_fails_closed_when_issue_record_is_absent ... ok
test web::handlers::coding::plan_start_tests::plan_coding_sweep_is_benign_noop_when_issue_has_no_logical_codebase_authority ... ok
```

- 汇总（同时覆盖共享夹具 `n3_fixture` 改动带来的回归面：`plan_coding_` + `plan_final_confirm_` 两个前缀覆盖这两个文件里的全部 37 处 `n3_fixture()` 用例）：

```
test result: ok. 76 passed; 0 failed; 0 ignored; 0 measured; 5895 filtered out; finished in 8.30s
```

- 未跑本族之外的全量（全量由 Main 统一跑）。

## 被改文件行数（供 Main 统一更新 `tests/it_core/large_file_guard.rs` 登记数字）

| 文件 | 改后行数 | 是否需改登记 |
| --- | --- | --- |
| `src/web/handlers/coding/plan_start_tests.rs` | **4106**（原 4001） | **需要**：登记表当前登记 4001 → 应改为 4106 |
| `src/web/handlers/coding/plan_final_confirm_tests.rs` | **874**（原 866） | 否（< 1200，未登记） |
| `src/product/work_item_revision_store/tests/initial_publication.rs` | **1064**（原 1028） | 否（< 1200，未登记） |
| `cadence/notes/2026-11-03_全量失败收尾_revision_coding_v1.0.md` | 本报告 | 否（不在扫描根） |

> `tests/it_core/large_file_guard.rs` **未改动**（按分工由 Main 统一更新）。

## 边界自查

- 未改任何生产实现（只改测试夹具与测试断言的诊断信息）。
- 未恢复 `RepositoryRouting::Legacy` 变体、未自造 reason_code（`issue_not_found` 既是既有稳定码，也是本用例**实测**得到的码）。
- 未改 spec / Plan / 技术方案；未删测试（族 D 为「改名 + 断新不变量」，旧覆盖面拆成两条保留）。
- 未 `git add` / `git commit` / `git stash` / `git checkout`。
- 未碰 `tests/it_core/large_file_guard.rs`。
- mtime 避让：本族三个目标文件在本波开工时均为 20:08（`plan_start_tests.rs` / `plan_final_confirm_tests.rs`）与 20:42（`initial_publication.rs`，属本波自身变更），改前已逐一 `ls -la --time-style=+%H:%M:%S` 复核；共享只读夹具 `src/product/logical_codebase/logical_test_fixture.rs` 全程**未改**。
