# tolerate-explained-empty-open-items 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans（或 subagent-driven-development）按任务逐项执行。步骤使用 checkbox（`- [ ]`）跟踪。

**Goal:** 让 Story artifact「待确认项」中"空标记 + 解释"的合法表述不再被校验器误杀，同时真未解决开放问题仍被拦截。

**Architecture:** 仅改 `artifact_constraints.rs` 的 `open_item_line_is_resolved` 空标记分支：硬 cue 与上游推导白名单分支保持不变，软 cue 不再单独拒绝已声明空标记的行；并给空标记加句读边界判定。Story `open_item_policy_hint` 同步收紧文案。

**Tech Stack:** Rust（workspace 内 crate `aria`，单测在 `src/product/workspace_engine/tests/part_10.rs` / `part_31.rs`）。

**Spec:** `openspec/changes/tolerate-explained-empty-open-items/`（proposal/specs/design/tasks）。

## Global Constraints

- Rust 标准命令：`cargo test -p aria --lib <filter>`、`cargo fmt`、`cargo clippy --workspace --all-targets`、`cargo test --workspace`；🔴 禁止 `-j 1`。
- 不改重试协议、reviewer must_fix 规则、其他 workspace 类型的待确认项行为（该启发式仅作用于 Story）。
- 既有 part_10.rs 38 个单测全部保持通过（含所有上游推导白名单负例）。

---

### Task 1: 空标记 + 解释容错（TDD）

**Files:**
- Modify: `src/product/workspace_engine/artifact_constraints.rs`（`open_item_line_is_resolved`、`open_item_empty_marker_raw_remainder`）
- Test: `src/product/workspace_engine/tests/part_10.rs`

**Interfaces:**
- Consumes: `open_item_empty_marker_raw_remainder(line) -> Option<String>`（既有）、`open_item_line_has_hard_unresolved_cue(line) -> bool`（既有）、`open_item_remainder_claims_upstream_derivation` 分支（既有，不动）。
- Produces: `open_item_line_is_resolved` 行为变更（无导出新符号）。

- [ ] **Step 1: 写失败单测**（加入 part_10.rs，复用既有 fixture 结构）

```rust
#[test]
fn story_artifact_accepts_empty_marker_with_benign_explanation() {
    let report = validate_workspace_artifact_constraints(
        "# Aria Provider Setup Story Spec\n\n\
         ## 范围\n**来源**：Issue `issue_0001` — provider 安装引导。\n\n\
         ## 用户故事\n作为用户，我要完成 provider 安装。\n\n\
         ## 功能需求\n- [REQ-001] 系统支持 provider 检查。\n\n\
         ## 成功标准\n- [AC-001] 用户能看到 provider 状态。\n\n\
         ## 待确认项\n无待确认项。Issue 已明确需求、格式规则、验收示例与范围限制；单元测试运行器选型、库文件与页面文件的具体路径、模块文件扩展名（如 `.mjs`）等属实现细节，留待 Design 阶段决策，不构成本 Story 的未决问题。\n\n\
         ## 非功能需求\n无。\n",
        &WorkspaceType::Story,
    );

    assert!(
        report.passed,
        "empty marker with benign explanation should not be treated as an open item: {:?}",
        report.blocking_reasons()
    );
}
```

- [ ] **Step 2: 运行确认失败**：`cargo test -p aria --lib story_artifact_accepts_empty_marker_with_benign_explanation`，期望 FAIL（禁止内容含「待确认项未通过 AskUserQuestion 交互解决」）。

- [ ] **Step 3: 写负例/边界单测**（同文件）

```rust
#[test]
fn story_artifact_rejects_open_item_disguised_as_wu_prefix() {
    // 「无论…」不得被单字「无」前缀判为空标记开头
    let report = validate_workspace_artifact_constraints(
        // …同上 fixture，待确认项正文换成：
        "## 待确认项\n无论使用哪种单元测试运行器，都需要确认其选型。\n",
        // 断言 !report.passed，blocking_reasons 含 待确认项+AskUserQuestion
    );
}
```

（完整 fixture 复制 Step 1 结构，仅替换待确认项正文与断言为 `assert!(!report.passed)`。）

- [ ] **Step 4: 实现**（artifact_constraints.rs）
  - `open_item_empty_marker_raw_remainder`：命中 marker 前缀后，要求 remainder 为空或首字符属于边界集合 `['。','.','，',',','；',';','：',':','！','!','？','?','（','(',' ','\\t']`，否则返回 `None`。
  - `open_item_line_is_resolved` 空标记分支：保持「推导白名单分支」与「硬 cue → false」不变，最终回退由 `!has_unresolved_cue || has_resolved_cue` 改为 `true`。
- [ ] **Step 5: 运行**：`cargo test -p aria --lib workspace_engine`（或 `--lib story_artifact`），期望全绿（新旧 38+2 例）。
- [ ] **Step 6: 提交**：`git add -A && git commit -m "fix(workspace): tolerate benign explanations after empty open-item marker"`

### Task 2: Story 待确认项策略提示收紧

**Files:**
- Modify: `src/product/workspace_engine/artifact_constraints.rs`（Story `open_item_policy_hint` 文案）
- Test: `src/product/workspace_engine/tests/part_31.rs`

- [ ] **Step 1: 写失败断言**：在 `story_schema_contract_exposes_open_item_resolution_protocol` 中新增 `assert!(schema.contains("不得附加解释"))`。
- [ ] **Step 2: 运行确认失败**：`cargo test -p aria --lib story_schema_contract_exposes_open_item_resolution_protocol`，期望 FAIL。
- [ ] **Step 3: 修改 hint 文案**：在 Story `open_item_policy_hint` 的「若无开放问题，写『无待确认项』」后追加「；只写『无待确认项』四个字，不得附加解释，解释性内容写入其他章节」。
- [ ] **Step 4: 运行确认通过**：同 Step 2 命令，期望 PASS。
- [ ] **Step 5: 提交**：`git commit -am "feat(workspace): tighten story open-item empty marker guidance"`

### Task 3: 整体验证

- [ ] `cargo fmt`
- [ ] `cargo clippy --workspace --all-targets`
- [ ] `cargo test --workspace`，全绿后读取输出作为新鲜证据汇报。
