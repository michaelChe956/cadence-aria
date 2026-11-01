# 技术方案：story / design / work item 三环节评审修正（v1.2，逻辑代码库限定）

> 性质：方案，**不含实现代码**。v1.0 / v1.1 保留（`…_v1.0.md`、`…_v1.1.md`），本文为 owner 2026-10-23 逐条裁定后的修订版。标准依据：`cadence/designs/2026-10-22_技术方案_系统环节与角色标准_v1.0.md`。
> **硬约束（owner 原话）**："这两个方案都不能影响单仓的使用哈，都是在多仓上做改造。"
> → **单仓库代码库（`logical_codebase_id == None`）行为必须与今天完全一致**；所有改动只在多仓库逻辑代码库（`Some(_)`）生效。
> 理念：先做通、后打磨；先解决阻断与理念问题；不过度设计。
> 工作目录校验（bash 实测）：`pwd` = `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`；`git branch --show-current` = `feat-b-0808-add-monorepo`。✅
> **行号口径**：本文所有行号均为 2026-10-23 bash 实测（`wc -l` / `grep -n` / `sed -n`）。本仓 `read`/CodeGraph 快照过期，二者冲突以 bash 为准。v1.1 中被实测推翻的落点已在正文标注。

## 〇、owner 裁定（9 条，不可改）与本版落点

| # | 裁定 | 落点 |
|---|---|---|
| 1 | 多仓评审器身份不用会话编号 → **每会话新生成 UUID**，与会话一一绑定，会话删除即清除；单仓维持窗口编号；**author 侧同样改造** | 新条目 **17**（阶段一，独立改动点） |
| 2-3 | 第 9 条 severity、第 10 条 revision 轮只看旧问题：**均不改**（story 有人工审核；不发散） | 条目 9 / 10 → 「不改」 |
| 4 | 第 11 条缺 `evidence`/`required_action`：**改**（不收） | 条目 11（只多仓） |
| 5 | 第 12 条 verdict 结构错误静默降级：**先不改**，单列已知缺陷 | §十 待办 1 |
| 6 | 第 13 条旧轮历史丢 evidence：**改**（保留 message+evidence） | 条目 13（只多仓） |
| 7 | 第 14 条 splitter 摘要：**改**（分层 + 失败显式） | 条目 14（只多仓） |
| 8 | 第 11/13/14 作用范围：**只多仓**；单仓与今天完全一样 | 三条均以形态门禁分派 |
| 9 | 逐仓基线 = 各仓 **当前 HEAD 所在分支**（`git rev-parse --abbrev-ref HEAD`），不再猜 main/master；detached / 空仓 → 该仓标「基线未识别」，其余仓继续；单仓仍 fail-closed | 条目 16（只多仓） |

## 一、形态门禁（全文唯一判定键）

**判定键（owner 已核准）**：`IssueRecord.logical_codebase_id`（`src/product/models/project.rs:141-143`）。

| 形态 | 判定 | 本批处理 |
|---|---|---|
| 单仓库代码库 | `None` | **完全现状**：评审窗口、`begin_plan_review_window`、基线/对账/恢复、`plan_review_window_worktree` 早返回、`assert_plan_review_window`、`reviewer_verify_policy` 窗口 id、事件与文案。**零改动** |
| 多仓库逻辑代码库 | `Some(_)` | 走本文条目 |

**引擎侧判据（实测简化）**：`logical_provider_gateway().is_some()`——`src/product/workspace_engine/lifecycle.rs:662-667` 既有口径即「非空即逻辑会话(Task 11 注入)」。**不引入 store 读取**（v1.1 曾提"store 可用则 `IssueStore::get`"，实测引擎无 issue store 句柄，该写法取消）。

**唯一门禁函数**（`prompts.rs`，紧邻 `is_aggregate_planning_session`；后者是"聚合视野"判据，不是形态判据，不拿它代替）：
`fn is_logical_codebase_issue(&self) -> bool { self.logical_provider_gateway().is_some() }`

**无引擎句柄处**（splitter 静态方法等）：直接用 `issue.logical_codebase_id.is_some()`（`IssueRecord` 是入参）。
**总原则**：所有删除项一律"条件化"——不物理删除单仓仍在用的共享路径；新逻辑只挂在多仓分支。

## 二、阶段划分（3 阶段，每阶段结束可跑通）

