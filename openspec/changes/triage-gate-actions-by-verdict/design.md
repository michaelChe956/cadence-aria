# 设计：分诊门动作集合按最新审查结论派生

## 1. 判定表（唯一派生规则）

设 `verdict` = 该 attempt 已持久化的 Code Review 结论中最新一条的 verdict（口径与 `send_code_review_feedback_to_coder` 的读取一致：结论集合最后一条，`src/product/coding_workspace_engine/rework.rs:529-541`）。

| 最新 verdict | `retry_review` | `manual_continue` | `abort` | `send_to_coder` | 集合基数 |
|---|---|---|---|---|---|
| `request_changes` | ✓ | ✓ | ✓ | ✓ | 4 |
| `blocked` | ✓ | ✓ | ✓ | ✓ | 4 |
| `approve` | ✓ | ✓ | ✓ | ✗ | 3 |
| 读不到（无结论／不可解析／读取失败） | ✓ | ✓ | ✓ | ✗ | 3 |

Wire 值来自 `ReviewVerdict` 的 `#[serde(rename_all = "snake_case")]`（`src/product/coding_models/review.rs:79-85`）。

**为什么 `blocked` 也允许 `send_to_coder` 是安全的**：三个分诊门与 `code_review_blocked` 门互斥——verdict=`blocked` 且无可执行 finding 时只落 `code_review_blocked`（`src/product/coding_workspace_engine/code_review.rs:246-269`；spec 既有 requirement「单次审查结论只允许落地一个 blocked gate」）。因此在这三个分诊门上，`verdict ∈ {request_changes, blocked}` 与 `send_code_review_feedback_to_coder` 的可执行前置（`rework.rs:542-549`）等价，派生结果不会与执行能力脱节。

## 2. 事实源与可观测性

派生发生在**服务端建门时**，事实源就是内存中刚持久化的那条 Code Review 结论（`code_review.rs:283-310` 三个 reason code 的分支），与 `evidence_refs` 里的报告 id 同源。契约要求的「可观察」用两级落地：

- **权威面**：门 payload 的 `available_actions`（`CodingGateRequired`，`src/product/coding_models/gate.rs:137-161`）就是派生结果本身；客户端 MUST NOT 自行增删。
- **原始事实面**：会话状态消息已同时下发 `pending_gates` 与该 attempt 的 `code_review_reports`（`src/web/coding_ws_handler/protocol.rs:65`），最新一条的 verdict 即派生所用事实。客户端的「为什么少了这个按钮」指引文案据此生成。

**选项与取舍**

| 方案 | 内容 | 代价 | 结论 |
|---|---|---|---|
| A（推荐） | 不改 gate schema；权威面 + 原始事实面如上 | 客户端要取 `code_review_reports` 最后一条做文案 | **采用**。零新写路径、零 `CreateBlockedGateInput` 字面量改动（当前 45 处，其中 25 处在测试）、零前端类型新增 |
| B | 给 `CodingGateRequired` 加 `latest_review_verdict`（`#[serde(default)]`） | 需新 store 写入口；`CreateBlockedGateInput` 有 45 处字面量，直接加字段会全部报错，只能仿 `attach_blocked_gate_triage_summary` 的独立回写，产生「建门」与「写 verdict」两写之间的崩溃窗口 | 可选。若实现认为门需要自描述再走 B：`available_actions` 仍为权威，字段缺失时客户端不得自行增删动作 |

## 3. fail-closed 依据

读不到 verdict 时选择「不含 `send_to_coder`」而不是「含」：

- 该动作一旦执行会写 rework instruction、改 stage、递增返修计数（`rework.rs:575-597`），属**有副作用的不可逆写**；缺事实时放出该动作等于用默认值赌一次业务写入。
- 服务端前置本身对 verdict 未知的情形也只能拒绝（读不到结论直接报 `send_to_coder_missing_code_review_report`，`rework.rs:536-541`）；契约与实现同向，才不产生新的「可见但必然失败」。
- 若缺事实时拒绝开门，用户会失去 `retry_review`／`manual_continue`／`abort` 这三个永远安全的出口——因此缺事实 MUST 只影响这一个动作，不影响门本身落地。

## 4. 派生时机与重新派生

