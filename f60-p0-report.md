# F-60 P0 实施报告：Markdown 契约出口强制注入（方案因素三/五）

- 日期：2026-09-26
- 状态：已实施（P0 全部四项；验收见下）
- 分支：feat-b-0808-add-monorepo（worktree），BASE 8ba23f47
- 方案：`cadence/designs/2026-09-25_技术方案_F60一次成功率_v1.0.md`（因素三「上下文漂移与统一契约注入点」/因素五 P0）
- 提交：fix(prompt) 前缀

## 一、改动摘要

### 1. 唯一出口装配函数（方案：同一装配函数合并 fence/一级标题/gate 正文/choice 决策归档）

`src/product/workspace_engine/prompts.rs` 新增 `markdown_author_output_contract_block(workspace_type, mentions_prior_artifact)`：

- 以 `artifact_constraint_spec_for` 为唯一规则源（`author_artifact_schema_contract_for` 渲染）；
- 合并：fence／一级标题／内部代码块四反引号处理与旧稿裸 markdown 防误教 → 按类型 gate 正文（**无条件渲染当前类型全条款**）→ choice 决策归档（`author-decision-*` 绑定）→ 负面清单（`AUTHOR_ARTIFACT_NEGATIVE_LIST`）→ 结构骨架；
- **移除了旧 `append_author_artifact_output_contract` 的 `!prompt.contains(ARTIFACT_SCHEMA_CONTRACT_MARKER)` 省略逻辑**——末端合同不再因 prompt/历史中出现旧 marker 而被跳过（方案红线）。

### 2. 出口级注入（方案：置于最终构造 StreamingProviderInput.prompt 的出口）

| 出口 | 覆盖变体 | 改动 |
|---|---|---|
| `build_streaming_input` | FullConversation（初次）+ DeltaOnly（choice 应答续跑、design 增量） | 基线教学后出口装配：初次＝完整、Delta＝短引用（四个 Markdown 类型） |
| `build_revision_input_with_resume` | 用户反馈（有/无 resume）、reviewer delta/full、Codex resume-stall fresh（`build_revision_input_without_resume`） | 出口分层装配：resume 增量轮＝短引用；fresh 轮＝完整装配＋mentions 旧稿防误教 |
| `build_work_item_plan_streaming_input(_fresh/_with_session)` | 计划流直发 | 新增显式合同族参数 `PlanAuthorOutputContract`：`Structured`（现状全部调用方：JSON Outline/Split/Draft + SC compiler-source）不拼 Markdown 合同；`MarkdownArtifact` 注入同一装配（WorkItemPlan gate 全条款） |

### 1b. 分层合同注入（用户裁决 2026-09-26，P0 必改项）

出口装配按「轮次类型」二态分发（`markdown_author_output_contract_for_round`）：

- **初次生成 = 完整装配**（现状语义不变）：FullConversation、fresh 修订轮（无 resume id，含 Codex resume-stall fresh——fresh provider 会话无会话内记忆，prompt 必须自足，方案因素一）、计划流 MarkdownArtifact 直发。
- **后续轮 = 一行短引用**（`markdown_author_output_contract_short_reference`，条目由 `author_artifact_short_reference_items` 从同一 `artifact_constraint_spec_for` 规则源渲染）：DeltaOnly 续跑（choice 应答）、resume 增量修订（用户反馈/reviewer delta）。内容＝fence/H1/四反引号提醒 + **当前类型全部必需二级 heading** + **全部必需稳定 ID 与追踪 token（[REQ-*]/[AC-*]/source id 按类型置换）** + 决策归档指针（author-decision-*）+ 指向会话开头 `[artifact_schema_contract]` 完整合同；不重复 schema 全文/负面清单/骨架。
- 短引用有效性由 P2 真实 provider 测量验证，不够再升级完整装配（用户裁决原文）。

### 1c. prompt 质量预算常量（story/design markdown author 族）

`prompts.rs` 新增两档预算（参照 plan 链 `WORK_ITEM_PLAN_MARKDOWN_PROMPT_MAX_BYTES` 先例）：

| 常量 | 值 | 分层后实测（2026-09-26） |
|---|---|---|
| `MARKDOWN_AUTHOR_PROMPT_MAX_BYTES`（初次完整装配，empty-session 夹具） | 6,000 B | Story 3,583 B / Design 3,076 B |
| `MARKDOWN_AUTHOR_DELTA_REFERENCE_MAX_BYTES`（后续轮短引用） | 1,200 B | Story 659 / Design 713 / WorkItem 587 / WorkItemPlan 645 B |

短引用较完整装配块（1-3KB）每轮省约 80%，N 轮续跑不再线性膨胀。历史与基线注入不计入该预算（由既有滑动窗口/基线预算约束）。

### 1d. 双测算裁决修正（oracle+max 互证，2026-09-26 第二轮）

**短引用仅在「可信同一物理会话 resume」时安全；fresh／resume 不可用必须完整合同**（否则短引用指向会话开头合同的指针悬空）：

