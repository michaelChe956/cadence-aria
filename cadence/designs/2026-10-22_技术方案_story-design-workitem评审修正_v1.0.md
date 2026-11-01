# 技术方案：story / design / work item 三环节评审修正（v1.0）

> 性质：方案。**不含实现代码**。
> 标准依据：`cadence/designs/2026-10-22_技术方案_系统环节与角色标准_v1.0.md`（5 环节；issue 选定真实仓库集合 1~N 无主次；三环节产出落 aria 规划目录 → 无零锁恢复；work item 单仓单 coder）。
> 理念（owner，最高优先级）：**先删后加、先通后优**。不为"可能情况"预埋机制；不引入新抽象层；每阶段结束必须能走完 story→design→work item。
> 工作目录校验（2026-10-23 实测）：`pwd` = `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0808-add-monorepo`；`git branch --show-current` = `feat-b-0808-add-monorepo`。✅ 符合铁律。

## 一、判定口径（先定，后文不再重复论证）

1. **范围唯一权威 = issue 选定集合**，即 `RepositoryContextSet.set`（manifest ∩ active ∩ selection，`planning_context_set.rs:48-115`）。单值 `IssueRecord.repo_id`、`WorkspaceSession.repository_path` 降级为兼容字段，**不再参与任何范围判定**。
2. **三环节评审无窗**：不开窗、不建快照、不做基线对账、不做恢复。窗口机制的删除范围严格限定在 `workspace_engine`（coding 侧 `coding_workspace_engine` 原样保留）。
3. **材料只落两处**：`[canonical_inputs]`（成员清单 + 逐仓实况 + 逐仓基线 → 每一轮评审都可见）与 prompt 尾部指令块（只给 author，reviewer 侧被既有过滤器丢弃即正确）。
4. **失败一律 fail-closed 并带稳定错误码**；不猜、不静默降级、不省略成员。
5. **work item 每个 item 单仓**，但**材料**必须是集合全量（work item 阶段的"选哪个仓"是判定问题，不是范围收缩）。

## 二、阶段划分（3 阶段）

| 阶段 | 条目 | 阶段结束「可跑通」判定 |
|---|---|---|
| 一｜删窗·集合权威·材料稳定注入 | 1,2,3,4,6,15 | issue 选 2 仓 → story / design / work item 各跑通 author→reviewer→verdict 一轮；reviewer prompt 含 2 仓完整清单；`reviewer-windows/windows/` 无新增；全树无 `primary` 残留 |
| 二｜逐仓实况与判定 | 5,16,8,9,10 | 材料含每仓实况与基线行；漏项/选错仓/验收不可判定夹具 → reviewer 必须出 `must_fix`（非 `suggestion`） |
| 三｜容错与解析打磨 | 11,12,13,14 | 各条负例夹具通过；三者仍可走通全流程（改的是失败面与摘要面） |

## 三、阶段一（打通）

### 1｜删除「主仓 / 主成员」概念
- 落点：`workspace_engine/review/drive.rs:44-60`、`:386-400`（调用点）、`:1690-1701`（回落）、`:1998-2020`（事件）；`workspace_repository.rs:285-318`；`review/drive_tests.rs:390-400,700-900`
- 改法：`plan_review_window_worktree` 整体删除（窗口没了就没有"目标解析"）；`resolve_primary_member_repository` 与其 `logical_repository_primary_member` 错误 kind 删除；`emit_primary_member_anchor_note` 删除；`prompts.rs:759/812/866` 的"禁止回落到任意单一 primary 仓库"改为"禁止回落到任意单一仓库"（保留反指令语义，去掉 `primary` 词）。
- 验收：`grep -rn "primary" src/product/workspace_engine src/product/workspace_repository.rs` 零命中（注释/文案/标识符全清）；drive_tests 中 3 个 primary 锚定用例删除。
- 风险：低。纯删除；唯一行为变化是"聚合根形态不再硬失败"，因为不再需要窗口目标。

