## Completed

已完成**单仓、非 aggregate Design Workspace 链路**的只读代码级审计。未修改文件、未执行任何 Git 写操作，aggregate 相关实现仅按要求标记为“超出范围”。

## A1 现状结构表（链路环节→文件:行→行为摘要）

| 链路环节 | 文件:行/符号 | 行为摘要 |
|---|---|---|
| Design artifact 结构定义 | `src/product/workspace_engine/artifact_constraints.rs:122-155` `artifact_constraint_spec_for(WorkspaceType::Design)` | 定义 7 个必需 heading、3 族必需 ID、`source id` 追踪 token、4 个禁止 heading、`[TASK-*]`/`WI-*` 禁止 token，以及一条 Design reviewer 边界规则。 |
| Design artifact 实际校验 | `artifact_constraints.rs:222-296` `validate_workspace_artifact_constraints` | 从所有 Markdown heading 中找 required/forbidden；校验 required ID 和 token；校验结构污染、嵌套 artifact fence、`<thinking>`。通过条件只是各类“缺失/命中”集合为空。 |
| Heading 级别实际规则 | `src/product/workspace_engine/parsers/choice.rs:233-246` `normalize_workspace_heading_line` | 接受 `#` 至 `######` 任意级别 heading；而 schema renderer 声明“使用二级 heading”。因此 `### 设计决策` 也能满足当前 validator，存在**文案契约与实际 gate 不一致**。 |
| ID/source token 匹配细节 | `artifact_constraints.rs:857-922` `required_token_present`、`has_explicit_source_id_traceability`、`find_bracket_prefixed_tokens` | `source id` 可由字面字符串或 `issue_`、`story_spec_`、`design_spec_`、`work_item_` 任一出现满足。`[DEC-*]` 等匹配仅要求 token 以此前缀开头并有 `]`，未要求非空/数字后缀、唯一性、归属章节或逐项追踪；required ID 检查直接扫原始 content，围栏代码中的示例也会计入。 |
| author schema 渲染 | `artifact_constraints.rs:310-362` `author_artifact_schema_contract_for` | 从同一 `ArtifactConstraintSpec` 渲染 `[artifact_schema_contract]`，列出必需二级 heading、ID 示例、source token、禁止 heading/token。 |
| reviewer schema gate 渲染 | `artifact_constraints.rs:321-328` `reviewer_artifact_schema_gate_for` | 告知 reviewer：任何缺失必需项或命中禁止项都必须形成 `must_fix`，且不得输出 `pass`。这是 LLM 指令，不会在 reviewer 返回 `pass` 后由代码复算“是否已报告每个问题”。 |
| 初次 author prompt | `src/product/workspace_engine/prompts.rs:334-383` `build_prompt` | 注入运行阶段契约；若 system context 尚无 `[artifact_schema_contract]` 则注入 parser-derived schema；**始终**注入 Design skeleton；再接入 Author 模式历史压缩、缺失上下文 note、当前用户输入。标准 Web 链路的 artifact fence/用户决策说明来自 system context。 |
| 初次 author 的 system output schema | `src/web/workspace_context/prompts.rs:146-168` `output_schema_for(Design)` | 明确要求 `artifact` fence、fence 内第一行是 Design Spec 一级标题、追踪上游 Story、结构化交互决策写入“设计决策/追踪关系”，再追加 parser-derived schema。标准 socket 链路由 `ensure_workspace_context_message` 注入该 system context。 |
| artifact retry prompt | `prompts.rs:82-117` `build_artifact_retry_prompt` | 显式要求单个完整 artifact fence、禁止 `<thinking>`、列出 deterministic blocking reasons；随后注入 schema、Design skeleton、结构化交互决策契约。 |
| reviewer full revision prompt | `src/product/workspace_engine/prompts/revision.rs:107-155` `build_revision_full_prompt` | 注入路由引用、Author 模式历史压缩、缺失 context note、上一版 artifact、review comments/summary、用户补充、artifact fence 说明、schema、决策契约和 Design skeleton。 |
| reviewer resume/delta revision prompt | `revision.rs:75-105` `build_revision_delta_prompt` | 注入路由引用、review comments/summary、用户补充、artifact fence 说明、schema、决策契约；**不注入历史压缩、上一版全文或 skeleton**，依赖 provider resume 会话中的既有上下文。 |
| Design skeleton | `prompts.rs:471-506` `author_artifact_skeleton_example`、`structured_interaction_artifact_decision_contract` | Skeleton 包含 7 个必需 heading，文案声称“缺稳定 ID、REQ/AC 与追踪 token，不能照抄”。它本身无 `[DEC-*]`/`[CMP-*]`/`[API-*]` 和 source token，无法通过 gate；但“REQ/AC”是 Story 用语，不够贴合 Design。Design 决策契约要求将 choice 写入“设计决策”或“追踪关系”，并映射至 `[DEC-*]`。 |
| 三处 skeleton 注入情况 | `prompts.rs:346-348`、`prompts.rs:110-116`、`revision.rs:149-152` | `build_prompt`、`build_artifact_retry_prompt`、`build_revision_full_prompt` 均有 skeleton 和“不能照抄”；`build_revision_delta_prompt` 没有 skeleton，属于有意的 resume/token 控制策略。 |
| Design review 输入组装 | `src/product/workspace_engine/prompts/review.rs:15-111` `build_review_input` | 依次注入：workspace 类型、Design boundary rules、schema review gate、Reviewer 压缩历史、缺失 context note、中间 artifact diff、未关闭强 finding、当前已提取 artifact、artifact fence 已剥离说明、nonce reviewer JSON contract。`structured_output_contract` 始终带真实 nonce。 |
| Design reviewer 边界规则 | `artifact_constraints.rs:152-154`、`299-307` `reviewer_boundary_rules_for` | 完整规则如下：<br>“**Design artifact: Work Item Plan、开发任务列表、任务拆分、测试计划、测试范围或场景、测试文件或模块、测试框架或夹具、测试命令、构建命令、执行 checklist 或将测试或验证职责分配给组件或文件必须报告为 must_fix；仅把 [DEC-*] 关联到 [REQ-*]/[AC-*] 且不描述如何测试的抽象验收可追踪性不得报告为 must_fix。**” |
| reviewer nonce few-shot | `prompts.rs:166-199` `reviewer_output_contract`；调用点 `review.rs:82-95` | 统一注入一个 `EXAMPLE_NONCE` sentinel 示例，其 JSON 是通用 `verdict/summary/findings/severity` schema；后面再给真实 nonce 模板。当前**没有**“抽象 DEC→REQ/AC 追踪应不报强 finding”与“可执行测试计划必须 must_fix”的 Design 专属对照判例。 |
| Review→AuthorConfirm | `src/product/workspace_engine/review/routing.rs:5-101` `complete_review`、`route_review_report_to_author_confirm` | Story/Design 无论 reviewer `pass`、`revise` 或 `needs_human`，均将格式化报告写进会话，回到 `AuthorConfirm`；reviewer `pass` 不自动 Confirmed。 |
| 用户自由反馈入口 | `src/product/workspace_engine/prompts/author_revision.rs:7-27` `build_author_revision_prompt` | 仅注入：增量修订规则、当前 artifact（以三反引号包裹）、用户反馈、要求文末追加“改动摘要”。没有 schema、artifact fence 返回格式、Design skeleton、结构化交互决策契约、缺失 context note、历史压缩或未关闭 finding。 |
| `build_author_revision_prompt` 调用方 | `src/product/workspace_engine/prompts/revision.rs:18-72` `build_revision_input_with_resume` | `rg` 结果显示唯一生产调用是 `revision.rs:47`：`pending_revision_context.is_some() && latest_review_verdict.is_none()` 时走该 prompt；直接调用仅见其单元测试 `author_revision_loop.rs:477`。 |
| 单仓 Design 用户反馈是否实际经过该入口 | `decisions.rs:145-179` → `src/web/workspace_ws_handler/decisions.rs:175-229` → `review/drive.rs:411-470` → `revision.rs:45-47` | 是。AuthorConfirm 页面提交 `AuthorDecision::Revise` 后，记录 `pending_revision_context`、进入 `Revision`、spawn `ProviderRunKind::Revision`；构造 revision input 时命中 author-feedback 分支。`RequestRevision` WebSocket 消息是 HumanConfirm 阶段入口，普通 Design 当前常规路径在 AuthorConfirm，不能替代此路径。 |
| 单仓 Workspace 确认 | `decisions.rs:52-190`、`280-325` `handle_author_decision`、`finalize_current_artifact` | 用户可 `AcceptFinalize`，或 `Accept` 后由 review 再回 AuthorConfirm。最终会标记 artifact `confirmed_by=human`、更新 Workspace/Design spec confirmation status、转 `Completed` 并增加 Completed timeline node。单仓没有额外的 Design 内容/业务校验；聚合确认守卫为**超出范围**，单仓不增加校验。 |
| 下游确认消费 | `src/web/handlers/lifecycle.rs:971-1003` `validate_confirmed_design_specs` | 仅检查 Design record 存在且 `confirmation_status == Confirmed`，不重新校验 artifact heading、ID、source traceability 或 reviewer finding。 |
| Runtime N08 Design review | `src/runtime_units/design_review.rs:54-165` `run_design_review` | 独立于 Web Workspace 的 canonical runtime 链：评审 canonical inputs；`pass/conditional_pass` 后将 Design 写入 OpenSpec 并重编 constraint bundle，`revise/fail` 路由 N09。它不在此处写 Lifecycle 的 `Confirmed` 状态。 |
| OpenSpec write/recompile | `src/runtime_units/clarification/openspec.rs:62-83,172-226` `write_design_to_openspec_and_recompile` | 写入当前 change 的 `design.md`，使用 Design projection 生成 OpenSpec 格式，再重建 manifest/bundle；若 bundle 的 design decision 和 component 均为空则失败。实现不按 logical/aggregate 分支，单仓行为相同。 |
| 历史压缩算法 | `src/product/workspace_engine/prompts/history_compaction.rs:7-178,189-335` | 无 `WorkspaceType` 分支。保留最近两轮、最新 artifact、choice audit 原文、Reviewer canonical input；早期轮生成摘要；关联不确定、NUL 或摘要失败时 fail-closed 为全量历史。 |
| 历史压缩三入口 | `prompts.rs:354-360`、`revision.rs:125-135`、`review.rs:47-57` | `build_prompt`、`build_revision_full_prompt`、`build_review_input` 都进入同一 compactor，故 Design 自动适用。resume delta 不压缩历史，author-feedback prompt 完全未调用 compactor。 |
| Design choice/decision 保护 | `parsers/choice.rs:3-23,226-230`；`provider_drive/choice_audit.rs:15-72`；`history_compaction.rs:294-300,317-334,388-415` | choice parser 明确接受 Story/Design；daemon 将 choice 回答持久化为 system audit；compactor 对任何 choice audit 强制原文重放并尝试摘要。Design 的 `[DEC-*]` 是 artifact 内容，没有单独 semantic parser；但 audit 原文受保护。 |
| Review 强 finding 重放 | `history_compaction.rs:173-177` 与 `review.rs:64-68` | compactor 已把未关闭 `blocking/must_fix` 追加到压缩历史；`build_review_input` 又追加一次。多轮且成功压缩时会重复，属于共享 token 冗余。 |
| Design 测试地图：`part_02` | `src/product/workspace_engine/tests/part_02.rs:159-180,284-365` | 覆盖 markdown workspace gate 已启用、Design artifact “外层 fence 已剥离”提示、boundary rule 被注入。Design boundary 断言是 `prompt.contains(完整规则文本)`，不是 candidate→expected finding。前部 artifact contract 实例主要是 Story。 |
| Design 测试地图：`part_10` | `tests/part_10.rs:1-77,890-915` | 验证缺 `source id` 的 Design 会失败；generic reviewer structured value loop 包含 Design。没有完整 Design candidate 的“各 heading/三族 ID/禁止项→具体 finding”覆盖。 |
| Design 测试地图：`part_19` | `tests/part_19.rs:182-236` | Story/Design/WorkItem 共用 reviewer 中断恢复测试；仅证明可恢复，不证明 Design review 边界、结构或用户反馈契约。 |
| Design 测试地图：`part_31` | `tests/part_31.rs:167-184,187-478,919-932` | 覆盖 Design boundary 文案存在、schema label/ID example 被 prompt 注入、review schema gate 文案存在、单仓 Design 不带 structured output contract。多数是 grep/label 存在式断言；没有真实 Design candidate→review finding 对照。 |
| Design 测试地图：author revision | `tests/author_revision_loop.rs:461-509`；`author_revision_review_routing.rs:21-145` | helper 强制 `WorkspaceType::Story`；测试当前 artifact、feedback、改动摘要、分流和 review 后反馈，但没有 Design 用户反馈 revision prompt/完整产物回归。 |
| Design 测试地图：`part_32` | `tests/part_32.rs:347-490` | 验证三个压缩入口、choice audit、强 finding、canonical input、artifact diff，但 fixture 固定 `WorkspaceType::Story`、Story REQ/AC artifact。未证实 Design 多轮 DEC/CMP/API/choice 语义。 |

