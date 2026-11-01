# Tasks: bound-review-findings-to-work-item-scope

合同：本 change 的 `proposal.md` / `design.md` / `specs/work-item-plan-single-candidate/spec.md`
（REQ-WSC-12/13/14/15）。REQ-WSC-15 的门卡 UI 呈现由并行改动实现，其契约条款与语义核对在本清单第 6 节。

## 1. finding 契约扩展（REQ-WSC-12）

- [x] 1.1 `web/workspace_ws_types/review.rs`：`ReviewFindingDisposition`（`actionable|advisory`）；
      `ReviewFinding` 增 `disposition` / `target_work_item_id` / `finding_key`；`ReviewFinding` 改
      「derive Deserialize + 手写 Serialize」，序列化时按 `sha256(severity␁message)[..16]` 现算 key。
      **验收：** 可选字段仍只在 Some 时输出（旧 wire 字节不变）；key 恒输出且不可由模型伪造。
- [x] 1.2 `parsers.rs` strict 白名单增 `disposition` / `target_work_item_id`，类型与非空校验；
      `parsers/lenient_findings.rs` 增 `parse_optional_finding_disposition`（非法值 →
      `invalid_finding_field`）与 `parse_optional_target_work_item_id`（空串/非字符串 → None）。
      **验收：** 缺失 → None（语义 actionable）；非法 → 不合规；历史 payload 照旧解析。
- [x] 1.3 7 处 reviewer schema 增 `disposition` + `target_work_item_id` 字段形状
      （`prompts/review.rs` ×5、`review_parts/draft_input.inc.rs`、`review_parts/single_candidate_input.inc.rs`）。
- [x] 1.4 共享输出契约（`prompts.rs::reviewer_output_contract`）注入 `FINDING_SCOPE_RULES`
      （`owner_guidance_rules.rs`）：范围约束 + 收敛约束，7 个 builder 同一份措辞。
      **验收：** 源码级钉——7 处 schema 均含两个字段名，契约均含该常量。

## 2. 平台硬规则：advisory 不阻断（REQ-WSC-12）

- [x] 2.1 `parsers.rs::review_gate_for` 与 `effective_work_item_plan_review_verdict` 只计 `actionable`。
- [x] 2.2 `prompts/history_compaction.rs::render_open_required_findings` 只列 `actionable`。
- [x] 2.3 门卡计数：后端在 findings 出站携带 `disposition`；前端 `gate-prompt-copy.ts`
      `isActionableFinding` / `actionableFindings` 同口径（缺失＝actionable）。
- [x] 2.4 机械 finding 生产者显式声明处置：`plan_preflight.rs`（Error→MustFix / `class_hint=repairable`；
      Warning→**Suggestion + class_hint=Advisory**，`disposition: None`——缺失＝actionable 语义，但
      severity/class_hint 已使其不阻断，`plan_preflight.rs:273-322`）、
      `contract_prerevision_parts/finding_projection.inc.rs`（同口径，`:16-46`）。
      **验收：** 模型把 advisory 标成 must_fix 也不阻断（有用例，不经模型自觉）。

## 3. owner 排除项 durable + 回喂（REQ-WSC-13）

- [x] 3.1 `models/human_gate.rs`：`HumanFindingDispositionKind`、`HumanDispositionRecord`
      （含 `is_valid_finding_key`）、`HumanGateTurnRejection`。
- [x] 3.2 新 store `lifecycle_store/human_dispositions.rs`：
      `list_human_dispositions` / `record_human_dispositions`（按 key 幂等）/
      `save_human_gate_turn_rejection`（同 turn 幂等）/ `load_…` / `latest_…`；
      落点 `workspace-sessions/{session}/human-dispositions.json`、`human-gate-rejections/{turn}.json`，
      走既有 live-write 准入。
- [x] 3.3 入站：`WsInMessage::HumanGateFeedback` 增 `excluded_finding_ids`（`#[serde(default)]`）；
      REST `HumanActionRequest::Feedback` 同款；两路在提交反馈前记录排除项（空＝零副作用）。
- [x] 3.4 引擎 `record_owner_finding_exclusions`（摘要取本轮 verdict 的同一内容身份 finding）。
- [x] 3.5 `review_context.rs::human_dispositions_section` + 纯渲染
      `render_human_dispositions_section`；注入单候选复评与通用评审（有上轮上下文的路径），
      Outline/Draft/Batch/Projection 不注入。
