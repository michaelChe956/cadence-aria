# 计划文档 · 实施计划 · WIG 基线树事实注入与前轮 finding 状态复算

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 WIG 作者拿到 plan 基线树的事实、让评审知道前轮 finding 在本轮是否仍存在，从而消除「越界重驱 → 重复报错 → 升级人工介入」这条失败链。

**Architecture:** 三个独立可回退的增量。① **D5 事后**：编译器生成 `acceptance_path_not_in_baseline` finding 时，把基线树中同目录/同名的最近现存路径附进 message（落点已在 `baseline_tree` 作用域内，零新增装载）。② **D5 事前**：作者轮启动前装载**校验器同源**的 `plan_baseline_tree`，作为 L1 的扩展注入 `[confirmed_context]` 的 `baseline:` 段（走上下文面，不动固定规范面预算常量）。③ **D6**：评审 prompt 组装时，用**当前候选**的机械报告对前轮 findings 做**内容身份求交** → 同身份去重 → 分「待办 / 已关闭」两区注入，把「是否已解决」的判定主体从模型移到平台。

**Tech Stack:** Rust 2021（crate `cadence-aria`）、cargo test、OpenSpec 变更校验（`openspec validate --strict`）。

**Spec:**
- `openspec/changes/rework-prompt-input-surface/specs/work-item-plan-single-candidate/spec.md`（MODIFIED `REQ-WSC-06`）
- `openspec/changes/rework-prompt-input-surface/design.md`（D5 / D6 决策）
- `openspec/changes/rework-prompt-input-surface/tasks.md`（工作包 5 / 6 / 7）
- 证据：`cadence/analysis-docs/2026-11-09_分析报告_WIG产品流程实测与根因锁定_v1.0.md`

**工作目录：** `.worktrees/feat-wig-optimize-11-08`（分支 `feat-wig-optimize-11-08`）

---

## Global Constraints

- 本 change 是 **REQ-WSC-06 的唯一 owner**，**只 MODIFIED 这一条 requirement**；不得触碰其它 active change 的 requirement。
- 排序约束（`design.md`「共享面与排序」）：`mechanicalize`（序 1）→ `isolate`（序 5）→ `close`（序 6）→ **本 change（序 7，最后）**。本计划三个任务全部落在本 change 内部，**不改变全局次序**。
- **不得改** `ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION`（`src/product/work_item_plan_compiler/types.rs:121`）的逐字文本。
- **不得改** `mechanicalize` 的四类 typed IR 诊断资格清单与机械化判定口径。
- **不得改** `close` 的有效必关闭判定、severity 呈现与 owner 排除口径。
- **不得上调** `WORK_ITEM_PLAN_MARKDOWN_PROMPT_QUALITY_BUDGET_BYTES`（23,500）与 `WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES`（32,000）两个常量。
- **不得**引入第二套基线树来源：一切基线树事实 MUST 来自 `crate::product::workspace_engine::plan_preflight::plan_baseline_tree`。
- **不得**由平台臆造路径：注入或回灌的每个候选路径 MUST ∈ 基线树。
- 求交键 MUST 为**内容身份**（`contract_field` vs 机械报告 `code`），MUST NOT 为 `FindingFingerprint`（其派生含 `message`，机械报告 message 与评审 message 不同 ⇒ 按指纹求交恒不相交）。
- 数字纪律：本计划引用的实测数字（32,609 字符首轮 prompt、43 路径 / 1,774 B 基线树、上下文面占用 24,146 B、preflight 失败 2/4、需人工介入 2/4、墙钟中位数 520.8s / 788.8s）**全部是基线对照值**，一律写成「实施后须复测」，**不得**写成已达成结论。
- 单仓路径义务：`summarize_repository_structure` 单仓分支逐字不动（REQ-PLN-17 既有场景）。

---

## Review Focus

以下五类是规格隐含、但任何单一任务的测试都不会覆盖，最可能咬人的地方。每条在 owning task 内补一条测试。

1. **基线树装载失败（`Err`）**：合理预期是**作者轮仍能启动并产出候选**（既有 fail-closed 点在 Evaluate 阶段），而不是启动即失败。归 Task 2。
2. **机械报告 ref 缺失或不可读**：合理预期是**如实降级**（不 panic、不静默当作「全部已解决」）。静默降级会把前轮 finding 全判为已关闭，从而放过真实未收敛项。归 Task 3。
3. **同目录无候选但同名有候选**：合理预期是回灌**同名**候选而不是空列表；空列表会让教学重驱再次盲改。归 Task 1。
4. **前轮 finding 无 `contract_field`**：合理预期是归入**不可机械判定**一类（既不进待办也不进已关闭），而不是被当作已解决。归 Task 3。
5. **大仓基线树（数千路径）**：合理预期是上下文面**不超 32,000 B** 且**成员仓全部在场**（不能因截断把某个成员仓整体挤掉）。归 Task 2。