| 阶段 | 条目 | 阶段结束判定 |
|---|---|---|
| 一｜lc 身份 + 删窗 + 集合权威 + 材料稳定注入 | **17**, 1, 2, 3, 4, 6, 15 | 单仓 story/design/work item 全链路与今天逐项一致（窗口/事件/基线/门控/digest）；lc 选 2 仓跑通 author→reviewer→verdict，reviewer prompt 含 2 仓完整清单、无窗口记录、身份为会话 UUID |
| 二｜lc 逐仓实况与判定 | 5, 16, 8 | 材料含每仓实况与基线行；漏项/选错仓/验收不可判定夹具必出 `must_fix`；单仓 reviewer 文案逐字不变（条目 8） |
| 三｜lc 解析与容错 | 11, 13, 14 | 各负例夹具通过；单仓同输入输出字节一致（9/10 **不改**；12 移入 §十 待办） |

## 三、阶段一

### 17｜每会话 UUID 身份（**独立改动点**；lc 用 UUID，单仓保持窗口编号）
- **现状（bash 实测）**：
  - reviewer 身份 = 窗口编号：`role_run_window_id(session_id, run_id)` → `"{attempt}-{role_run}-w01"`（`src/product/reviewer_window_store.rs:60-61`）；由 `begin_plan_review_window`（`review/drive.rs:1757-1767`）铸造。
  - 该 id 经 `reviewer_verify_policy()`（`review/drive.rs:1842-1846`）→ `ProviderToolPolicy::reviewer_verify(window_id)`（`src/cross_cutting/streaming_provider/mod.rs:125-130`）→ canonical digest 域 `reviewer-verify-window:{window_id}`（`mod.rs:859-874`）；8 处 prompt 构建消费该方法（`prompts/review.rs:220,422,563,745,824`、`review_parts/draft_input.inc.rs:128,297`、`review_repair.rs:55`）。
  - author 身份 = `resource_scope_id`，`author_root_launch.inc.rs:232` 传 `format!("role_run_author_{}", session_id)` → 进 `env_policy_digest`（`provider_execution.rs:552-561`）。
- **改法**：
  1. **数据**：`WorkspaceSessionRecord`（`models/workspace.rs:807`）新增 `#[serde(default)] pub review_identity_id: String`；`create_workspace_session`（`lifecycle_store/workspace.rs:308-353`）赋值 `uuid::Uuid::new_v4().simple().to_string()`（`uuid` v4 已在 `Cargo.toml:42`，同仓用例 `coding_attempt_store/attempt.rs:321`）；`WorkspaceSession`（`types.rs:75`）加同名字段，`from_record`（`types.rs:123`）镜像。
  2. **存量记录**（字段为空）：首次读取时生成并**持久化一次**（新增 `LifecycleStore::set_workspace_session_review_identity`，已有值不覆盖）。
  3. **清除**：随 `WorkspaceSessionRecord` 的磁盘删除自动消失（`web/handlers/lifecycle/deletion.rs:226` 删 sessions 目录；`:562-575` 注释确认删除后读不到记录）——**不新建映射 store、不写清理钩子**。
  4. **reviewer**：`reviewer_verify_policy()` 改 `match self.active_review_window { Some(a) => reviewer_verify(a.handle.window_id.clone()), None => reviewer_verify(self.session.review_identity_id.clone()) }`——8 处调用点零改动。
  5. **author**：`author_root_launch.inc.rs:232` 改为 `if self.is_logical_codebase_issue() { format!("role_run_author_{}", self.session.review_identity_id) } else { format!("role_run_author_{}", self.session.session_id) }`。
- **单仓证明**：单仓恒有窗口 → `reviewer_verify_policy` 走 `Some` 分支，digest 与今天逐字节一致；author 侧 `else` 分支原文；`role_run_window_id` 与其窗口 store 一行不改。
- **验收**：lc 同一会话跨轮/跨 repair，digest 域恒为 `reviewer-verify-window:<同一个 uuid>`；删除会话后该 uuid 不再被任何记录引用；单仓 digest/窗口编号不变。
- 风险：中（改的是合同 digest 域，但只走 lc 分支）。**不采用** uuid v5 派生（owner 要求"每会话新生成"且映射随会话删除）。

### 1｜删除「主仓 / 主成员」概念（仅 lc 分支）
- 落点：`review/drive.rs:1689-1747`（`plan_review_window_worktree`，lc 分支 `:1698-1746`）、`:1998-2020`（`emit_primary_member_anchor_note`）、调用点 `:54`/`:386`、`src/product/workspace_repository.rs:291-318`、`prompts.rs:766/818` 文案。
- 改法：lc 不再进入 `plan_review_window_worktree`（条目 3 已让 lc 跳过），故删除其 lc 分支；`resolve_primary_member_repository`、`logical_repository_primary_member` kind、`emit_primary_member_anchor_note`、`primary_anchored` 变量删除；文案"任意单一 primary 仓库"→"任意单一仓库"。
- **单仓证明**：① `:1695-1697` 的 `if target.join(".git").exists() { return Ok((target, false)) }` 是单仓唯一出口，primary 分支在其后不可达；② `resolve_primary_member_repository` 唯一调用点在 `:1727`（早返回之后）；③ 该函数对 `RepositoryRouting::Legacy` 直接 `Err`（`workspace_repository.rs:311-315`）；④ primary 用例全部为 `logical_fixture`（`drive_tests.rs:557,562` `logical_codebase_id: Some(...)`），窗口内联测试才是 `None`。
- 验收：单仓评审仍走 `:1695-1697` 早返回；lc 侧 `grep -i primary` 在评审链路零命中。风险：低。

