# Tasks：fix-usage-and-timing-metrics

> **本清单是待办，不是已完成记录。** 每项完成前 MUST NOT 勾选。
> 契约来源：本 change 的 `specs/usage-transparency/spec.md`（ADDED「计量口径与耗时埋点（SHALL）」+ MODIFIED「本地会话文件 usage 采集（SHALL）」+ MODIFIED「消息气泡 token 展示（SHALL）」）。
> 依据：主方案 §4（方案 F）、§8 序 0（F 必须最先）、§10.1（X8 裁决、F-1 补充）；实测基线 §12。
> 排序：本 change 与其它 change **无写面重叠**，可独立先行；按主方案 §8 序 0，F **必须最先**（不先修，后续 token 优化不可度量）。
> **X7 不在本清单内**：候选尝试流水（generate/parse/IR/structure 阶段与 attempt 关联）已由 `bound-sc-generation-teaching` 的 MODIFIED 条款（WSC-25 族）承载，本 change 不重复承接。

## 0. 前置：基线锚定与口径纪律落纸（不改行为）

- [ ] 0.1 锚定改前事实并记录：同 role 跨 provider run 的 usage 在改前**只落 1 条**（run1 被整条覆盖）；codex/reviewer 侧改前取到的是**末轮值**而非累计值；`timeline_nodes.json` 的 `duration_ms` 改前**恒为 null**；`run.json` 改前**无** `elapsed_ms`。
- [ ] 0.2 落纸口径纪律（官方一手文档 `code.claude.com/docs/en/agent-sdk/cost-tracking.md`）：per-step `output_tokens` 是 placeholder ⇒ **output 不作总量**；去重后的 per-step input/cache 准确 ⇒ 可按 `message.id` 去重求和；claude 侧 CLI `result.usage` 的范围**未判定** ⇒ **不得声称具体数值**。
- [ ] 0.3 落纸不确定性纪律：TTL 净价差、`Read` 截断行为、effort `high` 提速量级、F 修复后的收益**均为未实测/未判定**，MUST NOT 在本 change 的任何产物或对外表述中写成已测结论。

## 1. F-1 · usage 事件按 provider run 区分（跨 run 并存 / 同 run 幂等）

> 契约：MODIFIED「本地会话文件 usage 采集（SHALL）」。

- [ ] 1.1 改 `src/product/workspace_engine/mappings.rs:169-179`：usage 事件的 `event_id` 由「仅 role」改为「role + 该 provider run 的运行身份」，使同一 role 的不同 run 产生不同标识。
- [ ] 1.2 确认 `src/product/workspace_engine/mappings.rs:90-111` 的 upsert 语义与 1.1 一致：同标识（同一 run 的重放）合并为一条 ⇒ **幂等**；不同标识 ⇒ 并存而非覆盖 ⇒ **跨 run 并存**。MUST NOT 通过「逐条累加累计事件」来实现并存（会造成重复计费）。
- [ ] 1.3 三个调用点传入同一口径的运行身份：`src/product/workspace_engine/provider_drive/work_item_plan.rs:412`、`src/product/workspace_engine/review/drive_parts/reviewer_provider_session.inc.rs:422`、`src/product/workspace_engine/provider_drive.rs:611`。
- [ ] 1.4 **回归位迁移（主方案 §10.1「F-1 补充」）**：`src/product/workspace_engine/tests/part_07.rs:753-761`、`src/product/workspace_engine/tests/part_09.rs:146-155`、`src/product/workspace_engine/tests/usage_persistence_diagnostic.rs:106-122` 三处 `event_id == "usage_author"` 固定断言 MUST 改为按新标识规则匹配并**断言条数与标识互异**；MUST NOT 弱化为「至少存在一条」。

## 2. F-2 · codex/reviewer 侧累计口径（**前置判定不可跳过**）

> 契约：MODIFIED「本地会话文件 usage 采集（SHALL）」+ MODIFIED「消息气泡 token 展示（SHALL）」。

- [ ] 2.1 **【前置 · 不得跳过】判定 `total_token_usage` 的语义域**：构造**两个连续 turn + reset** 的跨 turn fixture，判定 `total_token_usage` 是「会话累计」还是「本 turn 累计」，结论 MUST 落盘可复现。**本项未完成前，2.2～2.4 MUST NOT 实施**（X8 裁决：否决「无条件把 nested 提到 field 前」）。
- [ ] 2.2 按 2.1 的判定结论改 `src/cross_cutting/codex_provider/parse.rs:461-484` 的取值优先级：
  - 判定为**会话累计** ⇒ per-turn 上报**仍取 per-turn 值**，累计值**另走 run 级口径**承载，MUST NOT 覆盖 per-turn 值；
  - 判定为**本 turn 累计** ⇒ MAY 直接以该值作为该 turn 的 per-turn 值。
  - **MUST NOT** 采用与 2.1 结论相反的无条件优先级。
- [ ] 2.3 异常处理：会话累计值相对前次基线为**负差**、**缺基线**或**会话已重置** ⇒ 标记为**未知或新起点**，MUST NOT 猜零。
- [ ] 2.4 呈现侧对齐：气泡只渲染 per-turn 口径值；会话累计口径的值 MUST NOT 被渲染为该 turn 的用量；未知/新起点 MUST NOT 渲染为 0 或其它猜测值。

## 3. F-3 · 耗时埋点（节点级 `duration_ms` + run 级 `elapsed_ms`）

> 契约：ADDED「计量口径与耗时埋点（SHALL）」。

