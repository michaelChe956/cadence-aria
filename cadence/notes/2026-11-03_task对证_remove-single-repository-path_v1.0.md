# task 对证清单：remove-single-repository-path（块3）

- 日期：2026-11-03
- 对证对象：`openspec/changes/remove-single-repository-path/tasks.md`（**共 52 条 task**，第 1~11 组）
- 工作目录：`/home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02`
- 对证人：TaskVerifyBlock3（只读核验，不改源码）
- 证据来源：① 块3 实施报告（片2/片3/片4/片5/片6，见下）；② **本人亲自打开的代码**（`src/`、`web/src/`、`tests/`）；③ 本 change 的 5 份 spec delta；④ 块3 实施 Plan
- 判定规则：**报告有明确证据**（改动位置 / 测试输出 / 文件清单）**且** **本人亲眼确认产物在代码里存在** ⇒ `- [x]`；否则保持 `- [ ]` 并归入三类之一（待切片落地 / 部分完成 / 未证实）
- ⚠️ **本清单只改 `tasks.md` 的勾选框**：任务文字、编号、顺序、其它段落一字未动；未改源码 / spec / design / proposal / Plan；未 `git add` / `git commit`

---

## 1. 统计

| 分类 | 条数 | 占比 |
|---|---|---|
| **task 总数** | **52** | 100% |
| **已勾选 `- [x]`** | **30** | 57.7% |
| **未勾选 `- [ ]`** | **22** | 42.3% |
| ├─ 待切片落地 | 18 | — |
| ├─ 部分完成 | 4 | — |
| └─ 未证实 | **0** | — |

### 已勾选（30 条）按组分布

| 组 | 已勾选 | 未勾选 | 说明 |
|---|---|---|---|
| §1 路由三态→两态（1.1~1.6） | 0 | 6 | 块3 片1 **在途** |
| §2 issue 归属收敛（2.1~2.5） | 0 | 5 | 块3 片1 **在途** |
| §3 32 处改写与入口对齐（3.1~3.5） | 0 | 5 | 片1 在途（3.1~3.3/3.5）；3.4 部分完成 |
| §4 前端单仓入口删除（4.1~4.4） | 3 | 1 | 片2 落地；4.4 部分完成 |
| §5 新建项目直接多仓（5.1~5.4） | 3 | 1 | 片2 落地；5.2 部分完成 |
| §6 1 成员与 N 成员同路径（6.1~6.5） | 5 | 0 | 片4 落地（仅加测试） |
| §7 布局规则表与失败面（7.1~7.5） | 5 | 0 | 片5 落地 |
| §8 入口依赖前置提示（8.1~8.3） | 5→3 | 0 | 片5 落地 |
| §9 票3 后端异步作业（9.1~9.5） | 5 | 0 | 片6 落地 |
| §10 票3 前端一键按钮（10.1~10.5） | 4 | 1 | 片6 落地；10.3 部分完成 |
| §11 回归与门禁（11.1~11.5） | 2 | 3 | 11.3 / 11.5 已勾；11.1/11.2/11.4 未勾 |

### 三个重点核对项的裁定（题面特别要求）

| 重点项 | 裁定 | 依据 |
|---|---|---|
| **「32 处出现点（26 消费 + 6 构造）/ 19 个文件」** | **未勾选（3.1）→ 待切片落地** | 本人实跑题面给出的清单命令：**生产口径仍为 32 处 / 19 个文件**（与题面数字完全一致 ⇒ 清单本身有效），且 `RepositoryRouting::Legacy` 变体定义仍在 `src/product/logical_codebase/repository_routing_types.inc.rs:47`。块3 片1 的 34 处改写**还在途**，按题面要求处理。 |
| **「2 处 `#[cfg(test)]` 命中必须处置」** | **未勾选（3.1）→ 待切片落地** | 两处仍在原位：`src/product/coding_attempt_store/group_validation.rs:695`（构造点）、`src/web/handlers/support_parts/coding_cleanup.inc.rs:171`（匹配臂）。同属 3.1 清单，随片1 一并处置。 |
| **「块3 片1 正在做 34 处改写」** | **§1 全组 + §2 全组 + 3.1/3.2/3.3/3.5 + 11.1/11.2 全部按「待切片落地」处理** | 片1 **无实施报告落盘**（`cadence/notes/` 下只有片2/3/4/5/6 的 `2026-11-03_实施_块3_片N_v1.0.md`）；代码侧片1 处于**中间态**：RED 测试已写入（`src/product/logical_codebase/repository_routing_tests.inc.rs:16/63/141` 三条新用例），但生产代码未改 ⇒ crate 当前编译不过（与 Main 的 cargo 通报一致）。 |

---

## 2. 逐条对照表（52 条）

> 图例：✅ 已完成（已勾选）｜⏳ 待切片落地｜◐ 部分完成｜❓ 未证实

### §1 路由三态 → 两态，`Legacy` 变体整个移除（映射 `REQ-ROUTE-01`）

| # | 结论 | 证据出处（报告:行号 + 代码:行号） |
|---|---|---|
| 1.1 | ⏳ 待片1 | 报告：**无片1 实施报告**；代码：`src/product/logical_codebase/repository_routing_types.inc.rs:47` 仍有 `Legacy { repository_id: String }`；`:100-102` `(None,None) => RepositoryRouting::Legacy` |
| 1.2 | ⏳ 待片1 | 代码：`repository_routing_types.inc.rs:63-72` `resolve_issue_logical_codebase_id` 仍返回 `Result<Option<String>, _>`，`Err(NotFound) => Ok(None)` 未删 |
| 1.3 | ⏳ 待片1 | 代码：`repository_routing_types.inc.rs:76-97` `issue_codebase_stores` 仍有 `None =>` 的 project 级 store 回退分支 |
| 1.4 | ⏳ 待片1 | 代码：`repository_routing_types.inc.rs:100-102` `(None, None)` 仍落 `Legacy`（非 `FailClosed`） |
| 1.5 | ⏳ 待片1（红线核对须在片1 落地后重做） | 本人**当前**核实红线对象零改动：`src/product/logical_codebase/store.rs:437` `migrate_legacy` 在库；`src/product/app_paths.rs:76` `logical_codebase_root` 在库；`src/product/dangling_reference/patrol/lc_partition.rs` **grep `Legacy` 命中 0**；`src/product/logical_codebase/repository_routing_resolver.inc.rs` **grep `Legacy` 命中 0**。但片1 会动同一批文件 ⇒ 核对须在片1 落地后重做 |
| 1.6 | ⏳ 待片1 | 代码：`src/product/logical_codebase/repository_routing_tests.inc.rs:16` `classify_none_none_fails_closed_without_legacy_arm` / `:63` `resolve_issue_logical_codebase_id_returns_string_for_bound_issue` / `:141` `issue_codebase_stores_scopes_to_lc_subtree` **已写入但仍是 RED**（`:74` 用 `logical_codebase_id: "…".to_string()`，而 `src/product/issue_store.rs:13` 字段仍是 `Option<String>` ⇒ 编译不过） |

### §2 issue 归属字段收敛与自动化载体判定（映射 `REQ-ROUTE-03`、`REQ-ROUTE-C5-TARGET`）

| # | 结论 | 证据出处 |
|---|---|---|
| 2.1 | ⏳ 待片1 | 代码：`src/product/issue_store.rs:13` `pub logical_codebase_id: Option<String>`；`:26` `CreateProductIssueWithRepositoryInput` 同；`:131/:154` 写盘仍可 `None` |
| 2.2 | ⏳ 待片1 | 代码：`src/product/issue_store.rs:71-81` `get()` 直接 `read_json`，**无**「缺归属 → `issue_not_bound_to_logical_codebase`」校验；仅 `:329-364` 有 RED 测试 `old_issue_record_without_lc_id_reports_explicit_error` |
| 2.3 | ⏳ 待片1 | 代码：`src/web/handlers/support.rs:210` `enum AutomationCarrierResolution` 仍有 `SingleRepository` 变体；`src/web/handlers/automation_enrollment.rs:384`、`:398`、`src/web/handlers/automation_target.rs:93`、`:101`、`src/web/advance_plan.rs:219`、`:253`、`src/web/coding_start.rs:832` 仍有单仓载体判定臂 |
| 2.4 | ⏳ 待片1 | 代码：`grep -rn "issue_not_bound_to_logical_codebase" src/ web/src/` **只命中 `src/product/issue_store.rs:297`（注释）与 `:329`（测试断言）**；`src/web/error.rs` 稳定码→状态表**无**该码；`web/src/` 无中文文案 |
| 2.5 | ⏳ 待片1 | 同 2.2/2.3/2.4：失败面 4xx 与「不写 enrollment」的用例均属片1 用例组，尚未落地 |

