# WP5 关闸报告（task 5.3）：跨会话批量确认真实链 + R-2 全程终检

**日期**：2026-09-30
**工作包**：`cockpit-fullcourse-connection-resilience` task 5.3（WP5·P4 清尾关闸）
**验证口径**（tasks.md 原文）：清点表齐备；（5.1 已交付）跨会话批量真实链证据——两个真实会话各有开态门时批量确认各自恰一次；其他连接经广播观察到关闭；危险动作反例；全程 R-2 口径终检。

## 0. 结论总览

| 验证面 | 结果 | 证据段落 |
|---|---|---|
| 清点表齐备（5.2 无沉默项，7/7 有处置+理由） | ✅ | §6 |
| 跨会话批量真实链·断言一：两会话各恰一次确认（durable timeline 各一个 confirm 关门节点、无重复） | ✅ | §4.1 |
| 跨会话批量真实链·断言二：其他连接（observer）经广播观察到两门关闭 | ✅ | §4.2 |
| 跨会话批量真实链·断言三：危险动作不参与批量（被拒反例×2 种×2 会话，门零受动） | ✅ | §4.3 |
| 附加反例：重复 confirm 幂等拒否（无第二次关门） | ✅ | §4.4 |
| R-2 终检：web/src 用户可见文案零「根治」；change/设计文档 29 处命中全为形态级或纪律自引用 | ✅ | §5 |

**判定：5.3 关闸三口径全过，WP5 收口条件齐备。**

红线自查：主服务器 aria-dev-v49a（127.0.0.1:4317）全程只作客户端、未重启未触碰；零直写（全部经产品 REST/WS 面）；载体全部 `project_0003`（issue_0024/0025/0026/0027，无 project_0001）；无新缺口登记（一处客户端观察项登记于 §4.5，非缺口）。

## 1. 验证环境

- 服务器：`aria-dev-v49a`＝`./target/debug/aria web --workspace <worktree> --host 127.0.0.1 --port 4317 --work-item-plan-single-candidate`（14:23 起持续在跑，本验证未动）
- 前端产物：`web/dist`（13:14 构建）由同服务器托管，`index-DacrUKlO.js` 含「跨会话批量确认」UI 文案（产品面在线）
- 真实 provider：author=pi / reviewer=pi（legacy 直连，同 p1e/A4/A5 先例口径）

## 2. 批量确认面定位（产品面与等价 API 面）

产品面 = 驾驶舱收件箱跨会话多选批量确认。代码链路（五点）：

| # | 位置 | 事实 |
|---|---|---|
| 1 | `web/src/components/chat-workspace/cockpit/CockpitInbox.tsx:904-917`（`isSelectableGate`） | 只有 `kind="human_gate"` 且 `stage !== "author_confirm"` 且门开（`closed===null`、无 action block）的条目提供多选勾选；`batch_confirm`/`compile_recovery` 等其余门种与未知 kind 一律 fail-closed 不可选 |
| 2 | `web/src/state/cockpit-operation-semantics.ts:51-55` | `BULK_OPERATION_WHITELIST = {"confirm"}`——批量面唯一白名单操作是 confirm，危险动作（abort/terminate/restart 类）不存在批量入口 |
| 3 | `web/src/pages/ChatCockpitPage.tsx:738-752`（`handleBulkConfirm`） | 当前会话门走既有单门动作；**其他会话**条目进入 `useBulkConfirmStore.start()`（批量 runner） |
| 4 | `web/src/state/bulk-confirm-runner.ts:41-42,105-133,201-215` | 批量协议：每目标一条短命 WS——`hello{role:"driver",last_seen_node_id:null}` → 首帧 `session_state` → 发 `{type:"confirm"}` → 等 `human_gate_closed`=confirmed／protocol_error=rejected／8s 超时=failed；按 sessionId 去重；并发上限 `BULK_CONFIRM_CONCURRENCY=3` |
| 5 | `src/web/workspace_ws_handler/decisions/inbound.rs:198-212` | 服务端：SC 流 confirm 帧 → `handle_human_gate_termination(Approve)`；角色仲裁（4.3）与 session 级广播（4.5）为前置 |