### Design 结构契约精确摘录

**必需 heading（schema 文案要求二级 heading）：**

1. `设计范围` / `Design Scope`
2. `设计决策` / `Design Decisions`
3. `公共组件` / `Shared Components`
4. `API 契约` / `API Contract`
5. `数据模型` / `Data Model` / `Data Entities`
6. `风险` / `Risks`
7. `追踪关系` / `Traceability`

**必需稳定 ID：** 至少出现一次 `[DEC-*]`、`[CMP-*]`、`[API-*]`。  
**必需 source token：** `source id`，或当前实现接受的 `issue_`、`story_spec_`、`design_spec_`、`work_item_`。  
**禁止 heading：** `Work Item Plan`、`任务拆分`/`Task Breakdown`、`开发任务`/`Development Tasks`、`执行 checklist`/`Execution Checklist`。  
**禁止 token：** `[TASK-*]`、`WI-*`。

结论：validator 确实是“每个 heading/ID/token **至少出现一次**”的存在性校验，不验证逐项覆盖、ID 唯一、ID 语法完整性、ID 所在章节、DEC/CMP/API 的相互追踪，或正文测试计划的自然语言边界。

---

## A2 缺口与修改点清单

> 建议 Change 名称：`harden-single-repo-design-weak-models`。  
> 变更边界：仅 `WorkspaceType::Design` 且普通单仓 Workspace 的 author/review/revision/confirmation/campaign；不触及 aggregate 分支，不改变 Story、WorkItem、WorkItemPlan 的可观察行为。

