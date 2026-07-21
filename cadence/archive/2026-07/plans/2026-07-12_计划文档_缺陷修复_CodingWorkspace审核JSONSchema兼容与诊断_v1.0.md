# Coding Workspace 审核 JSON Schema 兼容与诊断实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让 Coding Review 合法的 `verdict="blocked"` + `severity="blocked"` 输出被稳定解析，并把 JSON 语法错误与业务 Schema 错误显示为不同诊断。

**Architecture:** 保留 Coding Workspace 现有直接 JSON 解析和 blocked Gate 路由，不新增 Provider repair turn。输入层将 `blocked` 规范化为 `FindingSeverity::Error`；Prompt 明确固定枚举；解析 fallback 根据 `serde_json::Error::classify()` 输出语法或 Schema 诊断。

**Tech Stack:** Rust stable、Serde/serde_json、Cargo 单元测试。

## Global Constraints

- 当前 `coding_attempt_0001`、Role Run、Code Review Report 和 blocked Gate 只能只读检查，不得原地重写或自动重放。
- 不新增第二次 Coding Review Provider repair 调用。
- `severity="blocked"` 仅作为输入兼容别名；序列化输出仍为 `error`。
- 保持 `retry_review`、`send_to_coder`、`abort` 的现有 Gate 行为。
- 遵循 TDD：每项生产修改前必须先运行对应 RED 测试并确认失败原因。
- 禁止给任何 Cargo 命令添加 `-j 1`；定向 lib 测试必须使用 `cargo test --locked --lib <filter>`。
- 只 stage 本计划涉及的文件；不得 reset、stash、clean 或提交当前 worktree 的其他 dirty 改动。
- 所有修改后的 Rust 源码与测试文件必须小于 800 行。

---

### Task 1: 兼容 blocked severity 并区分解析错误

**Files:**

- Modify: `src/product/coding_models/review.rs:64-94`
- Modify: `src/product/coding_workspace_engine/review_parser.rs:96-170`
- Test: `src/product/coding_workspace_engine/tests/parser_prompt.rs:341-435`

**Interfaces:**

- Consumes: `FindingSeverity::deserialize()`、`parse_review_payload(full_output, default_source_stage)`。
- Produces: `blocked` 输入别名映射为 `FindingSeverity::Error`；`blocked_review_payload(full_output, error)` 按 serde 错误类型生成精确 summary。

- [ ] **Step 1: 写 blocked severity RED 测试**

在 `parser_prompt.rs` 增加与真实 `code_review_0009` 同构的最小测试：

```rust
#[test]
fn review_parser_accepts_blocked_finding_severity_as_error() {
    let payload = r#"{
      "verdict": "blocked",
      "summary": "dependency handoff blocker",
      "findings": [
        {
          "severity": "blocked",
          "file_path": "src/web/runtime/provider.rs",
          "line": 109,
          "message": "shared gate is not wired",
          "required_action": "inject the shared gate",
          "source_stage": "code_review"
        }
      ]
    }"#;

    let parsed = parse_review_payload(payload, CodingExecutionStage::CodeReview);

    assert_eq!(parsed.verdict, ReviewVerdict::Blocked);
    assert_eq!(parsed.summary, "dependency handoff blocker");
    assert_eq!(parsed.findings.len(), 1);
    assert_eq!(
        parsed.findings[0].severity,
        crate::product::coding_models::FindingSeverity::Error
    );
    assert_eq!(parsed.findings[0].message, "shared gate is not wired");
}
```

- [ ] **Step 2: 运行测试确认 RED**

Run:

```bash
cargo test --locked --lib review_parser_accepts_blocked_finding_severity_as_error
```

Expected: FAIL；当前 parser 回退为 Blocked + 空 findings，`parsed.summary` 以“review 输出不是有效 JSON”开头。

- [ ] **Step 3: 写语法/Schema 诊断 RED 测试**

