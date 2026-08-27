# work-item-typed-outcome-policy Specification

## Purpose

为 workitem 段提供确定性运行控制：reviewer finding 经显式 parser adapter 变为机器可读分类，由中央策略层依据预算、指纹与服务端 review scope 裁决 typed outcome，使每次运行到达 `completed`、`awaiting_human`、`stopped_needs_human` 或 durable `failed`，消除死循环并支持断线恢复。

## Requirements

### Requirement: finding 机器可读分类与失败关闭（REQ-TOP-01）

系统 SHALL 使 reviewer finding 携带 `category`、`class_hint`、`contract_field`；历史数据缺三字段 SHALL 恢复为 None，未知字段名 SHALL 被拒绝。系统 SHALL 通过 `parse_review_envelope(&serde_json::Value) -> Result<ParsedReviewEnvelope, ReviewStructuredOutputError>` 显式预检或转换 category/class_hint；不得以 `Option<Enum>` 的默认 serde 行为处理未知字符串。

parser SHALL 保留 `ParsedReviewEnvelope { raw_verdict, normalized_gate, findings }`。分类 SHALL 使用 raw_verdict；routing SHALL 使用策略 outcome；parser SHALL NOT 因强 finding 将原生 needs_human 改写为 revise 后供分类器使用。

未知 category 的唯一失败链路 SHALL 为 `UnknownFindingCategory(parser) → ClassificationError::UnknownCategory → FatalReason::UnknownCategory → unknown_finding_category diagnostic`；未知 class_hint 对应为 `UnknownFindingClassHint → ClassificationError::UnknownClassHint → FatalReason::UnknownClassHint → unknown_class_hint`。字段类型错误 SHALL 到 `InvalidStructuredOutput`；这些错误均 SHALL durable failed，绝不降级 NeedsHuman/HumanRequired。

#### Scenario: 旧格式 finding 兼容解析

- **WHEN** 解析不含新字段的历史 finding 数据
- **THEN** 三个字段按缺省值恢复为 None，解析不失败

#### Scenario: 未知 category 失败关闭

- **WHEN** finding 的 category 为枚举外未知值
- **THEN** parser 返回 UnknownFindingCategory，经唯一映射以 unknown_finding_category diagnostic durable failed；不得降级为 NeedsHuman 或 HumanRequired

#### Scenario: 未知 class_hint 失败关闭

- **WHEN** finding 的 class_hint 为枚举外未知值
- **THEN** parser 返回 UnknownFindingClassHint，经唯一映射以 unknown_class_hint diagnostic durable failed；不得降级为 NeedsHuman 或 HumanRequired

### Requirement: 中央策略裁决与预算边界（REQ-TOP-02）

系统 SHALL 提供仅消费已分类 finding、运行历史与预算的纯策略评估器，裁决 `valid`、`repairable`、`human_required`、`fatal` 四类 typed outcome。原始 verdict 与 severity SHALL NOT 直接驱动状态跳转。优先级固定为：转换预算耗尽 → 重复指纹 → 返修需求 → human_required → valid。

`max_repairs=0` 或 repair budget 耗尽遇 repairable SHALL 产生 HumanRequired；`max_transitions=0` 或已达上限 SHALL 产生 Fatal(TransitionBudgetExhausted)；`max_manual_repairs=0` 的人工门 SHALL 只留批准/终止；持久化计数超预算、溢出或不变量不一致 SHALL 产生 StateCorruption。transitions_used SHALL 仅在 stage transition 成功 durable commit 后增加。advisory SHALL NOT 写入 seen fingerprints。

#### Scenario: 重复指纹不自动再试

- **WHEN** 本轮非 advisory finding 的指纹已存在于运行历史
- **THEN** 裁决为 HumanRequired(RepeatedFingerprint)，交互模式进入 awaiting_human，auto 模式 stopped_needs_human；不触发自动返修

#### Scenario: 自动返修预算耗尽

- **WHEN** 自动返修次数已达上限或 max_repairs 为 0，且存在 mechanical error 或 repairable 需求
- **THEN** 裁决为 HumanRequired(RepairBudgetExhausted)，交互模式 awaiting_human，auto 模式 stopped_needs_human；不得 fatal

### Requirement: 终态矩阵与 durable reservation（REQ-TOP-03）

