# 技术方案：story / design / work item 三环节评审修正（v1.1，逻辑代码库限定）

> 性质：方案。**不含实现代码**。v1.0 保留（`…评审修正_v1.0.md`），本文为 owner 2026-10-23 新增硬约束后的修订版。
> 标准依据：`cadence/designs/2026-10-22_技术方案_系统环节与角色标准_v1.0.md`。
> **新增硬约束（owner 原话）**："这两个方案都不能影响单仓的使用哈，都是在多仓上做改造。"
> → **单仓库代码库路径行为必须与今天完全一致**；所有改动只作用于**多仓库逻辑代码库**。
> 理念：先删后加、先通后优；每阶段结束必须能走完 story→design→work item。
> 工作目录校验（2026-10-23 实测）：`pwd` = `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`；`git branch --show-current` = `feat-b-0808-add-monorepo`。✅

## 一、形态门禁（本版新增，全文唯一判定键）

**判定键（owner 已核准）**：`IssueRecord.logical_codebase_id`（`models/project.rs:141-143`，注释"Some 表示逻辑代码库 lc_id；None 表示单仓代码库"）。

| 形态 | 判定 | 本批处理 |
|---|---|---|
| 单仓库代码库 | `None` | **完全现状**：保留评审窗口、`begin_plan_review_window`、基线/对账/恢复、`plan_review_window_worktree` 早返回、`assert_plan_review_window`、`reviewer_verify_policy` 窗口 id、事件与文案。**零改动** |
| 多仓库逻辑代码库 | `Some(_)` | 走本文 16 条 |

新增唯一门禁函数（`prompts.rs`，紧邻 `is_aggregate_planning_session`；后者是"聚合视野"判据，**不是**形态判据，本批不拿它代替）：

```rust
fn is_logical_codebase_issue(&self) -> bool   // true = 多仓(lc)；false = 单仓(现状)
// store 可用 → IssueStore::get(project, issue).logical_codebase_id.is_some()
// store 缺失(内存态引擎) → self.logical_provider_gateway.is_some()
//     （lifecycle.rs:662 既有口径："非空即逻辑会话"）
// 读取失败 → false（fail-safe 回单仓现状路径，绝不让单仓落到新逻辑）
```

**总原则**：所有删除项一律"条件化"——不物理删除单仓仍在用的共享路径；新逻辑只挂在 `is_logical_codebase_issue() == true` 分支。

## 二、阶段划分（3 阶段，每阶段结束可跑通）

| 阶段 | 条目 | 阶段结束判定 |
|---|---|---|
| 一｜lc 删窗·集合权威·材料稳定注入 | 1,2,3,4,6,15 | 单仓 story/design/work item 全链路与今天逐项一致（窗口/事件/基线/门控）；lc 选 2 仓跑通 author→reviewer→verdict，reviewer prompt 含 2 仓完整清单且无窗口记录 |
| 二｜lc 逐仓实况与判定 | 5,16,8,9,10 | 材料含每仓实况与基线行；漏项/选错仓/验收不可判定夹具必出 `must_fix`；单仓 reviewer 规则文案逐字不变（条目 8） |
| 三｜容错与解析 | 11,12,13,14 | 各负例夹具通过，两者仍可走通全流程 |

## 三、阶段一

### 1｜删除「主仓 / 主成员」概念（仅 lc 分支）
- 落点：`drive.rs:1690-1746`（`plan_review_window_worktree` 的 lc 分支 `:1695-1746`）、`:1998-2020`（`emit_primary_member_anchor_note`）、`:44-60`/`:386-400`（调用点）、`workspace_repository.rs:285-318`、`prompts.rs:759/812/866` 文案
- 改法：**保留** `plan_review_window_worktree`（单仓仍用），只删其 lc 分支（`:1695` 之后到 `:1746` 的 business/primary 解析）——因为 lc 会话不再开窗，该分支无消费者；`resolve_primary_member_repository` 与 `logical_repository_primary_member` kind、`emit_primary_member_anchor_note`、`primary_anchored` 变量删除；prompt 文案"任意单一 primary 仓库"→"任意单一仓库"。
- **单仓证明**：① `:1695-1697` `if target.join(".git").exists() { return Ok((target, false)); }` 是单仓唯一出口，`primary` 分支在其后、不可达；② `resolve_primary_member_repository` 全树唯一调用点 `drive.rs:1727`，位于该早返回之后；③ 该函数对 `RepositoryRouting::Legacy` 直接 `Err`（`workspace_repository.rs:307-312`），单仓路由永不产出 primary；④ primary 相关用例全部为 `logical_fixture`（`drive_tests.rs:557,562` `logical_codebase_id: Some(legacy_lc)`）——反证：窗口内联测试才是 `None`（`drive.rs:2063,2174,2373`）。
- 验收：单仓 issue 跑评审，`plan_review_window_worktree` 仍在 `:1695` 早返回（加一条 debug 断言或既有测试即证）；lc 侧 `grep primary` 零命中。
- 风险：低。删的是单仓不可达分支。

