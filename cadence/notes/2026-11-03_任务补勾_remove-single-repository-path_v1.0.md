# 任务补勾｜remove-single-repository-path（块3）v1.0

- 对象：`openspec/changes/remove-single-repository-path/tasks.md`（**只改勾选框**，任务文字 / 编号 / 顺序 / 行数一律未动）
- 工作目录：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 判据：①实现产物在当前代码里存在（逐条查证到 文件:行号）且②实施报告有证据 ⇒ 勾；①存在②报告未覆盖 ⇒ 勾并注明「代码实况为据，报告未覆盖」；实现确实没做 ⇒ 不勾并注明缺什么 + 归属切片；实现被后续裁决作废 ⇒ 不勾并注明因何作废。
- 本轮**未使用** 09:34 的对证快照做分类依据；快照只作对照，所有结论均以当前代码为准。

---

## 1. 统计

| 维度 | 数量 |
|---|---|
| 任务总数 | **52** |
| 本轮前已勾 | 30 |
| 本轮新勾 | **18** |
| 本轮后已勾 | **48** |
| 本轮后未勾 | **4** |

### 逐类分布（本轮新勾的 18 条）

| 类 | 条数 | 任务编号 |
|---|---|---|
| A. ①实现存在 + ②报告有证据 | 12 | 1.1 / 1.2 / 1.3 / 1.4 / 1.6 / 2.1 / 2.2 / 2.3 / 2.5 / 3.1 / 3.2 / 3.3 |
| B. ①实现存在 + ②报告未覆盖（代码实况为据） | 6 | 1.5 / 3.4 / 3.5 / 10.3 / 11.1 / 11.2 |
| C. 实现确实没做（保持未勾） | 1 | 2.4 |
| D. 用例/证据未编写（保持未勾） | 1 | 4.4 |
| E. 真机 / 人工验收未执行（保持未勾） | 2 | 5.2 / 11.4 |
| F. 因裁决作废（保持未勾） | **0** | — |

> **B 类说明**：1.5 的红线核对结论、3.4 的发起侧收窄、3.5 的用例集合、10.3 的版本落后【更新】引导、11.1/11.2 的「代码与用例已落地、只差绿测输出」——这些在实施报告里只有过程性描述或明确标注「待统一绿测」，**代码实况为据**勾选。
> **F 类为 0**：块3 的 4 条未勾**没有一条**是因后续裁决作废。裁决作废的三类（单仓入口删除 ⇒ 无归属一律 fail-closed / `logical_codebase_id` 收敛必填 / 候选表 `action_hint` 不在 REQ-BOOT-08 范围）在块3 的任务文字里**已按新口径改写完毕**，不存在「任务仍要求旧形态 ⇒ 作废」的情形。3.4 的措辞已按块3 Plan §6 R19 更正，本轮**只改勾选框、未动一字**。

---

## 2. 逐条对照表（52 条全覆盖）

### §1 路由三态 → 两态（REQ-ROUTE-01）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 1.1 | ✅ 勾 | `src/product/logical_codebase/repository_routing_types.inc.rs:41-53`（枚举仅 `Logical`/`FailClosed`；`:39` 注释「单仓 `Legacy` 变体已随去单仓入口整体移除」）；全仓 match 已穷尽化（`src/product/coding_attempt_repository.rs:47/:84`、`src/product/coding_evaluation_context/builder.rs:296-373`、`src/product/coding_workspace_engine/gates.rs:135-146`、`src/product/workspace_engine/advance.rs:1072-1102`、`src/product/workspace_engine/advance_split.rs:78-82`） |
| 1.2 | ✅ 勾 | `src/product/logical_codebase/repository_routing_types.inc.rs:59-67` —— `fn resolve_issue_logical_codebase_id(...) -> Result<String, ProductStoreError>`，`issue_store.get()` 不存在即 `NotFound` 错误，`None` 分支已删 |
| 1.3 | ✅ 勾 | `src/product/logical_codebase/repository_routing_types.inc.rs:71-79` —— `fn issue_codebase_stores(app_paths, lc_id: &str)`，只走 `LogicalCodebaseStore::for_lc` / `IssueCodebaseSelectionStore::for_lc`，project 级 store 的 `None` 回退已删；`:70` 注释「project 级 store 的 None 回退已随 Legacy 变体删除」 |
| 1.4 | ✅ 勾 | `src/product/logical_codebase/repository_routing_types.inc.rs:100`（`(None, None) => Self::unbound_issue()`）→ `:85-91` `unbound_issue()` = `FailClosed(TargetMissing)`；稳定码 `src/product/logical_codebase/repository_routing_types.inc.rs:26` = `repository_routing_target_missing`；reason 前缀 `issue_not_bound_to_logical_codebase`（`:89`）；**无物理仓回退**（全仓无 Legacy 臂） |
| 1.5 | ✅ 勾（B 类：代码实况为据，报告未覆盖逐处核实结论） | 红线四项**均在库**：①`migrate_legacy` 34 处命中（`src/` 内 grep）；②`logical_codebase_root` 56 处命中；③`src/product/dangling_reference/patrol/lc_partition.rs:7`（「新版子树 + 旧版别名，两路都扫」）+ `:36-44`；④`src/product/logical_codebase/repository_routing_resolver.inc.rs:198-222` / `:347-358` 布局冲突判定（`repository_routing_legacy_conflict`）在库。残留 `legacy` 命名全部落在**布局/旧 project-level 布局**语义（`legacy_members`/`legacy_root`/`repository_routing_legacy_conflict`），无一处指单仓路由语义 |
| 1.6 | ✅ 勾 | `src/product/logical_codebase/repository_routing_tests.inc.rs:19-27`（`(None,None)` → `FailClosed` + 稳定码断言，panic 文案「must fail closed without a legacy arm」）、`:30-35`（`(Some,Some)` → `Logical`）、`:37-47`（`(Some,None)` → `FailClosed(TargetMissing)`）、`:50-60`（`(None,Some)` → `FailClosed(OrphanedSelection)`）、`:225-258`（`issue_codebase_stores` 只认 lc 子树）、`:147-165`（`resolve_issue_logical_codebase_id` 返回 `String`） |

