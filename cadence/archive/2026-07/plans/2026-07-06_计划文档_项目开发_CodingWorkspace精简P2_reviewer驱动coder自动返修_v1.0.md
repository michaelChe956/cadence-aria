# Coding Workspace 精简 Plan 2：reviewer 驱动 coder 自动返修

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** 当 code_reviewer 输出 `request_changes` 时，直接将 findings 作为返修指令，复用之前的 coder provider（resume session、增量喂入）自动重跑 Coding，受 `max_auto_rework` 上限约束；超出上限才停下等人。

**Architecture:** 在 Plan 1 完成的基础上，在 runner.rs 的 CodeReview 段增加 `request_changes` 分支：若未超上限则调新函数 `execute_reviewer_driven_rework`（在 `rework.rs` 中实现），该函数将 `CodeReviewReport.findings` 组装成 `CodingReworkInstruction`，用 coder provider（`snapshot.coder`）+ resume session + `build_coding_delta_prompt` 增量驱动 coder 重跑；超上限则创建 blocked gate 等人。同时强化 `build_coding_prompt` / `build_coding_delta_prompt` 的自检契约。

**Tech Stack:** Rust（edition 2024），依赖 `CodingReworkInstruction`、`increment_attempt_rework_count`、`provider_resume_session_id_for_attempt`、`build_coding_delta_prompt`（均已存在）。

## Global Constraints

- 禁止 `cargo test -j 1`
- 新函数 `execute_reviewer_driven_rework` 放在 `rework.rs`（和现有 `execute_rework_with_commands` 同文件）
- coder 返修使用 `build_coding_delta_prompt`（resume session 增量指令），**不是** `build_rework_prompt`（那是给 analyst 的）
- 返修使用的 provider 固定取 `snapshot.coder`，不使用 analyst/reviewer provider
- `CodingExecutionStage::Rework` 枚举保留（向后兼容）；新返修轮次的 timeline node 依然记录为 `Rework` stage，role run 记录为 `CodingProviderRole::Coder`

---

### Task 1：强化 coder 自检契约

**Files:**
- Modify: `src/product/coding_workspace_engine/prompts.rs`（`build_coding_prompt` 和 `build_coding_delta_prompt`）

**Interfaces:**
- Consumes: 现有 `build_coding_prompt`（`prompts.rs:113`）、`build_coding_delta_prompt`（`prompts.rs:179`）
- Produces: 两函数输出的 prompt 末尾增加强制自检约束段

- [x] **Step 1: 在 build_coding_prompt 末尾追加自检约束**

在 `prompts.rs` 中找到 `build_coding_prompt` 的末尾（当前最后追加的是 `dependency_bootstrap_guidance()` 和执行要求段），在现有执行要求段**之后**追加：

```rust
    prompt.push_str(
        "\n自检契约（完成前必须执行，不得跳过）:\n\
         - 实际执行上述验证命令并将完整输出粘贴到报告中。\n\
         - 如果测试输出包含 \"0 tests\" 或 \"running 0 tests\"，视为测试未覆盖，必须补充测试用例。\n\
         - 每个新增的 .rs 源文件必须已挂载到 crate（通过 mod 声明或 lib.rs/main.rs 的模块树），\
否则 cargo check 不会发现其编译错误。\n\
         - 完成前执行 git diff --stat，确认预期文件确实有变更，无多余或遗漏的文件。\n",
    );
```

- [x] **Step 2: 在 build_coding_delta_prompt 末尾追加同样的自检约束**

在 `build_coding_delta_prompt` 末尾（`dependency_bootstrap_guidance()` 之后）追加：

```rust
    prompt.push_str(
        "\n自检契约（完成前必须执行，不得跳过）:\n\
         - 实际执行上述验证命令并将完整输出粘贴到报告中。\n\
         - 如果测试输出包含 \"0 tests\" 或 \"running 0 tests\"，视为测试未覆盖，必须补充测试用例。\n\
         - 每个新增的 .rs 源文件必须已挂载到 crate（通过 mod 声明或 lib.rs/main.rs 的模块树），\
否则 cargo check 不会发现其编译错误。\n\
         - 完成前执行 git diff --stat，确认预期文件确实有变更，无多余或遗漏的文件。\n",
    );
```

- [x] **Step 3: 编译确认**

```bash
cd /home/michael/workspace/github/cadence-aria/.worktrees/feat-b-0630
cargo check --locked 2>&1 | head -30
```

