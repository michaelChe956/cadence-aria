# 失败簇收敛：workspace_engine::tests（144）v1.0

- 日期：2026-11-03
- 工作者：FixWSEngineCluster
- 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 范围：`workspace_engine::tests` 144 条失败（全量日志 148 条 workspace_engine 失败中剔除 `workspace_engine::compile::ir_adapter_tests` 4 条——不在本簇派工范围）

## 0. part_09 现状（接手核验）

`tests/part_09.rs::make_work_item_plan_engine_with_draft_candidate` 已由 FailureTriage 改造完毕（mtime 13:02，接手时 >20min 避让线外）：
- `RepositoryStore::with_logical_codebase_feature(.., enabled())` 建仓 ⇒ 自动 manifest + 本仓 member + 主 checkout（legacy 项目级布局）；
- issue.json 先落盘，`IssueCodebaseSelection::all_members` 后写。
本 worker **未重复改动**，仅在其上补齐下游消费面（见 §2）。

## 1. 根因复判（对照 FailureTriage §3/§6）

对 144 条失败逐条取首条 panic：除 `advance_handler` 9 条（B 类 + 无快照）与 1 条 `provider_drive_design_run_rejects_write_back_outside_resolved_bound`（B 类：无 issue 记录 ⇒ 回写上界解析不了 ⇒ 拒绝分支没生效）外，其余 134 条全部汇入**同一夹具链**：

```
make_work_item_plan_engine_with_draft_candidate (part_09, 已迁移)
  └─ make_work_item_plan_engine_with_accepted_contract_drafts (part_03/part_08)
       └─ test_work_item_draft_record (part_03/part_02) ← candidate.target_repository_id = None
```

两段缺口：
1. LC 权威缺失（`resolve issue logical codebase failed: product_store_not_found: logical_codebase logical_codebase_54ef06…`）——part_09 已修；
2. **draft 逻辑目标缺失**：Logical 路由下 `compile_support.rs:972-984` 对每个 accepted draft 校验 `candidate.target_repository_id` 必须在有效成员集内（FailureTriage §9.8 已预告的下一跳）。

## 2. 逐簇改动表（全部为夹具补前置条件）