### §3 生产 32 处 / 19 文件的 `Legacy` 出现点逐处改写与全部入口对齐（映射 `REQ-ROUTE-01`、`REQ-ROUTE-02`）

| # | 结论 | 证据出处 |
|---|---|---|
| 3.1 | ⏳ 待片1（**重点项**） | 报告：**无片1 实施报告**；代码（本人实跑题面命令）：**仍 32 处 / 19 个文件**。分布：`workspace_repository.rs`×4、`handlers/lifecycle.rs`×3、`handlers/coding/group.rs`×3、`coding_attempt_store/group_validation.rs`×3、`coding_attempt_store/admission.rs`×3、`support_parts/coding_cleanup.inc.rs`×2、`coding_attempt_repository.rs`×2，其余 11 文件各 1 处。枚举定义 `repository_routing_types.inc.rs:47`。两处 `#[cfg(test)]` 命中仍在：`group_validation.rs:695`（构造点）、`coding_cleanup.inc.rs:171`（匹配臂） |
| 3.2 | ⏳ 待片1 | 代码：题面点名的 8 个入口**全部仍命中** `RepositoryRouting::Legacy` —— `src/product/workspace_repository.rs:91/177/232/260`（`workspace_repository_for_session` 所在文件）、`src/web/handlers/coding/group.rs:318/406/518`、`src/web/handlers/coding.rs:118`（删除路径）、`src/web/workspace_context/builder.rs:136`（`coding_ws_handler/context.rs`）、`src/web/handlers/coding/repository_resolution.rs:4`（`resolve_work_item_repository`）、`src/product/coding_evaluation_context/builder.rs:297`、`src/web/workspace_ws_handler/socket.rs:240`、`src/product/workspace_engine/draft_batch/compile_support.rs:84` |
| 3.3 | ⏳ 待片1 | 代码：`src/web/handlers/product_resources.rs:158-216` `create_product_issue` 仍有 `None` 单仓分支 → `find_repository(...)`（物理存在性校验）+ `logical_codebase_id: None`；`:177-179` 仍强制 `repository_required` |
| 3.4 | ◐ **部分完成** | 报告：片3 §0（口径裁决）+ §1.1/§1.2/§1.3；代码：**发起侧已收窄** —— `web/src/api/types/lifecycle.ts:350` 新增 `EnrollmentCarrierTarget`（仅 LC 两变体）、`:432`/`:453` 命令 target 改用它、`web/src/components/lifecycle/automation-reenable.ts:63` `carrierTargetFromEnrollmentTarget`（`single_repository` → `null`）、`:88` `LEGACY_CARRIER_BLOCKED_MESSAGE`；**投影侧变体与消费分支按「方案 A」全部保留** —— `web/src/api/types/lifecycle.ts:371`、`web/src/api/types/work-item-plan.ts:679`、`web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:589`、`web/src/state/workspace-cockpit-inbox-items.ts:56`。⇒ **「删除类型变体及其消费分支」这一半未做**（Plan §6 R19 已预登记该冲突并建议改写 tasks 措辞） |
| 3.5 | ⏳ 待片1 | 各入口一致性 / fail-closed 4xx 用例属片1 用例组，尚未落地（3.1 的 32 处未改 ⇒ 入口行为必然仍分叉） |

### §4 前端单仓入口删除（映射 `REQ-REG-16`、`REQ-MRE-01`）

| # | 结论 | 证据出处 |
|---|---|---|
| 4.1 | ✅ 已完成 | 报告：片2 §1.1（`:7` 类型删 / `:18`/`:24` prop 删 / `:31` state 删 / `:100-119` radio 删 / `:131-140` 按钮删）+ §2 表 #1#2；代码：`web/src/components/lifecycle/AddCodebaseDialog.tsx` 现 138 行，`grep "单仓库\|single_repo\|radio\|继续添加"` **命中 0**；`web/src/components/lifecycle/AddCodebaseDialog.test.tsx:13`「无单仓入口：查不到「单仓库」radio，也查不到「继续添加单仓库」按钮」、`:33`「默认直接渲染多仓表单」 |
| 4.2 | ✅ 已完成 | 报告：片2 §1.2 + §2 表 #3#4；代码：`web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx:15` `logical_codebase_id: string`（必填）、`:121` `setRepositoryError("请选择逻辑代码库")`、`:203` 下拉只列 `logical`、`:148` `base_branch: null`；测试 `:68`「无「单仓库代码库」选项」、`:99`「未选逻辑代码库提交被拒：不调用 onCreate」 |
| 4.3 | ✅ 已完成 | 报告：片2 §1.6（6 个测试文件改写清单）+ §3 T2.1~T2.9 红→绿；代码：`AddCodebaseDialog.test.tsx`（4 例，`:13/:33/:49/:142`）、`CreateLifecycleIssueDialog.test.tsx`（10 例）、`IssueLifecycleWorkbench.codebases.test.tsx:69-72`、`IssueLifecycleWorkbench.crud.test.tsx:107`、`IssueLifecycleWorkbench.lc-partition.test.tsx:320/349/351`、`web/src/onboarding/steps.test.ts:64-75` —— **测试与组件同步改写，不是只改组件** |
| 4.4 | ◐ **部分完成** | 前两项 ✅：`AddCodebaseDialog.test.tsx:13`（无单仓选项）、`CreateLifecycleIssueDialog.test.tsx:68`（下拉只列逻辑代码库）+ `:119`（断言 `logical_codebase_id` + primary）、payload 类型 `logical_codebase_id: string`（无 `null` 路径）。**第三项「缺 LC 提交被稳定错误码拒绝」未做** —— 前端只做**本地文案**拒绝（`CreateLifecycleIssueDialog.tsx:121`「请选择逻辑代码库」），**稳定错误码 `issue_not_bound_to_logical_codebase` 在 `src/web/error.rs` 码表与 `web/src/` 中均不存在**（见 2.4）⇒ 该项归属片1 后端 |

### §5 新建项目直接多仓，「建逻辑代码库」为独立步骤（映射 `REQ-REG-16`）

| # | 结论 | 证据出处 |
|---|---|---|
| 5.1 | ✅ 已完成 | 报告：片2 §1.4 + §2 表 #5；代码：`web/src/onboarding/steps.ts:74` `id: "add-logical-codebase"`、`:76` `title: "建逻辑代码库（多仓）"`、`:78`「新建项目流程固定为：创建项目 → 建逻辑代码库（选聚合根 + 登记成员仓）→ 创建 issue。」、`:85`「创建 Issue（绑定逻辑代码库）」；`web/src/components/lifecycle/CreateProjectDialog.tsx` grep `logical\|聚合根\|member` **命中 0**（表单内无 LC 字段 ⇒ 界面无单仓选项）；`web/src/onboarding/steps.test.ts:64-75` 断言「不再出现「单仓」路径」 |
| 5.2 | ◐ **部分完成** | 「不新造页面」✅：改造后的既有「添加代码库」对话框即建 LC 入口 —— `web/src/components/lifecycle/AddCodebaseDialog.tsx` 恒渲染多仓表单（无新页面）；`web/src/components/lifecycle/AddCodebaseDialog.test.tsx:49`「创建成功后同弹窗进入登记，复用聚合根自动发现（建 LC 入口不新造页面）」。**「真机走一遍」未做**：片2 报告 §3/§4 只有 vitest 命令输出，全仓库**无真机走查记录**；真机口径归属 task 11.4（Plan §5.1，合并后 controller 执行） |
| 5.3 | ✅ 已完成 | 报告：片2 §1.1/§1.3 + 片5 §1 表 #3；代码：聚合根选择 + 名称 = `AddCodebaseDialog.tsx`（多仓表单）；成员登记 = `web/src/components/lifecycle/LogicalCodebaseRegistrationWizard.tsx`（409→550 行，`:25` 树 import、`:306` `<LogicalCodebaseLayoutTree/>`）；聚合初始化 = `web/src/components/lifecycle/LogicalCodebaseBootstrapCard.tsx:25/:270`；**创建项目表单内无 LC 字段**（`CreateProjectDialog.tsx` grep 命中 0）⇒ 「不得塞进创建项目表单」✅ |
| 5.4 | ✅ 已完成 | 报告：片2 §3 T2.9 + 片5 §1.1 C5；代码：全新项目直接多仓 = `steps.ts:78` + `AddCodebaseDialog.test.tsx:33`；未建 LC 时无法创建 issue = `web/src/components/lifecycle/IssueLifecycleWorkbenchView.tsx:77-91` `createIssueGate`（无 `member_count > 0` 的 LC ⇒ `canCreate: false` + 原因文案）、`:121-127` 渲染禁用态与原因；建 LC 步骤可独立进入并完成登记 = `AddCodebaseDialog.test.tsx:49` + `IssueLifecycleWorkbench.lc-partition.test.tsx:320` |