### 2｜单值字段降级为兼容（仅 lc 分支）
- 落点：`prompts/review_context.rs:618-634`、`models/project.rs:138`（`base_branch` 兼容字段）、`types.rs:119`。
- 改法：gateway 分支（`:621-627`）对 lc 返回 `(authority_root, Some(authority_root))`，不再要求 `repository_path` 是成员 worktree；**`:628-633` else 分支（单仓）逐字不动**。字段保留，注释"兼容字段，非 lc 范围权威"。
- **单仓证明**：gateway 为 `None` → 直接落 `:628-633`，`:629` 仍用 `session.repository_path`（字段不能删）。
- 验收：单仓 `working_dir` 仍取 `repository_path`、`working_directory` 仍 `None`；lc reviewer 两者均为 LC 根。风险：低-中。

### 3｜lc 会话不再开窗（单仓保留窗口 + 零锁恢复）
- **现状（实测，修正 v1.1）**：入口与内层**已分开**——`drive_review_session`（`review/drive.rs:24-73`）做 target 解析(:45)/primary 事件(:53-60)/`begin_plan_review_window`(:61-68)/内层(:69)/`close_plan_review_window`(:72)；内层 `drive_review_session_with_window`（`:78`）已无窗口语义。gateway 侧同构：`:363`（入口）/`:407`（内层）。**v1.1 说的"内层改名去掉 `_with_window`"取消——无需改名。**
- 改法：
  1. 两个入口首行加门：`if self.is_logical_codebase_issue() { return self.drive_review_session_with_window(provider, command_rx).await; }`（gateway 同理）→ lc 完全跳过 target/primary/begin/close。
  2. `assert_plan_review_window`（`:1816-1840`）内部改为形态分派：lc → 新增 `assert_plan_review_material()`；单仓 → 原函数体。**6 处调用点（`:128,196,326,466,539,670`）零改动**（v1.1 曾打算逐处改，改为函数内分派，少 6 处改动）。
  3. `assert_plan_review_material()`：lc 会话调 `PlanningContextSetResolver::resolve`（store 读，无索引/无 FS 遍历）+ 材料渲染；集合空或渲染失败 → `plan_review_material_unresolved: <cause>`。在 `build_review_input()` 前与 repair 前各一次（由上述 6 处原调用点自然覆盖）。
- **单仓证明**：`begin_plan_review_window`/`close_plan_review_window`/`plan_review_main_scope`/两个 store 构造/两个常量/`ActivePlanReviewWindow`/`active_review_window`/`emit_plan_review_window_attention` **全部物理保留且入口原序列不动**；`assert_plan_review_window` 单仓分支即原体；6 调用点未触碰；窗口内联测试（`drive.rs:2063,2174,2373` 的 `None` 夹具）必须全绿。
- 验收：单仓 `reviewer-windows/windows/` 仍产新记录、`plan_review_window_*` 错误码仍可达；lc 无新记录、无窗口错误码、无 primary 事件。风险：中（本批唯一大改，风险收敛在 lc 分支）。

### 4｜仓库清单全量注入，超预算显式阻断（lc 专用）
- 落点：`src/product/logical_codebase/planning_context_set.rs:157-300`（`InventoryInjectionBudget`/`render_compact_inventory`：`:217` 全量→`:228` 裁 profile→`:243` 只留目标成员 + `omitted_member_ids`）、`logical_codebase/mod.rs:103-104`。
- 改法：删 soft/hard 裁剪与 `omitted_member_count`/`omitted_member_ids`/`truncated`；`render_compact_inventory` 改全量渲染 + 绝对上限 500 KiB，超限 `Err(InvalidRecord{kind:"planning_material_over_budget"})`。
- **单仓证明**：该 resolver 只在 `RepositoryRouting::Logical` 分支构造（`web/workspace_context/builder.rs:141-199` 置 `aggregate_planning`）；单仓 `aggregate_planning=None`，永不进入。`InventoryInjection*` 类型面属 lc 独占。
- **边界**：`logical_codebase/aggregate_index/freshness.rs:214/284/328` 另有一套 `trimmed_profile_count`/`omitted_member_count` 渲染（`AggregateIndexRenderer`），**无生产调用者**，本批不改（§十 待办 3）。
- 验收：lc N=3 全量命中；超 500 KiB → 明确错误码且评审不启动；单仓 prompt 字节不变。风险：低。

