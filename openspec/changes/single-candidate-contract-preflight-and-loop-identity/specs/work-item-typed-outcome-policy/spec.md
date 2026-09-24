# work-item-typed-outcome-policy Delta

## MODIFIED Requirements

### Requirement: review 两阶段与 scope 契约（REQ-TOP-04）

系统 SHALL 以**被审产物为锚**执行两轮 review 预算：一个 review cycle 为同一被审产物（outline 节点 / 某 draft 的 outline_id / 某 batch_id / **单候选流的 `sc:candidate:<source_revision_hash>`**）从首次 review 到 pass 或终态；初评最多一次、复评最多一次均为 **cycle 级**预算。一个 session 合法包含多个 cycle，cycle 间互不消耗预算；session 级 review 总计数仅用于指标与诊断，不做门控。仅当 cycle 内初评或机械校验产生可修复问题且聚合自动返修预算未消费时，允许一次聚合返修及最多一次复评。复评 SHALL 仅验证原 finding 指纹是否重现，并重跑同一 invocation 的机械校验；SHALL NOT 做开放式新审查、changed-path 归因或对 finding 自由文本作路径推断。复评后未解决 finding SHALL NOT 自动返修。单候选流的自动返修后复评 SHALL 以 Verification scope 执行（不得每轮以 Initial 重新开放）。

系统 SHALL 持久化 repairs_used、manual_repairs_used、initial_review_count、verification_review_count，以及按 cycle 的 `review_cycles: Map<cycle_key, {initial_count, verification_count}>`（cycle_key = 被审产物标识；单候选流为 candidate 身份，SHALL NOT 使用 reviewer 节点 id 或每轮变化的值）。scope SHALL 使用 `Initial { initial_revision_id, scope_digest } | Verification { original_fingerprints, repaired_revision_id, mechanical_report_ref, scope_digest }`；初评 SHALL NOT 用空字符串承载不适用复评字段。scope digest SHALL 使用版本化 canonical 字节编码、字段排序、BTreeSet 稳定序、SHA-256 和 `review_scope_v1:<64 lowercase hex>` 前缀。digest 不符、机械报告缺失或不符等**结构性** scope 违例 SHALL 为 Fatal(ProtocolViolation)；复评 finding 指纹不在原始集合时 SHALL 产出 `HumanRequired(VerificationNewFindings)`，按终态矩阵进入 `awaiting_human` 或 `stopped_needs_human`，不得 Fatal 或自动返修。

Finding identity SHALL 使用**结构化 canonical key**：category 存在时以 `category + 结构化对象引用`（受限提取的 WI/CT/AC 等稳定 ID，按类型排序、剥数组下标与路径顺序差异、规范化 field 尾段）构造；机械 finding SHALL 直接以确定性投影（coverage/contract projection 的 consumer WI + contract CT + field + capability refs）构造。reviewer 自由文本 SHALL NOT 作为 identity 成分；无法受限提取稳定 ID 的字段 SHALL 标记为 unstable 身份，SHALL NOT 与明确 ID 的 finding 合并或误判重复——策略层消费 unstable 身份做重复裁决时 SHALL fail-safe 进入人工处理。不同 WI/CT edge 的 findings SHALL NOT collapse 为同一 identity；同一问题的不同措辞 SHALL 得到同一 identity。legacy schema（无 category）SHALL 保留既有 normalized fallback。

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
