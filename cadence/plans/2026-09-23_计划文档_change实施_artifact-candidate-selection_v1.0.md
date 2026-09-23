# change 实施 artifact-candidate-selection 实施计划 v1.0

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 把共享 artifact 提取从「首开—末闭」跨块区间改为「顶层候选枚举 + workspace-aware 唯一 gate-passing 选择」，配失败诊断与弱模型 prompt 负面清单，修复 F-46。

**Architecture:** `artifact_extraction.rs` 新增无类型候选扫描器（枚举+元数据）；`artifact_constraints.rs` 新增 typed selector（对每个候选取既有 gate 结果，唯一通过才选）；全部生产调用方迁移到 selector；诊断走既有 `NodeDetail.execution_events`（kind=artifact 有界 JSON）；prompt 改三个既有注入点。

**Tech Stack:** Rust（workspace crate，`cargo test --locked --lib` 定向、clippy/fmt 门禁）。

**Spec:** `openspec/changes/artifact-candidate-selection/`（proposal/specs/design/tasks 四件套，strict valid）；完整设计调查 `.superpowers/sdd/2026-09-23_计划文档_change实施_plan-compile-gate-visibility_v1.0/f46-fix-design.md` 与 `f46-fix-oracle.md`。

## Global Constraints

- artifact gate 全部禁止规则零放宽：所选正文内 `<thinking>`、nested artifact fence 仍硬拒绝（REQ-ACS-01 末场景）。
- 不解除 kimi artifact retry 排除：`provider_allows_artifact_retry` 与 `kimi_is_excluded_from_artifact_retry` 测试不动（`kimi-code-provider-integration` SHALL）。
- 不改 AskUserQuestion 桥接与 auto permission 语义。
- 诊断不复制完整 provider 原文（原文已持久化于 session assistant message/streaming content）；不新增 WebSocket/UI 公开语义。
- split JSON 的 Work Item Plan 流（`WorkItemSplitProviderOutput`）不注入 Markdown 负面清单、不接 selector。
- 测试命令规范：`cargo test --locked --lib <过滤名>`（禁 `-j`、禁不带 `--lib` 的定向）；完成前 `cargo fmt --check` 与 `cargo clippy --all-targets --all-features --locked -- -D warnings` 零告警。
- F-46 真实样本：`.aria/projects/project_0001/issues/issue_0002/workspace-sessions/workspace_session_0007.json` 的 assistant `content`（19,921 字符；示意 block :200-203、`</thinking>` :267、真实候选 :270-361，均按 content 内换行 1-based）。

## Review Focus（spec 未显式钉死、最可能咬人的输入——每条已挂到所属任务测试）

1. **三个候选（1 示意 + 2 合法）**：必须 Ambiguous 失败，不做多数决/最长决 → Task 2 测试。
2. **候选外尾随一级标题**：存在完整候选（即便全失败）时禁止回落 heading fallback → Task 1/2 测试。
3. **CRLF 行尾**：行号/字节边界不错位（scanner 行进基于 `\n`，行号计数须容忍 `\r`）→ Task 1 测试。
4. **空候选（opening 紧跟 closing）**：枚举不 panic、gate 按缺标题拒 → Task 1 测试。
5. **诊断事件在极早期失败（node detail 尚空）**：upsert 安全创建不 panic → Task 4 测试。

---

### Task 0: F-46 真实候选 validator 实跑验证（设计 Open Question 收口）

**Files:**
- 无代码改动；产出验证结论写入本计划执行记录与 worker 报告。

**Steps:**

- [ ] **Step 0.1** 提取 F-46 原文候选为临时文件并实跑既有 gate：

```bash
cd <worktree>
python3 - <<'EOF'
import json
d=json.load(open('.aria/projects/project_0001/issues/issue_0002/workspace-sessions/workspace_session_0007.json'))
# assistant content 按消息定位（messages 中 role=assistant 的 content 字段；结构以实际 json 为准）
EOF
```
以提取的 content 写一次性 Rust 集成测试（临时，不入库）调用 `validate_workspace_artifact_constraints`（workspace_type=Story），断言第 270-361 行候选单独送入时 `passed == true`。

- [ ] **Step 0.2** 记录结论：
  - 通过 → 后续 Task 2 的红绿样本用真实候选；
  - 不通过 → scanner 行为不变，Task 2 样本改用合成合规候选（见 Task 2 Step 2.1 合成样本），并在报告注明真实候选的阻断原因清单（这本身是有价值证据）。

### Task 1: 顶层候选扫描器（extraction 层，无类型）

**Files:**
- Modify: `src/product/artifact_extraction.rs`
- Test: 同文件内联 `mod tests`（仓内惯例）

