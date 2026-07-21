# CodeReviewer 上下文修复与回退重审 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. 当前任务禁止未明确授权的多代理委派，因此不使用 subagent-driven-development。

**Goal:** 修复 CodeReviewer 的 Work Item 上下文、单 Unit diff 与 pre-review 证据生命周期，并将 `coding_attempt_0001` 回退到 Coder Run #14 完成后重新执行一次全新 Review。

**Architecture:** 提取产品层共享 Work Item 上下文加载器，供 Coder、CodeReviewer 与 EvaluationContextPack 使用；为单 Unit Code Review 选择 `attempt.head_commit` 作为 diff 基线，GroupFinalReview 继续使用 `main`；CodeReviewer 不再要求 approve 后才产生的 final handoff/completion commit。平台修复验证并推送后，以 rollback skill 的备份、清单和指纹规则执行窄范围 Run #17 回退，不重置 Unit、不清理业务 worktree。

**Tech Stack:** Rust 2024、Tokio、serde/serde_json、Axum WebSocket、Cargo 测试、Git、jq。

## Global Constraints

- 只在 `/home/michaelche/workspace/github/cadence-aria/.worktrees/feat-b-0709` 修改平台代码。
- 不修改 `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001` 中 Coder Run #14 的业务代码。
- 不运行 E2E、Playwright、浏览器自动化测试或浏览器环境安装命令。
- 允许单元测试、非浏览器集成测试、`cargo fmt --check`、Clippy、check 和 Rust 全量测试。
- 禁止给 Cargo 命令添加 `-j 1`。
- 保留 CodeReviewer 与 GroupFinalReview 已有非 E2E Prompt 边界。
- 保留 feat worktree 中与本任务无关的 `.superpowers/sdd/final-review-fix-report.md` 修改，不暂存、不覆盖。
- 回退前备份 Attempt 数据，并记录业务 diff、Coder output、handoff 与验证日志指纹。
- rollback-coding-attempt skill 只用于安全清单、边界核对、备份与一致性校验；不得执行该 skill 的 Unit 重置或 worktree 清理步骤，因为本次目标是同一 Unit 内回退 Review #17 并保留 Coder #14 的全部修改。

---

## File Map

- Create `src/product/coding_work_item_context.rs`: 统一加载正式 Work Item、Verification Plan、Source Draft Supplement 与 Workspace artifact fallback。
- Modify `src/product/mod.rs`: 导出共享上下文模块。
- Modify `src/web/coding_ws_handler/context.rs`: Coder 改为调用共享加载器，保留 WebSocket gate/provider 辅助逻辑。
- Modify `src/product/coding_workspace_engine/reports.rs`: Reviewer 改为调用共享 Work Item 上下文。
- Modify `src/product/coding_workspace_engine/prompts.rs`: 单 Unit Review 使用正确 diff 基线；补充 pre-review handoff/commit 生命周期协议。
- Modify `src/product/coding_workspace_engine/types.rs`: 增加缺失 Unit diff 基线的明确错误。
- Modify `src/product/coding_evaluation_context/builder.rs`: EvaluationContextPack 使用共享 Work Item markdown；CodeReviewer 不再产生 final handoff 缺失警告。
- Modify `src/product/coding_evaluation_context/specs.rs`: 支持 compiled Work Item markdown 作为 artifact version 缺失时的权威回退。
- Modify `src/product/coding_evaluation_context/repo.rs`: 支持调用方传入 diff base，避免固定使用 `main`。
- Modify `src/product/coding_evaluation_context/tester_execution.rs`: 保持 Tester 现有 base branch 行为并适配新签名。
- Modify `src/product/coding_workspace_engine/tests/parser_prompt.rs`: 覆盖正式 Work Item fallback、Unit diff 与 Reviewer 协议。
- Modify `src/product/coding_evaluation_context/tests.rs`: 覆盖 Work Item compiled context 与 pre-review evidence。
- Runtime data under `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001`: 只在平台验证完成后执行 Run #17 窄范围回退。

---

### Task 1: 提取共享 Work Item 上下文加载器

**Files:**
- Create: `src/product/coding_work_item_context.rs`
- Modify: `src/product/mod.rs`
- Modify: `src/web/coding_ws_handler/context.rs`
- Modify: `src/product/coding_workspace_engine/reports.rs`
- Test: `src/product/coding_workspace_engine/tests/parser_prompt.rs`