### 6｜材料改稳定注入（lc 专用；单仓**不注入**成员清单）
- 落点：`prompts.rs:754-775`（`aggregate_story_scope_prompt`，marker 文案在 `:759`）、`:44`（`const AGGREGATE_SCOPE_MARKER`）、`prompts/history_compaction.rs:317-328`（`must_replay_raw`）、`web/workspace_context/builder.rs:141-199`。
- 改法：`aggregate_*_scope_prompt` 拆「材料块（进 `[canonical_inputs]`，插在 `Repository 路径` 之后、`关联上下文` 之前）+ 指令块（留消息尾部）」；材料块内**不得出现 `\n\n[`**（会截断 `generation_brief_section` 段解析）。保留 `AGGREGATE_SCOPE_MARKER`（`prompts.rs:44`）。`must_replay_raw` 加宽为 `has_canonical_inputs && (mode == Reviewer || content.contains(AGGREGATE_SCOPE_MARKER))`（`AGGREGATE_SCOPE_MARKER` 为私有 const，同模块树 `super::AGGREGATE_SCOPE_MARKER`）。
- **单仓结论**：单仓**不注入**成员清单/逐仓实况。硬理由：`builder.rs` 单仓走 legacy 分支，已有 `Repository: <name> (<id>)` + `Repository 路径` 两行即完整；一旦注入会命中 `AGGREGATE_SCOPE_MARKER`，使 `is_aggregate_planning_session`（`prompts.rs:432`）、`is_aggregate_story_or_design`（`:419-423`）与 structured-output 协议误判为聚合会话 → **必须不注入**。
- 验收：lc 3 轮以上，reviewer prompt 仍完整含清单与实况；单仓 prompt 与今天字节一致（`part_31.rs:840-1070` 段断言 + 快照对比）。风险：中（段界写错会破坏 `[constraint_summary]`，由既有断言兜住）。

## 四、阶段二

### 5｜补逐仓实况（lc 专用）
- 落点：**新增** `src/product/logical_codebase/member_reality.rs`；接线 `planning_context_set.rs`（`resolve` 后逐成员渲染）与 `prompts.rs` 材料块。
- 改法：每仓固定 4 行：`职责`（`RepositoryContextSet.role`，空→`未识别`）、`关键目录`（分层摘要 ≤12 项）、`入口`（根下 `Cargo.toml/package.json/go.mod/pom.xml/Makefile` 等，≤6）、`构建/测试命令`（可判定则 `cargo test`/`npm test`/`mvn test`，否则 `未识别`）+ **`基线` 行（由条目 16 提供）**。单仓（成员）≤2 KiB，`read_dir` 一层，不做代码分析。
- **单仓证明**：装配点在 lc 专属 resolver 之后；条目 6 已定"单仓不注入成员材料"。
- 验收：lc 2 仓夹具（Rust+Vue）出现两段实况，缺项显 `未识别`。风险：低-中（每次 prompt 构建遍历成员仓根目录一层）。

### 16｜逐仓基线 = 各仓当前 HEAD 分支（lc 专用；owner 裁定 9）
- **现状（实测）**：唯一解析链 `resolve_effective_base_branch`（`src/product/issue_baseline.rs:98-114`）；`None` 口径走 `default_local_branch`（`:84-91`）——**硬编码 `["main","master"]` 猜测**，皆无即 `NoDefaultBranch`；`resolve_issue_baseline_tree`（`prompts/baseline_teaching.rs:47-73`）lc 短路 `Ok(None)` 在 `:50-52`。
- 改法：
  1. `issue_baseline.rs` 新增 `resolve_member_head_branch(checkout: &Path) -> Result<String, String>`，复用既有 `run_git`（`:52-61`）执行 `git rev-parse --abbrev-ref HEAD`：成功且输出 ≠ `HEAD` → 分支名；输出 `HEAD`（detached）、命令失败、空仓（无 commit）→ `Err(<诊断>)`。
  2. `member_reality.rs` 逐仓调用它，写成 `基线: <branch>` / `基线: 未识别(<诊断>)`；**该仓失败只影响本仓那一行，评审继续**（不整轮终止）。
  3. `resolve_issue_baseline_tree` 的 lc 短路与 `:53-73` 代码不动，仅更新注释"lc 基线由逐仓材料承担"；不引入存储迁移、不改 `BaselineTreeRef`、不碰 `cross_cutting`（lc 的 `baseline_tree` 仍 `None`，不假装硬边界）。