**Interfaces（Produces，Task 2/3/4 消费）:**

```rust
pub(crate) struct FencedArtifactCandidate {
    pub markdown: String,        // fence 内正文（不含 fence 行）
    pub opening_line: usize,     // 1-based，content 换行计数
    pub closing_line: usize,
    pub opening_byte: usize,
    pub closing_byte_end: usize, // closing fence 行尾字节偏移
    pub sha256: String,          // markdown 正文 hex
}
/// 枚举顶层完整 fenced artifact 候选；XML <artifact> marker 存在时沿用既有
/// 优先路径（本次不扩 XML 多候选）；无任何 marker 时既有 fallback 不动。
pub(crate) fn scan_top_level_fenced_candidates(output: &str) -> Vec<FencedArtifactCandidate>
```

**Steps:**

- [ ] **Step 1.1 失败测试**（内联 mod tests，一次写全）：

```rust
// 样本 A：F-46 形态（合成，行结构等价 :198-:361）
const F46_LIKE: &str = "前言\n```artifact\n# 示意标题\n...\n```\n中间 <thinking>过程</thinking> 文本\n```artifact\n# 真实 Story Spec\n\n## 范围\n- x\n```\n尾随说明\n";
#[test]
fn scan_finds_two_top_level_candidates() {
    let c = scan_top_level_fenced_candidates(F46_LIKE);
    assert_eq!(c.len(), 2);
    assert!(c[0].markdown.contains("示意标题"));
    assert!(c[1].markdown.contains("真实 Story Spec"));
    assert_eq!(c[1].opening_line, 8);
}
#[test]
fn scan_skips_nested_inner_fences() { /* 三反引号外层含 ```bash 内层：内层不算候选边界 */ }
#[test]
fn scan_four_backtick_outer_holds_inner_triple() { /* ````artifact 外层 + 内层 ```code```：单候选完整 */ }
#[test]
fn scan_crlf_line_numbers_stable() { /* \r\n 样本：行号与 A 的 \n 版本一致 */ }
#[test]
fn scan_empty_candidate_is_enumerated_not_panicking() { /* opening 紧跟 closing：候选 markdown 为空 */ }
#[test]
fn scan_unclosed_inner_same_length_fence_yields_no_candidate() { /* 同长度 fence 歧义：该块不产出候选 */ }
#[test]
fn existing_extraction_tests_unchanged() { /* 既有 XML/fallback/heading 测试零改动全绿 */ }
```

- [ ] **Step 1.2** `cargo test --locked --lib artifact_extraction` 确认新用例红（scan 未实现）。
- [ ] **Step 1.3** 实现扫描器：逐行状态机（顶层扫描→见 ` ```artifact `/` ````artifact ` 起 opening（记录 fence 字符与长度）→嵌套态（同字符且长度≥opening 的非裸 fence=inner 进入；inner 退出后的裸 fence 且长度≥opening 才可 closing）→closing 后回顶层。行号 1-based、`\r` 容忍、byte 偏移随行累计。同长度歧义（inner 未成对闭合）时该块丢弃不产出候选。
- [ ] **Step 1.4** `cargo test --locked --lib artifact_extraction` 全绿（含既有测试）。
- [ ] **Step 1.5** `cargo fmt --check` + clippy 零告警；commit `feat(extraction): 顶层 fenced artifact 候选枚举器（F-46）`。

### Task 2: workspace-aware 唯一 gate-passing selector

**Files:**
- Modify: `src/product/workspace_engine/artifact_constraints.rs`（selector 与类型；若文件超 large_file_guard 1200 行则拆 `artifact_selection.rs` 同目录）
- Test: 内联 mod tests

**Interfaces:**
- Consumes: Task 1 `scan_top_level_fenced_candidates`；既有 `validate_workspace_artifact_constraints(&markdown, workspace_type) -> report{passed, blocking…}`。
- Produces（Task 3/4 消费）:

```rust
pub(crate) enum SelectionVerdict { Unique, NoPassing, Ambiguous }
pub(crate) struct CandidateGateResult {
    pub opening_line: usize, pub closing_line: usize, pub sha256: String,
    pub passed: bool, pub blocking_reasons: Vec<String>,
}
pub(crate) struct CandidateSelection {
    pub verdict: SelectionVerdict,
    pub selected_markdown: Option<String>, // 仅 Unique 时 Some
    pub candidates: Vec<CandidateGateResult>,
    pub raw_output_chars: usize, pub raw_output_sha256: String,
    pub used_legacy_fallback: bool,        // 无 fenced 候选时走既有单候选 fallback 的标记
}
pub(crate) fn select_workspace_artifact(full_output: &str, workspace_type: WorkspaceType) -> CandidateSelection
```