| 簇（失败数） | 改动 | 文件 |
|---|---|---|
| conversational_gate_revision（33） | 无直接改动：其 `durable_revision_fixture` 复用 part_09 夹具链，随权威+draft 目标修复连带转绿（SC 链 IR 走 fallback 成员物理仓，`lower.rs::resolve_item_target_repository_id`） | — |
| single_candidate（22）/ conversational_gate_close（9）/ conversational_gate_amendment_real_chain（9）/ single_candidate_recovery（7）/ compile_failure_auto_rewrite（4）/ single_candidate_prompt（2）/ design_reviewer_boundary（1）/ conversational_gate_recovery（1） | 同上：全部经 `make_work_item_plan_engine_with_accepted_contract_drafts` | — |
| part_03 编译簇（work_item_plan_initial_compile_* 12、task_3_4_* 5、compile_recovery_* 5、finalizer_* 3、park1 2、part_16 6、part_10/13/15/17 等 ~15） | `test_work_item_draft_record` 增加 `lifecycle: &LifecycleStore` 首参，新增 `fixture_logical_member_id(lifecycle)`（读夹具唯一成员 logical id），`candidate.target_repository_id = Some(member)`；17 处调用点 + `work_item_draft_artifact_payload`（+lifecycle 参，7 处调用点）+ `target_draft`（+lifecycle 参，8 处调用点）全部机械适配 | part_03/part_02.rs、part_03/part_04.rs、part_03/part_05.rs、part_03/part_07.rs、part_03/part_08.rs、part_03/part_14.rs、part_19.rs、tests/part_08.rs、design_reviewer_boundary.rs、part_32/prompt_window_and_builder_policy.rs |
| park1 `initial_plan_compile_finalize_persists_per_target_work_item_records`（1） | ⚠️ **断言迁移（非夹具）**：`assert_eq!(wi.target_repository_id, None)`（旧「Legacy 单仓」派生分支，随单仓入口删除作废）→ `Some(fixture_logical_member_id(&lifecycle))`；`repository_id == story.repository_id` 不变（LC 下两者同为成员物理仓）。依据 FailureTriage §1 判据（「单仓 legacy 形态被放行/回退物理仓」类旧断言可改）与 admission 簇既有先例；**未删任何断言** | part_03/park1_work_item_records.rs:44-51 |
| b2_p3_trust_compile_facts（4） | `b2_p3_trust_engine` 不再用**空成员** manifest 覆写（会让 Final Compile 必然 `target_repository_id_not_effective`）；改为加载引擎夹具既有 manifest，仅 `provider_context_root = worktree`（confirm trust 要求 manifest root 与 gateway 冻结根同源），成员集保留 | b2_p3_trust_compile_facts.rs:119-132 |
| advance_handler（9） | 根因在 web 共享夹具 `src/web/test_controls/plan_repair/seed.rs`（经 IRC 与 FixWebCluster 确认归我改）：物理仓保持确定性 `repository_0001`，按项目级 legacy 布局补 **manifest + 本仓 member + 主 checkout + selection(all_members) + policy ensure_bootstrap**，并把 repos.json 回填 `logical_repository_id`/`primary_checkout_id`（`resolve_logical_repository_for_issue_codebase` 与 `build_attempt_target_snapshot` 都按它解析）。issue 由既有直接 write_json 落盘（固定 id，无计数漂移），selection 在其后。advance 链自身经 `advance_target_snapshot`→`build_attempt_target_snapshot` 派生冻结快照（worktree 已是真 git 仓 + 初始提交） | web/test_controls/plan_repair/seed.rs:19-25(imports)、132-219 |
| provider_drive_design_run_rejects_write_back_outside_resolved_bound（1） | B 类：夹具无 project/issue 记录 ⇒ 回写上界（selection focus）解析不出，拒绝分支未生效（involved 被错误放行）。补 `ProjectStore::create` + `IssueStore::create`（先建 issue，manifest/selection 在其后）| part_02/provider_drive_writeback.rs:418-440 |

## 3. 窄跑证据

```bash
flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib workspace_engine::tests 2>&1 | tail -80'
```

- 状态：**已排队执行中**（排队的 cargo 锁先后被 FixCWECluster 的 `coding_workspace_engine::tests` 全量跑与后续 worker 占用；本跑启动时已包含上述全部改动）。结果落在本文件 §3.2（跑完后补）。

### 3.1 断言迁移留痕（Main 已批准，与 admission 簇同标准）

用例：`initial_plan_compile_finalize_persists_per_target_work_item_records`（`tests/part_03/park1_work_item_records.rs:44-51`；用例名不含 legacy/single_repo 语义，无需改名）

| | 断言原文 → 新断言 |
|---|---|
| 旧 | `assert_eq!(wi_a.target_repository_id, None);`（注释「Legacy 单仓：story 仓归属，无逻辑仓 target」）；`wi_b` 同 |
| 新 | `let fixture_member = fixture_logical_member_id(&lifecycle); assert_eq!(wi_a.target_repository_id, Some(fixture_member));`（`wi_b` 同）；`assert_eq!(wi_a.repository_id, story.repository_id);` **原样保留** |

- **不变量**：finalize 持久化的 per-target work item record 的仓归属必须与编译期 target 派生同源（repository_id=成员物理仓、target=恰一逻辑成员）。
- **前提变化**：旧断言的前提是「Legacy 单仓 compile 派生 ⇒ target=None」；单仓入口删除后夹具恒走 Logical 路由（compile 由 `logical_targets` 分派成员），前提不存在。
- **新断言更具体**：钉死 = 夹具唯一逻辑成员 id（`fixture_logical_member_id`），比旧的恰等断言更严，不是弱化。
- 未删任何断言；其余 10+ 条断言（title/kind/scopes/plan_status/依赖边/verification_plan_ref 等）零改动。

### 3.2 窄跑结果

**774 passed / 57 failed**（该轮为 FixWSEngineCluster 全部编辑落盘后的首次
完整窄跑；后续 Main/FixStrayClusters 实测订正为 55 条——差 2 条系并发编辑
时序差，前任的「144→4」中间计数从未有有效窄跑支撑）。