---

## Task 1: D5 事后 · 越界报错回灌基线树最近候选路径

**Files:**
- Create: `src/product/work_item_plan_compiler/baseline_candidates.rs`（新文件，一个职责：给定缺失路径与基线树，返回最近候选路径）
- Modify: `src/product/work_item_plan_compiler/mod.rs`（挂 `mod baseline_candidates;`）
- Modify: `src/product/work_item_plan_compiler/validate.rs:70-95`（`acceptance_path_not_in_baseline` finding 的 message 追加候选）
- Test: `src/product/work_item_plan_compiler/tests/preflight_baseline_paths.rs`

**Interfaces:**
- Consumes: `context.baseline_tree: Option<&BTreeSet<String>>`（`PlanCandidateValidationContext` 既有字段，无需新增）
- Produces: `pub(crate) fn nearest_baseline_candidates(missing: &str, baseline_tree: &BTreeSet<String>) -> Vec<&str>` —— 返回**同父目录前缀**的路径；无则返回**同名（basename 相等）**的路径；仍无则返回空 `Vec`。顺序为 `BTreeSet` 字典序。返回值**全部 ∈ `baseline_tree`**。

- [ ] **Step 1: 写失败测试**

在 `src/product/work_item_plan_compiler/tests/preflight_baseline_paths.rs` 追加两条测试（沿用该文件既有的 `compile_candidate` 夹具与调用形态）：

```rust
#[test]
fn acceptance_path_finding_lists_nearest_baseline_candidates() {
    // 夹具：基线树含 "tests/integration/notice-delete-e2e.mjs" 与 "tests/unit/notice.test.mjs"
    // 候选计划引用 "tests/e2e/notice-delete-e2e.mjs"（不在基线树内）
    // 断言：finding message 含 "tests/integration/notice-delete-e2e.mjs"
    //      且含 "tests/unit/notice.test.mjs"
    //      且 message 中出现的每个候选路径都 ∈ 基线树
}

#[test]
fn acceptance_path_finding_reports_no_candidate_when_none_matches() {
    // 夹具：基线树只含 "src/main/App.vue"；计划引用 "scripts/run.mjs"
    // 断言：message 含"无可用候选"字样，且不含任何不在基线树内的路径
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --release -p cadence-aria acceptance_path_finding_ -- --nocapture`
Expected: FAIL（断言缺失候选路径 / message 未含候选字样），非编译错误。

- [ ] **Step 3: 实现 `nearest_baseline_candidates`**

在 `src/product/work_item_plan_compiler/baseline_candidates.rs` 新建：

```rust
pub(crate) fn nearest_baseline_candidates(missing: &str, baseline_tree: &BTreeSet<String>) -> Vec<&str>
```

口径（实现者只写这三步，不做额外相似度算法）：
1. 取 `missing` 的父目录前缀（`rsplit_once('/')` 的目录部分；无 `/` 则无目录前缀）。
2. 若有目录前缀 ⇒ 收集 `baseline_tree` 中以该前缀 + `/` 开头的路径。
3. 若结果为空 ⇒ 收集 `baseline_tree` 中 basename 与 `missing` 的 basename 相等的路径。

- [ ] **Step 4: 在 finding message 追加候选**

`src/product/work_item_plan_compiler/validate.rs` 的 `acceptance_path_not_in_baseline` 分支：保持既有 message 前缀与 `ACCEPTANCE_PATH_NOT_IN_BASELINE_REPAIR_ACTION` **逐字不动**，在其后追加：

- 有候选：`；基线树中同目录最近可用路径：{candidates.join("、")}`
- 无候选：`；基线树中无同目录或同名可用路径`

