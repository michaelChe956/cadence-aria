# 提案：usage 与耗时的计量口径修正（方案 F）

> 状态：**草案 —— 只立项，不实施**。`tasks.md` 是待办清单，不是已完成记录。
> 依据：主方案 `cadence/designs/2026-11-08_技术方案_WIG生成加速与收敛性加固_v1.0.md` §4（方案 F：可观测性修正，**前置项**）、§8.0（工作包 → OpenSpec change 映射）、§10.1（X8 裁决与 F-1 补充）；上位裁决 `cadence/designs-reviews/2026-11-08_决策_WIG六项owner裁决_v1.0.md` §7（F 行）与 §7.1（1、2 两条集成约束）；实测基线 `cadence/analysis-docs/2026-11-08_WIG实测证据基线_v1.0.md` §12。
> 需求编号：capability `usage-transparency`（**不新建 capability**）。本 change 对既有两条 requirement 做 MODIFIED，并 ADDED 一条「计量口径与耗时埋点（SHALL）」。

## Why

主方案 §4 的定性是「**不修它，任何 token 优化都无法被度量**」——本次现场的 run1 消耗在账面上完全不存在。实测基线 §12 已把三条缺陷坐实到代码级：

1. **F-1 · 跨 provider run 的 usage 被整体覆盖**：`mappings.rs:169-179` 的 `event_id` 固定为 `usage_{role}`，而 `mappings.rs:90-111` 的 upsert 对同 id **整条替换** ⇒ run1 与 run2 同属一个 timeline 节点（`timeline_node_001`）⇒ **run1 整条丢失**。
2. **F-2 · codex/reviewer 侧累计值永远取不到**：`codex_provider/parse.rs:461-484` 优先级**先扁平 `inputTokens`（末轮）、后嵌套 `total_token_usage`（累计）** ⇒ 累计值永远走不到 fallback（实测 reviewer 转录：平台报 61,051/6,310/58,496，累计值 370,004/18,022/289,408）。
3. **F-3 · 无耗时埋点**：`session_state/timeline.rs:195` 写死 `duration_ms: None`、`:403-445` 从不写该字段；`work_item_split_engine/parse.rs:766-777` 写 `run.json` 时不带 `elapsed_ms`。

三条缺陷改法明确，但**不能照抄**：**F-2 的「一行改动」已被 owner 2026-11-09 裁决否决**（主方案 §10.1 X8）——在册规格要求「读取**最近的 per-turn** usage 记录……渲染对应 **turn** 的 token 用量」，无条件把累计值提前会让多 turn 气泡显示全程累计，**违反在册 spec**；**F-1 的「只换 event_id」也不够**（主方案 §10.1「F-1 补充」）——同 role 跨 run 必须并存、同 run 重放必须幂等、**不得**逐条相加重复计费。故本 change 要把**计量口径**本身写成契约。

## What Changes

- **ADDED「计量口径与耗时埋点（SHALL）」**（capability `usage-transparency`）：
  - **per-run 累计**：`run.json` 落 `elapsed_ms` 与 usage，usage 取 CLI `result` 的 running total，**不逐条相加**；
  - **per-node 合计**：`timeline_node_details/*.json` 的 usage 事件**并存不覆盖**，由消费方求和；
  - **节点级耗时**：`duration_ms` 于 `completed_at` 落盘时算出（字段已存在于 `src/web/workspace_ws_types/timeline.rs:84`）；
  - **run 级耗时**：写 `run.json` 时落 `elapsed_ms`（落点 `src/product/work_item_split_engine/parse.rs:766-777`）；
  - **口径纪律**：per-step `output_tokens` 是 placeholder ⇒ assistant message 的 output **不作总量**；去重后的 per-step input/cache 准确 ⇒ 可按 `message.id` 去重求和；claude 侧 CLI `result.usage` 的精确范围**本次未判定** ⇒ **不得声称具体数值**。
- **MODIFIED「本地会话文件 usage 采集（SHALL）」**：F-1 与 F-2 的采集侧契约 —— 同 role 跨 provider run 的 usage **并存不覆盖**、同 run 重放**幂等**、**不重复计费**；codex/reviewer 侧采用累计口径前 **MUST 先判定** `total_token_usage` 属会话累计还是本 turn 累计，判定未确立前 **MUST NOT** 用累计值替换 per-turn 上报值。
- **MODIFIED「消息气泡 token 展示（SHALL）」**：X8 裁决的呈现侧契约 —— 气泡只渲染 per-turn 口径值，会话累计值**不得**被渲染为该 turn 用量（另由 run 级口径承载）；负差/缺基线/会话重置标**未知或新起点**，**不猜零**。
- **X8 的前置判定成为 F-2 的必经步骤**：跨 turn fixture（两个连续 turn + reset fixture）先判定 `total_token_usage` 的语义域，结论落盘后才允许取累计口径。

## 非目标