本验证的批量执行 = 上述 runner 协议的**逐帧等价复刻**（`/tmp/wp5_bulk_confirm2.mjs` phase3：同 hello、同 confirm 帧、同 8s 超时、同 sessionId 去重、同并发上限），因此结论可直接映射到产品面行为。

## 3. 双门造场（真实 provider 全链，两门同时在场）

每载体真实链：`issue 创建 → story(pi)→author_confirm 门→HTTP confirm→Confirmed → design(pi)→同前 → work-item-plans:prepare(pi/pi,interactive) → start_generation → choice REST 应答 → author/reviewer 各两轮 → human_confirm 开态门停等`。

| 载体 | issue | plan 会话 | 门节点 | 门开时刻（UTC） | durable 形态（门开时） |
|---|---|---|---|---|---|
| C | issue_0026 | workspace_session_0133 | timeline_node_004 | 09:38:15 | waiting_for_human + human_confirm:active + phase=approval |
| D | issue_0027 | workspace_session_0134 | timeline_node_006 | 10:04:34 | waiting_for_human + human_confirm:active + phase=approval |

C 门开 26 分钟后 D 门开——**两门同时在场**（门等待无超时，P2 语义下 driver 断开只撤 lease 不动门；D 门开时刻起两门并存直至批量动作）。佐证轮 A/B（issue_0024/0025，session_0108/0107，09:26/09:29 双门并存）同形态。

命令：`node /tmp/wp5_gate_setup.mjs <tag>`（A/B/C/D）；日志 `/tmp/wp5_setup_{A,B,C,D}.log`（`GATE_OPEN` 行含 stage/status/gate_node/event_seq）。

## 4. 批量确认真实链（主证轮 C+D；`node /tmp/wp5_bulk_confirm2.mjs /tmp/wp5_gate_C.json /tmp/wp5_gate_D.json`）

### 4.1 断言一：两会话各恰一次确认 ✅

一次批量动作（并发 2 ≤ 前端并发上限 3，等价 `runBulkConfirm` 单次调用）同时确认两门：

```
phase3 [C] session_state→confirm sent          （10:04:49.597）
phase3 [D] session_state→confirm sent          （10:04:49.598）
phase3 outcome {"tag":"C",...,"status":"confirmed","ms":4805}
phase3 outcome {"tag":"D",...,"status":"confirmed","ms":6489}
```

durable 恰一次（前后快照，`/tmp/wp5_bulk_result2.json`）：

| 会话 | before | after |
|---|---|---|
| C·0133 | status=waiting_for_human，human_confirm 节点=[timeline_node_004:active]，run_history={repairs:0,transitions:1,manual:0} | status=confirmed/phase=completed，human_confirm 节点=[timeline_node_004:**completed**]（仍恰 1 个），run_history 逐字段不变 |
| D·0134 | status=waiting_for_human，human_confirm 节点=[timeline_node_006:active]，run_history={repairs:1,transitions:1,manual:0} | status=confirmed/phase=completed，human_confirm 节点=[timeline_node_006:**completed**]（仍恰 1 个），run_history 逐字段不变 |

每会话恰一个 confirm 关门节点、无重复关门、无 supersede 竞态；批量动作未额外消耗 repair/transition 预算。终态链完整：human_confirm:completed → work_item_plan_compile:completed → completed（C 6 节点/D 8 节点 timeline）。

### 4.2 断言二：其他连接经广播观察到关闭 ✅

批量动作前各挂一条常驻 `role=observer` WS（phase2）。两 observer 各**恰一次**收到关门广播，帧序完整（无刷新、被动连接实时可见）：

```
phase5 [C] observer frames=[session_state,session_state,timeline_node_updated,stage_change,
  timeline_node_created,artifact_update×7,timeline_node_updated,stage_change,
  timeline_node_created,human_gate_closed] closed_count=1
phase5 [D] observer frames=[…同型…] closed_count=1
```

广播载荷：`{"type":"human_gate_closed","decision":"confirm","stage":"completed","event_seq":14}`（C）/ `event_seq:15`（D）——session 级单调 `event_seq` 在场（4.5 契约）。

