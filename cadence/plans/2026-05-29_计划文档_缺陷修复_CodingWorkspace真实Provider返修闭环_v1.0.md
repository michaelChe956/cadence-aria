# CodingWorkspace 真实 Provider 返修闭环缺陷修复 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 修复真实 provider 全流程中 CodeReview/InternalPrReview 提出问题后，后续 Coder 未按 findings 修复、Review JSON 误判通过、运行产物被提交的问题。

**Architecture:** 返修闭环以 `AnalystDecision` 为路由源，将 `summary/fix_hints/questions/source_stage` 持久化为下一轮 Coding 输入。Review 输出解析必须 fail-closed，但对真实 provider 常见字段别名做兼容归一化。测试输出统一写入 Aria attempt store，ReviewRequest 提交阶段只 staging 业务变更并过滤运行产物。

**Tech Stack:** Rust、Tokio、Serde、Axum WebSocket、Git CLI、Vitest/Playwright。

---

## 一、执行前置条件

- 必须在 worktree `/Users/michaelche/Documents/git-folder/github-folder/cadence-aria/.worktrees/product-workbench-issue-lifecycle` 执行。
- 不修改主工作区 `/Users/michaelche/Documents/git-folder/github-folder/cadence-aria` 中的同名计划文件。
- 使用宿主机 Rust/Cargo，不使用 Docker。
- 所有实现按 TDD 执行：先新增失败测试，再实现，再跑定向测试。

## 二、问题摘要

真实 `naruto` 爬楼梯流程暴露出 5 个缺陷：

1. CodeReview / InternalPrReview 已指出 `.pyc` 和 `.aria/coding-artifacts/test-output/*.log` 不应提交，但下一轮 Coder prompt 仍只包含原始 Work Item，没有包含返修原因和 fix hints。
2. CodeReview provider 输出 JSON 中使用了 `medium` / `low` / `blocking` severity，当前 `ReviewFinding.severity` 只接受 `error` / `warning` / `info`，反序列化失败后 `parse_code_review_payload()` fallback 成 `Approve`。
3. 真实 provider findings 可能使用 `file` / `description` / `recommendation` / `title` 字段，当前 `ReviewFinding` 只接受 `file_path` / `message` / `required_action` / `source_stage`。
4. `execute_review_request()` 使用 `git add -A`，把测试运行产物和 Python `__pycache__` 全量提交。
5. `run_all_tests()` 把 stdout/stderr 写到目标 worktree 内的 `.aria/coding-artifacts/test-output/`，导致 reviewer diff/status 天然被运行产物污染。

## 三、文件结构

- Modify: `src/product/coding_models.rs`
  - 扩展 Review finding severity 反序列化。
  - 新增 `CodingReworkInstruction`。
- Modify: `src/product/coding_attempt_store.rs`
  - 新增保存、读取、消费返修指令。
  - 新增 attempt artifact 根目录与 artifact 读写路径方法。
- Modify: `src/product/coding_workspace_engine.rs`
  - `execute_rework()` 在 NeedsFix 时保存返修指令。
  - `execute_coding()` 使用真实 coder provider 名称，并注入未消费返修指令。
  - `parse_review_payload()` 按调用方 stage 解析 CodeReview/InternalPrReview 输出。
  - `execute_review_request()` 使用安全 staging 方法。
  - `execute_testing()` 将 artifact 根目录传给 test executor。
- Modify: `src/product/git_workspace_service.rs`
  - 新增安全 staging 方法，排除运行产物。
- Modify: `src/product/test_executor.rs`
  - 将测试 stdout/stderr 写到 Aria attempt store，而不是目标代码 worktree。
- Modify: `src/web/handlers.rs`
  - `/api/coding-attempts/:attempt_id/artifacts/:artifact_id` 从 attempt store 读取测试 artifact。
- Modify: `src/web/coding_ws_handler.rs`
  - 调用 `execute_coding()` 时传入实际 coder provider 名称，或适配新的 engine 签名。