### §2 issue 归属字段收敛与自动化载体判定（REQ-ROUTE-03 / REQ-ROUTE-C5-TARGET）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 2.1 | ✅ 勾 | `src/product/models/project.rs:144` —— `pub logical_codebase_id: String`（`Option<String>` 已消失，serde 默认属性已删）；`src/product/issue_store.rs:15` / `:28` —— `CreateProductIssueInput` / `CreateProductIssueWithRepositoryInput` 的 `logical_codebase_id: String`；**无数据迁移脚本**（`src/product/logical_codebase/migration.rs:40` 只作兼容性说明） |
| 2.2 | ✅ 勾 | `src/product/issue_store.rs:186-196` —— `read_issue_record`：字段缺失/空串 → `ProductStoreError::InvalidRecord { kind: "issue_not_bound_to_logical_codebase" }`（不补默认、不推导单仓）；用例 `src/product/issue_store.rs:314-352` `old_issue_record_without_lc_id_reports_explicit_error`（逐字断言 `kind == "issue_not_bound_to_logical_codebase"`）；`src/web/error.rs:178` → 400 BAD_REQUEST；`src/web/handlers/support_parts/product_store_error.inc.rs:253-261` → 稳定码 + 400 + details |
| 2.3 | ✅ 勾 | `src/web/handlers/support.rs:211-219` —— `AutomationCarrierResolution` **只剩 `LogicalCodebase`** 变体；`src/web/handlers/support.rs:228-245` —— `resolve_automation_carrier` 的 `None` 分支 → `ApiError::validation("issue_not_bound_to_logical_codebase", …)`（400），**不再有「无 LC 归属→单仓」判定**；消费侧同源：`src/web/handlers/automation_target.rs:92`、`src/web/handlers/automation_enrollment.rs:385`、`src/web/advance_plan.rs:228/:244`、`src/web/coding_start.rs:754/:778/:879`、`src/web/handlers/automation_gateway_preflight.rs:259`；全仓 `AutomationCarrierResolution::SingleRepository` = **0** |
| 2.4 | ❌ **不勾** | **实现确实没做** —— 见 §3「未勾清单」第 1 条 |
| 2.5 | ✅ 勾 | `src/web/handlers/automation_target.rs:727-741`（T1.6 `automation_target_never_projects_single_repository_carrier`：无归属 issue 的 GET 投影 `!is_success` + `payload["code"] == "issue_not_bound_to_logical_codebase"`）、`:743-757`（T1.7 `enable_with_unbound_issue_fails_closed`：PUT Enable 4xx + 零 enrollment 写入）；老记录报错用例 `src/product/issue_store.rs:314-352`；成员不一致仍 fail-closed `src/product/logical_codebase/repository_routing_types.inc.rs:105-112` |

