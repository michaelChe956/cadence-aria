# 技术方案：Work Item Draft Prompt 固定开销瘦身与上限重论证

- 日期：2026-07-25
- 版本：v1.0
- 关联 Change：`improve-work-item-draft-generation-reliability`（修订其"不放宽 11,000-byte 上限"约束）
- 关联 Plan：`cadence/plans/2026-07-25_计划文档_基线修复_WorkItemDraft串行上下文预算_v1.0.md`（将被修订/替换）

## 背景与根因

### 现象

Work Item Plan `workspace_session_0003`：draft_001 审核通过并被接受后，系统为第二个 outline（`outline_compact_duration_unit_tests`，直接依赖 draft_001）构建 Draft prompt 时 fail-closed 报错：

```text
work item draft prompt exceeds the 11000-byte provider-context limit
```

Draft run 节点（`timeline_node_012`）卡在 `active/pending`，Provider 从未启动，且失败未落状态（附带问题，见 §5）。

### 根因（量化）

以 session_0003 真实数据按当前代码逐段复算 prompt（UTF-8 字节）：

| 组成 | 字节 | 性质 |
|---|---|---|
| 静态模板（canonical_field_contract 2,246 / hard_rules 1,726 / self_check+projection+registration 950 / canonical_projection 540 / 模板头+output 675） | 6,137 | 固定开销 |
| runtime_contract（routing_reference 1,360 + 阶段/gate/openspec/superpowers 契约 ~1,346） | ~2,706 | 固定开销 |
| current_work_item_outline JSON（真实中文 outline） | 1,678 | 变量 |
| 直接依赖投影（已实施的最小化 output/handoff 合同投影） | 1,101 | 变量 |
| confirmed_plan_trace + 命令目录占位等 | ~430 | 变量 |
| **合计** | **~11,711** | **超限 711** |

关键事实：

1. 固定开销 8,843 B 占预算 80%，变量预算仅剩 ~2,157 B，真实中文数据（~3,209 B）必然超限。
2. 上限按 UTF-8 字节计，1 汉字 = 3 字节，11,000 B 仅约 3,700 汉字。
3. 历史测量（draft prompt 模板字面量）：三投影提交 `bd8799eb` 仅 +472 B；真正的增长来自 `d6cb0522`（+3,244 B，canonical_field_contract 等）与 `3066a7a7`（+~2,700 B runtime_contract）。**11,000 上限与最大一批固定开销在同一提交引入，按小号英文 fixture 校准，从未用真实中文数据验证。**
4. 已实施的"直接依赖最小化投影"修复方向正确，但只压缩了变量，未审固定开销；回归 fixture 过小，无法拦截真实规模数据。

## 目标

1. 真实规模的串行 Draft prompt（含已接受直接依赖）稳定低于质量预算。
2. 上限值有明确论证：硬兜底防病态序列化回归，质量预算防输出质量下降。
3. Prompt 瘦身不降低契约约束力：被删减的只能是重复表述，字段白名单、集合关系、可信命令纪律的语义必须完整保留。

## 方案

### 1. 固定开销瘦身（估计省 ~2,300 B）

| 动作 | 目标段落 | 省字节 | 说明 |
|---|---|---|---|
| A | `[canonical_field_contract]` | ~950 | 引入简写记号（如 `str+` = non-empty string、`[T]` = array of T），消除 9 次 `non-empty string`、8 次 `array of` 重复；字段集合与约束语义不变 |
| B | `[hard_rules]` | ~800 | 删除 3 条与 field_contract/self_check 完全重复的条款（verification_plan 逐字段复制、canonical_contract 字段白名单、verification_plan 只能含 checks）；输出唯一性条款并入 `[output]` |
| C | runtime_contract superpowers 段 | ~400 | TDD/验证闭环要求与 hard_rules 重复，改为指引性引用 |
| D | `[self_check]`+`[projection]`+`[registration]` | ~350 | registration 单句并入 projection；self_check 保持对集合关系的引用式表述 |
| E | `[canonical_projection]` | ~190 | 四条映射压缩为两条 |
| — | routing_reference（1,360 B） | 0 | **不动**：Cadence 规则读取门禁强制要求全文引用 |

缩减前后（session_0003 真实数据）：

| | 固定开销 | 变量 | 总 prompt | vs 12,000 质量预算 |
|---|---|---|---|---|
| 缩减前 | 8,843 | 3,209 | ~11,711 | 余 ~289（且超旧上限 711） |
| 缩减后 | ~6,500 | 3,209 | ~9,700 | 余 ~2,300 |

### 2. 上限重论证（双层）

调查事实：prompt 通过 stdin JSON 消息发送给 Claude Code / Codex，不经 CLI 参数，无 OS ARG_MAX 约束；真实物理边界是模型上下文窗口（200k tokens 级 ≈ 60 万字节中文）。

- **第一层 · 硬兜底（fail-closed guard）**：`WORK_ITEM_DRAFT_PROMPT_MAX_BYTES` 从 11,000 调整为 **65,536（64KB）**。用途在代码注释中写明：拦截病态序列化回归（如整条 `WorkItemDraftRecord` 被注入 prompt 的 bug 类别），不再兼任质量预算。错误码 `work_item_draft_prompt_too_large` 与 fail-closed 语义保持不变。
- **第二层 · 质量预算（确定性测试）**：新增/修订预算测试，使用**真实规模中文 fixture**（中文 outline + 真实 handoff 合同），断言 prompt < **12,000 B**。质量目标依靠模板瘦身 + 真实验证维持，不再依赖 fail-closed 报错。

### 3. OpenSpec 修订

- 修订 `improve-work-item-draft-generation-reliability` 的 design.md：将"不放宽 11,000-byte 上限"约束改写为双层模型（64KB 硬兜底 + 12KB 质量预算），记录本论证依据。
- 修订/替换 Plan《基线修复_WorkItemDraft串行上下文预算》：保留已实施的依赖投影，新增瘦身动作 A–E、上限调整与真实规模 fixture。

## 范围约束

- 仅修改 `src/product/work_item_split_engine/prompts.rs`、`parse.rs`（上限常量与注释）及其确定性测试；routing_reference 全文保留。
- 不新增第三方依赖、评估模块、CLI、CI、Hook、Provider 调用或持久化语料。
- 不放宽 Parser、`WorkItemDraftLocalValidator`、接受门禁或 fail-closed 语义。

## 附带修复（一并实施）

`build_work_item_draft_streaming_input` 失败时，Draft run / timeline 节点未落 failed，卡在 `active/pending`（session_0003 的 `timeline_node_012` 即此状态）。需将 prompt 构建失败落为失败节点并记录错误，保证 UI 可恢复。

## 验证策略

1. **确定性验证**：真实规模中文 fixture 预算测试（< 12,000 B）、既有 draft prompt 契约测试全量回归、`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`。
2. **真实 Provider 验证（需人工显式授权）**：按 `cadence/project-rules/work-item-draft-prompt-validation.md`，交付前提醒操作者授权 Case A、Case B 各 10 次真实 Claude Code 验证；未授权不调用 Provider，不勾选相关 OpenSpec tasks。

## 风险与权衡

- [瘦身破坏契约约束力] → 只删重复表述；契约测试逐条断言关键短语仍在；Case A/B 验证首次输出有效率不下降。
- [64KB 硬兜底过松，质量失控] → 质量预算测试（12KB、真实 fixture）承担质量门禁；硬兜底只防病态回归。
- [routing_reference 被视为可压缩] → 明确不动，避免破坏 Cadence 规则读取门禁。