预期：0 errors。

- [x] **Step 4: 提交**

```bash
git add src/product/coding_workspace_engine/prompts.rs
git commit -m "feat(coding): add coder self-check contract to prompts"
```

---

### Task 2：实现 execute_reviewer_driven_rework

**Files:**
- Modify: `src/product/coding_workspace_engine/rework.rs`（新增 `execute_reviewer_driven_rework` 函数）

**Interfaces:**
- Consumes:
  - `build_coding_delta_prompt(attempt, context, rework_instruction, context_notes) -> String`（`prompts.rs:179`）
  - `provider_resume_session_id_for_attempt(attempt, &CodingProviderRole::Coder, provider_name) -> Option<String>`（`lifecycle.rs:31`）
  - `record_attempt_provider_session(attempt, role, provider, session_id, node_id)`（`lifecycle.rs:52`）
  - `increment_attempt_rework_count(project_id, issue_id, attempt_id) -> Result<CodingExecutionAttempt>`
  - `create_rework_timeline_node(attempt, rework_round) -> Result<CodingTimelineNode>`（`timeline.rs`）
  - `create_role_run(attempt, stage, role, trigger, node_id)` / `update_role_run_status`
  - `save_rework_instruction(instruction)` / `list_rework_instructions`
  - `coding_execution_context(app_paths, attempt)`（`runner.rs` 顶部调用）
  - `CodingReworkInstruction { id, attempt_id, source_stage, rework_round, summary, fix_hints, questions, created_at, consumed_by_node_id, consumed_at }`（`coding_models/context.rs`）
  - `execute_coding_with_commands(attempt, provider, context, command_rx)`（`provider_stream.rs:15`）
- Produces:
  - `pub async fn execute_reviewer_driven_rework(attempt, provider, context, command_rx) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError>`
  - 返回的 attempt.stage 为 `CodingExecutionStage::Coding`（返修后重新 Coding）或 `Blocked`（超上限）

- [x] **Step 1: 在 rework.rs 末尾添加新函数**

在 `src/product/coding_workspace_engine/rework.rs` 末尾（`continue_rework_after_limit_for_attempt` 之后，最后一个 `}` 之前）添加：

