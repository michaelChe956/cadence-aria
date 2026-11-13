# WIG A4/A3 只读定位结果

> 重要前提更正：题面给的路径 `src/product/workspace_engine/run/single_candidate.rs` 等在 worktree `feat-wig-optimize-11-08` 中**不存在**。实际落点（按行数匹配）如下：
> - `run/single_candidate.rs`(1199) → `src/web/workspace_ws_handler/run/single_candidate.rs`
> - `structure_reredrive.rs`(865) → `src/web/workspace_ws_handler/run/single_candidate_structure_reredrive.rs`
> - `sc_compile_teaching.rs`(566) → `src/product/workspace_engine/prompts/sc_compile_teaching.rs`
> - `sc_revision.rs`(181) → `src/product/work_item_split_engine/prompts/sc_revision.rs`
> - `preflight.rs`(1180) → `src/web/workspace_ws_handler/run/single_candidate_compile_preflight.rs`

---

## Q1 SC 终局失败的决定点

**层**：web WS handler 的 provider-run 层；**函数**：`run_single_candidate_author`（`src/web/workspace_ws_handler/run/single_candidate.rs:701`，循环体 `let candidate_item_count = loop {` 在 `:833`）。

三个终局失败决定点（同函数内）：

| 失败类 | 决定点 | 说明 |
|---|---|---|
| compile 失败（parse/lowering） | `single_candidate.rs:901-916`（消息拼装 `:901`，`persist_single_candidate_terminal_phase(Failed)` `:912`，`Err(Message)` `:915`） | 在 `grant == None` 分支 |
| IR 预检失败 | `single_candidate.rs:989-1004`（persist `:1000`，return `:1003`） | 在 `grant == None` 分支 |
| 候选结构门拒收 | `single_candidate.rs:1064-1079`（persist `:1075`，return `:1078`） | Denied/Drift 分支 |
| （附带）引擎直接 Err | `single_candidate.rs:1086-1089` | `complete_single_candidate_work_item_plan_author` 返回 Err |

**该点作用域内实际有的数据**：
- **轮次集合：无。** 只有 `first_round_failure: Option<String>`（`single_candidate.rs:832`），保存的是**第一轮失败类的一段已 join 文本**，不是轮次集合；中间轮无留存。
- **逐轮 blocking_reasons：无。** 终局消息只拼「第一轮串（`first_round_failure`）+ 最后一轮串（`reasons.join("; ")` / `preflight.presentation_reasons().join("; ")` / 结构门固定文案 `structure_gate_first_round_failure()`）」。当前轮的 `diagnostics: Vec<CompilerDiagnostic>`（compile 类）与 `preflight: PreflightVerdict`（IR 类）在该分支仍在作用域。
- **provider 原始输出：不在作用域。** `full_output` 已被 move（`single_candidate.rs:828` `let mut compile_source = full_output;`）且已是归一化后文本；raw 文本只在 split run 里落盘（见 Q3）。
- **重驱 prompt：不在作用域**（`reredrive_prompt` 只活在 `'teach:` 块与 `if let Some((attempt, reredrive_prompt))` 块内，`:869`/`:880`/`:967`/`:1038`）。仅 `prompt_ref`（构造器名）与 `prompt_digest`（hash）进了教学窗口记录。
- **已编译 source：compile 类与 IR 类在作用域（`compile_source: String`，= 该轮归一化后的完整 markdown）；结构门类不在**（`:1012` 已把 `compile_source` move 进 `complete_single_candidate_work_item_plan_author`），结构门只能拿到 ref（`failure.source_revision_ref`）。