### 2｜单值字段降级为兼容（仅 lc 分支）
- 落点：`prompts/review_context.rs:618-634`、`models/project.rs:138`、`types.rs:119`
- 改法：`review_launch_target_and_cwd` 的 gateway 分支（`:621-627`）对 lc 会话返回 `(authority_root, Some(authority_root))`，不再要求 `repository_path` 是成员 worktree。**`:628-633` 的 else 分支（单仓）逐字不动**。字段本身保留（存量反序列化），加注释"兼容字段，非 lc 范围权威"。
- **单仓证明**：gateway 分支的条件是 `self.logical_provider_gateway()` 非空，即 lc 会话（`lifecycle.rs:662`）；单仓 `None` → 直接落到 `:628-633`，其中 `:629` 继续使用 `session.repository_path`（单仓仍然是唯一仓路径，字段不能删）。
- 验收：单仓 reviewer 的 `working_dir`/`working_directory` 与今天一致（`working_directory=None` 透传）；lc reviewer 两者均为 LC 根。
- 风险：低-中。lc 侧失去"单成员 target"视野，但材料已改为集合全量注入。

### 3｜lc 会话不再开窗（单仓保留窗口 + 零锁恢复）
- 落点：`drive.rs:24-77`（`drive_review_session`）、`:363-405`（`drive_review_session_via_gateway`）；定义 `:1441-1450`、`:1582-1584`、`:1600-1685`、`:1752-1814`、`:1816-1840`、`:1842-1856`、`:1857-1955`、`:1958-1997`；`types.rs:922-924`；`review.rs:10`
- 改法：两个入口函数体内加门——`if self.is_logical_codebase_issue() { 直接调内层（内层改名去掉 `_with_window` 后缀） } else { 现状代码 :45-72 / :378-403 原样保留 }`。lc 内层不调 `assert_plan_review_window`（`:128,196,326,466,539,670` 的 6 处**全部保留**，仅单仓可达）。`begin_plan_review_window`/`close_plan_review_window`/`plan_review_main_scope`/两个 store 构造/两个常量/`ActivePlanReviewWindow`/`active_review_window` 字段/`emit_plan_review_window_attention` **全部物理保留**（单仓唯一使用者）；`drive_review_session_via_gateway` 的 lc 分支不再调 `begin`(:394)/`close`(:403)，`drive_review_session` 同理(:62,:72)。
- **替代①（材料采集前置校验）**：新增 `assert_plan_review_material()`——lc 会话调 `PlanningContextSetResolver::resolve`（store 读，无索引/无 FS 遍历）+ 材料渲染，集合空或渲染失败 → `plan_review_material_unresolved: <cause>`；在 `build_review_input()` 前与 repair 前各一次。单仓仍走 `assert_plan_review_window`（6 处原样）。
- **替代②（受信归属，两形态共存）**：`reviewer_verify_policy()`（`:1842-1856`）改为 `match self.active_review_window { Some(a) => reviewer_verify(a.handle.window_id) /* 单仓：与今天逐字节一致 */, None => reviewer_verify(self.session.session_id.clone()) /* 仅 lc */ }`。`plan_review_launch_ownership()`（`:1670-1685`）保留（单仓用），lc 分支传 `None`（已核 `drive.rs:1498-1500` 与 Claude `start_owned:859-862` 均容忍）。
- **单仓证明**：单仓仍调 `begin_plan_review_window`，故 `active_review_window` 恒 `Some` → policy 走窗口 id 分支，canonical digest 与今天一致；6 处 `assert_plan_review_window` 与开/收口行未被触碰；窗口内联测试是 `logical_codebase_id: None` 夹具（`drive.rs:2063,2174,2373`），改后这些用例必须继续全绿——即为单仓回归的硬证据。
- 验收：单仓：`reviewer-windows/windows/` 仍有新记录、`plan_review_window_*` 错误码仍可达、产物 digest 不变；lc：无新记录、无窗口错误码、无 primary 事件。
- 风险：中（本批唯一大改）。但条件化后单仓代码路径是"未改动的原函数体"，风险收敛在 lc 分支。