首次 aggregate repairable（机械错误与 review repairable 共用 `max_repairs=1`）SHALL 自动返修一次；复评未解决、重复指纹、repair budget 耗尽和原生 human_required SHALL 分流为 interactive awaiting_human 或 auto stopped_needs_human；transition budget 耗尽、状态损坏、未知协议、持久化失败和安全不变量破坏 SHALL fatal。fatal 与 stopped_needs_human SHALL 分别计数和展示。

自动返修的 durable reservation SHALL 持有 token、owner、provider_start_idempotency_key、状态和 commit 信息。CAS 先写 Reserved，成功记录 provider 启动后才一次性增加 repairs_used 并转 ProviderStarted；启动失败 SHALL 按 token 幂等转 Released 且 repairs_used 不增。恢复时 Reserved 无启动账本 SHALL 释放；ProviderStarted/Committed SHALL 使用同一 key 恢复，不得二次启动。

#### Scenario: auto 模式遇人工需求不空转

- **WHEN** auto_if_valid 运行裁决出 human_required
- **THEN** 运行以 stopped_needs_human 结束，保存可恢复人工门快照与诊断，不挂起等待、不标 fatal

#### Scenario: 人工返修后同指纹重现回到同一门

- **WHEN** interactive 模式下人工反馈返修后同一 finding 指纹再次出现
- **THEN** 运行回到 awaiting_human 并展示指纹历史，不自动再试、不转 fatal

### Requirement: review 两阶段与 scope 契约（REQ-TOP-04）

系统 SHALL 以**被审产物为锚**执行两轮 review 预算：一个 review cycle 为同一被审产物（outline 节点 / 某 draft 的 outline_id / 某 batch_id）从首次 review 到 pass 或终态；初评最多一次、复评最多一次均为 **cycle 级**预算。一个 session 合法包含多个 cycle（outline → 各 draft → batch），cycle 间互不消耗预算；session 级 review 总计数仅用于指标与诊断，不做门控。仅当 cycle 内初评或机械校验产生可修复问题且聚合自动返修预算未消费时，允许一次聚合返修及最多一次复评。复评 SHALL 仅验证原 finding 指纹是否重现，并重跑同一 invocation 的机械校验；SHALL NOT 做开放式新审查、changed-path 归因或对 finding 自由文本作路径推断。复评后未解决 finding SHALL NOT 自动返修。

系统 SHALL 持久化 repairs_used、manual_repairs_used、initial_review_count、verification_review_count，以及按 cycle 的 `review_cycles: Map<cycle_key, {initial_count, verification_count}>`（cycle_key = 被审产物标识）。scope SHALL 使用 `Initial { initial_revision_id, scope_digest } | Verification { original_fingerprints, repaired_revision_id, mechanical_report_ref, scope_digest }`；初评 SHALL NOT 用空字符串承载不适用复评字段。scope digest SHALL 使用版本化 canonical 字节编码、字段排序、BTreeSet 稳定序、SHA-256 和 `review_scope_v1:<64 lowercase hex>` 前缀。digest 不符、机械报告缺失或不符等**结构性** scope 违例 SHALL 为 Fatal(ProtocolViolation)；复评 finding 指纹不在原始集合时，因可能是 reviewer 改写措辞或发现关联问题，SHALL 产出 `HumanRequired(VerificationNewFindings)`，并按终态矩阵进入 `awaiting_human` 或 `stopped_needs_human`，不得 Fatal 或自动返修。

Finding identity SHALL use `category + contract_field` when category is present, excluding class and free-text message; this keeps the identity stable when reviewer wording changes. Findings from the legacy schema without category SHALL retain the normalized `class + message + contract_field` fallback algorithm.

#### Scenario: 复评新 finding 降级人工处理

- **WHEN** 复评 finding 的指纹不在原始 invocation scope 集合中，或 reviewer 仅改变同类同字段 finding 的措辞
- **THEN** 不将该 finding 视为结构性协议 Fatal；前者按 `VerificationNewFindings` 进入 awaiting_human 或 stopped_needs_human，不触发第二次自动返修，后者通过结构化 fingerprint 保持原身份

#### Scenario: 复评结构性 scope 违例仍失败关闭

- **WHEN** scope digest 不符，或 invocation 要求的机械校验报告缺失/不符
- **THEN** 按 `Fatal(ProtocolViolation)` durable failed，绝不进入人工门

### Requirement: 运行状态持久化、CAS 与兼容（REQ-TOP-05）