对 `missing` 中的**每个**路径各取一次候选并合并去重。

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test --release -p cadence-aria acceptance_path_finding_ -- --nocapture`
Expected: PASS。

- [ ] **Step 6: 跑既有基线路径测试确认无回归**

Run: `cargo test --release -p cadence-aria preflight_baseline_paths -- --nocapture`
Expected: 既有 5 条测试（`f56_route_reference_outside_baseline_is_blocked_with_repair_advice`、`baseline_covered_references_do_not_produce_findings`、`non_path_text_is_not_extracted`、`trusted_command_path_outside_baseline_is_blocked`、`quoted_span_reference_outside_baseline_is_blocked`）全绿。

- [ ] **Step 7: 提交**

```bash
git add src/product/work_item_plan_compiler/baseline_candidates.rs \
        src/product/work_item_plan_compiler/mod.rs \
        src/product/work_item_plan_compiler/validate.rs \
        src/product/work_item_plan_compiler/tests/preflight_baseline_paths.rs
git commit -m "feat(plan-compiler): 越界报错回灌基线树最近候选路径"
```

---

## Task 2: D5 事前 · 基线树清单直供（L1 扩展，走上下文面）

**Files:**
- Modify: `src/product/work_item_split_engine/prompts.rs`（`WorkItemPlanMarkdownAuthorContext` 新增字段 + `[confirmed_context]` 注入 `baseline:` 段）
- Modify: `src/product/work_item_split_engine/prompts/sc_revision.rs`（返修轮携带同一份清单）
- Modify: `src/web/workspace_ws_handler/run/single_candidate.rs`（作者轮前装载基线树，三处 context 构造点 `:1010` / `:1032` / `:1041` 附近）
- Test: `src/product/work_item_split_engine/tests/prompt_contract.rs`

**Interfaces:**
- Consumes: `crate::product::workspace_engine::plan_preflight::plan_baseline_tree(&lifecycle, &project_id, &issue_id) -> Result<Option<BTreeSet<String>>, String>`（既有 `pub(crate)` 函数，**不改其语义**）
- Produces: `WorkItemPlanMarkdownAuthorContext` 新增字段
  ```rust
  /// plan 基线树可用路径清单（按成员仓分组的渲染文本）。
  /// `None` = 基线树不可得（无仓 issue）⇒ 该段缺席。
  pub baseline_tree: Option<&'a str>,
  ```
  `prompts.rs` 中新增渲染函数
  ```rust
  pub(crate) fn render_baseline_tree_section(tree: &BTreeSet<String>, member_dirs: &[String]) -> String
  ```
  —— 按 `member_dirs` 分组、**成员仓优先**排序；输出为每行 `成员目录: 路径`；超上限时按成员仓保底截断（见 Step 4 预算）。

- [ ] **Step 1: 写失败测试**

在 `src/product/work_item_split_engine/tests/prompt_contract.rs` 追加：

```rust
#[test]
fn markdown_author_prompt_injects_baseline_tree_section() {
    // 用既有 split_prompt_fixture()，context.baseline_tree = Some("busi: src/Main.java\nfrontend: src/App.vue")
    // 断言：prompt 含 "baseline:"；含 "src/App.vue"；含 "busi"
}

#[test]
fn markdown_author_prompt_omits_baseline_section_when_tree_absent() {
    // context.baseline_tree = None
    // 断言：prompt 不含 "baseline:"
}
```

同时在**同文件既有预算断言处**追加一条独立预算断言（不改既有断言）：

```rust
#[test]
fn markdown_author_prompt_with_baseline_tree_stays_within_context_budget() {
    // 构造 3 成员仓 × 500 路径的基线树
    // 断言：上下文面（[confirmed_context] 段）字节 <= WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES
    //      且成员仓三个名字全部在场（不得因截断把某个成员仓整体挤掉）
}
```

- [ ] **Step 2: 运行测试确认失败**

Run: `cargo test --release -p cadence-aria markdown_author_prompt_ -- --nocapture`
Expected: FAIL（`baseline_tree` 字段不存在 → 编译错误即为红；补上字段后转为断言失败）。

- [ ] **Step 3: 给 context 加字段并在 prompt 注入**

`src/product/work_item_split_engine/prompts.rs`：
1. `WorkItemPlanMarkdownAuthorContext` 新增 `pub baseline_tree: Option<&'a str>`。
2. `build_work_item_plan_markdown_prompt` 在 `[confirmed_context]` 段之后、`story_spec_ids:` 之前插入 `baseline:{baseline_tree}`（仅当 `Some` 时；`None` 时**整段缺席**，不留空标记）。
3. 新增 `render_baseline_tree_section(tree: &BTreeSet<String>, member_dirs: &[String]) -> String`：
   - 按 `member_dirs` 顺序输出每个成员仓分组；`member_dirs` 之外的路径归入 `(未归属)` 组并排最后。
   - 每组内路径按字典序。
   - 总量上限取 `WORK_ITEM_PLAN_MARKDOWN_CONTEXT_BUDGET_BYTES / 4`（8,000 B）作为**渲染上限**：超限时**按成员仓轮转保底**——每轮每个成员仓各取 1 条，直到用满上限，保证**每个成员仓至少有 1 条**（Review Focus 第 5 条）。