**该点的持久化写出路径**：
- **事件/执行事件**：`WorkspaceEngine::emit_execution_event`（`src/product/workspace_engine/provider_drive/work_item_plan.rs:465`）→ 落 timeline node detail（`update_node_detail` `src/product/workspace_engine/session_state/timeline.rs:330` → `persist_timeline_nodes` `:569` → `store.save_timeline_nodes`）+ 广播 `EngineEvent::ExecutionEvent`。**终局失败本身不发射执行事件**；事件只发生在 prompt（`:339`/`:394`）、归一化（`:193`/`:234`）、契约指令（`:319`）处。
- **工件/artifact**：终局失败路径**不写** artifact。`update_artifact` 只在成功路径（`src/product/workspace_engine/single_candidate.rs:875-878`）。
- **会话记录（session record）**：
  - 相位/状态：`engine.persist_single_candidate_terminal_phase`（`src/product/workspace_engine/single_candidate.rs:75`）→ `lifecycle.compare_and_save_single_candidate_phase` → `single_candidate_phase=Failed` + `status=Failed`。
  - 策略诊断：`engine.record_single_candidate_compile_failure`（`src/product/workspace_engine/compile/single_candidate.rs:1147`）→ `lifecycle.compare_and_save_policy_route` → `session.policy_diagnostics`。
  - 教学窗口关闭：`lifecycle.close_sc_teaching_window(..., FailedTerminal)` — `deny_and_close`（`src/web/workspace_ws_handler/run/single_candidate_structure_reredrive.rs:215`）；窗口记录落 `session.sc_generation_teaching_windows`。
- **追加式尝试流水（独立文件）**：`lifecycle.append_candidate_attempt`（`src/product/lifecycle_store/candidate_attempts.rs:285`）→ `issues/{issue}/candidate-attempts.jsonl`（`CANDIDATE_ATTEMPTS_FILE_NAME` `:74`）。generate 段写点：`record_generate_segment_failure`（`single_candidate_structure_reredrive.rs:518`），经 `record_generate_segment_compile_failure` `:567` / `record_generate_segment_ir_validation_failure` `:591`。
- **失败 timeline 节点**：arm 层 `engine.finish_active_run_with_failed_node(message)`（`src/web/workspace_ws_handler/run/provider_run/single_candidate_arm.rs` 的 `Message` 分支）→ `src/product/workspace_engine/session_state/timeline.rs:54`；随后 `WsOutMessage::Error{message}` 广播。
- **结构门类额外**：`route_author_structure_gate_failure`（`src/product/workspace_engine/single_candidate_structure_failure.rs:336`）内 `self.record_candidate_compile_failure(&failure)`（`:358`）→ `save_candidate_compile_status` + 追加尝试流水。

---

## Q2 `failure_source_ref`

- **类型定义**：`src/product/work_item_plan_policy/types.rs:476` — `pub failure_source_ref: Option<String>`，所属结构 `ScTeachingWindowRequest`（`:471`，doc `:469`「除 failure_source_ref 外全部必填」）。`#[serde(default, skip_serializing_if="Option::is_none")]`。
- **生产点（唯一）**：`src/web/workspace_ws_handler/run/single_candidate_structure_reredrive.rs:95` `sc_teaching_failure_request(...)`，其中 `:106-107`：
  ```rust
  // 初次可教学失败处尚无 durable 候选 source 引用 ⇒ 显式 None。
  failure_source_ref: None,
  ```
  其余出现处（`src/product/work_item_plan_policy/types.rs` 之外）全部是测试 fixture（`workspace_sc_teaching_*_tests.rs`、`tests_types.rs`、`single_candidate/teaching_window_*`）。
- **消费点**：**无任何生产消费**。字段仅被 `request.clone()` 带进窗口记录（`src/product/lifecycle_store/workspace_sc_teaching_window.rs:159` `request: request.clone()` → 落 session 记录），但：
  - `sc_teaching_frozen_input_digest`（`types.rs:640-651`）只拼 `business_request_digest / material_digests / context_digest / failure_source_sha256` — **不含** `failure_source_ref`；
  - `evaluate_sc_teaching_approval`（`types.rs:653`）判据④只查 `failure_source_sha256 / failure_typed_kinds / context_digest / prompt_digest`；
  - `verify_frozen_inputs`（`types.rs:628`）只比 digest + `prompt_ref` + `prompt_digest`。
  → 字段是「写进去没人读」，恒 None 无任何副作用。
