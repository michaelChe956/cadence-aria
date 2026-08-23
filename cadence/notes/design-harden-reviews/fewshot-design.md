## C1 现行规则文本摘录

`src/product/workspace_engine/artifact_constraints.rs:299-308`（`reviewer_boundary_rules_for`，唯一注入点，共 5 处调用：`prompts/review.rs:43,178,480,660,741`）：

```rust
pub(crate) fn reviewer_boundary_rules_for(workspace_type: &WorkspaceType) -> String {
    let spec = artifact_constraint_spec_for(workspace_type);
    let mut output = String::from("\n[artifact_boundary_must_fix_rules]\n");
    for rule in spec.reviewer_must_fix_rules {
        output.push_str("- ");
        output.push_str(rule);
        output.push('\n');
    }
    output
}
```

`artifact_constraints.rs:152-154`，`artifact_constraint_spec_for(Design).reviewer_must_fix_rules` 全文（单条，原文照录）：

```text
Design artifact: Work Item Plan、开发任务列表、任务拆分、测试计划、测试范围或场景、测试文件或模块、测试框架或夹具、测试命令、构建命令、执行 checklist 或将测试或验证职责分配给组件或文件必须报告为 must_fix；仅把 [DEC-*] 关联到 [REQ-*]/[AC-*] 且不描述如何测试的抽象验收可追踪性不得报告为 must_fix。
```

Design 其余相关契约（同一 spec，供判例保持一致）：required_headings 为 `设计范围/设计决策/公共组件/API 契约/数据模型/风险/追踪关系`；required_id_patterns 为 `[DEC-*]/[CMP-*]/[API-*]`；required_tokens 为 `source id`；forbidden_headings 含 `Work Item Plan/任务拆分/开发任务/执行 checklist`；forbidden_tokens 为 `[TASK-*]`、`WI-*`。deterministic 校验（`validate_workspace_artifact_constraints`）不含任何测试关键词扫描——「测试计划越界」目前完全由 reviewer 判断。

全局 few-shot 现行写法（`prompts.rs:166-189`，`reviewer_output_contract`）：先 `完整示例（仅用于理解结构，绝不可照抄 nonce）` + `<ARIA_STRUCTURED_OUTPUT nonce="EXAMPLE_NONCE">` + `schema_with_nonce("EXAMPLE_NONCE", schema)`，再 `实际输出模板（必须使用本请求 nonce）` + 真实 nonce 块。`schema_with_nonce` 把 `{"nonce":"<n>",` 前置进 schema 体。防照抄由 `parse_structured_output` 的 nonce 精确比较承担（`structured_output.rs:122-131`，`EXAMPLE_NONCE` 永不派发）。现有测试 `prompts.rs:783-813` 断言 `EXAMPLE_NONCE` 首次出现早于真实 nonce。

`tests/part_31.rs:167-184` 的现状：只对 `reviewer_boundary_rules_for(Design)` 返回串做 9 个 `contains` 断言（`抽象验收可追踪性`、`不得报告为 must_fix`、8 个禁止词面），不构造任何 candidate，不走 prompt 组装，不走 verdict 解析——即分析文档 §3 P1-A 所说「验证规则字符串存在，而不是验证真实 Design candidate 得到预期 finding」。

## C2 判例 few-shot 完整文案（可直接进入实现）

放置模块：新建 `src/product/workspace_engine/prompts/reviewer_boundary_examples.rs`（约 45 行，避免继续增长 923 行的 `artifact_constraints.rs`），在 `prompts.rs:7-13` 的 `mod` 列表加 `mod reviewer_boundary_examples;`。导出：

```rust
pub(super) fn reviewer_boundary_examples_for(workspace_type: &WorkspaceType) -> &'static str {
    match workspace_type {
        WorkspaceType::Design => DESIGN_REVIEWER_BOUNDARY_EXAMPLES,
        _ => "",
    }
}
```