### 2｜单值字段降级为兼容
- 落点：`models/project.rs:138`（`repo_id`）、`workspace_engine/types.rs:119`（`repository_path`）、`prompts/review_context.rs:618-634`
- 改法：`review_launch_target_and_cwd` 的 gateway 分支不再要求 `repository_path` 必须是成员 worktree——聚合规划会话（`is_aggregate_planning_session()`）直接返回 `(authority_root, Some(authority_root))`；单仓 legacy 分支不动。字段本身保留（存量记录反序列化），加注释标注"兼容字段，非范围权威"。
- 验收：聚合会话 reviewer 的 `working_dir` = `working_directory` = LC 根；单仓 issue 行为字节不变（既有 review 用例全绿）。
- 风险：低-中。若某 provider 依赖 target≠cwd 才能读成员仓，聚合会话会失去该能力——但这正是本批要删的"单仓对账视野"，且评审材料已改为集合全量注入。

### 3｜删除三环节评审的窗口/基线/对账/恢复（含两处替代）
- 落点：`review/drive.rs` 调用点 `:62,72,128,141,196,204,326,334,394,403,466,482,539,550,670,680`；定义 `:1441-1450`（`ActivePlanReviewWindow`）、`:1582-1584`（两个 store dir 常量）、`:1600-1670`（`plan_reviewer_window_store`/`plan_reviewer_snapshot_store`/`plan_review_main_scope`）、`:1670-1685`（`plan_review_launch_ownership`）、`:1752-1814`（`begin_plan_review_window`）、`:1816-1840`（`assert_plan_review_window`）、`:1842-1856`（`reviewer_verify_policy`）、`:1857-1955`（`close_plan_review_window`）、`:1958-1997`（`emit_plan_review_window_attention`）、`:2100-2470`（窗口内联测试）；`types.rs:922-924`（`active_review_window` 字段）；`review.rs:10`（re-export）；`lifecycle.rs:339,625`（字段初始化）；`prompts/review.rs`、`review_repair.rs`、`review_parts/*.inc.rs` 的 8 处 `reviewer_verify_policy()` 调用保留调用形态。
- 改法：入口 `provider_run.rs:355-365` 不变（仍调 `drive_review_session(_via_gateway)`），但两个外层函数体退化为"直接调 `*_with_window` 内层并改名去 `_with_window` 后缀"（去掉开窗/收口两行）。
- **替代①（材料采集前置校验）**：新增 `fn assert_plan_review_material(&self) -> Result<(), String>`——聚合规划会话走 `PlanningContextSetResolver::resolve`（store 读，无索引/无 FS 遍历）+ 材料渲染，集合为空或渲染失败 → `plan_review_material_unresolved: <cause>`。在 `build_review_input()` 之前与 repair 之前各调一次（与旧 `assert_plan_review_window` 的两个位置 1:1 对应）。单仓会话直接 `Ok(())`。
- **替代②（provider 启动的受信归属）**：`reviewer_verify_policy()` 改为从**会话 id** 铸造 `ProviderToolPolicy::reviewer_verify(self.session.session_id.clone())`（语义不变：Reviewer 全开、无 deny；`validate_tool_policy_for_role` 只校验非空，`projection.rs` 只用它做 digest 域）；`plan_review_launch_ownership()` 删除，两处 `start_owned`/`start_review_session_via_gateway_owned` 传 `None`（已核：`drive.rs:1498-1500` 的 `match` 与 Claude `start_owned` 均容忍 `None`）。
- 验收：跑一轮聚合 story 评审后 `app_paths.root()/reviewer-windows/windows/` 无新文件；无 `plan_review_window_*` 错误码；无 `primary-member anchored` 事件；四家 provider 的 Reviewer 启动守卫仍通过（`streaming_provider/tests.rs:900-1030` 的守卫矩阵需就近改为"session 引用"）。
- 风险：中。`pub(crate)` 面与测试面牵连 5 个文件（`part_32.rs:259-300`、`tool_policy_audit_wiring.rs:246`、`single_candidate_lc_admission.rs:2499`、`drive_tests.rs` 多处）。**这是本批唯一的大改**，建议单独一次提交，便于回滚。