### 4｜仓库清单全量注入，超预算显式阻断（lc 专用）
- 落点：`planning_context_set.rs:160-300`、`logical_codebase/mod.rs:103`
- 改法：删 soft/hard 裁剪与 `omitted_member_count` / `omitted_member_ids` / `truncated`；`render_compact_inventory` 改全量渲染 + 绝对上限 256 KiB，超限 `Err(InvalidRecord{kind:"planning_material_over_budget"})`。
- **单仓证明**：`render_compact_inventory` 唯一生产调用点 `planning_context_resolver.rs:472`，而该 resolver 只在 `RepositoryRouting::Logical` 分支构造（`builder.rs:163-176`）；单仓 `aggregate_planning = None`，永不进入。`InventoryInjection*` 的类型面属 lc 独占，删除对单仓零影响。
- 验收：lc N=3 全量命中；超 256 KiB → 明确错误码且评审不启动；单仓 prompt 字节不变。
- 风险：低。

### 6｜材料改稳定注入（lc 专用；单仓**不注入**成员清单）
- 落点：`web/workspace_context/builder.rs:236-281`、`prompts.rs:747-880`、`prompts/history_compaction.rs:317-328`
- 改法：`aggregate_*_scope_prompt` 拆为「材料块（进 `[canonical_inputs]`，插在 `Repository 路径` 之后、`关联上下文` 之前）+ 指令块（留消息尾部）」。材料块内**不得出现 `\n\n[`**（会截断 `generation_brief_section` 的段解析）。必须保留 `AGGREGATE_SCOPE_MARKER`（`## 聚合代码库成员清单`，`prompts.rs:44`）。
- 改法（续，条件化）：`must_replay_raw` 加宽为 `has_canonical_inputs && (mode == Reviewer || content.contains(AGGREGATE_SCOPE_MARKER))`——author 侧只对 lc 材料原文重放。`AGGREGATE_SCOPE_MARKER` 是 `prompts.rs:44` 的私有 `const`，同模块树可直接 `super::AGGREGATE_SCOPE_MARKER`。
- **单仓结论（owner 问题 6）**：单仓**不注入**成员清单/逐仓实况。理由：① `builder.rs` 只在 `RepositoryRouting::Logical` 时置 `aggregate_planning`，单仓走 legacy 分支，已有 `Repository: <name> (<id>)` + `Repository 路径` 两行即完整描述唯一仓；② 一旦注入，消息会命中 `AGGREGATE_SCOPE_MARKER`，使 `is_aggregate_planning_session`（`prompts.rs:411-416`）、`is_aggregate_story_or_design`（`:419-423`）与 structured-output 协议误判为聚合会话 → **必须不注入**，这是硬约束而非优选。
- 验收：lc 3 轮以上评审，reviewer prompt 仍完整含清单与实况；单仓 prompt 与今天字节一致（用 `part_31.rs:840-1070` 段断言 + 单仓快照对比）。
- 风险：中。段界写错会破坏 `[constraint_summary]`；由既有 prompt 断言兜住。

### 15｜author 与 reviewer 允许同 provider（仅记边界）
- 落点：本文 §八。**不改代码**，两形态同样适用。
- 验收：文档记录即完成。风险：无。

## 四、阶段二