**Interfaces:**
- Consumes: `ProductAppPaths`、`CodingExecutionAttempt`、`LifecycleWorkItemRecord`、`VerificationPlan`。
- Produces:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CompiledCodingWorkItemContext {
    pub(crate) markdown: Option<String>,
    pub(crate) verification_commands: Vec<String>,
}

pub(crate) fn load_coding_work_item_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CompiledCodingWorkItemContext, ProductStoreError>;
```

- [ ] **Step 1: 写 Work Item 无 artifact version 的失败测试**

在 `parser_prompt.rs` 新增 `code_review_prompt_uses_compiled_work_item_without_artifact_version`：

```rust
#[tokio::test]
async fn code_review_prompt_uses_compiled_work_item_without_artifact_version() {
    // 创建正式 LifecycleWorkItemRecord 和 VerificationPlan，但不创建 artifact version。
    // planned_implementation_context 包含 "compiled implementation context"。
    // forbidden_write_scopes 包含 "tests/**"。
    // verification command 包含 "cargo test --locked --lib compiled_context"。
    let prompt = engine
        .build_code_review_prompt(&attempt, &worktree, None)
        .await
        .expect("code review prompt");

    assert!(prompt.contains("compiled implementation context"));
    assert!(prompt.contains("tests/**"));
    assert!(prompt.contains("cargo test --locked --lib compiled_context"));
    assert!(!prompt.contains("未找到 Work Item markdown"));
}
```

- [ ] **Step 2: 运行测试并确认 RED**

Run:

```bash
cargo test --locked --lib code_review_prompt_uses_compiled_work_item_without_artifact_version
```

Expected: FAIL，因为 `work_item_markdown_for_attempt` 只读取 artifact version。

- [ ] **Step 3: 创建共享加载器并移动现有编译逻辑**

把 `context.rs` 中以下职责移动到 `coding_work_item_context.rs`，保持行为不变：

- `CompiledWorkItemContext`
- `compiled_work_item_context`
- `needs_source_draft_supplement`
- `final_compile_draft_supplement`
- `workspace_artifact_work_item_markdown`
- `compiled_work_item_markdown`
- `verification_command_lines`
- `merge_work_item_markdown`
- `merge_verification_commands`
- 仅由上述函数使用的 markdown/list 辅助函数

共享入口实现必须返回正式编译 markdown，即使 Workspace 没有 artifact version：

```rust
pub(crate) fn load_coding_work_item_context(
    app_paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<CompiledCodingWorkItemContext, ProductStoreError> {
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let current_work_item_id = attempt
        .current_work_item_id
        .as_deref()
        .unwrap_or(&attempt.work_item_id);
    let compiled = compiled_work_item_context(
        &lifecycle,
        app_paths,
        attempt,
        current_work_item_id,
    )?;
    let workspace = if compiled.markdown.is_none()
        || compiled.needs_workspace_artifact_fallback
    {
        workspace_artifact_work_item_markdown(
            &lifecycle,
            attempt,
            current_work_item_id,
        )?
    } else {
        None
    };
    let markdown = merge_work_item_markdown(compiled.markdown, workspace);
    let verification_commands =
        merge_verification_commands(compiled.verification_commands, markdown.as_deref());

    Ok(CompiledCodingWorkItemContext {
        markdown,
        verification_commands,
    })
}
```

- [ ] **Step 4: Coder 与 Reviewer 接入共享加载器**

`coding_execution_context` 只做协议类型映射：

```rust
let context = load_coding_work_item_context(app_paths, attempt)?;
Ok(CodingExecutionContext {
    work_item_markdown: context.markdown,
    verification_commands: context.verification_commands,
})
```

`work_item_markdown_for_attempt` 改为：

```rust
Ok(load_coding_work_item_context(self.store.paths(), attempt)?.markdown)
```

- [ ] **Step 5: 运行定向测试并确认 GREEN**

```bash
cargo test --locked --lib code_review_prompt_uses_compiled_work_item_without_artifact_version
cargo test --locked --lib coding_execution_context
```

Expected: 新测试通过，既有 Coder context 测试全部通过。

- [ ] **Step 6: 提交共享上下文改动**

```bash
git add src/product/coding_work_item_context.rs src/product/mod.rs \
  src/web/coding_ws_handler/context.rs \
  src/product/coding_workspace_engine/reports.rs \
  src/product/coding_workspace_engine/tests/parser_prompt.rs
git commit -m "fix: unify coding work item context loading"
```

---

### Task 2: 让 EvaluationContextPack 使用正式 Work Item 内容

**Files:**
- Modify: `src/product/coding_evaluation_context/builder.rs`
- Modify: `src/product/coding_evaluation_context/specs.rs`
- Test: `src/product/coding_evaluation_context/tests.rs`

**Interfaces:**
- Consumes: `load_coding_work_item_context`。
- Produces: `EvaluationWorkItemContext.raw_markdown_or_sections` 在 artifact version 缺失时包含正式编译 Work Item。

- [ ] **Step 1: 写 EvaluationContextPack 失败测试**

新增 `evaluation_context_uses_compiled_work_item_without_artifact_version`：

```rust
#[test]
fn evaluation_context_uses_compiled_work_item_without_artifact_version() {
    // 创建正式 Work Item 和 Verification Plan，不创建 artifact version。
    let pack = build_evaluation_context_pack(
        paths,
        &attempt,
        EvaluationContextRole::CodeReviewer,
    )
    .expect("evaluation context");

    assert!(pack.work_item.raw_markdown_or_sections.contains("compiled implementation context"));
    assert!(pack.work_item.raw_markdown_or_sections.contains("tests/**"));
    assert!(pack.work_item.raw_markdown_or_sections.contains("cargo test --locked"));
}
```

- [ ] **Step 2: 运行测试并确认 RED**

```bash
cargo test --locked --lib evaluation_context_uses_compiled_work_item_without_artifact_version
```

Expected: FAIL，当前 `work_item_context` 在没有 artifact version 时写入空字符串。

- [ ] **Step 3: 扩展 work_item_context 输入**

签名调整为：

```rust
pub(super) fn work_item_context(
    work_item: &LifecycleWorkItemRecord,
    version: Option<&ArtifactVersion>,
    compiled_markdown: Option<&str>,
    session: Option<&WorkspaceSessionRecord>,
    warnings: &mut Vec<String>,
) -> EvaluationWorkItemContext;
```

内容优先级：artifact version 非空时保留 artifact snapshot；否则使用 `compiled_markdown`；两者均无值才为空。

Builder 调用：

```rust
let compiled = load_coding_work_item_context(&lifecycle_paths, attempt)?;
let work_item_context = work_item_context(
    &work_item,
    work_item_version.as_ref(),
    compiled.markdown.as_deref(),
    work_item_session,
    &mut context_warnings,
);
```

- [ ] **Step 4: 运行 EvaluationContextPack 测试**

```bash
cargo test --locked --lib evaluation_context_uses_compiled_work_item_without_artifact_version
cargo test --locked --lib coding_evaluation_context
```

Expected: 新旧上下文测试全部通过。

- [ ] **Step 5: 提交 EvaluationContext 改动**

```bash
git add src/product/coding_evaluation_context/builder.rs \
  src/product/coding_evaluation_context/specs.rs \
  src/product/coding_evaluation_context/tests.rs
git commit -m "fix: include compiled work item in reviewer context"
```

---

### Task 3: 单 Unit Code Review 使用 head_commit diff 基线

**Files:**
- Modify: `src/product/coding_workspace_engine/prompts.rs`
- Modify: `src/product/coding_workspace_engine/types.rs`
- Modify: `src/product/coding_evaluation_context/builder.rs`
- Modify: `src/product/coding_evaluation_context/repo.rs`
- Modify: `src/product/coding_evaluation_context/tester_execution.rs`
- Test: `src/product/coding_workspace_engine/tests/parser_prompt.rs`
- Test: `src/product/coding_evaluation_context/tests.rs`

**Interfaces:**
- Produces:

```rust
pub(crate) fn code_review_diff_base(
    attempt: &CodingExecutionAttempt,
) -> Result<&str, CodingWorkspaceEngineError>;
```

- [ ] **Step 1: 写 Unit diff 基线失败测试**

新增 `group_code_review_uses_previous_unit_head_commit_as_diff_base`：

```rust
#[tokio::test]
async fn group_code_review_uses_previous_unit_head_commit_as_diff_base() {
    // commit A: main baseline
    // commit B: previous unit adds previous_unit.txt
    // working tree: current unit adds current_unit.txt
    attempt.scope = CodingAttemptScope::WorkItemGroup;
    attempt.base_branch = first_commit;
    attempt.head_commit = Some(previous_unit_commit);

    let prompt = engine
        .build_code_review_prompt(&attempt, &worktree, None)
        .await
        .expect("code review prompt");

    assert!(prompt.contains("current_unit.txt"));
    assert!(!prompt.contains("previous_unit.txt"));
}
```

并新增 `group_code_review_rejects_missing_head_commit`，期望 `CompletionCommitMissing`。

- [ ] **Step 2: 运行测试并确认 RED**

```bash
cargo test --locked --lib group_code_review_uses_previous_unit_head_commit_as_diff_base
cargo test --locked --lib group_code_review_rejects_missing_head_commit
```

Expected: 第一条因为使用 base branch 而包含 previous unit；第二条不会报明确错误。

- [ ] **Step 3: 实现 Code Review diff base**

```rust
pub(crate) fn code_review_diff_base(
    attempt: &CodingExecutionAttempt,
) -> Result<&str, CodingWorkspaceEngineError> {
    if attempt.scope == CodingAttemptScope::WorkItemGroup {
        return attempt.head_commit.as_deref().ok_or_else(|| {
            CodingWorkspaceEngineError::CompletionCommitMissing(attempt.id.clone())
        });
    }
    Ok(&attempt.base_branch)
}
```

`build_code_review_prompt` 使用该 ref 调用 `git_diff`。`build_internal_pr_review_prompt` 不变，继续使用 `attempt.base_branch`。

- [ ] **Step 4: Evaluation repo_context 接收显式 diff base**

`repo_context` 新签名：

```rust
pub(super) fn repo_context(
    attempt: &CodingExecutionAttempt,
    work_item: Option<&LifecycleWorkItemRecord>,
    diff_base: Option<&str>,
    warnings: &mut Vec<String>,
) -> EvaluationRepoContext;
```

CodeReviewer + WorkItemGroup 使用 `attempt.head_commit.as_deref()`；InternalReviewer、Coder、Tester 保持 `attempt.base_branch`。若 CodeReviewer Group 缺少 head commit，加入 `code_review_diff_base_missing` 并返回空 diff context，不能回退到 main。

- [ ] **Step 5: 写 repo_context 一致性测试**

新增 `code_reviewer_repo_context_uses_group_head_commit`，断言 `changed_files` 只包含 current Unit 文件；新增 `internal_reviewer_repo_context_keeps_full_branch_diff`，断言完整 diff 仍包含 previous Unit。

- [ ] **Step 6: 运行 diff 相关测试**

```bash
cargo test --locked --lib group_code_review_uses_previous_unit_head_commit_as_diff_base
cargo test --locked --lib group_code_review_rejects_missing_head_commit
cargo test --locked --lib code_reviewer_repo_context_uses_group_head_commit
cargo test --locked --lib internal_reviewer_repo_context_keeps_full_branch_diff
```

Expected: 全部通过。

- [ ] **Step 7: 提交 diff 基线改动**

```bash
git add src/product/coding_workspace_engine/prompts.rs \
  src/product/coding_workspace_engine/types.rs \
  src/product/coding_evaluation_context/builder.rs \
  src/product/coding_evaluation_context/repo.rs \
  src/product/coding_evaluation_context/tester_execution.rs \
  src/product/coding_workspace_engine/tests/parser_prompt.rs \
  src/product/coding_evaluation_context/tests.rs
git commit -m "fix: scope code review diff to active unit"
```

---

### Task 4: 修正 pre-review handoff 与 commit 证据语义

**Files:**
- Modify: `src/product/coding_evaluation_context/builder.rs`
- Modify: `src/product/coding_workspace_engine/prompts.rs`
- Test: `src/product/coding_evaluation_context/tests.rs`
- Test: `src/product/coding_workspace_engine/tests/parser_prompt.rs`

**Interfaces:**
- CodeReviewer 不再收到 `work_item_handoff_missing`。
- InternalReviewer 在正式 handoff 缺失时仍收到 `work_item_handoff_missing`。

- [ ] **Step 1: 写角色差异失败测试**

```rust
#[test]
fn code_reviewer_does_not_require_final_handoff_before_approval() {
    let pack = build_evaluation_context_pack(
        paths.clone(),
        &attempt,
        EvaluationContextRole::CodeReviewer,
    )
    .expect("code reviewer pack");
    assert!(!pack.coder_evidence.unwrap().evidence_warnings.contains(
        &"work_item_handoff_missing".to_string(),
    ));

    let final_pack = build_evaluation_context_pack(
        paths,
        &attempt,
        EvaluationContextRole::InternalReviewer,
    )
    .expect("internal reviewer pack");
    assert!(final_pack.coder_evidence.unwrap().evidence_warnings.contains(
        &"work_item_handoff_missing".to_string(),
    ));
}
```

- [ ] **Step 2: 运行测试并确认 RED**

```bash
cargo test --locked --lib code_reviewer_does_not_require_final_handoff_before_approval
```

Expected: FAIL，因为两种 Reviewer 当前都会收到 warning。

- [ ] **Step 3: 限定 handoff warning 角色**

```rust
if handoff.is_none()
    && matches!(provider_role, EvaluationContextRole::InternalReviewer)
{
    evidence_warnings.push("work_item_handoff_missing".to_string());
}
```

CodeReviewer 仍保留 completion report、raw output refs 与可见 handoff（若存在），只是缺失时不警告。

- [ ] **Step 4: 加强 CodeReviewer Prompt 生命周期协议**

在 `code_review_material_protocol()` 中加入：

```text
- WorkItemGroup 当前 Unit 的 completion commit 与平台 final unit handoff 在 Code Review approve 后才生成；Code Review 前为空是正常状态，不得据此创建 finding、request_changes 或 blocked。
- Code Review 阶段应以 Coder completion report、raw/artifact refs、实际测试输出和当前 Unit diff 判断验证证据；真正缺失或自相矛盾的 required verification evidence 仍必须记录。
```

GroupFinalReview 协议保持 completed units 必须有 handoff/commit。

- [ ] **Step 5: 运行协议与证据测试**

```bash
cargo test --locked --lib code_reviewer_does_not_require_final_handoff_before_approval
cargo test --locked --lib code_review_material_protocol
cargo test --locked --lib group_final_review_material_protocol_requires_group_handoff_checks
```

Expected: 全部通过。

- [ ] **Step 6: 提交证据语义改动**

```bash
git add src/product/coding_evaluation_context/builder.rs \
  src/product/coding_workspace_engine/prompts.rs \
  src/product/coding_evaluation_context/tests.rs \
  src/product/coding_workspace_engine/tests/parser_prompt.rs
git commit -m "fix: align reviewer evidence with unit lifecycle"
```

---

### Task 5: 平台全量验证、提交和推送

**Files:**
- Verify all files modified in Tasks 1–4.

- [ ] **Step 1: 运行格式检查**

```bash
cargo fmt --check
```

Expected: exit 0。

- [ ] **Step 2: 运行严格 Clippy**

```bash
cargo clippy --all-targets --all-features --locked -- -D warnings
```

Expected: exit 0，无 warning。

- [ ] **Step 3: 运行编译检查**

```bash
cargo check --locked
```

Expected: exit 0。

- [ ] **Step 4: 运行 Rust 全量测试**

```bash
cargo test --locked
```

Expected: exit 0，0 failed。不得运行任何 E2E/Playwright 命令。

- [ ] **Step 5: 检查 Prompt 边界和 diff**

```bash
rg -n "Reviewer 非 E2E 测试边界|WorkItemGroup 当前 Unit" \
  src/product/coding_workspace_engine/prompts.rs
git diff --check
git status --short
```

Expected: 非 E2E 协议仍存在；只有任务文件和预先存在的无关修改。

- [ ] **Step 6: 提交剩余格式或测试调整**

若 Tasks 1–4 已分别提交且无剩余任务改动，跳过；否则只暂存本任务文件：

```bash
git add src/product src/web/coding_ws_handler/context.rs
git commit -m "test: cover reviewer context recovery"
```

- [ ] **Step 7: 推送 feat-b-0709**

```bash
git push origin feat-b-0709
```

Expected: 远端 `origin/feat-b-0709` 指向本轮最终提交。

---

### Task 6: 备份并窄范围回退 Code Reviewer Run #17

**Files:**
- Runtime data under `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001`.
- Business worktree is read-only for this task.

**Interfaces:**
- Preserves: `coding_role_run_0030`、`coding_node_0031`、Coder output 0014、13-file business diff、handoff 和 verification logs。
- Removes from active history: Reviewer Run #17 / `coding_role_run_0031` and its derived records。
- Produces: 一个明确标记为控制用途的 `retry_review` blocked gate。

- [ ] **Step 1: 确认没有活跃 Runner/Provider**

```bash
ps -eo pid,etime,cmd | rg 'aria|claude|codex' | rg -v 'rg '
jq '{status,stage,updated_at}' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json
jq -c 'select(.status=="running")' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/role-runs/*.json
```

Expected: Attempt 为稳定的 `waiting_for_human/code_review`，无当前 running role run。

- [ ] **Step 2: 记录业务代码与 Coder 证据指纹**

在业务 worktree 执行：

```bash
git rev-parse HEAD
git status --short
git diff --stat
git diff --binary | sha256sum
sha256sum \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/units/coding_unit_0006/work-item-handoff.json \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/units/coding_unit_0006/verification/*.log
```

Expected: HEAD=`640d63d78a316275b42e5bcab6969d7588e13d19`，13 个修改文件；保存所有哈希用于回退后比对。

- [ ] **Step 3: 备份完整 Attempt 数据**

```bash
BACKUP="/tmp/cadence-aria-coding_attempt_0001-before-review17-rollback-$(date -u +%Y%m%dT%H%M%SZ)"
mkdir -p "$BACKUP"
cp -a \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001 \
  "$BACKUP"/
printf '%s\n' "$BACKUP"
```

Expected: 备份目录包含 attempt 顶层 JSON 和完整明细目录。

- [ ] **Step 4: 目视核对精确回退清单**

必须只包含：

```text
timeline node: coding_node_0032
role run: coding_role_run_0031
role events: coding_role_run_0031.jsonl
role artifacts: coding_role_run_0031/
chat entry: coding_node_0032_code_review_report.json
code review: code_review_0015.json
provider raw: code_review_0015.txt
blocked gate: coding_blocked_gate_0010.json（先删除 Run #17 内容，随后重建控制 gate）
provider conversation: last_node_id == coding_node_0032
```

不得删除 `coding_context_note_0005`、`coding_rework_instruction_0008`、Run #16 或 Coder Run #14。

- [ ] **Step 5: 使用 apply_patch 与精确文件删除执行回退**

- `timeline-nodes.json` 删除且仅删除 `coding_node_0032`。
- 删除上一步列出的 Run #17 文件和目录。
- Attempt 顶层设置：

```json
{
  "status": "blocked",
  "stage": "code_review",
  "rework_count": 3,
  "current_work_item_id": "work_item_compile_20260712024139064_006",
  "active_unit_id": "coding_unit_0006",
  "head_commit": "640d63d78a316275b42e5bcab6969d7588e13d19",
  "provider_conversations": [
    {
      "role": "coder",
      "provider": "codex",
      "provider_session_id": "019f5c1b-a738-7862-a3c9-598c3640fa2a",
      "last_node_id": "coding_node_0031"
    }
  ]
}
```

保留 Coder conversation 的原始 `updated_at`。Attempt `updated_at` 使用控制 gate 创建时间，不伪造为历史业务时间。

- [ ] **Step 6: 创建明确的 retry_review 控制 gate**

重建 `coding_blocked_gate_0010.json`，内容必须明确它是回退后的控制 gate，不是业务 Review：

```json
{
  "gate": {
    "gate_id": "coding_blocked_gate_0010",
    "kind": "blocked",
    "title": "重新执行代码审查",
    "description": "平台 Reviewer 上下文已修复；当前保留 Coder Run #14 完成状态，等待启动全新 Code Review。",
    "stage": "code_review",
    "role": "code_reviewer",
    "available_actions": [
      {
        "action_id": "retry_review",
        "label": "重新执行代码审查",
        "action_type": "retry_review"
      },
      {
        "action_id": "abort",
        "label": "终止",
        "action_type": "abort"
      }
    ],
    "reason_code": "code_review_retry_after_platform_fix"
  },
  "attempt_id": "coding_attempt_0001",
  "node_id": null,
  "status": "open"
}
```

时间字段使用实际 UTC 时间。

- [ ] **Step 7: 执行数据与业务指纹校验**

```bash
jq empty .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json
jq empty .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/timeline-nodes.json
jq '.[-1] | {id,status,title}' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/timeline-nodes.json
test ! -e .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/role-runs/coding_role_run_0031.json
```

Expected: timeline 最后节点为 `coding_node_0031` completed；Run #17 文件不存在；控制 gate open。

重新计算业务 worktree `git diff --binary | sha256sum` 与 handoff/log hashes，必须与 Step 2 完全一致。

---

### Task 7: 启动全新 Reviewer 并验证三层证据

**Files:**
- No source edits expected.
- New runtime records should reuse the rolled-back sequential slot for the new Review run.

- [ ] **Step 1: 确认新后端已加载平台修复**

```bash
curl -fsS http://127.0.0.1:4317/api/health
```

并检查运行二进制/日志来自 `feat-b-0709` 最新提交。若 cargo-watch 未自动重编，只重启后端，不需要重启前端。

- [ ] **Step 2: 通过现有 WebSocket 协议提交 retry_review**

连接：

```text
ws://127.0.0.1:4317/ws/coding-attempts/coding_attempt_0001
```

发送：

```json
{"type":"coding_hello","attempt_id":"coding_attempt_0001","last_seen_node_id":"coding_node_0031"}
{"type":"gate_response","gate_id":"coding_blocked_gate_0010","action_id":"retry_review","extra_context":null}
```

保持 WebSocket 连接，直到新的 Code Reviewer Role Run 完成或进入明确 blocked 状态。该控制动作不是 E2E 测试，不启动浏览器或 Playwright。

- [ ] **Step 3: 检查新 Prompt**

新 Prompt 必须满足：

```bash
rg -n "Final Compile Work Item|Forbidden Write Scopes|cargo test --locked|Reviewer 非 E2E" \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/artifacts/role-run-events/coding_role_run_0031/0001_prompt.txt
rg -n "未找到 Work Item markdown|work_item_handoff_missing" \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/artifacts/role-run-events/coding_role_run_0031/0001_prompt.txt
```

Expected: 第一条全部命中；第二条不命中 Reviewer 否决上下文。Prompt diff 中不得出现已提交 Unit 1–5 的累计变更。

- [ ] **Step 4: 检查全新 Provider session**

```bash
jq -c 'select(.event_type=="message_complete") | .payload.provider_session_id' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/role-run-events/coding_role_run_0031.jsonl
```

Expected: session ID 不等于已回退的 `29f21c07-793e-4c7c-ba24-b3396ba6c4e6`。

- [ ] **Step 5: 对比 Provider raw、Review 和 Role Run**

```bash
jq '{run_no,status,reason_code,node_id,raw_provider_output_refs}' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/role-runs/coding_role_run_0031.json
jq '{run_no,verdict,summary,findings}' \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/code-reviews/code_review_0015.json
```

确认 raw output、持久化 Review 与 Role Run 状态一致，区分：

- `approve`：真实通过。
- `request_changes`：真实业务 finding。
- `blocked`：真实依赖/人工澄清阻塞。
- Provider/解析失败：必须有对应 reason_code 和原始输出证据。

- [ ] **Step 6: 最终业务代码指纹复核**

业务 worktree 再次执行：

```bash
git rev-parse HEAD
git status --short
git diff --binary | sha256sum
```

Expected: 与回退前完全一致，Reviewer 未修改业务代码。

---

## Completion Report

最终汇报必须包含：

- 平台修改文件与提交 SHA。
- 定向测试、fmt、clippy、check、全量 Rust 测试结果。
- 明确说明未运行任何 E2E/Playwright。
- Attempt 备份路径和回退文件清单。
- 回退前后业务 diff/handoff/log 指纹比对结果。
- 新 Reviewer Provider session ID。
- 新 Reviewer 的真实 verdict 与 findings。
- 当前页面可继续执行的下一步。