### 4.3 断言三：危险动作不参与批量（反例） ✅

批量通道（与批量确认同型的短命 driver 连接）上尝试危险动作，两会话各两 种，全部被拒且**门零受动**（phase1，动作后 session_state 复核 `waiting_for_human + human_confirm:active` 不变）：

| 危险动作 | 服务端拒绝（原文截取） | 门状态 |
|---|---|---|
| `{"type":"restart_coding"}`（任务点名类） | `error: "invalid message: unknown variant 'restart_coding', expected one of user_message, context_note, start_generation, …"`——workspace WS 消息面不存在该类型，parse 面拒收 | 仍开 ✅ |
| `{"type":"work_item_plan_compile_recovery_action","action":"abort_and_rollback",…}`（真实 typed 危险动作：放弃并回滚） | `protocol_error INVALID_COMPILE_RECOVERY_ACTION: "work_item_plan_compile_recovery_action requires active work_item_plan_compile_recovery node"`（引擎安全拒否，compile.rs:911-923） | 仍开 ✅ |

产品面同构证据：`BULK_OPERATION_WHITELIST={"confirm"}`（§2#2）——危险动作在批量 UI 无入口；`isSelectableGate`（§2#1）——危险/未知门种不可多选。

### 4.4 附加反例：重复 confirm 幂等拒否（无第二次关门） ✅

批量确认完成、链路终态（stage=completed）后，以批量同型连接重发 `{type:"confirm"}`：

```
{"type":"protocol_error","code":"INVALID_MESSAGE_FOR_STAGE",
 "message":"message confirm not allowed in stage completed",
 "context":{"received":"confirm","stage":"completed"}}   （C·0133 与 D·0134 各一次，原文一致）
```

协议层直接拒否、零副作用。补齐运行窗口语义（代码级）：CAS 关门后、阶段未离开运行窗时迟到 confirm 走 `HUMAN_GATE_ALREADY_CLOSED` 幂等 no-op 分支（`src/product/workspace_engine/conversational_gate.rs:1360-1391`：不产生第二个 HumanGateClosed、不 abort）。两层合证：**任何时点的重复 confirm 都不可能造成第二次关门**；observer 侧 `already_closed_seen=0` 佐证无第二次关门广播。

### 4.5 观察登记（非缺口）：首轮 B 项客户端 8s 超时

首轮 A/B 验证（`/tmp/wp5_bulk_AB_run.log`）：A 客户端视图 confirmed（4281ms）；B 在产品超时 8s 内未收到 `human_gate_closed` 帧（`timeout_after_8000ms`），但**服务端真值确认成功**——durable 全链 Confirmed/completed、human_confirm 恰一个 completed 节点（§4.1 同型证据，已固化）。成因：关门处理含 CAS+候选快照评估+durable 写，两引擎并行时 B 耗时 >8s，帧到达晚于产品 `BULK_CONFIRM_TIMEOUT_MS=8000`。产品行为面：该条目 UI 短暂呈现 failed(timeout)、审计行诚实停留 `sent`（已发送未确认，runner 不虚构 confirmed），门关闭经广播/收件箱刷新即时可见，自愈呈现。第二轮 C/D（6489ms）在限内。**判定：不违反 REQ-CFC-06 任何场景（恰一次/广播/危险动作均成立），登记为客户端等待窗观察项；若后续真实使用中高频出现，可在批量面单独估时加宽超时或改闭门确认为 durable 回查。**

## 5. R-2 全程终检（「断连主嫌已根治」宣称零违规）

**口径**（REQ-WCR-05 / tasks.md 全局边界）：归因日志落地并经一次复现闭环前，任何文案不得宣称「断连主嫌已根治」；热修 af4f7ccf 表述限「消除已证实的前端断连源」；change 文档内形态级「根治」表述可留。归因闭环已于 4.7 完成（`a5-connection-family.md` §4+§8.1：四种 close 唯一归因+durable terminal reason 全过），全程无文案借闭环窗口新增主嫌宣称。

检索命令与命中（2026-09-30，HEAD=3f927393）：

