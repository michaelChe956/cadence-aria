# WP5 归属判定表（T5 Step 1 / REQ-RET-02 · design D2 共享变体判定留档）

> 基线：worktree `feat-b-0808-add-monorepo` HEAD=`d42edc03`（T4 收口后）实读。
> 判定规则（design D2）：**SC 路径是否消费**——SC 消费则保留（显式记录），不消费则删；必删集（REQ-WSC-07 点名三族+DEF-6 点名）直接进删除清单不加判定（契约钉死）。
> 复核方法：`grep -rn "<Symbol>" src --include="*.rs" | grep -v tests` 逐符号消费链实读（inbound 路由→handler→engine→run/spawn），与计划初判表不一致处以复核为准并显式记录理由。

## 1. 必删集（契约钉死，不判定）

| 变体+DTO | 消费链复核（佐证） | 处置 |
|---|---|---|
| `SelectWorkItemGenerationMode`+`WorkItemGenerationModeDto` | inbound.rs:379 唯一路由→`engine.select_work_item_generation_mode`；SC 恒拒收（`SINGLE_CANDIDATE_GENERATION_DECISION_FORBIDDEN`，protocol.rs:9-29+socket.rs 精确拒绝旁路）；前端 cockpit 发送器随本批删 | 删 |
| `WorkItemDraftDecision`+`WorkItemDraftDecisionDto` | inbound.rs:441→`handle_work_item_draft_decision`（draft_batch/decisions.rs:111，唯一 wire 调用方）；SC 恒拒收（同上） | 删 |
| `WorkItemBatchDecision`+`WorkItemBatchDecisionDto` | inbound.rs:488→`handle_work_item_batch_decision`（draft_batch/decisions.rs:4，唯一 wire 调用方）；SC 恒拒收（同上） | 删 |
| `ReviewDecisionResponse`（双选项语法） | inbound.rs:363→`handle_review_decision_from_handler`（decisions.rs:87）→engine `handle_review_decision`（decisions.rs:374，唯一 wire 调用方）；campaign driver SC 策略明确拒绝 SC 出站该消息（workitem_run_campaign.mjs:1295 白名单无 review_decision_response） | 删 |
| `HumanConfirm`+`HumanConfirmDecision` | inbound.rs:864（T2 SC 桥接=映射式，L2 删）+:196/:855（legacy 分流）；engine `handle_human_confirm`（decisions.rs:568，wire 唯一入口）；T4 后前端零发送（stage4-c3-t4-report §grep） | 删（`Confirm` 非 SC 分支改直连 `handle_confirm`，保 story/design approve 帧面=T4 §4 登记限制的既有形态） |
| `SelectRevisionPath`+`RevisionPath` | inbound.rs:747→`map_revision_path`→`handle_review_decision_from_handler`（同 ReviewDecisionResponse 链）；前端发送器随本批删 | 删（含 mapping.rs `map_revision_path`） |

## 2. 共享变体归属判定（逐符号复核）