- **单仓证明**：`resolve_effective_base_branch`/`default_local_branch` 一行不改 → 单仓仍 main→master 猜测且不可解析即 fail-closed 终止该轮；`:53-73` 逐字不动。
- 验收：lc 2 仓 → 两行各自 HEAD 分支（不再出现"猜 main"）；某仓 detached/空仓 → 该行 `未识别(<诊断>)` 且评审继续；单仓基线不可解析 → 仍 fail-closed。风险：低。

### 8｜补实质 must-fix 类别（**lc 注入，单仓文案逐字不变**）
- 落点：`artifact_constraints.rs:142`(Story)/`:176`(Design)/`:208`(WorkItem)/`:238`(WorkItemPlan) 的 `reviewer_must_fix_rules`；渲染器 `:507-516`（`reviewer_boundary_rules_for`，循环在 `:510`）。
- 改法：`reviewer_boundary_rules_for(workspace_type, logical: bool)`，仅 `true` 时追加 4 条：① 需求/范围漏项；② 仓库选择不合理（列了却不涉及/涉及却未列）；③ 验收不可判定；④ 跨仓影响缺失。参数由 `review` 各构建点传 `self.is_logical_codebase_issue()`。
- **单仓证明**：`logical=false` 时同一 `Vec`、同一循环，输出字符串与今天完全相同。
- 验收：`part_02/artifact_constraints_and_review_input.rs:321-360` marker 断言 + 单仓快照逐字对比；lc 漏项夹具 → 出 `must_fix`。风险：低。

### 9 / 10｜severity 分级与 revision 轮收敛——**均不改**（owner 裁定 2、3）
- 9 现状 `prompts/review.rs:201-202`（story/design/work item）、`:403-404`（outline）；10 现状 `:205`、`:407`。
- **决定：两处原文保持**。理由：①三环节 artifact 都进**人工确认**（human gate），severity 误判由人兜底，改文案只会抬高返修率并改变单仓 prompt 字节；②"只复核上一轮强返修项"是**有意的收敛**（防发散把长会话拖成无限返修），不是缺陷，v1.1 的"允许带证据提新问题"会把闭环重新打开。
- 验收：无代码改动；`prompts/review.rs:201-205`、`:403-407` 与今天逐字一致。风险：无。

## 五、阶段三（**只多仓**；单仓与今天完全一样——owner 裁定 8）

### 11｜finding 缺 `evidence`/`required_action` 不收（只多仓）
- **现状（实测）**：`parse_review_findings`（`src/product/workspace_engine/parsers.rs:785-883`）对两字段缺失取 `unwrap_or("")`（`:859-868`）→ 空证据 finding 仍进入 `findings`。而 reviewer schema 早已声明二者必填（`prompts/review.rs:199`、`:396`）——声明未落成门。
- **改法（不动 parsers.rs）**：parsers 的调用面含历史 JSON 与测试（`parsers.rs:136,256,267`），改写签名会波及单仓；改为在 `parse_review_completion_for_active_node`（`review/structured_output.rs:226-263`）解析成功后加 lc 分支：`self.is_logical_codebase_issue()` 且任一 finding 的 `evidence.trim().is_empty() || required_action.trim().is_empty()` → `Err(ReviewCompletionError::Schema(ReviewStructuredOutputErrorCode::MalformedFindings))`。该码与既有 malformed 同路：不在 `is_classification_fatal`（`:53-61`）内；因条目 12 未修，`is_repairable()==false`（`:63-82`）→ `fallback_review_verdict`（`:266-286`）→ `needs_human` + `diagnostic.code = malformed_findings`（用户看到明确码，而非静默收下空证据）。
- **单仓证明**：分支前置 `is_logical_codebase_issue()`；单仓解析路径字节不变。
- 验收：缺字段夹具 → lc `needs_human` 且 code=`malformed_findings`、finding 不入 `findings`；单仓同夹具行为与今天一致（照收）。风险：低-中（lc 返修率上升，属预期）。