- Test: `tests/product_coding_models.rs`
- Test: `tests/product_coding_attempt_store.rs`
- Test: `tests/product_coding_workspace_engine.rs`
- Test: `tests/product_git_workspace_service.rs`
- Test: `tests/product_test_executor.rs`
- Test: `tests/web_coding_attempt_api.rs`
- Test: `tests/web_coding_ws_handler.rs`

## 四、任务清单

### Task 1: Review JSON 解析 fail-closed 且兼容真实 provider 字段

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/product_coding_models.rs`
- Test: `tests/product_coding_workspace_engine.rs`

- [x] **Step 1: 新增 severity 兼容失败测试**

在 `tests/product_coding_models.rs` 增加测试，覆盖 severity 映射：

```rust
#[test]
fn review_finding_deserializes_provider_severity_aliases() {
    let json = r#"{"severity":"medium","file_path":"src/lib.rs","line":1,"message":"fix","required_action":"change","source_stage":"code_review"}"#;

    let finding: ReviewFinding = serde_json::from_str(json).expect("finding should parse");

    assert_eq!(finding.severity, FindingSeverity::Warning);
}
```

Run:

```bash
cargo test --locked --test product_coding_models review_finding_deserializes_provider_severity_aliases -- --nocapture
```

Expected: 当前失败，`medium` 不能反序列化。

- [x] **Step 2: 实现 severity 自定义反序列化**

在 `FindingSeverity` 上实现自定义 deserialize，映射规则：

```text
blocking | critical | high -> error
medium -> warning
low -> info
error | warning | info -> 原样
```

未知 severity 不默认为 approve；返回 serde error，让上层 `parse_review_payload()` fail-closed。

- [x] **Step 3: 新增真实 provider findings 字段兼容失败测试**

在 `tests/product_coding_workspace_engine.rs` 增加测试，输入真实 provider 形态：

```json
{
  "verdict": "request_changes",
  "summary": "范围污染",
  "findings": [
    {
      "severity": "blocking",
      "file": "__pycache__/x.pyc",
      "description": "不应提交运行产物",
      "recommendation": "从提交中移除 pyc 文件",
      "title": "运行产物进入提交"
    }
  ]
}
```

期望：
- verdict 为 `RequestChanges`
- severity 为 `Error`
- `file` 归一化到 `file_path`
- `description` 归一化到 `message`
- `recommendation` 归一化到 `required_action`
- 缺失 `source_stage` 时由调用方补为 `CodingExecutionStage::CodeReview`

Run:

```bash
cargo test --locked --test product_coding_workspace_engine parses_real_provider_review_finding_aliases -- --nocapture
```

Expected: 当前失败，payload 会 fallback 成 approve 或 findings 为空。

- [x] **Step 4: 将 parser 改为带默认 stage 的归一化入口**

将当前 `parse_code_review_payload(full_output)` 替换为：

```rust
fn parse_review_payload(
    full_output: &str,
    default_source_stage: CodingExecutionStage,
) -> CodeReviewProviderPayload
```

实现要点：
- `build_code_review_report()` 使用 `CodingExecutionStage::CodeReview`。
- `build_internal_pr_review()` 使用 `CodingExecutionStage::InternalPrReview`。
- finding 支持字段别名：
  - `file_path` 或 `file`。
  - `message` 或 `description`。
  - `required_action` 或 `recommendation`。
  - `source_stage` 缺失时使用 `default_source_stage`。
  - `title` 可忽略，不影响解析。
- JSON 完全无效、verdict 缺失或 verdict 非法时返回 `ReviewVerdict::Blocked`。
- top-level verdict 可解析时，不因单个可修复字段别名而降级为 `Blocked`。

- [x] **Step 5: 修改解析失败策略**

解析失败时返回：

```rust
CodeReviewProviderPayload {
    verdict: ReviewVerdict::Blocked,
    summary: format!("review 输出不是有效 JSON，已阻塞并等待人工确认: {}", trimmed_output),
    findings: Vec::new(),
    impact_scope: Vec::new(),
    pr_description: String::new(),
    commit_message_suggestion: String::new(),
    tested_evidence_refs: Vec::new(),
    diff_refs: Vec::new(),
}
```

禁止 fallback 为 `ReviewVerdict::Approve`。

- [x] **Step 6: 验证**

Run:

```bash
cargo test --locked --test product_coding_models review_finding_deserializes_provider_severity_aliases -- --nocapture
cargo test --locked --test product_coding_workspace_engine parses_real_provider_review_finding_aliases -- --nocapture
cargo test --locked --test product_coding_workspace_engine review_payload_parse_failure_blocks_instead_of_approves -- --nocapture
```

Expected: 相关测试通过。

### Task 2: 返修指令持久化并注入下一轮 Coder

**Files:**
- Modify: `src/product/coding_models.rs`
- Modify: `src/product/coding_attempt_store.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/product_coding_attempt_store.rs`
- Test: `tests/product_coding_workspace_engine.rs`

- [x] **Step 1: 新增模型**

在 `src/product/coding_models.rs` 新增：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodingReworkInstruction {
    pub id: String,
    pub attempt_id: String,
    pub source_stage: CodingExecutionStage,
    pub rework_round: u32,
    pub summary: String,
    pub fix_hints: Vec<String>,
    pub questions: Vec<String>,
    pub created_at: String,
    pub consumed_by_node_id: Option<String>,
    pub consumed_at: Option<String>,
}
```