```rust
    /// reviewer 驱动的 coder 自动返修。
    /// 从 `review_report` 的 findings 组装返修指令，
    /// 用 coder provider（resume session）增量重跑 Coding。
    /// 若未超 max_auto_rework 上限：写入 CodingReworkInstruction，increment rework_count，
    ///   stage → Coding，返回 updated attempt。
    /// 若已超上限：创建 blocked gate，status → Blocked，返回 blocked attempt。
    pub async fn execute_reviewer_driven_rework(
        &self,
        attempt: &CodingExecutionAttempt,
        review_report: &crate::product::coding_models::CodeReviewReport,
        context: &crate::product::coding_evaluation_context::CodingExecutionContext,
        provider: &dyn StreamingProviderAdapter,
        command_rx: &mut mpsc::Receiver<CodingRunnerCommand>,
    ) -> Result<CodingExecutionAttempt, CodingWorkspaceEngineError> {
        let current =
            self.store
                .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)?;
        let rework_round = current.rework_count + 1;

        if current.rework_count >= current.max_auto_rework {
            // 超上限 → blocked gate 等人
            use crate::product::coding_attempt_store::inputs::CreateBlockedGateInput;
            let summary = format!(
                "code review 连续要求修改 {} 次，已达上限，请人工介入。",
                current.rework_count
            );
            let findings_summary = review_report
                .findings
                .iter()
                .map(|f| format!("- [{:?}] {}", f.severity, f.message))
                .collect::<Vec<_>>()
                .join("\n");
            let gate = self.store.create_blocked_gate(CreateBlockedGateInput {
                attempt_id: current.id.clone(),
                project_id: current.project_id.clone(),
                issue_id: current.issue_id.clone(),
                title: "Code Review 返修超上限".to_string(),
                description: format!("{}\n\n最新 findings:\n", summary, findings_summary),
                stage: Some(CodingExecutionStage::Rework),
                role: Some(CodingProviderRole::Coder),
                evidence_refs: review_report
                    .raw_provider_output_ref
                    .clone()
                    .map(|r| vec![r])
                    .unwrap_or_default(),
            })?;
            let _ = self
                .event_tx
                .send(CodingWsOutMessage::CodingGateRequired { gate })
                .await;
            let updated = self.store.update_attempt_status(
                &current.project_id,
                &current.issue_id,
                &current.id,
                CodingAttemptStatus::Blocked,
            )?;
            return Ok(updated);
        }

        // 未超上限 → 组装返修指令，重跑 Coding
        let existing_instructions = self.store.list_rework_instructions(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let summary = if review_report.summary.is_empty() {
            format!("code review round {} 要求修改", review_report.round)
        } else {
            review_report.summary.clone()
        };
        let fix_hints: Vec<String> = review_report
            .findings
            .iter()
            .filter(|f| {
                matches!(
                    f.severity,
                    crate::product::coding_models::FindingSeverity::Error
                        | crate::product::coding_models::FindingSeverity::Warning
                )
            })
            .map(|f| {
                let location = match (&f.file_path, f.line) {
                    (Some(path), Some(line)) => format!("{}:{} ", path, line),
                    (Some(path), None) => format!("{} ", path),
                    _ => String::new(),
                };
                let action = f
                    .required_action
                    .as_deref()
                    .map(|a| format!(" → {}", a))
                    .unwrap_or_default();
                format!("{}{}{}", location, f.message, action)
            })
            .collect();

        let instruction = CodingReworkInstruction {
            id: next_sequential_id("coding_rework_instruction", existing_instructions.len()),
            attempt_id: current.id.clone(),
            source_stage: CodingExecutionStage::CodeReview,
            rework_round,
            summary: summary.clone(),
            fix_hints,
            questions: Vec::new(),
            created_at: Utc::now().to_rfc3339(),
            consumed_by_node_id: None,
            consumed_at: None,
        };
        self.store.save_rework_instruction(&instruction)?;

        let updated = self.store.increment_attempt_rework_count(
            &current.project_id,
            &current.issue_id,
            &current.id,
        )?;
        let updated = self.store.update_attempt_stage(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            CodingExecutionStage::Coding,
        )?;

        // 创建 rework timeline node（stage=Rework，role=Coder）
        let node = self.create_rework_timeline_node(&updated, rework_round)?;
        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingTimelineNodeCreated { node: node.clone() })
            .await;

        let coder_provider_name = self
            .store
            .get_role_provider_config_snapshot(&updated.project_id, &updated.issue_id, &updated.id)?
            .coder;

        // 组装增量 prompt（依赖 resume session，不重建完整上下文）
        let prompt = build_coding_delta_prompt(&updated, context, Some(&instruction), None);

        let _ = self
            .event_tx
            .send(CodingWsOutMessage::CodingExecutionEvent {
                event: provider_prompt_event(
                    &node.id,
                    &coder_provider_name,
                    prompt.clone(),
                    CodingPromptMode::DeltaConversation.event_detail(),
                ),
            })
            .await;

        let role_run = self.store.create_role_run(
            &updated,
            CodingExecutionStage::Rework,
            CodingProviderRole::Coder,
            CodingRoleRunTrigger::Initial,
            Some(node.id.clone()),
        )?;

        let input = AdapterInput {
            provider_type: provider_type_for_name(&coder_provider_name),
            role: AdapterRole::Coder,
            worktree_path: updated
                .worktree_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string()),
            prompt,
            context_files: Vec::new(),
            output_schema: String::new(),
            timeout: DEFAULT_PROVIDER_TIMEOUT_SECS,
            max_retries: 0,
        };
        let resume_provider_session_id = self.provider_resume_session_id_for_attempt(
            &updated,
            &CodingProviderRole::Coder,
            &coder_provider_name,
        );
        let worktree_path = updated
            .worktree_path
            .clone()
            .ok_or_else(|| CodingWorkspaceEngineError::MissingWorktree(updated.id.clone()))?;
        let mut provider_input = streaming_input_from_adapter(&input, worktree_path.clone());
        provider_input.workspace_session_id = Some(updated.id.clone());
        provider_input.resume_provider_session_id = resume_provider_session_id;
        provider_input.permission_mode =
            role_permission_mode_for_attempt(&self.store, &updated, CodingProviderRole::Coder)?;

        let _full_output = self
            .run_provider_stream_to_completion(CodingProviderStreamRun {
                attempt: &updated,
                node_id: &node.id,
                role_run: Some(&role_run),
                provider,
                legacy_input: &input,
                input: provider_input,
                provider_name: &coder_provider_name,
                provider_role: CodingProviderRole::Coder,
                command_rx,
                allow_legacy_stream_fallback: true,
                timeout: None,
                timeout_reason_code: None,
            })
            .await?;

        self.store.update_role_run_status(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &role_run.id,
            CodingRoleRunStatus::Completed,
            None,
        )?;
        self.complete_timeline_node(
            &updated.project_id,
            &updated.issue_id,
            &updated.id,
            &node.id,
            CodingTimelineNodeStatus::Completed,
            Some(format!("reviewer 返修 round {}", rework_round)),
        )
        .await?;

        self.store
            .get_attempt(&updated.project_id, &updated.issue_id, &updated.id)
            .map_err(CodingWorkspaceEngineError::from)
    }
```