### 13｜旧轮摘要保留 `message`/`evidence`（只多仓）
- **现状（实测）**：`FindingSummary{severity, required_action}`（`prompts/history_compaction.rs:346-350`）；`extract_findings`（`:352-386`）只认 `severity:`/`required_action:` 两前缀（`:362-377`）；`render_round_summary`（`:275-292`）只输出这两行。而 reviewer 原文里 message/evidence 是存在的（见既有夹具 `:637`），被解析丢弃。
- **改法**：`FindingSummary` 增 `message`/`evidence`；解析补 `message:`/`evidence:` 前缀（缺省 `(not recorded)`）；渲染补两行。`HistoryCompactionInput`（`:16-22`）增 `detail_findings: bool`，4 个调用点传 `self.is_logical_codebase_issue()`（`prompts.rs:540`、`prompts/review.rs:159`、`prompts/revision.rs:154`、`prompts/author_revision.rs:57`）；`false` 时渲染分支短路为今天的字符串。
- **单仓证明**：`detail_findings=false` 时输出与今天逐字相同（渲染处单一 `if`）。
- 验收：`history_compaction.rs:620-700` 夹具加断言含原始 message/evidence（lc 夹具）；单仓压缩结果快照逐字不变。风险：低（lc 摘要变长 → 复跑字符预算断言）。

### 14｜WorkItem splitter 目录摘要分层 + 失败显式（只多仓）
- **现状（实测）**：`summarize_repository_structure`（`src/product/work_item_split_engine/context.rs:147-168`）：`if let Ok(reader)` 吞掉 `read_dir` 错误（`:149`）、`entries.truncate(30)`（`:162`）、空仓返回伪内容 `"(empty repository)"`（`:164`）。
- **改法**：签名增 `logical: bool`。lc：分层摘要（顶层 ≤12 项 + 每个顶层目录一级子项 ≤6，总 ≤40 行，复用条目 5 的 `member_reality` 实现）；`read_dir` 失败 → `repository_structure_unavailable: <io error>`；空仓 → `repository_structure_empty: <path>`（不再用伪内容）。单仓：`logical=false` 走原实现，原字符串逐字。
- **调用点（5 处，全部需传参）**：`work_item_split_engine/prompts.rs:444,475,550,595`（静态方法，传 `issue.logical_codebase_id.is_some()`，`issue` 是入参）、`web/workspace_ws_handler/run/single_candidate.rs:915`（`engine` 在作用域，同口径）。该值只作 prompt 文本（`:914` 赋值后交给 prompt 构建），不改行为契约。
- **单仓证明**：`false` 分支 = 原 4 行实现（`truncate(30)` + `"(empty repository)"` 原样）。
- 验收：lc 空仓夹具 → 显式 `repository_structure_unavailable`/`empty`；多层仓 → 出现二级条目；单仓 prompt 逐字不变。风险：低。

## 六、逐改动点：现状 → 改后效果

| # | 现状（文件:行，bash 实测） | 改后效果（用户可见） |
|---|---|---|
| 17a | reviewer 身份 = `{session}-{run}-w01` 窗口编号（`reviewer_window_store.rs:60-61` → `review/drive.rs:1842-1846`） | lc 评审器身份变为**与会话一一绑定的 UUID**，同会话跨轮/跨 repair 恒定；单仓仍是窗口编号 |
| 17b | author `resource_scope_id` = `role_run_author_<session_id>`（`author_root_launch.inc.rs:232`） | lc author 的 `resource_scope_id` 变为 `role_run_author_<会话 UUID>`；单仓不变 |
| 17c | 会话记录无身份字段；删除会话时无身份映射可清 | `WorkspaceSessionRecord` 增 `review_identity_id`，随会话记录删除自动清除（`deletion.rs:226`） |
| 1 | lc 评审经 `plan_review_window_worktree` 解析出 primary 成员并 emit "primary-member anchored"（`review/drive.rs:54,386,1689-1747`） | lc 不再出现 primary 概念/事件，reviewer 一次看到全部成员；单仓窗口路径不变 |
| 2 | lc reviewer target 必须是成员 worktree，缺则报 `logical plan review requires an explicit member target worktree`（`review_context.rs:621-627`） | lc reviewer 以 LC 根为 target/cwd，不再因成员 worktree 缺失启动失败；单仓 `:628-633` 不变 |
| 3 | `drive_review_session` 无条件开窗、收窗，`assert_plan_review_window` 对 lc 也生效（`drive.rs:24-73,1816-1840`） | lc 不开窗、无窗口错误码；材料缺失改为 `plan_review_material_unresolved: <cause>` 明确阻断；单仓窗口与零锁恢复不变 |
| 4 | lc 清单按 soft/hard 预算裁剪，附 `omitted_member_count`（`planning_context_set.rs:217-243`） | lc 清单全量注入；超 500 KiB 报 `planning_material_over_budget`，评审不启动（不静默缺员） |
| 6 | 聚合清单粘在消息尾部，历史压缩/长会话后丢失；单仓无该块 | lc 清单进 `[canonical_inputs]` 稳定重放，3 轮以上评审仍可见；单仓 prompt 字节不变 |
| 5 | 材料只有成员 id/profile，无逐仓实况 | lc 每仓出现「职责/关键目录/入口/构建测试命令/基线」5 行，缺项显 `未识别` |
| 16 | 默认链硬编码猜 `main`→`master`（`issue_baseline.rs:84-91`） | lc 基线 = 各仓 `git rev-parse --abbrev-ref HEAD`；detached/空仓该仓标「基线未识别」，其余仓继续；单仓仍 fail-closed |
| 8 | reviewer must-fix 规则无多仓类别（`artifact_constraints.rs:507-516`） | lc reviewer 必查：需求漏项/仓库选择不合理/验收不可判定/跨仓影响缺失；单仓文案逐字不变 |
| 11 | 缺 `evidence`/`required_action` 取空串照收（`parsers.rs:859-868`） | lc 此类 finding 不收，判定 `needs_human` 且 code=`malformed_findings`（不再"空证据也通过"）；单仓照旧 |
| 13 | 旧轮摘要只留 severity/required_action（`history_compaction.rs:275-292,346-350`） | lc 旧轮 finding 保留 message+evidence，长会话返修有据可查；单仓摘要逐字不变 |
| 14 | splitter 目录摘要截断 30 条、read_dir 失败静默成 `(empty repository)`（`work_item_split_engine/context.rs:147-168`） | lc 摘要分层可读，读取失败显式 `repository_structure_unavailable: <io error>`；单仓摘要逐字不变 |
| 9 | severity 规则（`prompts/review.rs:201-202,403-404`） | **不改**：保持"只有影响下一阶段可用性才能 must_fix"，由人工审核兜底 |
| 10 | revision 轮只看旧问题（`prompts/review.rs:205,407`） | **不改**：保持"只复核上一轮强返修项、不发散" |
| 12 | schema 类结构错误不重试，直接 `needs_human`（`structured_output.rs:43-82,266-286`） | **本轮不改**，见 §十 待办 1（现状：用户只看到"需人工确认"，诊断码在 `structured_output_diagnostic`） |

