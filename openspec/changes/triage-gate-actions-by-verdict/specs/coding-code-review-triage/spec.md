# 规范增量：分诊门动作集合按最新审查结论派生

<!-- 来源：owner 2026-10-25 裁定（方案 A）。本 delta 改写本 capability 的三条既有 requirement：
     两条经 RENAMED 改名（原文断言「恰好四个人工处置动作」「独立于分诊门四动作」，与新规则矛盾），
     一条（送回 Coder 动作）保留名称并按派生规则重写正文。
     动机：动作集合恒含 send_to_coder，而服务端前置在 verdict ∉ {request_changes, blocked} 时必然
     拒绝，形成「按钮可见但必然失败」的死动作（现场 send_to_coder_latest_review_not_actionable）。
     BREAKING：三个 Code Review 分诊门的动作集合不再恒为四项。
     本 delta 不触碰同 capability 的其余 requirement（三门落地条件、单次结论至多一门、
     Reviewer 输出契约边界、manual_continue 续跑）。 -->

## RENAMED Requirements

- FROM: `### Requirement: 分诊门禁必须提供四个人工处置动作`
- TO: `### Requirement: 分诊门禁动作集合必须按最新审查结论派生`

- FROM: `### Requirement: 验证处理入口独立于分诊门四动作`
- TO: `### Requirement: 验证处理入口独立于分诊门动作集合`

## MODIFIED Requirements

### Requirement: 分诊门禁动作集合必须按最新审查结论派生

上述三个 Code Review 分诊门禁落地时，系统 MUST 按该 attempt 最新一次 Code Review 结论的 verdict 派生可执行动作集合：基础集合 MUST 恰好包含 `retry_review`、`manual_continue` 与 `abort`；当且仅当该 verdict ∈ {`request_changes`, `blocked`} 时，集合 MUST 额外包含 `send_to_coder`，其余 verdict（含 `approve`）下 MUST NOT 包含 `send_to_coder`。最新 verdict 读不到（该 attempt 无 Code Review 结论、结论不可解析或读取失败）时，系统 MUST 按不含 `send_to_coder` 派生，MUST NOT 以默认包含该动作兜底，且 MUST 仍落地该门禁而不是因缺少事实拒绝开门。系统 MUST NOT 在这些门禁上提供触发 plan repair 的动作，MUST NOT 在动作集合之外（例如门呈现面）自行构造 `send_to_coder`。

「最新一次结论」的口径 MUST 与送回 Coder 动作的既有读取口径一致（同一 attempt 结论集合中的最后一条）。派生所用事实 MUST 可观察：门 payload 的 `available_actions` 是权威动作集合，客户端 MUST NOT 自行增删动作；该 attempt 的 Code Review 结论集合（含每条结论的 verdict）随会话状态下发，客户端 MUST 以其中最新一条作为说明文案依据。系统 MAY 在门 payload 上额外携带派生成因（如最新 verdict 字段）；携带时其值 MUST 与派生所用 verdict 一致，且 MUST NOT 改变 `available_actions` 的权威性。

对升级前已落地且仍 open 的门禁，系统 MUST NOT 回写其动作集合；该门实例的呈现 MUST 依据最新 verdict 事实禁用 `send_to_coder` 并给出替代动作指引（`retry_review`／`manual_continue`），MUST NOT 依赖用户提交失败后再禁用。

#### Scenario: 门禁动作集合完整

- **WHEN** 任一 Code Review 分诊门禁落地，且该 attempt 最新一次 Code Review 结论的 verdict 为 `request_changes` 或 `blocked`
- **THEN** 其可执行动作集合 MUST 恰好包含 `retry_review`、`manual_continue`、`abort` 与 `send_to_coder`，且四项均可执行

#### Scenario: 门禁动作不触发 plan repair

- **WHEN** 用户在 Code Review 分诊门禁上执行任一可执行动作（含 `send_to_coder`）
- **THEN** 系统 MUST NOT 唤起 plan repair 流程，plan repair 的既有唤起条件保持不变

#### Scenario: 最新结论不可返修时动作集合为三项

- **WHEN** 门禁落地时该 attempt 最新一次 Code Review 结论的 verdict 为 `approve`
- **THEN** 可执行动作集合 MUST 恰好包含 `retry_review`、`manual_continue` 与 `abort`，MUST NOT 包含 `send_to_coder`，且门呈现面 MUST 仍可从该门推进（`retry_review` 与 `manual_continue` 可用），MUST NOT 出现任何本次必然失败的按钮

#### Scenario: 读不到最新 verdict 时按三项 fail-closed

- **WHEN** 门禁落地时无法读到该 attempt 的最新 Code Review 结论（无结论记录、结论不可解析或读取失败）
- **THEN** 系统 MUST 按不含 `send_to_coder` 派生（动作集合恰好三项），MUST NOT 以默认包含 `send_to_coder` 兜底，且该门禁 MUST 正常落地并保持可操作