**Steps:**

- [ ] **Step 2.1 失败测试**：合成合规 Story 候选（真实候选若 Task 0 验证通过则另加真实样本断言）；用例=①唯一通过选中（示意缺 `## 范围` 等 heading 被拒+候选 2 完整合规）②零通过：`blocking_reasons` 逐候选保留、不回落 heading ③双合法→Ambiguous 且 `selected_markdown==None` ④三候选(1 示意+2 合法)→Ambiguous ⑤候选全失败但尾随一级标题→不取标题 ⑥无任何 marker→legacy fallback 单候选走同一 gate、`used_legacy_fallback=true`。
- [ ] **Step 2.2** 跑红：`cargo test --locked --lib artifact_selection`（或 artifact_constraints 过滤名）。
- [ ] **Step 2.3** 实现：扫描→逐候选 gate→verdict 判定；禁止任何「取末块/最长块」分支；fallback 仅在 `scan` 返回空时启用。
- [ ] **Step 2.4** 跑绿 + 全量 `cargo test --locked --lib`；fmt/clippy；commit `feat(artifact): workspace-aware 唯一 gate-passing 候选选择器（REQ-ACS-01）`。

### Task 3: 生产调用方全量迁移

**Files（Modify，全部既有）:**
- `src/product/workspace_engine/provider_drive.rs`：`should_retry_missing_workspace_artifact`、`ProviderEvent::Completed` 提取分支、`complete_assistant_message`
- `src/product/workspace_engine/provider_drive/artifact_retry.rs`：`workspace_artifact_blocking_reasons`、失败摘要
- `src/product/workspace_engine/parsers/choice.rs`
- `src/product/workspace_engine/lifecycle.rs`（恢复路径）
- `src/product/workspace_engine/mappings.rs::latest_artifact_from_messages` + `types.rs::WorkspaceSession::from_record`（签名加 workspace_type）
- `src/product/coding_work_item_context.rs::latest_assistant_artifact_markdown`
- `src/web/` 侧若有直接调用 `extract_artifact_content` 的 handler 一并迁移（实施时以 `grep -rn 'extract_artifact_content' src/` 全量清单为准，清单写入报告）

**Interfaces:**
- Consumes: Task 2 `select_workspace_artifact`。
- Produces: 生产路径零 `extract_artifact_content` 直调（唯二豁免：`artifact_extraction.rs` 自身单测；selector 内部 fallback）。

**Steps:**

- [ ] **Step 3.1** `grep -rn 'extract_artifact_content' src/ --include='*.rs'` 产出调用清单落报告；逐点改为 `select_workspace_artifact(...)`：Unique→取 `selected_markdown`；NoPassing/Ambiguous→走各点既有失败分支（retry 判断/失败摘要/不恢复），失败文案附 `candidate_count/passing_count`。
- [ ] **Step 3.2** 迁移序：provider_drive 主链 → artifact_retry → choice → lifecycle → mappings/types → coding_work_item_context → web handler；`full_output`/`full_content` 双源在 `complete_assistant_message` 入口统一选择（先选 raw 源再 selector，单次消费）。
- [ ] **Step 3.3** 每迁一组跑对应过滤测试；迁移完跑全量 `cargo test --locked --lib` + `--test it_web`（WS/handler 面触及）。
- [ ] **Step 3.4** reload/recovery 回归：session reload（`from_record`）与 coding fallback 用同一 selector 的断言用例（合成双 block session record）。
- [ ] **Step 3.5** fmt/clippy；commit `refactor(workspace): 全调用方迁移至 artifact 候选选择器（REQ-ACS-01）`。

### Task 4: 候选选择诊断事件（REQ-ACS-02）

**Files:**
- Modify: `src/product/workspace_engine/provider_drive/artifact_retry.rs`（或等价 diagnostics 构造点）
- Test: 内联/邻近测试

**Interfaces:**
- Consumes: Task 2 `CandidateSelection`（含逐候选结果与 sha256）；既有 `emit_execution_event` upsert、`NodeDetail.execution_events` 结构（`event_id`/`kind`/`title`/`detail`/`output`）。
- Produces: `kind="artifact"`、`event_id="artifact_diag_{node_id 前 8}_{raw_sha256 前 8}"`、`output=None`；`detail` 为有界 JSON：

```json
{"diagnostic_version":1,"workspace_type":"story","raw_output_chars":19921,
 "raw_output_sha256":"<hex>","used_legacy_fallback":false,
 "candidates":[{"opening_line":200,"closing_line":203,"sha256":"<hex>","passed":false,
   "blocking_reasons":["..."]}],
 "candidate_count":2,"passing_count":1,"selection":"unique"}
```