- **为什么恒 None（代码事实）**：
  1. 唯一生产点被固定在「初次可教学失败」这一位置，且显式写死 `None`（`single_candidate_structure_reredrive.rs:107`）；三个调用点（`src/web/workspace_ws_handler/run/single_candidate.rs:859`、`:949`、`:1024`）都没有把 ref 传进来——第三个调用点手里**有** `failure.source_revision_ref`（结构门类已 seal，`single_candidate_structure_failure.rs:348-354`），但组装请求时未取用。
  2. compile/IR 失败类在那一刻**确实没有** durable 候选 source 引用：converge 失败即 return，`put_source_revision` 从未执行（`src/product/workspace_engine/single_candidate.rs:625-640`）；生成段流水行的 `source_revision_ref` 也恒 `None`（`candidate_attempt_record_from_generate_segment` 调用点传 `None`，`single_candidate_structure_reredrive.rs:518` 段内）。

---

## Q3 失败 candidate 的 source 文本持久化

三个互不相同的落点：

**(a) durable source revision（内容寻址，结构化文件）**
- 写：`WorkItemPlanSourceStore::put_source_revision` — `src/product/work_item_plan_source_store.rs:182`。
- 路径：`record_path`（`:600-616`）→ `issues/{issue_id}/work-item-plan-sources/{plan_id}/source_revision/{id}.json`（kind 常量 `SOURCE_REVISION_KIND` `:16`）；canonical ref 形如 `project/{p}/issue/{i}/plan/{plan}/source_revision/{id}`（`canonical_ref` `:739`）。
- SC 写点：`src/product/workspace_engine/single_candidate.rs:664`（在 `complete_single_candidate_work_item_plan_author`，`:588`）。
- **关键事实：只在 converge 成功后写** ⇒ compile 类 / IR 预检类失败轮的 source **从不落此**。
- 读：`WorkItemPlanSourceStore::get_source_revision(scope, canonical_ref)` — `:229`（含 hash/content_hash 复验）。SC 读点：`src/product/workspace_engine/compile/single_candidate.rs`（final compile 装载，`get_source_revision(&scope,&source_ref)`）、`src/web/workspace_ws_handler/run/single_candidate.rs:629`（fresh 返修装载上一版全文）。

**(b) provider 原始输出全文（JSON 文件）**
- 写：`complete_work_item_split_provider_run_in_admission` — `src/product/work_item_split_engine/parse.rs:752-777`，`write_json(&dir.join("structured_output.json"), structured_output)`。
- 路径：`work_item_split_provider_run_dir`（`parse.rs:650-662`）→ `work-item-split-provider-runs/{session_id}/{role_run_seq}/`；同目录 `run.json` 存 `provider_run_id`（`ws-{session}-split-run-{seq}`）、`status`、`prompt_chars`（**只有字符数，prompt 全文不落此处**）、`structured_output_ref`。
- 调用链：`close_plan_split_run_with_markdown`（`src/web/workspace_ws_handler/run/gateway_start.rs:551-562`，把 `full_output` 包成 `serde_json::Value::String`）← 调用点 `src/web/workspace_ws_handler/run/single_candidate.rs:803`（首轮）与 `:141`（教学重驱轮）。
- 读取者：目前**没有**生产读点（只有 `read_work_item_split_provider_run_status` `parse.rs:825` 读 run.json 状态，不读 structured_output）。

**(c) 追加式尝试流水（只留引用与缺失原因，不留全文）**
- `CandidateAttemptRecord` — `src/product/lifecycle_store/candidate_attempts.rs:184`；字段 `source_revision_ref` `:196`、`plan_candidate_ir_ref` `:199`、`source_missing_reason` `:224`、`failure`（`CandidateAttemptFailure` `:154`：failure_id/code/typed_kinds/cycle_fingerprint）。
- generate 段（compile/IR 类）行：`candidate_attempt_record_from_generate_segment` `:627`，`source_revision_ref` 传 `None`，`source_missing_reason` 装 detail + A1 的 `[sc_parse_fact_audit]` 审计分区（`single_candidate_structure_reredrive.rs:518` 段内）。
- 结构门类行：`candidate_attempt_record_from_failure` `:519`，`source_revision_ref` 来自 `status.source_revision_ref`（**有值**）。
- 落盘：`append_candidate_attempt` `:285` → `issues/{issue}/candidate-attempts.jsonl`。