同时把 `prompts.rs` 内两处字面量 `"EXAMPLE_NONCE"` 提为 `pub(crate) const REVIEWER_EXAMPLE_NONCE: &str = "EXAMPLE_NONCE";`，判例常量与测试统一引用，避免第二份占位值。

`DESIGN_REVIEWER_BOUNDARY_EXAMPLES` 正文（判例内 sentinel 一律用 `EXAMPLE_NONCE`；产物摘录每行以 `| ` 前缀，使其不构成合法 markdown heading，降低被当成真实 heading 或被抄进产物的概率；ID 用 90x 段位显式占位）：

```text

[design_reviewer_boundary_examples]
以下是 Design 边界的对照判例，只用于校准 severity 口径。判例中的 ID、文件名与 nonce 均为虚构占位；EXAMPLE_NONCE 任何请求都不会派发，照抄判例会被 nonce 校验拒绝。真实 finding 的 evidence 必须逐字引自当前产物。

判例 1（抽象验收追踪，合规）—— 产物摘录：
| ## 设计决策
| - [DEC-901] 删除采用软删除并保留审计字段。
| ## 追踪关系
| - source ids: Story Spec story_spec_example, Issue issue_example。
| - [DEC-901] -> [REQ-903] / [AC-904]（验收口径：删除后查询不再返回该记录）
期望裁决：把 [DEC-*] 关联到 [REQ-*]/[AC-*] 且不描述如何测试，是 Design 必须承担的抽象追踪，最高只能是 suggestion；findings 为空的 pass 同样正确。
<ARIA_STRUCTURED_OUTPUT nonce="EXAMPLE_NONCE">
{"nonce":"EXAMPLE_NONCE","verdict":"pass","summary":"设计边界正确，追踪为抽象验收口径","findings":[{"severity":"suggestion","message":"[DEC-901] 的验收口径可补一句数据保留期，便于下游引用；不影响本阶段可用性","evidence":"## 追踪关系：[DEC-901] -> [REQ-903] / [AC-904]（验收口径：删除后查询不再返回该记录）","required_action":"可选：在 [DEC-901] 补充保留期约束"}]}
</ARIA_STRUCTURED_OUTPUT>

判例 2（可执行测试内容，越界）—— 产物摘录：
| ## 公共组件
| - [CMP-902] RetryPolicy：统一重试与退避，并负责为自身编写单元测试与集成测试。
| ## 风险
| - 回归验证计划：第一步在 tests/retry_policy.rs 用 mockito 搭建夹具，第二步执行 cargo test --locked retry_policy，第三步补充 3 条超时场景用例。
期望裁决：出现具体测试文件或模块、测试框架或夹具、可运行命令、分步测试场景，或把测试与验证职责指派给组件或文件，必须 must_fix。
<ARIA_STRUCTURED_OUTPUT nonce="EXAMPLE_NONCE">
{"nonce":"EXAMPLE_NONCE","verdict":"revise","summary":"Design 越界写入可执行测试计划与测试职责分派","findings":[{"severity":"must_fix","message":"越界的可执行测试内容：测试文件、测试框架、测试命令与分步测试场景属于 Work Item 阶段，留在 Design 会与下游拆分重复或冲突","evidence":"## 风险：第一步在 tests/retry_policy.rs 用 mockito 搭建夹具，第二步执行 cargo test --locked retry_policy","required_action":"删除测试文件、框架、命令与分步场景，只保留 [DEC-*] 到 [REQ-*]/[AC-*] 的抽象验收口径"},{"severity":"must_fix","message":"把验证职责指派给组件：[CMP-902] 被写成自身测试的负责方，属于执行期分工，不属于 Design 边界","evidence":"## 公共组件：[CMP-902] RetryPolicy：统一重试与退避，并负责为自身编写单元测试与集成测试","required_action":"将 [CMP-902] 改写为职责与接口边界描述，删除测试负责方表述"}]}
</ARIA_STRUCTURED_OUTPUT>

判例 3（风险章节合法提及验证归属，不得误伤）—— 产物摘录：
| ## 风险
| - [DEC-905] 幂等键冲突概率未知；缓解：由下游 Work Item 阶段安排验证，Design 阶段不定义测试方案。
期望裁决：只出现「测试」「验证」词面，而没有具体文件、框架、命令、分步场景或职责指派时，不构成 must_fix。
<ARIA_STRUCTURED_OUTPUT nonce="EXAMPLE_NONCE">
{"nonce":"EXAMPLE_NONCE","verdict":"pass","summary":"风险缓解只声明验证归属，未越界","findings":[]}
</ARIA_STRUCTURED_OUTPUT>

判定顺序：先判断是否命中可执行信号（具体测试文件或模块、测试框架或夹具、可运行命令、分步测试场景、把测试或验证职责指派给组件或文件）；命中才可 must_fix，未命中最高 suggestion。实际输出只能使用下方模板中的本请求 nonce。
```