- [ ] 3.1 节点级：`src/product/workspace_engine/session_state/timeline.rs:195`（写死的 `duration_ms: None`）与 `:403-445`（终态落盘路径）—— 节点进入终态并落 `completed_at` 时**同批算出** `duration_ms`；字段已存在于 `src/web/workspace_ws_types/timeline.rs:84`，MUST NOT 新增重复字段。
- [ ] 3.2 run 级：`src/product/work_item_split_engine/parse.rs:766-777` 写 `run.json` 时落 `elapsed_ms`。
- [ ] 3.3 缺端规则：缺任一端时刻时该字段 SHALL 保持为空且可辨，MUST NOT 伪造 0。

## 4. 计量口径与消费方纪律

> 契约：ADDED「计量口径与耗时埋点（SHALL）」。

- [ ] 4.1 per-run 累计：`run.json` 的 usage 取 CLI `result` 的 running total，MUST NOT 把该 run 的 per-step usage 事件逐条相加当总量。
- [ ] 4.2 per-node 合计：`timeline_node_details/*.json` 的 usage 事件并存不覆盖，由消费方求和；消费方 MUST NOT 把覆盖式快照当全量。
- [ ] 4.3 output 不作账：MUST NOT 把 assistant message 的 per-step `output_tokens` 逐条相加当作总 output。
- [ ] 4.4 input/cache 可核：落盘 usage 的 input 与 cache SHALL 等于转录按 `message.id` 去重求和的值。

## 5. 验收（主方案 §4「验证」段逐条落成可执行用例）

> 四条均为主方案 §4「验证」段的原条目；第 ② 条按 X8 裁决限定为「在 2.1 判定已确立的前提下」。

- [ ] 5.1 **验收①（同 role 两次上报）**：同 role 发两次 `UsageReport` ⇒ 断言落盘 **2 条** usage 事件、且两条标识不同。（对应 ADDED「per-node usage 事件并存」+ MODIFIED「同 role 跨 provider run 的 usage 并存」）
- [ ] 5.2 **验收②（扁平与嵌套并存的 codex 事件）**：构造同时含扁平 usage 与嵌套 `total_token_usage` 的 codex `turn/completed` 事件 ⇒ 在 2.1 判定已确立的前提下，断言 **run 级累计口径**取到嵌套累计值，且 **per-turn 口径未被该累计值覆盖**。（对应 ADDED「per-run 累计取 running total」+ MODIFIED「累计口径的跨 turn 前置判定」）
- [ ] 5.3 **验收③（真实 session 的耗时）**：真实 session ⇒ 断言节点的 `duration_ms` **非 null**；并断言 `run.json` 含 `elapsed_ms`。（对应 ADDED「节点级耗时与 completed_at 同批落盘」+ ADDED「run 级耗时落盘」）
- [ ] 5.4 **验收④（修复后重跑同一场景）**：修复后重跑同一场景 ⇒ 断言落盘 usage **等于**转录按 `message.id` 去重求和的 **input / cache**；**output 不作断言**（占位值语义）。（对应 ADDED「input 与 cache 按 message.id 去重求和」+ ADDED「output 占位值不作总量」）
- [ ] 5.5 **X8 前置判定用例**：两个连续 turn + reset fixture ⇒ 断言 2.1 的判定结论已落盘；负差/缺基线/会话重置被标记为未知或新起点而非 0；气泡不显示 0。（对应 MODIFIED「差分异常不猜零」+ MODIFIED「未知或新起点不显示为 0」）
- [ ] 5.6 **未判定口径用例**：任何对外输出中 MUST NOT 出现 claude 侧 CLI `result.usage` 的具体累计数值。（对应 ADDED「未判定的口径不得声称数值」）

## 6. 收口与门禁

- [ ] 6.1 `openspec validate fix-usage-and-timing-metrics --strict` 通过。
- [ ] 6.2 归档演练：`/tmp` 副本归档后 `openspec validate --specs --strict` 全绿；主规格 `usage-transparency` 的**原有 2 条 requirement 的 4 个场景标题与正文逐字仍在**；三条 requirement 的描述段均 ≤500 字符。
- [ ] 6.3 排序与边界登记复核：本 change 与其它 change 无写面重叠（design「共享面与排序」逐文件表）；**X7 未被重复承接**（本 change 目录内对 WSC-25 族 requirement 编号与候选尝试流水文件名的引用零命中）。

## Workflow follow-up / 需 owner 另授权后（**不阻归档**）

- [ ] W1 真实 provider 调用与修复后收益量化：须 owner 另行授权后执行；本 change 不授权任何真实调用，**不承诺**任何 token / 耗时降幅。
- [ ] W2 claude 侧 CLI `result.usage` 原文取得与范围判定：取得原文后另行立项；在此之前保持未判定。
- [ ] W3 coding 引擎侧独立 usage 事件映射实现是否存在同类覆盖式问题：超出本 change 范围，待独立评估（design Open Questions #4）。

## requirement 映射

| 工作包 | requirement |
|---|---|
| 1.x | MODIFIED「本地会话文件 usage 采集（SHALL）」（并存 / 幂等 / 不重复计费） |
| 2.1 | MODIFIED「本地会话文件 usage 采集（SHALL）」（累计口径的跨 turn 前置判定） |
| 2.2 / 2.3 | MODIFIED「本地会话文件 usage 采集（SHALL）」（判定后的分工 / 差分异常不猜零） |
| 2.4 | MODIFIED「消息气泡 token 展示（SHALL）」（气泡只显示该 turn 的 per-turn 值 / 未知或新起点不显示为 0） |
| 3.x | ADDED「计量口径与耗时埋点（SHALL）」（节点级耗时 / run 级耗时） |
| 4.x | ADDED「计量口径与耗时埋点（SHALL）」（per-run 累计 / per-node 合计 / output 不作账 / input-cache 去重求和） |
| 5.x | 三条 requirement 的全部新增场景 |