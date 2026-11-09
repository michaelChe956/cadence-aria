# Design：usage 与耗时的计量口径修正（方案 F）

## Context

主方案 §4 把方案 F 定为**前置项**：「不修它，任何 token 优化都无法被度量」。本次现场的可观测性缺陷已由实测基线 §12 坐实到代码级，三条互相独立：

| 缺陷 | 根因（代码级） | 现场事实 |
|---|---|---|
| **F-1** 跨 provider run 的 usage 被整体覆盖 | `src/product/workspace_engine/mappings.rs:169-179` 的 `event_id` 固定为 `usage_{role}`；同文件 `:90-111` 的 upsert 对同 id **整条替换**（`*existing = event;`） | run1 与 run2 同属 `timeline_node_001`（覆盖 11:53:56→12:38:35 全程）⇒ run1 整条丢失 |
| **F-2** codex/reviewer 侧累计值永远取不到 | `src/cross_cutting/codex_provider/parse.rs:461-484` 的 `parse_codex_usage` 优先级**先扁平 `inputTokens`（末轮）、后嵌套 `total_token_usage`（累计）** | reviewer 转录最后一条 `token_count`：`last_token_usage` in=61,051 / out=6,310 / cached=58,496；`total_token_usage` in=370,004 / out=18,022 / cached=289,408 |
| **F-3** 无耗时埋点 | `src/product/workspace_engine/session_state/timeline.rs:195` 写死 `duration_ms: None`，`:403-445` 从不写该字段；`src/product/work_item_split_engine/parse.rs:766-777` 写 `run.json` 时不带 `elapsed_ms` | `timeline_nodes.json` 4 个节点 `duration_ms` 全为 null |

**两条不能照抄的改法**（这是本 change 存在的理由，而非「补三行代码」）：

- **F-2 的「一行改动」已被 owner 2026-11-09 裁决否决**（主方案 §10.1 X8）。在册规格 `usage-transparency` 要求「读取**最近的 per-turn** usage 记录……渲染对应 **turn** 的 token 用量」；无条件把嵌套累计值提到扁平值之前 ⇒ 多 turn 会话的气泡显示**全程累计**而非该 turn 值，**违反在册 spec**。
- **F-1 的「只换 event_id」不够**（主方案 §10.1「F-1 补充」）：同一 role 的**两个不同 provider run** 的 usage 必须**并存**；同 run 重放必须**幂等**；**不得**把累计事件逐条相加造成重复计费。

## Goals / Non-Goals

**Goals**

- G1 三个缺陷各自有明确、可验收的改法，且改法**不违反在册 spec**。
- G2 四个计量口径（per-run 累计 / per-node 合计 / 节点级耗时 / run 级耗时）各自归位、互不串用。
- G3 口径纪律写进契约：output 占位值不作总量、input/cache 按 `message.id` 去重求和、未判定项不得声称数值。
- G4 X8 的前置判定成为 F-2 的**必经步骤**（不是可选优化）。
- G5 与其它 change **无写面重叠**，可独立先行，且按主方案 §8 序 0 处于**必须最先**的位置。

**Non-Goals**

- 不承接 X7（教学重驱轮进候选尝试流水 jsonl 文件）——已由 `bound-sc-generation-teaching` 覆盖（见 D6）。
- 不改 per-turn 采集时机与容错策略；不改气泡呈现形态。
- 不判定 claude 侧 CLI `result.usage` 的范围（未找到 `result` 原文）。
- 不承诺任何 token / 耗时收益；不追溯修正已被覆盖的历史数据。
- 不跨到 coding 引擎侧独立的 usage 事件映射实现。
- 不动 provider 计量上游（CLI 侧字段语义）。

## Decisions

### D1 · F-1：usage 事件的落盘身份按 **provider run** 区分

`event_id` 不再只由 role 构成。调用点 SHALL 把该 provider run 的运行序号（或等价运行身份）传入，`event_id` 由 **role + 运行身份** 构成。

由此得到三条性质（与主方案 §10.1「F-1 补充」逐条对应）：