### 4｜仓库清单全量注入，超预算显式阻断
- 落点：`planning_context_set.rs:160-300`（`InventoryInjectionBudget` / `render_compact_inventory` / `render_inventory_lines`）、`logical_codebase/mod.rs:103`、`planning_context_resolver_tests.inc.rs:700-730`
- 改法：删掉 soft/hard 两档裁剪与 `omitted_member_count` / `omitted_member_ids` / `truncated`，`render_compact_inventory` 改为**全量渲染 + 绝对上限**：材料块总字节 > 256 KiB → `Err(ProductStoreError::InvalidRecord{kind:"planning_material_over_budget"})`。绝不省略成员。
- 验收：构造 N=3 与超大 alias 的成员夹具 → 全量命中；构造超 256 KiB 夹具 → 明确错误码且评审不启动。
- 风险：低。删的是裁剪分支；`render_compact_inventory` 签名由 `Result<InventoryInjection, _>` 保持（仍返回 Result），调用方 `planning_context_resolver.rs:472` 无需改。

### 6｜材料改稳定注入（reviewer 每轮可见）
- 落点：`web/workspace_context/builder.rs:236-281`、`workspace_engine/prompts.rs:747-880`、`prompts/history_compaction.rs:317-328`
- 改法：把 `aggregate_*_scope_prompt` 拆成两半——`aggregate_member_material_block(inventory, per_repo)`（**材料**：`## 聚合代码库成员清单` + 逐仓实况）插入 `[canonical_inputs]` 段内（`Repository 路径` 之后、`关联上下文` 之前）；`aggregate_*_scope_prompt(...)` 只保留**指令**部分并仍追加到消息尾部。必须保留 `AGGREGATE_SCOPE_MARKER`（`## 聚合代码库成员清单`）字符串——`is_aggregate_planning_session()` 依赖它。材料块内不得出现 `\n\n[`（会截断 `generation_brief_section` 的段解析）。
- 改法（续）：`must_replay_raw` 的 `has_canonical_inputs` 条件去掉 `mode == Reviewer` 限制——author 轮次压缩同样不吞该消息。
- 验收：跑 3 轮以上评审，reviewer prompt 的 `会话上下文` 中仍完整出现成员清单与逐仓实况（`reviewer_context_filter.rs:8-32` 不改，靠材料在 `canonical_inputs` 内自然命中）；author 修订轮同样可见。
- 风险：中。`builder.rs` 的 `format!` 大字符串插入位置错会破坏 `[constraint_summary]` 段界 → 用现有 `part_31.rs:840-1070` 的 prompt 断言夹具兜住。

### 15｜author 与 reviewer 允许同 provider（仅记边界）
- 落点：本文 §十。**不改代码**。
- 验收：文档记录即完成。
- 风险：无。

## 四、阶段二（逐仓材料与判定）

### 5｜补逐仓实况（职责 / 关键目录 / 入口 / 构建测试命令）
- 落点：**新增** `src/product/logical_codebase/member_reality.rs`；接线 `planning_context_set.rs`（`resolve` 后按 `RepositoryContextSet` 逐个渲染）与 `prompts.rs` 的 `aggregate_member_material_block`
- 改法：每仓渲染固定 4 行：`职责`（取 `RepositoryContextSet.role`，空 → `未识别`）、`关键目录`（分层摘要，≤12 项）、`入口`（根下 `Cargo.toml/package.json/go.mod/pom.xml/Makefile` 等存在即列，≤6 项）、`构建/测试命令`（从上述清单可判定则给 `cargo test` / `npm test` / `mvn test`，否则 `未识别`）。单仓预算 ≤2 KiB，**不递归分析代码**。
- 验收：2 仓夹具（一 Rust 一 Vue）→ 材料中出现两段实况，字段齐全、缺项显 `未识别`；采样进 prompt 后 reviewer 能引用（用 `[canonical_inputs]` 断言）。
- 风险：低-中。额外 FS 读取仅限成员仓根目录（`read_dir` 一层，与 item 14 共用分层实现），不进入评审热路径的多次重复——每次 prompt 构建只遍历一次。