### D-01：为 Design reviewer 增加边界对照 few-shot

- **文件:符号**
  - `src/product/workspace_engine/prompts/review.rs:37-95` `build_review_input`
  - 可在同文件增加 `design_reviewer_boundary_examples()`；不建议重写通用 `reviewer_output_contract`。
- **现状**
  - 已注入完整文字边界规则、三档 severity、真实 nonce 与 `EXAMPLE_NONCE` 通用 JSON schema。
  - 没有 Design 专属的正反例。
- **问题**
  - 弱模型需要同时理解两条相反边界：抽象 `[DEC-*] → [REQ-*]/[AC-*]` 追踪不可误报强 finding；测试计划/命令/文件/职责分配必须报 `must_fix`。单句长规则的分类稳定性不可由现有字符串断言证明。
- **建议改法**
  - 仅当 `WorkspaceType::Design` 时，紧随 boundary rules 追加 `[design_boundary_examples]`：
    1. 合法案例：DEC 到 REQ/AC 的抽象追踪，无测试步骤、命令、文件或职责分配；明确结果为“不得产生 `blocking/must_fix`”。
    2. 违规案例：明确测试计划、测试命令或组件测试职责；给出 `severity=must_fix`、evidence、最小 required_action。
  - 两个示例均使用 `EXAMPLE_NONCE`，并清楚标明绝不可复用 nonce/ID/文案；真实模板仍沿用现有 nonce。