存储语义：每次 `NeedsFix` 保存一条指令；下一轮 Coding 只读取最新未消费指令；Coding node 创建后立即标记消费。

- [x] **Step 2: 新增 store 测试**

在 `tests/product_coding_attempt_store.rs` 增加测试：

```rust
#[test]
fn saves_reads_and_consumes_latest_coding_rework_instruction() {
    // 创建 attempt 后保存两条 instruction。
    // 读取 latest unconsumed 应返回第二条。
    // mark consumed 后再次读取应返回 None。
}
```

Run:

```bash
cargo test --locked --test product_coding_attempt_store saves_reads_and_consumes_latest_coding_rework_instruction -- --nocapture
```

Expected: 当前失败，store 方法不存在。

- [x] **Step 3: 实现 store API**

在 `CodingAttemptStore` 增加：

```rust
pub fn save_rework_instruction(&self, instruction: &CodingReworkInstruction) -> Result<(), ProductStoreError>
pub fn latest_unconsumed_rework_instruction(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> Result<Option<CodingReworkInstruction>, ProductStoreError>
pub fn mark_rework_instruction_consumed(&self, project_id: &str, issue_id: &str, attempt_id: &str, instruction_id: &str, node_id: &str) -> Result<CodingReworkInstruction, ProductStoreError>
```

建议路径：

```text
.aria/projects/<project_id>/issues/<issue_id>/coding-attempts/<attempt_id>/rework-instructions/<instruction_id>.json
```

- [x] **Step 4: `execute_rework()` 保存 NeedsFix 指令**

在 `apply_analyst_decision()` 的 `AnalystVerdict::NeedsFix` 分支中，当 `attempt.rework_count < attempt.max_auto_rework` 时保存 instruction：
- `source_stage`
- `rework_round`
- `decision.summary`
- `decision.fix_hints`
- `decision.questions`

达到自动返修上限时不再保存新 instruction，保持当前阻塞/回退逻辑。

- [x] **Step 5: `build_coding_prompt()` 注入返修上下文**

修改 `build_coding_prompt()` 签名，接收 `Option<&CodingReworkInstruction>`，下一轮 Coder prompt 增加：

```text
上一轮返修要求:
- 来源阶段: CodeReview
- 摘要: ...
- 修复提示:
  1. ...
  2. ...
- 待澄清问题:
  1. ...

本轮必须优先修复上述问题。完成前请检查 git diff/status，确认 reviewer 指出的文件或行为已处理。
```