系统 SHALL 创建时持久化 flow_kind、run_policy、RunHistory、scope、human gate snapshot、reservation、policy diagnostics 与 provider-start ledger。RunHistory SHALL 是 `#[serde(default, deny_unknown_fields)]`，以 Default 恢复历史 JSON 缺任意 history 字段为 0/空集，同时拒绝未知字段；FindingFingerprint SHALL transparent serde 为经校验的 64 位小写 hex。flow_kind 的旧默认值 SHALL 为 legacy，run_policy 的旧默认值 SHALL 为 interactive。

`CreateWorkspaceSessionInput` SHALL 以 `Option<WorkItemPlanSessionOptions>` 承载 workitem 专属配置；仅 WorkItemPlan 可提供 Some，其他 workspace 类型 SHALL 显式拒绝。运行中变更策略或 flow_kind SHALL 返回 protocol error。所有 delta、reservation、next state/status、scope、gate、diagnostic 和 provider ledger SHALL 通过 expected-revision CAS 原子提交；CAS 冲突 SHALL 重读重评。

`WorkspaceSessionStatus` SHALL 有独立 durable `stopped_needs_human` 与 `failed` 状态。SessionState SHALL 兼容透传 `session_status`、flow_kind、run_policy、run_history、review_invocation_scope、human_gate_snapshot、policy_diagnostics、provider_start_ledger；这 SHALL NOT 要求改变人工门产品 UI。

#### Scenario: 重启后路径不漂移

- **WHEN** 后端重启或全局 rollout 开关变化后，旧 session 重连
- **THEN** session 按持久化 flow_kind、run_policy 和 durable state 继续，不受全局开关影响

#### Scenario: 运行中变更策略被拒

- **WHEN** 运行进行中收到变更 run_policy 或 flow_kind 的请求
- **THEN** 系统返回 protocol error，运行状态不变

### Requirement: 人工门快照、恢复与 driver 协议（REQ-TOP-06）

待决人工门 SHALL 与 session record 同一原子写入，包含 findings、repeated fingerprints、trigger、`attempts_used = repairs_used + manual_repairs_used`、`manual_repairs_remaining = max_manual_repairs.saturating_sub(manual_repairs_used)` 与 resumable。resumable 仅适用于无 fatal/persistence diagnostic 的 auto stopped session；AwaitingHuman 由原 interactive session 继续，completed/failed 无 gate。`max_manual_repairs=0` 或耗尽时返修反馈 SHALL 被拒绝，门只留批准/终止。

stopped_needs_human 显式接管 SHALL 在后端新建 interactive session 并记录关联事件；原 session 的状态、history、events 不可篡改。HTTP 接管入口已于阶段 1.5 落地（`POST /api/workspace-sessions/{id}/takeover`）；对话流式 UI 入口仍归阶段 3。恢复与 driver 验证 provider 启动次数 SHALL 只从 provider_start_ledger 已启动 idempotency key 去重值读取；不得从 WebSocket event、stage 或 revision 猜测。

#### Scenario: 重连后人工门恢复

- **WHEN** 运行处于 awaiting_human 时客户端断线重连
- **THEN** 重连后的 SessionState 完整包含 durable human gate，前端恢复既有呈现，不重复创建、不丢失

#### Scenario: stopped 运行接管不改写历史

- **WHEN** 用户对 stopped_needs_human 运行执行显式接管
- **THEN** 系统以 interactive 策略创建新运行并记录关联事件；原运行保持 stopped_needs_human，原事件与计数保持不变

## 验收指标

- codex + pi 各 1 案例（naruto 单仓语料）在 auto_if_valid 下到达明确终态，无死循环、无硬超时；
- 每个 review cycle 内 `initial_count ≤ 1`、`verification_count ≤ 1`、`repairs_used ≤ cycle` 预算；session 级总计数仅从 `$.run_history` durable 字段读取并展示，不作为门控；
- 14 条 fixture 分类结果与预期一致：rep2/3/4 的 9 个真实 finding、rep1 round-1 的 2 个真实 pass/suggestion Advisory finding，以及 3 个带人工标注 class_hint 的 Repairable 变体；
- 恢复覆盖 awaiting_human、stopped takeover、completed/failed 重放、generate/repair 中断，启动次数从 ledger 读取；
- legacy 路径既有 workitem 测试全绿，旧 session 正常加载。