| 性质 | 机制 | 验收 |
|---|---|---|
| 跨 run **并存** | run1/run2 的标识不同 ⇒ upsert 走 `push` 分支而非替换分支 | 同 role 发两次 `UsageReport` ⇒ 落盘 **2 条**、标识不同 |
| 同 run **幂等** | 同一 run 的重复上报命中同一标识 ⇒ upsert 替换为同一条 | 同一条事件重复上报 ⇒ 落盘仍为 1 条 |
| **不重复计费** | 消费方按并存事件求和，不把累计事件逐条相加当增量 | 落盘 input/cache == 转录按 `message.id` 去重求和值 |

三个调用点（`provider_drive/work_item_plan.rs:412`、`review/drive_parts/reviewer_provider_session.inc.rs:422`、`provider_drive.rs:611`）SHALL 传同一口径的运行序号，避免同一 run 在三条链路上产生不一致的标识。

**回归位迁移**：`src/product/workspace_engine/tests/part_07.rs:753-761`、`part_09.rs:146-155`、`src/product/workspace_engine/tests/usage_persistence_diagnostic.rs:106-122` 三处 `event_id == "usage_author"` 固定断言 SHALL 迁移为「按新标识规则匹配 + 断言条数与标识互异」，**MUST NOT 弱化为「至少有一条」**。

### D2 · F-2：累计口径的取值以**跨 turn fixture 判定**为前置

**否决**「无条件把 `nested("/total_token_usage/…")` 提到 `field()` 之前」（X8 裁决原文）。改为三段式：

1. **前置判定**：用**两个连续 turn + reset fixture** 判定 `total_token_usage` 是「会话累计」还是「本 turn 累计」，结论 MUST 落盘可复现。
2. **按判定结果分工**：
   - 判定为**会话累计** ⇒ per-turn 上报**仍取 per-turn 值**；累计值**另走 run 级口径**承载，MUST NOT 覆盖 per-turn 值。
   - 判定为**本 turn 累计** ⇒ MAY 直接以该值作为该 turn 的 per-turn 值。
3. **异常不猜零**：负差 / 缺基线 / 会话重置 ⇒ 标记为**未知或新起点**，MUST NOT 填 0。

判定未确立前（fail-safe）⇒ 沿用现有 per-turn 取值，MUST NOT 取累计值替换 per-turn 值。

> **与主方案 §4「验证」② 的对齐**：主方案 §4 的验证②原文是「构造同时含扁平与嵌套 usage 的 codex `turn/completed` 事件 ⇒ 断言取到嵌套累计值」。该条须按 X8 裁决限定为：**在跨 turn fixture 已判定 `total_token_usage` 语义域的前提下**，断言 **run 级累计口径**取到嵌套累计值，且 **per-turn 口径未被该累计值覆盖**。无此前置判定时该断言不成立 —— 不得以「取到累计值」单条断言冒充 X8 的前置判定。

### D3 · F-3：节点级与 run 级耗时各自落盘

| 层级 | 字段 | 落点 | 语义 |
|---|---|---|---|
| 节点级 | `duration_ms` | `src/product/workspace_engine/session_state/timeline.rs`（`:195` 的写死 `None` 与 `:403-445` 的终态落盘路径） | 节点进入终态并落 `completed_at` 时**同批算出**；字段已存在于 `src/web/workspace_ws_types/timeline.rs:84` |
| run 级 | `elapsed_ms` | `src/product/work_item_split_engine/parse.rs:766-777`（写 `run.json`） | 该 provider run 的墙钟耗时，与 `prompt_chars` 等既有字段并列 |

**缺端规则**：缺任一端时刻时该字段 SHALL 保持为空且可辨（沿用 `Option` 语义），MUST NOT 伪造 0。

### D4 · 计量口径表（四个口径各自归位）

| 口径 | 数据源 | 取值规则 | 禁止 |
|---|---|---|---|
| **per-run 累计** | `run.json` 的 `elapsed_ms` + usage | usage 取 CLI `result` 的 running total | 逐条累加 per-step 事件当总量 |
| **per-node 合计** | `timeline_node_details/*.json` 的 usage 事件 | 事件**并存不覆盖**，由消费方求和 | 消费方把覆盖式快照当全量 |
| **节点级耗时** | timeline 节点的 `duration_ms` | 终态落 `completed_at` 时同批算出 | 恒为 null / 恒为 0 |
| **run 级耗时** | `run.json` 的 `elapsed_ms` | 写 `run.json` 时落盘 | 用逐条事件的时间戳累加代替墙钟 |