### §3 生产 32 处 / 19 文件 `Legacy` 出现点逐处改写与入口对齐（REQ-ROUTE-01/02）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 3.1 | ✅ 勾 | **全仓 0 命中**：`grep -rn "RepositoryRouting::Legacy" src/ web/src/ --include=*.rs --include=*.ts --include=*.tsx` → **0**（含测试文件）。**2 处 `#[cfg(test)]` 命中已一并处置**：`src/product/coding_attempt_store/group_validation.rs:682-684`（原构造点 → `RepositoryRouting::unbound_issue()`，改写为 FailClosed）、`src/web/handlers/support_parts/coding_cleanup.inc.rs:161-185`（原匹配臂 → `Logical`/`FailClosed` 两态，`:158-160` 注释「无归属旧数据在 routing 层即 FailClosed，绝不静默回退物理仓库」）。枚举定义处 `src/product/logical_codebase/repository_routing_types.inc.rs:39` 已无 `Legacy { repository_id: String }` |
| 3.2 | ✅ 勾 | 入口逐个对齐（无单仓分支、无物理仓回退）：`src/product/workspace_repository.rs:83-113`（`resolve_work_item_repository_context`：`Logical` → 按 selection 解析；`FailClosed` → `Err`）、`src/web/handlers/coding/repository_resolution.rs:60-85`（SingleRepo 形状 → resolver 必拒 `kind_mismatch`，注释明写「统一分流不再回退物理仓」）、`src/product/coding_evaluation_context/builder.rs:296-373`、`src/web/handlers/support_parts/coding_cleanup.inc.rs:161-185`、`src/product/coding_attempt_repository.rs:41-131`、`src/product/workspace_engine/advance.rs:1072-1102`、`src/product/workspace_engine/advance_split.rs:78-82`、`src/product/coding_workspace_engine/gates.rs:135-146` |
| 3.3 | ✅ 勾 | `src/web/handlers/product_resources.rs:157-180` —— `create_product_issue` 的 `None` 臂整段删除，直达 `create_logical_codebase_issue(&request.logical_codebase_id, …)`（`:170-174` 注释「单仓创建入口已删除」）；`src/web/handlers/product_resources.rs:230-250` —— 创建链只剩 `require_logical_codebase` + `validate_logical_codebase_primary`（LC 作用域校验），**无独立「repo_id 物理存在性」前置门** |
| 3.4 | ✅ 勾（B 类：代码实况为据） | 发起侧收窄全部落地：`web/src/api/types/lifecycle.ts:350-365`（新增 `EnrollmentCarrierTarget` 两变体）、`:365-372`（`EnrollmentTarget` 三变体**保留**，`single_repository` 仍为 legacy 只读）、`web/src/components/lifecycle/automation-reenable.ts:63-85`（守卫 `carrierTargetFromEnrollmentTarget`，`case "single_repository": return null`）、`:87-91`（`LEGACY_CARRIER_BLOCKED_MESSAGE` = 「需重建为逻辑代码库授权」）、`web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:132-137`（`submitCarrierTarget` / `legacyCarrierBlocked` → 禁用提交 + 提示）、`:583-590`（`enrollmentTargetLabel` 的 `case "single_repository"` 只读渲染保留）、`web/src/state/workspace-cockpit-inbox-items.ts:56`（`c1TargetLabel` 的 `case "single_repository"` 保留）、`web/src/components/lifecycle/useIssueLifecycleGeneration.ts:440/:566`（命令构造处守卫）。**禁删对象仍在库**：`src/product/logical_codebase/types.rs:31`（`EnrollmentTarget::SingleRepository`）、`src/product/logical_codebase/repository_routing_types.inc.rs:151-152`（`RepositoryTargetKind::SingleRepo`）、`web/src/api/types/work-item-plan.ts` 的 `EnrollmentTargetDto.single_repository`；Rust 侧 `AutomationCarrierResolution::SingleRepository` 已删（见 2.3） |
| 3.5 | ✅ 勾（B 类：代码实况为据，绿测待统一跑） | 用例集合在库：`src/product/logical_codebase/repository_routing_tests.inc.rs:19-60`（同一状态在各入口一致分流）、`src/web/handlers/automation_target.rs:727-757`（无归属 fail-closed 且不返回 200）、`src/product/issue_store.rs:314-352`（老记录报错不静默回退）、`src/product/workspace_repository_tests.rs`（T1.9）、`src/web/advance_plan.rs`（T1.11）。多仓既有行为回归用例：`tests/it_core/single_member_lc_lifecycle.rs`（763 行）+ `tests/it_core/single_member_path_guard.rs`（556 行） |

### §4 前端单仓入口删除（REQ-REG-16 / REQ-MRE-01）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 4.1 | ✅ 勾 | `web/src/components/lifecycle/AddCodebaseDialog.tsx` —— `grep -n "单仓\|single"` **0 命中**（radio 与「继续添加单仓库」按钮已删）；用例 `web/src/components/lifecycle/AddCodebaseDialog.test.tsx:13`（查不到「单仓库」radio / 「继续添加单仓库」按钮）、`:33`（默认直接渲染多仓表单） |
| 4.2 | ✅ 勾 | `web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx:14-15`（`logical_codebase_id: string` 必填）、`:203`（下拉只列逻辑代码库）；用例 `web/src/components/lifecycle/CreateLifecycleIssueDialog.test.tsx:68` |
| 4.3 | ✅ 勾 | `web/src/components/lifecycle/AddCodebaseDialog.test.tsx:13/:33/:49`、`web/src/components/lifecycle/CreateLifecycleIssueDialog.test.tsx:68/:99/:119/:159/:212/:242/:284/:318` —— 组件与测试同步改写 |
| 4.4 | ❌ **不勾** | **用例未编写** —— 见 §3「未勾清单」第 2 条（前两项已落地，第三项缺） |