- **不承接 X7（教学重驱轮进候选尝试流水 jsonl 文件）**：X7 已由 `bound-sc-generation-teaching` 的 **MODIFIED 条款（WSC-25 族）** 覆盖（该 delta 已要求「阶段 MUST 能区分 generate、parse、typed lowering、typed IR 校验与结构拒收」且「若本次触发了 provider 启动则 SHALL 记录其 attempt 序号与 attempt key」）⇒ **本 change 不重复承接**，见 design「与既有 change 的边界」。
- **不改 per-turn 采集语义本身**：kimi 的 `usage.record`（`usageScope=="turn"`）读取、文件缺失时静默不上报等既有行为逐字保留，不借本 change 改动采集时机或容错策略。
- **不改气泡的呈现形态**：紧凑展示形态（input/output/cache_read）与「usage 缺失时不显示该行」逐字保留；本期只约束**渲染的值来自哪个口径**。
- **不承诺任何 token / 耗时收益**：本 change 只修计量，使收益**可被度量**；收益本身须在 F 修好后另行量化（主方案 §10.2）。
- **不声称 claude 侧 CLI `result.usage` 的范围**：未找到 `result` 原文 ⇒ 本期不判定、不声称具体数值。
- **不改 provider 计量上游**：不要求 CLI 侧改变其 usage 字段语义。
- **不做全载体计量治理**：coding 引擎侧另有独立的 usage 事件映射实现，本期不跨到该路径。

## 范围

| 能力 | 变更 |
|---|---|
| `usage-transparency` | **ADDED**「计量口径与耗时埋点（SHALL）」（per-run 累计 / per-node 合计 / 节点级耗时 / run 级耗时 + output 占位不作账 + input/cache 按 `message.id` 去重求和 + 未判定口径不得声称数值） |
| `usage-transparency` | **MODIFIED**「本地会话文件 usage 采集（SHALL）」（F-1：跨 run 并存 / 同 run 幂等 / 不重复计费；F-2：累计口径的跨 turn 前置判定、分工与异常标注） |
| `usage-transparency` | **MODIFIED**「消息气泡 token 展示（SHALL）」（X8：气泡只渲染 per-turn 值、累计值另走 run 级口径、未知/新起点不猜零） |

**不新建 capability**：`usage-transparency` 主规格已存在（2 条 requirement、各 2 场景），本 change 只在其上做 MODIFIED 与 ADDED。

## 关键落点（供实现期定位，坐标以 `.worktrees/feat-wig-optimize-11-08/` 为根）

| 缺陷 | 落点 |
|---|---|
| F-1 | `src/product/workspace_engine/mappings.rs:90-111`、`:169-179`；三个调用点 `src/product/workspace_engine/provider_drive/work_item_plan.rs:412`、`src/product/workspace_engine/review/drive_parts/reviewer_provider_session.inc.rs:422`、`src/product/workspace_engine/provider_drive.rs:611` |
| F-1 回归位 | `src/product/workspace_engine/tests/part_07.rs:753-761`、`src/product/workspace_engine/tests/part_09.rs:146-155`、`src/product/workspace_engine/tests/usage_persistence_diagnostic.rs:106-122`（三处 `event_id == "usage_author"` 固定断言） |
| F-2 | `src/cross_cutting/codex_provider/parse.rs:461-484` |
| F-3 | `src/product/workspace_engine/session_state/timeline.rs:195`、`:403-445`；`src/product/work_item_split_engine/parse.rs:766-777` |

## OpenSpec 阈值判定

**命中**。本 change 新增数据契约（`elapsed_ms` 落盘位置与语义、`duration_ms` 落盘时机）、改变既有采集与展示行为、并修改验收边界（per-turn 气泡的值来源口径）⇒ 按 `AGENTS.md:28` 须先落 OpenSpec 契约，再经 owner 审阅 → `writing-plans` → TDD。

## 风险与取舍

| 风险 | 处置 |
|---|---|
| 「把累计值提到扁平值之前」是最省事的一行改动，但**违反在册 spec**（多 turn 气泡显示全程累计） | **已否决**（owner 2026-11-09）。改为跨 turn fixture 前置判定 + run 级/ per-turn 双口径分工 |
| 跨 turn fixture 的判定结论会随 CLI 版本漂移 | 判定结论 MUST 落盘并可复现；判定未确立前 MUST NOT 取累计口径（fail-safe 取 per-turn） |
| `event_id` 带 run 序号会改变既有落盘标识，三处固定断言会红 | 断言迁移列入 tasks 的验收项，**不改弱断言强度**（仍须断言条数与标识互异） |
| 节点级 `duration_ms` 需要真实起止时刻，缺任一端时会出现不可计算 | 缺端时 SHALL 保持为空并可辨，MUST NOT 伪造 0 |
| 计量口径修好后**历史数据仍不可信**（旧事件已被覆盖且无法还原） | 只对修复后新产生的数据给出口径；MUST NOT 追溯宣称旧数据已修正 |
| 与 `bound-sc-generation-teaching` 的 WSC-25 流水重复承接 | X7 明确由 bound 覆盖，本 change 不重复承接（见上「非目标」） |

## 影响

- **只改规划产物**：本 change 的四件产物为 `proposal.md`、`design.md`、`tasks.md`、`specs/usage-transparency/spec.md`；不改 `src/**`、不改 `openspec/specs/**`、不改其它 change 目录。
- **排序**：本 change 与其它 change **无写面重叠**，可独立先行；且按主方案 §8 序 0，F **必须最先**，否则后续 token 优化不可度量。