- `build_streaming_input` 分流判据与 revision 出口统一，**以 `resume_provider_session_id` 为准，不依赖 adapter 自称**：`DeltaOnly && resume_id.is_some()` → 短引用；`DeltaOnly` 无 resume（fresh／kimi MCP／pi 等策略漂移成 fresh）→ 完整装配；`FullConversation` 初次 → 完整装配。这同时覆盖 adapter 层把 resume 漂移成 fresh 的场景——出口层看到的 resume id 为空即自足降级为完整合同。
- 窗口结论（max 实测）：kimi/pi 默认 1M 窗口下 6-8 轮分层累计 3.4-4.1K tok（真实 BPE 237-354 tok/轮）<0.5%，膨胀担忧在分层+大窗口下不成立；262K 较小配置仍由预算常量看守（§1c 保留）。
- system 持久位方案不采纳（pi 有能力基础但 aria 未接线、kimi 无会话 system 槽），登记为后续演进（若 kimi ACP 未来提供会话 system 槽，短引用可再瘦）。
- 测试：`exit_contract_covers_full_and_delta_for_all_markdown_workspace_types` 扩为三分支断言（Full=完整 / DeltaOnly 无 resume=完整 / DeltaOnly+resume=短引用 × 四类型）；choice 两轮与 stale-marker delta 腿夹具对齐真实生产形态（choice 续跑必有已记录 author 会话）。

配套拆除业务分支点注入（出口统一，不再「记得追加」）：

- `lifecycle.rs::take_pending_author_choice_prompt`：移除 Story|Design 的 F60Fix 防线 2 点注入（**测试语义保留**，见 §三）；choice 点只产问答内容——顺带消除了契约文本进入 session 历史/压缩历史造成的 marker 污染源。
- `prompts/author_revision.rs`：移除 Design-only 契约调用（**补齐 F-60 确认缺口 2**：Story/WorkItem/WorkItemPlan 的 AuthorConfirm 反馈修订此前完全无契约，现在四类型全走出口）。
- `prompts/revision.rs` 两个 builder：移除契约/骨架点注入。
- `build_prompt`：移除顶部条件 schema 注入（旧逻辑遇 system marker 整段省略）与骨架，出口统一负责。

### 3. 计划流合同族显式化（方案：勿给 JSON 子链误拼）

`PlanAuthorOutputContract`（types.rs，pub）由调用方显式声明，禁止从 prompt 文本猜用途。核实当前全部 13 个生产调用点：JSON Outline/Split/Draft（followups/provider_run/draft_batch）与 SC compiler-source markdown（single_candidate 三处 + SC human-gate revision）均 `Structured`。**SC markdown 流走 work-item-plan compiler 而非 artifact gate**（`prepare_author_delivery_for_compile` → `converge_work_item_plan_source`），误拼 7-heading gate 合同会教坏格式——故保持 Structured。`MarkdownArtifact` 为设计要求的显式入口，行为由测试锁定（`#[allow(dead_code)]` + 注释说明）。

### 4. 不变项（方案明示）

- `web/workspace_context/prompts.rs::output_schema_for` 首次 brief 不动（全量背景）。
- `build_artifact_retry_prompt` 失败后路径不动（不新增或扩大）。
- gate 判定（`validate_workspace_artifact_constraints`）与自动调用次数不变。
- reviewer prompt（`reviewer_artifact_schema_gate_for`）不动。

## 二、验收证据（变体表全入口）

新测试 `src/product/workspace_engine/tests/f60_exit_contract_matrix.rs`（7 用例，全部经构造的 `StreamingProviderInput` 断言，非 helper 字符串）：

| 用例 | 覆盖 |
|---|---|
| `exit_contract_covers_full_and_delta_for_all_markdown_workspace_types` | 初次 Full＝完整装配 / 后续 Delta＝短引用 × Story/Design/WorkItem/WorkItemPlan（四 Markdown 类型），用户内容原样保留 |
| `exit_contract_survives_stale_marker_in_session_history_and_delta_content` | 历史 system marker 与 delta 原文 marker 均不抑制末端合同（两形态）；完整形态末端为当前类型逐字节同源渲染 |
| `choice_followup_two_rounds_each_carry_terminal_contract` | choice 两轮（Story+Design），choice 点纯问答，每轮 delta 出口以短引用收尾；delta 内容逐字节开头 |
| `author_feedback_revision_tiers_by_resume_and_covers_all_markdown_types` | 用户反馈：fresh＝完整装配（mentions 旧稿防误教）/ resume＝短引用 × 四类型（确认缺口 2 补齐） |
| `reviewer_revision_delta_full_and_codex_fresh_carry_terminal_contract` | reviewer resume delta＝短引用 / fresh full 与 Codex resume-stall fresh＝完整装配 × 四类型 |
| `plan_streaming_input_contract_family_is_explicit_and_json_chain_stays_clean` | Structured 无 marker 无契约前缀（JSON 子链不误拼）；MarkdownArtifact 末端完整装配 |