### §5 新建项目直接多仓（REQ-REG-16）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 5.1 | ✅ 勾 | `web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx:203`（只列逻辑代码库）+ `web/src/components/lifecycle/AddCodebaseDialog.tsx`（默认多仓表单，无模式选择）；后端 `src/web/handlers/product_resources.rs:157-180` 单一路径 |
| 5.2 | ❌ **不勾** | **真机未走** —— 见 §3「未勾清单」第 3 条（「不新造页面」已落地并有测试钉住） |
| 5.3 | ✅ 勾 | `web/src/components/lifecycle/AddCodebaseDialog.test.tsx:49`（「创建成功后同弹窗进入登记，复用聚合根自动发现（建 LC 入口不新造页面）」）；`web/src/components/lifecycle/LogicalCodebaseRegistrationWizard.tsx` 承载聚合根选择 + 成员登记 + 聚合初始化 |
| 5.4 | ✅ 勾 | `web/src/components/lifecycle/CreateLifecycleIssueDialog.test.tsx:99`（未选逻辑代码库提交被拒、不调用 `onCreate`）、`:159`（未勾成员阻止提交）、`:284`（1 成员 ≡ 单选同一路径）、`:318`（未勾任何成员阻止提交）；`web/src/components/lifecycle/AddCodebaseDialog.test.tsx:49` |

### §6 1 成员与 N 成员同路径（REQ-REG-17）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 6.1 | ✅ 勾 | `tests/it_core/single_member_path_guard.rs`（T4.1/T4.2/T4.3 源码级清单扫描，含阴性对照）；实测 `grep -rn "members.len() == 1\|member_ids.len() == 1\|member_count == 1\|require_multiple_members" src/ web/src/` 仅 1 命中且为**测试断言**（`src/web/advance_plan.rs:1436`，`assert!(manifest.member_ids.len() == 1, "fixture 形态…")`，非生产分支） |
| 6.2 | ✅ 勾 | 同上守卫 T4.2（`no_gate_requires_at_least_two_members_for_single_member_lc`）零命中 |
| 6.3 | ✅ 勾 | `tests/it_core/single_member_lc_lifecycle.rs`（T4.4~T4.7）；`src/product/logical_codebase/bootstrap_projector.inc.rs` 的 identity 步判据为 `active_members >= 1` |
| 6.4 | ✅ 勾 | `tests/it_core/single_member_path_guard.rs`（T4.1 断言生产代码零「成员数为 1」专门分支、T4.2 零「≥2 成员」门禁、T4.3 零拒绝文案；每条带阴性对照防恒真空扫） |
| 6.5 | ✅ 勾 | `tests/it_core/single_member_lc_lifecycle.rs`（T4.4 首建到 `PlanningReady`、T4.5 仓序自动边零条、T4.6 恰索引 1 成员、T4.7 1↔N 无特殊流程）；**绿测证据**：13:26 实跑 `cargo test --locked --test it_core single_member` → `7 passed; 0 failed`（`cadence/notes/2026-11-03_实施_块3_片4_v1.0.md` §5.4） |

### §7 布局规则表、目录树示意与失败面动作（REQ-LAYOUT-01/02/04/05）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 7.1 | ✅ 勾 | `src/product/logical_codebase/failure_copy.rs`（`FailureCopy` 五字段 `src/product/logical_codebase/failure_copy.rs:59-79`、`describe` `:268`、`describe_code` `:287`、`render_detail` `:353`）+ `src/product/logical_codebase/failure_copy_table.inc.rs:14`（`LAYOUT_RULES` 六条规则）、`:78`（`TABLE` 码表）、`:691`（`FAMILY_ENTRIES`）、`:725`（`canonical_key`）；块3 无第二份文案表 |
| 7.2 | ✅ 勾 | `web/src/components/lifecycle/LogicalCodebaseLayoutTree.tsx:44`（「只在 .worktrees/ 下的代码不可被索引」+ 目录树示意）；入口 `web/src/components/lifecycle/AddCodebaseDialog.tsx` / `LogicalCodebaseRegistrationWizard.tsx` |
| 7.3 | ✅ 勾 | `src/product/logical_codebase/failure_copy_table.inc.rs:81-127`（七个对外稳定码逐条 title/reason/action 三段）、`:51-66`（`ALIASES` 内部码 `member_path_outside_root` / `member_symlink_escape` / `nested_worktree` 只作反查）；前端消费 `web/src/state/workspace-cockpit-inbox-items.ts:167` / `web/src/components/lifecycle/LogicalCodebaseRegistrationWizard.test.tsx:8-14` |
| 7.4 | ✅ 勾 | `src/web/types/logical_codebase_bootstrap.rs:24-40`（`FailureCopyDto` 五字段与 `failure_copy::FailureCopy` 一一对应）；`web/src/components/lifecycle/BootstrapFailureLine.tsx` / `web/src/components/lifecycle/LogicalCodebaseRegistrationWizard.tsx` 只渲染成品 |
| 7.5 | ✅ 勾 | `web/src/components/lifecycle/LogicalCodebaseRegistrationWizard.test.tsx:343-345`（逐条断言布局码的失败面文案）；`web/src/state/workspace-cockpit-projection.inbox.test.ts:591-614` |