- **影响面**
  - 仅普通/单仓 Design reviewer prompt；不改变 live severity parser、review gate 或 sentinel parser。
- **测试建议**
  - `src/product/workspace_engine/tests/part_31.rs`：断言 Design prompt 同时含两个 candidate、`EXAMPLE_NONCE`、正例“不报强 finding”、反例 `must_fix`。
  - `src/product/workspace_engine/tests/part_10.rs`：以两个固定 reviewer JSON fixture 走 `parse_review_verdict`，验证正例可用户确认、反例为 `RequiresRevision`；fixture 名称和 evidence 必须对应真实 candidate，而非只 grep 规则文字。
- **优先级：P1**

### D-02：补足 Design artifact contract 的 candidate→finding 回归矩阵

- **文件:符号**
  - `src/product/workspace_engine/tests/part_02.rs`
  - 被测：`src/product/workspace_engine/artifact_constraints.rs:122-155,222-296`。
- **现状**
  - 已有 Design 缺 `source id` 的单一负例；大部分完整性、禁止项和 retry 测试为 Story。
  - `part_31` 对 Design 的多数验证只是 schema/rule label 是否出现在 prompt 中。
- **问题**
  - 不能证明 Design 的 7 heading、三族 ID、source token、禁止 heading/token 在真实 candidate 中对应正确 `ArtifactValidationReport` finding。
