# 变更提案：分诊门动作集合按最新审查结论派生

## Why

三个 Code Review 分诊门（`code_review_output_human_triage`、`code_review_verification_incomplete`、`code_review_operational_blocker`）今天无条件提供四个动作：`retry_review`、`send_to_coder`、`manual_continue`、`abort`（`src/product/coding_workspace_engine/provider_failure.rs:145-156`）。

而服务端对「送回 Coder 返修」有硬前置：`send_code_review_feedback_to_coder` 只在最新一次审查结论 verdict ∈ {`blocked`, `request_changes`} 时才执行，否则返回 `send_to_coder_latest_review_not_actionable`（`src/product/coding_workspace_engine/rework.rs:542-549`）。分诊门恰恰可以在 verdict 不属于该集合时落地（例如 verdict=`approve` 但 finding 命中运维阻塞路由，`src/product/coding_workspace_engine/code_review.rs:283-310`）。于是产生**死动作**：按钮可见、点了必然失败，用户只能靠一次失败才知道该动作不可用。

现场已有临时缓解：前端按服务端返回的 `send_to_coder_latest_review_not_actionable` 对该门实例把「提交给 Coder 修复」置灰并就近指引（`web/src/pages/CodingWorkspaceControls.tsx:453-476`、`:641`）。这是**事后**兜底，不是权威派生：用户仍要先点一次、失败一次，且动作集合与实际可执行能力长期不一致。

owner 2026-10-25 裁定（方案 A）：把「分诊门四动作」的契约改成**按最新审查结论派生**——`send_to_coder` 只在 verdict ∈ {`request_changes`, `blocked`} 时出现，其余 verdict 下不出现；读不到 verdict 时按不含处理。本 change 只落地契约，不含实现。

## What Changes

- **动作集合按 verdict 派生**：三个 Code Review 分诊门落地时，基础动作集合恰好为 `retry_review`、`manual_continue`、`abort`；**当且仅当**该 attempt 最新一次 Code Review 结论的 verdict ∈ {`request_changes`, `blocked`} 时，集合额外包含 `send_to_coder`。其余 verdict（含 `approve`）下 **MUST NOT** 包含。
- **fail-closed**：最新 verdict 读不到（无结论、不可解析、读取失败）时按不含 `send_to_coder` 派生，**不得**默认包含；且不得因缺事实而拒绝落地该门（门本身仍必须可用）。
- **派生事实可观察**：门 payload 的 `available_actions` 是权威动作集合（客户端 MUST NOT 自行增删）；该 attempt 的 Code Review 结论集合（含每条 verdict）随会话状态下发（`src/web/coding_ws_handler/protocol.rs:65`），客户端以其中最新一条作为说明文案依据。可选在门 payload 上追加派生成因字段，但不得改变 `available_actions` 的权威性。
- **派生时机**：仅在门落地时派生；`retry_review` 产生新结论后，新门按新结论重新派生——不得因上一次结论不可返修而永久禁用或永久缺失该动作。
- **既有约束保留**：verdict=`request_changes` 时 `send_to_coder` MUST 可用（改写措辞而非删除）；verdict 不可返修时服务端前置拒绝语义保留（`send_to_coder_latest_review_not_actionable` 仍可观察）。
- **验证处理入口边界改写**：「验证处理」入口仍独立于门动作集合、不作为门动作出现、不唤起 plan repair；但「恰好四项」的措辞改为「转入不改变由 verdict 派生的集合」。
- **BREAKING**：三个 Code Review 分诊门的动作集合不再恒为四项。任何依赖「恒四动作」的断言、文案与客户端逻辑都必须改为按 `available_actions` 渲染。

## 过渡期现状（本轮已落地，非本 change 范围）

- 前端过渡缓解已存在：按服务端已判定的 `send_to_coder_latest_review_not_actionable` 事实，对**该门实例**把「提交给 Coder 修复」置灰 + 就近指引，**服务端动作集合未改**（`web/src/pages/CodingWorkspaceControls.tsx:453-476`、`:641`）。
- 本 change 描述**目标态**：服务端权威派生，前端按 `available_actions` 渲染，删除「先失败一次才知道不可用」的路径。
- 已经落在磁盘上的在途门（存量 open gate）**不回写**：其动作集合保持落地时的样子；呈现层按最新 verdict 事实对该实例继续禁用 `send_to_coder` 并给替代指引（依据从「提交失败的错误码」改为「最新 verdict 事实」），服务端前置拒绝语义不变。

## Capabilities

### New Capabilities

- 无。派生规则落在既有分诊门能力的既有语义邻域内。

### Modified Capabilities

- `coding-code-review-triage`：以 RENAMED 把 `分诊门禁必须提供四个人工处置动作` 改名为 `分诊门禁动作集合必须按最新审查结论派生`（正文与场景整体改写为派生规则），把 `验证处理入口独立于分诊门四动作` 改名为 `验证处理入口独立于分诊门动作集合`（措辞从「恰好四项」改为「派生结果不被转入改变」），并 MODIFIED `分诊门禁的送回 Coder 动作必须可用`（对齐「集合含该动作时必然可执行 / 不含时服务端仍拒绝」）。
- `coding-verification-triage`：MODIFIED `独立验证处理入口（REQ-CVT-03）`，把「三个 Code Review 分诊门 SHALL 仍恰好提供四动作」改为「SHALL 保持转入前由最新 verdict 派生的动作集合（是否含 `send_to_coder` 不变）」。

## Impact

- 本 change 只形成契约工件：不修改 `src/`、不改前端、不改 `cadence/`、不跑 cargo、不重启服务。
- 受影响的实现面（由后续 Plan 展开）：`create_review_blocked_gate` 的三门分支（`provider_failure.rs:145-156`）改为按最新 verdict 派生；客户端渲染与过渡置灰逻辑改为消费 `available_actions` + 最新 verdict 事实；分诊门测试与前端门动作测试的断言。
- 不受影响的既有语义：`code_review_blocked` 门（verdict=`blocked` 且无可执行 finding）、`code_review_report_not_accepted` 门、`internal_review_*` 三门、`failed_code_review_recoverable` 恢复门、`reviewer_rework_limit_reached` 门、coder 侧 `coding_output_human_triage` / `coding_operational_blocker` 门——它们不是本 capability 的三个分诊门，动作集合保持现状。
- 单条审查结论仍至多落地一个 blocked gate（既有 requirement 不变）；`code_review_blocked` 情形下 verdict=`blocked` 但不可执行 finding 走既有互斥分支，因此本派生规则在三个分诊门上与 `send_code_review_feedback_to_coder` 的前置判定等价。

## 非目标

- 不修 `code_review_blocked` 门在 verdict=`blocked` 且无可执行 finding 时的 `send_to_coder` 死动作（另一条路径，单独处理）。
- 不修 `code_review_report_not_accepted` 门的动作集合。
- 不改 `send_code_review_feedback_to_coder` 的 verdict 前置，也不放宽 `retry_review` 轮次超限路径的语义。
- 不改 `manual_continue` / `abort` 的语义与续跑白名单。
- 不引入新的门类型、不改门存储结构（新增派生成因字段仅在实现 Plan 判定必要时additive 追加）。