- [ ] **Step 4: 装载点接线**

`src/web/workspace_ws_handler/run/single_candidate.rs`，函数 `preflight_lc_member_rules_before_author_run`（`:606`）内，三处 `WorkItemPlanMarkdownAuthorContext { ... }` 构造之前：

```rust
// 基线树装载（作者轮启动前）。与 Evaluate 期（single_candidate.rs:591）同函数同来源。
let baseline_tree_text = match crate::product::workspace_engine::plan_preflight::plan_baseline_tree(
    &lifecycle,
    &project_id,
    &issue_id,
) {
    Ok(Some(tree)) => Some(render_baseline_tree_section(&tree, &member_dirs)),
    Ok(None) => None,          // 无仓 issue ⇒ 该段缺席，零回归
    Err(error) => {
        // 装载失败不阻断作者轮：记诊断，既有 fail-closed 点仍在 Evaluate 阶段
        eprintln!("[aria] plan baseline tree unavailable for author prompt: {error}");
        None
    }
};
```

`member_dirs` 取既有成员目录上下文（与 D3 成员仓摘要**同一来源**，不得另造第二套成员目录口径）。

- [ ] **Step 5: 返修轮携带同一份清单**

`src/product/work_item_split_engine/prompts/sc_revision.rs` 的两个函数（`:31` / `:67`）复用同一 `context.baseline_tree`；确认返修轮 prompt 中 `baseline:` 段在场。

- [ ] **Step 6: 运行测试确认通过**

Run: `cargo test --release -p cadence-aria markdown_author_prompt_ -- --nocapture`
Expected: PASS（含既有 `work_item_plan_markdown_prompt_inlines_grammar_boundaries_and_real_findings` 的预算断言仍绿 —— **固定规范面预算常量未上调、未变**）。

- [ ] **Step 7: 跑既有 prompt 契约测试确认无回归**

Run: `cargo test --release -p cadence-aria prompt_contract -- --nocapture`
Expected: 全绿；既有预算断言打印的 `SC author prompt bytes=... margin=...` 与改造前一致（基线树不进固定面）。

- [ ] **Step 8: 提交**

```bash
git add src/product/work_item_split_engine/prompts.rs \
        src/product/work_item_split_engine/prompts/sc_revision.rs \
        src/web/workspace_ws_handler/run/single_candidate.rs \
        src/product/work_item_split_engine/tests/prompt_contract.rs
git commit -m "feat(sc-author): 基线树可用路径清单直供 author prompt（L1 扩展）"
```

---

## Task 3: D6 · 前轮 finding 状态：内容身份求交 + 去重 + 分区注入

**Files:**
- Modify: `src/product/workspace_engine/prompts/review_context.rs:522-586`（`single_candidate_previous_round_findings_section`）
- Modify: `src/product/workspace_engine/tests/single_candidate_prompt/mod.rs:101-141`（夹具 `persist_verification_artifacts` 支持注入 findings）
- Test: `src/product/workspace_engine/tests/single_candidate_prompt/review_prompt.rs`（既有 `single_candidate_review_prompt_injects_previous_round_finding_identities` 扩展 + 新增）
- Test: `src/product/workspace_engine/tests/review_disposition_platform.rs`（既有 `previous_round_section_omits_owner_disposed_findings` 扩展）

**Interfaces:**
- Consumes: `engine.session.mechanical_report_ref: Option<String>`（Evaluate 阶段落盘）；机械报告的 `report.findings: Vec<WorkItemSplitFinding>`（`code` 字段）
- Produces: `src/product/workspace_engine/prompts/review_context.rs` 内新增
  ```rust
  /// 前轮 finding 的**内容身份**：与机械报告 finding 的 `code` 可比。
  /// 形如 "plan_preflight.intent_unexecutable"；无可比身份时返回 None。
  pub(crate) fn content_identity(contract_field: Option<&str>) -> Option<String>;
  ```
  以及清单载荷结构：`{ "unresolved_findings": [...], "resolved_findings": [...], "duplicate_count": N }`

- [ ] **Step 1: 扩展夹具支持 findings**

`src/product/workspace_engine/tests/single_candidate_prompt/mod.rs` 的 `persist_verification_artifacts` 增加一个带 findings 的变体（**不改动既有签名**，保持既有调用点不变）：