### §6 1 成员与 N 成员走完全同一套路径（映射 `REQ-REG-17`）

| # | 结论 | 证据出处 |
|---|---|---|
| 6.1 | ✅ 已完成 | 报告：片4 §2 第 6 点 + §4.2（无排除扫描输出：`src/web/advance_plan.rs:1565`、`src/web/workspace_ws_handler/tests/single_candidate_lc_admission.rs:470` 两处命中**均为测试代码**，排除后 `scanned production files: 1222 … hits: 0`）；代码：清单已固化为守卫模式表 —— `tests/it_core/single_member_path_guard.rs:377` `members.len() == 1`、`:382` `member_ids.len() == 1`、`:387` `member_count == 1`、`:392` `require_multiple_members`、`:397` `min_members`、`:402` `minimum_member`、`:473` `minimum_member_count`、`:482-493` 中文拒绝串；路径剥离器 `:146` `is_test_path_segment` / `:154` `is_test_file_name` / `:208` `strip_inline_test_code`（区分生产分支与测试夹具断言） |
| 6.2 | ✅ 已完成 | 报告：片4 §4.2（生产代码零命中 ⇒ **无门禁可移除**，任务即「移除全部」已由「本就没有 + 锁死」满足）；代码：`single_member_path_guard.rs:552` `no_gate_requires_at_least_two_members` 守卫 + `:353` `assert_negative_control`（合成违规必被检出，防空扫假绿） |
| 6.3 | ✅ 已完成 | 报告：片4 §2 第 1/2/3 点（`bootstrap_projector.inc.rs` 的 `active_members >= 1` 判据、`compile_support.rs::change_order_pairs` 零分支双重循环、`member_checkout_prefixes.rs` 无成员数分支）；代码：守卫 `single_member_path_guard.rs:544` `no_production_branch_special_cases_single_member`；自然退化证据 = `tests/it_core/single_member_lc_lifecycle.rs:503` `single_member_change_order_has_zero_auto_edges`、`:618` `single_member_indexes_one_member`、`:528` `change_order_auto_edge_rule_source_lock`（规则源唯一 + 零 `if`/`== 1` + 恰两个消费点） |
| 6.4 | ✅ 已完成 | 报告：片4 §3 不变量 A/B/C；代码：`single_member_path_guard.rs:544` / `:552` / `:560` 三条源码级断言（每条前置阴性对照 `:353`），并注册于 `tests/it_core.rs:44-47`（`mod single_member_lc_lifecycle` + `mod single_member_path_guard`） |
| 6.5 | ✅ 已完成 | 报告：片4 §3 不变量 D~G + §4.1（T4.8 假绿防护：注册后 `-- --list` 列出 7 个用例）；代码：`tests/it_core/single_member_lc_lifecycle.rs:477` `single_member_lc_registration_and_first_build_reaches_planning_ready`（聚合初始化五步全 Completed）、`:503`（仓序自动边零条）、`:618`（成员/聚合索引各 1）、`:662` `single_member_count_change_needs_no_special_flow`（1→N 无迁移、无「切回单仓」路径） |

### §7 布局规则表、目录树示意与失败面动作（映射 `REQ-LAYOUT-01/02/04/05`）

| # | 结论 | 证据出处 |
|---|---|---|
| 7.1 | ✅ 已完成 | 报告：片5 §1 范围声明（0 处 Rust 改动、不建文案表）+ §6.1 grep 自证（`layout_guidance|layout-guidance|LayoutFailureAction` 命中 3 处**全是注释**、0 处可执行引用；`ls` 四个目标文件均 `No such file or directory`）+ §3.8；代码：块2 单一来源 `src/product/logical_codebase/failure_copy.rs:464-470` 已登记 7 个对外稳定码；块3 侧 `LogicalCodebaseLayoutTree.test.tsx` T5.3 源码级锁（`slice5_files_contain_no_layout_copy_constants`） |
| 7.2 | ✅ 已完成 | 报告：片5 §1 表 #1/#3 + §2.1/§2.2；代码：`web/src/components/lifecycle/LogicalCodebaseLayoutTree.tsx`（43 行）：`:24`「─ 逻辑代码库根（普通空目录，不是 git 仓）」、`:30`「─ 成员仓（直接子目录）」、`:44`「成员仓代码要放在仓库根或普通子目录；只在 .worktrees/ 下的代码不可被索引。」；挂载两处 —— 登记入口 `LogicalCodebaseRegistrationWizard.tsx:306`、聚合初始化卡片 `LogicalCodebaseBootstrapCard.tsx:270` |
| 7.3 | ✅ 已完成 | 报告：片5 §1 表 #4 + §2.3（T5.7 七码逐条，`:405` ×7）；代码：`LogicalCodebaseRegistrationWizard.tsx:69-96` `readFailureCopy`（逐字段 `isNonEmptyText` 校验 `title_zh`/`reason_zh`/`action_zh`/`machine_line` + `isFailureCopyTriage` 校验 `triage`）、`:100-107` 三段映射、`:504/511/519/527` 四个独立元素（`lc-registration-error-title`/`-reason`/`-action`/`-machine-line`）；测试 `:8-14` 七个码常量 + `it.each` 逐条 |
| 7.4 | ✅ 已完成 | 报告：片5 §1 表 #4（任一字段不合法 ⇒ `null`，**不**降级验收）+ §5 C3；代码：`readFailureCopy` 五字段必填校验（同 7.3）；块3 零 Rust 改动 ⇒ 未另抄文案表（§6.1 `ls` 自证四个文件均未建） |
| 7.5 | ✅ 已完成 | 报告：片5 §2.3（T5.7 七码逐条，**不抽样**）+ §2.4（T5.4 入口渲染树）；代码：`LogicalCodebaseRegistrationWizard.test.tsx:343-405`（`it.each` 七码逐条断言三段文案）、`:334` T5.4 找 `lc-layout-tree`；`LogicalCodebaseLayoutTree.test.tsx` 3 例（`:tree_shows_root_and_direct_children` / `tree_explains_worktrees_are_not_indexable` / `slice5_files_contain_no_layout_copy_constants`） |

### §8 入口的依赖前置提示（codegraph）（映射 `REQ-LAYOUT-03`）

| # | 结论 | 证据出处 |
|---|---|---|
| 8.1 | ✅ 已完成 | 报告：片5 §6 表 #1/§2.3（T5.8 两个用例）；代码：`web/src/components/lifecycle/LogicalCodebaseRegistrationWizard.test.tsx:421` `codegraph_missing_shows_install_entry`、`:449` `codegraph_version_mismatch_shows_update_entry`；文案取自 `details.copy`（与块2 同源），**块3 未自建任何文案常量** |
| 8.2 | ✅ 已完成 | 报告：片5 §6 表 #5（复用块2 组件、`capabilityConfirmed` 传值、`onSettled` 重跑 `getInstallTargets()`、不自拼 `acknowledge_remote_script`）；代码：`LogicalCodebaseRegistrationWizard.tsx:26` `import { ProviderInstallActionButton }`、`:538` `<ProviderInstallActionButton` —— 组件本体 `web/src/components/lifecycle/ProviderInstallActionButton.tsx` 属块2 交付，块3 **零改动、自带零安装实现与文案** |
| 8.3 | ✅ 已完成 | 报告：片5 §6 表 #5（`provider-install-verify-codegraph` 不得出现，说明版本不符走【更新】分支）；代码：`ProviderInstallActionButton.tsx:165-166`（`capabilityConfirmed && versionGap < 0` ⇒ 更新）、`:192`「安装」、`:219`「更新」、`:224` `provider-install-versions-{target}`（当前 vs 要求版本）；测试 `LogicalCodebaseRegistrationWizard.test.tsx:442` `provider-install-start-codegraph`、`:445` `provider-install-manual-command-codegraph`、`:474` `provider-install-start-codegraph`、`:477` `provider-install-versions-codegraph`、`:481` `queryByTestId("provider-install-verify-codegraph")` 为 null |

### §9 票3：批量核验的异步作业（后端）（映射 `REQ-PCR-03`）