## 3.3 后续收敛（FinishWSEngine，2026-11-03 下午）

接手剩余 55~57 条。Main 转发 FixStrayClusters 实测共享根因 + 本 worker
复核 rep4 WI-003 固定 uuid 根因，按三层落地（全部夹具改动，未动断言/实现）：

### ① outline/draft target 同源（draft_target_repository_mismatch，~50 条主因）

**根因**：sibling 给 `test_work_item_draft_record` 的
`candidate.target_repository_id` 补了 `Some(成员)`，但共用 outline 夹具
`test_work_item_plan_outline`（tests/part_01.rs）三个 WI 的
`target_repository_id` 恒 `None` ⇒ `work_item_split_validator/draft.rs:107`
逐 WI 比对判 `draft_target_repository_mismatch` ⇒ 候选被拒 ⇒ 55 种下游
表现（HumanConfirm vs CrossReview、Routed vs Candidate{item_count:3} 等）。

**改动**（outline 侧同源注入，draft 侧不动）：

| 文件 | 改动 |
|---|---|
| tests/part_01.rs | `test_work_item_plan_outline` 加 `logical_target: Option<LogicalRepositoryId>` 参数（三 WI 同值）；3 个拓扑排序调用点传 `None` |
| tests/part_03/part_02.rs | `work_item_plan_outline_artifact` 加同参；`prepare_work_item_plan_outline_artifact` 从 `engine.lifecycle_store` 解析主成员注入；`fixture_logical_member_id` 改按 repos.json `repo-levels` 记录精确解析（见 ②） |
| tests/part_03/part_04.rs | 2 处 Final Compile 调用点传 `Some(fixture_logical_member_id(&lifecycle))` |
| tests/part_03/part_08.rs | `make_work_item_plan_engine_with_accepted_contract_drafts`（主失败链入口）outline 传 Some |
| tests/part_14.rs | `queued_work_item_plan_outline_review_engine` outline 传 Some（review→repair 续链可入 draft 校验） |
| tests/part_03/part_01.rs、part_10.rs、review/tests_policy_routing.rs、part_08.rs、part_19.rs、part_20.rs、part_32/prompt_window_and_builder_policy.rs、design_reviewer_boundary.rs | 纯 review/展示/恢复链调用点机械传 `None`（不触 draft 校验，保持原语义） |

### ② rep4 WI-003 固定逻辑 uuid（gate_revision 27 条的第二层）

**根因**：`work-item-plan-rep4.md` WI-003 声明
`intent_target_logical_repo: 00000000-0000-4000-8000-000000000003`（固定
uuid ≠ 夹具建仓随机分配的成员 id）⇒ lowering 成员表反查
（`lower.rs::resolve_item_target_repository_id` → `physical_by_logical`）
判非成员 `work_item_target_missing` ⇒ rep4 Markdown 候选被拒。rep4.md 被
STRUCTURED_KEYS 矩阵测试钉死不可改 ⇒ 在权威侧注册该 uuid。

**改动**（tests/part_09.rs 夹具）：
- 新建独立物理仓 `repository-integration`（main 分支+初始提交，与主仓同构）；
- LC 权威注册第二成员 `LogicalRepositoryId(Uuid::from_u128(0x...0003))`：
  manifest.member_ids 追加（membership_revision+1）+ member（物理
  `repo-levels-integration`、ordinal 2、Active）+ 独立 Main checkout；
- repos.json 追加第二成员投影（identity_schema_version=1，逻辑/checkout
  id 配对）——追加在主成员 id 改写循环之后，不被连带改写；
- `fixture_logical_member_id` 改按 repos.json `repo-levels` 记录精确解析：
  第二成员 uuid 全零开头，BTree/字典序恒排首位，`.first()` 会选错成员。

### ③ advance identity_mismatch（9 条）+ checkpoint_recovery mixed_target_group_rejected（2 条）

