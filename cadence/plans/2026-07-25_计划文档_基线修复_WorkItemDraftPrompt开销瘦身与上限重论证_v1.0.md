# Work Item Draft Prompt 开销瘦身与上限重论证 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 让真实规模中文串行 Work Item Draft prompt 稳定低于 12,000-byte 质量预算，将 fail-closed 上限重新论证为 64KB 硬兜底，并修复 prompt 构建失败后 run 节点不落 failed 的悬挂问题。

**Architecture:** 保持 `build_work_item_draft_invocation` 的 fail-closed 语义不变，仅把 `WORK_ITEM_DRAFT_PROMPT_MAX_BYTES` 重定位为拦截病态序列化回归的硬兜底（65,536 B）；质量预算（12,000 B）由真实规模中文 fixture 的确定性测试承担。Prompt 瘦身只删除重复表述、引入简写记号，契约语义逐条保留。run 失败落状态复用既有 `finish_active_run_with_failed_node`。

**Tech Stack:** Rust 2024、Serde JSON、既有 Work Item Split Engine 单元/集成测试、Cargo。

**关联设计：** `cadence/designs/2026-07-25_技术方案_WorkItemDraftPrompt固定开销瘦身与上限重论证_v1.0.md`（含根因量化与缩减前后对比）。本 Plan 取代《2026-07-25_计划文档_基线修复_WorkItemDraft串行上下文预算_v1.0.md》的未完成部分（其 Task 1-2 依赖投影已实施并保留）。

## Global Constraints

- 关联 Change：`improve-work-item-draft-generation-reliability`；本 Plan 修订其 design 中"不放宽 11,000-byte 上限"约束为双层模型（64KB 硬兜底 + 12KB 质量预算）。
- 仅修改 `src/product/work_item_split_engine/prompts.rs`、`src/product/work_item_split_engine/parse.rs`、对应单元测试、`src/web/workspace_ws_handler/run/provider_run.rs`、`src/web/workspace_ws_handler/run/followups.rs`、`tests/it_web/web_work_item_plan_serial/part_01_draft_repair.rs`、`openspec/changes/improve-work-item-draft-generation-reliability/design.md`。
- routing_reference（`src/product/cadence_skills/routing_reference.rs`）全文保留，不得精简。
- 不放宽 Parser、`WorkItemDraftLocalValidator`、接受门禁或 fail-closed 语义；不新增第三方依赖、评估模块、CLI、CI、Hook、Provider 调用或持久化语料。
- Rust 命令必须使用 `--locked` 且不得使用 `-j 1`；定向单元测试用 `cargo test --locked --lib <过滤名>`。
- 此任务改变 Work Item Draft Prompt：确定性验证后，交付前必须提醒操作者按 `cadence/project-rules/work-item-draft-prompt-validation.md` 明确授权 Case A、Case B 各 10 个有效首次输出的真实 Claude Code 验证；未授权不得调用 Provider，不得勾选 `improve-work-item-draft-generation-reliability` 的 3.3 与 4.1。

## 文件结构与职责

| 文件 | 动作 | 职责 |
|---|---|---|
| `src/product/work_item_split_engine/prompts.rs` | 修改 | 上限常量改为 64KB 并注释用途；新增质量预算常量；模板与 runtime contract 瘦身 A–E。 |
| `src/product/work_item_split_engine/parse.rs` | 修改 | too_large 错误文案从硬编码 `11000-byte` 改为引用常量。 |
| `src/product/work_item_split_engine/tests/part_01.rs` | 修改 | 超旧上限可调用、超硬兜底拒绝、真实中文 fixture 质量预算测试。 |
| `src/product/work_item_split_engine/tests/prompt_contract.rs` | 修改 | 硬编码 `11_000` 断言改引用新常量。 |
| `src/web/workspace_ws_handler/run/provider_run.rs` | 修改 | prompt 构建失败时落 failed 节点。 |
| `src/web/workspace_ws_handler/run/followups.rs` | 修改 | 宏内同一 Err 分支落 failed 节点。 |
| `tests/it_web/web_work_item_plan_serial/part_01_draft_repair.rs` | 修改 |  oversized feedback 触发 prompt 构建失败后节点落 failed 的集成回归。 |
| `openspec/changes/improve-work-item-draft-generation-reliability/design.md` | 修改 | 上限约束改写为双层模型并记录论证。 |