体量：约 2.3k 字符 / 3.9KB UTF-8，仅注入 Design reviewer prompt。

插入位置与拼接函数改动点（`prompts.rs`）：

1. `reviewer_output_contract`（`:166`）签名增加一个参数 `boundary_examples: &str`，插在 `EXAMPLE_NONCE` 完整示例之后、`实际输出模板（必须使用本请求 nonce）` 之前。`format!` 模板改为 `...</ARIA_STRUCTURED_OUTPUT>\n{boundary_examples}实际输出模板（必须使用本请求 nonce）：\n...`。这样真实 nonce 仍是 prompt 里最后出现的 nonce，防照抄的顺序不变（并可加强为 `rfind` 断言）。
2. `prompts/review.rs` 6 处调用（`:82,285,413,598,668,811`）统一多传 `reviewer_boundary_examples_for(&self.session.workspace_type)`。只有 Design 返回非空，WorkItemPlan/Story/WorkItem 返回 `""`，行为零变化；不新增 aggregate 分支。
3. `prompts.rs` 模块内既有单测（`:773`、`:800`、`:817`）补第 5 个实参 `""`。

单仓限定说明：判例按 `WorkspaceType` 键控，不读 `AGGREGATE_SCOPE_MARKER`，因此不触碰 `is_aggregate_story_or_design` 分支，也不改 `structured_output_contract`；aggregate Design review 会看到同一段文本（语义一致，且 aggregate 的 metadata 契约不在本 change 范围）。回归以「无 aggregate marker 的普通 Design session」为主断言，另加一条 Story/WorkItem 不含该 marker 的负例。

## C3 回归 fixture 方案

新建 `src/product/workspace_engine/tests/design_reviewer_boundary.rs`（约 170 行），在 `tests.rs` 的 include 列表末尾按 `severity_three_tier.rs` 的先例加 `include!("tests/design_reviewer_boundary.rs");`。不并入 `part_31.rs`（现 961 行，`tests/it_core/large_file_guard.rs` 限 1200 行）。复用 `part_01.rs` 的 `setup()`、`make_session()`、`artifact_payload()`、`complete_design_artifact()`。

三个最小 candidate 片段（都补齐 7 个 heading + `[DEC-*]/[CMP-*]/[API-*]` + `source id`，保证 deterministic gate 通过，从而把分类责任固定在 reviewer 侧）：

