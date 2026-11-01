# Tasks

所有任务本轮已实现（后端 + 前端 + 契约）。检查项以实际跑过的命令为准，未跑的门禁不勾。

## 1. 契约层（REQ-WSC-11）

- [x] 1.1 `src/web/workspace_ws_types/review.rs`：新增 `OwnerOption { label, consequence }`（`deny_unknown_fields`）；`ReviewFinding` 增 `owner_question: Option<String>` 与 `owner_options: Option<Vec<OwnerOption>>`，均 `#[serde(default, skip_serializing_if = "Option::is_none")]` 保持旧 wire 字节不变。
- [x] 1.2 `src/product/workspace_engine/parsers.rs`：新增诊断码 `ReviewStructuredOutputErrorCode::MissingOwnerGuidance` → `missing_owner_guidance`；严格适配器 `parse_raw_review_finding` 白名单放开两字段，宽松适配器 `parse_review_findings` 同步解析（同一 payload 不同入口结论一致）。
- [x] 1.3 fail-closed：`owner_guidance_error(requires_owner_decision, findings)` + `validate_owner_guidance_pairing`。**通用解析路径按「模型宣称 `needs_human`」触发**（不看归一后 gate——`blocking`/`must_fix` 会把它归一成 `revise`）；WorkItemPlan 解析路径按 `effective_verdict == NeedsHuman` 触发；历史重解析入口按 `OwnerGuidancePolicy::Historical` **不强制**。空白问题/空选项/有效选项 <2 归一为未提供 → 先报 `missing_owner_guidance`，再查半截字段 `invalid_finding_field`。
- [x] 1.4 不合规处置复用既有 `fallback_review_verdict` → `structured_output_diagnostic`（不新增上报/展示机制）；诊断码随 `needs_human` 结论卡到达前端既有 `StructuredOutputDiagnosticView`。

## 2. Prompt 层

- [x] 2.1 `src/product/workspace_engine/prompts.rs`：新增 `OWNER_GUIDANCE_RULES`（人话、禁内部术语、2–3 互斥选项、`pass`/`revise` 留空），由 `reviewer_output_contract` 统一注入，7 处 reviewer prompt builder 全继承。
- [x] 2.2 7 处示例 schema 补字段形状：`prompts/review.rs`（通用评审 / 单候选 outline / Plan Projection / Outline 评审 / Batch 评审）、`prompts/review_parts/draft_input.inc.rs`（单 Draft 评审）、`prompts/review_parts/single_candidate_input.inc.rs`（单候选 Plan 评审）。原 `"findings":[]` 的两处改为带字段 finding 示例，避免模型把字段写到顶层被静默丢弃。
- [x] 2.3 `pass`/`revise`（含 `revise_batch`、`plan_reopen_required`）明确留空，不加新要求，gate 优先级不变。

## 3. 界面层

- [x] 3.1 `web/src/api/types/workspace.ts`：新增 `WorkspaceOwnerOption`，`WorkspaceReviewFinding` 增可选 `owner_question`/`owner_options`。
- [x] 3.2 `web/src/components/chat-workspace/finding-list.tsx`：`ReviewFinding` 增两字段；新增 `reviewOwnerGuidance(findings)` 取第一条完整人话裁决（缺任一字段即视为没有，不拼接不猜测）。
- [x] 3.3 `web/src/components/chat-workspace/entries/ReviewVerdictEntry.tsx`：`needs_human` 且有人话裁决时先渲染「需要你决定一件事」区块（问题 + 选项 label/consequence + 「把选项名原文作为补充指令发回去」指引），技术 findings 折叠进「技术细节」`<details>`（默认收起、不删除）；`pass`/`revise` 渲染逐字不变。
- [x] 3.4 数据链路：字段随 findings 走既有通道进入 metadata——live `workspace-ws-message-handler.ts`（整体透传 `findings`）与 rebuild `workspace-chat-rebuild.ts`（`getArrayField(detail.verdict, "findings")`）均无需改造。

## 4. 测试（TDD 红 → 绿）

- [x] 4.1 后端 `parsers.rs::owner_guidance_tests`（13 条）：缺字段/空白问题/空选项 → `missing_owner_guidance`；完整字段解析并逐字保留；`pass`/`revise` 不要求；派生分诊（`revise` 无 finding）保留既有 summary/comments；半截字段 → `invalid_finding_field`；WorkItemPlan outline `needs_human` 同上；`revise_batch`/`plan_reopen_required` 不要求；`fallback_review_verdict` 诊断码 = `missing_owner_guidance`。
- [x] 4.2 前端 `ReviewVerdictEntry.test.tsx`（新增 4 条）：人话问题 + 选项 label/consequence + 指引可见；技术细节在 `details` 内且默认收起；`pass` / `revise` 无回归。
- [x] 4.3 既有构造点适配：`ReviewFinding` 新增两字段后 23 处字面量补齐 `None`（编译驱动定位，不改语义）。
- [x] 4.4 既有模型输出 fixture 适配：`part_03/part_01.rs`、`part_18.rs`、`part_10.rs` 的 `needs_human` 样本补人话字段（`revise_batch`、`InvalidOutlineReference`、`InvalidReviewScope` 等前置错误路径不动）。