**Steps:**

- [ ] **Step 4.1 失败测试**：①gate 失败路径触发诊断事件且字段齐（合成 selection）②诊断 upsert 幂等（同 event_id 二次调用不重复）③`detail` 不含完整原文（长度上界断言）④诊断写失败→失败摘要含 `artifact diagnostic persistence failed` 且 gate 结论不变 ⑤node detail 为空时 upsert 安全创建。
- [ ] **Step 4.2** 跑红→实现（构造点在 provider_drive 失败分支与 selector 返回处，经既有 execution event 通道）→跑绿。
- [ ] **Step 4.3** 全量 lib；fmt/clippy；commit `feat(artifact): 候选选择有界诊断事件（REQ-ACS-02）`。

### Task 5: prompt 负面清单（REQ-ACS-03）

**Files（Modify）:**
- `src/web/workspace_context/prompts.rs::output_schema_for`
- `src/product/workspace_engine/prompts.rs::append_author_artifact_output_contract`
- `src/product/workspace_engine/prompts.rs::build_artifact_retry_prompt`（与共享清单对齐去重）

**Interfaces:**
- Produces: 共享常量（建议 `prompts.rs` 内 `pub(crate) const AUTHOR_ARTIFACT_NEGATIVE_LIST: &str`），三注入点引用同一常量。

**Steps:**

- [ ] **Step 5.1 失败测试**（prompt contract 断言，既有 prompt 测试风格）：①三注入点输出含负面清单五条关键词（「一个」「artifact fence 之外」「`<thinking>`」「四反引号」「不得作为候选回显」）②Story/Design/WorkItem/legacy WorkItemPlan 四类型初次 prompt 均含 ③split JSON prompt 不含 ④retry prompt 与共享清单语义一致无重复长段。
- [ ] **Step 5.2** 负面清单文案（design §6.2 定稿，全文入常量）：

```text
输出纪律（负面清单）：
- 最终响应只生成一个完整的顶层 artifact fenced block；不要输出多个完整候选、示意 block，或把早前示例复制成 block。
- 过程说明、思考与决策解释必须写在最终 artifact fence 之外；不要输出 <thinking>/</thinking> 标签。
- artifact 正文不得包含另一个 artifact fence。
- 正文内需要三反引号代码块时，外层使用四反引号 ````artifact ... ```` 包裹；不要用与外层同长度的 fence 造成边界歧义。
- prompt 中的骨架/示例只是结构说明，不得作为最终候选回显；最终 fence 内第一行必须是当前工作类型的一级标题。
```

- [ ] **Step 5.3** 跑红→注入→跑绿；全量 lib；fmt/clippy；commit `feat(prompt): author artifact 负面清单三注入点统一（REQ-ACS-03）`。

### Task 6: 跨类型表驱动回归 + change 级门禁（REQ-ACS 全场景 + story 一次成功新判据）

**Files:**
- Test: `src/product/workspace_engine/` 下既有跨类型测试文件旁新增表驱动用例（实施时按邻近测试惯例落位）

**Steps:**

- [ ] **Step 6.1** 表驱动（Story/Design/WorkItem/WorkItemPlan × 场景矩阵）：唯一有效候选恢复 / 双有效歧义失败 / 所选正文污染仍拒 / 候选外文本不回落 / reload 不错切 / 「一次成功」含前置示意 block 场景（story 判据）。
- [ ] **Step 6.2** 跑全量：`cargo test --locked --lib`、`--test it_web`、`--test it_core`（复跑判定 known-flaky）、前端面若无触及则记录不跑依据。
- [ ] **Step 6.3** `openspec validate artifact-candidate-selection --strict` 复跑；large_file_guard 绿（超 1200 行即拆）。
- [ ] **Step 6.4** fmt/clippy 门禁；commit `test(artifact): 跨类型表驱动回归与门禁收口`。

---

## Self-Review 结论

- **Spec 覆盖**：REQ-ACS-01→Task 1/2/3；REQ-ACS-02→Task 4；REQ-ACS-03→Task 5；story MODIFIED 判据→Task 6.1；Non-Goals 零任务触碰（Global Constraints 钉死）。Task 0 收口 design Open Question。
- **占位符**：无 TBD/「适当处理」类步骤；fence 状态机、诊断 JSON、负面清单文案均给全文。
- **类型一致性**：`FencedArtifactCandidate`/`CandidateSelection`/`SelectionVerdict` 在 Task 1/2 定义、3/4 按同名消费。
- **Review Focus**：五条均已挂对应任务测试（Task 1×3、Task 2×2、Task 4×1 中含空 detail）。