- [x] 3.6 前端 state：`gate entry metadata.findings[]` 增 camelCase 别名
      `findingKey` / `targetWorkItemId`（`disposition` 同名）；facade
      `feedback(feedback, { excludedFindingIds })`；`useWorkspaceWs.sendHumanGateFeedback` 第三参；
      空串仅在带排除项时放行；无排除时保持旧 2 参调用与旧 REST payload 形态。
      **验收：** 排除→落盘→下一轮 prompt 携带→不再报 actionable（store + 渲染用例钉住）。

## 4. 门 turn 拒收可操作化（REQ-WSC-14）

- [x] 4.1 `conversational_gate.rs::record_terminal_rejection`：只对已终态
      `Failed(validation_reject)` 生效，落 durable 并回传；落盘失败打点但不吞回执。
- [x] 4.2 `human_gate_rejection_guidance`：typed 稳定码 → 人话原因 + 下一步动作；未知码给可行动兜底。
- [x] 4.3 出站新帧 `WsOutMessage::HumanGateTurnRejected`（4 个可操作字段）+ `human_gate_turn_rejected_frame`；
      `provider_run.rs` 在终态拒收时改发该帧，非终态仍发既有 `ProtocolError`。
- [x] 4.4 前端 state：`HumanGateTurnRejectionState` + `applyHumanGateTurnRejected`（live/observer 两 store）、
      门卡 metadata 四个 camelCase 键；`web/api/types` 增 `human_gate_turn_rejected` 事件形状。

## 5. 门禁与回归

- [x] 5.1 新增用例：disposition 缺失/非法/明确三分、advisory 不阻断（通用 + 单候选）、
      必关闭清单过滤、finding_key live/rebuild 同值、清单与拒收记录 durable/幂等、
      `[human_dispositions]` 渲染与空清单零注入、7 处 schema 源码级钉。
- [x] 5.2 全量门禁：`cargo test --locked`（lib / it_core / it_web）、`clippy`、`fmt`、
      `large_file_guard` 全绿；前端 `pnpm -C web test` + `pnpm -C web build` 绿。
- [x] 5.3 契约 delta：`openspec validate bound-review-findings-to-work-item-scope --strict` exit 0。

## 6. REQ-WSC-15：人工门可操作性（契约成文 + 语义核对）

- [x] 6.1 把 REQ-WSC-15 的四条条款写进 delta spec（被拒可操作 / advisory 与 actionable
      视觉可分且计数只计 actionable / 排除动作仅在有平台 key 时渲染 / 排除即时反馈）。
- [x] 6.2 逐条核对并行前端实现（`finding-list.tsx`、`entries/GatePromptEntry.tsx` 与其用例）
      是否兑现语义，结论与 `file:line` 记入 `design.md` §6；未兑现项如实标出。
- [x] 6.3 前端消费的 camelCase 键由本 change 的 state 层从 snake_case wire 统一转换
      （`workspace-chat-rebuild.ts`），不改并行实现的读取面。

## 7. k3 双裁定修订（P0-2A / P0-2B / P0-3 / REQ-WSC-12 / REQ-WSC-14 / REQ-WSC-16）

- [x] 7.1 **P0-2A**：`exclude_only_feedback_requested`（`feedback` 空 + 合法 key）在 WS 与 REST 两路
      分流——落 durable 即受理，不建修订 turn、不跑 `validate_feedback`；WS 回
      `human_gate_exclusions_accepted`，REST 回 200 accepted；落盘失败不冒充受理。
      **验收：** it_core 端到端「排除-only 帧 → 清单落盘 → 无 `INVALID_HUMAN_GATE_FEEDBACK` → 无修订 turn」。
- [x] 7.2 **P0-2B**：`review/disposition_override.rs` 平台裁决（`finding_key ∈ 清单 → advisory`；
      `revise` 无 actionable → pass），接线在 `complete_review` / 恢复派生 / 指纹闸门两侧；
      `classify_finding` 让 `disposition` 先于 `class_hint` 生效。
      **验收：** 真实 store 清单 → 引擎裁决用例（不依赖模型听话）；advisory 不进指纹/不耗 budget 用例。