| fixture | candidate 关键片段 | 期望分类 |
|---|---|---|
| `design_candidate_with_abstract_traceability` | `## 追踪关系` 内 `- [DEC-001] -> [REQ-001] / [AC-001]（验收口径：删除后查询不再返回该记录）` | 抽象追踪 → 最高 suggestion，禁止 must_fix |
| `design_candidate_with_executable_test_plan` | `## 风险` 内 `第一步在 tests/idempotency.rs 用 mockito 搭建夹具，第二步执行 cargo test --locked idempotency`；`## 公共组件` 内 `[CMP-001] …并负责为自身编写单元测试` | 两条 must_fix（越界测试内容 + 职责指派） |
| `design_candidate_with_risk_mentioning_verification` | `## 风险` 内 `缓解：由下游 Work Item 阶段安排验证，Design 阶段不定义测试方案。` | 不得 must_fix（误伤对照） |

断言（contract-driven，全部经真实 candidate → 真实 prompt → 真实解析路径，不做「判例文案存在」式 grep 之外的验收）：

1. `design_reviewer_prompt_injects_boundary_examples_once_before_the_request_nonce`：三个 candidate 分别塞进 `session.artifact`（`session.messages` 不含聚合 marker），`engine.build_review_input()`；断言 `prompt.matches("[design_reviewer_boundary_examples]").count() == 1`；断言 `prompt.rfind("EXAMPLE_NONCE") < prompt.rfind(&contract_nonce)`（nonce 取自 `input.structured_output_contract.as_ref().unwrap().nonce`）；断言 `[artifact_boundary_must_fix_rules]` 与 `[artifact_schema_review_gate]` 仍各出现 1 次；断言 prompt 不含 `</ARIA_STRUCTURED_OUTPUT nonce=`（保持 `part_08` 已有的闭合标签契约）。
2. `non_design_reviewer_prompts_exclude_design_boundary_examples`：Story / WorkItem / WorkItemPlan（`build_review_input` 会分派到 plan 分支）三条 prompt 均 `!contains("[design_reviewer_boundary_examples]")`。
3. `design_boundary_case_verdicts_map_to_expected_gates`：对每个 candidate 用本请求 nonce 组装判例形状的 reviewer 输出，走 `ProviderCompletion::from_output(output, Some(&contract), None)` + `engine.parse_review_completion_for_active_node(&completion)`，断言：
   - 抽象追踪 → `ReviewVerdictType::Pass` + `ReviewGate::UserConfirmAllowed`，`findings` 全为 `Suggestion`，且无 `MustFix|Blocking`；
   - 可执行测试 → `ReviewVerdictType::Revise` + `ReviewGate::RequiresRevision`，恰好 2 条 `MustFix`，`evidence` 非空；
   - 风险提及验证 → `Pass` + `UserConfirmAllowed` + `findings.is_empty()`。
   这一条直接把 `review_gate_for`（`parsers.rs:585`）的三档映射与判例期望绑在一起，判例文案若与 severity 语义漂移即失败。
4. `copied_design_boundary_example_never_auto_revises`：把判例 2 的 sentinel 块原样（含 `EXAMPLE_NONCE`）作为 reviewer 输出，对本请求 contract 解析，断言 `Err(ReviewCompletionError::Syntax(e))` 且 `e.code == StructuredOutputErrorCode::NonceMismatch`、`e.observed_nonce == Some("EXAMPLE_NONCE")`，并断言 `fallback_review_verdict(...)` 得到 `NeedsHuman` + `UserTriageRequired`（照抄不会变成自动返修）。
5. `design_boundary_candidates_stay_out_of_the_deterministic_gate`：三个 candidate 都断言 `validate_workspace_artifact_constraints(candidate, &WorkspaceType::Design).passed`。这条把「本 change 不引入测试关键词 pre-gate」写成契约，未来若有人加关键词扫描会立即红灯，需显式改契约。