| # | 结论 | 证据出处 |
|---|---|---|
| 9.1 | ✅ 已完成 | 报告：片6 §1.1/§1.3/§1.5 + §2 生命周期图；代码：`src/web/app.rs:193` `POST .../capabilities/revalidate-all`、`:197` `GET .../capabilities/revalidate-all/{job_id}`；`src/web/handlers/mod.rs:99` `mod logical_codebase_capability_batch;`、`:164-165` 两个 handler 导出；`src/web/handlers/logical_codebase_capability_batch.rs`（324 行）；作业结构 `src/product/logical_codebase/capability_revalidate_batch.rs:176-190` `CapabilityRevalidateBatchJob{job_id,status,results[]}`；`:294` `job_id = "capability_batch_{uuid}"`；`start()` 内 `tokio::spawn` ⇒ 立即返回 |
| 9.2 | ✅ 已完成 | 报告：片6 §4（四家并发的实现方式）+ §3.3 T6.2；代码：`capability_revalidate_batch.rs:25` `use futures_util::future::join_all;`、`:339-345` `join_all(futures)` + `tokio::time::timeout(self.job_deadline, ...)`（**非 `for` 内逐家 `await`**）；`revalidate_call_for(...)` **只转发**既有单家 `revalidate`（单家语义未改） |
| 9.3 | ✅ 已完成 | 报告：片6 §1.1 状态映射表 + §3.3 T6.3/T6.5；代码：`capability_revalidate_batch.rs:59` `BATCH_REASON_CLI_NOT_INSTALLED = "cli_not_installed"`、`:55` `already_confirmed`、`:67` `job_deadline_exceeded`；`:90` `BatchItemStatus { Running, Confirmed, Failed, Skipped, Timeout }`；`classify_outcome` 把 `Err(ProbeFailed{action:None})` 映成 `skipped`（CLI 不可用）、`Err(ProbeFailed{action:Some})` 映成 `failed`（不阻断其它家） |
| 9.4 | ✅ 已完成 | 报告：片6 §1.1（durable 文件 `…/capability-revalidate-batch/<job_id>.json`）+ §2 + §3.3 T6.7；代码：`capability_revalidate_batch.rs:313-323` `get()` → `read_job`（**终态仍可读**，未命中返回 `capability_revalidate_batch_job_not_found`）、`:366` `apply_item` 写锁下重读作业并落盘、`:345-347` 超时后 `mark_unfinished_timeout`；测试 `capability_revalidate_batch_tests.inc.rs:305` `job_state_is_durable_across_reads`、`:267` `unfinished_provider_at_terminal_is_marked_timeout` |
| 9.5 | ✅ 已完成 | 报告：片6 §3.3 测试点↔用例对照表（T6.1~T6.9）；代码：`capability_revalidate_batch_tests.inc.rs:49` `start_returns_job_id_without_waiting_for_probe`、`:95` `four_providers_run_concurrently`、`:135` `single_provider_failure_does_not_block_others`、`:183` `already_confirmed_provider_returns_immediately`、`:216` `missing_cli_is_skipped_with_stable_reason`、`:267`、`:305`、`:348` `batch_is_never_auto_triggered`；`src/web/handlers/logical_codebase_capability_batch.rs:227` `post_returns_job_id_immediately`、`:276` `get_returns_per_provider_results`、`:311` `unknown_logical_codebase_is_404` |

### §10 票3：一键按钮与逐项进度（前端）（映射 `REQ-PCR-04`）

| # | 结论 | 证据出处 |
|---|---|---|
| 10.1 | ✅ 已完成 | 报告：片6 §1.9 + §3.2 绿；代码：`web/src/components/lifecycle/ProviderCapabilityBatchBar.tsx:24` `const POLL_INTERVAL_MS = 2_500;`（2.5s ∈ 2~3s）、`:218` 按钮「一键核验全部」、终态后不再排下一拍（§3.2 变异测试「终态不停止轮询」已红→还原）；面板接线 `web/src/components/lifecycle/LogicalCodebaseManagementPanel.tsx`（+import +渲染块）+ `LogicalCodebaseManagementPanel.test.tsx` |
| 10.2 | ✅ 已完成 | 报告：片6 §1.9 + §3.2；代码：`ProviderCapabilityBatchBar.tsx:218` `running ? \`核验中（${done}/${BATCH_SIZE}）\` : "一键核验全部"` + `disabled`（按钮禁用）；`:35-39` `STATUS_LABELS` = 核验中 / 已确认 / 核验失败 / 跳过（CLI 不可用）/ 超时；测试 `ProviderCapabilityBatchBar.test.tsx:142`「点击后显示「核验中（N/4）」、按钮禁用，逐项状态随轮询更新」 |
| 10.3 | ◐ **部分完成** | 前两项 ✅：失败行【重试】= `ProviderCapabilityBatchBar.tsx:257-268`（`capability-batch-retry-{provider}`），走**既有单 provider 端点**且不重跑整批 —— 测试 `ProviderCapabilityBatchBar.test.tsx:185` 断言单端点 POST 恰 1 次、批量 POST 恰 0 次；`skipped` 行【安装】= `:266-275` 嵌块2 `ProviderInstallActionButton`（测试 `:230`「跳过行显示【安装】入口，不显示为核验失败」）。**第三项「版本落后项引导【更新】」未做**：批量条**只有** `skipped` 分支挂 `ProviderInstallActionButton`，且恒传 `cliAvailable={false} capabilityConfirmed={false}` ⇒ 按块2 `ProviderInstallActionButton.tsx:165-166` 的规则（`cliAvailable && capabilityConfirmed && versionGap < 0` 才显示【更新】）**永远只出【安装】，永远不出【更新】**；批量作业的逐项状态（`confirmed`/`failed`/`skipped`/`timeout`）也**没有「版本落后」这一态**（版本落后落到 `failed` / `probe_failed`）⇒ spec delta `provider-capability-revalidation/spec.md` 的场景「版本落后引导到更新」在批量条上**无实现** |
| 10.4 | ✅ 已完成 | 报告：片6 §1.9 + §3.3 T6.14；代码：`ProviderCapabilityBatchBar.tsx:225`「逐 LC 核验：会真实跑一次探测，可能几分钟，消耗 provider 额度。」；测试 `ProviderCapabilityBatchBar.test.tsx:306`「按钮区说明核验作用域与成本」 |
| 10.5 | ✅ 已完成 | 报告：片6 §3.3 T6.10~T6.13；代码：一键发起与进度渲染 = `ProviderCapabilityBatchBar.test.tsx:142`；**刷新后按 `job_id` 恢复** = `ProviderCapabilityBatchBar.tsx:52` 读 `localStorage`、`:68` 写、`:97` 挂载时续读 durable 作业（测试 `:187`/`:232`/`:265` 先 `localStorage.setItem(STORAGE_KEY,"job_0001")` 再 `renderBar()` ⇒ 走的是「刷新后按 job_id 恢复」路径）；失败项单独重试不重跑整批 = `:185`；终态停止轮询 = `:263` |

### §11 回归与门禁（映射全部）

| # | 结论 | 证据出处 |
|---|---|---|
| 11.1 | ⏳ 待片1 + 合并后回归 | 报告：无任何切片提供「多仓全流程逐条回归（冷启动、索引、group coding、跨仓验收）」的完整证据；Plan §片1 映射「11.1~11.2 的**后端部分**」归属片1，片1 未落地 ⇒ 整体待片1 落地后重做 |
| 11.2 | ⏳ 待片1 + 编译修复 | 报告：片5 §3.8 明确「`cargo test --locked --lib failure_copy` **无法运行 —— crate 当前不编译（21 errors / 13 warnings）**」；Main 2026-11-03 通报同样指出 `root_recipe_snapshot.inc.rs` / `provider_installation/**` / `automation_enrollment_conflict.rs` 三处待修 ⇒ **单测全绿无法成立**；行数门禁 `large_file_guard` 与隔离/纪律测试同受编译阻塞 |
| 11.3 | ✅ 已完成 | 报告：片3 §0「6 条 Plan 验收命令全部绿：… `pnpm test` 全量 **260 files / 2656 tests**；`pnpm tsc -b` 退出码 0；`pnpm build` 退出码 0」；片5 §3.6 `pnpm tsc -b` **零输出（退出码 0）**、§3.7 `pnpm build` **通过**、§5 C7「同伴 B3S3 回报全仓 `pnpm test` 260 files / 2656 tests 全绿」；代码：改写/删除的单仓相关前端测试均已在库（`AddCodebaseDialog.test.tsx` 4 例、`CreateLifecycleIssueDialog.test.tsx` 10 例、`IssueLifecycleWorkbench.codebases.test.tsx:69-72`、`IssueLifecycleWorkbench.crud.test.tsx:107`、`IssueLifecycleWorkbench.lc-partition.test.tsx:320/349/351`、`web/src/onboarding/steps.test.ts:64-75`），`grep` 确认**无残留**断言单仓 radio / 「继续添加单仓库」/「单仓库代码库」的旧用例（仅保留**存量物理仓徽标**呈现用例，按 Plan §0.3 合法） |
| 11.4 | ⏳ 待合并后 controller 真机验收 | 报告：全部 5 份块3 实施报告中**无任何真机走查记录**（片2 §3 只有 vitest；片5 §3.6/§3.7 只有 tsc/build；片6 §3.2 只有 vitest）；Plan §5.1「真机验收（task 11.4）」明列由 **controller 在合并后**执行 |
| 11.5 | ✅ 已完成 | 本人**亲自执行**：`openspec validate remove-single-repository-path --strict` → 输出 `Change 'remove-single-repository-path' is valid`（退出码 0）；代码：proposal.md、design.md、tasks.md 与 **5 份 spec delta**（`specs/logical-codebase-issue-entry/spec.md`、`specs/logical-codebase-layout-guidance/spec.md`、`specs/logical-codebase-registration/spec.md`、`specs/provider-capability-revalidation/spec.md`、`specs/web-runtime-repository-routing/spec.md`）全部在库 |