### 5｜补逐仓实况（lc 专用）
- 落点：**新增** `src/product/logical_codebase/member_reality.rs`；接线 `planning_context_set.rs`（`resolve` 后逐成员渲染）与 `prompts.rs` 材料块
- 改法：每仓固定 4 行：`职责`（`RepositoryContextSet.role`，空→`未识别`）、`关键目录`（分层摘要 ≤12 项）、`入口`（根下 `Cargo.toml/package.json/go.mod/pom.xml/Makefile` 等，≤6）、`构建/测试命令`（可判定则 `cargo test`/`npm test`/`mvn test`，否则 `未识别`）。单仓（成员）≤2 KiB，`read_dir` 一层，**不做代码分析**。
- **单仓证明**：同条目 4——装配点在 lc 专属 resolver 之后，单仓不进；且 §三/条目 6 已定"单仓不注入成员材料"。
- 验收：lc 2 仓夹具（Rust+Vue）出现两段实况，缺项显 `未识别`。
- 风险：低-中。FS 读取限于成员仓根目录一层，每次 prompt 构建遍历一次。

### 16｜逐仓基线（lc 专用；**不改** `baseline_teaching.rs`——v1.1 相对 v1.0 的落点修正）
- 落点：`member_reality.rs`（基线行）+ `planning_context_set.rs` 装配处；`prompts/baseline_teaching.rs:47-73` **保留原样**
- 改法：逐仓调用既有 `resolve_effective_base_branch(member_checkout, issue.base_branch.as_deref())`，结果写入 item 5 的实况行 `基线: <branch>` / `基线: 未识别(<诊断>)`。`resolve_issue_baseline_tree` 的 lc 短路（`:50-52` `Ok(None)`）**保持不变**，仅更新注释说明"lc 基线由逐仓材料承担"；不引入存储迁移、不改 `BaselineTreeRef`、不碰 `cross_cutting`（lc 的 `baseline_tree` 仍 `None`，不假装硬边界）。
- **单仓证明**：`:50` 之后（`:53-73`）单仓代码逐字不动，仍 `repository_path` + `resolve_effective_base_branch` fail-closed；形态门禁在 `:50` 之前的判据不变。
- 验收：lc 2 仓 → 两行各自基线；某成员缺目标分支 → `未识别(<诊断>)` 且评审继续；单仓基线不可解析 → 仍 fail-closed 终止该轮。
- 风险：低。v1.0 曾打算改 `resolve_issue_baseline_tree`，本版改为"材料承担"——**对单仓更安全，且少改一个文件**。

### 8｜补实质 must-fix 类别（**lc 注入，单仓文案逐字不变**）
- 落点：`artifact_constraints.rs:142`(Story)/`:176`(Design)/`:208`(WorkItem)/`:238`(WorkItemPlan)；渲染器 `:506-515`（`reviewer_boundary_rules_for`）
- 改法：`reviewer_boundary_rules_for(workspace_type)` 增第二参 `logical: bool`，仅在 `true` 时追加 4 条：① 需求/范围漏项；② 仓库选择不合理（列了却不涉及/涉及却未列）；③ 验收不可判定；④ 跨仓影响缺失。参数由 `review` 各构建点传 `self.is_logical_codebase_issue()`。
- **单仓证明**：`:508` 的 `spec.reviewer_must_fix_rules` 在 `logical=false` 时输出与今天完全相同的字符串（同一 `Vec`、同一循环），文案逐字不变。
- 验收：`part_02/artifact_constraints_and_review_input.rs:321-360` 的 marker 断言 + 单仓快照对比（逐字）；lc 漏项夹具 → 出 `must_fix`。
- 风险：低。

### 9 / 10｜severity 与 revision 轮规则
- 落点：`prompts/review.rs:201-205`（通用面）、`:403-407`（outline 面）
- 改法：9 = 把"只有影响下一阶段可用性才能 must_fix"改为正面清单（漏项/选错仓/验收不可判定/跨仓缺失/与上游冲突 → `blocking|must_fix`；纯风格/美化/未来扩展 → `suggestion`）。10 = "只复核上一轮强返修项"追加"允许带具体证据时提新的实质问题，仍禁无证据发散"。
- **形态**：这两条是**判定缺陷修正**（现状把内容问题系统性压成 `suggestion`），与仓库形态无关，**默认两形态同时生效**。若 owner 要求单仓逐字不变，则需按 `is_logical_codebase_issue()` 分两套文案（成本：2 处 × 2 套字符串）——见 §九 待拍板 2。
- 验收：漏项夹具改前只出 `suggestion`、改后出 `must_fix`；第二轮新引入不可判定验收 → `must_fix`。
- 风险：低-中。会提高返修率（预期）；若拍板要求单仓不变，加门即可。

