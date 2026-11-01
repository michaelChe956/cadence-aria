# Tasks: surface-compile-failure-on-human-gate

合同：`proposal.md` / `design.md` / `specs/work-item-plan-single-candidate/spec.md`（REQ-WSC-18/19/20）。
前端门卡呈现由并行改动实现（不改 `web/src/**`）；本清单只覆盖后端与契约。

## 1. 完整诊断 durable 落盘（REQ-WSC-18）

- [x] 1.1 `models/human_gate.rs`：`CandidateCompileFailure`（`failure_id`/`code`/`message`/`detail`/
      `recorded_at`）与 `CandidateCompileStatus`（候选绑定 + 可空 failure），附
      `belongs_to_candidate` / `current_failure` 纯判定（任一侧 ref 缺席即不匹配）。
      **验收：** 双侧逐字相等才算当前候选；缺 refs 不匹配。
- [x] 1.2 `lifecycle_store/human_dispositions.rs`：`save_candidate_compile_status`（单文件覆盖写，
      走同族 live-write 准入）、`load_candidate_compile_status`、
      `current_candidate_compile_failure(&record)`（单源判据）。
      **验收：** 缺席读 None 不报错；完整 `detail` 逐字落盘。
- [x] 1.3 `workspace_engine/compile_failure_surface.rs`：`candidate_compile_status`（失败事实）、
      `cleared_candidate_compile_status`（成功覆盖清除）、`detail_text`（失败身份 + 运行身份 +
      typed 渲染）、`owner_message`（人话）。
- [x] 1.4 `compile/single_candidate.rs`：SC initial compile 唯一返回边界收口——Err 侧 seal 后落完整
      诊断，Ok 侧落「无失败」。**验收：** plain `?` 站点（`final_compile_prepare_failed`）也落盘。

## 2. 依赖环诊断人话化（REQ-WSC-18）

- [x] 2.1 `draft_batch/compile_support.rs`：`dependency_cycle_error` + `cycle_reason`——复用既有
      `find_cycle_witness`（确定性有向 witness），点名逆行仓序的那条依赖与两仓 `change_order`
      位次；机器 id 段以 `CYCLE_IDS_MARKER` 分隔留在原文。
      **验收：** 错误串仍以 `dependency_cycle:` 开头且含环上 id（旧断言不变）；人话段用 Work Item
      标题与真实仓序位次。
- [x] 2.2 找不到 witness 时退化为通用可行动文案，不虚构环形状。

## 3. 门卡投影（REQ-WSC-18）

- [x] 3.1 `web/workspace_ws_types/out.rs`：`CandidateCompileFailureDto`
      （`code`/`message`/`detail?`/`owner_hint?`/`rewrite_guidance?`）+ `From<&CandidateCompileFailure>`；
      `WsOutMessage::SessionState` 增 `candidate_compile_failure`（可选、缺席省略）。
      **验收：** 旧 wire 字节不变（无失败时会话帧该字段不出现）。
- [x] 3.2 `session_state.rs`：`candidate_compile_failure: self.candidate_compile_failure_projection()`。
- [x] 3.3 `conversational_gate_parts/dispositions.inc.rs`：投影/判据/教学三个只读助手
      （`candidate_compile_failure_projection` / `current_candidate_compile_failure` /
      `ensure_current_candidate_compile_not_failed` / `candidate_compile_failure_author_teaching`）。

## 4. 确认动作 fail-closed（REQ-WSC-19）

- [x] 4.1 `conversational_gate.rs`：`close_human_gate` Approve 臂在静态资格预检与任何带写 CAS
      **之前** `ensure_current_candidate_compile_not_failed()?`。
      **验收：** 稳定码 `CANDIDATE_COMPILE_FAILED`；会话 status/phase/门快照/预算零变化。
- [x] 4.2 `controls.rs`：`handle_confirm` SC Approval 臂同一判据、同一文案（零 compile 副作用）。
- [x] 4.3 无法证明同属一个候选时 fail-open（不拦确认）；终止（Abandon）不受影响。
      **验收：** 旧候选失败记录在场也不拦。

## 4.5 自动重写的执行（REQ-WSC-21）

