# 设计：把审查 finding 绑定到 Work Item 范围

## 1. 病根与三处落点

现场（`workspace_session_auto_e5f6b290-f231-479a-9c5c-c7d63c93515e`）三轮 reviewer 反复报同三条
finding，其中「仓库里那份前端规范文档的『中文 locale 尚未配置』缺口必须回写」是**产品侧文档的当前
状态**——改 Work Item 不可能让它消失。owner 两次说「这不归本 WI」，一次被当普通反馈收下，一次被判
`validation_reject`，自动化停在原地，界面只有「✓ 可直接确认（需先处理预判）」。

| 现象 | 病根 | 落点 |
|---|---|---|
| reviewer 范围无约束 | 7 处输出 schema 只规定格式与 severity，没有一句「finding MUST 落在本 WI 写面内」 | `prompts/review.rs`（5 处）+ `review_parts/{draft,single_candidate}_input.inc.rs`（各 1 处），共享契约 `prompts.rs::reviewer_output_contract` |
| 平台无法机器判别一条 finding 能否靠修本 WI 收敛 | schema 无 `disposition`/`target_work_item_id`，finding 无平台身份 | `web/workspace_ws_types/review.rs` + `parsers.rs` + `parsers/lenient_findings.rs` |
| owner 的「不归我管」只有一种语义 | feedback 被固定表述为「本回合的 typed 修改范围」，没有结构化排除出口 | `workspace_ws_types/in_.rs` + `decisions/inbound.rs` + durable 清单 |
| 下一轮 reviewer 拿不到上轮处置 | 单候选复评输入只有 IR + mechanical report（外加 fingerprint 清单） | `prompts/review_context.rs` |
| 门卡被拒后不说清发生了什么 | 失败 turn 只有 `failure_class`，无诊断、无下一步 | `conversational_gate_parts/revision_turn.inc.rs` + `run/provider_run.rs` + 新 durable 记录 |

## 2. 契约形状

### 2.1 finding 新增字段（wire，snake_case）

```
disposition?: "actionable" | "advisory"       # 缺失 = actionable（fail-safe）
target_work_item_id?: string                   # 指不到具体 WI 时留空
finding_key: string                            # 平台现算，序列化恒输出
```

`finding_key = lower_hex(sha256(severity + "\u{1}" + message))[..16]`，`severity` 取 wire 词表
（`blocking`/`must_fix`/`suggestion`）。

**落地形态（与实现一致）**：`ReviewFinding` 保持 `Serialize`/`Deserialize` derive，`finding_key` 是
`Option<String>`（`skip_serializing_if = "Option::is_none"`）的**投影字段**——不在任何 durable 记录里写，
只在两个投影点按内容补全：

- live：`ReviewVerdict::with_computed_finding_keys()`（`session_state_parts/historical_review.inc.rs`
  的 `review_complete_event_from_verdict`）；
- rebuild：`parsers.rs::normalize_historical_review_findings`（反序列化前按归一化后的
  `(severity, message)` 注入）。

两条路径调用同一个 `ReviewFinding::finding_key_for`，因此同 key；模型无法伪造（strict 白名单拒收该字段，
`Raw` 适配入口恒 `None`）。**没有**改成「手写 Serialize 恒输出」——那样会把投影身份写进每一份
durable/wire 字节。

### 2.2 平台硬规则（不依赖模型标对）

只计 `disposition == actionable` 的 finding：

- `parsers.rs::review_gate_for`（通用评审：`pass` → `RequiresRevision` 的判据）
- `parsers.rs::effective_work_item_plan_review_verdict`（单候选/Outline/Draft/Batch：`pass` → `revise` 的归一）
- `prompts/history_compaction.rs::render_open_required_findings`（必关闭清单，回喂 author）
- **作者返修输入面**（REQ-WSC-14「advisory 的 `required_action` MUST NOT 要求作者执行」）：
  `review/feedback.rs::format_actionable_review_feedback`（供 `review/routing.rs` 的
  `pending_revision_context` 与 `plan_outline/revision.rs::work_item_plan_outline_revision_feedback`
  使用）与 `work_item_split_engine/prompts/sc_revision.rs::render_review_revision_block`——
  `[review_findings]` 只列 actionable；被降级的 finding 只以「仅供提示，本轮不要修改」的单列
  提示（仅条数）出现，MUST NOT 带 message/required_action，也 MUST NOT 进入
  `[revision_directives]` 的「必须逐条修复」清单。全量 finding 视图
  （`format_review_feedback`）保留给留档/对话流报告，不进作者返修输入面
- `prompts/review_context.rs::single_candidate_previous_round_findings_section`（上一轮 finding
  身份清单）：durable 原始 verdict 先按 owner 已处置清单过滤，再按 advisory 分类过滤——同一份
  复评 prompt 不得既称「上一轮未收敛项」又称「该事实已被排除」
