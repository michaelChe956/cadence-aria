# 高层工作包：分诊门动作集合按最新审查结论派生

本文件记录高层工作包及其契约映射；精确文件、命令、测试与提交切分由 `cadence/plans/` 的已确认 Plan 展开。实现已按本契约落地（commit `3876c701`），客户端亦已改为只按 `available_actions` 渲染、不再自行置灰；各条仍作为归档前逐条核对清单，勾选在核对时进行。

## 1. 服务端派生（权威动作集合）

- [ ] 1.1 在 Code Review 分诊门建门路径（`src/product/coding_workspace_engine/provider_failure.rs:183-204` 的三个 reason code 分支，最新 verdict 取 `latest_code_review_verdict`，`:123`）按该 attempt 最新一条 Code Review 结论的 verdict 派生 `available_actions`：基础 `retry_review`+`manual_continue`+`abort`，当且仅当 verdict ∈ {`request_changes`, `blocked`} 追加 `send_to_coder`；读取口径与 `send_code_review_feedback_to_coder`（`rework.rs:529-541`）一致。验收：三个 reason code 各自在 `request_changes`／`blocked` 下为四项、在 `approve` 下为三项，且动作顺序稳定（REQ 分诊门禁动作集合必须按最新审查结论派生）。
- [ ] 1.2 实现读不到 verdict 的 fail-closed：结论缺失、不可解析、读取失败三条路径都按三项派生，门仍正常落地并可操作；不得默认包含 `send_to_coder`，也不得因缺事实拒绝开门。验收：三条负例各落一门的动作集合恰好三项、attempt 状态为 `blocked`、门可被 `retry_review` 收口（同条 requirement）。
- [ ] 1.3 保证派生只在建门发生：同 attempt／node／stage／reason_code 命中 store 去重路径时按新 verdict 覆盖 `available_actions`（`coding_attempt_store/gate.rs:1364-1378` 既有行为），读路径（`normalize_blocked_gate`，`gate.rs:1472-1498`）MUST NOT 被扩展到三个分诊门。验收：读取门不产生写盘、`code_review_blocked` 的既有补动作行为不变（同条 requirement）。
- [ ] 1.4 `retry_review` 之后重新派生：门收口 → 新 Code Review 落地 → 新门按新结论派生；不引入任何持久「禁用位」。验收：`approve` 三项门 → `retry_review` → `request_changes` → 新门四项（同条 requirement）。

## 2. 事实源与呈现面

- [ ] 2.1 客户端权威渲染：门动作按钮只由 `available_actions` 生成，MUST NOT 自行增删；补前端契约断言「三项门不渲染『提交给 Coder 修复』」。验收：三项门 DOM 中无该动作、四项门有（REQ 派生事实可观察）。
- [ ] 2.2 就绪指引依据派生事实：集合不含「提交给 Coder 修复」时，按门 payload 的 `latest_review_verdict`（缺失时可回落会话状态 `code_review_reports` 最后一条）生成说明文案与替代动作指引（`retry_review`／`manual_continue` 的真实 label），MUST NOT 依赖提交失败返回的错误码。验收：三项门首次渲染即出现指引、无任何失败往返；事实缺失时给通用说明而非猜测（同条 requirement）。
- [ ] 2.3 呈现层只做指引、不做派生：任何情形（含存量在途门）下 MUST NOT 禁用、隐藏或改写 `available_actions` 里的动作，也 MUST NOT 按 verdict 自行改动按钮可用性；存量门的可执行性由服务端前置决定（拒绝 + 可观察码），呈现层 MAY 用该码给替代动作指引。验收：存量四动作门 + `approve` 结论 → 动作集合仍为四项、按钮未被禁用，点击后服务端返回 `send_to_coder_latest_review_not_actionable` 且不落 instruction；代码中不存在按 `errorCode`／verdict 置灰的路径（同条 requirement）。
- [ ] 2.4 派生成因字段（已按方案 B 落地）：`CodingGateRequired.latest_review_verdict: Option<ReviewVerdict>` additive（`#[serde(default, skip_serializing_if)]`，`src/product/coding_models/gate.rs:156-164`）+ 独立 store 回写 `attach_blocked_gate_latest_review_verdict`（`src/product/coding_attempt_store/gate.rs:292-312`），仅对三个分诊门 reason code 写入、失败不影响开门（`provider_failure.rs:233-249`）；去重复用同门实例时显式写 `None` 清掉上一轮残留。验收：字段只供文案（缺失时客户端不据此增删动作）、attach 失败时门与动作集合仍正确、旧 gate JSON 反序列化为 `None`（REQ 派生事实可观察，取舍见 `design.md` §2）。

## 3. 服务端前置与既有拒绝语义

- [ ] 3.1 保留 `send_code_review_feedback_to_coder` 的 verdict 前置（verdict ∉ {`blocked`, `request_changes`} → `send_to_coder_latest_review_not_actionable`，`rework.rs:542-549`）与「缺报告拒绝」路径（`rework.rs:536-541`）；存量在途门与该前置共存：不回写也不放宽。验收：对三项门与存量四动作门分别直发 `send_to_coder`，均返回该错误码且不落地 rework instruction、attempt 状态不变（REQ 分诊门禁的送回 Coder 动作必须可用）。
- [ ] 3.2 确认 `request_changes` 下 `send_to_coder` 仍走代码审查反馈返修路径并成功（落地 instruction、stage → `Coding`、返修计数 +1），且不走轮次超限路径；`should_resume_runner_after_gate_response` 白名单与 `manual_continue`／`abort` 语义不变（`src/web/coding_ws_handler/runner.rs:316-339`）。验收：既有 `verdict 为 request_changes 时送回 Coder` 与 `manual_continue 后自动续跑` 用例保持绿（同条 requirement + REQ 分诊门人工继续后回到原编排链）。

