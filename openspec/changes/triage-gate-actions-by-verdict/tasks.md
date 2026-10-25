# 高层工作包：分诊门动作集合按最新审查结论派生

本文件只记录未来 apply 的高层工作包及其契约映射；精确文件、命令、测试与提交切分由后续 `cadence/plans/` 的已确认 Plan 展开。本次仅创建契约工件，不执行、不勾选实现任务。

## 1. 服务端派生（权威动作集合）

- [ ] 1.1 在 Code Review 分诊门建门路径（`src/product/coding_workspace_engine/provider_failure.rs:145-156` 的三个 reason code 分支）按该 attempt 最新一条 Code Review 结论的 verdict 派生 `available_actions`：基础 `retry_review`+`manual_continue`+`abort`，当且仅当 verdict ∈ {`request_changes`, `blocked`} 追加 `send_to_coder`；读取口径与 `send_code_review_feedback_to_coder`（`rework.rs:529-541`）一致。验收：三个 reason code 各自在 `request_changes`／`blocked` 下为四项、在 `approve` 下为三项，且动作顺序稳定（REQ 分诊门禁动作集合必须按最新审查结论派生）。
- [ ] 1.2 实现读不到 verdict 的 fail-closed：结论缺失、不可解析、读取失败三条路径都按三项派生，门仍正常落地并可操作；不得默认包含 `send_to_coder`，也不得因缺事实拒绝开门。验收：三条负例各落一门的动作集合恰好三项、attempt 状态为 `blocked`、门可被 `retry_review` 收口（同条 requirement）。
- [ ] 1.3 保证派生只在建门发生：同 attempt／node／stage／reason_code 命中 store 去重路径时按新 verdict 覆盖 `available_actions`（`coding_attempt_store/gate.rs:1334-1348` 既有行为），读路径（`normalize_blocked_gate`，`gate.rs:1432-1458`）MUST NOT 被扩展到三个分诊门。验收：读取门不产生写盘、`code_review_blocked` 的既有补动作行为不变（同条 requirement）。
- [ ] 1.4 `retry_review` 之后重新派生：门收口 → 新 Code Review 落地 → 新门按新结论派生；不引入任何持久「禁用位」。验收：`approve` 三项门 → `retry_review` → `request_changes` → 新门四项（同条 requirement）。

## 2. 事实源与呈现面

- [ ] 2.1 客户端权威渲染：门动作按钮只由 `available_actions` 生成，MUST NOT 自行增删；补前端契约断言「三项门不渲染『提交给 Coder 修复』」。验收：三项门 DOM 中无该动作、四项门有（REQ 派生事实可观察）。
- [ ] 2.2 就绪指引改依据最新 verdict：同屏取会话状态 `code_review_reports` 最后一条的 verdict 生成「为什么没有送回 Coder」文案与替代动作指引（`retry_review`／`manual_continue` 的真实 label），MUST NOT 依赖提交失败后返回的错误码。验收：三项门首次渲染即出现指引、无任何失败往返（同条 requirement）。
- [ ] 2.3 存量在途门专用兜底：对升级前已 open 且动作集合仍含 `send_to_coder` 的分诊门实例，按最新 verdict 事实禁用该动作并给指引；服务端不回写存量门。验收：构造存量四动作门 + `approve` 结论 → 该实例按钮禁用且仍有指引；`request_changes` 结论 → 按钮可用（同条 requirement）。
- [ ] 2.4 派生成因字段可选：默认不加 gate 字段；若实现认为需要门自描述，则按 `CodingGateRequired` additive（`#[serde(default)]`）+ 独立 store 回写（仿 `attach_blocked_gate_triage_summary`）落地，并保证 `available_actions` 仍为权威、字段缺失时客户端不自行增删动作（REQ 派生事实可观察，方案 A/B 取舍见 `design.md` §2）。

## 3. 服务端前置与既有拒绝语义