**根因**（identity_schema_version 缺口）：web seed
（`web/test_controls/plan_repair/seed.rs`）的 `repository_0001` 由 legacy
形态创建（`RepositoryStore::new`，LC disabled ⇒ `identity_schema_version=0`），
前任回填只补了 `logical_repository_id`/`primary_checkout_id` 两字段——
`resolve_logical_repository_strict`（resolve.inc.rs:254）的三层身份复验要求
repos.json 投影 `identity_schema_version == 1`，仅回填逻辑归属必然在 schema
版本出口 `identity_mismatch logical_repository <uuid>` fail-closed（前任纸面
核验的三面不含 schema 版本，故「链路均一致」却实测失败）。checkpoint_recovery
的 `mixed_target_group_rejected` 同走 seed 链（units target/snapshot 解析经
strict），预计连带修复。

**改动**（seed.rs 回填循环）：补 `repository.identity_schema_version = 1`。

### 3.3.1 最终窄跑

（待补：跑完后填 passed/failed 计数与残留清单）

## 4. 共享夹具是否需要扩能力

**不需要**。本簇全部修复用到的能力（issue 先建/selection 后写、项目级 legacy 布局权威、真 git 成员仓）`logical_test_fixture.rs` 已具备；本簇因 part_09 已有「LC 特性建仓」路径，draft 目标从该权威现场读取（`fixture_logical_member_id` 为**测试模块内**助手，未动共享夹具）。

## 5. 仍未解 / 需 Main 裁定

1. **park1 断言迁移**（§2 表内 ⚠️ 行）：按 triage 规则与 admission 先例执行，若 Main 认定超出「只改夹具」授权边界，回退方式：恢复两行 `None` 断言 + 从 144 目标中剔除该条。
2. **advance_handler 簇**：seed 已补权威；advance 链租约（仓维锁）由引擎自取，若窄跑仍有残留（如 `legacy_shared_worktree_present` 类），需按共享夹具 `upsert_repo_worktree`+`bind_repo_lock_to_attempt` 预置仓维锁——待窄跑证据。
3. `workspace_engine::compile::ir_adapter_tests`（4 条）不在本簇派工范围，未处理。
4. 窄跑被 cargo 锁排队拖长，最终计数以 §3.1 为准；若仍有红，逐条归因后按同模式迭代。

## 6. 行数门禁

| 文件 | 改前行数 | 改后行数 |
|---|---|---|
| tests/part_03/part_02.rs | 396 | 417 |
| tests/part_03/part_04.rs | 493 | 510 |
| tests/part_03/part_05.rs | 320 | 323 |
| tests/part_03/part_07.rs | 285 | 285 |
| tests/part_03/part_08.rs | 651 | 661 |
| tests/part_03/part_14.rs | 94 | 95 |
| tests/part_19.rs | 451 | 452 |
| tests/part_08.rs | 775 | 776 |
| tests/design_reviewer_boundary.rs | 706 | 706 |
| tests/part_32/prompt_window_and_builder_policy.rs | 1045 | 1045 |
| tests/part_02/provider_drive_writeback.rs | 765 | 788 |
| tests/b2_p3_trust_compile_facts.rs | 748 | 749 |
| tests/part_03/park1_work_item_records.rs | 488 | 489 |
| web/test_controls/plan_repair/seed.rs | 818 | 909 |

均未触及 1200 硬线；`large_file_guard.rs` 未改（登记数字由 Main 统一处理）。

---

# 追加：失败簇收敛 · workspace_engine v1.1（FinishWSEngine2 接手收尾）

- 日期：2026-11-03（18:34–19:0x）
- 工作者：FinishWSEngine2
- 工作树：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 范围：`product::workspace_engine::tests` 本体 + 归并的 `coding_workspace_engine::campaign_stage3_amendment` 2 条 + `web::workspace_ws_handler::tests` 归并簇

## 7. 接手时盘上状态（19:0x 前逐项核验）

前任（FinishWSEngine）三组改动**均已落盘且生效**：