| 变体 | SC 消费证据（实读） | 判定 | 处置 |
|---|---|---|---|
| `RequestRevision` | **有**：WorkItemPlan 分支（inbound.rs:760-852）→`request_work_item_plan_revision`（plan_outline/revision.rs:559，AuthorConfirm 阶段=SC plan-repair 消费通道，CodingWorkspacePage regenerate 面=唯一活发送器）；SC 出站白名单外但经 socket.rs:718-720 AuthorConfirm+WorkItemPlan 特例放行 | 保留 | 删非 plan 分支 RequestChange 桥接（:853-862，engine `handle_human_confirm` 随删）——非 plan 会话发此消息→protocol error（在途限制登记） |
| `WorkItemPlanCompileRecoveryAction`+`WorkItemPlanCompileRecoveryActionDto` | **有**：compile.rs:492 进入（`mark_latest_compile_transaction_recovery_required`=SC compile 链可达恢复态）；inbound.rs:547 路由；前端 `sendWorkItemPlanCompileRecoveryAction` 在用（cockpit/legacy 页 compile recovery 按钮） | 保留（显式记录） | 保留 |
| `ConfirmPlanAmendment`/`CancelPlanAmendment`/`StartLinkedWorkspaceAmendment` | **有**：amendment 链=SC 基座（REQ-CG 语义不动） | 保留 | 保留 |
| `AuthorDecision`+枚举 | **无**：inbound.rs:375 唯一路由→`handle_author_decision_from_handler`→engine `handle_author_decision`（含 WorkItemPlan outline confirm 分支 `handle_work_item_plan_outline_decision`，均 wire 唯一入口）；SC 门走 typed 三命令（Confirm/HumanGateFeedback/AbandonHumanGate）。**复核发现（初判理由修正，结论不变）**：该消息族同时是 story/design 会话（legacy 流）与 legacy-flow workitem 的 author 确认通道，cockpit（story/design 默认页，chat-cockpit-mode.ts:12-16）仍装配发送器（ChatCockpitPage:463-472/:1074）——属 T4 §4 已登记的「T5 后 legacy 决策通道整体终止」范围（REQ-RET-03 在途限制）；story/design 产物确认另有 HTTP `POST /api/workspace-sessions/{id}/confirm`（e2e helpers 即用此通道） | 删（复核确认） | 删 wire+枚举+engine `handle_author_decision`/`handle_work_item_plan_outline_decision`+cockpit/legacy 页发送器与按钮面；`AuthorDecisionOutcome` 枚举随之退役 |
| `RequestOutlineRevision` | **无**：inbound.rs:411→`request_work_item_plan_outline_revision`（plan_outline/authoring.rs:50，wire 唯一调用方）；前端死发送面（T4 清单 #5 待 T5；cockpit 发送器随本批删）；SC outline 修订走 human_gate_feedback | 删（复核确认） | 删 wire+engine fn+前端发送器 |
| `SaveHumanPresentationRevision`+`HumanPresentationScopeDto` | **无**：inbound.rs:567→`save_human_presentation_revision_command`（human_presentation.rs:72，wire 唯一调用方）；前端发送器 T4 已删（inventory #9），残余=死类型+store begin/fail 动作 | 删（复核确认） | 删 wire+DTO+engine fn+前端类型与 store 动作；`WsOutMessage::HumanPresentationRevisionSaved/SaveFailed` 出站变体一并退役（唯一生产点随删） |
| `RevertWorkItem` | **无**：inbound.rs:952→`apply_revert_mark`（wire 唯一调用方，grep 实测无其他消费）；前端 `sendRevertWorkItem` T4 已删（死类型）；SC 路径无 revert 消费（Step 1 实测裁定） | 删（实测裁定） | 删 wire+engine `apply_revert_mark`+前端 `RevertWorkItemMessage` 类型 |
| `Confirm`/`HumanGateFeedback`/`Advance`/`AbandonHumanGate`/`Abort` | **有**：typed 门命令族（T2 重承载落地）+全局取消（Abort 语义不动） | 保留 | 保留（`Confirm` 非 SC 分流改直连 `handle_confirm`——story/design approve 帧面保持，见必删集 HumanConfirm 行） |
| 通用（UserMessage/ContextNote/Hello/Ping/Rollback/StartGeneration/RetryInterruptedRun/ProviderSelect/PermissionResponse/ChoiceResponse） | 与 legacy 决策协议无关（连接/通用通道） | 保留 | 保留 |

## 3. ContextBlocker 决策族（oracle P2 双审修订项：二选一判定）

**消费链实读（三入口）**：`enter_work_item_plan_context_blocker` 仅 3 个调用点（plan_outline/authoring.rs:213/:248/:318），全部位于 `complete_work_item_plan_outline_author` 与 `complete_work_item_plan_outline_author_output_error`——两者仅被 `WorkItemPlanLegacyAuthor` run 臂（provider_run.rs:183-599）与 followups 大纲重跑（run/followups.rs:791，`drive_current_work_item_plan_outline_run`=legacy 大纲续跑）调用；SC run 走 `WorkItemPlanSingleCandidateAuthor`→`single_candidate::run_single_candidate_author`（provider_run.rs:600），`single_candidate.rs` 全文零 context blocker 引用。→ **SC 不触达，三入口全 legacy 流**（oracle 复核结论成立）。

**处理器归属**：`handle_work_item_plan_context_blocker_decision`（engine decisions.rs:699）唯一入口=engine `handle_human_confirm` 特判（:577-583）——随 `HumanConfirm` 必删族死亡。**判定=（b）随 L2 删**：