- [x] 4.5.1 `compile_failure_surface::auto_rewrite_verdict`：把候选编译失败翻译成既有 F5-A 通道
      吃的机械返修 verdict（`Revise` + `RequiresRevision` + 一条 `MustFix`/`ContractGap`/
      `Repairable` finding；`message` 只含结构身份，`required_action` 带重写要点且不要求改 design）。
- [x] 4.5.2 `compile.rs::route_candidate_compile_failure_to_author_rewrite`：Final Compile 失败分支
      在开人工确认门之前调用；`Running` 才执行，动作是
      `route_single_candidate_contract_prerevision`（`complete_review` → 策略路由
      `TriggerAggregateRepair` → `request_provider_run(WorkItemPlanSingleCandidateAuthor)`）；
      `Box::pin` 打断 `E0733` 类型级 async 递归。
      **验收：** 候选无效 + 预算未用尽 + 非同一环 → 开评审轮并重驱 author；否则回落既有门。
- [x] 4.5.3 预算：`max_auto_rewrites()` = `max_manual_repairs`（author 重写次数预算，平台自动重写与
      owner 反馈重写**同一池**）；`max_repairs`（机械返修池）**不**参与自动重写。
- [x] 4.5.4 结构化指纹：`cycle_structure_fingerprint`（成员集合 + 边集合规范化摘要，
      `{序号}@{仓}` 键、排除编译运行 id）；同一环重复 → `phase=stopped` 提前停，与预算无关。

## 5. owner / author 双份指引（REQ-WSC-20）

- [x] 5.1 `compile_failure_surface`：`owner_hint`（候选无效 + 让 author 重新生成 + design 可能需
      一起重写）、`rewrite_guidance`（四条要点）、`author_teaching_block`
      （`[candidate_compile_failure]` 段）、`approve_rejected_message`（原因 + 会再失败 + 下一步）。
- [x] 5.2 `prompts/human_gate_revision.rs`：`ScManualRevisionPromptInput.compile_failure_teaching`
      可选段；`build_sc_manual_revision_prompt_for_turn` 注入当前候选的教学块。
      **验收：** 有失败时段内逐条要点在场；无失败时提示词逐字不变。

## 6. 测试

- [x] 6.1 `compile_failure_surface` 单测：人话/原文分离、非环类型保留完整 cause、候选绑定、
      结构失败人话、确认拒收文案三要素。
- [x] 6.2 `lifecycle_store/tests/human_gate_recovery.rs`：完整诊断 durable（80 行长文本逐字）、
      候选绑定命中/不命中/缺 refs、成功后覆盖清除。
- [x] 6.3 `workspace_engine/tests/candidate_compile_failure_surface.rs`：投影三要素 + owner_hint +
      rewrite_guidance、旧候选不投影、确认 fail-closed 零副作用、修订提示词含/不含教学段、
      compile 失败边界落完整 durable 诊断。
- [x] 6.4 回归：既有 `compile_rejects_change_order_cycle` 等环断言仍通过。

## 7. 门禁

- [x] 7.1 `cargo fmt` / `cargo clippy --all-targets` / 全量 `lib` / `it_core` / `it_web`。
- [x] 7.2 行数红线（< 1200，含 `.inc.rs`）。
- [x] 7.3 `openspec validate surface-compile-failure-on-human-gate --strict` exit 0。

## 8. 未纳入本 change 的缺口（另开票据，见 design.md §6 / §8 Risks）

- [ ] 8.1 候选门 SG1 的 change_order 自动边只看 `intent_contract.target == LogicalCodebase`，
      `single_repository` 候选会漏掉跨仓环，直到 Final Compile 才炸。收窄该口径会改变候选门判定与
      冻结 digest，且 SG1 的 DependencyCycle 修复通道尚未接线 → 单独立项，**本 change 未修**。
- [ ] 8.2 `auto_rewrite.remaining`（durable `auto_rewrites_used`）与门快照
      `manual_repairs_remaining` 是同一语义池的两处账本，尚未合并为单一计数器（owner 反馈 turn 仍按
      既有 gate 预算扣减）；合并属独立改动，**本 change 未做**。
- [ ] 8.3 Fake reviewer / 未启用 reviewer 的会话里 `start_review()` 直达 `enter_human_confirm`，
      自动重写退化为开门（既有语义，未改）。