```rust
#[test]
fn review_parser_distinguishes_schema_error_from_json_syntax_error() {
    let schema_error = parse_review_payload(
        r#"{"verdict":"blocked","findings":[{"severity":"unexpected"}]}"#,
        CodingExecutionStage::CodeReview,
    );
    assert!(schema_error.summary.contains("review JSON Schema 校验失败"));
    assert!(schema_error.summary.contains("unknown variant"));

    let syntax_error = parse_review_payload(
        r#"{"verdict":"blocked","findings":["#,
        CodingExecutionStage::CodeReview,
    );
    assert!(syntax_error.summary.contains("review 输出不是有效 JSON"));
    assert!(!syntax_error.summary.contains("Schema 校验失败"));
}
```

- [ ] **Step 4: 运行诊断测试确认 RED**

Run:

```bash
cargo test --locked --lib review_parser_distinguishes_schema_error_from_json_syntax_error
```

Expected: FAIL；两个输入当前都使用同一“review 输出不是有效 JSON”summary，且不包含 serde 错误。

- [ ] **Step 5: 实现最小兼容映射**

在 `FindingSeverity::deserialize()` 中把 `blocked` 加入 Error 分支和期望值列表：

```rust
"error" | "blocked" | "blocker" | "blocking" | "critical" | "high" | "must_fix" => {
    Ok(Self::Error)
}
```

期望值列表增加 `"blocked"`，其余映射保持不变。

- [ ] **Step 6: 实现分类诊断**

将 parser 改为保留 serde error：

```rust
pub(crate) fn parse_review_payload(
    full_output: &str,
    default_source_stage: CodingExecutionStage,
) -> CodeReviewProviderPayload {
    let json = extract_json_object(full_output).unwrap_or(full_output);
    match serde_json::from_str::<RawCodeReviewProviderPayload>(json) {
        Ok(raw) => raw.into_payload(default_source_stage),
        Err(error) => blocked_review_payload(full_output, &error),
    }
}
```

将 fallback 签名和 summary 改为：

```rust
pub(crate) fn blocked_review_payload(
    full_output: &str,
    error: &serde_json::Error,
) -> CodeReviewProviderPayload {
    let prefix = match error.classify() {
        serde_json::error::Category::Syntax | serde_json::error::Category::Eof => {
            "review 输出不是有效 JSON"
        }
        serde_json::error::Category::Data => "review JSON Schema 校验失败",
        serde_json::error::Category::Io => "review JSON 解析失败",
    };
    CodeReviewProviderPayload {
        verdict: ReviewVerdict::Blocked,
        summary: format!(
            "{prefix}，已阻塞并等待人工确认: {error}; 原始输出: {}",
            non_empty_trimmed(full_output).unwrap_or_else(|| "<empty>".to_string())
        ),
        findings: Vec::new(),
        impact_scope: Vec::new(),
        pr_description: String::new(),
        commit_message_suggestion: String::new(),
        tested_evidence_refs: Vec::new(),
        diff_refs: Vec::new(),
    }
}
```

- [ ] **Step 7: 运行 parser GREEN 与回归**

Run:

```bash
cargo test --locked --lib review_parser
```

Expected: blocked alias、Schema 诊断、语法诊断及现有 alias/source-stage tests 全部 PASS。

- [ ] **Step 8: 提交 Task 1**

```bash
git add src/product/coding_models/review.rs \
  src/product/coding_workspace_engine/review_parser.rs \
  src/product/coding_workspace_engine/tests/parser_prompt.rs
git commit -m "fix: parse blocked coding review findings"
```

---

### Task 2: 对齐 Code Reviewer Prompt 与 Schema

**Files:**

- Modify: `src/product/coding_workspace_engine/prompts.rs:20-45`
- Modify: `src/product/coding_workspace_engine/prompts.rs:288-325`
- Test: `src/product/coding_workspace_engine/tests/parser_prompt.rs:188-340`

**Interfaces:**

- Consumes: `code_review_material_protocol()`、`group_final_review_material_protocol()`。
- Produces: 两类 Reviewer Prompt 都声明 verdict 与 severity 的精确枚举，以及 blocked finding 使用 error 的规则。