- 门卡「需要解决 N 项」计数：后端把 `disposition` 放进 findings 出站；前端计数同口径过滤（缺失＝actionable）

过滤口径唯一：`ReviewFinding::is_actionable()`（`disposition != advisory`，缺失＝actionable），
先于 `class_hint` / 旧轴 fallback 生效。

缺失 `disposition` 一律视作 `actionable`——历史 durable 记录与旧 provider 输出因此保持逐字阻断行为。

### 2.3 durable 已处置事实清单

落点与既有 gate turn / session 记录同族：`workspace-sessions/{session_id}/human-dispositions.json`，
走同一 live-write 准入（`resolve_session_scope` + `with_optional_entity_live_write`）。

```json
{ "records": [ { "finding_key": "0123456789abcdef",
                 "message_summary": "中文 locale 尚未配置",
                 "disposition": "excluded",
                 "recorded_at": "2026-10-30T12:00:00Z" } ] }
```

按 `finding_key` 幂等：同 key 重复提交零写、摘要不被覆盖（首次处置即事实）。

### 2.4 reviewer prompt 回喂

`review_context.rs::human_dispositions_section` → `[human_dispositions]` 段：

```
[human_dispositions]
owner 已处置、不得重复报为 actionable 的事实：
- 0123456789abcdef：中文 locale 尚未配置（owner 排除/已处置）
```

只注入**拿得到上轮上下文**的两条路径：单候选 Plan 复评（紧跟 `Previous Review Round Findings`）、
通用 workspace 评审（有 `latest_review_verdict`）。Outline / Draft / Batch / Projection 是全新候选
轮次，没有「上一轮处置」可对账，**不注入**（不硬塞）。清单为空时零注入，输入面字节不变。

### 2.5 门 turn 拒收可操作化

`validation_reject` 终态时落 `workspace-sessions/{session_id}/human-gate-rejections/{turn_id}.json`：

```json
{ "turn_id": "...", "rejected_feedback": "<原文逐字>", "reason": "<人话原因>",
  "next_actions": ["..."], "diagnostics": ["<逐字诊断>"], "recorded_at": "..." }
```

原因/下一步由 typed 诊断稳定码映射（`STRUCTURAL_ATTEMPTS_EXHAUSTED` / `STRUCTURAL_REPAIR_UNPRODUCIBLE`
/ `STRUCTURAL_EXISTING_GATE` / `STRUCTURAL_OPERATIONAL_WAIT` / `CANDIDATE_DRIFTED`），未知诊断给可行动
兜底文案——绝不把错误码直接抛给 owner。

出站：新帧 `human_gate_turn_rejected { turn_id, rejected_feedback, rejection_reason,
rejection_next_actions, rejection_diagnostics }`。**只在 turn 真正终态失败时发**；in-band 非终态拒收
（构建期 fail-closed、等待面）仍走既有 `ProtocolError(SC_MANUAL_REVISION_VALIDATION_REJECTED)`——
不冒充终态事实。

### 2.6 k3 双裁定补记：平台裁决已处置事实（REQ-WSC-14）

只把清单塞进 prompt 不算兑现——模型仍可照旧报 `actionable`（现场连报三轮）。因此新增
`review/disposition_override.rs`，在**有 session 上下文**的地方做平台归一：

```
finding_key ∈ 该 session 已处置清单  →  强制 disposition=advisory
revise 且 无任何 actionable  →  归一为 pass（不驱作者返修）
```

归一面只覆盖 `revise`：`revise_batch` / `plan_reopen_required` **不参与**归一——它们是专用 legacy
返修机制（既有契约用例依赖其既有行为），`revise_batch` 的归一留在 policy 路由侧，不在此处重算。

调用点（同一纯函数，不另立第二口径）：

| 调用点 | 覆盖的判定面 |
|---|---|
| `review/routing.rs::complete_review`（持久化原始 verdict **之后**） | `latest_review_verdict`（必关闭清单/下一轮 prompt）、policy 路由、门快照 findings |
| `lifecycle.rs::finalize_persistent_derivation` | 刷新/重启后的同一投影 |
| `review/routing_scope.rs::single_candidate_consecutive_repeated_finding_count` | 指纹闸门两侧（durable 里存的是原始 verdict） |
| `work_item_plan_policy/classify.rs::classify_finding` | `actionable_findings`（返修预算/transition budget）——`disposition != actionable` 先于 `class_hint` 生效 |

normalize 结果**不落 durable**（durable 只保 provider 原始 verdict + 已处置清单两事实），刷新后由同一清单
重新算出同一结果。

### 2.7 P0-3 修法：durable 零改写

