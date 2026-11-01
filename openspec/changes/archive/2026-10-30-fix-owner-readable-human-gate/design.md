# 设计：owner 可读的人工裁决（needs_human 人话化）

## Context

需求来源是 owner 2026-10-30 真机现场：`needs_human` 结论卡上唯一可见的是 reviewer finding 的技术原文（`write_policy.exclusive_scopes`、`fingerprint`、`AC-00x`、`done_when_refs`），答案虽然在里面，但 owner 「看不懂 → 写不出补充指令」。

三处事实决定了本设计的最小切口（均已在本轮实现中核实）：

| 锚点 | 观察事实 | 设计含义 |
|---|---|---|
| `src/product/workspace_engine/prompts/review.rs` 等 7 处 schema | finding 只有 `severity/message/evidence/required_action`，没有「问 owner 一个问题」的位置 | 模型想「请你选 A 还是 B」只能塞进 `required_action` 自由文本 → 必须扩字段，不能只改文案 |
| `src/product/workspace_engine/review/structured_output.rs` | 解析失败一律经 `fallback_review_verdict` 落 `needs_human` + `StructuredOutputDiagnostic`（前端已有 `StructuredOutputDiagnosticView`） | 不合规输出复用这条既有通道，不新造上报/展示机制 |
| `web/src/hooks/workspace-ws-message-handler.ts`、`web/src/state/workspace-chat-rebuild.ts` | live 路径整体透传 `findings` 数组；rebuild 路径 `getArrayField(detail.verdict, "findings")` 整体透传 | 字段挂在 findings 上即可零改动进入 metadata，不需要第二套投影 |

## Goals / Non-Goals

**目标**

- `needs_human` 的 owner 呈现义务可判定：人话问题 + 互斥选项 + 后果 + 一句作答指引。
- 不合规输出 fail-closed，且诊断码可行动（`missing_owner_guidance`）。
- 全部面向 owner 的 reviewer 路径同一口径（7 处 prompt builder）。
- `pass`/`revise` 渲染零回归。

**非目标**

- 不解析 owner 的回答，不生成命令、不出站。
- 不改 review gate 优先级（`blocking`/`must_fix` 仍压过 `needs_human`），不改任何 confirm/compile/advance 许可。
- 不删技术 findings（只折叠）。
- 不改 coding 侧 code review 契约（verdict 词汇与门语义属另一 capability）。

## Decisions

### D1：字段挂在 `findings[]` 上，而不是 verdict 顶层

- 现场模型是把「请你选 A/B」写在触发问题的那个 finding 的 `required_action` 里，字段跟着 finding 走符合模型的自然位置；
- 前端两条路径（live `review_complete`、rebuild `detail.verdict.findings`）本来就把 findings 数组整体透传，挂在 findings 上等于零改造；
- verdict 顶层新增字段需同步 `EngineEvent`、WS DTO、rebuild、REST detail 四处投影，收益为零。

`OwnerOption { label, consequence }` 用 `deny_unknown_fields`，`ReviewFinding` 两个新字段是 `Option` + `skip_serializing_if`：旧记录、旧 wire 字节逐字不变（严格解析器的字段白名单同步放开，否则同一 payload 会被路由层的严格适配器判 `invalid_finding_field`）。

### D2：fail-closed 的触发口径（按路径各取"需要 owner 决定"的真语义）

**下界**：`owner_options` 有效项 `>= 2`（`MIN_OWNER_OPTIONS`）才算合规——「唯一选项」不构成互斥选择，owner 仍无从作答；上限不设（模型给 4 个选项不是缺陷）。前端 `reviewOwnerGuidance` 用同一口径，避免「后端判合规、前端渲染不出选择感」。

- **通用 workspace 评审**（`parse_review_value`）：触发条件 = 模型**宣称** `needs_human`。
  为什么不用归一后的 gate：`blocking`/`must_fix` finding 会让 `review_gate_for` 返回 `requires_revision`（gate 优先级不变），而现场那条 finding 正是 must_fix 级别——只看 gate 会漏掉「模型明确要人」的真实意图。反过来，`revise` 无 finding 被升级为人工分诊属**派生分诊**（模型自称 revise，并未宣称需要人决定），`part_10::parse_review_verdict_reads_json_contract_from_tail_block` 等既有契约要求保留其 summary/comments，故不在本条强制面（已用测试 `revise_without_findings_keeps_existing_human_triage_without_owner_guidance` 钉住）。
- **历史重解析入口**（`parse_historical_review_json` / `parse_historical_work_item_plan_review_json`）：**不强制**（`OwnerGuidancePolicy::Historical`）。存量 reviewer 消息文本在本 change 之前不可能带人话字段，若沿用同一校验会解析失败 → `.ok()?` 变 `None` → 恢复链把既有 `summary`/`findings` 丢成兜底摘要，作者修订轮也拿不到待关闭 finding。live 入口对同一 payload 仍 fail-closed。
- **WorkItemPlan 评审**（`parse_work_item_plan_review_value`）：触发条件 = `effective_verdict == NeedsHuman`（等价于宣称 `needs_human`：`malformed` 已在前一步报错，`pass`+blocking 被归一为 `revise`）。
  为什么不是「最终 generic verdict 是 `needs_human`」：`revise_batch` 与 `plan_reopen_required` 也映射到 generic `needs_human`，但它们是**系统自动返修动作**（重写 batch / 重开 outline），不向 owner 提问；对它们强制提问条款是错的。