**四个口径互不串用**：per-node 的合计**不是** per-run 累计；per-run 的 usage**不是** per-turn 值；节点级耗时**不是** run 级耗时。

### D5 · 口径纪律（官方一手文档 `code.claude.com/docs/en/agent-sdk/cost-tracking.md`）

| 纪律 | 内容 |
|---|---|
| output 是占位值 | per-step `output_tokens` 是 placeholder ⇒ assistant message 的 output **不能**逐条相加当总量 ⇒ **output 不作账** |
| input/cache 去重后准确 | 去重后的 per-step input / cache 是准确的 ⇒ 可按 `message.id` 去重求和 ⇒ 落盘 input/cache **SHALL 等于**该去重求和值 |
| `result.usage` 范围未判定 | claude 侧 CLI `result.usage` 是 invocation 累计还是末轮值，**本次未判定**（未找到 `result` 原文）⇒ **不得声称具体数值** |

### D6 · **X7 已由 `bound-sc-generation-teaching` 覆盖，本 change 不重复承接**（显式声明）

主方案 §10.1 的 X7（WSC-25 族的尝试流水漏记 generate 段 —— 教学重驱轮应进候选尝试流水 jsonl 文件）**已由 `openspec/changes/bound-sc-generation-teaching/` 的 MODIFIED 条款承载**：该 delta 已要求「阶段 MUST 能区分 generate、parse、typed lowering、typed IR 校验与结构拒收」，且「若本次触发了 provider 启动则 SHALL 记录其 attempt 序号与 attempt key」。

⇒ **本 change 不承接 X7**，不改候选尝试流水文件及其任何行为；本 change 的 `specs/usage-transparency/spec.md` 中**零命中** WSC-25 族的 requirement 编号与候选尝试流水文件名。

### D7 · 与既有 change 的边界（不重复承接同一行为）

| 行为 | 归属 | 本 change 的态度 |
|---|---|---|
| 候选尝试流水（generate/parse/IR/structure 阶段与 attempt 关联） | `bound-sc-generation-teaching`（WSC-25 族的 MODIFIED 条款） | **不重复承接**（D6） |
| 契约类 finding 的必关闭判定 | `close-contract-findings-before-pass`（MODIFIED REQ-WSC-17） | 不涉及 |
| effort 档位冻结 | `explicit-sc-author-effort` | 不涉及；本 change 不写任何 effort 结论 |
| reviewer 投影裁剪 | `rework-prompt-input-surface`（独占 MODIFIED REQ-WSC-06） | 不涉及 |

## 根因 → 改法 → 验证 逐项映射

| 缺陷 | 根因 | 改法（契约层） | 验证（可执行用例） |
|---|---|---|---|
| **F-1** | `event_id` 只含 role ⇒ 跨 run 同 id；upsert 覆盖式替换 ⇒ run1 整条丢失 | usage 事件标识带 provider run 运行身份 ⇒ 跨 run 并存、同 run 幂等、不得逐条相加重复计费 | **①** 同 role 发两次 `UsageReport` ⇒ 断言落盘 **2 条**、标识不同；**④** 修复后重跑同一场景 ⇒ 落盘 usage == 转录按 `message.id` 去重求和的 input/cache（**output 不作断言**） |
| **F-2** | `parse_codex_usage` 先扁平（末轮）后嵌套（累计）⇒ 累计值永远取不到；但无条件提前会违反 per-turn 在册 spec | **前置判定**（跨 turn fixture 判定 `total_token_usage` 语义域）→ 按判定结果分工（会话累计 ⇒ run 级承载、per-turn 不变；本 turn 累计 ⇒ 可直接取）；负差/缺基线/重置 ⇒ 未知或新起点 | **②** 构造同时含扁平与嵌套 usage 的 codex `turn/completed` 事件 ⇒ **在判定已确立的前提下**断言 run 级口径取到嵌套累计值、per-turn 口径未被覆盖；**X8 前置判定用例**：两个连续 turn + reset fixture ⇒ 断言累计域判定结论落盘、异常标未知/新起点而非 0 |
| **F-3** | 节点 `duration_ms` 写死 `None` 且终态路径不写；`run.json` 无 `elapsed_ms` | 终态落 `completed_at` 时同批算出 `duration_ms`；写 `run.json` 时落 `elapsed_ms` | **③** 真实 session ⇒ 断言 `duration_ms` 非 null；并断言 `run.json` 含 `elapsed_ms` |
| **口径纪律** | per-step output 是占位值；claude 侧 `result.usage` 范围未判定 | output 不作总量；input/cache 按 `message.id` 去重求和；未判定项不得声称数值 | **④** 的 input/cache 断言；**output 不作断言**；未判定口径的「不得声称数值」用例 |