`latest_review_verdict_from_node_details` 原先用「归一化后 `to_value` ≠ persisted」判是否需要回写；由于
`normalize_historical_review_findings` 会按内容注入 `finding_key`，该比较**恒不成立**，于是每次读历史记录
都把投影字段写回 durable。修法：比较两侧前都剔除 `finding_key`（`verdict_without_finding_keys`），
只在真正的历史归一（severity 归一 / `impact` 并入 message）发生时才回写。

### 2.8 排除撤销（REQ-WSC-16）

清单原只有 push 去重，没有删除入口——owner 排错后该 key 永久降级，没有「重新提出为 actionable」的路径。
新增 store `revoke_human_dispositions`（按 key 删除、幂等、零写）与人工命令
`POST /api/workspace-sessions/{session_id}/human-actions` 的
`{"type":"revoke_finding_exclusions","command_id":"…","finding_keys":["…"]}` → 200
`{"command_id":"…","state":"accepted","remaining_finding_keys":[…]}`。撤销与门身份无关（会话级 durable 事实），
因此不带 `expected_gate_id`。

## 3. 与既有实现的关系（不新造并行状态机）

- 复用 `HumanGateTurn`（不新增字段，避免 36 处字面量扩散）：拒收信息是**并列的 durable 记录**，
  以 `turn_id` 关联。
- 复用 `ReviewGate`/`WorkItemPlanReviewVerdict` 归一链：新规则只加一个 `is_actionable()` 过滤。
- 复用 `owner_guidance_rules.rs` 的共享契约注入点（`reviewer_output_contract`）：范围/收敛约束与
  REQ-WSC-11 的人话约束同一份措辞、同一处注入，新增第 8 个 builder 不会漏。
- 排除动作走既有入站命令族（`human_gate_feedback` 加可选字段），不新增命令。

## 4. 兼容与失败语义

| 情况 | 行为 |
|---|---|
| 旧客户端不发 `excluded_finding_ids` | 与改动前逐字相同 |
| 历史 durable verdict 无 `disposition`/`finding_key` | 照旧解析；语义 actionable；序列化时补 key |
| 模型输出非法 `disposition` | `invalid_finding_field`（既有不合规通道） |
| 模型把范围外事实标成 `must_fix` | 平台仍不阻断（`advisory` 判定优先） |
| 排除时找不到对应 finding 摘要 | 仍落 key（排除生效），摘要为空串 |
| 拒收记录写盘失败 | 仍回执可操作信息并打点（owner 优先知情），日志留证 |
| `feedback` 为空 + 合法排除 key（排除-only 帧） | 落 durable 即受理（回 `human_gate_exclusions_accepted`），不建修订 turn、不判「反馈为空」 |
| 排除落盘失败（排除-only 帧） | 回 `Error`，不冒充已受理（REQ-WSC-15「发起失败不显示为已排除」） |
| 模型对已处置事实仍报 `actionable` | 平台强制降级为 advisory：不驱返修、不进必关闭清单、不占 transition budget |
| `revise` 但无任何 actionable finding | 归一为 pass（不驱作者返修） |
| 读历史 review verdict | durable 零改写（比较前剔除投影 `finding_key`） |
| 撤销排除 | 删除该 key（幂等）；其后再报 actionable 照常阻断 |

## 5. 验收锚点（TDD）

1. `parsers.rs`：缺失 disposition → actionable 且仍阻断；`must_fix + advisory` → `UserConfirmAllowed`；
   单候选 `pass + advisory` → 仍 Pass；非法值 → `invalid_finding_field`。
2. `parsers.rs`：`finding_key` 16 位 hex、wire 投影带 key、durable 反序列化后重算逐字相等。
3. `history_compaction.rs`：advisory 不进必关闭清单；同 payload 去掉 disposition 仍进。
4. `lifecycle_store/tests/human_dispositions.rs`：清单幂等 + 重启可读 + key 形态校验；拒收记录
   durable/幂等/可读，guidance 对未知诊断给可操作兜底。
5. `review_context.rs`：`[human_dispositions]` 渲染逐条 key+摘要并声明不得再报 actionable；空清单零注入。
5a. `review/disposition_override.rs`：owner 排除 key → 强制 advisory + `revise` 归一为 pass；未排除的
    actionable 照常阻断；空清单零改动；P0-1 加固（契约类 advisory 仍 actionable）与 owner 事实的优先级。
5b. `tests/review_disposition_platform.rs`：真实 store 的清单 → 引擎平台裁决；撤销后恢复 actionable；
    下一轮 reviewer prompt 段（`human_dispositions_section`）携带该事实。