### 16｜聚合会话按需逐仓解析基线
- 落点：`prompts/baseline_teaching.rs:44-80`（`resolve_issue_baseline_tree`）
- 改法：删掉 `is_aggregate_planning_session() → Ok(None)` 的短路。聚合会话改为**逐仓**调用 `resolve_effective_base_branch(checkout_path, issue.base_branch.as_deref())`，结果写入 item 5 的逐仓实况行 `基线: <branch>` / `基线: 未识别(<诊断>)`；**不引入存储迁移、不改 `BaselineTreeRef`、不碰 `cross_cutting`**（`baseline_tree` 字段对聚合会话仍为 `None`，硬边界不假装存在）。单仓会话原 fail-closed 语义完全不动。
- 验收：2 仓夹具 → 材料含两行各自基线；某成员去掉目标分支 → 该行 `未识别(<诊断>)`，评审仍启动。
- 风险：低。语义分叉（单仓 fail-closed / 聚合标注降级）已记入 §十 待拍板。

### 8｜补实质 must-fix 类别
- 落点：`workspace_engine/artifact_constraints.rs:142`（Story）、`:176`（Design）、`:208`（WorkItem）、`:238`（WorkItemPlan）
- 改法：每类在既有"越界"规则后追加 4 条固定文案（纯文本，经 `reviewer_boundary_rules_for` 渲染进 `[artifact_boundary_must_fix_rules]`）：① 需求/范围漏项；② 仓库选择不合理（列了却不涉及 / 涉及却未列）；③ 验收标准不可判定（无证据、无判据）；④ 跨仓影响缺失（涉及多仓却未写影响/顺序）。WorkItem/WorkItemPlan 额外保留"只能一个真实仓库"的边界规则。
- 验收：`part_02/artifact_constraints_and_review_input.rs:321-360` 的 marker 断言扩展为逐条文案断言；人工夹具（漏写一个 involved 仓）→ reviewer 输出 `must_fix`。
- 风险：低。纯文案；注意别把"测试计划/实现步骤"类 forbidden 内容误允许（Design 规则已含此禁令，追加不动它）。

### 9｜severity 不再系统性压低内容问题
- 落点：`prompts/review.rs:201-202`（Story/Design/WorkItem 通用面）、`:403-404`（WorkItemPlan outline 面）——两处同一契约文案
- 改法：把"只有影响下一阶段可用性才能 must_fix"改为**正面清单**：需求漏项、仓库选择错误、验收不可判定、跨仓影响缺失、与上游 spec 冲突 → `blocking|must_fix`；纯风格/措辞/美化/未来扩展才 `suggestion`。
- 验收：同一份"漏项"夹具改前只在 suggestion 出现、改后命中 must_fix（用既有 reviewer prompt 快照对比）。
- 风险：低。可能提高返修率，属预期。

### 10｜revision 轮允许带证据提新实质问题
- 落点：同上 `prompts/review.rs:205`、`:407`
- 改法："只复核上一轮强返修项"改为"复核上一轮强返修项**并**允许在给出具体证据（引用当前产物的段落/ID）时提出新的实质问题；仍禁止无证据的发散式风格建议"。
- 验收：第二轮夹具（上一轮已关，但产物新引入一个不可判定验收）→ 出 `must_fix`；纯风格夹具 → 仍 `suggestion` 或不出。
- 风险：低。

## 五、阶段三（容错与解析）

### 11｜finding 缺 `evidence`/`required_action` 不默认空串
- 落点：`workspace_engine/parsers.rs:785-880`（`parse_review_findings` 宽松入口），`:855-866`
- 改法：`evidence`/`required_action` 缺失或空串 → `malformed = true` 且该条**丢弃**（走既有 `malformed` 汇总路径），不再 `unwrap_or("")`。严格入口 `parse_raw_review_finding`（`:611-700`）本就允许可选，保持不变。
- 验收：缺字段夹具 → 该 finding 不进 `findings`、`malformed=true` 可见；既有 `parsers.rs:970-1180` 用例补齐。
- 风险：低-中。丢弃会让本可用的 must_fix 消失 → 与 item 12 的"重试一次"配合使用。