**下一轮重驱能否按 id 读回完整文本？**
- **结构门类：能。** source revision 已落盘（`single_candidate.rs:664` 先于结构门判定执行），ref 在 `failure.source_revision_ref`（`single_candidate_structure_failure.rs:88-92`、`:351`）与尝试行 `source_revision_ref`；按 ref + `SourceStoreScope` 调 `get_source_revision` 即得全文。既有读回样板：`src/web/workspace_ws_handler/run/single_candidate.rs:598-645`。
- **compile/IR 类：不能（无 source revision id）。** 只能从 split run 的 `structured_output.json` 取 provider 原始全文；但那是 raw 文本（不是归一化后的 `compile_source`），且当前代码**没有**把 `role_run_seq` 与「失败轮」绑定的 durable 索引（`provider_attempt` 关联只到窗口 attempt 级别，`generate_segment_provider_attempt` `candidate_attempts.rs:672`）。

---

## Q4 教学重驱 prompt 的组装点

三条路径，入口与入参结构：

1. **compile 类**：`build_typed_compile_reredrive_prompt(payload: &TeachingPayload, autorepair_instructions: &[String], registered_requirement_ids: &[String]) -> String` — `src/web/workspace_ws_handler/run/single_candidate_compile_preflight.rs:817`；调用点 `src/web/workspace_ws_handler/run/single_candidate.rs:854`。
   → 委派 `crate::product::workspace_engine::build_work_item_plan_compile_reredrive_prompt(parse_reasons: &[String], lowering_faults: &[LoweringFault], ir_faults: &[IrFault], autorepair_instructions: &[String], registered_requirement_ids: &[String]) -> String` — `src/product/workspace_engine/prompts/sc_compile_teaching.rs:70`
   → 渲染本体 `render_teaching_reredrive_prompt(facts: &TeachingReredriveFacts<'_>, registered_requirement_ids: &[String]) -> String` — `sc_compile_teaching.rs:89`。
   `TeachingReredriveFacts`（`sc_compile_teaching.rs:39-53`）字段：`parse_reasons: &'a [String]` / `lowering_faults: &'a [LoweringFault]` / `ir_faults: &'a [IrFault]` / `autorepair_instructions: &'a [String]` — **无 source 字段**。
2. **IR 预检类**：`build_preflight_ir_reredrive_prompt(preflight: &PreflightVerdict, registered_requirement_ids: &[String]) -> String` — `preflight.rs:773`；调用点 `single_candidate.rs:948`。内部走同一 compile 模板（`preflight.failure_reasons` + `preflight.ir_faults`）+ `append_ir_repair_instructions`（`preflight.rs:790`）。
3. **结构门类**：`author_structure_repair_prompt(failure: &PlanningCompileFailure) -> String` — `src/product/workspace_engine/single_candidate_structure_failure.rs:112`；调用点 `single_candidate.rs:1024`。纯字符串拼装（失败原因 + 两个修法 + 保护项 + `REVISION_TEACHING`），**不带 source**。

**注入「上一版完整 source」的注入位置与可得性**：
- compile 类：注入点应在 `render_teaching_reredrive_prompt`（`sc_compile_teaching.rs:89`，在「具体失败原因」块之前/之后）或给 `TeachingReredriveFacts` 加字段。调用点 `single_candidate.rs:854` 处**拿得到上一轮完整 source 文本**：`compile_source: String`（该轮失败的归一化 markdown）；同一处 `:861` 已在算 `sc_sha256(&compile_source)`。
- IR 类：同上，`:948` 处 `compile_source` 在作用域。
- 结构门类：`:1024` 处 `compile_source` 已被 move（`:1012`），**拿不到文本**；只有 ref（`failure.source_revision_ref`）→ 需按 ref 回读（样板见下）。
- **既有「上一版完整 source 自含教学」样板（可直接套）**：`build_work_item_plan_markdown_fresh_revision_prompt(request, issue, repository, context, review, previous_source: &str)` — `src/product/work_item_split_engine/prompts/sc_revision.rs:69`；把上一版完整 source 以 ```` ```markdown ```` 围栏块 `[previous_source]` 注入（`:84-92`），并受 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES` 硬预算（`:100-106`）。其数据来源正是「按 source ref 装载上一版全文」：`src/web/workspace_ws_handler/run/single_candidate.rs:598-645`（`previous_source_for_fresh_revision`，`:629` 调 `get_source_revision`）。