## 回退

- **纯观测面，回退代价最低**：本 change 只改**计量与埋点**，不改任何业务判定与执行路径 ⇒ 回退仅需停用新写入路径；已写入的 `usage` 事件与 `duration_ms` / `elapsed_ms` **保留可读**（append-only 记录不因回退被删除）。
- **不丢历史计数**：回退 MUST NOT 删除或重算已落盘的 usage 事件；旧口径（末轮值）的事件保持原样，不因回退被「修正」成累计值。
- **不承诺收益**：回退不影响任何 token / 耗时口径的对外表述 —— 本 change **从不承诺**降幅（见 W1）。

## 共享面与排序（跨 change）

**处置裁决 §2.2 的全局表**（`cadence/designs-reviews/2026-11-09_裁决_WIG四change审阅问题处置_v1.0.md` §2.2）给的是 0～6 的串行顺序。本 change **不在该表的六行之内**（它是新增的独立行），按主方案 §8 序 0 定位为**必须最先**：

| 序 | 包 | 承担的共享面 | 前置 |
|---|---|---|---|
| **0′** | **本 change `fix-usage-and-timing-metrics`** | `usage-transparency` 的 ADDED 计量口径 + MODIFIED 采集/展示口径 | 无 —— **必须最先**（主方案 §8 序 0：F 不先修，后续 token 优化不可度量） |

**对称登记义务**（处置 §2.2「每个 change 的 design 必须列出与它共写同一文件的其它包，并写明相对次序」）：逐文件核对如下 ——

| 文件 | 本 change 写入 | 与其它包共写？ | 相对次序 |
|---|---|---|---|
| `src/product/workspace_engine/mappings.rs` | 是（usage 事件标识） | **否** —— 处置 §2.2 的文件级归属表未列该文件 | 无共写方 |
| `src/cross_cutting/codex_provider/parse.rs` | 是（codex usage 取值） | **否** —— 未列 | 无共写方 |
| `src/product/workspace_engine/session_state/timeline.rs` | 是（节点级耗时） | **否** —— 未列 | 无共写方 |
| `src/product/work_item_split_engine/parse.rs` | 是（run 级耗时） | **否** —— 未列 | 无共写方 |
| `src/product/workspace_engine/tests/{part_07,part_09,usage_persistence_diagnostic}.rs` | 是（回归位迁移） | **否** —— 未列 | 无共写方 |

⇒ **与 `mechanicalize-plan-compile-repair`、`fix-author-rewrite-quota-pool`、`bound-sc-generation-teaching`、`explicit-sc-author-effort`、`isolate-plan-criterion-namespace`、`close-contract-findings-before-pass`、`rework-prompt-input-surface` 均无写面重叠**，**独立可先行**（主方案 §1.3 F 行「可先行立项」）。

**与「决策六：拆分大文件」的先后**：决策六的写面是 `run/single_candidate.rs`、`workspace_engine/prompts.rs`、`contract_autorepair.rs` —— 与本 change 的写面**无交集**，故本 change **不依赖**决策六先行；反之本 change 也不占用决策六的拆分预算。

**与其它包的行为边界**（不是写面，但不得重复承接）：`bound-sc-generation-teaching` 承载 WSC-25 候选尝试流水（含 X7 的 generate 段阶段与 attempt 关联），本 change **不承接**（D6、D7）；`close-contract-findings-before-pass` 承载契约类 finding 的必关闭判定，本 change 不涉及。

**集成 owner 事项**：本 change 只动 `src/product/workspace_engine/mappings.rs`、`src/cross_cutting/codex_provider/parse.rs`、`src/product/workspace_engine/session_state/timeline.rs`、`src/product/work_item_split_engine/parse.rs` 与三处既有测试文件，**不触碰** handler 与 prompt 共享文件（`run/single_candidate.rs`、`workspace_engine/prompts.rs`）⇒ 无需与 B/C 的集成 owner 争用同一切片。