另含 `markdown_author_family_prompt_quality_budget`（两档预算断言）。

末端断言核心：完整形态 `prompt.rfind(marker)` 之后逐字节等于 `author_artifact_schema_contract_for(当前类型)`，且决策归档/负面清单/骨架同块随行、其后无第二个 schema 块；短引用形态点名同一规则源的全部必需 heading／ID／token 且不重复全文。

## 三、旧测试改造（方案明示防误拦）

- `part_31.rs::full_revision_prompt_does_not_repeat_schema_from_generation_context` → 重命名 `full_revision_prompt_terminal_contract_stays_current_despite_stale_generation_context_marker`：改「marker 只出现一次」为「末端最新合同正确」（≥2 且末端为当前渲染）；路由引用 `[cadence_project_rules]` 去重语义保持。
- `part_31.rs::retry_and_revision_prompts_render_parser_derived_schema`：delta/full 腿改经 `build_revision_input_with_resume`（F-46 回归）；retry 腿不变；骨架断言按分层限定 retry/full（delta 为短引用无骨架）。
- `part_01.rs` 三个 choice 测试：story（更名 `..._short_reference`）与 design choice 测试按分层裁决改断言短引用必备项（fence/H1/heading/REQ-AC/source id/author-decision 指针），仍经 `build_streaming_input(DeltaOnly)` 出口（F60Fix 语义在分层形态下保留）；`author_choice_followup_resumes_author_provider_session` 直通断言改「内容逐字节开头 + 末端契约在场」。
- `part_10_negative_list.rs`：共享负面清单「author contract」腿改经出口（同一常量、恰一份）。
- `part_04.rs` 两处 delta 断言：旧产物 H1 不回放的意图改为 `!contains("# Story Spec\n")`；`revision_prompt_requires_structured_interaction_decisions_in_artifact` 的决策断言改「决策归档」短引指针（author-decision-* 仍在场）。
- `author_revision_loop.rs::design_author_revision_prompt_includes_output_contract_skeleton_and_context_note`：改经出口断言。

## 四、验证记录

- TDD：矩阵测试先行（实现前 `cargo check` 以 E0433/E0308 红——新 API/参数不存在即不可通过）；绿后全量回归。
- `cargo test --locked --lib`：**3650 passed / 0 failed**（含 workspace_engine 772；分层前基线 3649 全绿亦在案）。
- `cargo test --locked --lib f60_exit_contract_matrix`：7/7 绿。
- `cargo fmt` / `cargo clippy --all-targets --all-features --locked -- -D warnings`：绿（0 警告）。
- 大文件门禁（≤1200 行）：lifecycle.rs 1180、part_31.rs 1167、prompts.rs 940、矩阵测试 391，全达标。
- 全量 `cargo test --locked`（含 it_core/it_web 集成）：见本报告末尾补充（F60Guard 统一全量门禁复核）。
- 行为边界：`normalize_generation_prompt` trim、聚合 structured-output 协议、artifact retry、reviewer gate、SC compiler 链路均不变（对应测试全绿）。

## 五、预算收口与接手

- **P1b（待 Main 重派）**：`work_item_split_engine/prompts.rs` 的 Outline 初次/修订共享合同渲染、真实来源 ID 教学、Split 初次/redo 合同族、Draft 核对——本轮完全未触碰该域（零交集）；其合同族出口已由本次 `PlanAuthorOutputContract::Structured` 显式化预留接入点。
- **P2（增益待测）**：正例（真实 issue/story/design ID 置换）与出稿前自检清单未做（方案标注需 A/B 实测）；装配函数是唯一挂载点，P2 只需扩 `markdown_author_output_contract_block`。
- 现场复测（Kimi 首轮成功率）未做——方案明示不以「测试了 prompt 包含字符串」代替行为测量。
- 过程事故存档：共享 worktree 曾被 controller 两次 `git stash`（p0-wip/p0-wip-2）波及本工作，已恢复并固化于本 commit；多 worker 并发下请用路径限定 stash。

## 六、文件清单

```
src/product/workspace_engine/prompts.rs                     出口装配 + build_prompt 清理 + 计划流合同族
src/product/workspace_engine/prompts/revision.rs            修订流出口注入
src/product/workspace_engine/prompts/author_revision.rs     缺口 2 补齐（点注入移除）
src/product/workspace_engine/lifecycle.rs                   choice 点注入升级为出口级
src/product/workspace_engine/types.rs                       PlanAuthorOutputContract
src/product/workspace_engine/mod.rs                         再导出
src/product/workspace_engine/tests/f60_exit_contract_matrix.rs  验收矩阵（新增）
src/product/workspace_engine/tests.rs                       注册矩阵模块
tests/part_01.rs part_04.rs part_10_negative_list.rs part_31.rs part_09.rs part_32/...  旧测试改造
author_revision_loop.rs baseline_teaching.rs                测试改造/导入
draft_batch/runs.rs web/workspace_ws_handler/run/{followups,provider_run,single_candidate}.rs  调用点显式合同族
```
