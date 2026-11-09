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
8. **O1 已裁决（2026-11-08 owner 裁决 §2.2/§2.3）**：不改 `REQ-SG-02A`，内容 autorepair 迁原授权 author；平台仅保留主规格明文允许的交付前规范化与不改 source 的等价解析（详见下文「与 `REQ-SG-02A` 的关系」节）。
9. **WSC-09 资格清单扩充（裁决 §1.4/§1.5）**：四类 typed IR 诊断与确定性结构拒收进入唯一资格清单；本 change 是 MODIFIED WSC-09 唯一持有者（详见下文专节）。

## 架构与改动面

```
author 输出
  │
  ├─ normalize_delivery_before_compile（既有：heading/EARS/前言）
  ├─ 【新】lower 等价解析 + LoweringFault 构造（lower.rs:1121 唯一工厂，24 调用点）
  │      └─ 唯一映射 → 采用 + equivalence_canonicalization 审计
  │      └─ 不唯一   → typed fault → 恰一次结构化教学重驱（资格=typed 完备，非码白名单）
  │      └─ 无 fault → 维持既有终态
  ├─ contract_autorepair::converge_work_item_plan_source（既有 4 类 → O1 裁决后收窄为 typed 缺口/机器指令 producer，移除本地 source mutation）
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

## 与 `REQ-SG-02A` 的关系：O1 已由 owner 裁决（裁决 §2.2/§2.3）

> 本 change 的 `specs/work-item-plan-compile-repair/spec.md` 头注释引用本节。

`REQ-SG-02A`（主规格 `openspec/specs/workspace-structural-governance/spec.md`）规定：所有内容修复（含「原范围内 autorepair 补 capability/删 key/handoff/ref」）**必须**经原授权 author，**MUST NOT** 系统直 patch 已持久或已评审的 Markdown/IR；仅「交付前前言修剪、固定中文 heading→英文、EARS 空白规范化」可保留。

**裁决结论（2026-11-08 owner 裁决 §2.2/§2.3）：不改 `REQ-SG-02A`，内容 autorepair 迁原授权 author。** 平台只保留主规格明文允许的既有交付前规范化与不改 source 的等价解析；「未持久化」「只减不增」「唯一解」均不构成平台改内容的豁免；现有 `contract_autorepair`（`contract_autorepair.rs:65`）的本地 source mutation 与该裁决正面撞线，按裁决 §2.4 落地清单移除（见 tasks 第 8 节）。

- 字母澄清（裁决 §2.1 末段）：本节旧「O1 选项 A」＝修订 SG-02A 放开例外（**已否决**），旧「B」＝全部内容修复转 author；本轮裁决写实质结论「不扩权」，不只写「选 A/B」。
- 四类规则收窄口径（裁决 §2.2 表）：
  - `required_capability_missing`：**取消自动追加 capability**；保留 typed 缺口/机器指令 producer，author 回查原业务与实现承诺，在保护闭包内产新 revision。
  - `unconsumed_required_handoff`：**取消删 ref 或替换为 `[]` 的本地「修复」**；提供完整来源事实，author 可合法重组，平台不得删真实边过门。
  - `unknown_structured_key`：**不再按诊断行删业务承载行**；只生成「保留内容、迁往合法字段」的指令；无 typed/无法定位则不自动。
  - 重复 trusted command：**取消本地删 command/整 check**；producer 指出重复及受影响 check/ref，有 manual 与无 manual 两支均交 author。
- `ProtectedBusinessFacts` 与 `StructuralRepairInstruction`（`contract_prerevision.rs:315-427`）继续复用；修复内容只由 author 执行，平台验证其前置/后置条件。

## MODIFIED REQ-WSC-09 的 typed IR 资格扩充（裁决 §1.4/§1.5）

- 归属：本 change 的 MODIFIED REQ-WSC-09 是唯一持有者；`bound-sc-generation-teaching`（REQ-WSC-28）已登记归档硬门禁——本扩充落地后其方可归档，bound 不得重复 MODIFIED WSC-09；effort / isolate / close 均不含 WSC-09。
- 资格清单加入四类本期可证明定位的 IR 诊断：`unknown_done_when_ref`、`unknown_reviewer_check_ref`、`acceptance_criterion_without_reviewer_check`、`unknown_requirement_ref`（现场 15 条 `unknown_done_when_ref` 不在旧清单，证据见实测基线）。
- 每类必须携带：WI、task/check/criterion、错误 ref、本 WI 合法集合或真实 spec 集合、原行（**无行即「不可定位」**）、所需字段形态。
- producer 落点：`src/product/work_item_contract/validation.rs:245-312` 与拆出的 handler 预检；新增内部 `src/product/work_item_plan_compiler/ir_fault.rs` 承载 typed payload；**不得**从现 `:265-267` 的 message 反解析 task ID。
- 其余结构 kind 复用 `planning_failure.rs` 的事实完整性规则；新增类别必须显式进入规格/typed 白名单，不因 `code` 前缀或一句「缺少」自动获权。
- 数据库/基线/成员解析失败（原 handler `:288-357`）明确为 Operational，不得复用「Some(reasons) 即重驱」。
- 「恰好一次」以前置授权、合法 provider 与剩余机会为条件；保留总额外 ≤1；禁止以 `any(可教)` 放过混合 operational 集合。

## 回退（裁决 §2.5）

- **可关闭该类自动教学**：回退的首要动作是**关闭该类自动教学**、保留 typed 诊断与合法 refs（裁决 §2.5「回退：可关闭该类自动教学、保留 typed 诊断及合法 refs」）。
- **不回退到系统直改业务内容**：回退 MUST NOT 恢复「系统直接删 / 补业务内容」—— **不回退到系统直接删/补业务内容**是硬红线（裁决 §2.5「不回退到系统直接删/补业务内容」）。四类规则的 source mutation 一旦移除，**不因回退重新引入**。
- **内容修改权仍归原授权 author**：回退后内容变化仍走原授权 author，平台只保留主规格明文允许的交付前前言修剪 / 固定中文 heading→英文 / EARS 空白规范化与不改 source 的等价解析。
- **已写新格式数据只由兼容版本读取**：裁决 §2.5 同款口径 —— 新格式落盘数据只能由兼容版本读取，旧二进制不得直接回滚。

## 共享面与排序（处置裁决 §2.2）

- 前置：**决策六大文件拆分先落**（`run/single_candidate.rs`、`workspace_engine/prompts.rs`、`contract_autorepair.rs` 纯移动），本 change 才动 handler。
- `run/single_candidate.rs`（handler）写序：决策六拆分 → **1 mechanicalize（本 change，typed 判据）** → 3 bound（claim/教学）→ 4 effort（env 装配）→ 5 isolate（`forged_done_when` 删除与教学面）。
- `review/routing*.rs`、`review/closure_policy.rs` 写序：**1 mechanicalize** → 6 close（闭合谓词）。
- 本 change 不写 `workspace_engine/prompts.rs`、`lifecycle_store/workspace.rs`、`models/workspace.rs`、`work_item_plan_policy/*`（属序 0/3/4/5/6 包）。

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