### Task 1: 硬兜底上限 64KB 与动态错误文案

**Files:**

- Modify: `src/product/work_item_split_engine/prompts.rs:31-32`
- Modify: `src/product/work_item_split_engine/parse.rs:200-210`
- Modify: `src/product/work_item_split_engine/tests/part_01.rs`
- Modify: `src/product/work_item_split_engine/tests/prompt_contract.rs`
- Test: `src/product/work_item_split_engine/tests/part_01.rs`

**Interfaces:**

- Consumes: 既有 `build_work_item_draft_invocation(&WorkItemPlanOutline, &str, WorkItemGenerationMode, &[WorkItemDraftRecord], Option<&str>) -> ApiResult<WorkItemDraftInvocation>`。
- Produces:
  - `pub(crate) const WORK_ITEM_DRAFT_PROMPT_MAX_BYTES: usize = 65_536;`（硬兜底）
  - `pub(crate) const WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES: usize = 12_000;`（质量预算，供测试引用）
  - too_large 错误 `details` 中 `max_prompt_bytes == 65_536`。

- [ ] **Step 1: 写 RED 测试（超旧上限可调用 + 超硬兜底拒绝）**

在 `src/product/work_item_split_engine/tests/part_01.rs` 中、既有 `single_item_prompt_projects_direct_dependency_within_provider_budget` 之后新增两个测试。复用该测试的 outline/accepted_backend fixture 构造（直接复制其 fixture 代码，不要提取共享函数之外的新抽象）：

```rust
#[test]
fn serial_prompt_above_legacy_11000_limit_remains_invocable_below_hard_backstop() {
    // 与 single_item_prompt_projects_direct_dependency_within_provider_budget 相同的 fixture；
    // 追加 6,000 字节 feedback，使总 prompt 超过旧 11,000 上限但远低于 64KB 硬兜底。
    let feedback = "f".repeat(6_000);
    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_frontend",
        WorkItemGenerationMode::Serial,
        &[accepted_backend],
        Some(&feedback),
    )
    .expect("prompt above the legacy 11000-byte limit must remain invocable below the 64KB hard backstop");
    assert!(
        invocation.prompt.len() > 11_000,
        "fixture must actually exceed the legacy limit: {} bytes",
        invocation.prompt.len()
    );
    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_MAX_BYTES,
        "fixture must stay below the hard backstop: {} bytes",
        invocation.prompt.len()
    );
}

#[test]
fn serial_prompt_above_hard_backstop_is_rejected() {
    let feedback = "f".repeat(70_000);
    let error = build_work_item_draft_invocation(
        &outline,
        "outline_frontend",
        WorkItemGenerationMode::Serial,
        &[accepted_backend],
        Some(&feedback),
    )
    .expect_err("prompt above the 64KB hard backstop must fail closed");
    assert_eq!(error.code, "work_item_draft_prompt_too_large");
    assert_eq!(error.details["max_prompt_bytes"], 65_536);
    assert!(
        error.details["prompt_bytes"].as_u64().expect("prompt_bytes") >= 65_536,
        "details must report the actual prompt size: {}",
        error.details
    );
}
```

- [ ] **Step 2: 运行 RED 验证**

Run:

```text
cargo test --locked --lib serial_prompt_above_
```

Expected: 两个测试均 FAIL——第一个报既有 `work_item_draft_prompt_too_large`（旧上限 11,000），第二个 `max_prompt_bytes` 仍为 `11000`。