- **建议改法**
  - 新增一组完整 Design fixture 与表驱动负例：
    - 合法 candidate：7 heading、`[DEC-001]`、`[CMP-001]`、`[API-001]`、source ids、抽象追踪均通过。
    - 缺任一 heading：断言对应 `missing_required_headings`。
    - 缺任一 ID family / source：断言对应 `missing_required_ids`。
    - `Work Item Plan`、`任务拆分`、`开发任务`、`执行 checklist`：断言 `forbidden_headings`。
    - `[TASK-001]`、`WI-001`：断言 `forbidden_tokens`。
  - “测试计划”自然语言案例不应假装由当前 deterministic validator 拦截；应由 D-01 reviewer fixture 覆盖，明确其预期是 reviewer `must_fix`。
- **影响面**
  - 仅测试；固定普通单仓 Design contract 的现有行为。
- **测试建议**
  - 测试名显式使用 `design_artifact_*_reports_expected_finding`，避免继续使用 `contains("规则文案")` 型断言。
- **优先级：P1**

### D-03：封闭单仓 Design 的用户自由反馈 revision prompt 后门

- **文件:符号**
  - `src/product/workspace_engine/prompts/author_revision.rs:7-27` `build_author_revision_prompt`
  - 复用 `src/product/workspace_engine/prompts.rs:439-467` `append_author_artifact_output_contract`
  - 复用 `prompts.rs:471-506` Design skeleton/decision contract
  - 复用 `prompts/history_compaction.rs:38-75`。
- **现状**
  - 当前只有“增量修订 + 当前 Markdown + 用户反馈 + 改动摘要”。
  - 与 `build_revision_full_prompt` 相比，缺 artifact fence、schema、Design skeleton、结构化 choice/decision 合同、压缩历史、缺失 context note。
- **问题**
  - 单仓 Design 用户在 AuthorConfirm 提交反馈时，确实经过本 prompt；弱模型会失去 `[DEC-*]/[CMP-*]/[API-*]`、source traceability、返回 artifact fence 和 choice 决策的显式约束。
  - Kimi/Pi 不一定获得 artifact retry，不能只依赖最终 gate 失败。
- **建议改法**
  - 在 `WorkspaceType::Design` 分支中追加：
    1. Author 模式的 `compact_history`；
    2. `append_missing_context_notes_to_prompt`；
    3. `append_author_artifact_output_contract(..., true)`；
    4. `author_artifact_skeleton_example(Design)`。
  - 保持 Story/WorkItem/WorkItemPlan 的既有 bytes/语义不变；不要借此把共享后门扩展成跨 workspace 大改。
  - 当前 artifact 的三反引号展示可保留；返回格式必须由新增 artifact output contract 明确规定。
- **影响面**
  - 仅单仓 Design 的 `AuthorDecision::Revise` 路径；不改变 reviewer full/delta revision。
- **测试建议**
  - `author_revision_loop.rs` 新建 Design fixture：验证实际 `build_revision_input()` prompt 含 `[artifact_schema_contract]`、完整 artifact fence 要求、7 heading skeleton、`[DEC-001]`/`[CMP-001]`/`[API-001]` examples、source token 与用户反馈。
  - 使用 provider fixture 输出完整 Design artifact，验证经 artifact gate 后回到 AuthorConfirm，且不创建 aggregate contract。
- **优先级：P1**

### D-04：把多轮历史压缩回归从 Story fixture 扩展为 Design fixture

- **文件:符号**
  - `src/product/workspace_engine/tests/part_32.rs:347-490`
  - 被测：`build_prompt`、`build_revision_full_prompt`、`build_review_input` 和 `compact_history`。
- **现状**
  - 三入口算法对 `WorkspaceType` 无分支，理论上 Design 自动适用。
  - `part_32` fixture 固定 Story，包含 REQ/AC，不能证明 Design 选择、DEC/CMP/API 与多轮 revision 的保留行为。
- **问题**
  - “共享算法覆盖”不等于“Design 输入形态已回归”；目前没有 Design 多轮 candidate、Design choice 映射、Design strong finding 的输入长度/语义守护。
- **建议改法**
  - 将现有测试参数化，或增加等价 Design fixture：
    - 四轮完整 Design artifact；
    - 早期 `author-decision-*`/`[DEC-*]` choice audit；
    - 当前 artifact 含 DEC/CMP/API/source ids；
    - 一个未关闭 `must_fix` Design finding。
  - 三入口分别断言：早期轮被摘要、最近两轮原文保留、choice audit/最新 artifact/强 finding不丢失。
- **影响面**
  - 仅测试；验证共享 compactor 对单仓 Design 的适用性。
- **测试建议**
  - 对 D-03 的 author-feedback prompt 额外断言 choice audit 被输入，避免其再次绕开多轮决策。
- **优先级：P1**