- [x] 7.3 **P0-3**：`latest_review_verdict_from_node_details` 比较前两侧剔除 `finding_key`，durable 零改写。
      **验收：** 读历史记录后 `NodeDetail.verdict` 逐字不变。
- [x] 7.4 **REQ-WSC-12 返修触发路径**：指纹集合 / transition budget / `revise` 归一三处只看 actionable；
      契约类缺口（`contract_gap`/`self_contradiction`）标 advisory 仍按 actionable（P0-1 加固）。
- [x] 7.5 **REQ-WSC-14 拒收可读**：`session_state` 增 `human_gate_turn_rejection`（四要素，缺席省略）；
      前端 `setSessionState` 以它重建终态 turn（不覆盖更新的 live turn）。
      **验收：** `build_session_state` 投影用例 + 前端 store 用例。
- [x] 7.6 **REQ-WSC-16 撤销排除**：store `revoke_human_dispositions`（幂等）+ REST
      `revoke_finding_exclusions`（200 + `remaining_finding_keys`）。
- [x] 7.7 design.md 与实现对齐（§2.1 投影字段形态、§2.6 平台裁决、§2.7 durable 零改写、§2.8 撤销），
      契约 delta 增 REQ-WSC-16/17，`openspec validate --strict` exit 0。

## 8. 作者返修输入面只认 actionable（k3 P1 阻塞项 + P3 一致性批）

- [x] 8.1 `review/feedback.rs` 增 `format_actionable_review_feedback`（`format_review_feedback` 保留给留档/
      对话流），接线 `review/routing.rs`（`pending_revision_context`）与
      `plan_outline/revision.rs::work_item_plan_outline_revision_feedback`（含逐条简表与
      impact-closure 判据）；`ReviewFinding::is_actionable()` 为唯一过滤口径。
      **验收：** part_09 用例——owner 降级 finding 的 message/required_action 不进 outline 返修反馈。
- [x] 8.2 `work_item_split_engine/prompts/sc_revision.rs::render_review_revision_block`：`[review_findings]`
      只迭代 actionable；advisory 以 `[review_advisory_findings]`（仅条数，不含 message/required_action）
      单列「仅供提示，本轮不要修改」；「必须逐条修复」指令显式限定 `[review_findings]` 的 actionable 条目。
      **验收：** 契约用例钉住 prompt 不含降级 finding 的 message/required_action。
- [x] 8.3 用例（红→绿）：`feedback.rs` actionable 变体、SC 返修 prompt、outline 返修反馈、
      `single_candidate_previous_round_findings_section` 过滤——四处断言降级 finding 不进注入面。
- [x] 8.4 `prompts/review_context.rs::single_candidate_previous_round_findings_section`：durable 原始
      verdict 先按会话已处置清单过滤，再按 advisory 分类过滤（同一 prompt 不自相矛盾）。
- [x] 8.5 design.md §2.2 补「作者返修输入面」判定面与过滤口径；§2.6 归一范围改为 `revise` 参与、
      `revise_batch`/`plan_reopen_required` 不参与；§6 file:line 按现实现重钉（含排除逻辑迁至
      `entries/gate-finding-exclusion-state.tsx`）；tasks 2.4 措辞按 `plan_preflight.rs` /
      `finding_projection.inc.rs` 实证改写。
- [x] 8.6 契约 delta：REQ-WSC-14 增「作者返修输入面只认 actionable + 上一轮清单先按已处置清单过滤」条款
      与场景；`openspec validate … --strict` exit 0。
- [ ] 8.7 **转交前端（controller 已记录裁决）**：durable 拒收重建加「门内已有更新的在途 turn 则不重建」判据。
      实际文件是 `web/src/state/workspace-ws-store.ts:1183-1218`
      （`durableRejectionTurnFromSession`），不是 `src/web/workspace_ws_store.ts`（本仓不存在）；
      该文件属 `web/src/**`（本批次禁改），由 controller 转交前端 owner。
      判据建议：同会话 `prev.humanGateTurn` 的 `opened_at`/`turn_id` 表示更新的在途 turn 时不重建；
      或 durable `rejection.recorded_at` 早于当前门快照开门时间则不重建。