- [ ] 3.1 保留 `send_code_review_feedback_to_coder` 的 verdict 前置（verdict ∉ {`blocked`, `request_changes`} → `send_to_coder_latest_review_not_actionable`，`rework.rs:542-549`）与「缺报告拒绝」路径（`rework.rs:536-541`）；存量在途门与该前置共存：不回写也不放宽。验收：对三项门与存量四动作门分别直发 `send_to_coder`，均返回该错误码且不落地 rework instruction、attempt 状态不变（REQ 分诊门禁的送回 Coder 动作必须可用）。
- [ ] 3.2 确认 `request_changes` 下 `send_to_coder` 仍走代码审查反馈返修路径并成功（落地 instruction、stage → `Coding`、返修计数 +1），且不走轮次超限路径；`should_resume_runner_after_gate_response` 白名单与 `manual_continue`／`abort` 语义不变（`src/web/coding_ws_handler/runner.rs:316-339`）。验收：既有 `verdict 为 request_changes 时送回 Coder` 与 `manual_continue 后自动续跑` 用例保持绿（同条 requirement + REQ 分诊门人工继续后回到原编排链）。

## 4. 测试落点

- [ ] 4.1 `src/product/coding_workspace_engine/tests/code_review_triage.rs`：三处既有动作断言（`:157`、`:243`、`:377`，夹具 verdict 均为 `request_changes`）保留四项结论、把文案改为「可返修结论下为四项」；新增 `approve` → 三项、读不到结论 → 三项、`retry_review` 后重新派生三个用例（REQ 分诊门禁动作集合必须按最新审查结论派生）。
- [ ] 4.2 `web/src/pages/CodingWorkspaceControls.gate-actions.test.tsx`：三项门不渲染 `send_to_coder`、四项门渲染；指引文案取自最新 verdict；删除对「先提交失败再置灰」路径的依赖断言（REQ 派生事实可观察）。
- [ ] 4.3 验证处理入口回归：`src/product/coding_workspace_engine/tests/verification_triage_gates.rs` 与 `src/web/handlers/coding/verification_surface.rs` 相关用例断言「转入前后动作集合逐项相同（含是否含 `send_to_coder`）、门仍 open、finding 不变」（REQ-CVT-03）。
- [ ] 4.4 互斥与不受影响面回归：`blocked` + 无可执行 finding 只落 `code_review_blocked`（`code_review_triage.rs:388-410` 既有用例）；`internal_review_*` 三门仍为 `["retry_internal_review", "manual_continue", "abort"]`（`tests/provider_failure_recovery/retry_and_gate.rs:1071-1078`）；`code_review_report_not_accepted` 与 `reviewer_rework_limit_reached` 动作集合不变（REQ 单次审查结论只允许落地一个 blocked gate + 本 change 非目标）。

## 5. 契约一致性与归档硬门

- [ ] 5.1 【归档硬门】归档前复核在途 change 的 header 重叠：`batch1-coding-autopilot-mainline` 在 `coding-verification-triage` 只 ADDED `REQ-CVT-06`；若它先归档，重取主规范确认本 delta 的 `REQ-CVT-03` MODIFIED 基线与三个 scenario 名（`从 coder 输出门转入`、`从 Code Review 门转入`、`重复转入被拒`）仍完整；其 delta 说明文字「REQ-CVT-03 保持不变」需在归档时同步失效说明（REQ-CVT-03）。
- [ ] 5.2 【归档硬门】按 RENAMED → MODIFIED 顺序核对主规范：`coding-code-review-triage` 归档后不得残留「恰好四人」「四动作」措辞，且不得出现 delta 段头；两条改名后的 requirement 名称与正文一致（REQ 分诊门禁动作集合必须按最新审查结论派生 + REQ 验证处理入口独立于分诊门动作集合）。
- [ ] 5.3 全量一致性复核：`openspec validate triage-gate-actions-by-verdict --strict` 绿；实现完成后逐条核对本 change 两个 delta 的每个 scenario 都有对应用例或明确的不适用理由；未实现工作包不勾选。

## 后续工作流

- 契约审阅通过后才展开 `cadence/plans/` 的实施 Plan 与 apply；本次工件创建不代表任何源码行为已实现。
- 非目标（不在本 change 实现）：`code_review_blocked` 门在不可执行 finding 情形下的 `send_to_coder` 死动作、`code_review_report_not_accepted` 门动作集合、`send_code_review_feedback_to_coder` 前置放宽、轮次超限路径语义调整。