### D2b（F2）：呈现以「是否携带人话字段」为准，不以 verdict 字面为准

后端强制面保持「模型宣称 needs_human」；**可见性**改由字段存在驱动：结论卡与门卡只要本轮 findings 携带完整人话裁决就渲染人话区块，不看 `verdict` 字面。理由：

- 宣称 `needs_human` 但同轮存在 `blocking`/`must_fix` 时，`review_gate_for` 会把归一 verdict 变成 `revise`；若可见性只看 `verdict==="needs_human"`，这些**被强制要求写**的字段就成了「必须写、永不显示」的死字段（F2）。
- 反过来，选 (b)「强制面收窄到归一后 verdict」会连带把「`revise` 无 finding 被升级为人工分诊」也纳入强制，破坏既有契约（`part_10::parse_review_verdict_reads_json_contract_from_tail_block` 要求保留 summary/comments）。
- 于是采用「强制面 = 宣称 needs_human；可见面 = 字段存在」：**凡被要求写的 payload，其字段必在结论卡与门卡之一可见（实际两张都可见）**，且不携带字段的 `pass`/`revise`（全部契约合规 payload 与全部存量记录）呈现逐字不变。

### D3：先报 `missing_owner_guidance`，再查半截字段

空白/空数组在解析期归一成「未提供」，于是「`owner_question` 空白」与「完全没写」同判 `missing_owner_guidance`（可行动的那个码）；只有在**不**要求人话字段的结论里出现半截字段（只填问题或只填选项）才判 `invalid_finding_field`。顺序反过来会把"空白问题"误报成结构错误，让模型无从修复。

### D4：单一 prompt 口径常量

`prompts.rs::OWNER_GUIDANCE_RULES` 由 `reviewer_output_contract` 统一注入，7 处 reviewer prompt builder 自动继承，杜绝「一类有人话、一类没有」的分叉；各 builder 的示例 schema 另行补齐字段形状（两处原本是 `"findings":[]` 的改为带字段的 finding 示例，避免模型把 `owner_question` 写到顶层被静默丢弃）。文案显式禁止内部术语，并给出反例（`write_policy`、`exclusive_scopes`、`done_when_refs`、`AC-001`、`REQ-*`）。

### D5：界面只做「主区人话 + 折叠技术」

`ReviewVerdictEntry` 仅在 `verdict === "needs_human"` 且 findings 中存在完整人话裁决时切换呈现（`reviewOwnerGuidance` 取第一条完整的）；此时技术 findings 全部折进 `<details>技术细节</details>`。缺字段（历史记录、诊断卡）时退回既有渲染——前端不拼接、不猜测，讲解义务由服务端 fail-closed 保证。`pass`/`revise` 分支一行未动。

## Risks / Trade-offs

| 风险 | 处置 |
|---|---|
| 模型忘记新字段 → 整轮判不合规，owner 只看到诊断卡 | 这是有意的 fail-closed：诊断卡仍显示 reviewer comments 与原始输出预览，且 prompt 条款 + schema 示例双管齐下；相比"给 owner 一段看不懂的技术文"，可行动性更高 |
| `revise` 无 finding 被升级为人工分诊时仍无人话字段 | 属**派生分诊**（模型自称 revise，未宣称需要人决定），既有契约（`part_10::parse_review_verdict_reads_json_contract_from_tail_block`）要求保留其 summary/comments，故本 change 明确不覆盖该状态；prompt 已声明 `revise` 必须带 finding，长期由主模型行为 + 该条款收敛 |
| 多仓 finding 完整性门（`malformed_findings`）与新码交叉 | 完整性门在前，返回 `malformed_findings`；新码只在其后被判定，两条诊断码互不覆盖 |
| 单候选 Verification scope 的解析失败升级（F4，已修） | 该 scope 既有合同会把它升级成 `verification_scope_violation` → durable failed 并**掩掉诊断码**——对「needs_human 漏写人话」这是本 change 新引入的错裁决。故在 `verification_scope_error_if_needed` 精确豁免 `Schema(MissingOwnerGuidance)`（保留 schema 错误 → needs_human + 诊断卡）；`is_classification_fatal` 不动（`drive.rs` 也用它，会把该码在所有阶段变成 abort） |
| prompt 字节预算（单候选 96KiB） | 新增约 400 字节；实测单候选评审 prompt 仍远低于上限 |

## Migration / Compatibility