```rust
pub(super) fn persist_verification_artifacts_with_findings(
    lifecycle: &LifecycleStore,
    plan_id: &str,
    findings: Vec<WorkItemSplitFinding>,
) -> (String, String)
```

既有 `persist_verification_artifacts` 改为调用它并传空 `Vec`。

- [ ] **Step 2: 写失败测试**

在 `src/product/workspace_engine/tests/single_candidate_prompt/review_prompt.rs` 追加四条测试（沿用既有 `single_candidate_review_prompt_injects_previous_round_finding_identities` 的夹具搭建形态：造一个 `ReviewerRun` 节点 + durable verdict）：

```rust
#[test]
fn previous_round_section_marks_unresolved_finding_as_pending() {
    // 前轮 finding: contract_field = "plan_preflight.intent_unexecutable"
    // 当前机械报告 findings: code = "intent_unexecutable"
    // 断言：prompt 含 "unresolved_findings"；该 finding 落在待办区；含 required_action
}

#[test]
fn previous_round_section_marks_resolved_finding_as_closed() {
    // 前轮 finding: contract_field = "plan_preflight.intent_unexecutable"
    // 当前机械报告 findings: 空（已修复）
    // 断言：prompt 含 "resolved_findings"；该 finding 落在已关闭区；该区不含 required_action
}

#[test]
fn previous_round_section_deduplicates_same_content_identity() {
    // 前轮 4 条同 contract_field 的 finding
    // 断言：待办区该身份只出现 1 条；且出现重复计数
}

#[test]
fn previous_round_section_resolves_by_content_identity_not_fingerprint() {
    // 前轮 finding message = "[intent_unexecutable] xxx"；机械报告 message = "xxx"（两者不同）
    // 断言：仍被正确判为「已关闭」（证明按 content identity 求交，而非按含 message 的指纹）
}
```

- [ ] **Step 3: 运行测试确认失败**

Run: `cargo test --release -p cadence-aria previous_round_section_ -- --nocapture`
Expected: FAIL（prompt 中不存在 `unresolved_findings` / `resolved_findings` 分区）。

- [ ] **Step 4: 实现求交与分区**

`src/product/workspace_engine/prompts/review_context.rs` 的 `single_candidate_previous_round_findings_section`：

1. **装载当前机械报告**：`engine.session.mechanical_report_ref` → `None` 或装载失败 ⇒ **如实降级**：清单按「不可机械判定」处理（既不进待办也不进已关闭），**不得**当作「全部已解决」。装载失败**不 panic**，降级为不注入分区标记并在 payload 中标注 `mechanical_report_available: false`。
2. **内容身份**：前轮 finding 的 `contract_field` 形如 `plan_preflight.intent_unexecutable` ⇒ 身份为去掉 `plan_preflight.` 前缀后的 `intent_unexecutable`；机械报告 finding 的 `code` 即该身份。`contract_field` 缺失或无 `plan_preflight.` 前缀 ⇒ 该 finding **不可机械判定**（既不进待办也不进已关闭）。
3. **求交**：当前机械报告的 `code` 集合 ∩ 前轮身份集合。
4. **去重**：同一身份只保留首条，其余折叠为计数 `duplicate_count`。
5. **分区注入**：
   - `unresolved_findings`：身份 ∈ 机械报告 `code` 集合 ⇒ 附 `fingerprint` / `category` / `contract_field` / `required_action` / `content_identity`。
   - `resolved_findings`：身份 ∉ 机械报告 `code` 集合（且身份可判定）⇒ 附 `fingerprint` / `category` / `contract_field` / `content_identity` / `resolved_by`（当前候选 revision 标识），**不含 `required_action`**。
6. **纪律段改写**：把现有 `[previous_round_repetition_discipline]` 改为以分区事实为准的表述：已关闭区条目**不得**再作为待办重复上报；判定 SHALL NOT 由模型依据清单文本自行推断。

- [ ] **Step 5: 运行测试确认通过**

Run: `cargo test --release -p cadence-aria previous_round_section_ -- --nocapture`
Expected: PASS。

- [ ] **Step 6: 跑既有评审测试确认无回归**

Run: `cargo test --release -p cadence-aria review_disposition_platform review_prompt -- --nocapture`
Expected: 全绿（既有 `previous_round_section_omits_owner_disposed_findings` 仍绿 —— owner 排除口径未被本改动触碰）。

- [ ] **Step 7: 提交**