### 12｜verdict 结构错误给诊断码并可自动重试一次
- 落点：`review/drive.rs` 首次 completion 的 `match`（direct `:150-200` 与 gateway `:495-560` 两个镜像）、`review/structured_output.rs:265-290`（`fallback_review_verdict`）
- 改法：`ReviewCompletionError::Schema` 不再只对 Kimi 可修复——把 repair 门槛从 `provider_allows_review_repair(&reviewer) && is_repairable()` 放宽为 `is_repairable() || matches!(Schema)`，四条 provider 各重试一次；`is_classification_fatal()` 分支严格不动（仍直落 fatal）。重试后仍失败 → 既有 `fallback_review_verdict`（`structured_output_diagnostic.code` 已带明确码）。
- 验收：喂结构错误的 verdict → 观察 `structured_output_repair_event` 一次 + 第二次成功走正常 verdict；两次失败 → `needs_human` 且 diagnostic code 非空。
- 风险：中。多烧一次 provider 调用；Pi 的不重试策略被改动（`provider_allows_review_repair` 的既有语义需在注释里显式说明这次扩展）。

### 13｜旧轮摘要保留 `message`/`evidence`
- 落点：`prompts/history_compaction.rs:279-292`（`render_round_summary`）、`:344-385`（`FindingSummary` + `extract_findings`）
- 改法：`FindingSummary` 增 `message`/`evidence` 字段；渲染行补 `message:` 与 `evidence:`；解析器对应补两个前缀（`message:`/`evidence:`），缺省 `(not recorded)`。
- 验收：`history_compaction.rs:620-700` 的 `four_round_history` 夹具加断言：压缩摘要含原始 `message` 与 `evidence` 文本。
- 风险：低。摘要变长 → 与 item 4 的预算规则无关（摘要走既有字符预算），但需复跑压缩预算断言。

### 14｜WorkItem splitter 目录摘要分层 + 失败显式
- 落点：`work_item_split_engine/context.rs:147-172`（`summarize_repository_structure`），调用点 `prompts.rs:444,475,550,595`、`web/.../single_candidate.rs:915`
- 改法：改为**分层摘要**（顶层 ≤12 项 + 每项一级子目录 ≤6 项，总 ≤40 行），复用 item 5 的 `member_reality` 分层实现；`read_dir` 失败不再返回 `"(empty repository)"`，返回显式 `"repository_structure_unavailable: <io error>"`。
- 验收：空仓夹具 → 输出 `repository_structure_unavailable`（不再是"empty"）；多层仓夹具 → 出现二级条目。
- 风险：低。另有 `single_candidate.rs:915`（单候选 preflight）共用该函数，需一并确认调用方不解析其文本。

## 六、要删清单（不留死代码）

**代码路径**
- `drive.rs`：`plan_review_window_worktree`、`begin_plan_review_window`、`assert_plan_review_window`、`close_plan_review_window`、`plan_review_launch_ownership`、`plan_review_main_scope`、`plan_reviewer_window_store`、`plan_reviewer_snapshot_store`、`emit_plan_review_window_attention`、`ActivePlanReviewWindow`、`PLAN_REVIEWER_SNAPSHOT_STORE_DIR`、`PLAN_PROVIDER_RESOURCE_STORE_DIR`、`PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED`、`drive_review_session_with_window` / `drive_review_session_via_gateway_with_window`（改名去后缀）、全部窗口内联测试（`:2100-2470`）
- `workspace_repository.rs`：`resolve_primary_member_repository`、`logical_repository_primary_member` 错误 kind
- `planning_context_set.rs`：`InventoryInjectionBudget`、soft/hard 裁剪分支、`omitted_member_count` 渲染、`InventoryInjection.omitted_member_ids`/`truncated`
- `types.rs`：`WorkspaceSession.active_review_window` 字段；`review.rs:10` re-export；`lifecycle.rs:339,625` 初始化
- `work_item_split_engine/context.rs`：30 项 `entries.truncate(30)` 与 `"(empty repository)"` 文案

**事件 / 用户可见文案**
- `EngineEvent`"审查窗口主成员锚定(primary-member anchored;非主成员面未对账——R8 类已知边界…)"
- `plan_review_window_*` 全部错误码与 `emit_plan_review_window_attention` 的 5 条人工注意文案（含 `reviewer-windows/windows/<本窗>.json` 指引）