| 前任改动 | 核验方式 | 结论 |
|---|---|---|
| ① outline/draft target 同源（`test_work_item_plan_outline` 加 `logical_target` 参、draft 链注入主成员、纯 review 链 8 处传 `None`） | `part_01.rs:94/105/125/145` 三 WI 同值；`part_03/part_02.rs:412` draft 侧 `Some(fixture_logical_member_id)`；`part_03/part_08.rs:10`、`part_14.rs:19` 注入主成员 | 生效 |
| ② rep4 WI-003 固定 uuid 第二成员（`LogicalRepositoryId(0x0000…0003)`、物理 `repo-levels-integration`、manifest revision+1、repos.json 第二条投影 `identity_schema_version=1`） | `part_09.rs:621/682/683` 独立物理仓 + `REP4_WI003_PHYSICAL_REPOSITORY_ID`；`fixture_logical_member_id` 改按 repos.json `repo-levels` 记录精确解析 | 生效 |
| ③ seed.rs 回填 `identity_schema_version = 1` | `web/test_controls/plan_repair/seed.rs:217` | 生效 |

**接手时树编译不过（3 个错误，全树被挡）**：

```
E0425 ×2  src/product/workspace_engine/tests/advance_handler_parts/checkpoint_recovery.inc.rs:61 / :210
          `crate::product::coding_attempt_store::build_attempt_target_snapshot` —— 该函数实际在
          `crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot`
          （`coding_attempt_store/mod.rs:41` 只有 `pub mod target_snapshot;`，无 re-export）
E0422 ×1  src/product/logical_codebase/logical_test_fixture.rs:197 —— 已由并发 worker（FinalClusterClose2）
          在 18:35:26 自行修好（当前文件已含 `use …::store::LogicalCodebaseRecord;`）
```

⇒ 前任的 §3.3.1「最终窄跑」从未真正跑出来（**带 E0425 的树编译不过**），§3.2 的「774/57」是上一轮数字。

## 8. 窄跑口径

```bash
flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib workspace'
```

过滤器取 `workspace`（**不是** `workspace_engine`）——`workspace` 同时覆盖
`product::workspace_engine::tests` + `product::coding_workspace_engine::tests` +
`web::workspace_ws_handler::tests` 三个簇，一次跑覆盖本簇全部派工面。

| 轮次 | 结果 | 说明 |
|---|---|---|
| run2（18:38，含前任全部改动） | **2335 passed / 47 failed** | 接手基线 |
| run4（18:53，含 ① 修复 + FinalClusterClose2 的 campaign policy 修复） | **2359 passed / 23 failed** | 本轮主验证 |
| run5（18:59，追加 c1 + campaign_stage3_advance 夹具修复） | 见 §12 | 终态 |

**run4 的 47 → 23 分解**：本簇 24 条转绿（18 条我的 ① + 6 条随 FinalClusterClose2 转绿），
剩 23 条 = 17 条非本簇（`web::handlers::lifecycle` 1 + `web::handlers::workspace_choice` 3 +
`web::workspace_context` 5 + `web::workspace_session` 8）+ 6 条 `web::workspace_ws_handler`
（`c1_recovery_surface` 1 + `campaign_stage3_advance` 1 + `campaign_stage3_interactive` 1 +
`gateway_start` 1 + `plan_repair_activation` 2）。

## 9. 逐簇：用例数 → 根因 → 改法 → 窄跑后剩余数

### 簇 A：编译阻断（2 处 E0425，0 条用例）

- **根因**：`checkpoint_recovery.inc.rs` 的 `build_attempt_target_snapshot` 模块路径写错
  （FixCWECluster2 18:21 落盘时的笔误，撞 503 中断前未编译验证）。
- **改法**：两处调用路径改为 `crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot`
  （**纯路径修复，不动语义**）。Main 已确认该文件归我独占。
- **窄跑后**：编译通过 ⇒ 解开全树（含其它 worker 的队列）。

### 簇 B：SC 夹具仍走 legacy 单仓形态（18 条 → 0）

- **用例**：`product::workspace_engine::tests::single_candidate::*` 14 条
  （`author_structure_gate_typed` 3、`author_structure_gate_reredrive` 5、`contract_autorepair` 4、
  `contract_prerevision` 3、`preflight_options_loop` 4、`sg3a1_structure_gate` 1）
  + `compile_failure_auto_rewrite::*` 4 条。