```
grep -rn "根治" web/src --include="*.ts" --include="*.tsx"        → 0 命中
grep -rn "根治" web/ --include="*.html" --include="*.json" --include="*.css"（含 dist bundle）→ 0 命中
grep -rn "断连" web/src --include="*.ts" --include="*.tsx"        → 3 命中，全为代码注释（useCockpitNodeDetailHydration.ts:25、ChatCockpitPage.tsx:871/887），无用户可见文案
grep -rn "根治" openspec/changes/cockpit-fullcourse-connection-resilience/ cadence/reports/cockpit-fullcourse-connection-resilience/ → 17 命中（5 文件）
grep -rn "根治" cadence/designs/2026-09-16_设计文档_3.8驾驶舱全程化与连接韧性_v1.0.md → 12 命中
```

### 5.1 用户可见文案（R-2 主判据）：**零命中，通过**

web/src 源码（含全部 tsx/ts 字符串、注释、测试）与已构建 dist bundle 均无「根治」字样；断连相关字样仅存在于代码注释，不进入任何渲染面。热修 af4f7ccf 本体（`useWorkspaceWs.ts` visibility 保活+门等待静默守卫）纯行为修复、零文案。

### 5.2 change 文档 17 处命中逐条分型（全部「可留」类，无违规）

| # | 位置 | 表述（关键句） | 分型 |
|---|---|---|---|
| 1 | proposal.md:24 | 「RCA §5 根治四条」 | 形态级（机制方案组名） |
| 2 | proposal.md:34 | 「快照门投影终态守卫（…0017 形态根治）」 | 形态级 |
| 3 | proposal.md:36 | 「phase 回门节点（…0429 形态根治…）」 | 形态级 |
| 4 | proposal.md:51 | 「REQ-CG-05（…0429 形态根治；回…）」 | 形态级 |
| 5 | proposal.md:59 | 「归因闭环前文案不得宣称『断连主嫌已根治』（R-2，热修表述=『消除已证实的前端断连源』）」 | 纪律自引用 |
| 6 | design.md:32 | 「0429 恢复链根治并承接 3.7 挂账 C 正向门关闭」 | 形态级 |
| 7 | design.md:62 | 「P3b phase 回门=P2 族内最前（0429 唯一根治…）」 | 形态级 |
| 8 | design.md:92 | 「『顺带根治 M2-5』——M2-5 现场在 coding WS…RCA §5 根治设计覆盖 workspace 面」 | 形态级/范围界定 |
| 9 | design.md:124 | 「口径纪律（R-2）：归因日志落地并经一次复现闭环前，任何文案不得宣称『断连主嫌已根治』——热修 af4f7ccf 表述=『消除已证实的前端断连源』」 | 纪律自引用 |
| 10 | design.md:130 | 「R1｜口径纪律（R-2）：归因闭环前误宣称根治」 | 纪律自引用（风险表） |
| 11 | specs/…/spec.md:69 | 「Scenario: 断连后业务终态不被覆盖（0437 形态根治）」 | 形态级 |
| 12 | specs/…/spec.md:74 | 「Scenario: 已终态 run 不被断连改写（0429 形态根治）」 | 形态级 |
| 13 | specs/…/spec.md:140 | 「此前任何文案 MUST NOT 宣称『断连主嫌已根治』（热修表述限…）」 | 纪律条款本体 |
| 14 | tasks.md:10 | 「R-2 口径纪律：…任何文案不得宣称『断连主嫌已根治』…」 | 纪律自引用 |
| 15 | tasks.md:22 | WP1 关闸记录「R-2 通过（用户可见文案零『根治』，change 文档 5 处均为形态级表述或纪律自引用）」 | 关闸记录（引用当时 5 处；其后 P3b/P2 落地新增 #3/6/7/11/12 等形态级表述，均为本表可留类） |
| 16 | defer-ledger.md:28 | 「0429 形态根由 4.0 phase 回门根治（3c0bff63），测试今日复跑绿」 | 形态级+修复证据引用 |
| 17 | defer-ledger.md:36 | 「task 4.0 = commit 3c0bff63（门开时相位回门节点，四案根治）」 | 形态级 |

