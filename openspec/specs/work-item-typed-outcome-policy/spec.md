# work-item-typed-outcome-policy Specification

## Purpose

为 workitem 段提供确定性运行控制：reviewer finding 经显式 parser adapter 变为机器可读分类，由中央策略层依据预算、指纹与服务端 review scope 裁决 typed outcome，使每次运行到达 `completed`、`awaiting_human`、`stopped_needs_human` 或 durable `failed`，消除死循环并支持断线恢复。

## Requirements

### Requirement: finding 机器可读分类与失败关闭（REQ-TOP-01）

系统 SHALL 使 reviewer finding 携带 `category`、`class_hint`、`contract_field`

#### Scenario: 缺失恢复与未知拒绝

- **WHEN** 解析缺少三字段的历史 finding 数据，或 finding 携带未知字段名时
- **THEN** 历史数据缺三字段 SHALL 恢复为 None，未知字段名 SHALL 被拒绝

#### Scenario: 显式预检转换类别字段

- **WHEN** 预检或转换 finding 的 category/class_hint 时
- **THEN** 系统 SHALL 通过 `parse_review_envelope(&serde_json::Value) -> Result<ParsedReviewEnvelope, ReviewStructuredOutputError>` 显式预检或转换 category/class_hint

#### Scenario: 禁止默认 serde 兜底

- **WHEN** 遇到未知字符串时
- **THEN** 不得以 `Option<Enum>` 的默认 serde 行为处理未知字符串

#### Scenario: 解析信封保留三字段

- **WHEN** parser 产出 ParsedReviewEnvelope 时
- **THEN** parser SHALL 保留 `ParsedReviewEnvelope { raw_verdict, normalized_gate, findings }`

#### Scenario: 分类使用原始 verdict

- **WHEN** 分类器为 finding 判定分类时
- **THEN** 分类 SHALL 使用 raw_verdict

#### Scenario: 路由使用策略 outcome

- **WHEN** routing 决定 finding 流转时
- **THEN** routing SHALL 使用策略 outcome

#### Scenario: 不改写原生人工结论

- **WHEN** 存在强 finding 且原生结论为 needs_human 时
- **THEN** parser SHALL NOT 因强 finding 将原生 needs_human 改写为 revise 后供分类器使用

#### Scenario: 未知类别唯一失败链

- **WHEN** finding 的 category 为枚举外未知值时
- **THEN** 未知 category 的唯一失败链路 SHALL 为 `UnknownFindingCategory(parser) → ClassificationError::UnknownCategory → FatalReason::UnknownCategory → unknown_finding_category diagnostic`

#### Scenario: 未知提示唯一失败链

- **WHEN** finding 的 class_hint 为枚举外未知值时
- **THEN** 未知 class_hint 对应为 `UnknownFindingClassHint → ClassificationError::UnknownClassHint → FatalReason::UnknownClassHint → unknown_class_hint`

#### Scenario: 字段类型错误映射

- **WHEN** finding 字段类型错误时
- **THEN** 字段类型错误 SHALL 到 `InvalidStructuredOutput`

#### Scenario: 解析错误持久失败

- **WHEN** 未知 category、未知 class_hint 或字段类型错误发生时
- **THEN** 这些错误均 SHALL durable failed，绝不降级 NeedsHuman/HumanRequired

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

系统 SHALL 提供仅消费已分类 finding、运行历史与预算的纯策略评估器，裁决 `valid`、`repairable`、`human_required`、`fatal` 四类 typed outcome

#### Scenario: 原始结论不驱动跳转

- **WHEN** 以原始 verdict 与 severity 驱动状态跳转时
- **THEN** 原始 verdict 与 severity SHALL NOT 直接驱动状态跳转

#### Scenario: 裁决优先级固定

- **WHEN** 命中多个裁决条件时
- **THEN** 优先级固定为：转换预算耗尽 → 重复指纹 → 返修需求 → human_required → valid

#### Scenario: 返修预算耗尽转人工

- **WHEN** 存在 repairable 且 max_repairs=0 或 repair budget 耗尽时
- **THEN** `max_repairs=0` 或 repair budget 耗尽遇 repairable SHALL 产生 HumanRequired

#### Scenario: 转换预算耗尽转致命

- **WHEN** max_transitions=0 或已达转换上限时
- **THEN** `max_transitions=0` 或已达上限 SHALL 产生 Fatal(TransitionBudgetExhausted)

#### Scenario: 人工门只留批准终止