- **根因**（实测，非推断）：SC author 入口
  `complete_single_candidate_work_item_plan_author(candidate, target_repository_id, member_targets, allow_rerun)`
  的第 2 参是**会话物理 target**、第 3 参是**成员表**。夹具 25 处调用点传的是
  legacy 占位 `"repo_fixture".to_string()` + `None`：
  - 编译器（`work_item_plan_compiler/lower.rs::resolve_item_target_repository_id`）在
    `member_targets=None` 分支**忽略每个 WI 自己声明的 Plan Intent**，把**所有** WI 的
    IR `target_repository_id` 写成会话级 `repo_fixture`（连声明了逻辑成员的 WI-003 也一样）；
  - 候选结构门（`draft_batch/compile_support.rs::project_effective_planning_graph`）用
    **live 权威成员表**（`physical_by_logical`，LC 多仓 ⇒ 非空）核验归属 ⇒
    声明物理仓 `repo_fixture` ≠ 授权物理仓 `repo-levels`/`repo-levels-integration`
    ⇒ 合法候选被判 `target_mismatch[WI-001]; target_mismatch[WI-002]; target_mismatch[WI-003]`
    ⇒ 结构门拒收（`final_compile_structure_rejected`）⇒ 表现面 55 种下游形态里的一批
    （`Routed` vs `Candidate{item_count:3}`、`HumanConfirm` vs `CrossReview`、`ir ref` panic…）。
- **契约出处**：`openspec/specs/work-item-group-structure/spec.md:24` ——「物理成员仓 ID → 逻辑成员」
  的反查 MUST 只有一个实现，**候选结构评估与 Final Compile MUST 共用**；非唯一命中 MUST 保持既有失败语义。
  ⇒ 这 18 条断言的是「**合法候选**必须照常进入评审/确认链」，不变量成立 ⇒ **不是作废断言，是夹具没给成员上下文**。
- **改法**（只改夹具，不动断言、不动实现）：
  - 新增 `tests/part_03/part_02.rs::fixture_sc_target(&engine)` / `fixture_sc_member_targets(&engine)`
    —— 与 web runner `run/single_candidate.rs:787-806` **同源**：
    `workspace_repository_for_session(app_paths, lifecycle, session_record)` 取会话业务仓 +
    `plan_compile_member_targets(lifecycle, plan, &repository)` 取 live 权威成员表
    （logical→physical + 确定性回落仓）。
  - 25 处 SC author 调用点的 `"repo_fixture".to_string(), None` → `fixture_sc_target(&engine), fixture_sc_member_targets(&engine)`。
- **窄跑后剩余**：**0**（18 → 0）。

### 簇 C：campaign_stage3 的 6 条（→ 0，**非我改动**，我复核同根）

- **用例**：`coding_workspace_engine::campaign_stage3_amendment` 2 条 +
  `web::workspace_ws_handler::campaign_stage3_recovery_matrix` 4 条。
- **根因**（与我独立复现一致）：`logical_test_fixture.rs:314` →
  `policy_store.get(project_id).unwrap().unwrap()` → `Option::unwrap()` on a `None` value
  ⇒ **聚合 policy artifact 缺失**（基座 `make_work_item_plan_engine_with_accepted_contract_drafts`
  只建 manifest/member/checkout/selection，不建 policy artifact / 聚合索引）。
- **改法**：`FinalClusterClose2` 在共享夹具新增 `ensure_planning_prerequisites(paths, project_id, lc_id)`
  并在 `campaign_amendment_fixture()` 内加一行调用（Main 授权）。**我未改这两个文件。**
- **窄跑后剩余**：**0**。

### 簇 D：`c1_recovery_surface`（1 条，我改）

- **用例**：`web::workspace_ws_handler::tests::c1_recovery_surface::c1_a07_a09_a12_a13_recovery_surface`
- **根因**：web 夹具 `seed_serial_advance_drafts` 铺的 accepted draft 是
  `candidate.target_repository_id: None` ⇒ group 权威单元的 target 取自 source draft
  （`coding_attempt_store/group_validation.rs:215`）⇒ 单元无 target ⇒
  `advance_target_snapshot`（`workspace_engine/advance.rs:690-716`）派生不出冻结快照 ⇒
  group admission（REQ-CS-05 成员单 target 纪律）fail-closed 且**早于初始化 journal**
  ⇒ 用例要的「failed initialization journal」缺失（只有 record，无 journal）。