- [x] **Step 6: Coding node 创建后标记 consumed**

`execute_coding()` 创建 Coding timeline node 后：
- 读取 latest unconsumed instruction。
- 构建 prompt 时注入 instruction。
- 发送 prompt event 后，将 instruction 标记为当前 coding node 消费。

如果 provider 后续失败，仍保留 consumed 审计记录，不重复注入同一指令；新的失败应由下一次 Rework 生成新 instruction。

- [x] **Step 7: 验证**

Run:

```bash
cargo test --locked --test product_coding_workspace_engine coding_prompt_includes_rework_fix_hints -- --nocapture
cargo test --locked --test web_coding_ws_handler rework_needs_fix_feeds_next_coding_prompt -- --nocapture
```

Expected: prompt 中能看到 CodeReview/InternalPrReview 的 `summary`、`fix_hints` 和 `questions`。

### Task 3: Coder provider 名称与实际 role snapshot 保持一致

**Files:**
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/web/coding_ws_handler.rs`
- Test: `tests/web_coding_ws_handler.rs`
- Test: `tests/product_coding_workspace_engine.rs`

- [x] **Step 1: 新增失败测试**

构造 role provider snapshot：
- `author = Fake`
- `coder = Codex`

执行 Coding gate 后断言 provider prompt event 和 `AdapterInput.provider_type` 使用 `coder = Codex`，而不是 `author = Fake`。

Run:

```bash
cargo test --locked --test web_coding_ws_handler coding_stage_uses_role_snapshot_coder_provider -- --nocapture
```

Expected: 当前失败，`execute_coding()` 内仍使用 `attempt.provider_config_snapshot.author`。

- [x] **Step 2: 修改 `execute_coding()` 签名或内部取值**

优先方案：在 `execute_coding()` 内通过 store 读取 role snapshot 的 `.coder`：

```rust
let coder_provider = self
    .store
    .get_role_provider_config_snapshot(&attempt.project_id, &attempt.issue_id, &attempt.id)?
    .coder;