- **WHEN** max_manual_repairs=0 的人工门收到人工操作时
- **THEN** `max_manual_repairs=0` 的人工门 SHALL 只留批准/终止

#### Scenario: 持久化计数异常判损坏

- **WHEN** 持久化计数超预算、溢出或不变量不一致时
- **THEN** 持久化计数超预算、溢出或不变量不一致 SHALL 产生 StateCorruption

#### Scenario: 转换计数提交后才增

- **WHEN** stage transition 成功 durable commit 后
- **THEN** transitions_used SHALL 仅在 stage transition 成功 durable commit 后增加

#### Scenario: 建议项不写指纹

- **WHEN** advisory finding 进入 seen fingerprints 写入时
- **THEN** advisory SHALL NOT 写入 seen fingerprints

#### Scenario: 重复指纹不自动再试

- **WHEN** 本轮非 advisory finding 的指纹已存在于运行历史
- **THEN** 裁决为 HumanRequired(RepeatedFingerprint)，交互模式进入 awaiting_human，auto 模式 stopped_needs_human；不触发自动返修

#### Scenario: 自动返修预算耗尽

- **WHEN** 自动返修次数已达上限或 max_repairs 为 0，且存在 mechanical error 或 repairable 需求
- **THEN** 裁决为 HumanRequired(RepairBudgetExhausted)，交互模式 awaiting_human，auto 模式 stopped_needs_human；不得 fatal

### Requirement: 终态矩阵与 durable reservation（REQ-TOP-03）

首次 aggregate repairable（机械错误与 review repairable 共用 `max_repairs=1`）SHALL 自动返修一次

#### Scenario: 人工需求分流两模式

- **WHEN** 出现复评未解决、重复指纹、repair budget 耗尽或原生 human_required 时
- **THEN** 复评未解决、重复指纹、repair budget 耗尽和原生 human_required SHALL 分流为 interactive awaiting_human 或 auto stopped_needs_human

#### Scenario: 安全不变量破坏致命

- **WHEN** 出现 transition budget 耗尽、状态损坏、未知协议、持久化失败或安全不变量破坏时
- **THEN** transition budget 耗尽、状态损坏、未知协议、持久化失败和安全不变量破坏 SHALL fatal

#### Scenario: 两种终态分别计数

- **WHEN** 运行以 fatal 或 stopped_needs_human 结束时
- **THEN** fatal 与 stopped_needs_human SHALL 分别计数和展示

#### Scenario: 返修预留持有登记

- **WHEN** 创建自动返修的 durable reservation 时
- **THEN** 自动返修的 durable reservation SHALL 持有 token、owner、provider_start_idempotency_key、状态和 commit 信息

#### Scenario: 预留先写再计数

- **WHEN** reservation 经 CAS 写入并记录 provider 启动时
- **THEN** CAS 先写 Reserved，成功记录 provider 启动后才一次性增加 repairs_used 并转 ProviderStarted

#### Scenario: 启动失败释放预留

- **WHEN** provider 启动失败时
- **THEN** 启动失败 SHALL 按 token 幂等转 Released 且 repairs_used 不增

#### Scenario: 恢复释放无账本预留

- **WHEN** 恢复时 reservation 处于 Reserved 且无启动账本时
- **THEN** 恢复时 Reserved 无启动账本 SHALL 释放

#### Scenario: 同键恢复不二次启动

- **WHEN** 恢复时 reservation 处于 ProviderStarted 或 Committed 时
- **THEN** ProviderStarted/Committed SHALL 使用同一 key 恢复，不得二次启动

#### Scenario: auto 模式遇人工需求不空转

- **WHEN** auto_if_valid 运行裁决出 human_required
- **THEN** 运行以 stopped_needs_human 结束，保存可恢复人工门快照与诊断，不挂起等待、不标 fatal

#### Scenario: 人工返修后同指纹重现回到同一门

- **WHEN** interactive 模式下人工反馈返修后同一 finding 指纹再次出现
- **THEN** 运行回到 awaiting_human 并展示指纹历史，不自动再试、不转 fatal

### Requirement: review 两阶段与 scope 契约（REQ-TOP-04）

系统 SHALL 以**被审产物为锚**执行两轮 review 预算：一个 review cycle 为同一被审产物（outline 节点 / 某 draft 的 outline_id / 某 batch_id / **单候选流的 `sc:candidate:<source_revision_hash>`**）从首次 review 到 pass 或终态