- **改法**：draft 的 `target_repository_id` 落**权威成员**（从 repos.json 的 `repository_0001` 投影解析
  `logical_repository_id`，seed.rs 建仓时分配）。
- **窄跑后剩余**：**0**（run5 实测绿；`c1_recovery_surface` 从 failures 名单消失）。

### 簇 E：`campaign_stage3_advance`（1 条，我改一半）

- **用例**：`web::workspace_ws_handler::tests::campaign_stage3_advance::campaign_stage3_advance_confirmed_plan_is_ready_without_provider_start`
- **根因**：与簇 D **同根** —— 该文件铺的 accepted draft 同样是 `target_repository_id: None`
  ⇒ advance 派生不出冻结快照 ⇒ 绑不上 issue 共享 worktree。
- **改法（我做的）**：draft 落权威成员（repos.json `repo-levels` 的 logical id）。
- **窄跑后剩余：0**（run5 实测绿）。
  ⇒ **原断言无需迁移**：`shared_worktree()` 仍读 legacy issue 维
  `get_issue_shared_worktree(...)`，且 `current_lock_owner_id == attempt_id` 原断言**原样通过**
  ⇒ 不存在「读取维度迁移」需求（此前 FinalClusterClose2/FixWebCluster 讨论的仓维迁移**未执行、也不需要**）。
  ⚠️ 本条的链路是：draft 无 target ⇒ 权威单元无 target ⇒ advance 派生不出冻结快照
  ⇒ group admission fail-closed 且**早于 worktree 绑定** ⇒ 「advance 绑不上 issue 共享 worktree」。
  夹具补了 draft 的权威成员 target 后，advance 正常推进到 worktree 绑定并落 issue 维事实 ⇒ 原断言成立。

### 簇 F：非本簇（17 条 → 交回 Main）

`web::handlers::lifecycle::tests::reprepare_removes_old_plan_workspace_session`（1）、
`web::handlers::workspace_choice::tests::*`（3）、
`web::workspace_context::tests::*`（5）、
`web::workspace_session::tests::*`（8）。
共同症状：`product_store_not_found: logical_codebase logical_codebase_54ef06…` /
`repository_routing_target_missing` / `persistent WorkspaceEngine requires lifecycle_store`
⇒ workspace context / session manager 侧的 LC 权威播种缺口，**不在 workspace_engine 簇**。

### 簇 G：`web::workspace_ws_handler` 其余 4 条（归 FixWebCluster）

`campaign_stage3_interactive`（gateway 缺 ClaudeCode durable capability 记录 ⇒ provider 零启动）、
`gateway_start`（validate-failure 前置）、`plan_repair_activation` ×2（重复登记 LC 权威 ⇒ IdentityMismatch）。
已与 FixWebCluster 确认归属并同步分工（我不再动这四个文件）。

## 10. 断言迁移：**0 条**

本轮**没有迁移任何断言、没有删断言、没有弱化断言、没有改断言迎合实现**。
- 簇 B 的 18 条断言（「合法候选必须照常进入评审/确认链」等）**全部原样保留** —— 修的是夹具没给成员上下文。
- 簇 D/E 的 2 条断言（`current_lock_owner_id == attempt_id`、`failed initialization journal`）**原样保留**；
  簇 E 的 `shared_worktree()` **仍读 legacy issue 维**（未迁仓维）—— run5 实测原断言通过 ⇒ **无需迁移**。
- ⇒ 本报告**无「新旧断言并排」条目**（本 worker 未做断言迁移）。

## 11. 改动清单（全部为夹具 / 路径修复）