---

## Q5 `preflight.rs` 守的预算

文件：`src/web/workspace_ws_handler/run/single_candidate_compile_preflight.rs`（1180 行）。

- **该文件本身没有字节预算常量**；它守的是**条数上限**：
  - `MAX_AUTOREPAIR_INSTRUCTION_LINES: usize = 4` — `src/product/workspace_engine/contract_autorepair.rs:88`（单点定义，全链共用）。
  - 截断函数 `cap_autorepair_instruction_lines(lines: Vec<String>) -> Vec<String>` — `contract_autorepair.rs:93-102`（取前 4 条，超限补一行「共 N 条」计数摘要）。
  - preflight 侧用法：`compile_failure_surface(source, diagnostics, context) -> (Vec<String>, Vec<String>)` — `preflight.rs:299-317`（reasons 面已截断；instruction_lines 返回**未截断**原串，由 prompt 侧截断）。
- **字节预算在同一链的另一个文件**：`MAX_PARSE_FACT_BLOCK_BYTES: usize = 32_000` — `src/product/workspace_engine/prompts/sc_compile_teaching.rs:31`（只约束 parse 事实块）。
  - **字节数取自**：`reason.len()`（每条诊断事实块字符串的 UTF-8 字节）+ `"- ".len()` + 1，累加变量 `fact_block_bytes`，判定 `fact_block_bytes + rendered_bytes > MAX_PARSE_FACT_BLOCK_BYTES` → **整条省略**（不切半条），末尾记 omitted 计数 — `sc_compile_teaching.rs:112-141`。
  - 条数上限复用 `MAX_AUTOREPAIR_INSTRUCTION_LINES`（`sc_compile_teaching.rs:109`）。
- 相邻的其它字节预算（供 A3 注入时对齐）：
  - `WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES: usize = 262_144` — `src/product/work_item_split_engine/prompts.rs:39`；字节取自 `prompt.len()`（`sc_revision.rs:100-106`、`prompts.rs:361`）。**这是「注入上一版完整 source」时最相关的一条硬预算**。
  - `MARKDOWN_AUTHOR_PROMPT_MAX_BYTES = 6_000` / `MARKDOWN_AUTHOR_DELTA_REFERENCE_MAX_BYTES = 1_200` — `src/product/workspace_engine/prompts.rs:148`/`:154`。
  - `SC_MANUAL_REVISION_PROMPT_QUALITY_BUDGET_BYTES = 32_000`、`SC_MANUAL_REVISION_FEEDBACK_MAX_BYTES = 8_192` — `prompts/human_gate_revision.rs:10`/`:13`。
  - `SINGLE_CANDIDATE_REVIEW_PROMPT_MAX_BYTES = 96*1024` — `prompts/review_context.rs:706`。

---

## Q6 事件发射路径