```

并将它用于：
- `provider_prompt_event(&node.id, &coder_provider, prompt.clone())`
- `AdapterInput.provider_type = provider_type_for_name(&coder_provider)`

`coding_ws_handler.rs` 仍负责选择实际 provider 实例；两者必须来自同一 role snapshot。

- [x] **Step 3: 验证**

Run:

```bash
cargo test --locked --test web_coding_ws_handler coding_stage_uses_role_snapshot_coder_provider -- --nocapture
cargo test --locked --test product_coding_workspace_engine execute_coding_emits_prompt_for_coder_provider -- --nocapture
```

Expected: Coding prompt event 与 AdapterInput provider type 均使用实际 coder provider。

### Task 4: 提交范围过滤，禁止运行产物进入 ReviewRequest

**Files:**
- Modify: `src/product/git_workspace_service.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/product_git_workspace_service.rs`
- Test: `tests/product_coding_workspace_engine.rs`

- [x] **Step 1: 新增失败测试**

构造临时 git repo，包含：

```text
climbing_stairs.py
tests/test_climbing_stairs.py
__pycache__/climbing_stairs.cpython-310.pyc
tests/__pycache__/test_climbing_stairs.cpython-310.pyc
pkg/sub/__pycache__/x.cpython-310.pyc
.aria/coding-artifacts/test-output/planned_001.stdout.log
.aria/coding-artifacts/test-output/planned_001.stderr.log
```

调用新的 staging 方法后，期望 staged 文件只包含：

```text
climbing_stairs.py
tests/test_climbing_stairs.py
```

Run:

```bash
cargo test --locked --test product_git_workspace_service stages_source_changes_without_runtime_artifacts -- --nocapture
```

Expected: 当前失败，因为仍是 `git add -A`。

- [x] **Step 2: 实现安全 staging**

新增：

```rust
pub async fn git_add_work_item_changes(&self, worktree_path: &Path) -> Result<(), GitWorkspaceError>
```

实现步骤：
- `git add -A`
- `git diff --cached --name-only -z`
- 解析 NUL 分隔路径。
- 对每个 staged path 判断是否应排除：
  - `path == ".aria"` 或 `path.starts_with(".aria/coding-artifacts/")`
  - `path == "__pycache__"` 或 `path.contains("/__pycache__/")` 或 `path.starts_with("__pycache__/")`
  - `path.ends_with(".pyc")`
- 对每个应排除路径执行：

```bash
git restore --staged -- <path>
```

不要使用 shell glob，不删除用户文件，只从 staged 区移除。

- [x] **Step 3: 替换 ReviewRequest 提交逻辑**

将 `execute_review_request()` 中的：

```rust
self._git_service.git_add_all(worktree_path).await?;
```

替换为：

```rust
self._git_service.git_add_work_item_changes(worktree_path).await?;
```

- [x] **Step 4: 处理空提交**

如果过滤后没有 staged changes，`git commit` 会失败。该场景应转为 `ReviewRequest` 阶段 blocked，并给出明确 summary：

```text
过滤运行产物后没有可提交的业务变更，请检查上一轮 Coder 是否只修改了运行产物。
```

不要继续 push。

- [x] **Step 5: 验证**

Run:

```bash
cargo test --locked --test product_git_workspace_service stages_source_changes_without_runtime_artifacts -- --nocapture
cargo test --locked --test product_coding_workspace_engine review_request_does_not_commit_runtime_artifacts -- --nocapture
cargo test --locked --test product_coding_workspace_engine review_request_blocks_when_only_runtime_artifacts_changed -- --nocapture
```

Expected: commit 中不包含 `.pyc`、`__pycache__`、`.aria/coding-artifacts`。

### Task 5: 测试 artifacts 与目标仓库 diff 解耦

**Files:**
- Modify: `src/product/coding_attempt_store.rs`
- Modify: `src/product/test_executor.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Modify: `src/web/handlers.rs`
- Test: `tests/product_coding_attempt_store.rs`
- Test: `tests/product_test_executor.rs`
- Test: `tests/web_coding_attempt_api.rs`

- [x] **Step 1: 新增失败测试**

在 `tests/product_test_executor.rs` 增加测试，验证 `run_all_tests()` 后目标 worktree 的 `git status --short` 不出现 `.aria/coding-artifacts/test-output`。

Run:

```bash
cargo test --locked --test product_test_executor test_outputs_do_not_pollute_target_worktree_status -- --nocapture
```

Expected: 当前失败，因为 artifacts 写在 worktree 内。

- [x] **Step 2: 在 store 暴露 attempt artifact 路径**

在 `CodingAttemptStore` 增加：

```rust
pub fn attempt_artifact_root(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf
pub fn attempt_test_output_root(&self, project_id: &str, issue_id: &str, attempt_id: &str) -> PathBuf
pub fn attempt_test_output_path(&self, project_id: &str, issue_id: &str, attempt_id: &str, artifact_id: &str) -> Result<PathBuf, ProductStoreError>
```

路径：

```text
.aria/projects/<project_id>/issues/<issue_id>/coding-attempts/<attempt_id>/artifacts/test-output/
```

- [x] **Step 3: 修改 test executor 输出根目录**

调整 `run_all_tests()` 和 `execute_test_command()`：

```rust
pub async fn execute_test_command(
    spec: &TestCommandSpec,
    worktree_path: impl AsRef<Path>,
    artifact_output_root: impl AsRef<Path>,
) -> Result<TestCommand, TestExecutorError>

pub async fn run_all_tests(
    attempt_id: &str,
    worktree_path: impl AsRef<Path>,
    artifact_output_root: impl AsRef<Path>,
    specs: &[TestCommandSpec],
) -> Result<TestingReport, TestExecutorError>
```