- [ ] **Step 1: 写 Prompt RED 测试**

```rust
#[test]
fn review_prompts_list_exact_finding_severity_values() {
    for protocol in [
        code_review_material_protocol(),
        group_final_review_material_protocol(),
    ] {
        assert!(protocol.contains("verdict 只能使用 approve、request_changes、blocked"));
        assert!(protocol.contains("severity 只能使用 error、warning、info"));
        assert!(protocol.contains("verdict=blocked 时，阻塞 finding 使用 severity=error"));
        assert!(protocol.contains("不得使用 severity=blocked"));
    }
}
```

- [ ] **Step 2: 运行测试确认 RED**

Run:

```bash
cargo test --locked --lib review_prompts_list_exact_finding_severity_values
```

Expected: FAIL；现有 protocol 没有 severity 枚举和 blocked 映射说明。

- [ ] **Step 3: 实现 Prompt 契约**

在 `code_review_material_protocol()` 与 `group_final_review_material_protocol()` 中加入完全相同的核心约束：

```text
- verdict 只能使用 approve、request_changes、blocked。
- finding.severity 只能使用 error、warning、info。
- verdict=blocked 时，阻塞 finding 使用 severity=error；不得使用 severity=blocked。
```

保留现有 JSON-only、source_stage 和材料审查规则。

- [ ] **Step 4: 运行 Prompt GREEN 与 parser 回归**

Run:

```bash
cargo test --locked --lib parser_prompt
```

Expected: 所有 parser/prompt 单元测试 PASS。

- [ ] **Step 5: 提交 Task 2**

```bash
git add src/product/coding_workspace_engine/prompts.rs \
  src/product/coding_workspace_engine/tests/parser_prompt.rs
git commit -m "fix: align coding review prompt schema"
```

---

### Task 3: 完整验证与真实数据只读验收

**Files:**

- Verify only: `.aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/**`
- Verify only: `/home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001`

**Interfaces:**

- Consumes: Task 1/2 的 parser 与 Prompt 修改。
- Produces: 完整门禁证据、当前 Attempt/共享 worktree 未被修改的哈希与 Git 指纹证据。

- [ ] **Step 1: 记录真实数据修复前哈希与共享 worktree 指纹**

```bash
sha256sum \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001.json \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/role-runs/coding_role_run_0020.json \
  .aria/projects/project_0001/issues/issue_0001/coding-attempts/coding_attempt_0001/code-reviews/code_review_0009.json
git -C /home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001 status --short
git -C /home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001 diff --stat
git -C /home/michaelche/workspace/github/cadence-aria/.worktrees/aria-issues/issue_0001 rev-parse HEAD
```

- [ ] **Step 2: 运行标准 Rust 门禁**

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Expected: 全部 exit 0；完整测试没有失败。

- [ ] **Step 3: 检查行数与 diff**

```bash
wc -l \
  src/product/coding_models/review.rs \
  src/product/coding_workspace_engine/review_parser.rs \
  src/product/coding_workspace_engine/prompts.rs \
  src/product/coding_workspace_engine/tests/parser_prompt.rs
git diff --check
git status --short --branch
```

Expected: 所有文件小于 800 行；diff 无 whitespace error；其他 dirty 文件仍原样存在且未被 stage/commit。

- [ ] **Step 4: 重新计算真实数据哈希与共享 worktree 指纹**

重复 Step 1 的命令。Expected: 三个 JSON 哈希、共享 issue worktree HEAD、modified/untracked 文件集合和 diff stat 与修复前一致。

- [ ] **Step 5: 汇报用户后续操作**

服务重启后，用户手动点击一次“重试代码审查”。预期新 Reviewer 输出即使继续使用 `severity="blocked"`，也会解析为 blocked report 并保留 findings；如果 dependency blocker 仍真实存在，系统仍停在 Code Review blocked Gate，而不会再显示误导性的“不是有效 JSON”。