```bash
git add src/product/workspace_engine/prompts/review_context.rs \
        src/product/workspace_engine/tests/single_candidate_prompt/mod.rs \
        src/product/workspace_engine/tests/single_candidate_prompt/review_prompt.rs \
        src/product/workspace_engine/tests/review_disposition_platform.rs
git commit -m "feat(review): 前轮 findings 按内容身份求交去重并分区注入"
```

---

## Task 4: 验收复测（同装置 n≥4/臂）

**Files:**
- Create: `cadence/analysis-docs/wig-product-flow-2026-11-09/harvest_serial.py`（**已在基线轮落盘**，本任务只复用，不新建）
- Create: `cadence/analysis-docs/2026-11-09_分析报告_WIG基线树事实注入与前轮状态复算_复测_v1.0.md`（复测报告）

**Interfaces:**
- Consumes: 基线轮的装置脚本与收割器（`cadence/analysis-docs/wig-product-flow-2026-11-09/`）
- Produces: 复测报告，登记 preflight 失败率、需人工介入率、重写轮次、墙钟、usage 五项与基线并列

- [ ] **Step 1: 构建被测二进制**

Run: `cargo build --release -p cadence-aria`
Expected: 成功。**记录二进制来源树**（cargo `.fingerprint/cadence-aria-*` 路径哈希可用于区分来源树），并确认它与 `feat-wig-optimize-11-08` 的 WIG 关键路径**零差异**（`git diff` 对 `work_item_split_engine/`、`workspace_engine/`、`work_item_plan_compiler/`、`work_item_contract/`、`work_item_plan_policy/`）。

- [ ] **Step 2: 按基线装置执行 n≥4/臂串行交替复测**

复用 `cadence/analysis-docs/wig-product-flow-2026-11-09/reproduce.md` 的装置：独立工作区副本 + 独立端口 + **同时刻只跑一个**；单臂流程 `prepare` → **等 `terminated_at` 落盘** → `PUT automation-enrollment`（**必须带 `command_id`**）→ 等 plan 终态 → 停 daemon。
**唯一变量**：实施前 vs 实施后（档位固定不变）。
Expected: 每臂 ≥4 run，全部跑完或明确记录停在人工门。

- [ ] **Step 3: 收割并对照**

Run: `python3 cadence/analysis-docs/wig-product-flow-2026-11-09/harvest_serial.py`
Expected: 输出逐 run 明细 + 对照表。

- [ ] **Step 4: 写复测报告**

报告须并列登记（基线 → 复测）：preflight 失败率（基线 2/4）、需人工介入率（基线 2/4）、重写轮次（基线中位数 0.5）、墙钟（中位数 xhigh 520.8s / high 788.8s）、usage（input 中位数 101,281 / 63,221）。
**不得**把未复现的收益写成因果结论；若样本不足（n<4/臂），如实写明置信不足。

- [ ] **Step 5: OpenSpec 严格校验**

Run: `openspec validate rework-prompt-input-surface --strict`
Expected: exit 0。

- [ ] **Step 6: 描述段压缩回 ≤500 字符**

MODIFIED `REQ-WSC-06` 描述段当前 705 字符（归档门禁 ≤500）。压缩时 MUST NOT 削弱「基线树事实直供」与「前轮 finding 状态由平台机械复算」两项义务的 SHALL 强度。
Run: `python3 -c "<读 spec.md 描述段并打印长度>"`
Expected: ≤500。

- [ ] **Step 7: 提交**

```bash
git add cadence/analysis-docs/2026-11-09_分析报告_WIG基线树事实注入与前轮状态复算_复测_v1.0.md \
        cadence/analysis-docs/wig-product-flow-2026-11-09/
git commit -m "docs(analysis): 基线树事实注入与前轮状态复算的同装置复测报告"
```

---

## 附：与基线轮的对应关系（便于验收对账）

| 基线实测项 | 基线值 | 本计划的预期作用点 |
|---|---|---|
| `acceptance_path_not_in_baseline` 越界 | high-4323 命中，2/4 run | Task 1（回灌候选）+ Task 2（事前给事实） |
| `intent_unexecutable` 越界 | xhigh-4324 命中 | Task 2（事前给事实）+ Task 3（避免重复报） |
| 重驱工具调用 0 次 | 26.4s / 152.4s | Task 1（给具体修改对象） |
| 前轮清单 4 条同 fingerprint | xhigh-4324 | Task 3 Step 4.4（去重） |
| 升级 `human_required` | 2/4 run | Task 3（分区后不再重复报已解决项） |