- 旧会话记录：`ReviewFinding` 两字段 `Option` + `default`，反序列化为 `None`，rebuild 卡按既有渲染；不需要数据迁移。
- 旧 wire：`skip_serializing_if` 保证无这两字段的记录序列化字节不变（与 SG7 `machine_repair` 同一纪律）。
- **存量 reviewer 消息文本重解析**（`latest_review_verdict_from_messages` → `parse_historical_*`）：按 `OwnerGuidancePolicy::Historical` 不强制人话字段，保留既有 findings/summary；否则恢复链会丢掉待关闭 finding 清单（F3）。
- 无 provider 协议变更：仍是一次性 reviewer 调用 + 同一 sentinel JSON envelope，只是 finding 多了两个可选键。

## 覆盖的 needs_human 产出路径（本 change 全部覆盖）

| # | Prompt builder | 落点 |
|---|---|---|
| 1 | 通用 workspace 评审 `build_review_input` | `prompts/review.rs` schema + `reviewer_output_contract` 注入条款 |
| 2 | 单候选 WorkItemPlan outline 评审 `build_work_item_plan_review_input` | 同上 |
| 3 | Plan Projection 评审 `build_projection_plan_review_input` | 同上（原 `"findings":[]` → 带字段 finding 示例） |
| 4 | Work Item Plan Outline 评审 `build_work_item_plan_outline_review_input` | 同上 |
| 5 | 单 Work Item Draft 评审 `build_work_item_draft_review_input` | `prompts/review_parts/draft_input.inc.rs` |
| 6 | 整 Batch 评审 `build_work_item_batch_review_input` | `prompts/review.rs` |
| 7 | 单候选 Plan 评审 `build_single_candidate_plan_review_input` | `prompts/review_parts/single_candidate_input.inc.rs`（原 `"findings":[]` → 带字段 finding 示例） |

校验侧两处：`parsers/…`（通用 `parse_review_value_with_owner_policy` 与 `parse_work_item_plan_review_value_with_owner_policy`，outline/item/batch）；字段解析在严格适配器 `parse_raw_review_finding` 与宽松适配器 `parse_review_findings`（已移入 `parsers/lenient_findings.rs`）两条都放开，避免同一 payload 在不同入口得到不同结论。人话条款常量在 `prompts/owner_guidance_rules.rs`，注入点仍是唯一 `reviewer_output_contract`。
呈现侧两处：`entries/ReviewVerdictEntry.tsx`（结论卡）与 `entries/GatePromptEntry.tsx`（门卡），共用人话区块 `finding-list.tsx::ReviewOwnerGuidanceBlock`。

## 行数门禁与拆分留痕

- `parsers.rs` 1656 → 拆出 `parsers/owner_guidance.rs`（解析+校验+测试）、`parsers/error_codes.rs`（错误码纯迁移）、`parsers/lenient_findings.rs`（宽松 findings 解析族），现 **1053 行**（未登记文件，硬线 1200）。
- `prompts.rs`：`OWNER_GUIDANCE_RULES` 与其契约测试移入 `prompts/owner_guidance_rules.rs`，现 **1656 行**（登记上限 1681，未再上调）。
- 三条 `GRANDFATHERED` 上调（`contract_prerevision.rs` 1911→1913、`prompts.rs` 1665→1681、`tests/single_candidate.rs` 1545→1551）**已获 owner 批准（2026-10-30，选项 A：先上调）**，留痕见 `cadence/notes/2026-10-30_待办_行数上限治理_v1.0.md`；本轮不再改动代码。

## Test Plan（红 → 绿）

- 后端（`parsers.rs::owner_guidance_tests`）：`needs_human` 缺字段/空白问题/空选项 → `missing_owner_guidance`；带完整字段 → 解析成功且字段逐字保留；`pass`/`revise` 不要求；半截字段 → `invalid_finding_field`；WorkItemPlan outline `needs_human` 同上，`revise_batch`/`plan_reopen_required` 不要求；诊断通道测试断言 `fallback_review_verdict` 的 `diagnostic.code == "missing_owner_guidance"`。
- 前端（`ReviewVerdictEntry.test.tsx`）：`needs_human` 渲染问题/选项/后果/指引，技术细节折叠在 `details` 内且默认收起；不携带字段的 `pass`/`revise` 不出现人话区、findings 直接展示；携带字段的 `revise`（宣称 needs_human 被归一）人话区可见（F2）；仅 1 个选项不渲染（F1）。
- 前端（`GatePromptEntry.test.tsx`）：门卡带人话字段时渲染问题 + 选项 + 指引且保留 `gate-findings`；不带时不渲染空白块；1 个选项不渲染。
- prompt 契约钉（`prompts/owner_guidance_rules.rs::tests`）：`reviewer_output_contract` 必含 `OWNER_GUIDANCE_RULES` 与两个字段名；源码级断言 7 处 builder 全部注入且 schema 含 `owner_question`（观察 2）。
- 历史重解析（`historical_review_reparse_does_not_require_owner_guidance`）：存量 `needs_human` 保留 summary/findings，live 同 payload 仍不合规（F3）。
- 协议豁免（`verification_scope_missing_owner_guidance_stays_schema_error_not_protocol_fatal`）：复评轮漏写人话 → `missing_owner_guidance`，不是 `verification_scope_violation`（F4）。