## 七、要删 / 要保留（条件化清单）

**删除（lc 分支及其无消费者部分）**
- `workspace_repository.rs`：`resolve_primary_member_repository`（`:291`）+ `logical_repository_primary_member` kind
- `review/drive.rs`：`plan_review_window_worktree` 的 lc 分支（`:1698-1746`）、`emit_primary_member_anchor_note`（`:1998-2020`）、`primary_anchored`；窗口内联测试中 lc 形态 primary 用例
- `planning_context_set.rs`：`InventoryInjectionBudget`、soft/hard 裁剪、`omitted_member_count` 渲染、`InventoryInjection.omitted_member_ids`/`truncated`
- `work_item_split_engine/context.rs`：`entries.truncate(30)` 与 `"(empty repository)"`（仅 lc 分支；单仓分支保留原文）
- 事件/文案：「审查窗口主成员锚定(primary-member anchored;…R8 类已知边界)」；`prompts.rs:766/818` 的"任意单一 primary 仓库"
- `primary` 命名：`resolve_selected_logical_repository(…, primary, …)` 入参名与注释（**仅改名，函数保留**——4 处活调用 `workspace_repository.rs:113,193,379,411`）、primary 相关测试用例

**保留（单仓唯一使用者，一行不改）**
- `begin_plan_review_window`(`drive.rs:1752-1814`)、`assert_plan_review_window`(`:1816-1840`) 及 6 处调用、`close_plan_review_window`(`:1857-1955`)、`plan_review_main_scope`、`plan_review_launch_ownership`(`:1670-1685`)、`plan_reviewer_window_store`/`plan_reviewer_snapshot_store`、三个 store 目录常量、`ActivePlanReviewWindow`、`WorkspaceSession.active_review_window`、`emit_plan_review_window_attention` 及 5 条文案、`reviewer_verify_policy` 的窗口 `Some` 分支、`review_launch_target_and_cwd` 的 `:628-633`、`resolve_issue_baseline_tree` 的 `:53-73`、`role_run_window_id` 与窗口 store

## 八、不要做

- 不做多成员对账域、集合屏障、报告成员分节、根 guard、物理 no-exec、ZeroWrite 分档。
- 不改 coding 侧任何文件；不改 `coding_workspace_engine` 的窗口/对账。
- 不改 `cross_cutting/streaming_provider` 的 `BaselineTreeRef` 为多值；不改 `ReviewInvocationScope`。
- **不新建 store/账本/窗口类型/前端组件**（身份 UUID 只用会话记录上的一个字段承载）。
- **不改任何单仓可达代码的行为**（含 prompt 字节、事件、门控、窗口记录、合同 digest）；不把 `is_aggregate_planning_session` 当形态判据。
- 不让 WorkItem splitter 一次吃多仓。
- 不改条目 9/10 文案；本轮不碰 `structured_output` 的 repair 门槛（待办 1）；15｜author 与 reviewer 允许同 provider 仅记边界、不改代码（两形态同样适用）。

