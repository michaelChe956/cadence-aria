## MODIFIED Requirements

### Requirement: review 两阶段与 scope 契约（REQ-TOP-04）

系统 SHALL 以**被审产物为锚**执行两轮 review 预算：一个 review cycle 为同一被审产物（outline 节点 / 某 draft 的 outline_id / 某 batch_id / **单候选流的 `sc:candidate:<source_revision_hash>`**）从首次 review 到 pass 或终态

指纹算法代际 SHALL 由会话固定：同一 session 的 finding identity、original/seen 指纹集合与 review scope 摘要 MUST 使用同一代际算法；MUST NOT 在同一 session 中途切换代际，也 MUST NOT 把不同命名域的实体当作同一身份。

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

#### Scenario: 指纹代际按会话固定

- **WHEN** 一个 session 已经确定指纹算法代际
- **THEN** 该 session 的 finding identity、original/seen 指纹集合比较与 scope 摘要 MUST 全部使用该代际算法；MUST NOT 存在任何消费面静默使用其它代际或「最新默认」

#### Scenario: 旧代际历史按原配方比较

- **WHEN** 一个冻结在旧代际的 session 读取其历史 `seen_fingerprints` / `original_fingerprints` 并产生新 finding
- **THEN** 新 finding MUST 用旧代际同一身份配方计算并与历史集合比较，MUST NOT 因算法升级重算或替换历史指纹，MUST NOT 把老问题判成新 finding

#### Scenario: 新旧命名域不合并

- **WHEN** 同一计划中同时存在 `AC-001` 与 `WIC-001`
- **THEN** 两者 MUST 是不同实体身份，MUST NOT 因编号相同而互相映射、合并或判为重复

#### Scenario: scope 摘要配方按代际区分

- **WHEN** 一个旧代际 scope 被校验，一个新代际 scope 被构造
- **THEN** 旧代际 scope 的摘要配方 MUST 逐字不变并仍可校验通过；新代际 scope MUST 把代际版本与 original 集合绑定进摘要输入

#### Scenario: scope 版本与 session 代际不一致即失败关闭

- **WHEN** scope 携带的代际版本与当前 session 的代际不一致，或 scope 摘要与所声明代际不匹配
- **THEN** 该状态 SHALL 按既有结构性协议错误 `Fatal(ProtocolViolation)` 失败关闭，MUST NOT 重算摘要后假装同一 invocation，MUST NOT 降级为人工门或自动返修

### Requirement: 运行状态持久化、CAS 与兼容（REQ-TOP-05）

系统 SHALL 创建时持久化 flow_kind、run_policy、RunHistory、scope、human gate snapshot、reservation、policy diagnostics 与 provider-start ledger

`RunHistory` SHALL 持久化指纹算法代际：缺省代际为旧代际（兼容既有历史），新建受控会话 SHALL 显式写入新代际；接管与恢复 SHALL 沿用父 session 的代际。缺证据或未知代际 SHALL NOT 被当作新起点，MUST NOT 清空 history 集合或充值任何预算。

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

#### Scenario: 代际字段缺省为兼容值

- **WHEN** 读取一条不含指纹代际字段的既有历史记录
- **THEN** 该记录 SHALL 按旧代际解释，MUST NOT 被当作新代际，也 MUST NOT 因缺字段触发重算或状态损坏

#### Scenario: 新建受控会话显式写入新代际

- **WHEN** 平台新建一个受控单候选会话
- **THEN** 该会话的持久化历史 SHALL 显式记录新代际，MUST NOT 依赖「最新默认」隐式取值

#### Scenario: 接管与恢复沿用父代际

- **WHEN** 一个受控 auto session 被接管为 interactive 子会话，或原会话被恢复 / 重连 / 产出新候选
- **THEN** 子会话与恢复路径 SHALL 沿用父 session 的代际，MUST NOT 重置代际，MUST NOT 因换会话标识而获得新的预算或新的重复判重起点

#### Scenario: 未知代际按状态损坏拒收

- **WHEN** 读取到指纹代际字段存在但取值不被本版本识别的记录
- **THEN** 该记录 SHALL 按状态损坏拒收，MUST NOT 猜测为任一已知代际后继续消费，也 MUST NOT 静默丢弃该记录

#### Scenario: 缺证据不清 history 不充预算

- **WHEN** 系统在某轮评审中缺少可比对的原始指纹集合、机械报告或其他基线事实
- **THEN** 系统 SHALL 走既有等待或人工处理路径，MUST NOT 清空 seen/original 集合、MUST NOT 充值任何预算、MUST NOT 跳过重复闸门，MUST NOT 把缺证据当作新 finding 开启自动重试