## 不确定性纪律（**未实测项 MUST NOT 写成已测结论**）

以下事项在本 change 的规划、验收与任何对外表述中**必须保持未判定/未实测状态**：

| 未实测 / 未判定项 | 状态 | 纪律 |
|---|---|---|
| claude 侧 CLI `result.usage` 的精确范围（invocation 累计 vs 末轮值） | **未判定**（未找到 `result` 原文） | MUST NOT 声称具体数值；只约束「取 running total」这一口径方向 |
| TTL（缓存 1h）净价差 | **未实测** | MUST NOT 写成已测收益或确定事实 |
| `Read` 输出上限的默认值与截断行为 | **未确认**（主方案 §10.4） | MUST NOT 写成已实现或已验证 |
| effort `high` 的提速量级 | **未实测**（裁决 §3.2 / §3.5：本机单次 A/B，跨环境复现性未验证） | MUST NOT 写成「44 分钟 → 10–15 分钟」或任何量化结论 |
| F 修复后的 token / 耗时收益 | **不可度量直到 F 修好**（主方案 §10.2） | MUST NOT 预先声称降幅 |
| codex 侧 reviewer 用量的低估倍数（实测基线 §12 的 6.06× / 2.85× / 4.95×） | **既有样本的实测事实**（单一转录样本） | 只能作为「修前口径失真」的证据；MUST NOT 写成修复后的收益，也 MUST NOT 当作跨样本规律 |

**另需保持的不确定性**：历史数据不可信 —— 已被覆盖的 run1 usage 无法还原，MUST NOT 追溯宣称旧数据已修正；本 change 的口径只对修复后新产生的数据生效。

## Risks / Trade-offs

| 风险 | 处置 |
|---|---|
| 「累计值优先」是最省事的一行改动，但违反在册 per-turn spec | 已否决（X8）；改为前置判定 + 双口径分工，判定未确立前 fail-safe 取 per-turn |
| 跨 turn fixture 的判定结论会随 CLI 版本漂移 | 结论 MUST 落盘可复现；判定未确立 ⇒ 取 per-turn（fail-safe） |
| `event_id` 变化使三处固定断言红 | 断言迁移列入 tasks 验收项，**不弱化强度** |
| `duration_ms` 需要真实起止时刻 | 缺端保持为空且可辨，MUST NOT 伪造 0 |
| 修好后历史数据仍是错的 | 口径只对新数据生效；不追溯宣称 |
| 与其它 change 串行耦合 | 已逐文件核对：**无写面重叠**；只对 `bound` 的 X7 做行为边界声明 |

## Migration Plan

1. **无数据迁移**：既有 timeline node detail 的 usage 事件与既有 `run.json` 不回填、不改写；新口径只对修复后新产生的数据生效。
2. **标识变更的兼容期**：usage 事件标识由「role」扩展为「role + 运行身份」。消费方 SHALL 按「同一 run 内同标识、同 run 跨不同 run 不同标识」的新规则读取；旧数据中不存在运行身份信息 ⇒ 旧事件按其原标识读取，**MUST NOT** 猜测其所属 run。
3. **回退**：停用新的标识规则与耗时落盘后，既有落盘数据仍可按旧规则读出；已写入的新形态数据由兼容版本读取，不做反向改写。

## Open Questions

| # | 问题 | 状态 | 处置 |
|---|---|---|---|
| 1 | `total_token_usage` 是会话累计还是本 turn 累计？ | **未判定** | 由 tasks 的**前置判定**工作包以跨 turn fixture 判定，结论落盘后才允许取累计口径 |
| 2 | claude 侧 CLI `result.usage` 是 invocation 累计还是末轮值？ | **未判定**（未找到 `result` 原文） | 保持未判定；MUST NOT 声称具体数值；后续取得原文后再立项 |
| 3 | 节点级 `duration_ms` 的起止时刻以哪一侧为准（发起 / 落盘）？ | 待实现期按既有 `started_at` / `completed_at` 语义确定 | 契约只要求「同批算出、缺端不伪造 0」，不预先钉死实现细节 |
| 4 | coding 引擎侧另有独立的 usage 事件映射实现，是否同样存在 F-1 的覆盖式问题？ | **未核实**（超出本 change 范围） | 登记为待独立评估项；本 change **不跨到该路径**，也 MUST NOT 声称其已受影响或不受影响 |