- 派生**只在建门时**执行一次；不在读取时重新计算（读路径不写盘，避免 GET/WS 推送触发业务变更）。
- `retry_review` 先把门按既有原语收口（`resolved_action=retry_review`）并唤回 runner（`should_resume_runner_after_gate_response` 白名单已含 `retry_review`，`src/web/coding_ws_handler/runner.rs:316-339`），新一轮 Code Review 落地后重新建门 → 按新 verdict 重新派生。
- 若建门命中 store 的去重路径（同一 attempt／node／stage／reason_code 的 open 门，`src/product/coding_attempt_store/gate.rs:1334-1348`），该路径本来就会用新输入覆盖 `available_actions`，因此「同门实例上重新派生」也成立，无需额外机制。
- 契约明确禁止把上一次结论的不可返修性做成持久禁用位：不存在这样的状态字段，也不得新增。

## 5. 存量在途门（不回写）

- 升级前已 open 的分诊门带着四动作落盘。**不回写**：不扫描、不改写存量门 JSON，避免读路径副作用与并发写风险，也让审计保留「当时派生依据」的原样。
- 呈现层对这些实例按第 2 节的原始事实面（最新 verdict）继续禁用 `send_to_coder` 并给替代动作指引；判定依据从「上一次提交失败返回 `send_to_coder_latest_review_not_actionable`」改为「最新 verdict 不属于可返修集合」——这是本轮前端过渡缓解的**唯一保留形态**（存量门专用），新门不再触发它。
- 服务端前置拒绝语义不变（第 1 节），所以即便客户端漏了禁用，也不会产生错误业务写入。
- 存量门会在 `retry_review`／`manual_continue`／`abort`／`send_to_coder` 任一次应答后自然消失，无需迁移脚本。

## 6. 边界（本 change 不改）

- `code_review_blocked` 门（`provider_failure.rs:157-163` 的 else 分支给 `[retry_review, send_to_coder, abort]`）在 verdict=`blocked` 且无可执行 finding 时，其 `send_to_coder` 仍是必然失败的死动作。它不属于本 capability 的三个分诊门，本 change 仅记录，不修。
- `code_review_report_not_accepted`（报告未采信）门同理。
- `internal_review_*` 三动作、`failed_code_review_recoverable` 单动作、`reviewer_rework_limit_reached` 三动作、coder 侧 `coding_output_human_triage`（`retry_coding`／`abort`）与 `coding_operational_blocker`（三动作）全部不变。
- 读路径的 `normalize_blocked_gate` 只对 `code_review_blocked` 补 `send_to_coder`（`coding_attempt_store/gate.rs:1432-1458`），本 change MUST NOT 把它扩展到三个分诊门，否则会在读取时重新塞回被派生掉的死动作。

## 7. 兼容与迁移风险

- **BREAKING 面**：三个分诊门不再恒为四动作。现存依赖「恒四动作」的断言在 `src/product/coding_workspace_engine/tests/code_review_triage.rs:157`、`:243`、`:377`（三处夹具 verdict 均为 `request_changes`，因此结论不变，只需把断言文案从「必须为四项」改为「可返修结论下为四项」）。前端 `web/src/pages/CodingWorkspaceControls.gate-actions.test.tsx` 的门动作用例需改为按 `available_actions` 断言。
- **其它 spec 依赖**：`openspec/specs/` 内只有 `coding-code-review-triage` 与 `coding-verification-triage` 断言「恰好四项」，本 change 两者都改；`openspec/changes/archive/**` 的历史文本不改（已归档，属历史记录）。
- **在途 change**：`batch1-coding-autopilot-mainline` 在 `coding-verification-triage` 只 ADDED `REQ-CVT-06`，与本 delta 的 `REQ-CVT-03` 无 header 重叠；其 delta 说明文字「REQ-CVT-03 保持不变」在本 change 归档后失效，归档前需复核（作为 tasks 的归档硬门）。
- **跨 capability 文案**：`coding-verification-triage` REQ-CVT-05 的文案口径不涉及动作集合，无需改动。

## 8. 未验证假设

- 假设：三个分诊门上不存在「verdict=`blocked` 且无可执行 finding」仍落三门之一的路径。已按 `code_review.rs:246-269` 的互斥分支核实；若实现新增分支绕过该互斥，派生规则会与服务端前置（含 actionable findings 判定，`rework.rs:448-455`）不一致，需在实现 Plan 的测试中加互斥回归（tasks 4.4）。