## 4. 测试落点

- [ ] 4.1 `src/product/coding_workspace_engine/tests/code_review_triage.rs`：三处既有动作断言（`:160`、`:251`、`:390`，夹具 verdict 均为 `request_changes`）保留四项结论、把文案改为「可返修结论下为四项」；`approve` → 三项、读不到结论 → 三项、`retry_review` 后重新派生三个用例（已随 `3876c701` 落地，归档前核对断言与场景一一对应）（REQ 分诊门禁动作集合必须按最新审查结论派生）。
- [ ] 4.2 `web/src/pages/CodingWorkspaceControls.gate-actions.test.tsx`：三项门不渲染 `send_to_coder`、四项门渲染；指引文案取自门 payload 的 `latest_review_verdict`；断言不存在「按 `errorCode` 失败后置灰」与「按 verdict 改写动作」的路径（动作只来自 `available_actions`）（REQ 派生事实可观察）。
- [ ] 4.3 验证处理入口回归：`src/product/coding_workspace_engine/tests/verification_triage_gates.rs` 与 `src/web/handlers/coding/verification_surface.rs` 相关用例断言「转入前后动作集合逐项相同（含是否含 `send_to_coder`）、门仍 open、finding 不变」（REQ-CVT-03）。
- [ ] 4.4 互斥与不受影响面回归：`blocked` + 无可执行 finding 只落 `code_review_blocked`（`code_review_triage.rs:407` 起既有用例）；`internal_review_*` 三门仍为 `["retry_internal_review", "manual_continue", "abort"]`（`tests/provider_failure_recovery/retry_and_gate.rs:1076`）；`code_review_report_not_accepted` 与 `reviewer_rework_limit_reached` 动作集合不变（REQ 单次审查结论只允许落地一个 blocked gate + 本 change 非目标）。

## 5. 契约一致性与归档硬门

- [ ] 5.1 【归档硬门】归档前复核在途 change 的 header 重叠：`batch1-coding-autopilot-mainline` 在 `coding-verification-triage` 只 ADDED `REQ-CVT-06`；若它先归档，重取主规范确认本 delta 的 `REQ-CVT-03` MODIFIED 基线与三个 scenario 名（`从 coder 输出门转入`、`从 Code Review 门转入`、`重复转入被拒`）仍完整；其 delta 说明文字「REQ-CVT-03 保持不变」需在归档时同步失效说明（REQ-CVT-03）。
- [ ] 5.2 【归档硬门】按 RENAMED → MODIFIED 顺序核对主规范：`coding-code-review-triage` 归档后不得残留「恰好四人」「四动作」措辞，且不得出现 delta 段头；两条改名后的 requirement 名称与正文一致（REQ 分诊门禁动作集合必须按最新审查结论派生 + REQ 验证处理入口独立于分诊门动作集合）。
- [ ] 5.3 全量一致性复核：`openspec validate triage-gate-actions-by-verdict --strict` 绿；实现完成后逐条核对本 change 两个 delta 的每个 scenario 都有对应用例或明确的不适用理由；未实现工作包不勾选。

## 6. 存量门处置（不回写、不派生）

- [ ] 6.1 存量门保持落地时的动作集合：升级前已 open 的分诊门 MUST NOT 被回写、MUST NOT 在读取路径重新派生。验收口径：存量四动作门在升级后读取到的 `available_actions` 与升级前逐字一致，读路径不写盘（同条 requirement 第三段）。
- [ ] 6.2 存量门的可执行性由服务端前置兜底：对该门实例提交 `send_to_coder` MUST 被 `send_code_review_feedback_to_coder` 拒绝（verdict ∉ {`blocked`, `request_changes`} → `send_to_coder_latest_review_not_actionable`，`rework.rs:536-549`），不落 rework instruction、不改 attempt 状态、不关门；呈现层 MAY 用该码给替代动作指引，MUST NOT 增删/禁用动作。验收：存量门 + `approve` 结论下，UI 提交与直发 WS 两条路径行为一致（同条 requirement 第三段 + REQ 分诊门禁的送回 Coder 动作必须可用）。
- [ ] 6.3 【待裁定】是否在归档前主动清理存量门（一次性回写其动作集合，或统一收口存量门实例）：默认**不做**（不扫描、不改写历史门记录）；若后续裁定要做，MUST 先补一条明确的迁移 requirement（含幂等与审计口径）再进实现 Plan，不得在实现里顺手迁移。验收：归档前留下「不做／已做」的结论记录（同条 requirement 第三段）。

## 后续工作流

- 契约审阅通过后才展开 `cadence/plans/` 的实施 Plan 与 apply；本次工件创建不代表任何源码行为已实现。
- 非目标（不在本 change 实现）：`code_review_blocked` 门在不可执行 finding 情形下的 `send_to_coder` 死动作、`code_review_report_not_accepted` 门动作集合、`send_code_review_feedback_to_coder` 前置放宽、轮次超限路径语义调整。