---

## 3. 未勾选的 22 条：逐条具体原因

### 3.1 「待切片落地」18 条

| # | 归属 | 具体原因（本人查证结果） |
|---|---|---|
| 1.1 | 块3 片1 | `RepositoryRouting::Legacy { repository_id: String }` 仍在 `src/product/logical_codebase/repository_routing_types.inc.rs:47`；生产代码 32 处匹配臂仍全在。片1 无实施报告落盘。 |
| 1.2 | 块3 片1 | `resolve_issue_logical_codebase_id` 仍返回 `Result<Option<String>, _>`（`repository_routing_types.inc.rs:63-72`），`None` 分支未删。 |
| 1.3 | 块3 片1 | `issue_codebase_stores`（`repository_routing_types.inc.rs:76-97`）仍有 `None =>` 的 project 级 `LogicalCodebaseStore::new` / `IssueCodebaseSelectionStore::new` 回退。 |
| 1.4 | 块3 片1 | `classify` 的 `(None, None)` 仍返回 `Legacy`（`repository_routing_types.inc.rs:100-102`），未落 `FailClosed`。 |
| 1.5 | 块3 片1（须片1 落地后重做） | 红线四个对象**当前**核实零改动（`store.rs:437` `migrate_legacy`、`app_paths.rs:76` `logical_codebase_root`、`dangling_reference/patrol/lc_partition.rs` 与 `repository_routing_resolver.inc.rs` 的 `Legacy` 命中均为 0），但片1 将改写同一批 `Legacy` 语义 ⇒ 「逐处核实留下的 `legacy` 命名代码属布局而非单仓语义」这一步**必须等片1 落地**才能做。 |
| 1.6 | 块3 片1 | 三条用例已写入 `repository_routing_tests.inc.rs:16/63/141`，但仍是 **RED**（`:74` 用 `String` 赋值给 `issue_store.rs:13` 的 `Option<String>` 字段 ⇒ crate 编译不过）；「不再有 Legacy 臂」在生产代码上尚不成立。 |
| 2.1 | 块3 片1 | `src/product/issue_store.rs:13` 与 `:26` 字段仍是 `Option<String>`；`:131/:154` 写盘仍可 `None`；`:214/:285` 夹具仍用 `logical_codebase_id: None`。 |
| 2.2 | 块3 片1 | `issue_store.rs:71-81` 的 `get()` **无**「缺归属 → 明确错误」校验（直接 `read_json`）；只有 `:329-364` 的 RED 测试。 |
| 2.3 | 块3 片1 | `AutomationCarrierResolution::SingleRepository` 变体仍在 `src/web/handlers/support.rs:210`；判定臂仍在 `automation_enrollment.rs:384`、`automation_target.rs:93`、`advance_plan.rs:219`、`coding_start.rs:832`。 |
| 2.4 | 块3 片1（依赖块2 文案单一来源） | `issue_not_bound_to_logical_codebase` 在 `src/web/error.rs` 稳定码→状态表**无登记**，`web/src/` 也**无**该码的中文文案（全仓只命中 `issue_store.rs:297` 注释与 `:329` 测试断言）。 |
| 2.5 | 块3 片1 | 「无归属 issue 投影 / Enable 返回 4xx 且不写 enrollment」「有归属但成员不一致 fail-closed」「老记录报错」三条用例均未落地（依赖 2.1~2.4 先落地）。 |
| 3.1 | 块3 片1 | **题面重点项**。本人实跑题面清单命令：**生产口径仍 32 处 / 19 个文件**（数字与题面一致 ⇒ 清单有效），枚举定义仍在 `repository_routing_types.inc.rs:47`；两处 `#[cfg(test)]` 命中（`group_validation.rs:695` 构造点、`coding_cleanup.inc.rs:171` 匹配臂）**原位未处置**。 |
| 3.2 | 块3 片1 | 题面点名的 8 个入口全部仍命中 `RepositoryRouting::Legacy`（逐条位置见 §2 表 3.2 行）。 |
| 3.3 | 块3 片1 | `src/web/handlers/product_resources.rs:158-216` 的 `create_product_issue` 仍有 `None` 单仓分支（`find_repository` 物理存在性校验 + `logical_codebase_id: None` 落库），且 `:177-179` 仍强制 `repository_required`。 |
| 3.5 | 块3 片1 | 「各入口对同一状态结果一致 / 无归属一律 fail-closed 且不返回 200 / 多仓既有行为逐条回归」三条用例属片1 用例组；3.1 的 32 处未改 ⇒ 入口行为仍分叉。 |
| 11.1 | 块3 片1（后端部分）+ 合并后回归 | 无任何切片提供「多仓全流程逐条回归（冷启动、索引、group coding、跨仓验收）」的完整证据；Plan §片1 把 11.1~11.2 的后端部分映射给片1，片1 未落地。 |
| 11.2 | 块3 片1 + 编译修复 | crate **当前编译不过**（片5 §3.8：21 errors / 13 warnings；Main 通报：`root_recipe_snapshot.inc.rs`、`provider_installation/**`、`automation_enrollment_conflict.rs` 三处待修）⇒ 「单测全绿 + `large_file_guard` 绿 + 隔离与纪律测试绿」**客观不成立**。 |
| 11.4 | 合并后 controller（Plan §5.1） | 5 份块3 实施报告中**无真机走查记录**；真机验收六项（新建项目直接多仓 / 1 成员全链 / 3 成员仓序 / 六条布局规则 / 一键核验 4 家并发 / 核验后重试聚合初始化）**一项未做**。 |

### 3.2 「部分完成」4 条

| # | 已完成什么 | 还差什么 |
|---|---|---|
| 3.4 | **发起侧已收窄**：`web/src/api/types/lifecycle.ts:350` 新增 `EnrollmentCarrierTarget`（仅 LC 两变体），`:432`/`:453` 的 Enable / Rebind 命令 target 改用它；`web/src/components/lifecycle/automation-reenable.ts:63` `carrierTargetFromEnrollmentTarget`（`single_repository` → `null`）+ `:88` `LEGACY_CARRIER_BLOCKED_MESSAGE`；`WorkItemPlanOptionsDialog.tsx:118-143` 守卫 + `:420-430` 提示行；`useIssueLifecycleGeneration.ts` 两处命令构造点守卫。 | **「删除 plan / cockpit 侧 `single_repository` 类型变体及其消费分支」这一半未做**：变体与消费分支**原样保留**在 `api/types/lifecycle.ts:371`、`api/types/work-item-plan.ts:679`、`WorkItemPlanOptionsDialog.tsx:589`、`state/workspace-cockpit-inbox-items.ts:56`（按「方案 A：只收窄发起侧 + legacy 投影只读」）。注：块3 Plan §6 **R19 已预登记**该冲突 ——「tasks.md 3.4 的措辞未区分投影侧/发起侧 ⇒ 需同步修正 tasks.md 3.4 的措辞（属 change 文件，块3 不改 ⇒ 报 controller/owner）」。 |
| 4.4 | ① 对话框无单仓选项 ✅（`AddCodebaseDialog.test.tsx:13`、`CreateLifecycleIssueDialog.test.tsx:68`）；② 无 `logical_codebase_id: null` 的提交路径 ✅（`CreateLifecycleIssueDialog.tsx:15` 类型必填 `string`、测试 `:119` 断言带 `logical_codebase_id` + primary）。 | ③ 「缺 LC 提交被**稳定错误码**拒绝」❌：前端只做**本地文案**拒绝（`CreateLifecycleIssueDialog.tsx:121`「请选择逻辑代码库」，不落库），**稳定错误码 `issue_not_bound_to_logical_codebase` 全仓不存在**（`src/web/error.rs` 码表无、`web/src/` 无中文文案）⇒ 归属片1 后端（= 2.2/2.4）。 |
| 5.2 | 「改造后的既有『添加代码库』对话框即『建逻辑代码库』入口」+「**不新造页面**」✅：改造后的 `AddCodebaseDialog.tsx`（138 行，恒渲染多仓表单）即入口，无新页面；`AddCodebaseDialog.test.tsx:49`「创建成功后同弹窗进入登记，复用聚合根自动发现（建 LC 入口不新造页面）」。 | 「**真机走一遍**」❌：片2 报告 §3/§4 只有 vitest 命令输出，全仓库无真机走查记录；真机口径归属 task 11.4（Plan §5.1，合并后 controller 执行）。 |
| 10.3 | ① 失败项每行保留【重试】✅（`ProviderCapabilityBatchBar.tsx:257-268`，测试 `:185` 断言单 provider 端点恰 1 次、批量端点 0 次）；② `skipped` 项引导【安装】✅（`:266-275` 嵌块2 `ProviderInstallActionButton`，测试 `:230` 断言不显示为「核验失败」）。 | ③ 「**版本落后项引导【更新】**」❌：批量条只有 `skipped` 一个分支挂 `ProviderInstallActionButton`，且恒传 `cliAvailable={false} capabilityConfirmed={false}` ⇒ 按块2 `ProviderInstallActionButton.tsx:165-166`（`cliAvailable && capabilityConfirmed && versionGap < 0` 才出【更新】）**永远只出【安装】、永不出【更新】**；批量逐项状态（`confirmed`/`failed`/`skipped`/`timeout`）也没有「版本落后」这一态 ⇒ spec delta 场景「版本落后引导到更新」在批量条上无实现。 |