- [x] **Step 2: 检查 CodingPromptMode::DeltaConversation 是否存在**

```bash
grep -rn "DeltaConversation\|CodingPromptMode" src/product/coding_workspace_engine/ | head -10
```

若不存在 `DeltaConversation`，改用 `CodingPromptMode::FullConversation.event_detail()`（delta 标识是可选优化，不影响功能）。

- [x] **Step 3: 编译确认**

```bash
cargo check --locked 2>&1 | head -60
```

若有编译错误，常见原因及修复：

- `CodingExecutionContext` 路径错误 → 确认正确的 use 路径（在 `mod.rs` 中找 `use` 语句）
- `CreateBlockedGateInput` 字段不对 → 查看 `src/product/coding_attempt_store/inputs.rs:47` 确认字段名
- `FindingSeverity` 路径错误 → 用 `review.rs` 中的路径

- [x] **Step 4: 提交**

```bash
git add src/product/coding_workspace_engine/rework.rs
git commit -m "feat(coding): implement execute_reviewer_driven_rework"
```

---

### Task 3：在 runner.rs 的 CodeReview 段接入自动返修逻辑

**Files:**
- Modify: `src/web/coding_ws_handler/runner.rs`（CodeReview 执行完后，根据 verdict 分支）

**Interfaces:**
- Consumes:
  - `execute_reviewer_driven_rework(attempt, review_report, context, provider, command_rx) -> Result<CodingExecutionAttempt>`（Task 2 产物）
  - `review_report.verdict`（`ReviewVerdict::Approve | RequestChanges | Blocked`）
  - `coding_execution_context`（已在 runner.rs 顶部调用，存在 `execution_context` 变量）
  - coder provider（`snapshot.coder`）
- Produces: CodeReview 段完成后的完整分支逻辑

- [x] **Step 1: 定位 CodeReview 段（Plan 1 完成后的 runner.rs）**

```bash
grep -n "execute_code_review_with_commands\|review_report\|ReviewVerdict\|RequestChanges\|Approve" \
    src/web/coding_ws_handler/runner.rs | head -20
```

Plan 1 完成后，CodeReview 段结构应为（删去 analyst 之后）：

```rust
{
    let Some(next) = await_stage_gate(... CodeReview ...).await? else { return Ok(()); };
    current = next;
    let reviewer_provider = provider_for(...)?;
    let review_report = engine.execute_code_review_with_commands(...).await?;
    current = coding_store.get_attempt(...)?;
    // handle_pending_runner_commands ...
    match current.stage {
        ... Coding | Testing | CodeReview => continue 'pipeline,
        ReviewRequest => {}
        _ => return emit_current_session_state(...).await,
    }
    // 后续 ReviewRequest / InternalPrReview ...
}
```

- [x] **Step 2: 在 execute_code_review_with_commands 调用之后插入 verdict 分支**

找到 `review_report` 赋值之后、`match current.stage` 之前的位置，插入：

```rust
            // reviewer 驱动返修：request_changes → 用 coder provider 增量重跑
            if review_report.verdict == ReviewVerdict::RequestChanges {
                let coder_provider_name = coding_store
                    .get_role_provider_config_snapshot(
                        &current.project_id,
                        &current.issue_id,
                        &current.id,
                    )?
                    .coder;
                let coder_provider =
                    provider_for(state, &coder_provider_name, "coding coder provider (rework)")?;
                current = engine
                    .execute_reviewer_driven_rework(
                        &current,
                        &review_report,
                        &execution_context,
                        coder_provider.as_ref(),
                        &mut command_rx,
                    )
                    .await?;
                current =
                    coding_store.get_attempt(&current.project_id, &current.issue_id, &current.id)?;
                if handle_pending_runner_commands(
                    &mut command_rx,
                    coding_store,
                    engine,
                    event_tx,
                    &current,
                )
                .await?
                {
                    return Ok(());
                }
                // blocked（超上限）或 coding（继续循环），统一走 pipeline 分支
                match current.status {
                    CodingAttemptStatus::Blocked | CodingAttemptStatus::WaitingForHuman => {
                        return emit_current_session_state(event_tx, coding_store, &current).await;
                    }
                    _ => continue 'pipeline, // stage=Coding → 回到 Coding 段
                }
            }

            if review_report.verdict == ReviewVerdict::Blocked {
                return emit_current_session_state(event_tx, coding_store, &current).await;
            }
```