### 5.3 设计文档（主契约）12 处命中分型：全部「可留」

均为形态级（「RCA §5 根治四条」×3、「0017 形态根治」、「0429 形态唯一根治」×2、「M2-4 根治」、「顺带根治 workspace 面被动连接盲区」）或纪律条款（§7-R'1、§8.2 口径检查、§8.3-R-2、§5.3-M2-1 注意项）。其中 §1.2-M2-1（:89）对热修的表述完全符合 R-2 限定：「**闭环前不得宣称主嫌已根治，热修表述 =『消除已证实的前端断连源』**」，并明示最终归因依赖 M2-4 归因日志复现闭环（已由 4.7 完成）。

**R-2 终检结论：通过。用户可见文案（含构建产物）零「根治」宣称；文档面 29 处命中无一例把「断连主嫌」表述为已根治的既成事实。**

## 6. 清点表齐备确认（5.2 无沉默项）

5.2 已勾；登记册为 tasks.md 5.2 行内逐项处置（对应设计文档 §5.3「M5-6 清点表」7 项）。逐项复核（处置+理由+落地事实）：

| 项 | 处置 | 落地事实核验 |
|---|---|---|
| 浏览器后台 timer 节流 | P2 重连重订阅架构无害化口径登记（非「不修」） | 4.5/4.7 落地；a5 报告 §1 intensive-throttling 矩阵过（隐藏 11min ping 稳态 60s）✅ |
| vite dev 重载循环 | defer（生产 rust-embed 稳定；开发工件） | 无行动项，理由在册 ✅ |
| L1 toast 活抓未取得 | defer（证据不可得如实留档） | 在册 ✅ |
| mock 工厂缺 `newCommandId` | 修（随 WP2/3 测试迁移同批） | `grep newCommandId web/src` →6 文件 19 命中，已落地 ✅ |
| 0017 hard_error 污染条目 | 文档处置（操作手册注明预期；终态守卫后不再新增） | 1.2 守卫落地+a5 §8.4 1.3 收口（GAP-1 复跑零错）✅ |
| 截图调用冻结吞点击 | defer（测试工件） | 在册 ✅ |
| M1-5 arch-code 反代入口 | 显式 defer + 文档指引 127.0.0.1:4317 直连 | `.codex/skills/prepare-aria-worktree-dev/SKILL.md`+`README.en.md:146` 指引在场 ✅ |

7/7 有处置与理由，无沉默项；4.1 存量处置（defer-ledger 6/6）与 4.7 矩阵/回退读已在 a5 报告 §8.4 收口。**清点表齐备 ✅。**

## 7. 证据索引（/tmp，供复核）

| 文件 | 内容 |
|---|---|
| `/tmp/wp5_gate_setup.mjs` / `/tmp/wp5_gate_resume.mjs` | 双门造场驱动（真实 provider 全链至 human_confirm 停等） |
| `/tmp/wp5_setup_{A,B,C,D}.log` | 四载体全链日志（issue/story/design/prepare/GATE_OPEN 各行） |
| `/tmp/wp5_gate_{A,B,C,D}.json` | 门快照（session/gate_node/stage/status/at） |
| `/tmp/wp5_bulk_confirm.mjs` / `/tmp/wp5_bulk_confirm2.mjs` | 批量确认真实链验证（runner 协议等价复刻；v2=真值断言版） |
| `/tmp/wp5_bulk.log`（固化副本 `/tmp/wp5_bulk_AB_run.log`） | 首轮 A/B 全过程（含 B 超时观察） |
| `/tmp/wp5_bulk2.log` + `/tmp/wp5_bulk_result2.json` | 主证轮 C/D 全过程与断言结果（before/after 快照、observer 帧序、危险反例、重复反例） |
| `/tmp/wp5_dup_probe.mjs` 输出（本文 §4.4 引用） | 重复 confirm 拒否帧原文 |
| durable 面 | `.aria/projects/project_0003/issues/issue_002{4..7}/workspace-sessions/*.json` + `workspace-timelines/*/timeline_nodes.json`（本文引用可直查） |

**WP5 5.3 关闸：三口径全过，报告落盘（未 git add，按任务边界）。**