## 九、验收

**单仓（`logical_codebase_id: None`）逐项一致**
1. 窗口与事件：`reviewer-windows/windows/` 下新增记录，window id（`{session}-{run}-w01`）与 generation 序列不变；`plan_review_window_*` 错误码路径可达，无新事件/文案。
2. 基线：`resolve_issue_baseline_tree` 仍 fail-closed（分支删/无分支 → 终止该轮），仍走 main→master 猜测。
3. 门控：`assert_plan_review_window` 6 处仍在并走原体；`reviewer_verify_policy` 仍返回窗口 id（canonical digest 不变）。
4. prompt：workspace context 与 reviewer prompt 字节一致（无成员清单、无逐仓实况、无 `detail_findings` 行）；`[artifact_boundary_must_fix_rules]` 文案逐字不变。
5. splitter 摘要：单仓 prompt 中仍为 `truncate(30)` + `(empty repository)` 原文。
6. **硬证据**：`drive.rs:2063,2174,2373` 的 `None` 夹具窗口用例、`drive_tests.rs` 窗口生命周期用例**不改一行且全绿**。

**多仓（`Some(_)`）新逻辑**
1. issue 选 2 仓 → story/design/work item 各跑通 author→reviewer→verdict。
2. reviewer prompt 每轮含 2 仓完整清单 + 逐仓实况（含 `基线:` 行）；无 `primary` 字样。
3. 无审查窗口记录、无 `primary-member anchored` 事件、无 `plan_review_window_*` 错误码。
4. 身份：同会话跨轮 canonical digest 的 `reviewer-verify-window:` 值恒定且为 UUID；author `resource_scope_id` 为 `role_run_author_<同一 uuid>`。
5. 超预算 → `planning_material_over_budget`；集合解析失败 → `plan_review_material_unresolved`；某仓 HEAD 不可识别 → 该行 `未识别(<诊断>)`，其余继续。
6. 缺 `evidence`/`required_action` → `needs_human` + `malformed_findings`；旧轮摘要含 message+evidence；splitter 空仓显式 unavailable。
7. 3 轮以上长会话，材料与清单仍完整可见。

## 十、已知缺陷 · 待办（本轮不做，逐条留证）

1. **verdict 结构错误静默降级（原条目 12）**——现象：schema 类结构化输出错误（`ReviewCompletionError::Schema(_)`）`recoverable_value()` 为 `None`（`review/structured_output.rs:43-51`），故 `is_repairable()` 恒 `false`（`:63-82`），不进一次性 repair，直接 `fallback_review_verdict` → `needs_human`（`:266-286`）；用户只看到"需要人工确认"，真实原因是可自愈的格式错误，诊断码埋在 `structured_output_diagnostic` 里。位置：`review/drive.rs:167`（direct 镜像）、`:510`（gateway 镜像）、`provider_allows_review_repair`（`:1417`）、静默重试收尾 `structured_output.rs:156-208`。**为何先不改**：要动 repair 门槛与四家 provider 的重试语义，风险高于本轮；且条目 11 落地后能先量出该错误在 lc 的真实频率。
2. **单仓 severity / revision 文案的判定缺陷仍在**——条目 9/10 决定不改，单仓（及 lc）继续把部分内容问题压成 `suggestion`、revision 轮继续只看旧问题。这是 owner 已知并接受的取舍；若后续要修，须按 `is_logical_codebase_issue()` 分两套文案并同步单仓 prompt 快照。
3. **`AggregateIndexRenderer` 的裁剪渲染无生产调用者**——`logical_codebase/aggregate_index/freshness.rs:214/284/328`（`trimmed_profile_count`/`omitted_member_count`）在本批核查中未找到生产调用链，但仍是死预算逻辑；待确认后清理（不影响本批行为）。

## 十一、待 owner 拍板：**无（清零）**

v1.1 遗留 3 项已全部被 owner 裁定消解：①`ReviewerVerify` 两种引用共存 → 裁定 1（lc 用会话 UUID，单仓用窗口 id，同一函数内 `match`）；②条目 9/10 是否对单仓生效 → 裁定 2/3（都**不改**，两形态一致保留现状）；③lc 逐仓基线失败语义 → 裁定 9（该仓标「基线未识别」，其余仓继续，不整轮终止）。