命令：`cargo test --locked --lib design_reviewer_boundary`、`cargo test --locked --lib workspace_engine`、`cargo test --locked --test it_core large_file_guard`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo fmt --check`。

## C4 风险与缓解

判例被模型照抄进产物 / 裁决

- 照抄裁决 JSON：`EXAMPLE_NONCE` 永不派发，`parse_structured_output` 精确比较 nonce 直接失败，由 C3 断言 4 钉死。
- 残余风险（需在 change 里显式处理）：`NonceMismatch` 在 `review/structured_output.rs:38-52` 的 `is_repairable` 白名单内，且 `recoverable_value` 会保留剥离 nonce 后的业务 JSON。也就是说照抄判例 2 会触发一次 envelope-only repair，把「tests/retry_policy.rs」这类虚构 must_fix 以新 nonce 复活，进而误触发返修。缓解：在 `is_repairable` 增加 `&& error.observed_nonce.as_deref() != Some(REVIEWER_EXAMPLE_NONCE)`，把「照抄示例」判为不可修复 → `fallback_review_verdict` 走 `UserTriageRequired`。该判定属共享层（Story 同受益），语义与已归档 spec 的「照抄示例无法通过校验」一致，建议纳入本 change 并配一条独立测试；若不做，必须在 change 的残余风险里写明「照抄可经 repair 复活虚构 finding」。
- 照抄产物摘录进 Design artifact：判例只进 reviewer prompt，不进 author 三个注入点，author 侧 skeleton 不变。摘录每行加 `| ` 前缀（非合法 heading）、ID 用 `DEC-901/CMP-902/REQ-903/AC-904/DEC-905` 占位段、文件名用 `tests/retry_policy.rs` 这类显然外部路径，并在判例首段声明「ID、文件名与 nonce 均为虚构占位」「evidence 必须逐字引自当前产物」。摘录内不含 `[TASK-*]`、`WI-*`、``` fence，即使被回抄也不会新增 deterministic 违规类型。

误伤「设计风险」章节合法提及测试策略

- 判例 3 就是负例对照，直接给出 pass + 空 findings 的期望裁决；C3 断言 3 把它作为回归。
- 判例末尾固定「判定顺序」：先判可执行信号（文件/模块、框架/夹具、可运行命令、分步场景、职责指派），未命中最高 suggestion；把弱模型容易做的「关键词命中即 must_fix」改成两段式判定。
- 本 change 明确不新增关键词 deterministic pre-gate（沿用分析文档 §3 P1-A、§7 的结论），并用 C3 断言 5 把「三个 candidate 都能过 artifact gate」写成契约；是否升格为 pre-gate留给 Design campaign 的假阳性/漏报数据决定。

其他残余风险