#### Scenario: 初评复评按周期计次

- **WHEN** 同一被审产物的 review cycle 执行初评与复评时
- **THEN** 初评最多一次、复评最多一次均为 **cycle 级**预算

#### Scenario: 会话可含多个周期

- **WHEN** 一个 session 合法包含多个 cycle 时
- **THEN** 一个 session 合法包含多个 cycle，cycle 间互不消耗预算

#### Scenario: 会话计数仅作指标

- **WHEN** 读取 session 级 review 总计数时
- **THEN** session 级 review 总计数仅用于指标与诊断，不做门控

#### Scenario: 可修复才放行返修复评

- **WHEN** cycle 内初评或机械校验产生可修复问题且聚合自动返修预算未消费时
- **THEN** 仅当 cycle 内初评或机械校验产生可修复问题且聚合自动返修预算未消费时，允许一次聚合返修及最多一次复评

#### Scenario: 复评只验指纹重跑校验

- **WHEN** 执行复评时
- **THEN** 复评 SHALL 仅验证原 finding 指纹是否重现，并重跑同一 invocation 的机械校验

#### Scenario: 复评不做开放式审查

- **WHEN** 复评进行中时
- **THEN** SHALL NOT 做开放式新审查、changed-path 归因或对 finding 自由文本作路径推断

#### Scenario: 复评未解决不自动返修

- **WHEN** 复评后 finding 仍未解决时
- **THEN** 复评后未解决 finding SHALL NOT 自动返修

#### Scenario: 单候选复评用验证域

- **WHEN** 单候选流的自动返修后触发复评时
- **THEN** 单候选流的自动返修后复评 SHALL 以 Verification scope 执行（不得每轮以 Initial 重新开放）

#### Scenario: 持久化周期级计数

- **WHEN** 持久化 review 计数与按 cycle 的计数时
- **THEN** 系统 SHALL 持久化 repairs_used、manual_repairs_used、initial_review_count、verification_review_count，以及按 cycle 的 `review_cycles: Map<cycle_key, {initial_count, verification_count}>`（cycle_key = 被审产物标识；单候选流为 candidate 身份，SHALL NOT 使用 reviewer 节点 id 或每轮变化的值）

#### Scenario: 范围使用两种形态

- **WHEN** 构造 review scope 时
- **THEN** scope SHALL 使用 `Initial { initial_revision_id, scope_digest } | Verification { original_fingerprints, repaired_revision_id, mechanical_report_ref, scope_digest }`

#### Scenario: 初评不用空串占位

- **WHEN** 构造初评 scope 时
- **THEN** 初评 SHALL NOT 用空字符串承载不适用复评字段

#### Scenario: 范围摘要版本化编码

- **WHEN** 计算 scope digest 时
- **THEN** scope digest SHALL 使用版本化 canonical 字节编码、字段排序、BTreeSet 稳定序、SHA-256 和 `review_scope_v1:<64 lowercase hex>` 前缀

#### Scenario: 结构违例判致命

- **WHEN** digest 不符或机械报告缺失/不符时
- **THEN** digest 不符、机械报告缺失或不符等**结构性** scope 违例 SHALL 为 Fatal(ProtocolViolation)

#### Scenario: 复评新指纹转人工

- **WHEN** 复评 finding 指纹不在原始集合时
- **THEN** 复评 finding 指纹不在原始集合时 SHALL 产出 `HumanRequired(VerificationNewFindings)`，按终态矩阵进入 `awaiting_human` 或 `stopped_needs_human`，不得 Fatal 或自动返修

#### Scenario: 身份用结构化规范键

- **WHEN** category 存在时构造 finding identity 时
- **THEN** Finding identity SHALL 使用**结构化 canonical key**：category 存在时以 `category + 结构化对象引用`（受限提取的 WI/CT/AC 等稳定 ID，按类型排序、剥数组下标与路径顺序差异、规范化 field 尾段）构造

#### Scenario: 机械发现用确定性投影

- **WHEN** 构造机械 finding 的 identity 时
- **THEN** 机械 finding SHALL 直接以确定性投影（coverage/contract projection 的 consumer WI + contract CT + field + capability refs）构造

#### Scenario: 自由文本不入身份

- **WHEN** 构造 finding identity 时
- **THEN** reviewer 自由文本 SHALL NOT 作为 identity 成分

#### Scenario: 不稳定身份安全兜底