- (a) typed 重承载 provide_context 通道——**否决**：SC 不触达该状态（消费链实读），无 SC 面需要该通道；前端 context blocker 门（GatePromptEntry）的 typed facade `feedback` 仅 `flow_kind==="single_candidate"` 放行（cockpit-action-routing.ts:39），legacy 会话客户端即拒发——即 T4 后该处理器已无任何可达入站消费。
- (b) 随 L2 删——**采纳**：REQ-RET-03「在途 legacy 会话不承诺决策通道连续性」已登记（T4 §4 同款限制族）；历史 context blocker 节点呈现（timeline 只读）不受影响。

**连带退役**：`append_work_item_plan_context_blocker_resolution`（唯一调用方=该处理器）、`human_confirm_payload_description`/`human_confirm_payload_source`（payload 解析，唯一消费=该处理器+`handle_human_confirm`）。`enter_work_item_plan_context_blocker`（状态进入）与 `enter_human_confirm_for_work_item_plan_author_failure` 保留——仍被 legacy 大纲 run 完成路径调用（在途会话读侧状态机，决策面死=登记限制）。

## 4. 引擎层连带处置（复核发现，SC 可达面保护）

| 面 | 复核证据 | 处置 |
|---|---|---|
| review/routing.rs `_` 臂 `RequiresRevision=>enter_review_decision`（:683-686） | **SC 可达**：`evaluate_work_item_policy_route` 在「Revise 且全部 finding 无分类且非 Repairable」时返回 None（:323-330）→`route_legacy_review`（policy 旁路臂，run1d 形态）→SC 会话首轮 RequiresRevision 落 review_decision 阶段——该阶段唯一合法消息=必删集两变体，删后成死局 | SC 加 flow 守卫改道 `enter_human_confirm`（与 F5-B repeated_findings 同款出口=SC 门 typed 面可应答）；非 SC 保留原路由（在途 legacy 会话=登记限制） |
| plan_repair_review.rs `route_plan_repair_candidate_review`（:221-247） | **SC 可达**：plan repair=会话内模式（plan_repair.rs:254 快照态），SC 会话可处 PlanReview 阶段；缺结构化 review/RequiresRevision→`enter_review_decision`（:223/:244）同死局风险 | 同上：SC 守卫改道 `enter_human_confirm`；非 SC 保留 |
| `skip_work_item_plan_optional_findings`（decisions.rs:518） | 唯一调用方=`handle_review_decision` "skip_optional_findings" 臂（:387） | 随删 |
| `request_work_item_plan_outline_revision`（plan_outline/authoring.rs:50） | 唯一调用方=inbound.rs:415（RequestOutlineRevision 臂） | 随删 |
| draft_batch `handle_work_item_draft_decision`/`handle_work_item_batch_decision` | wire 唯一入口（inbound :441/:488）；其余 batch/draft 内部函数（accept/rewrite/downgrade）仍被 review routing legacy 臂调用 | 仅删两 wire 决策处理器；内部状态机函数保留（在途读侧） |
| `ReviewDecisionOutcome` 枚举 | 保留者仍在用（`request_work_item_plan_revision` 返回型；inbound RequestRevision 臂 match） | 保留（未被引用的变体随编译面裁剪） |
| `enter_work_item_generation_mode`/`enter_work_item_draft_confirm`/`enter_work_item_batch_confirm`/`enter_work_item_plan_outline_confirm`/`enter_author_confirm` | 仍被 legacy run 完成路径/review routing 调用（在途会话状态机） | 保留（决策应答面死=登记限制，非本批强制全删） |

## 5. flow_kind 收敛（Step 3 落点）

| 面 | 处置 |
|---|---|
| `lifecycle_store/inputs.rs:229` `WorkItemPlanSessionOptions::default().flow_kind` | 翻 `SingleCandidate`（编译面最小动作；判定=新会话一律 SingleCandidate） |
| `lifecycle.rs` prepare preflight | 删 `LegacyFallback` 两分支（Legacy :653-665/Logical :696-708 的回落臂）；preflight 判定保留，失败→新路径 durable Failed 终态+原因（复用 `mark_single_candidate_prepare_failure` 形态） |
| `models/workspace.rs:53` `default_flow_kind()`（serde 缺字段默认） | **保留 Legacy**——读侧兼容：缺字段的历史记录即 legacy 时代产物（REQ-RET-03 只读历史值），不得误读为 SC |
| `conversational_gate_recovery.rs:104` Legacy→Err | 保留（读侧守卫，语义不变） |
| 其余 `flow_kind` 读点（compile/policy_route/single_candidate 等） | 不动（SC 判定/历史容忍语义既有） |