- [x] **Step 3: 确认 ReviewVerdict 已引入 use**

```bash
grep -n "ReviewVerdict\|use.*review\|use.*coding_models" src/web/coding_ws_handler/runner.rs | head -10
```

若未引入，在文件顶部 `use` 段加：

```rust
use crate::product::coding_models::ReviewVerdict;
```

- [x] **Step 4: 确认 execution_context 在 CodeReview 段可用**

```bash
grep -n "execution_context\|coding_execution_context" src/web/coding_ws_handler/runner.rs | head -10
```

`execution_context` 在 `runner.rs:143` 附近已绑定（`let execution_context = coding_execution_context(...)?`），在 CodeReview 段（同一 `'pipeline` 循环体内）可直接使用。

- [x] **Step 5: 编译确认**

```bash
cargo check --locked 2>&1 | head -60
```

预期：0 errors。

- [x] **Step 6: 提交**

```bash
git add src/web/coding_ws_handler/runner.rs
git commit -m "feat(coding): wire reviewer-driven rework into CodeReview pipeline"
```

---

### Task 4：编写新返修链路的单元测试

**Files:**
- Modify: `src/product/coding_workspace_engine/tests/provider_driven.rs`（新增测试用例）

**Interfaces:**
- Consumes: `execute_reviewer_driven_rework`（Task 2）、现有 `MockProvider`/`ScriptedProvider` 测试工具

- [x] **Step 1: 查看现有 provider_driven 测试结构**

```bash
head -80 src/product/coding_workspace_engine/tests/provider_driven.rs
```

了解：MockProvider 如何注册、如何触发 `execute_code_review`、如何断言 timeline node 和 rework_count。

- [x] **Step 2: 新增测试：reviewer request_changes 触发一轮自动返修**

在 `provider_driven.rs` 末尾新增：

```rust
#[tokio::test]
async fn test_reviewer_driven_rework_increments_rework_count() {
    // setup: 创建一个 attempt（stage=CodeReview，rework_count=0，max_auto_rework=2）
    // review_report: verdict=RequestChanges，findings=[一条 Error finding]
    // provider: ScriptedProvider 只需返回任意 coder 输出（不验证内容）
    // 调用 execute_reviewer_driven_rework
    // 断言：
    //   - returned attempt.rework_count == 1
    //   - returned attempt.stage == CodingExecutionStage::Coding
    //   - store 中存在一条 CodingReworkInstruction，source_stage=CodeReview
    //   - timeline node stage=Rework，role_run role=Coder

    // 具体实现参考同文件已有的 execute_code_review 测试用例结构
}
```

- [x] **Step 3: 新增测试：超上限时创建 blocked gate**

```rust
#[tokio::test]
async fn test_reviewer_driven_rework_blocks_when_over_limit() {
    // setup: attempt（rework_count=2，max_auto_rework=2）
    // review_report: verdict=RequestChanges
    // 调用 execute_reviewer_driven_rework
    // 断言：
    //   - returned attempt.status == CodingAttemptStatus::Blocked
    //   - store 中存在一条 blocked gate
    //   - rework_count 未变（仍为 2）
}
```

- [x] **Step 4: 运行新测试**

```bash
cargo test --locked --lib coding_workspace_engine::tests::provider_driven 2>&1 | tail -20
```

预期：新增 2 个测试全部 PASS。

- [x] **Step 5: 运行完整测试套件**

```bash
cargo test --locked 2>&1 | grep -E "test result|FAILED"
```

预期：`test result: ok. N passed; 0 failed`

- [x] **Step 6: 提交**

```bash
git add src/product/coding_workspace_engine/tests/provider_driven.rs
git commit -m "test(coding): add reviewer-driven rework unit tests"
```