### §8 入口的依赖前置提示（REQ-LAYOUT-03）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 8.1 | ✅ 勾 | `src/product/logical_codebase/aggregate_index/codegraph_cli.rs:311`（`codegraph_missing`）、`:119`（`codegraph_version_mismatch`）；`src/product/logical_codebase/aggregate_initialization_coordinator.rs:110` 列入依赖前置清单 |
| 8.2 | ✅ 勾 | `web/src/components/lifecycle/ProviderInstallActionButton.tsx:1-10`（块2 片7B 的 provider-cli-installation 入口，**块3 未自带第二套实现/文案**）；消费方 `web/src/components/lifecycle/ProviderCapabilityCard.tsx:19/:301/:351`、`web/src/components/lifecycle/ProviderCapabilityBatchBar.tsx:24/:336` |
| 8.3 | ✅ 勾 | `web/src/components/lifecycle/ProviderInstallActionButton.tsx:4-7`（未安装→【安装】；已安装已核验但版本落后→【更新】+ 当前版本 vs 最新版本）；`web/src/components/lifecycle/ProviderInstallActionButton.test.tsx:157` |

### §9 票3：批量核验异步作业（后端，REQ-PCR-03）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 9.1 | ✅ 勾 | `src/web/app.rs:193-194`（`POST …/capabilities/revalidate-all` → `post_lc_capability_revalidate_all`）、`:197-198`（`GET …/revalidate-all/{job_id}` → `get_lc_capability_revalidate_all_job`）；`src/web/handlers/logical_codebase_capability_batch.rs:51-71` |
| 9.2 | ✅ 勾 | `src/product/logical_codebase/capability_revalidate_batch.rs:25`（`use futures_util::future::join_all`）、`:327`（四家全并发推进）、`:343-345`（`join_all` 真并发，注释「真并发，不串行 await」）、`:309`（`tokio::spawn` 后台作业）；`:6` 模块头「四家并发各跑一次既有单家 revalidate」 |
| 9.3 | ✅ 勾 | `src/product/logical_codebase/capability_revalidate_batch.rs:123-124`（`skipped`：`cli_not_installed` / `probe_unsupported`；`failed`：`probe_failed` / `store_error`）；`src/web/handlers/logical_codebase_capability_batch.rs:299-300`（断言 `status == "skipped"` + `reason == "cli_not_installed"`） |
| 9.4 | ✅ 勾 | `src/product/logical_codebase/capability_revalidate_batch.rs:86-105`（逐项状态 `running`→`confirmed`/`failed`/`skipped`，终态未完成 → `timeout`）、`:230`（durable 存储目录 `capability-revalidate-batch`）、`:51`（作业默认总预算） |
| 9.5 | ✅ 勾 | `src/web/handlers/logical_codebase_capability_batch.rs:279-309`（发起即返回 / skipped / 并发时间戳）；`src/product/logical_codebase/capability_revalidate_batch_tests.inc.rs` |

### §10 票3：一键按钮与逐项进度（前端，REQ-PCR-04）

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 10.1 | ✅ 勾 | `web/src/components/lifecycle/ProviderCapabilityBatchBar.tsx:3-5`（按钮 → POST 立即拿 `job_id`，2.5s 轮询 durable 作业，终态停）、`:217-219`（`storeJobId` + `setJobId`）、`:154`（刷新后按 localStorage `job_id` 续读） |
| 10.2 | ✅ 勾 | `web/src/components/lifecycle/ProviderCapabilityBatchBar.tsx:37`（`running: "核验中"`）、`:276`（`核验中（${done}/${BATCH_SIZE}）` + 禁用）、`:104-105`（逐项状态 `已确认/核验中/核验失败/跳过/超时`） |
| 10.3 | ✅ 勾（B 类：代码实况为据） | `web/src/components/lifecycle/ProviderCapabilityBatchBar.tsx:85-94`（`installEntryKind`：`install`=CLI 不可用→【安装】、`update`=版本落后→【更新】）、`:294`（逐项判定）、`:316-319`（版本落后标记）、`:336-356`（复用块2 `ProviderInstallActionButton`）；失败项「重试」走既有单 provider 端点；用例 `web/src/components/lifecycle/ProviderCapabilityBatchBar.test.tsx:339-370`（「版本落后行渲染『版本落后』+【更新】与当前/最新版本（不是【安装】）」） |
| 10.4 | ✅ 勾 | `web/src/components/lifecycle/ProviderCapabilityBatchBar.tsx:283`（「逐 LC 核验：会真实跑一次探测，可能几分钟，消耗 provider 额度」） |
| 10.5 | ✅ 勾 | `web/src/components/lifecycle/ProviderCapabilityBatchBar.tsx:154/:181/:217`（按 `job_id` 恢复、未知/过期 `job_id` 停轮询）；`web/src/components/lifecycle/ProviderCapabilityBatchBar.test.tsx` |