- **准确调用入口**：`WorkspaceEngine::emit_execution_event(&mut self, event: ProviderExecutionEvent, node_id: Option<String>, agent: Option<ProviderName>)` — `src/product/workspace_engine/provider_drive/work_item_plan.rs:465`（固定三参；全仓 49 个调用点）。
- **类型**：`ProviderExecutionEvent` — `src/cross_cutting/streaming_provider/mod.rs:1133`；`ProviderExecutionEventKind`（`:1112`：Provider|Turn|Command|Output|Artifact|Usage）；`ProviderExecutionEventStatus`（`:1123`）。
- **落盘 + 广播**：node detail 持久化（`update_node_detail` `src/product/workspace_engine/session_state/timeline.rs:330` → `persist_timeline_nodes` `:569` → `store.save_timeline_nodes`）+ `EngineEvent::ExecutionEvent` 广播（`work_item_plan.rs:499`）。事件按 `event_id` upsert 覆盖（`upsert_execution_event_json`）。
- **durable 诊断（非事件）入口**：`record_single_candidate_compile_failure`（`src/product/workspace_engine/compile/single_candidate.rs:1147`）→ session `policy_diagnostics`；`record_candidate_compile_failure`（`:222`）→ `candidate_compile_status` + 尝试流水。

---

## 未找到项清单

1. **「A4 修复包」现成载体：未找到。** 搜索路径 `src/product/workspace_engine/`（`plan_repair_artifacts.rs`、`controls.rs`、`lifecycle.rs`）、`src/product/lifecycle_store/`；关键词 `repair_package`、`failure_package`、`修复包`、`ScFailurePackage`。仓内唯一近似的「修复包」是 `ArtifactPayload::PlanAmendmentManifest` / `WorkItemRevisionHistory`（`src/product/workspace_engine/plan_repair_artifacts.rs:68`/`:49`），但它们属 **plan repair（编码侧）**，与 SC 生成失败无关；`ArtifactPayload` 枚举（`src/web/workspace_ws_types.rs`）中无 SC 失败包变体。
2. **「携带 source 引用」的既有执行事件：未找到。** 遍历 49 个 `emit_execution_event` 调用点，grep `source_revision_ref` / `source_ref` 均无命中；`ProviderExecutionEvent` 字段只有 `event_id/kind/status/title/detail/command/cwd/output/exit_code`（`src/cross_cutting/streaming_provider/mod.rs:1133`），无结构化 source 引用槽位。
3. **「历轮全部 blocking_reasons」的 durable 留存：未找到。** 搜索 `src/web/workspace_ws_handler/run/single_candidate*.rs`、`src/product/lifecycle_store/candidate_attempts.rs`；关键词 `round`、`rounds`、`attempts_history`、`轮次`。仓内只有 `first_round_failure: Option<String>`（单串）与每次尝试一行流水（行内只有该轮的 codes/typed_kinds/detail）。
4. **失败轮的 provider 原始输出的 durable 索引（哪一轮对应哪个 role_run_seq）：未找到。** `structured_output.json` 有 run_id，但没有任何代码把「SC 失败轮」与 `role_run_seq` 关联落盘；`generate_segment_provider_attempt`（`candidate_attempts.rs:672`）只关联到教学窗口 attempt，不含 run 序号。
5. **`failure_source_ref` 的生产消费点（除测试）：未找到消费者。** 全仓 grep `failure_source_ref` 仅命中 `types.rs:476`（定义）、`single_candidate_structure_reredrive.rs:107`（写死 None）、以及若干测试 fixture。

---

## 可直接复用的既有样板清单