## 5. 门禁（2026-10-30 本地实测）

- [x] 5.1 `cargo test --locked --lib`：5696 passed / 0 failed / 7 ignored。
- [x] 5.2 `cargo test --locked --test it_core`：190 passed / 0 failed；`cargo test --locked --test it_web`：591 passed / 0 failed / 17 ignored。
- [x] 5.3 `cargo clippy --all-targets --all-features --locked -- -D warnings` 零告警（Finished dev profile）；`cargo fmt --all -- --check` 干净。
- [x] 5.4 `cargo test --locked --test it_core large_file_guard`：2 passed（`parsers.rs` 1656→1178 行已按子模块拆分；三个合法增长文件在 `GRANDFATHERED` 按 2026-10-30 先例登记，编号注释用 REQ-WSC-11 区分 B 的 REQ-WSC-10）。
  - **owner 裁决留痕（2026-10-30）**：三条 `GRANDFATHERED` 上调（`contract_prerevision.rs` 1911→1913、`prompts.rs` 1665→1681、`tests/single_candidate.rs` 1545→1551）**经 owner 明确批准「先上调」**（选项 A）。owner 同时指示「记录下这个问题，还有不少文件超过行数上限的，需要单独处理一下」→ 该结构性问题已独立立项：`cadence/notes/2026-10-30_待办_行数上限治理_v1.0.md`（实测：登记 109 个文件、**89 个已顶格**、0 超限；建议方案 = 登记表改目录级上限 + 分批拆最大的）。**本 change 的三条上调不依赖该待办**。
- [x] 5.5 `pnpm -C web test`：245 files / 2487 tests 全绿；`pnpm -C web build` 通过（tsc -b + vite）。
- [x] 5.6 `openspec validate fix-owner-readable-human-gate --strict` exit 0。

## 6. k3 审查修复（2026-10-30）

- [x] 6.0【必修】门卡 `GatePromptEntry` 渲染人话区：与结论卡共用 `finding-list.tsx::ReviewOwnerGuidanceBlock`（唯一实现），人话区置于 `gate-findings` 折叠区之前，无字段时整块不出现；用例断言 DOM 有 `owner-question` / 两个 `owner-option` / `owner-option-guidance` 且 `gate-findings` 保留。
- [x] 6.1【F1】`owner_options` 有效项下界 `>= 2`（`MIN_OWNER_OPTIONS`），上限不设；前端 `reviewOwnerGuidance` 同口径；spec 正文措辞同步为「至少 2 个」。
- [x] 6.2【F2】可见性改由「是否携带人话字段」驱动（不用 verdict 字面），强制面保持「宣称 needs_human」→「凡被要求写 ⇒ 结论卡与门卡均可见」。
- [x] 6.3【F3】`OwnerGuidancePolicy::{Live,Historical}`：两个 historical 入口不强制，保留既有 summary/findings；design 兼容性段补该旁路。
- [x] 6.4【F4】`verification_scope_error_if_needed` 精确豁免 `Schema(MissingOwnerGuidance)`（不动 `is_classification_fatal`），复评轮漏写人话落 `missing_owner_guidance` 诊断卡而非 durable failed；design Risks 行已改正。
- [x] 6.5【观察 2】prompt 契约钉：`prompts/owner_guidance_rules.rs::tests::every_reviewer_prompt_contract_carries_owner_guidance_rules`（含 7 处 builder 源码级断言）。
- [x] 6.6【观察 3】tasks 1.3 描述已按实现口径改正。
- [x] 6.7【Concern 1】三条 `GRANDFATHERED` 上调**已获 owner 批准（2026-10-30，选项 A：先上调）**，留痕 `cadence/notes/2026-10-30_待办_行数上限治理_v1.0.md`；本轮未再动代码。
- [x] 6.8【Concern 3】changelog 不做本 change 交付项：`web/src/whats-new/changelog.ts` 属发版号维度，**下次发版（0.0.10）时并入 highlights**（owner 可读的人工裁决呈现）。
- [x] 6.9 行数门禁再拆：`parsers.rs` 1053 行（抽出 `parsers/lenient_findings.rs`）、`prompts.rs` 1656 行（抽出 `prompts/owner_guidance_rules.rs`），均未新增登记项。