### 3.3 「未证实」0 条

**本轮无「报告说做了但代码里找不到」的条目。** 说明：

- 5 份块3 实施报告（片2/片3/片4/片5/片6）声称交付的产物，**本人逐个在代码里找到了对应文件/函数/文案/测试**（见 §2 各行的代码列）。
- 唯一「报告口径与任务文字不一致」的是 **3.4**（片3 按 controller 二次更正走「方案 A」，明确保留投影侧变体）—— 由于片3 报告**自己写明了**保留与否、且 Plan §6 R19 预登记了该冲突，故归入「部分完成」而非「未证实」。
- 片1（§1 全组、§2 全组、3.1/3.2/3.3/3.5、11.1/11.2）**没有实施报告落盘** ⇒ 属「待切片落地」，不属「未证实」。

---

## 4. 本人亲自查证过的代码位置清单

### 4.1 后端（`src/`）

| 位置 | 查证内容 |
|---|---|
| `src/product/logical_codebase/repository_routing_types.inc.rs:35-118` | 枚举三态仍全（`Legacy` 定义 `:47`；`classify` 的 `(None,None)=>Legacy` `:100`）；`resolve_issue_logical_codebase_id` 仍返回 `Option<String>` `:63`；`issue_codebase_stores` 仍有 `None` 回退 `:76-97` |
| `src/product/logical_codebase/repository_routing_tests.inc.rs:16/31/38/50/63/141` | 片1 已写入的 RED 用例（`classify_none_none_fails_closed_without_legacy_arm` / `resolve_issue_logical_codebase_id_returns_string_for_bound_issue` / `issue_codebase_stores_scopes_to_lc_subtree`），`:74` 的 `String` 赋值证明其对 `Option<String>` 的 RED |
| `src/product/issue_store.rs:1-98` | `IssueRecord.logical_codebase_id: Option<String>` `:13`；`CreateProductIssueInput` `:26`；`get()` `:71-81` 无「缺归属报错」校验；`create` `:131`/`:154` 仍可写 `None` |
| `src/product/issue_store.rs:315-373` | `old_issue_record_without_lc_id_reports_explicit_error` RED 测试 `:329-364`（断言 `InvalidRecord{kind:"issue_not_bound_to_logical_codebase"}`） |
| `src/web/error.rs:51-320` | 稳定码→状态表；**无** `issue_not_bound_to_logical_codebase` 条目 |
| `src/web/handlers/support.rs:210` | `enum AutomationCarrierResolution` 仍含 `SingleRepository` |
| `src/web/handlers/automation_enrollment.rs:384/398` | 单仓载体判定臂仍在 |
| `src/web/handlers/automation_target.rs:93/101` | 投影侧单仓载体分支仍在 |
| `src/web/advance_plan.rs:219/239/253` | Enrolled advance 的单仓载体臂仍在 |
| `src/web/coding_start.rs:754/778/832/900` | 自动 StartCoding 的单仓载体臂仍在 |
| `src/web/handlers/product_resources.rs:155-233` | `create_product_issue` 的 `None` 单仓分支 + `repository_required` 前置门（3.3 未做） |
| `src/web/handlers/product_resources.rs:455-475` | 另一处 `create_issue`（registry 版，与 3.3 无关） |
| `src/web/handlers/support_parts/coding_cleanup.inc.rs:171/222` | `#[cfg(test)]` 内的 Legacy 匹配臂仍在（3.1 重点项之一） |
| `src/product/coding_attempt_store/group_validation.rs:265/578/695` | Legacy 构造点 `:265`、匹配臂 `:578`、`#[cfg(test)]` 构造点 `:695`（3.1 重点项之一） |
| `src/product/logical_codebase/failure_copy.rs:323-329/464-470/584-607` | 块2 已登记 7 个对外稳定码（`:464-470`）+ 码表分支 + 单测 ⇒ 7.1/7.4 的单一来源已就位 |
| `src/product/logical_codebase/capability_revalidate_batch.rs:25/55-67/72-105/172-190/294/313-323/339-347/366` | 票3 作业编排层：原因码常量、状态枚举、durable 作业结构、job_id 生成、按 job_id 读取、join_all 并发 + 超时、逐项落盘 |
| `src/product/logical_codebase/capability_revalidate_batch_tests.inc.rs:49-348` | T6.1~T6.8 八条用例全在 |
| `src/product/logical_codebase/mod.rs:8` | `pub mod capability_revalidate_batch;` 模块声明 |
| `src/web/handlers/logical_codebase_capability_batch.rs:227/276/311` | T6.9 三条内联用例 |
| `src/web/handlers/mod.rs:99/164-165` | handler 模块声明与两个导出 |
| `src/web/app.rs:193-198` | 两条 `revalidate-all` 路由挂载 |
| `src/product/dangling_reference/patrol/lc_partition.rs` | 红线对象，`Legacy` 命中 0 |
| `src/product/logical_codebase/repository_routing_resolver.inc.rs` | 红线对象，`Legacy` 命中 0 |
| `src/product/logical_codebase/store.rs:437` | `migrate_legacy` 在库（红线） |
| `src/product/app_paths.rs:76` | `logical_codebase_root` 在库（红线） |

### 4.2 前端（`web/src/`）