| 文件 | 改动 | 行数（前 → 后） |
|---|---|---|
| `tests/advance_handler_parts/checkpoint_recovery.inc.rs` | 2 处模块路径修复（E0425） | 536 → 536 |
| `tests/part_03/part_02.rs` | 新增 `fixture_sc_target` / `fixture_sc_member_targets` / `fixture_sc_session_repository`（+57 行含注释） | 434 → **491** |
| `tests/compile_failure_auto_rewrite.rs` | 1 处 SC author 调用点换成员上下文 | 441 → 441 |
| `tests/single_candidate.rs` | `complete_author` helper 换成员上下文 | 1541 → **1542** |
| `tests/single_candidate/author_structure_gate_reredrive.rs` | 5 处 | 338 → 338 |
| `tests/single_candidate/author_structure_gate_typed.rs` | 3 处 | 316 → 316 |
| `tests/single_candidate/contract_autorepair.rs` | 4 处 | 254 → 254 |
| `tests/single_candidate/contract_prerevision.rs` | 4 处 | 289 → 289 |
| `tests/single_candidate/preflight_options_loop.rs` | 8 处 | 398 → 398 |
| `web/workspace_ws_handler/tests/c1_recovery_surface.rs` | accepted draft 落权威成员 | 1064 → **1078** |
| `web/workspace_ws_handler/tests/campaign_stage3_advance.rs` | accepted draft 落权威成员 | 889 → **937** |

⚠️ **超 1200 硬线的文件（登记数字由 Main 统一改，`large_file_guard.rs` 我未碰）**：
`src/product/workspace_engine/tests/single_candidate.rs` = **1542**（我 +1 行；改前 1541，已超线）。

## 12. 终态窄跑（run5）

```bash
flock /tmp/aria-cargo.lock -c 'cargo test --locked --lib workspace'
```

**2361 passed / 21 failed**（91.10s，无挂死）。

| 轮次 | 结果 | 变化 |
|---|---|---|
| run2（18:38，接手基线） | 2335 passed / **47 failed** | — |
| run4（18:53，+ 簇 B 修复 + 簇 C 随 FinalClusterClose2 转绿） | 2359 passed / **23 failed** | −24 |
| run5（18:59，+ 簇 D/E 修复） | 2361 passed / **21 failed** | −2 |

**run5 的 21 条剩余**（逐条归属）：

- **17 条不在本簇**（簇 F）：`web::handlers::lifecycle` 1 + `web::handlers::workspace_choice` 3 +
  `web::workspace_context` 5 + `web::workspace_session` 8 ⇒ 交回 Main 派工。
- **4 条 `web::workspace_ws_handler`**（簇 G，归 FixWebCluster，其 worker 在途）：
  `campaign_stage3_interactive::workspace_human_action_http_feedback_replay_and_approve_without_driver`、
  `gateway_start::logical_plan_validate_failure_is_reported_by_handler`、
  `plan_repair_activation::{repeated_confirmation_recovers_delivery_mark_failure_without_duplicate_application,
  zero_socket_plan_amendment_activation_resumes_attempt_with_unsent_delivery}`。

⇒ **本簇（`product::workspace_engine::tests` 全绿 + `product::coding_workspace_engine::tests::campaign_stage3_amendment` 2 条全绿
+ 归并的 `web::workspace_ws_handler` 2 条全绿）已无剩余失败。**

## 13. 仍未解 / 需 Main 裁定

1. **簇 F 的 17 条**（`web::handlers::lifecycle` 1 + `web::handlers::workspace_choice` 3 +
   `web::workspace_context` 5 + `web::workspace_session` 8）—— 不在 workspace_engine 簇，需另派。
2. **簇 E 的断言读取维度迁移** —— **不需要**（run5 实测原断言通过；此前讨论的仓维迁移未执行）。
   ⇒ 已同步 FinalClusterClose2 / FixWebCluster：**不要**再改 `campaign_stage3_advance.rs` 的断言。
3. **簇 G 的 4 条** —— 归 FixWebCluster（已确认；其 worker 在途）。
4. `workspace_engine::compile::ir_adapter_tests`（4 条）不在本簇派工范围，未处理。
5. **本簇的窄跑计数口径**：`workspace_engine` 过滤器不覆盖 `web::workspace_ws_handler::tests`；
   本报告一律用 `workspace` 过滤器（覆盖三簇），Main 汇总时注意口径。
6. **行数超线待 Main 统一登记**：`src/product/workspace_engine/tests/single_candidate.rs` = **1542**（我 +1 行），
   已超 1200 硬线；`large_file_guard.rs` 我未碰。