**`primary` 命名清单**（全树 grep 清零）
- `resolve_primary_member_repository`、`resolve_selected_logical_repository(..., primary, ...)` 的入参名与注释、`logical_repository_primary_member`、`manifest.member_ids.first()` 的"注册序=primary"注释、`primary_anchored` 变量、`emit_primary_member_anchor_note`、`prompts.rs:759/812/866` 的"任意单一 primary 仓库"、`drive_tests.rs:390-400,700-900` 的 primary 用例与注释

## 七、不要做

- 不做多成员对账域、集合屏障、报告成员分节、根 guard、物理 no-exec、ZeroWrite 分档。
- 不改 coding 侧任何文件（`coding_workspace_engine/*`、`code_review.rs`、`internal_pr_review.rs` 及其窗口/对账）。
- 不改 `cross_cutting/streaming_provider` 的 `BaselineTreeRef` 为多值（会让 coding 侧跟着变）。
- 不引入新存储、新账本、新窗口类型、新 schema 字段、新前端组件。
- 不改 `ReviewInvocationScope`（单候选协议，与窗口无关）。
- 不做 issue 模型多值化改造（集合表达复用现有 `IssueCodebaseSelection`）。
- 不改 `reviewer_context_filter` 的过滤集合（只靠把材料放进 `canonical_inputs` 解决可见性）。
- 不让 WorkItem splitter 一次吃多仓（见 §九，属独立项）。

## 八、风险与回滚点

| 风险 | 触发面 | 回滚点 |
|---|---|---|
| 删窗牵连 5 个测试文件 + `types.rs` 字段 | 阶段一（大提交） | 该提交独立可 revert；后续条目与之无耦合 |
| `builder.rs` 段界破坏（`\n\n[` 截断） | 阶段一 item 6 | `part_31.rs` 的 prompt 段断言；回滚仅需把材料块挪回尾部（退化为 `author` 可见、reviewer 不可见的上限） |
| 逐仓实况带来额外 FS 遍历 | 阶段二 item 5 | 实况渲染为纯函数、失败即 `未识别`，可单独关闭（返回空串 → 材料回退到清单） |
| Schema 重试翻倍 provider 调用 | 阶段三 item 12 | 门槛收紧回 Kimi-only 一行改动 |
| finding 缺字段被丢弃导致漏审 | 阶段三 item 11 | 降级为 `suggestion` 而非丢弃（一行改动） |

## 九、可后置打磨（先做通再谈）

1. `prompts/review.rs` 的 severity/revision 契约文案在 6 处重复（通用面 2 + outline 2 + batch/item 2），可抽成单一常量（当前按条分别改，避免动结构）。
2. WorkItem splitter 仍以**单仓**为输入（`build_generate_invocation(repository)`）——改为集合视图需另立一项（本批只修摘要质量）。
3. `ToolPolicyIntent::ReviewerVerify { window_id }` 的字段名已不代表窗口，可改 `contract_ref`（跨 4 家 adapter + coding 侧，收益小，本批不改名）。
4. `reviewer_verify_policy()` 的 8 处调用点可用一次 `build_*` 收口（结构性重构，非本批）。
5. 逐仓实况的"职责"目前取 `role` 字段；未来可从 README/包名派生，属打磨。
6. 材料 256 KiB 上限的具体数值，跑真实 LC 后按分布再定。

## 十、待 owner 拍板（3 项）

1. **逐仓基线失败时的语义**：本方案取"该仓标 `未识别(<诊断>)`、评审继续"（不阻断）；单仓会话保持既有 fail-closed。若要求聚合会话也 fail-closed，则某成员缺目标分支即整轮失败。
2. **`ReviewerVerify` 的合同引用改用 `session_id`**（variant 名不变、不改 cross_cutting）：确认可接受"合同引用不再是窗口 id"。若不可接受，则必须先新增一个不依赖窗口的 policy intent，再删窗。
3. **工作项 15（author 与 reviewer 允许同 provider）**：本批仅记为已知边界，不写代码。如需在 UI/契约上提示"同 provider 自审"风险，请明确是否纳入本批。