- [ ] **Step 3: 实现上限调整与动态文案**

`src/product/work_item_split_engine/prompts.rs:32`：

```rust
/// Fail-closed 硬兜底：只拦截病态序列化回归（如整条持久化记录被注入 prompt）。
/// 质量预算不由本常量承担，见 WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES 的预算测试。
/// prompt 经 stdin JSON 发送给 Provider，无 OS ARG_MAX 约束；真实物理边界是模型上下文窗口。
pub(crate) const WORK_ITEM_DRAFT_PROMPT_MAX_BYTES: usize = 65_536;
/// Draft prompt 质量预算：真实规模中文 fixture 的确定性预算测试阈值。
pub(crate) const WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES: usize = 12_000;
```

`src/product/work_item_split_engine/parse.rs:200-210` 把硬编码文案改为引用常量：

```rust
    if prompt.len() >= WORK_ITEM_DRAFT_PROMPT_MAX_BYTES {
        return Err(ApiError::validation_with_details(
            "work_item_draft_prompt_too_large",
            format!(
                "work item draft prompt exceeds the {}-byte provider-context hard backstop",
                WORK_ITEM_DRAFT_PROMPT_MAX_BYTES
            ),
            json!({
                "prompt_bytes": prompt.len(),
                "max_prompt_bytes": WORK_ITEM_DRAFT_PROMPT_MAX_BYTES,
                "outline_id": current_outline_id,
            }),
        ));
    }
```

`src/product/work_item_split_engine/tests/prompt_contract.rs` 中两处硬编码 `11_000` 断言（约第 74、110 行）改为引用 `WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES`，并更新断言文案为 "quality budget"。同时 `rg -n "11_000|11000" src/` 确认无其他残留引用。

- [ ] **Step 4: 运行 GREEN 与模块回归**

Run:

```text
cargo test --locked --lib serial_prompt_above_
cargo test --locked --lib work_item_split_engine
```

Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add src/product/work_item_split_engine/
git commit -m "fix: rejustify draft prompt limit as 64KB hard backstop"
```

### Task 2: Prompt 固定开销瘦身（A–E）

**Files:**

- Modify: `src/product/work_item_split_engine/prompts.rs`（`work_item_draft_runtime_contract` 与 `build_work_item_draft_prompt` 模板）
- Modify: `src/product/work_item_split_engine/tests/part_01.rs`
- Test: `src/product/work_item_split_engine/tests/part_01.rs`

**Interfaces:**

- Consumes: Task 1 的 `WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES`。
- Produces: 瘦身后的 prompt 仍含 routing_reference 全文、`[canonical_field_contract]` 封闭字段契约（简写记号版）、`[hard_rules]`、`[self_check]`、`[output]` nonce sentinel 段落。

- [ ] **Step 1: 写 RED 测试（真实规模中文 fixture 质量预算）**

在 `src/product/work_item_split_engine/tests/part_01.rs` 新增测试。fixture 模仿 session_0003 真实规模：outline 的 goal/scope/non_goals/verification_intent/handoff_notes 使用中文长文本（goal ~100 汉字；scope 3 条各 ~80 汉字；non_goals 3 条各 ~60 汉字；verification_intent 3 条各 ~90 汉字；handoff_notes ~80 汉字），accepted 直接依赖的 output_contracts 含 3 条各 ~50 汉字 capability、handoff_contract 含 6 个 required_fields：

```rust
#[test]
fn realistic_chinese_serial_prompt_stays_within_quality_budget() {
    // 真实规模中文 fixture（对齐 session_0003 实测：outline JSON ~1.7KB、依赖投影 ~1.1KB）。
    // 阈值 = 质量预算 12,000 内的余量目标 10,500（缩减估值 ~9,700 + 800 余量）；
    // 若实测缩减后规模偏离估值，按（实测 + 800）调整阈值并在 commit message 记录实测值。
    let invocation = build_work_item_draft_invocation(
        &outline,
        "outline_unit_tests",
        WorkItemGenerationMode::Serial,
        &[accepted_module_draft],
        None,
    )
    .expect("realistic serial prompt must stay invocable");
    assert!(
        invocation.prompt.len() < 10_500,
        "realistic Chinese serial prompt must stay within the slimmed margin target: {} bytes",
        invocation.prompt.len()
    );
    assert!(
        invocation.prompt.len() < WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES,
        "quality budget: {} bytes",
        invocation.prompt.len()
    );
    // 契约关键语义必须保留
    assert!(invocation.prompt.contains("[canonical_field_contract]"));
    assert!(invocation.prompt.contains("verification_plan"));
    assert!(invocation.prompt.contains("operational_gate"));
    assert!(invocation.prompt.contains("[cadence_original_routing_rules]"));
    assert!(invocation.prompt.contains("agent-routing-kernel.md"));
    assert!(invocation.prompt.contains("openspec-superpowers-workflow.md"));
    assert!(invocation.prompt.contains("[直接依赖的可消费交接合同]"));
    assert!(invocation.prompt.contains("ARIA_STRUCTURED_OUTPUT"));
}
```

- [ ] **Step 2: 运行 RED 验证**

Run:

```text
cargo test --locked --lib realistic_chinese_serial_prompt_stays_within_quality_budget
```

Expected: FAIL，实际字节数约 11,700（超 10,500 余量目标）。

- [ ] **Step 3: 实施瘦身 A–E**

在 `src/product/work_item_split_engine/prompts.rs` 中：

**A. `[canonical_field_contract]` 改简写记号（约省 950 B）**，整段替换为：

```text
[canonical_field_contract]
封闭类型契约（非示例）：记号 str+=非空 string，[T]=T 数组，obj=object；每个 obj 必须且只能含所列字段，所列字段全部必填，数组可空但元素不得缺/加字段。
- draft: obj{outline_id: str+, logical_work_item_id: str+, canonical_contract: obj, verification_plan: obj}。
- canonical_contract.schema_version: integer literal 1；identity: obj{logical_work_item_id: str+, title: string, kind: backend|frontend|integration|e2e|docs|infra|other}；goal: obj{summary: string}；non_goals: [string]。
- input_contracts: [obj{contract_id: str+, provider_logical_work_item_id: str+, required_capabilities: [string], compatibility_policy: require_all|require_any}]；output_contracts: [obj{contract_id: str+, capabilities: [string]}]。
- tasks: [obj{task_id: str+, statement: string, requirement_refs: [string], done_when_refs: [string]}]；write_policy: obj{exclusive_scopes: [string], forbidden_scopes: [string]}。
- acceptance_criteria: [obj{criterion_id: str+, statement: string, required_evidence: [source_diff|non_zero_test_execution|manual_check|handoff_field]}]。
- verification_checks: [obj{check_id: str+, command: string|null, manual_instruction: string|null, required: boolean, non_zero_test_execution_required: boolean}]；verification_plan: obj{checks: 与 verification_checks 完全相同的数组}。
- handoff_contract: obj{required_fields: 唯一 str+ 数组, provided_contract_refs: 唯一 str+ 数组, reviewer_check_refs: 唯一 str+ 数组}。
- blocker_rules: [obj{reason_code: str+, route: coder_rework|verification_retry|plan_repair_current|plan_repair_upstream|subgraph_replan|story_amendment|design_amendment|operational_gate, target_contract_refs: [string]}]；design_traceability: [obj{source_type: string, source_id: string, requirement_id: string}]。
```

**B. `[hard_rules]` 去重（约省 800 B）**，整段替换为：

```text
[hard_rules]
- 当前仅处于 human-confirmation 之前的候选阶段：必须读取并遵守 writing-plans 的拆分、TDD、验证与交接质量纪律；只将这些纪律体现在本候选中。
- 不得创建 cadence/plans/ 或任何 workspace 文件；不得提前执行 writing-plans 的落盘步骤；canonical writeback 与正式 Plan 落盘由 human-confirmation gate 与 daemon 负责，不得声称已完成。
- 仅在最后一个 nonce sentinel block 返回唯一 Canonical Contract Candidate JSON（不用 Markdown code fence），其 outline_id/logical_work_item_id 对应当前 `{outline_id}`/`{logical_work_item_id}`；draft 只含 [canonical_field_contract] 所列字段。
- 不得修改、新增、删除或重命名 Outline；不得输出 work_item_id、draft_id、status 等后端状态字段；logical_work_item_id 必须与其 identity 一致。
- handoff_contract 是 Canonical singleton；required_fields、provided_contract_refs、reviewer_check_refs 均非空且不重复。
- verification command 必须来自目标仓库的可信证据，不得根据 WorkItemKind 推导；证据不足进入 manual/repair/blocker，绝不使用 Aria 当前仓库命令兜底。
- 不得输出面向 Coder 的长篇 implementation_context；不要提前生成或渲染 Coder Projection 或 Reviewer Projection。
```

（删除与 field_contract/self_check 重复的 3 条：verification_plan.checks 逐字段复制、canonical_contract 必须且只能包含 schema_version 及所列字段、verification_plan 只能包含 checks；输出唯一性并入第三条。）

**C. `work_item_draft_runtime_contract` 的 `[superpowers_contract]` 段去重（约省 400 B）**，该段替换为：

```text
[superpowers_contract]
遵守 using-superpowers、writing-plans、TDD 与验证纪律；只生成候选，不执行代码修改。TDD 与验证闭环必须在当前项 exclusive_write_scopes 和已完成 depends_on handoff 下实际可执行，不得把后续 Work Item 才会提供的注册、接线、生成或部署作为前提。command 仅可来自目标仓库可信证据，不得根据 WorkItemKind 推导；证据不足用 manual/repair/blocker。每项必须可由单个 Claude Code/Codex 会话完成，estimated_context_tokens 不得超过 50k。
```

（删除"候选必须给出可执行目标、范围、非目标、结构化验证方案、依赖、交接和风险"——已由 field_contract 覆盖。）

**D. `[registration]`/`[projection]`/`[self_check]` 微压缩（约省 350 B）**：registration 改为单句"内部登记 acceptance criterion ID、traceability requirement ID、input/output contract ID 与上列可信命令；不输出该登记表。"；projection 删除"不得引用 verification check、task 或其他 ID"（与"完全一致"重复）；self_check 删除"以及 required command 的非空和目录成员关系"中的重复修饰，保留集合关系与空目录 operational_gate 语义全文。

**E. `[canonical_projection]` 四条并两条（约省 190 B）**：

```text
[canonical_projection]
- Draft 专有 Canonical projection 优先于 [allowed_outputs] 的通用表述；目标、范围和非目标映射到 identity、goal、write_policy、non_goals；TDD 与验证映射到 tasks、acceptance_criteria、verification_checks。
- 依赖、交接和风险映射到 input_contracts、output_contracts、handoff_contract、blocker_rules；不得输出 writing-plans 的 Markdown Plan 或新增 JSON 字段。
```

注意：Rust `format!` 模板中字面 `{`/`}` 必须写为 `{{`/`}}`；简写记号版 field_contract 含大量花括号，替换时逐一转义。

- [ ] **Step 4: 运行 GREEN 与契约回归**

Run:

```text
cargo test --locked --lib realistic_chinese_serial_prompt_stays_within_quality_budget
cargo test --locked --lib work_item_split_engine
cargo test --locked --lib prompt_contract
```

Expected: 全部通过；既有契约测试（字段白名单、nonce sentinel、catalog 边界）无回归。若有既有测试断言了被删除的重复文案，按"语义保留"原则更新该断言指向保留段落，并在 commit message 说明。

- [ ] **Step 5: Commit**

```bash
git add src/product/work_item_split_engine/
git commit -m "refactor: slim draft prompt fixed overhead within quality budget"
```

### Task 3: prompt 构建失败时 run 节点落 failed

**Files:**

- Modify: `src/web/workspace_ws_handler/run/provider_run.rs:463-471`
- Modify: `src/web/workspace_ws_handler/run/followups.rs:565-578`
- Test: `tests/it_web/web_work_item_plan_serial/part_01_draft_repair.rs`

**Interfaces:**

- Consumes: 既有 `WorkspaceEngine::finish_active_run_with_failed_node(message).await`（`src/product/workspace_engine/session_state/timeline.rs:32`），会把 active 节点更新为 `TimelineNodeStatus::Failed` 并走 `finish_failed_run`。
- Produces: prompt 构建失败时，draft run 节点状态 = `failed`，WS 客户端收到错误消息，不再悬挂 `active`。

- [ ] **Step 1: 写 RED 集成测试**

在 `tests/it_web/web_work_item_plan_serial/part_01_draft_repair.rs` 末尾新增（fixture 与 WS 辅助函数复用本文件既有模式）：

```rust
#[tokio::test]
async fn serial_oversized_feedback_fails_draft_run_node() {
    let _guard = WS_TEST_LOCK.lock().await;
    let _test_guard = enable_test_controls().await;
    let (app, _root, _prompts) = app_with_confirmed_story_and_design_and_streaming_outputs(vec![
        valid_outline_output(),
        valid_draft_output("outline_backend_session"),
    ])
    .await;
    let (_session_id, _plan_id, mut ws) = prepare_plan_accept_outline_and_select_serial(&app).await;

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["type"] == "timeline_node_created"
                && message["node"]["node_type"] == "work_item_draft_confirm"
        })
    })
    .await;

    // 70KB feedback 使 prompt 超过 64KB 硬兜底，构建必然 fail-closed。
    let oversized = "超".repeat(24_000);
    ws.send(Message::Text(
        json!({
            "type": "work_item_draft_decision",
            "outline_id": "outline_backend_session",
            "decision": "rewrite",
            "feedback": oversized
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send oversized rewrite");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(10), |messages| {
        messages.iter().any(|message| {
            message["node"]["node_type"] == "work_item_draft_run"
                && message["node"]["status"] == "failed"
        })
    })
    .await;
    assert!(
        messages.iter().any(|message| {
            message["type"] == "error"
                && message["message"]
                    .as_str()
                    .is_some_and(|text| text.contains("provider-context"))
        }),
        "client must receive the prompt-too-large error: {messages:#?}"
    );

    ws.close(None).await.ok();
}
```

（若 WS 消息中节点状态字段名不是 `node.status` 或更新消息类型不同，先读 `src/web/workspace_ws_handler/` 的 outbound 映射确认实际字段，再按实际形状断言；断言目标不变：draft run 节点落 failed + 收到含 `provider-context` 的错误。）

- [ ] **Step 2: 运行 RED 验证**

Run:

```text
cargo test --locked --test it_web serial_oversized_feedback_fails_draft_run_node -- --nocapture
```

Expected: FAIL——节点停在 `active`，recv 超时；证明当前 Err 分支不落 failed。

- [ ] **Step 3: 实现失败落状态**

`src/web/workspace_ws_handler/run/provider_run.rs` 的 Err 分支（约 463-471 行）把：

```rust
Err(message) => {
    engine.mark_active_run_finished(&run_label);
    drop(engine);
    let err = WsOutMessage::Error { message };
    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
    return;
}
```

改为：

```rust
Err(message) => {
    engine
        .finish_active_run_with_failed_node(message.clone())
        .await;
    drop(engine);
    let err = WsOutMessage::Error { message };
    let _ = send_json_outbound(&outbound_tx_for_task, &err).await;
    return;
}
```

`src/web/workspace_ws_handler/run/followups.rs` 宏内同一 Err 分支（约 565-578 行）做同样替换（保留其后 `clear_active_run_if_token(...).await` 调用不变）。

- [ ] **Step 4: 运行 GREEN 与串行回归**

Run:

```text
cargo test --locked --test it_web serial_oversized_feedback_fails_draft_run_node -- --nocapture
cargo test --locked --test it_web web_work_item_plan_serial
```

Expected: 全部通过。

- [ ] **Step 5: Commit**

```bash
git add src/web/workspace_ws_handler/run/ tests/it_web/web_work_item_plan_serial/
git commit -m "fix: fail draft run node when prompt build fails"
```

### Task 4: OpenSpec 修订、质量门禁与真实验证授权提醒

**Files:**

- Modify: `openspec/changes/improve-work-item-draft-generation-reliability/design.md`
- Modify: `openspec/changes/improve-work-item-draft-generation-reliability/tasks.md`

- [ ] **Step 1: 修订 design.md 上限约束**

把 design.md 中"不放宽 11,000-byte 上限"相关约束/风险条目改写为双层模型，引用论证依据：

```markdown
- Draft Prompt 上限采用双层模型：64KB（65,536 B）fail-closed 硬兜底只拦截病态序列化回归（prompt 经 stdin JSON 发送，无 ARG_MAX 约束；物理边界为模型上下文窗口）；12,000 B 质量预算由真实规模中文 fixture 的确定性预算测试承担。论证见 `cadence/designs/2026-07-25_技术方案_WorkItemDraftPrompt固定开销瘦身与上限重论证_v1.0.md`。
```

- [ ] **Step 2: 运行最终质量门禁**

Run:

```text
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo test --locked --lib --quiet
cargo test --locked --test it_web web_work_item_plan_serial
openspec validate improve-work-item-draft-generation-reliability --strict
git diff --check
```

Expected: 全部通过。

- [ ] **Step 3: 请求真实 Claude Code 试运行授权（不自动调用）**

向操作者发送以下提醒并等待明确授权：

> 本次改动涉及 Work Item Draft Prompt 或其结构化契约。建议按 `cadence/designs/2026-07-24_技术方案_WorkItemDraftPrompt测试基线_v1.0.md` 执行 Case A 与 Case B 各 10 个有效首次输出的 Claude Code 验证；是否授权执行？

未获授权时，停止在确定性验证状态；不得调用 Provider，也不得勾选 `improve-work-item-draft-generation-reliability` 的 3.3 或 4.1。

- [ ] **Step 4: 仅按获得的证据更新 OpenSpec tasks**

只有 Step 2 全部通过后，勾选 tasks.md 中已由确定性证据覆盖的条目；3.3/4.1 仅在操作者授权且 Case A、Case B 各 10/10 首次输出通过后勾选，否则保留未完成并在交付中说明。

## Plan Self-Review

- 覆盖：Task 1 修复 session_0003 超旧上限报错（11,711 B prompt 在 64KB 硬兜底下可调用）；Task 2 落实设计 §1 瘦身 A–E 并以真实中文 fixture 锁定质量预算；Task 3 修复附带发现的 run 悬挂问题；Task 4 完成 OpenSpec 契约修订与授权门禁。
- 类型一致性：常量名 `WORK_ITEM_DRAFT_PROMPT_MAX_BYTES` 保持既有导出名；新增 `WORK_ITEM_DRAFT_PROMPT_QUALITY_BUDGET_BYTES` 在 Task 1 定义、Task 2 消费；`finish_active_run_with_failed_node` 为既有公开方法，签名与 Task 3 用法一致。
- 边界：routing_reference 不动；Parser/Validator/接受门禁不变；Case A/B 未授权不执行；不新增依赖。