5c. `tests/review_disposition_platform.rs`：读历史 verdict 后 `NodeDetail.verdict` 逐字不变（无 `finding_key`）。
5d. `tests/review_disposition_platform.rs`：`build_session_state` 携带最近一次拒收四要素；从未拒收省略。
5e. `tests_evaluate.rs` / `tests_classify.rs`：advisory 不进指纹集合、不耗 transition budget；`disposition`
    压过 `class_hint`。
5f. `it_core`：排除-only WS 帧被受理（`human_gate_exclusions_accepted`）、清单落盘、无修订 turn。
5g. `lifecycle_store`：排除撤销按 key 删除且幂等。
6. `owner_guidance_rules.rs`：7 处 schema 均含 `disposition`/`target_work_item_id`，共享契约携带范围/收敛约束。

## 6. REQ-WSC-15：并行前端实现的语义核对（既成事实，逐条对照）

条款在本 change 成文（同一 change 目录只有一位 owner）；门卡 UI 呈现由并行改动实现。
下表按四条款逐条核对**语义是否被真正兑现**：

| 条款 | 结论 | 证据（file:line） |
|---|---|---|
| 1. 被拒时可操作：四要素（被拒反馈原文 / 可读原因 / ≥1 下一步 / 逐字诊断） | **已兑现** | `web/src/components/chat-workspace/entries/GatePromptEntry.tsx:141`（入口读 metadata → `gateRejectionInfoFromEntry(entry)`）、`:861` `gateRejectionInfoFromEntry`（四字段解析 + 全缺席返回 null）、`:903` `GateRejectionBlock`（逐字渲染，诊断用 `<pre>` 不截断）、`:534`（仅非 null 才渲染）。后端侧由本 change 的 `human_gate_turn_rejected` 帧 + durable `human-gate-rejections/{turn}.json` 供给 |
| 2a. 全缺席不渲染空白块 | **已兑现** | `GatePromptEntry.tsx:881-888`：四字段全空 → `return null`（`:887`），`GateRejectionBlock` 不挂载 |
| 2b. advisory 与 actionable 分组隔离 | **已兑现** | `finding-list.tsx:140` `isAdvisoryDisposition`（缺失＝actionable）、`:179-183`（advisory / actionable-blocking / actionable-optional 三组切分）、`:213` 组标题「仅供参考（不阻断）」、`:303` `AdvisoryBadge` 行级徽章（定义 `:368`） |
| 2c. 原因行「需要处理 N 项」只计 actionable | **已兑现** | `GatePromptEntry.tsx:112` `findings.filter(isActionableBlockingFinding)`（`requiredFindings`）、`:198-199` 用该计数调 `gateWhyRequiredCopy`；`finding-list.tsx:149-150` `isActionableBlockingFinding = !isAdvisoryDisposition && isBlockingFinding`。state 侧同口径见 `state/gate-prompt-copy.ts:25-30` `isActionableFinding` + `:34-39` `actionableFindings`（本 change） |
| 2d. advisory 的 `required_action` 非祈使样式 | **已兑现** | `finding-list.tsx:312-322`：advisory 走 muted 文本（`text-[var(--aria-ink-muted)]`，`data-testid="advisory-required-action"`），非 advisory 才 `font-medium` |
| 3a. 逐条「排除此项」，仅在有平台 key 时渲染 | **已兑现** | `finding-list.tsx:353-360`：`onExcludeFinding && finding.findingKey` 才渲染「排除此项」（`data-testid="finding-exclude-button"`；无 key fail-closed） |
| 3b. 发起失败不显示为已排除 | **已兑现** | 发送器已移入 `entries/gate-finding-exclusion-state.tsx:68-90` `handleExcludeFinding`：`:76` `sent === true` 才在 `:81` 置 `excludedFindingKeys`，否则返回 false 且不置位；被引擎拒收时 `:138` `effectiveExcludedFindingKeys` 全部失效 |
| 4. 排除成功即时反馈（已排除状态 + 下一轮不再报为需处理） | **已兑现** | `entries/gate-finding-exclusion-state.tsx:183-186` 本地注记「已排除 N 项：…下一轮评审不再报为需处理」、`finding-list.tsx:329-338` 行级「已排除」徽章（本地置位，不等下一轮刷新）、`:337-350` 撤销排除入口 |

对侧依赖（本 change 提供，已被上述实现消费）：`metadata.findings[]` 的 camelCase 别名
`findingKey` / `targetWorkItemId`（`state/workspace-chat-rebuild.ts:701` 的 `withFindingViewAliases`），
门卡 metadata 的 `rejectedFeedback` / `rejectionReason` / `rejectionNextActions` /
`rejectionDiagnostics`（同文件 `:661-664` 由 `projection.turn.rejection` 投影），以及
`actions.feedback(feedback, { excludedFindingIds })` 的双参门面（无排除项时保持旧 2 参调用形态，
接线在 `entries/gate-finding-exclusion-state.tsx`）。