- **WHEN** 字段无法受限提取稳定 ID 时
- **THEN** 无法受限提取稳定 ID 的字段 SHALL 标记为 unstable 身份，SHALL NOT 与明确 ID 的 finding 合并或误判重复——策略层消费 unstable 身份做重复裁决时 SHALL fail-safe 进入人工处理

#### Scenario: 不同边不撞身份

- **WHEN** 两个 finding 位于不同 WI/CT edge 时
- **THEN** 不同 WI/CT edge 的 findings SHALL NOT collapse 为同一 identity

#### Scenario: 同题异辞同身份

- **WHEN** 同一问题被以不同措辞报告时
- **THEN** 同一问题的不同措辞 SHALL 得到同一 identity

#### Scenario: 旧模式保留归一化兜底

- **WHEN** 处理无 category 的 legacy schema 时
- **THEN** legacy schema（无 category）SHALL 保留既有 normalized fallback

#### Scenario: 复评新 finding 降级人工处理

- **WHEN** 复评 finding 的指纹不在原始 invocation scope 集合中，或 reviewer 仅改变同类同字段 finding 的措辞
- **THEN** 不将该 finding 视为结构性协议 Fatal；前者按 `VerificationNewFindings` 进入 awaiting_human 或 stopped_needs_human，不触发第二次自动返修，后者通过结构化 fingerprint 保持原身份

#### Scenario: 复评结构性 scope 违例仍失败关闭

- **WHEN** scope digest 不符，或 invocation 要求的机械校验报告缺失/不符
- **THEN** 按 `Fatal(ProtocolViolation)` durable failed，绝不进入人工门

#### Scenario: 同题异措辞稳定判重

- **WHEN** 同一 capability 缺口在两轮 review 中被 reviewer 以不同自由文本措辞报告（对象 ID 相同）
- **THEN** 两轮 finding 得到同一 canonical identity，第二轮命中 seen fingerprints 触发 RepeatedFingerprint 路径，不产生新身份

#### Scenario: 异题不撞指纹

- **WHEN** 两个不同 WI/CT edge 上的同类 finding（自由文本可能相似）
- **THEN** canonical identity 不同，不 collapse、不误判重复

#### Scenario: unstable 身份 fail-safe

- **WHEN** reviewer finding 的 contract_field 无法受限提取任何稳定 ID
- **THEN** 该 finding 标记 unstable 身份；策略层不得据其做 RepeatedFingerprint 自动裁决，SHALL fail-safe 进入人工处理并展示诊断

#### Scenario: 单候选返修后进入 Verification

- **WHEN** 单候选流的聚合自动返修完成并触发复评
- **THEN** 复评以 `Verification { original_fingerprints, ... }` scope 执行且计入同 cycle 的 verification_count；SHALL NOT 以 Initial 重新开放初审预算

#### Scenario: 单候选 cycle key 锚定候选

- **WHEN** 同一 source_revision_hash 的候选经历多个 reviewer 节点（修订轮次）
- **THEN** review_cycles 使用同一 `sc:candidate:<source_revision_hash>` key，预算按 cycle 累计不因节点变化清零；新 source revision 才开启新 cycle

### Requirement: 运行状态持久化、CAS 与兼容（REQ-TOP-05）

系统 SHALL 创建时持久化 flow_kind、run_policy、RunHistory、scope、human gate snapshot、reservation、policy diagnostics 与 provider-start ledger

#### Scenario: 历史缺省恢复拒未知

- **WHEN** 读取缺任意 history 字段或含未知字段的历史 JSON 时
- **THEN** RunHistory SHALL 是 `#[serde(default, deny_unknown_fields)]`，以 Default 恢复历史 JSON 缺任意 history 字段为 0/空集，同时拒绝未知字段

#### Scenario: 指纹序列化为校验十六进制

- **WHEN** 序列化或反序列化 FindingFingerprint 时
- **THEN** FindingFingerprint SHALL transparent serde 为经校验的 64 位小写 hex

#### Scenario: 旧默认值为兼容值

- **WHEN** 读取缺 flow_kind 或 run_policy 的旧记录时
- **THEN** flow_kind 的旧默认值 SHALL 为 legacy，run_policy 的旧默认值 SHALL 为 interactive

#### Scenario: 会话输入承载专属配置

- **WHEN** 构造 CreateWorkspaceSessionInput 时
- **THEN** `CreateWorkspaceSessionInput` SHALL 以 `Option<WorkItemPlanSessionOptions>` 承载 workitem 专属配置