### D-05：新增“review 通过后人工确认”的单仓 Design 红线测试

- **文件:符号**
  - `src/product/workspace_engine/tests/author_revision_review_routing.rs`
  - `src/product/workspace_engine/tests/author_revision_loop.rs`
  - 被测：`review/routing.rs:71-101`、`decisions.rs:95-190,284-325`。
- **现状**
  - review pass 回 AuthorConfirm 的测试使用 Story；Design 只在少量泛型循环中出现。
  - 当前实现对普通单仓 Design 不增加额外业务校验，确认前主要依赖已经执行过的 author artifact gate。
- **问题**
  - 单仓 redline（“不进入 aggregate 分支、不要求 structured metadata、review pass 不自动完成、用户确认后才 Confirmed”）没有端到端回归。
- **建议改法**
  - 不建议为此新增生产业务 gate；新增 lifecycle-backed 单仓 Design 测试，依次验证：
    1. author 完整 artifact 已通过 gate；
    2. reviewer `pass` 后 stage 是 `AuthorConfirm` 而非 `Completed`；
    3. `AcceptFinalize` 后 lifecycle record 状态为 `Confirmed`；
    4. author input/revision input 的 structured output contract 均为 `None`。
- **影响面**
  - 仅测试，锁定单仓行为不被后续 aggregate 事项污染。
- **测试建议**
  - 使用真实 `LifecycleStore` 的单仓 Design record，而不是仅内存 `WorkspaceSession`。
- **优先级：P2**

### D-06：建立单仓 Design 弱模型 campaign 与冻结语料

- **文件:符号**
  - 新建建议目录：`cadence/reports/design-weak-model-campaign/`
  - 参考现有：`cadence/reports/story-weak-model-campaign/`。
- **现状**
  - 现有 Story campaign 有 corpus、digest、manifest、golden normalizer、报告；Design 没有等价产物。
- **问题**
  - 目前无法量化单仓 Design 的首次 artifact 成功、reviewer JSON、边界分类、feedback revision、fresh/resume token usage；不得从 Story sanity 外推。
- **建议改法**
  - 新建而非修改 Story golden 语义，至少包含：
    - `corpus/`：单仓 API/data model、choice→DEC、抽象追踪、测试越界、review-revision；
    - `corpus/digests.txt`：冻结语料哈希；
    - Design 专属 normalizer/golden：比较 DEC/CMP/API/source/traceability 语义；
    - manifest validator、baseline/revised pairing、gate report。
  - 记录 provider×strategy 的 author、reviewer syntax/schema、full-chain、retry、fresh/resume usage；不包含 aggregate metadata 指标。
- **影响面**
  - Cadence 验证产物与脚本；不影响线上代码。
- **测试建议**
  - 为 Design normalizer 追加自身的 golden-diff 单元测试；冻结 corpus 后验证 digest 和 manifest 完整性。
- **优先级：P1**

### D-07：按产品决定收紧 Design “稳定 ID / 二级 heading”的实际 grammar

- **文件:符号**
  - `src/product/workspace_engine/artifact_constraints.rs:222-296,874-906`
  - `src/product/workspace_engine/parsers/choice.rs:233-246`
  - 对应测试放入 `tests/part_02.rs`。
- **现状**
  - schema 要求二级 heading、稳定 ID，但实现接受任意 `#..######` heading 和诸如 `[DEC-]`、`[DEC-*]` 这类仅带前缀 token；围栏代码中的 ID/source token 也可能满足 required check。
- **问题**
  - 弱模型可用模板占位符、代码示例或三级 heading 通过“至少出现一次”gate，偏离结构契约。
- **建议改法**
  - 先由产品确认兼容性后，**仅对 Design**：
    - required/forbidden contract heading 只认 `##`；
    - required ID 只认非空、明确 grammar 的 Design stable ID；
    - required ID/source token 忽略 fenced code，但保持已有 inline-code ID 兼容。
- **影响面**
  - 单仓 Design artifact gate 可能拒绝旧宽松格式，需明确 migration/兼容策略。
- **测试建议**
  - 加入 `### 设计决策`、`[DEC-]`、`[DEC-*]`、仅在 fenced code 中出现的 ID/source 的负例；同时保留 inline-code 正例。
- **优先级：P2**

### D-08：校正 Design skeleton 的 anti-copy 提示语

- **文件:符号**
  - `src/product/workspace_engine/prompts.rs:478-480` `author_artifact_skeleton_example(Design)`
  - `src/product/workspace_engine/tests/part_31.rs:292-374`。