| 函数 | 路径:行 | 作用 |
|---|---|---|
| `emit_execution_event` | `src/product/workspace_engine/provider_drive/work_item_plan.rs:465` | 唯一的执行事件入口（落 node detail + 广播） |
| `emit_provider_prompt_event` | `src/product/workspace_engine/provider_drive/work_item_plan.rs:632` | 作者 prompt 事件（专用封装） |
| `drive_compile_teaching_reredrive` 内的事件样板 | `src/web/workspace_ws_handler/run/single_candidate_structure_reredrive.rs:339-368` | 发一条携带**完整 prompt 文本**（`output: Some(prompt)`）的事件，`event_id={node_id}_prompt_{event_key}`，`kind=Output`/`status=Started` |
| `drive_structure_gate_reredrive` 内的事件样板 | `single_candidate_structure_reredrive.rs:394-425` | 同形，`event_id={node_id}_prompt_structure_reredrive` |
| `emit_contract_autorepair_instructions_event` | `src/web/workspace_ws_handler/run/single_candidate_compile_preflight.rs:319-357` | 携带**失败原因/机器指令**的事件（`output` = 指令文本，`detail` = 条数摘要） |
| `emit_author_heading_normalized_event` / `emit_author_ears_normalized_event` | `preflight.rs:193-231` / `:234-275` | 携带诊断计数的事件；`event_id` 内容寻址（`author_normalization_event_id` `:175`） |
| `record_generate_segment_failure` | `single_candidate_structure_reredrive.rs:518` | generate 段失败流水行写入（stage/code/typed_kinds/detail/A1 审计分区），写失败降级为 `candidate_attempt_log_write_failed` |
| `candidate_attempt_record_from_generate_segment` | `src/product/lifecycle_store/candidate_attempts.rs:627` | 流水行构造器（source_ref/missing_reason/provider_attempt 三槽位） |
| `append_candidate_attempt` | `candidate_attempts.rs:285` | 流水追加（issue 级 jsonl，幂等） |
| `persist_single_candidate_terminal_phase` | `src/product/workspace_engine/single_candidate.rs:75` | 终态相位/状态落 session 记录 |
| `record_single_candidate_compile_failure` | `src/product/workspace_engine/compile/single_candidate.rs:1147` | durable 诊断（session.policy_diagnostics） |
| `record_candidate_compile_failure` | `compile/single_candidate.rs:222` | 候选编译事实（status + 尝试流水） |
| `close_sc_teaching_window(..., FailedTerminal)` / `deny_and_close` | `src/product/lifecycle_store/workspace_sc_teaching_window.rs:459` / `single_candidate_structure_reredrive.rs:215` | 教学窗口终态关闭（Denied/Drift 收口） |
| `establish_sc_teaching_window` | `workspace_sc_teaching_window.rs:117` | 建窗（`request.clone()` 落 session 记录 → `failure_source_ref` 唯一落盘处） |
| `sc_teaching_failure_request` | `single_candidate_structure_reredrive.rs:95` | 窗口请求构造（**`failure_source_ref` 唯一生产点**） |
| `sc_sha256` | `single_candidate_structure_reredrive.rs:43` | 教学链共用 sha256 hex（`failure_source_sha256`/`prompt_digest` 用它） |
| `put_source_revision` / `get_source_revision` | `src/product/work_item_plan_source_store.rs:182` / `:229` | durable source revision 写/读（含 hash 复验） |
| `build_work_item_plan_markdown_fresh_revision_prompt` | `src/product/work_item_split_engine/prompts/sc_revision.rs:69` | **「注入上一版完整 source」的唯一现成模板**（`[previous_source]` 围栏块 + 字节硬预算） |
| `previous_source_for_fresh_revision` 装载路径 | `src/web/workspace_ws_handler/run/single_candidate.rs:598-645` | **「按 source ref 读回上一版全文」的现成样板** |
| `close_plan_split_run_with_markdown` | `src/web/workspace_ws_handler/run/gateway_start.rs:551-562` | provider 原始输出全文落盘（`structured_output.json`） |
| `complete_work_item_split_provider_run_in_admission` | `src/product/work_item_split_engine/parse.rs:752-777` | 上者的底层写入（run.json + structured_output.json） |
| `update_artifact` / `ensure_plan_repair_artifact` | `src/product/workspace_engine/controls.rs:297` / `plan_repair_artifacts.rs:76` | 若 A4 要落「修复包」工件，这两处是工件写入样板（版本化 + `ArtifactRef` + `EngineEvent::ArtifactUpdate`） |
| `cap_autorepair_instruction_lines` / `MAX_AUTOREPAIR_INSTRUCTION_LINES` | `src/product/workspace_engine/contract_autorepair.rs:93` / `:88` | owner 可见面的指令条数上限（单点） |
| `render_teaching_reredrive_prompt` / `TeachingReredriveFacts` | `src/product/workspace_engine/prompts/sc_compile_teaching.rs:89` / `:39` | 教学 prompt 渲染本体 + typed 事实载体（A3 注入点） |

*本次全程只读：未创建/修改/删除任何文件，未执行任何写状态命令。*