| 位置 | 查证内容 |
|---|---|
| `web/src/components/lifecycle/AddCodebaseDialog.tsx`（138 行） | 单仓 radio / 「继续添加单仓库」/ mode 状态机全部不存在（grep 命中 0） |
| `web/src/components/lifecycle/AddCodebaseDialog.test.tsx:13/33/49/142` | 4 条用例（无单仓入口 / 默认多仓 / 建 LC 入口不新造页面 / 手工输入不被覆盖） |
| `web/src/components/lifecycle/CreateLifecycleIssueDialog.tsx:14-18/121/146-148/203` | `logical_codebase_id: string` 必填；缺 LC 本地拒绝；`base_branch: null`；下拉只列 `logical` |
| `web/src/components/lifecycle/CreateLifecycleIssueDialog.test.tsx:68/99/119/212/242/284/318` | 10 条用例（含「无「单仓库代码库」选项」「未选逻辑代码库提交被拒」「base_branch 恒 null」「单成员 ≡ 多选同一路径」） |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.test-utils.ts:787-792` | 存量物理仓 `kind: "single_repo"` 徽标夹具**保留**（Plan §0.3 合法） |
| `web/src/components/lifecycle/IssueLifecycleWorkbench.codebases.test.tsx:69-72` / `.crud.test.tsx:107` / `.lc-partition.test.tsx:320/349/351` | 单仓入口消失的断言 + 存量单仓呈现钉 |
| `web/src/onboarding/steps.ts:74/76/78/85` | 引导步骤 id / 标题 / 多仓流程文案 / Issue 绑定 LC 文案 |
| `web/src/onboarding/steps.test.ts:46-75` | 13 步断言 + 「不再出现「单仓」路径」 |
| `web/src/components/lifecycle/CreateProjectDialog.tsx` | `logical`/`聚合根`/`member` grep 命中 0 ⇒ 创建项目表单内无 LC 字段 |
| `web/src/components/lifecycle/IssueLifecycleWorkbenchView.tsx:75-91/121-127` | `createIssueGate`（无 `member_count>0` 的 LC ⇒ 禁用 + 原因文案）+ 渲染 |
| `web/src/api/types/lifecycle.ts:344-372/429-432/452-453` | `EnrollmentCarrierTarget`（两变体）/ `EnrollmentTarget`（三变体保留）/ 命令侧已改用 carrier 型 |
| `web/src/api/types/work-item-plan.ts:679` | `single_repository` 变体**仍在**（3.4 未做的一半） |
| `web/src/components/lifecycle/automation-reenable.ts:53-89` | `carrierTargetFromEnrollmentTarget` + `LEGACY_CARRIER_BLOCKED_MESSAGE` |
| `web/src/components/lifecycle/WorkItemPlanOptionsDialog.tsx:118-143/420-430/583-590` | 提交守卫 + legacy 提示行 + `case "single_repository"` 分支**保留**（3.4 未做的一半） |
| `web/src/components/lifecycle/WorkItemPlanOptionsDialog.target.test.tsx:152/331/353/397-437` | `legacy_single_repository_projection_renders_readonly` / `target_label_renders_logical_codebase_members` / `no_single_repository_in_carrier_commands` |
| `web/src/components/lifecycle/WorkItemPlanOptionsDialog.reenable.test.tsx:317-380` | `reenable_disabled_for_legacy_single_repository_binding` / `carrier_target_from_legacy_returns_null` |
| `web/src/state/workspace-cockpit-inbox-items.ts:53-57` | `c1TargetLabel` 的 `single_repository` 分支**保留**（3.4 未做的一半） |
| `web/src/state/enrollment-target-labels.ts:1-3` | 三变体覆盖说明（legacy 只读） |
| `web/src/components/lifecycle/LogicalCodebaseLayoutTree.tsx`（43 行） | 目录树三行 + 「不是 git 仓」+ 「直接子目录」+ `.worktrees/` 说明；无任何文案表 |
| `web/src/components/lifecycle/LogicalCodebaseLayoutTree.test.tsx` | 3 条（T5.1/T5.2/T5.3 源码级锁） |
| `web/src/components/lifecycle/LogicalCodebaseBootstrapCard.tsx:25/270` | 卡片挂载目录树（只加 2 行） |
| `web/src/components/lifecycle/LogicalCodebaseRegistrationWizard.tsx:25/26/69-107/306/500-527/538` | 树挂载 / 块2 安装按钮复用 / `readFailureCopy` 五字段校验 / 三段式四元素渲染 |
| `web/src/components/lifecycle/LogicalCodebaseRegistrationWizard.test.tsx:8-14/334/350/405/421/442/449/474/481` | 七码常量 / T5.4 / T5.5 / T5.7 逐条 / T5.8 两例（安装 + 更新 + 无 verify） |
| `web/src/components/lifecycle/ProviderInstallActionButton.tsx:4-11/165-166/192/219/224` | 块2 组件的 安装/核验/更新 判定规则（版本落后 ⇒ 更新） |
| `web/src/components/lifecycle/ProviderCapabilityBatchBar.tsx`（284 行） | `:24` 轮询间隔 / `:52`/`:68` localStorage 续读 / `:142` 延迟 / `:170` 单项重试 / `:218` 「核验中（N/4）」+ 禁用 / `:225` 作用域说明 / `:257-268` 重试按钮 / `:266-275` skipped ⇒ 安装（**无更新分支**） |
| `web/src/components/lifecycle/ProviderCapabilityBatchBar.test.tsx:142/185/230/263/306` | T6.10~T6.14 五条用例 |
| `web/src/components/lifecycle/LogicalCodebaseManagementPanel.tsx` | 批量条接线（import + 渲染块） |
| `web/src/components/lifecycle/LogicalCodebaseManagementPanel.test.tsx` | 面板渲染批量条断言 |
| `web/src/api/capability-revalidate-batch.ts` / `web/src/api/types/capability-revalidate-batch.ts` | 批量作业前端 API + DTO |
| `web/src/components/lifecycle/slice5-raw-imports.d.ts` | 片5 新建的 `*?raw` 声明（`?raw` 源码扫描用） |
| 全 `web/src`（`grep -rn "single_repository"` / `"继续添加单仓库"` / `"单仓库代码库"`） | 单仓**创建**入口全清；`single_repo` 徽标与 legacy 只读投影**按设计保留** |

### 4.3 集成测试（`tests/`）

| 位置 | 查证内容 |
|---|---|
| `tests/it_core.rs:44-47` | `mod single_member_lc_lifecycle` + `mod single_member_path_guard` 已注册（防「0 匹配假绿」） |
| `tests/it_core/single_member_path_guard.rs:146/154/208/353/377-493/544/552/560` | 路径剥离器 / 内联测试剥离 / 阴性对照 / 模式表（≥2 成员与 ==1 成员两类）/ 三条源码级断言 |
| `tests/it_core/single_member_lc_lifecycle.rs:477/503/528/618/662` | T4.4~T4.7 行为级用例 + 仓序规则源结构锁 |

### 4.4 OpenSpec 产物（本人亲自执行）

```text
$ openspec validate remove-single-repository-path --strict
Change 'remove-single-repository-path' is valid
```

---

## 5. 边界自证

| 约束 | 结论 |
|---|---|
| 只改 `tasks.md` 的勾选框 | ✅ 全部 30 次编辑只把行首 `- [ ] ` 替换为 `- [x] `；任务文字、编号、顺序、章节、附注段落**一字未动**；文件行数仍为 135 行 |
| 不改源码 | ✅ 未触碰 `src/`、`web/src/`、`tests/` 任何文件 |
| 不改 spec / design / proposal / Plan | ✅ 只读 |
| 不改其它 change 的 tasks.md | ✅ 只改 `openspec/changes/remove-single-repository-path/tasks.md` 一个文件 |
| 不 `git add` / `git commit` | ✅ 未执行 |
| 未新建除清单外的文件 | ✅ 只新建本清单一个文件 |
| 逐条走过（非抽查） | ✅ 52 条逐条判定，见 §2 完整对照表 |

### 收尾自证（真实输出）

```text
$ cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02
$ ls -la openspec/changes/remove-single-repository-path/tasks.md
-rw-r--r-- 1 michaelche michaelche 16720 Nov  3 09:35 openspec/changes/remove-single-repository-path/tasks.md
$ ls -la cadence/notes/2026-11-03_task对证_remove-single-repository-path_v1.0.md
-rw-r--r-- 1 michaelche michaelche 48817 Nov  3 09:37 cadence/notes/2026-11-03_task对证_remove-single-repository-path_v1.0.md
$ grep -c '^- \[x\]' openspec/changes/remove-single-repository-path/tasks.md
30
$ grep -c '^- \[ \]' openspec/changes/remove-single-repository-path/tasks.md
22
```

> 30 + 22 = 52 = task 总数 ✅

---

## 追加：tasks.md 3.4 措辞修正（块3 Plan §6 R19 落地）

> 追加时间：2026-11-03；执行角色：design（Task34WordingFixer），**只改文字、不动勾选框**。
> 范围：**只改** `openspec/changes/remove-single-repository-path/tasks.md:52`（3.4 一条），其余任务文字、编号、顺序与全部勾选框零改动。

### 1. 3.4 改前 → 改后（逐字）

**改前（原文，行 52）**：

```text
- [ ] 3.4 删除 plan / cockpit 侧 `single_repository` 类型变体及其消费分支（`web/src/api/types/lifecycle.ts`、`work-item-plan.ts`、`WorkItemPlanOptionsDialog.tsx`、`state/workspace-cockpit-inbox-items.ts`）
```

**改后（行 52）**：

```text
- [ ] 3.4 **前端 enrollment 载体收窄（方案 A：只收窄发起侧 + legacy 投影保留；原措辞「删除 `single_repository` 类型变体及其消费分支」已按块3 Plan §6 R19 更正，不得再按原措辞执行）**：**保留** plan / cockpit 侧 `single_repository` 的**类型变体**与**只读渲染** —— `EnrollmentTarget`（`web/src/api/types/lifecycle.ts`）三变体不删；`WorkItemPlanOptionsDialog.tsx` 的 `enrollmentTargetLabel` 与 `web/src/state/workspace-cockpit-inbox-items.ts` 的 `c1TargetLabel` 的 `case "single_repository"` 渲染分支照旧输出「单仓 repo_x」，**MUST NOT** 退化为「target 未知」；对应 legacy 展示用例（`WorkItemPlanOptionsDialog.target.test.tsx`、`workspace-cockpit-projection.inbox.test.ts`）一并保留；**只在发起侧收窄** —— `web/src/api/types/lifecycle.ts` 新增 `EnrollmentCarrierTarget`（`logical_codebase` / `logical_codebase_members` 两变体）供 Enable / Rebind 命令使用，`web/src/components/lifecycle/automation-reenable.ts` 新增守卫 `carrierTargetFromEnrollmentTarget`（`single_repository` → `null`），`WorkItemPlanOptionsDialog.tsx` 的 Enable / Rebind 提交路径按该守卫判定：为 `null` ⇒ **禁用提交 + 提示「需重建为逻辑代码库授权」**，**不得**静默降级、**不得**猜 LC；**照删**的是 Rust 侧载体判定 `AutomationCarrierResolution::SingleRepository`（见 2.3 与上文跨块串行登记 #4，本任务不重复实施）；**禁删** `EnrollmentTargetDto.single_repository`（`web/src/api/types/work-item-plan.ts:678-689`）、`EnrollmentTarget::SingleRepository`（`src/product/logical_codebase/types.rs:31`）、`RepositoryTargetKind::SingleRepo`（`src/product/logical_codebase/repository_routing_types.inc.rs:151-152`）—— 依据 `openspec/specs/work-item-group-structure/spec.md:14` 的多仓候选声明合同（`intent_target_kind: single_repository`）。依据：块3 Plan §2.3.1 第 3 条 / §2.3.2 / §2.3.3 / §6 R19
```

### 2. 矛盾定位与依据链

原措辞要求「删除类型变体」，与已批准设计「方案 A：只收窄发起侧 + legacy 投影保留」及多仓 plan 合同直接冲突（逐条已核原文）：

- `cadence/plans/2026-11-03_计划文档_实施_块3_去单仓入口与统一多仓流程_v1.0.md:534` —— §2.3.1 标题即「收敛为方案 A：只收窄『发起侧』，投影侧保留 legacy 只读」。
- 同文件 `:541` —— 「直接删变体 ⇒ 老单仓 enrollment 从『单仓 repo_x』退化为『target 未知』…… 这是**行为回归**」。
- 同文件 §2.3.1 第 3 条（`:547` / `:549`）—— 保留投影侧三变体与全部渲染分支（`c1TargetLabel`、`enrollmentTargetLabel`）；新增 `carrierTargetFromEnrollmentTarget`，`single_repository` → `null` ⇒ 禁用提交 + 提示，不得静默降级、不得猜 LC。
- 同文件 §2.3.2（`:550-563`）—— `lifecycle.ts` 三变体保留 + 新增 `EnrollmentCarrierTarget`；`automation-reenable.ts` 新增守卫；`WorkItemPlanOptionsDialog.tsx` 保留 `case` 渲染、只改提交路径；`workspace-cockpit-inbox-items.ts` 的 `c1TargetLabel` 保留。
- 同文件 §2.3.3 禁碰清单（`:569-577`）—— `web/src/api/types/work-item-plan.ts:678-689` 的 `EnrollmentTargetDto.single_repository` **保留**（plan-intent 合同）。
- 同文件 §6 **R19**（`:1138`）—— 「tasks.md:44（3.4）要求删 `work-item-plan.ts` 的 `single_repository` 变体，与多仓 plan 合同冲突 ⇒ **保留** `EnrollmentTargetDto`；**需同步修正 tasks.md 3.4 的措辞**」。
- 主 specs `openspec/specs/work-item-group-structure/spec.md:14` —— 「一个多仓候选声明 `intent_target_kind: single_repository`」⇒ 删前端变体会打断该合同（spec 只读，本次未改）。
- `cadence/notes/2026-11-03_波B交接点_v1.0.md` §⑧ —— `single_repository` 前端渲染**保留只读，不删**；**照删** `AutomationCarrierResolution::SingleRepository`；**禁删** `EnrollmentTarget::SingleRepository`（`src/product/logical_codebase/types.rs:31-33`）与 `RepositoryTargetKind::SingleRepo`（`repository_routing_types.inc.rs:151-152`）。

禁删三处已逐处实读确认存在：`work-item-plan.ts:678-679`（`EnrollmentTargetDto` 含 `{ kind: 'single_repository'; repository_id: string }`）、`types.rs:31-33`（`EnrollmentTarget::SingleRepository`）、`repository_routing_types.inc.rs:151-152`（`RepositoryTargetKind::SingleRepo`）。

### 3. 改写覆盖点自查（对照派工单要求）

| 要求点 | 改后 3.4 |
|---|---|
| 保留类型变体 + 只读渲染，仍输出「单仓 repo_x」，MUST NOT 退化「target 未知」 | ✅ 明确写出，并点名 `enrollmentTargetLabel` / `c1TargetLabel` |
| 只在发起侧收窄：`carrierTargetFromEnrollmentTarget` 守卫（`single_repository` → `null`）⇒ 禁用提交 + 提示「需重建为逻辑代码库授权」 | ✅ 明确写出，含「不得静默降级、不得猜 LC」 |
| 照删 Rust 侧 `AutomationCarrierResolution::SingleRepository` | ✅ 写出，且标注归 2.3 / 跨块登记 #4，本任务不重复实施 |
| 禁删三处（`work-item-plan.ts:678-689`、`types.rs:31`、`repository_routing_types.inc.rs:151-152`） | ✅ 逐条点名 + 理由（spec.md:14 多仓 plan 合同） |
| 依据标注：块3 Plan §2.3.1 第 3 条 / §2.3.2 / §2.3.3 / §6 R19 | ✅ 行尾标注 |

### 4. 同类矛盾通读结论

**已通读全部任务 1.1 ~ 11.5（共 52 条，含 3.4）**，按同一判据（「凡要求删除 `single_repository` 类型变体 / 只读渲染」）逐条比对：

- **除 3.4 外，无同类矛盾。**
- 提到单仓但**不属同类**、按纪律一字未动的任务及理由：
  - `2.3`：删的是 **Rust 侧载体判定**（`AutomationCarrierResolution::SingleRepository` 与「无 LC 归属→单仓」判定），正是 R19 口径里「照删」的部分，非前端类型变体；
  - `3.3`：删的是「issue 创建时校验单仓 `repo_id` 物理存在性」这一前置门，不涉类型/渲染；
  - `4.1` / `4.2` / `4.3`：前端**入口**（`AddCodebaseDialog`、`CreateLifecycleIssueDialog`）下线及对应对话框测试改写，属已批准范围，不涉 `single_repository` 类型变体与只读渲染；
  - `5.1`：新建流程界面不得出现单仓选项，同上；
  - `11.3`「含删除的单仓相关测试」：指 4.3 已落地的对话框测试删除，属状态描述，不是「要求删除 `single_repository` 类型/渲染」⇒ 未动（若需精确化请另开票）。
- `tasks.md` 中另外 2 处出现 `single_repository` 字面量的位置（`:14`、`:15` 跨块串行登记表）本就写「渲染保留 / 只在提交路径禁用」，与改后 3.4 一致，未改动。

### 5. 勾选框计数自证（改前 = 改后）

```text
$ cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02
$ grep -n '^- \[.\] 3\.4' openspec/changes/remove-single-repository-path/tasks.md
52:- [ ] 3.4 **前端 enrollment 载体收窄（方案 A：只收窄发起侧 + legacy 投影保留；...
$ grep -c '^- \[x\]' openspec/changes/remove-single-repository-path/tasks.md
30
$ grep -c '^- \[ \]' openspec/changes/remove-single-repository-path/tasks.md
22
```

> 改前 `30 / 22`，改后仍 `30 / 22`（30 + 22 = 52 = task 总数）；3.4 保持 `- [ ]` 未勾；TaskVerifyBlock3 已落地的 30 处 `- [x]` 原样保留。

### 6. `openspec validate --strict`（真实输出）

```text
$ npx --no-install openspec validate remove-single-repository-path --strict
Change 'remove-single-repository-path' is valid
$ echo $?
0
```

### 7. 未执行事项（纪律）

- 未 `git add` / `git commit`；未改 spec.md / design.md / proposal.md / Plan / 技术方案 / 源码 / `openspec/specs/` / 其它 change。

### 8. 文件自证（追加本报告后的真实输出）

```text
$ cd /home/michaelche/workspace/github/cadence-aria/.worktrees/bugfix-b-11-02
$ ls -l openspec/changes/remove-single-repository-path/tasks.md cadence/notes/2026-11-03_task对证_remove-single-repository-path_v1.0.md
-rw-r--r-- 1 michaelche michaelche 57447 Nov  3 09:40 cadence/notes/2026-11-03_task对证_remove-single-repository-path_v1.0.md
-rw-r--r-- 1 michaelche michaelche 18514 Nov  3 09:40 openspec/changes/remove-single-repository-path/tasks.md
$ wc -l openspec/changes/remove-single-repository-path/tasks.md cadence/notes/2026-11-03_task对证_remove-single-repository-path_v1.0.md
  135 openspec/changes/remove-single-repository-path/tasks.md
  404 cadence/notes/2026-11-03_task对证_remove-single-repository-path_v1.0.md
  539 total
```

> 本报告为**追加**（原 318 行内容逐字保留，追加后 404 行；`startswith(原内容)` 校验为 True）。