#### Scenario: 重试代码审查后按新结论重新派生

- **WHEN** 用户在三动作集合的分诊门禁上执行 `retry_review`，随后新落地的 Code Review 结论 verdict 为 `request_changes`
- **THEN** 新的分诊门禁动作集合 MUST 包含 `send_to_coder`（四项），MUST NOT 因上一次结论不可返修而永久缺失或永久禁用该动作

#### Scenario: 派生事实可观察

- **WHEN** 任一 Code Review 分诊门禁落地
- **THEN** 门 payload 的 `available_actions` 逐项等于派生结果，且会话状态下发的该 attempt Code Review 结论集合中最新一条的 verdict 与派生所用事实一致

#### Scenario: 存量在途门不回写

- **WHEN** 升级前已落地且仍 open 的分诊门禁在升级后被读取或应答
- **THEN** 其 `available_actions` MUST NOT 被回写或重新派生；呈现层按最新 verdict 事实禁用该实例的 `send_to_coder` 并给出替代动作指引；对该门的 `send_to_coder` 提交仍 MUST 由服务端前置校验拒绝

### Requirement: 分诊门禁的送回 Coder 动作必须可用

当 Code Review 分诊门禁的动作集合包含「送回 Coder 返修」时，用户在门上执行该动作 MUST 走代码审查反馈返修路径，落地 rework instruction、把 stage 置为 `Coding` 并递增返修计数。最近一次审查结论的 verdict 为 `request_changes` 或 `blocked` 时该动作 MUST 可用，MUST NOT 因 verdict 为 `request_changes` 而拒绝执行，且 MUST NOT 走审查轮次超限反馈路径。

verdict 不属于 {`request_changes`, `blocked`} 时，动作集合 MUST NOT 包含该动作（见「分诊门禁动作集合必须按最新审查结论派生」）；服务端 MUST 同时拒绝对该门实例的任何 `send_to_coder` 提交（含存量在途门、陈旧快照与脚本直发），返回可识别的不可执行原因，现有错误码 `send_to_coder_latest_review_not_actionable` MUST 保持可观察，且 MUST NOT 落地 rework instruction、MUST NOT 改变 attempt 状态。

#### Scenario: verdict 为 request_changes 时送回 Coder

- **WHEN** 最近一次审查结论 verdict 为 `request_changes`，attempt 处于 Code Review 分诊门禁的 `blocked` 状态，用户执行 `send_to_coder` 并提供操作说明
- **THEN** 系统落地 rework instruction，attempt stage 变为 `Coding`，返修计数递增，且不返回不可执行错误

#### Scenario: 未提供操作说明时拒绝执行

- **WHEN** 用户在 Code Review 分诊门禁上执行 `send_to_coder` 但未提供操作说明
- **THEN** 系统 MUST 拒绝该动作并保持 attempt 处于 `blocked` 状态

#### Scenario: verdict 不可返修时提交被拒绝

- **WHEN** 最新审查结论 verdict 为 `approve`，用户经存量在途门、陈旧快照或脚本直接提交 `send_to_coder`
- **THEN** 系统 MUST 返回可识别的不可执行原因（`send_to_coder_latest_review_not_actionable`），MUST NOT 落地 rework instruction、MUST NOT 改变 attempt 状态，且 MUST NOT 因此关闭该门

### Requirement: 验证处理入口独立于分诊门动作集合

三个 Code Review 分诊门的可执行动作集合 MUST 只由该门落地时的最新审查结论 verdict 派生决定；转入「验证处理」MUST NOT 增删门动作：`retry_review`、`manual_continue` 与 `abort` 恒在，`send_to_coder` 是否在集合内与转入前保持一致。当门的 finding 涉及计划验证命令不可满足时，系统 MAY 在门旁提供转入独立「验证处理」面的入口；该入口 MUST NOT 作为门动作出现在门的动作集合中，转入本身 MUST NOT 唤起 plan repair，MUST NOT 关闭或改写该门及其 finding。验证处理中用户批准「计划修订」时，系统 MUST 经既有 plan amendment 链由验证处理记录发起，而不是作为该门的动作。验证处理得出用户批准的等价证据或限域例外结论后，原链继续的效果 MUST 与在该门执行 `manual_continue` 一致并留下验证处理审计。

#### Scenario: 转入验证处理不改变门动作

- **WHEN** Code Review 验证不完整门打开，用户点击「进入验证处理」
- **THEN** 门的可执行动作集合与转入前逐项相同（含是否包含 `send_to_coder`），门与 finding 保持不变，验证处理记录被创建

#### Scenario: 验证处理批准后续跑

- **WHEN** 用户在验证处理面批准可信等价证据
- **THEN** 原门关闭并按 `manual_continue` 语义续跑，finding 保留且关联该验证处理审计，不唤起 plan repair