- **现状**
  - Design skeleton 的 anti-copy 原因沿用“缺稳定 ID、REQ/AC 与追踪 token”。
- **问题**
  - `[REQ-*]/[AC-*]` 不是 Design 的 parser-required ID family，降低弱模型对 `[DEC-*]/[CMP-*]/[API-*]` 的注意力。
- **建议改法**
  - 改为“缺 `[DEC-*]`、`[CMP-*]`、`[API-*]`、source id 与追踪 token，不能照抄”；保留 skeleton 本身不含这些内容，确保仍不能通过 gate。
- **影响面**
  - 单仓 Design 初次、retry、full revision prompt 文案。
- **测试建议**
  - Design 专项断言 skeleton 含三族 ID/source id 提示，且剥离 fence 后 validator 仍失败。
- **优先级：P2**

---

## A3 明确不建议改的（防过度工程，给理由）

1. **aggregate Design 分支：超出范围。**  
   包含 aggregate prompt、aggregate write-back、aggregate output parser、aggregate confirmation validation 等指定符号；本审计未深入，也不应混入本 Change。

2. **不要重写 sentinel nonce parser、三档 severity enum、history compactor 或 Kimi provider 服务。**  
   它们已是共享层。单仓 Design 应通过 D-01/D-04/D-06 验证消费效果，而非复制协议实现，避免重新出现协议漂移。

3. **不要立即把“测试”“命令”“文件”等宽泛关键词加入 deterministic Design validator。**  
   当前风险边界依赖语境：抽象验收说明、风险描述可能合法使用相近词。应先采用 D-01 的最小对照案例与 D-06 实测；只有产品确认无歧义形式后，才考虑 D-07 的有限 pre-gate。

4. **不要给 `build_revision_delta_prompt` 无条件补全量 skeleton/history。**  
   它面向 provider resume，会话中已有 artifact 与上下文；强行重复会扩大 token。D-03 的问题是**用户自由反馈**入口，而不是 resume delta。

5. **不要在 `build_prompt` 再复制完整 system output schema。**  
   标准 Web 链路已经通过 workspace context 注入 fence、决策和 parser schema；`build_prompt` 也以 marker 防重复。应优先修复实际绕过该 system context 的 Design author-feedback prompt。

6. **不要把 Runtime N08 的 OpenSpec write/recompile 与 Workspace 的人工 Confirmed 强行耦合。**  
   两者是不同的运行时链路和持久化语义；单仓弱模型加固只需为当前 Workspace 确认路径补红线测试，不应顺带改写生命周期架构。

7. **本 Change 不建议顺带清理 reviewer 强 finding 双重重放。**  
   该问题由 `history_compaction.rs:173-177` 与 `review.rs:64-68` 的共享组合造成，影响所有 workspace。应作为独立共享 token 优化事项；当前单仓 Design 只补 D-04 的实测和 token 记录。

8. **本 Change 不应把 `build_author_revision_prompt` 的修复扩展到 Story/WorkItem。**  
   它确实是共享后门，但严格范围要求本次只在 `WorkspaceType::Design` 分支补约束；跨 workspace 统一修复应单独立项并执行兼容性回归。

---

## A4 开放问题（需产品决策或实测数据的）

1. **Design reviewer 边界的产品判定面**  
   “测试策略”在 Design 中允许到什么粒度？例如“风险需要回归验证”是否允许，而“使用 cargo test --locked”是否必定 `must_fix`？D-01 的反例需要产品给出最终可执行边界。

2. **稳定 ID grammar 与 heading 兼容性**  
   是否接受历史 Design 的三级 heading、非数字 ID、`[DEC-*]` 模板占位？若不接受，D-07 需要 migration 或仅对新生成 artifact 生效的决策。

3. **source traceability 的充分性**  
   当前任一 `issue_`/`story_spec_` 片段即可满足 source token。产品是否要求每个 DEC/CMP/API 至少关联上游 source，还是本 Change 只维持“可见来源存在”的既有 contract？

4. **用户自由反馈修订的历史预算**  
   D-03 若将 compaction 接入 feedback prompt，需确认保留“最近两轮 + 全部 choice audit + 当前 artifact”是否在 Kimi/Pi 的 token 预算内；应由 D-06 按 fresh/resume/feedback 三种口径测量。

5. **单仓 Design campaign gate 阈值**  
   需要先确定 provider 组合、样本量、超时、重试计数口径和可接受的首轮/full-chain 指标；在有冻结 corpus 与配对基线前，不应声明成功率或 token 降幅目标已满足。