### §11 回归与门禁

| 编号 | 结论 | 代码位置（文件:行号） |
|---|---|---|
| 11.1 | ✅ 勾（B 类：代码实况为据，绿测待统一跑） | 多仓回归用例齐备：`tests/it_core/single_member_lc_lifecycle.rs`（冷启动/索引/group coding）、`tests/it_core/single_member_path_guard.rs`、`src/web/handlers/automation_target.rs:727-757`、`src/product/logical_codebase/repository_routing_tests.inc.rs:19-60` |
| 11.2 | ✅ 勾（B 类：代码实况为据，绿测待统一跑） | `tests/it_core/large_file_guard.rs`（行数门禁）、`tests/it_core/single_member_path_guard.rs`（隔离/纪律守卫）、`tests/it_core/single_member_lc_lifecycle.rs`；**局部绿测证据**：13:26 `it_core single_member` 7/7 绿（`实施_块3_片4_v1.0.md` §5.4）；**全量绿测输出待 Main 收口统一跑**（详见 §4 风险提示） |
| 11.3 | ✅ 勾 | 前端基线全绿：`pnpm test` 260 files / 2656 tests、`tsc -b` exit 0、`build` exit 0（`cadence/notes/2026-11-03_交付说明_块3_…md` §4 ④，片3/片5 报告） |
| 11.4 | ❌ **不勾** | **真机验收未执行** —— 见 §3「未勾清单」第 4 条 |
| 11.5 | ✅ 勾 | **本轮亲自复跑**：`openspec validate remove-single-repository-path --strict` → `Change 'remove-single-repository-path' is valid`，exit 0 |

---

## 3. 未勾清单（4 条）· 具体原因

### ① 2.4「该错误码的中文文案接入失败面单一来源（块2 `REQ-BOOT-08`）」— 类 C：实现确实没做

- **缺什么**：块2 的失败面文案单一来源 `src/product/logical_codebase/failure_copy.rs` + `failure_copy_table.inc.rs` 的 `TABLE`（`:78`）与 `ALIASES`（`:51`）中**均未登记 `issue_not_bound_to_logical_codebase`**；实测 `grep -rn "issue_not_bound_to_logical_codebase" src/product/logical_codebase/failure_copy*.rs` → **0 命中**。该错误码目前只有英文 message（`src/web/handlers/support.rs:239-244`）与 HTTP 状态映射（`src/web/error.rs:178` → 400），**没有中文失败面三段式文案**。
- **归属切片**：块3 片1（§① B 组，`src/product/issue_store.rs` / `src/web/error.rs` / `src/web/handlers/support_parts/product_store_error.inc.rs`）。但**文案本体在块2 侧**（`failure_copy_table.inc.rs`），块3 不得自建第二份文案表 ⇒ 需要「块2 码表登记该码」或「owner 裁定该码不在失败面范围」。
- **注意（可能的口径分歧）**：`REQ-BOOT-08` 的正文范围是「**聚合初始化与聚合索引的失败面，以及 `logical-codebase-layout-guidance` 引用的登记布局失败面**」（`openspec/changes/fix-aggregate-cold-start-and-audit-budget/specs/non-interrupt-repository-bootstrap/spec.md:109-111`），**不含**路由归属类错误码；块3 的 spec delta 也没有要求该码进失败面文案（`openspec/changes/remove-single-repository-path/specs/` 全文无「该码进 failure_copy」的要求）。因此本条要么**需要一次 owner 裁定把它移出范围 / 改写任务文字**，要么**需要在块2 码表补登记**。**在裁定落地前不勾**（不按「看起来该做了」处理）。
- **报告口径**：块3 交付说明 §3.3 备注与 §5 偏离表第 8 条已如实标注「出处待核 / T1.5 未编写」，**没有虚报已完成**；块3 片1 报告全文 `grep "failure_copy|文案|失败面"` → 0 命中，即该片**从未声称做过**。

### ② 4.4「用例：对话框无单仓选项；无 `logical_codebase_id: null` 的提交路径；缺 LC 提交被稳定错误码拒绝」— 类 D：三个子项里第三个的用例未编写

