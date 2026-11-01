# Design: mechanicalize-plan-compile-repair

> 完整技术论证（含 18 条取证事实、prompt 实测、度量、分歧清单）见
> `cadence/designs/2026-10-31_技术方案_Plan生成不可控性治理_v1.0.md`。本文只保留契约与架构边界。

## 决策记录

1. **契约四层**：L0 自修 / L1 投影补全 / L2 精确缺口 / L3 业务取舍。判据四问：能否平台判定 → 是否唯一解 → 修复是否只做删/去重/改名/追加 → 新内容能否从既有结构投影。
2. **判据只认 typed**，绝不认 message 措辞（沿用 `REQ-SG-05` 已废字符串旁路的教训，`web/.../run/single_candidate.rs:96-100`）。
3. **lowering typed 走独立枚举**：`CompilerDiagnostic` 无 serde（`types.rs:61`），加 `lowering_fault` 字段零 wire 污染；`PlanningFailureKind` 是 wire 枚举 + `classify_failure_set` 全集，改它污染面大（F16）。
4. **P0 不在人工门**：`is_auto_rewrite_code`（`compile_failure_surface.rs:409`）只作用于人工门，而现场 #2 死在更早的 handler（`run/single_candidate.rs:1301-1313`）→ P0 必须落 compiler/handler。
5. **不删边消环**：Final Compile 的 `depends_on` 由 `input_contracts` 推出（`compile_support.rs:1015-1037`），现场 #1 是真实数据流逆仓序；删边 = `REQ-SG-02A`「禁止假删边」。环只做结构化教学 + 重复则升级业务取舍。
6. **不接受「平台改 source 的 key」**：改走 parse 期等价解析（与 `normalize_delivery_before_compile` 同族），source 原样，产审计。
7. **不自动补 AC↔check**：会制造假验证证据。

## 架构与改动面

```
author 输出
  │
  ├─ normalize_delivery_before_compile（既有：heading/EARS/前言）
  ├─ 【新】lower 等价解析 + LoweringFault 构造（lower.rs:1121 唯一工厂，24 调用点）
  │      └─ 唯一映射 → 采用 + equivalence_canonicalization 审计
  │      └─ 不唯一   → typed fault → 恰一次结构化教学重驱（资格=typed 完备，非码白名单）
  │      └─ 无 fault → 维持既有终态
  ├─ contract_autorepair::converge_work_item_plan_source（既有 4 类，不扩收编面）
  ├─ Stage1 结构门 assess_single_candidate_structure（plan_projection.rs:1039）
  ├─ reviewer / 人工门
  └─ approve → Final Compile（无跳过分支，conversational_gate.rs:761-830）
         └─ 环：typed witness（含 sources）→ 呈现层用人话，判据只用 typed
```

| 组件 | 文件 | 改动要点 |
|---|---|---|
| lowering fault | `src/product/work_item_plan_compiler/types.rs:61-80`、`lower.rs:1121`（`diagnostic()` 工厂）、`lower.rs:443-480`（`lower_plan_intent`） | 加 `lowering_fault` 字段；构造点赋 fault；分支 key 等价解析 |
| 教学资格 | `src/web/workspace_ws_handler/run/single_candidate.rs:84-100`、`:1290-1300` | 资格判据由 `TEACHABLE_PARSE_FAILURE_CODES` 改为 typed 完备；重驱 prompt 渲染 typed payload |
| 环判定来源 | `src/product/workspace_engine/draft_batch/compile_support.rs:783-826`、`cycle_diagnostic.rs` | 判据改用 typed witness；不删边；人话段降为呈现层 |
| Final Compile 失败形态 | `src/product/workspace_engine/compile/single_candidate.rs:770`、`compile.rs:183` | 环失败透传为 `PlanningCompileFailureKind::Structure`，不再只包 `operational(final_compile_prepare_failed)` |
| 一次答完 | `src/product/workspace_engine/compile_failure_surface.rs:143-144`、`:370-410` | 填 `owner_question` / `owner_options`（由 typed 事实生成） |
| 可观测性 | `src/product/lifecycle_store/human_dispositions.rs:234,249` 同目录 | 新增追加写 `candidate-attempts.jsonl` |
| durable 模型 | `src/product/models/human_gate.rs:247` | 新字段 `default + skip_serializing_if`（`deny_unknown_fields`） |

## 与 `REQ-SG-02A` 的关系：遗留裁决 O1（待 owner 裁决，本 change 不裁）

> 本 change 的 `specs/work-item-plan-compile-repair/spec.md` 头注释引用本节为「遗留裁决 O1」。

`openspec/changes/fix-error-experience-structural-governance/specs/workspace-structural-governance/spec.md` 的 `REQ-SG-02A`（该 change 0/36 未实施）规定：所有内容修复（含「原范围内 autorepair 补 capability/删 key/handoff/ref」）**必须**经原授权 author，**MUST NOT** 系统直 patch 已持久或已评审的 Markdown/IR；仅「交付前前言修剪、固定中文 heading→英文、EARS 空白规范化」可保留。

- 现有 `contract_autorepair`（`contract_autorepair.rs:65`）与之**正面撞线**。
- 本 change 立场：**不新增直 patch**；新增修复一律「平台判定 → 结构化指令 → 原 author 执行」；parse 期等价解析不改 source，不触线。
- 遗留裁决（需 owner）：
  - **A（建议）** 修订 `REQ-SG-02A`，把「持久化前 + 纯等价规范化 + typed 判据 + 可审计」列为合法例外 → 保留 autorepair 现有能力，成本最低。
  - **B** 把 `contract_autorepair` 整体迁成「指令 → author」→ 完全合规，但每次修复多一轮 provider；现有证据表明弱模型不逐字应用补丁行（`contract_autorepair.rs:4-8` 模块头原话：「模型返修重写整份计划而不逐字应用补丁行，同族缺口跨指纹轮转」），收敛率会下降。

## 关键风险

| 风险 | 缓解 |
|---|---|
| L0 越界 → 静默抹掉真实业务依赖（「平台替模型做决定」） | 三条红线进 `REQ-CRP-01` + 负向测试（删边必须失败、映射不唯一必须不修） |
| `REQ-WSC-09` 改动面外溢 | 先改测试再改实现；改动只限资格判据，**不新增 attempt**；总重驱次数 ≤1 加断言 |
| `compile_support.rs` 2309/2310 行上限 | 先拆文件（先例 `cycle_diagnostic.rs`） |
| 图模型改错方向 | 先做奇偶性 fixture 证伪，不先改边模型 |
| 前端/durable 兼容 | `#[serde(default)]` + `skip_serializing_if`；稳定码保留一版 |

## 被否决的备选

| 备选 | 否决理由 |
|---|---|
| 把 lowering 码加进 `is_auto_rewrite_code` 扩白名单 | 对现场 #2 无效：它死在 handler，到不了人工门（`run/single_candidate.rs:1301-1313`） |
| 平台自动删「反向声明依赖边」消环 | Final Compile 的边由 `input_contracts` 推出，声明依赖不建边；删边=假删边（`REQ-SG-02A`）。该场景近乎空集 |
| 平台自动补 `reviewer_check_refs` | 造出假验证证据，后续 coder/reviewer 会当真 |
| 仅在 prompt 里再强调规则 | 已证伪：`prompts/sc_author_teaching.rs:41` 逐字写了仍被违反 |
| 直接改 source 的 key 名 | 触 `REQ-SG-02A`「不得原位修改 immutable 对象」；改走 parse 期等价解析 |