6. **确认时的持久化失败语义**  
   `finalize_current_artifact` 对 lifecycle 状态更新当前采用忽略错误的方式。它不是弱模型特有问题，但若产品要求“用户界面显示 Confirmed 必须等价于落库成功”，应另开生命周期可靠性 Change，而非混入本单仓 Design 加固 Change。

## Files Changed

- 无。任务明确要求只读审计，未创建 OpenSpec change 文件或修改任何源文件。

## Verification

| 命令 | 结果 |
|---|---|
| `ast-grep outline` 检查 artifact constraints、prompts、review、author revision、runtime unit、测试文件结构 | 通过；按代码阅读规则先获得符号与行区间。 |
| `codegraph explore 'WorkspaceEngine build_author_revision_prompt call paths Design revision' --max-files 12` | 通过；确认唯一生产调用位于 `prompts/revision.rs`，并识别 WebSocket→revision driver 调用链。 |
| `codegraph explore 'compact_history build_prompt build_revision_full_prompt build_review_input workspace' --max-files 12` | 通过；确认三入口共享 compactor。 |
| `rg` + 定向 `nl -ba` 审计 artifact、prompt、review、lifecycle、runtime、tests | 通过；所有结论均有文件:行证据。 |
| `git status --porcelain=v1 && git diff --quiet && git diff --cached --quiet && git diff --check` | 通过；工作树无 diff、暂存区为空、无 whitespace error。 |
| `cargo test` | 未运行；本任务为严格只读审计，未实施代码或测试变更。 |

## Notes

- 预先存在 `.codegraph/codegraph.db`，本次使用其只读 explore 能力；未执行会写入索引的初始化操作。
- aggregate 相关符号和逻辑均未纳入修改建议或深挖结论。
- 推荐先实施 D-01～D-04 与 D-06；D-07 必须等待产品兼容性决策。

```acceptance-report
{
  "criteriaSatisfied": [
    {
      "id": "criterion-1",
      "status": "not-applicable",
      "evidence": "本任务被明确限定为只读代码级审计，未授权实施 OpenSpec change；已输出严格限定单仓 Design 的文件、符号、行号、修改建议与测试清单。"
    }
  ],
  "changedFiles": [],
  "testsAddedOrUpdated": [],
  "commandsRun": [
    {
      "command": "ast-grep outline src/product/workspace_engine/{artifact_constraints.rs,prompts.rs,prompts/review.rs,prompts/author_revision.rs}",
      "result": "passed",
      "summary": "获得目标文件的结构化符号与行区间。"
    },
    {
      "command": "codegraph explore 'WorkspaceEngine build_author_revision_prompt call paths Design revision' --max-files 12",
      "result": "passed",
      "summary": "确认用户反馈 revision 的生产调用关系。"
    },
    {
      "command": "codegraph explore 'compact_history build_prompt build_revision_full_prompt build_review_input workspace' --max-files 12",
      "result": "passed",
      "summary": "确认三条 Design prompt 入口共用历史压缩器。"
    },
    {
      "command": "git status --porcelain=v1 && git diff --quiet && git diff --cached --quiet && git diff --check",
      "result": "passed",
      "summary": "工作树无变更、无暂存文件、无 diff 格式问题。"
    }
  ],
  "validationOutput": [
    "完成 artifact contract、author/revision/review、单仓确认、runtime writeback、history compaction 与指定测试地图的定向代码审计。",
    "未运行 cargo test；原因是只读审计任务未修改实现或测试。"
  ],
  "residualRisks": [
    "Design reviewer 边界分类仍主要依赖弱模型对长规则的理解，暂无专属正反例或真机统计。",
    "用户自由反馈 Design revision 当前缺 schema、artifact fence、skeleton 和历史/choice 约束。",
    "Design ID、heading level 与 source token 的实际 validator 比文案契约宽松。",
    "单仓 Design 尚无冻结弱模型 campaign，成功率、token 与边界误判率未测定。"
  ],
  "noStagedFiles": true,
  "diffSummary": "无代码或文档差异；仅完成只读审计报告。",
  "reviewFindings": [
    "no blockers: 未发现需要在本只读审计任务中直接修改的工作树差异。",
    "P1: 应在后续单仓 Design Change 中补 reviewer 边界 few-shot、用户反馈 revision contract、Design 多轮回归与冻结 campaign。"
  ],
  "manualNotes": "aggregate 相关实现按范围要求标记为超出范围，未提出修改建议。"
}
```