## 五、阶段三（与形态无关，两形态同样生效）

### 11｜finding 缺 `evidence`/`required_action` 不默认空串
- 落点：`parsers.rs:785-880`（宽松入口），`:855-866`
- 改法：缺失/空串 → `malformed = true` 且该条丢弃，去掉 `unwrap_or("")`。严格入口 `:611-700` 不变。
- 验收：缺字段夹具 → 该 finding 不进 `findings` 且 `malformed=true`。风险：低-中（丢 must_fix），与条目 12 重试配合。

### 12｜verdict 结构错误给诊断码并可自动重试一次
- 落点：`review/drive.rs` 两个镜像（direct `:150-200`、gateway `:495-560`）、`structured_output.rs:265-290`
- 改法：repair 门槛从 `provider_allows_review_repair(&reviewer) && is_repairable()` 放宽为 `is_repairable() || matches!(Schema)`（四条 provider 各重试一次）；`is_classification_fatal()` 分支严格不动；仍失败 → 既有 `fallback_review_verdict`（diagnostic code 非空）。
- 验收：结构错误 verdict → 观察到 1 次 `structured_output_repair_event` + 第二次成功；两次失败 → `needs_human` 且 code 非空。风险：中（多一次 provider 调用）。

### 13｜旧轮摘要保留 `message`/`evidence`
- 落点：`history_compaction.rs:273-292`、`:344-385`
- 改法：`FindingSummary` 增 `message`/`evidence`；渲染补两行、解析补两前缀，缺省 `(not recorded)`。
- 验收：`history_compaction.rs:620-700` 夹具加断言含原始 message/evidence。风险：低（摘要变长，复跑字符预算断言）。

### 14｜WorkItem splitter 目录摘要分层 + 失败显式
- 落点：`work_item_split_engine/context.rs:147-172`；调用点 `prompts.rs:444,475,550,595`、`web/.../single_candidate.rs:915`
- 改法：改分层摘要（顶层 ≤12 + 每项一级子目录 ≤6，总 ≤40 行，复用 `member_reality` 实现）；`read_dir` 失败返回 `repository_structure_unavailable: <io error>`，不再 `"(empty repository)"`。
- 验收：空仓夹具 → 显式 unavailable；多层仓 → 出现二级条目。风险：低（`single_candidate.rs:915` 共用，需确认调用方不解析其文本）。

## 六、要删 / 要保留（条件化清单）

**物理删除（两形态皆无消费者）**
- `workspace_repository.rs`：`resolve_primary_member_repository` + `logical_repository_primary_member` kind（唯一调用点在 lc 不可达分支）
- `planning_context_set.rs`：`InventoryInjectionBudget`、soft/hard 裁剪、`omitted_member_count` 渲染、`InventoryInjection.omitted_member_ids`/`truncated`
- `drive.rs`：`plan_review_window_worktree` 的 lc 分支（`:1695` 后至 `:1746`）、`emit_primary_member_anchor_note`、`primary_anchored`；窗口内联测试中 lc 形态的 primary 用例
- `work_item_split_engine/context.rs`：`entries.truncate(30)` 与 `"(empty repository)"`
- 事件/文案：「审查窗口主成员锚定(primary-member anchored;…R8 类已知边界)」；`prompts.rs:759/812/866` 的"任意单一 primary 仓库"
- `primary` 命名：`resolve_primary_member_repository`、`resolve_selected_logical_repository(…, primary, …)` 入参名与注释（**仅改名，函数保留**——它有 4 处活调用 `workspace_repository.rs:113,193,379,411`；`:307` 那处属待删函数内部）、"注册序=primary"、`primary_anchored`、`emit_primary_member_anchor_note`、`drive_tests.rs:390-400,700-900` 的 primary 用例