`TestCommand.stdout_ref` / `stderr_ref` 只保存文件名：

```text
planned_001.stdout.log
planned_001.stderr.log
```

不要再保存 `.aria/coding-artifacts/test-output/...` 相对路径。

- [x] **Step 4: 修改 engine 调用**

`execute_testing()` 使用：

```rust
let artifact_output_root = self.store.attempt_test_output_root(
    &attempt.project_id,
    &attempt.issue_id,
    &attempt.id,
);
let report = run_all_tests(&attempt.id, worktree_path, artifact_output_root, specs).await?;
```

- [x] **Step 5: 修改 artifact API handler**

`src/web/handlers.rs` 的 `coding_attempt_artifact_content()` 改为：
- 通过 `coding_store.get_attempt_by_id()` 找 attempt。
- 使用 `coding_store.attempt_test_output_path(...)` 定位 artifact。
- 不再依赖 `attempt.worktree_path/.aria/coding-artifacts/test-output`。

保留 `validate_relative_id(&artifact_id)`，禁止路径穿越。

- [x] **Step 6: 兼容旧 artifact ref**

如果历史 `stdout_ref` 是 `.aria/coding-artifacts/test-output/unit.stdout.log`，API handler 需要只取文件名 `unit.stdout.log` 再读取新 store 路径。新写入的 report 使用文件名格式。

- [x] **Step 7: 验证**

Run:

```bash
cargo test --locked --test product_test_executor test_outputs_do_not_pollute_target_worktree_status -- --nocapture
cargo test --locked --test web_coding_attempt_api reads_test_output_artifact_from_attempt_store -- --nocapture
cargo test --locked --test web_coding_attempt_api reads_legacy_test_output_artifact_ref_by_file_name -- --nocapture
```

Expected:
- 目标代码库 `git status --short` 不出现 `.aria/coding-artifacts/test-output`。
- TestingReport stdout/stderr 仍可通过 UI/API 读取。

### Task 6: InternalPrReview 返修后重新生成 ReviewRequest

**Files:**
- Modify: `src/web/coding_ws_handler.rs`
- Modify: `src/product/coding_workspace_engine.rs`
- Test: `tests/web_coding_ws_handler.rs`

- [x] **Step 1: 新增回归测试**

构造 provider fixture：
- 第一轮 Coding 产生业务变更。
- CodeReview approve。
- ReviewRequest 创建 commit A。
- InternalPrReview request_changes。
- Analyst needs_fix。
- 第二轮 Coding 修复问题。
- 第二轮 ReviewRequest 必须创建 commit B，并且 commit B 是最新 review request 的 `commit_sha`。

Run:

```bash
cargo test --locked --test web_coding_ws_handler internal_review_rework_creates_new_review_request_commit -- --nocapture
```

Expected: 若当前流程没有重新 commit/push 或 latest request 不更新，测试失败。

- [x] **Step 2: 确认现有 loop 行为并补缺**

当前 runner 在 `InternalPrReview -> Rework -> Coding` 后会回到 pipeline。实现时确认：
- 第二轮 CodeReview 通过后进入 `ReviewRequest`。
- `execute_review_request()` 创建新的 `review_request_N`。
- 最新 `ReviewRequest.commit_sha` 指向第二轮 commit。
- push 使用同一 branch 时为 fast-forward 更新。

如果现有行为已满足，只保留测试；若不满足，修正 runner stage 路由。

- [x] **Step 3: 验证**

Run:

```bash
cargo test --locked --test web_coding_ws_handler internal_review_rework_creates_new_review_request_commit -- --nocapture
```

Expected: InternalPrReview 返修后的最新 ReviewRequest 指向修复后的 commit。

### Task 7: 真实流程回归与 UI 证据

**Files:**
- Modify: `tests/web_coding_ws_handler.rs`
- Modify: `web/e2e/stage-ui.spec.ts` 或 Create: `web/e2e/coding-rework.spec.ts`