#### Scenario: 仅工作项可给专属配置

- **WHEN** 非 WorkItemPlan 的 workspace 类型提供 Some 时
- **THEN** 仅 WorkItemPlan 可提供 Some，其他 workspace 类型 SHALL 显式拒绝

#### Scenario: 运行中变更策略被拒

- **WHEN** 运行中收到变更策略或 flow_kind 的请求时
- **THEN** 运行中变更策略或 flow_kind SHALL 返回 protocol error

#### Scenario: 全部变更走 CAS 提交

- **WHEN** 提交 delta、reservation、状态、scope、gate、diagnostic 或 provider ledger 时
- **THEN** 所有 delta、reservation、next state/status、scope、gate、diagnostic 和 provider ledger SHALL 通过 expected-revision CAS 原子提交

#### Scenario: 冲突后重读重评

- **WHEN** expected-revision CAS 提交冲突时
- **THEN** CAS 冲突 SHALL 重读重评

#### Scenario: 状态含两种独立终态

- **WHEN** 读取 WorkspaceSessionStatus 时
- **THEN** `WorkspaceSessionStatus` SHALL 有独立 durable `stopped_needs_human` 与 `failed` 状态

#### Scenario: 状态兼容透传字段

- **WHEN** 组装 SessionState 时
- **THEN** SessionState SHALL 兼容透传 `session_status`、flow_kind、run_policy、run_history、review_invocation_scope、human_gate_snapshot、policy_diagnostics、provider_start_ledger

#### Scenario: 兼容不改人工门界面

- **WHEN** 以兼容透传方式暴露 SessionState 字段时
- **THEN** 这 SHALL NOT 要求改变人工门产品 UI

#### Scenario: 重启后路径不漂移

- **WHEN** 后端重启或全局 rollout 开关变化后，旧 session 重连
- **THEN** session 按持久化 flow_kind、run_policy 和 durable state 继续，不受全局开关影响

#### Scenario: 运行中变更策略被拒

- **WHEN** 运行进行中收到变更 run_policy 或 flow_kind 的请求
- **THEN** 系统返回 protocol error，运行状态不变

### Requirement: 人工门快照、恢复与 driver 协议（REQ-TOP-06）

待决人工门 SHALL 与 session record 同一原子写入，包含 findings、repeated fingerprints、trigger、`attempts_used = repairs_used + manual_repairs_used`、`manual_repairs_remaining = max_manual_repairs.saturating_sub(manual_repairs_used)` 与 resumable

#### Scenario: 可恢复仅限自动停等

- **WHEN** auto stopped session 判定 resumable 时
- **THEN** resumable 仅适用于无 fatal/persistence diagnostic 的 auto stopped session

#### Scenario: 人工门由原会话续跑

- **WHEN** 处于 AwaitingHuman 或 completed/failed 终态时
- **THEN** AwaitingHuman 由原 interactive session 继续，completed/failed 无 gate

#### Scenario: 人工预算耗尽只批准

- **WHEN** max_manual_repairs=0 或耗尽时提交返修反馈
- **THEN** `max_manual_repairs=0` 或耗尽时返修反馈 SHALL 被拒绝，门只留批准/终止

#### Scenario: 接管新建交互会话

- **WHEN** 对 stopped_needs_human 执行显式接管时
- **THEN** stopped_needs_human 显式接管 SHALL 在后端新建 interactive session 并记录关联事件

#### Scenario: 原会话事实不可改

- **WHEN** 接管后查看原 session 的状态、history、events 时
- **THEN** 原 session 的状态、history、events 不可篡改

#### Scenario: 接管入口已落地后端

- **WHEN** 使用 HTTP 接管入口接管时
- **THEN** HTTP 接管入口已于阶段 1.5 落地（`POST /api/workspace-sessions/{id}/takeover`）

#### Scenario: 流式界面仍属阶段三

- **WHEN** 从对话流式 UI 入口接管时
- **THEN** 对话流式 UI 入口仍归阶段 3

#### Scenario: 启动次数只读启动账本

- **WHEN** 恢复与 driver 验证 provider 启动次数时
- **THEN** 恢复与 driver 验证 provider 启动次数 SHALL 只从 provider_start_ledger 已启动 idempotency key 去重值读取

#### Scenario: 启动次数不靠事件猜

- **WHEN** 判定 provider 启动次数时
- **THEN** 不得从 WebSocket event、stage 或 revision 猜测

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