**物理保留（单仓唯一使用者，一行不改）**
- `begin_plan_review_window`(`:1752-1814`)、`assert_plan_review_window`(`:1816-1840`) 及其 6 处调用、`close_plan_review_window`(`:1857-1955`)、`plan_review_main_scope`、`plan_review_launch_ownership`(`:1670-1685`)、`plan_reviewer_window_store`/`plan_reviewer_snapshot_store`、`PLAN_REVIEWER_SNAPSHOT_STORE_DIR`/`PLAN_PROVIDER_RESOURCE_STORE_DIR`/`PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED`、`ActivePlanReviewWindow`、`WorkspaceSession.active_review_window`、`emit_plan_review_window_attention` 及其 5 条文案、`reviewer_verify_policy` 的窗口 id 分支、`review_launch_target_and_cwd` 的 `:628-633`、`resolve_issue_baseline_tree` 的 `:53-73`

## 七、不要做

- 不做多成员对账域、集合屏障、报告成员分节、根 guard、物理 no-exec、ZeroWrite 分档。
- 不改 coding 侧任何文件；不改 `coding_workspace_engine` 的窗口/对账。
- 不改 `cross_cutting/streaming_provider` 的 `BaselineTreeRef` 为多值。
- 不引入新存储/账本/窗口类型/schema 字段/前端组件；不改 `ReviewInvocationScope`。
- **不改任何单仓可达代码的行为**（含 prompt 字节、事件、门控、窗口记录）；不把 `is_aggregate_planning_session` 当形态判据。
- 不让 WorkItem splitter 一次吃多仓（见 §十 后置项）。

## 八、验收：单仓回归 + lc 新逻辑

**单仓（`logical_codebase_id: None`）逐项一致**
1. 窗口：`reviewer-windows/windows/` 下新增记录、window id 与 generation 序列不变。
2. 事件：`plan_review_window_*` 错误码路径可达；无新事件、无文案变更。
3. 基线：`resolve_issue_baseline_tree` 仍 fail-closed（分支删/无分支 → 终止该轮）。
4. 门控：`assert_plan_review_window` 6 处仍在；`reviewer_verify_policy` 仍返回窗口 id（canonical digest 不变）。
5. prompt：workspace context 与 reviewer prompt 字节一致（无成员清单、无逐仓实况）；`[artifact_boundary_must_fix_rules]` 文案逐字不变。
6. 硬证据：`drive.rs:2063,2174,2373` 的 `None` 夹具窗口用例、`drive_tests.rs` 全部窗口生命周期用例**不改一行且全绿**。

**多仓（`Some(_)`）新逻辑**
1. issue 选 2 仓 → story/design/work item 各跑通 author→reviewer→verdict。
2. reviewer prompt 每轮含 2 仓完整清单 + 逐仓实况 + 逐仓基线；无 `primary` 字样。
3. 无审查窗口记录、无 `primary-member anchored` 事件、无 `plan_review_window_*` 错误码。
4. 超预算 → `planning_material_over_budget` 阻断；集合解析失败 → `plan_review_material_unresolved`。
5. 3 轮以上长会话，材料仍完整可见。

## 九、待 owner 拍板（3 项）

1. **`ReviewerVerify` 两种引用如何共存**：本方案在同一函数内 `match self.active_review_window`——单仓仍用**窗口 id**（digest 与今天一致），lc（无窗）用 **`session_id`** 作合同引用。请确认这种"按形态选引用"的实现可接受；若要求 lc 也必须用某种结构化引用，则需先新增一个不依赖窗口的 policy intent（跨 4 家 adapter，成本上升）。
2. **条目 9/10（severity 与 revision 轮规则）是否对单仓也生效**：本方案默认**两形态同时生效**（修的是判定缺陷，不是形态问题，且单仓同样把内容问题压成 `suggestion`）。若要求单仓逐字不变，则改为按 `is_logical_codebase_issue()` 分两套文案（2 处）。
3. **lc 逐仓基线失败语义**：本方案取"该仓标 `未识别(<诊断>)`、评审继续"（规划产出不落仓，基线仅作参考）；单仓保持 fail-closed。是否要求 lc 也 fail-closed（任一成员缺目标分支即整轮终止）。