- **已落地**：子项①（无单仓选项）→ `web/src/components/lifecycle/AddCodebaseDialog.test.tsx:13`、`CreateLifecycleIssueDialog.test.tsx:68`；子项②（无 `null` 提交路径）→ `CreateLifecycleIssueDialog.test.tsx:19`（payload 恒带 `logical_codebase_id`）、后端 `src/web/types.rs:468-470`（`logical_codebase_id: String` 必填）。
- **缺什么**：子项③「缺 LC 提交被**稳定错误码**拒绝」的**用例未编写**（块3 片1 报告 §⑤ concerns 3 明列「T1.5（缺 LC 提交 4xx 显式用例）未编写」）。且实测：创建路径缺 `logical_codebase_id` 字段时走 axum `Json` 提取器的 **serde 缺字段反序列化 4xx（通用 422）**，**不是**字面稳定码 `issue_not_bound_to_logical_codebase`——后者只在「读取老记录缺归属」（`src/product/issue_store.rs:195`）与「载体判定无归属」（`src/web/handlers/support.rs:239`）两条路径产出。
- **归属切片**：块3 片1（§① F/G 组）收尾补一条 IT 用例（`POST /projects/{id}/issues` 缺 `logical_codebase_id` → 断言 4xx 且不落库）。
- **为何不勾**：子项③既无用例、也未达「稳定错误码」字面（是通用 422）。**不按「结构上已闭环」勾选**。

### ③ 5.2「确认改造后的既有『添加代码库』对话框即『建逻辑代码库』入口（**真机走一遍**），不新造页面」— 类 E：真机未走

- **已落地**：「不新造页面」—— `web/src/components/lifecycle/AddCodebaseDialog.test.tsx:49`（「创建成功后同弹窗进入登记，复用聚合根自动发现（建 LC 入口不新造页面）」）。
- **缺什么**：**「真机走一遍」未执行**（块3 交付说明 §3.3 C 组：该项的真机部分并入 11.4）。
- **归属切片**：controller / 真机验收（块3 Plan §5.1）。
- **为何不勾**：任务文字明确要求真机动作；真机动作未发生，不能用「测试钉住」替代。

### ④ 11.4「真机验收：新建项目直接多仓；1 成员 LC 全链通过；3 成员 LC 仓序正确；六条布局规则各给动作；一键核验 4 家并发出进度、失败可单项重试；核验后重试聚合初始化跑通」— 类 E：六项真机验收一项未做

- **缺什么**：上列**六项真机验收全部未执行**（块3 交付说明 §3.3 B 组：「一项未做」，归属合并后 controller）。
- **归属切片**：controller / 真机验收（块3 Plan §5.1）。
- **为何不勾**：这是人工验收活动，不是「代码与用例已落地、只差绿测输出」那一类；无任何真机执行记录。

---

## 4. 勾选滞后项的重判结果

本 change 在 09:40 的对证快照里把 **22 条**判为未勾（「待切片落地 18 / 部分完成 4」）。本轮**不照抄该快照**，逐条以当前代码重判：

| 原快照分类 | 条数 | 本轮重判结果 |
|---|---|---|
| 待切片落地（对证时无代码） | 18 | **17 勾**（1.1 / 1.2 / 1.3 / 1.4 / 1.5 / 1.6 / 2.1 / 2.2 / 2.3 / 2.5 / 3.1 / 3.2 / 3.3 / 3.5 / 11.1 / 11.2 / **3.4**）+ **1 不勾**（2.4） |
| 部分完成 | 4 | **3 勾**（3.4 / 10.3 / **4.4 仍不勾**——子项③用例未写）+ 3.4 已在上面计入 |
| 未证实 | 0 | — |

> 口径说明：3.4 在快照里属「部分完成」，本轮归入「已落地」；上表两行有交叠，**实际新增勾选 18 条**。

**逐条重判要点**：