- [x] **Step 1: 增加后端集成测试**

构造 provider fixture：
- 第一轮 CodeReview 输出 `request_changes`，finding 要求移除 `.pyc`
- Analyst 输出 `needs_fix + fix_hints`
- 第二轮 Coder prompt 必须包含该 fix hint
- 第二轮 ReviewRequest diff 不包含 `.pyc` / `.aria/coding-artifacts`

Run:

```bash
cargo test --locked --test web_coding_ws_handler code_review_findings_are_injected_into_next_coding_round -- --nocapture
```

Expected: 当前失败，prompt 中没有 finding。

- [x] **Step 2: 增加 E2E 或集成验收**

新增/扩展 E2E 验证：
- CodeReview 卡片出现 `需要修复`
- 后续 Coding 轮次可见返修提示
- TestingReport stdout/stderr 链接可打开
- 最终没有运行产物进入 ReviewRequest diff

Run:

```bash
cd web
pnpm exec playwright test e2e/coding-rework.spec.ts
```

Expected: E2E 通过。

### Task 8: 全量验证

- [x] **Step 1: Rust 格式与静态检查**

```bash
cargo fmt --check
cargo check --locked
cargo clippy --all-targets --all-features --locked -- -D warnings
```

- [x] **Step 2: Rust 全量测试**

```bash
cargo test --locked -j 1
```

- [x] **Step 3: 前端验证**

```bash
cd web
pnpm test
pnpm build
pnpm exec playwright test
```

- [ ] **Step 4: 真实 naruto 手动验收**

使用新的 `naruto` 测试 worktree 重新跑爬楼梯真实 provider 流程。

验收标准：
- CodeReview 若提出 `request_changes`，下一轮 Coder prompt 中必须出现对应 finding/fix_hints。
- 若 reviewer 要求移除 `.pyc` / `.aria/coding-artifacts`，后续 commit 不再包含这些文件。
- `git diff main --name-status` 只包含业务实现和测试文件，或明确允许的 `.gitignore` 更新。
- TestingReport 仍可查看 stdout/stderr。
- InternalPrReview 若要求修改，修复后会创建新的 ReviewRequest commit。
- Attempt 最终进入 `completed`，而不是在 CodeReview 阶段因重复同一问题耗尽 rework。

## 五、验收标准

1. Review provider 输出非严格 severity 时不会误判 approve。
2. Review provider 使用 `file` / `description` / `recommendation` 等字段时能归一化为 `ReviewFinding`。
3. Review JSON 完全解析失败或 verdict 非法时 fail-closed，状态为 blocked，不进入 ReviewRequest。
4. Analyst 的 NeedsFix summary/fix_hints/questions 会进入下一轮 Coder prompt。
5. Coding 阶段的 prompt event 和 AdapterInput 使用真实 coder provider，而不是旧 author 字段。
6. 自动返修轮次能针对上一轮 reviewer finding 做闭环修复。
7. ReviewRequest commit 不再包含 `.pyc`、`__pycache__`、`.aria/coding-artifacts/test-output`。
8. 测试 stdout/stderr 不写入目标代码 worktree，目标 repo `git status` 不被 test artifacts 污染。
9. TestingReport artifacts 仍可从 UI/API 查看，历史 artifact ref 能兼容读取。
10. InternalPrReview 返修后会生成新的 ReviewRequest commit。
11. 全量 Rust、前端、Playwright 验证通过。
12. 真实 `naruto` 爬楼梯场景不再复现“reviewer 提问题但 coder 没解决”的问题。

## 六、不做的事

- 不在本计划中改真实 provider 的账号配置。
- 不实现完整 PR 平台集成，只保证当前 branch-only ReviewRequest 质量。
- 不扩大 CodingWorkspace 到并发 attempt。
- 不改变 Work Item / Story / Design 生成流程。
- 不手动修改 `naruto` 当前残留 attempt 作为主修复路径；真实验收使用新的测试 worktree 重跑。