- token 成本：每个 Design reviewer 请求固定多约 3.9KB，与滑动窗口的降耗目标相抵一部分。缓解：仅 Design 注入；每判例摘录 ≤5 行；在 Design campaign 的 fresh/resume usage 口径里单列该增量，超预算时优先压缩判例 1 的 suggestion 文案。
- 三判例覆盖不了「构建命令」「执行 checklist」「开发任务列表」等其余禁止形态，规则文本仍是唯一约束。缓解：判例定位为口径校准而非穷举，规则条文保持不动；campaign 若发现某形态高漏报，再按同一模板追加判例而不是改规则。
- 判例文案与 `reviewer_must_fix_rules` 未来漂移。缓解：C3 断言 3 走 severity 映射而非字符串匹配，另可加一条轻量断言——判例文本包含规则里的可执行信号词面清单，使两处同改同测。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "satisfied",
      "evidence": "只读交付 C1-C4 四段设计：C1 摘录 artifact_constraints.rs:152-154 与 :299-308、prompts.rs:166-189 原文；C2 给出可直接落地的 Design 判例常量全文 + 新模块 prompts/reviewer_boundary_examples.rs + reviewer_output_contract 增参与 6 处调用点（review.rs:82,285,413,598,668,811）；C3 给出 3 个 candidate fixture、新测试文件 tests/design_reviewer_boundary.rs 与 5 组解析路径断言；C4 给出照抄与误伤各自缓解。范围限定单仓 Design（按 WorkspaceType 键控，不新增 aggregate 分支），未修改任何文件、未做 git 写操作、未派生子代理。"
    }
  ],
  "changedFiles": [],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "git status --porcelain",
      "result": "passed",
      "summary": "工作树干净，无改动、无暂存"
    },
    {
      "command": "ast-grep outline src/product/workspace_engine/artifact_constraints.rs / prompts/review.rs / tests/part_31.rs",
      "result": "passed",
      "summary": "按代码阅读规则先取结构大纲再定向精读"
    },
    {
      "command": "rg -n 'reviewer_output_contract|EXAMPLE_NONCE' src/product/workspace_engine/",
      "result": "passed",
      "summary": "定位全局 few-shot 唯一实现与 6 处 reviewer 调用点"
    },
    {
      "command": "cargo test --locked --lib design_reviewer_boundary",
      "result": "not-run",
      "summary": "本任务为只读设计，测试文件尚未实现"
    }
  ],
  "validationOutput": [
    "reviewer_boundary_rules_for 为唯一注入点，被 prompts/review.rs:43,178,480,660,741 调用；Design 规则为单条字符串（artifact_constraints.rs:152-154）",
    "reviewer_output_contract(prompts.rs:166) 现有 4 参，EXAMPLE_NONCE 示例在真实 nonce 模板之前；prompts.rs:783-813 已断言该顺序",
    "validate_workspace_artifact_constraints 不含任何测试关键词扫描，Design 越界判定完全在 reviewer 侧",
    "part_31.rs:167-184 现仅做 9 个规则字符串 contains 断言，无 candidate 构造，与分析文档 §3 P1-A 描述一致",
    "review_gate_for(parsers.rs:585) 确认 pass+suggestion→UserConfirmAllowed、含 must_fix→RequiresRevision，判例期望裁决与之自洽",
    "is_repairable(review/structured_output.rs:38-52) 将 NonceMismatch 列为可修复，构成照抄判例经 repair 复活的具体风险点，已在 C4 给出定点缓解",
    "part_31.rs 961 行、large_file_guard 上限 1200 行，故回归放新文件而非并入 part_31"
  ],
  "residualRisks": [
    "照抄判例 2 会命中 NonceMismatch 的 repair 白名单，可能把虚构 must_fix 以新 nonce 复活；需在实现时收紧 is_repairable（共享层小改）或在 change 中显式记录该残余风险",
    "判例固定增加约 3.9KB Design reviewer prompt，与滑动窗口降耗目标部分相抵，需在 Design campaign usage 口径单列",
    "判例按 WorkspaceType 键控，aggregate Design review 也会看到同一段文本（语义一致，无新增分支，但不属本次验收范围）",
    "三判例不覆盖构建命令、执行 checklist、开发任务列表等其余禁止形态，仍依赖规则条文",
    "reviewer 分类是模型行为，回归只能钉住 prompt 组装与解析路径，真实分类准确率需 Design 真机 campaign 数据"
  ],
  "noStagedFiles": true,
  "diffSummary": "无代码改动：本任务为只读设计交付（判例文案、插入点、回归方案、风险缓解）。",
  "reviewFindings": [
    "no blockers",
    "note: review/structured_output.rs:38-52 - NonceMismatch 属可修复错误，照抄 EXAMPLE_NONCE 判例可经 envelope repair 复活业务 payload，实现阶段需一并处理",
    "note: prompts.rs:166 增参会波及 review.rs 6 处调用与 prompts.rs 内 3 处单测，需同批更新以保持编译"
  ],
  "manualNotes": "插入位置选择在 EXAMPLE_NONCE 示例与真实 nonce 输出模板之间（而非整个 reviewer_output_contract 之后），以保持『真实 nonce 最后出现』的防照抄顺序；若父代理希望零签名改动，替代方案是在 build_review_input 末尾追加，但会让 EXAMPLE_NONCE 成为 prompt 中最后出现的 nonce，照抄风险上升，不建议。"
}
```