1. **3.1（全仓 0 命中 + 2 处 `#[cfg(test)]` 必处置）** — 简报预告「现已全仓 0 命中、变体已删 ⇒ 查证后应可勾」，**实测确认**：`grep -rn "RepositoryRouting::Legacy" src/ web/src/ --include=*.rs --include=*.ts --include=*.tsx` → **0**；两处 test-only 命中（`group_validation.rs:682-684`、`coding_cleanup.inc.rs:161-185`）均已改写为 `Logical`/`FailClosed`，**未留下任何 `Legacy` 引用** ⇒ **勾**。
2. **1.5（红线核对）** — 对证快照归为「待切片落地」，实测红线四项全部在库（`migrate_legacy` 34 处 / `logical_codebase_root` 56 处 / `lc_partition.rs:7/:36-44` 别名仓双路扫描 / `repository_routing_resolver.inc.rs:198-222`/`:347-358` 布局冲突判定）；残留 `legacy` 命名经逐处核实**全属布局语义**，无单仓路由语义 ⇒ **勾**（代码实况为据，报告未覆盖逐处核实结论）。
3. **3.4（前端载体收窄）** — 实测 `EnrollmentCarrierTarget`（`web/src/api/types/lifecycle.ts:350-365`）、守卫 `carrierTargetFromEnrollmentTarget`（`automation-reenable.ts:63-85`）、`LEGACY_CARRIER_BLOCKED_MESSAGE`（`:87-91`）、`WorkItemPlanOptionsDialog.tsx:132-137` 禁用提交路径、legacy 只读渲染（`:583-590` + `workspace-cockpit-inbox-items.ts:56`）、禁删对象（`types.rs:31` / `repository_routing_types.inc.rs:151-152`）全部在库 ⇒ **勾**。**任务文字一字未动**（已按块3 Plan §6 R19 更正）。
4. **10.3（版本落后引导【更新】）** — 实测 `installEntryKind`（`ProviderCapabilityBatchBar.tsx:85-94`）+ 版本落后标记（`:316-319`）+ 复用块2 组件（`:336-356`）+ 用例（`ProviderCapabilityBatchBar.test.tsx:339`）在库 ⇒ **勾**。
5. **11.1 / 11.2** — 属「待统一绿测」类：代码与用例均已落地，只差统一全量的绿测输出 ⇒ 按判据补充**打勾**，但见下方风险提示。

### ⚠️ 风险提示（收口必看）

- **11.1 / 11.2 若收口统一全量跑红，必须改回未勾**。判据补充明确写明「若绿测结果证明实现有缺陷 ⇒ 把该条改回未勾」。当前证据：全量 `cargo test --all-targets` 尚无绿测输出（最近一次全量 lib 跑为 10:17，因 28 个编译错误终止，0 条 `test result:`）；`it_core` 全量在 12:49 轮为 `121 passed / 77 failed`（77 失败位于 `workspace_ws_integration` 等其它切片，FailureTriage 在收）。
- **11.2 还含行数门禁**：Main 已通报 3 个文件零余量（`src/web/workspace_ws_handler/tests/provider_run_events.rs` 1257/1257、`single_candidate_lc_admission.rs` 2638/2638、`src/web/coding_ws_handler/tests/failed_review_recovery/runner.rs` 1242/1242）——`cargo fmt` 会改行数，收口重核门禁时若变红，11.2 同样要改回未勾。

---

## 5. 「报告说做了、代码里找不到」条目

| 条目 | 结论 |
|---|---|
| **2.4** | 块3 交付说明 §3.3 与 §5 偏离表第 8 条**如实标注「出处待核」**，块3 片1 报告全文**未提及** failure_copy / 文案 / 失败面（`grep` 0 命中）⇒ **不存在虚报**，但也**确实没做**。这是本轮唯一「任务要求有、实现确无」的条目。 |
| 其余 17 条本轮新勾 | 全部**逐条查到代码位置**（见 §2 表），无一条是「只信报告」。 |
| 4.4 | 报告（交付说明 §3.3 C 组）自述「前两项完成、第三项结构性闭环但字面稳定码未验、T1.5 未编写」——**报告口径诚实**，未虚报。 |
| 5.2 / 11.4 | 报告自述「真机未做」——**未虚报**。 |

**结论：本 change 无「实施报告声称已完成、但代码里找不到产物」的条目。** 唯一落差是 2.4（任务要求有、实现确无、报告已自认待核）。

---

## 6. 收尾自证

```
$ ls -la openspec/changes/remove-single-repository-path/tasks.md
-rw-r--r-- 1 michaelche michaelche 18514 Nov  3 17:16 openspec/changes/remove-single-repository-path/tasks.md

$ ls -la cadence/notes/2026-11-03_任务补勾_remove-single-repository-path_v1.0.md
-rw-r--r-- 1 michaelche michaelche 32936 Nov  3 17:19 cadence/notes/2026-11-03_任务补勾_remove-single-repository-path_v1.0.md

$ wc -l openspec/changes/remove-single-repository-path/tasks.md cadence/notes/2026-11-03_任务补勾_remove-single-repository-path_v1.0.md
  135 openspec/changes/remove-single-repository-path/tasks.md
  229 cadence/notes/2026-11-03_任务补勾_remove-single-repository-path_v1.0.md
  364 total

$ grep -c '^- \[x\]' openspec/changes/remove-single-repository-path/tasks.md
48

$ grep -c '^- \[ \]' openspec/changes/remove-single-repository-path/tasks.md
4
```

文件行数 **135**，与改前一致；**48 已勾 / 4 未勾 / 合计 52**。

**只改勾选框的自证**：改后任务的 52 个编号序列 `1.1 … 11.5` 完整且顺序不变，文件总行数 135 与改前一致，除 `- [ ]` → `- [x]` 外**无任何文本、编号、顺序、行增删改动**。
