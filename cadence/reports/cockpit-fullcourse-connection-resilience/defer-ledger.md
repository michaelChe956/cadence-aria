# defer-ledger —— cockpit-fullcourse-connection-resilience

> 登记 Artikel范围：本 change 各任务的 defer/处置账。结构参照 `cadence/reports/workitem-conversational-gate-advance/defer-ledger.md`（范围声明非缺陷关闭、残留如实登记）。
>
> 首批登记：task 4.1 存量卡死 session 处置（REQ-WCR-06）。登记人：A2 子任务（ReviveStale6），2026-09-30。

## 4.1 存量卡死 session 处置登记（2026-09-30）

### 口径（design §D 决策① / REQ-WCR-06）

P2/P3b 落地后**不回写、不修复历史 durable**；处置 = 以新二进制对每个存量 session 做复活验证——快照门 confirm 可达（复活续链）或获得明确可诊断拒绝（真实契约缺口 compile 正确拒 = 产品正确行为）；结果逐个记入本 ledger，不承诺全部复活。

### 前提事实：6 个处置对象已物理消亡（非本任务处置所致）

- **定位结果**：本 worktree `.aria` 数据面全量枚举，session 记录仅 `workspace_session_0001..0070` + 5 个 auto 会话（75 条记录，重建于 2026-09-21 之后）；目录名与文件内容双重搜索 `workspace_session_0166/0167/0168/0199/0277/0429` 均为零命中。
- **消亡原因（在案文档实锤）**：用户于 **2026-09-21 凌晨指令清空全部测试数据面**（298 issue + 55 worktree + 226 audit）供阶段 4 最终验证——`cadence/notes/2026-09-21_交接文档_阶段4收官与最终验证_v3.0.md` §1「数据面干净」与 §4.8 用户裁决第 8 条。6 个 session 的原宿主 issue（0166→issue_0138、0168→issue_0140、0277→issue_0198、0429→issue_0274 等）均在清理范围。
- **时间序**：2026-09-17 P2 关闸抽查时 0166/0429 仍在盘且有实录（`cadence/reports/2026-09-17_进度报告_3.8-P2专项_连接解耦_关闸证据_v1.0.md` §3）→ 09-21 凌晨用户清理 → 之后数据面为全新重建（2026-09-21 E2E 起编号重启）。

### 逐 session 登记

| session | 历史宿主 | 卡死形态（在案定性，最后已知） | 处置动作（产品面） | 结果 | 证据 |
|---|---|---|---|---|---|
| workspace_session_0166 | issue_0138 | **4 号修法卡死族**：codex 多边契约缺口 approval compile 拒后 phase 停 Evaluate 不回门节点（0429 形态家族）。09-17 实录：status=open / single_candidate_phase=failed / timeline_node_003 aborted_by_disconnect / last_active_run_id=run-1 | POST confirm + GET timeline-node-details + WS hello（恢复入口） | **不可复活——记录已消亡**（2026-09-21 用户清理）；产品面唯一可达结果 = 可诊断拒绝：HTTP 404 `workspace_session_not_found`；WS error `workspace session not found: product_store_not_found` + close 1006 | §证据 A1/A2 |
| workspace_session_0167 | 未在案（与 0166/0168 同族同期，3.6 台账 L319「三卡死 session 攒 4 号修法」） | 同 0166（4 号修法卡死族） | 同上 | 同上（404 可诊断拒绝） | §证据 A1/A2 |
| workspace_session_0168 | issue_0140 | 同 0166（4 号修法卡死族；convergence-36 exploratory 运行档案在 `cadence/reports/workitem-conversational-gate-advance/evidence/` 留档） | 同上 | 同上（404 可诊断拒绝） | §证据 A1/A2 |
| workspace_session_0199 | 未在案（pi×重 v5 rep1，0199 形态得名之源，3.6 台账 L309-310） | **plan 真契约缺口门开**：confirm 过 CAS 但 plan 真缺口被 approval compile 正确拒，门开着待修订（fail-closed = 产品正确行为；卡点 = 当版修订回路不收敛） | 同上 | 同上（404 可诊断拒绝）。形态级口径维持：真缺口 compile 正确拒 = 产品正确行为，非缺陷 | §证据 A1/A2 |
| workspace_session_0277 | issue_0198 | **0199 形态 levels 质量族 + driver kill 楔死测试工件**：6 轮修订 + 2 条人工 request-change 流转后终拒（3 项 required_capability_missing），82h 门快照形态正确（3.6 台账 L393、ui-1b-walkthrough README） | 同上 | 同上（404 可诊断拒绝）。driver 楔死面由 4.2/4.3（run manager + lease，driver close 只撤 lease 不楔死 run）承接 | §证据 A1/A2 |
| workspace_session_0429 | issue_0274 | **0429 形态代表（断连中止快照门）**：human-gate 真实业务结局 validation_reject 被 aborted_by_disconnect 覆盖 + 快照门 resumable 渲染但 confirm 被 `WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID` 死拒（RCA §3.2；P3b 验收点名承接） | 同上 | 同上（404 可诊断拒绝）。0429 形态根由 4.0 phase 回门根治（3c0bff63），测试今日复跑绿（§证据 A3） | §证据 A1/A2/A3 |

**登记结论**：6/6 = 终态化，终态原因 = **durable 记录已随 2026-09-21 用户数据面清理物理消亡**（早于本 change 4.0/4.4 落地后的任何处置窗口）；产品面对每个 id 给出明确可诊断拒绝（非静默）。无复活成功例——复活验证对原始记录不可执行，按设计口径「不承诺全部复活」如实登记。原始记录的最后已知状态以 2026-09-17 P2 关闸抽查实录与 3.6/3.8 在案台账为准（本表「卡死形态」列），未做任何回写。

### 形态级承接（原始 session 消亡，三形态修复面由已落地任务验证承接）

| 形态 | 承接 | 今日复跑（HEAD 0db10b20） |
|---|---|---|
| 0429（phase 不回门） | task 4.0 = commit `3c0bff63`（门开时相位回门节点，四案根治） | `conversational_gate_revision_reopens_evaluate_gate_at_approval_after_interruption` **PASS**（cargo test --lib） |
| 0437/close 覆盖 + 并发唯一终态 + 存量零误判 | task 4.4 = commit `af97d5fd`（close 零终态写入） | `workspace_ws_disconnect_during_active_run_writes_no_disconnect_terminal` / `workspace_ws_disconnect_does_not_cancel_active_provider_run` / `workspace_ws_completion_and_close_race_yields_single_terminal` / `workspace_ws_historical_disconnect_marker_is_preserved_not_misjudged` **4/4 PASS**（cargo test --test it_core） |
| 0437 直驱形态原测试 `workspace_ws_close_right_after_gate_opens_does_not_mark_failed` | 已随 legacy 决策面退役（T5/REQ-RET-02 退役留档，tests/it_core/workspace_ws_integration/part_06.rs:899），覆盖由上列 disconnect 零终态测试承接 | —（退役留档在案） |
| 0199（真缺口 compile 正确拒 + 门开修订回路） | REQ-CG-04/05 对话式修订接力（与 0429 同族测试面） | 同 0429 行 PASS |
| 0277（driver kill 楔死） | task 4.2/4.3（session-owned run manager + lease：driver close 只撤 lease/不取消 run；observer 拒写） | P2 关闸报告 §1 用例 1/2/7（62 passed 基线在案） |
| 0017（终态门投影守卫） | task 1.2 **未落地**（A1 契约冲突待用户裁决，见收尾迭代计划 §3-Q5）——0017 不在本 6 session 清单，不影响本登记 | — |

### 证据

**A1 定位与消亡**（worktree `.aria`，2026-09-30）：

```text
$ for s in 0166 0167 0168 0199 0277 0429; do find .aria -type d -name "workspace_session_$s"; done   # → 零输出
$ grep -rl "workspace_session_0429|workspace_session_0166" .aria --include="*.json"                 # → 零输出
$ find .aria -name "workspace_session_*.json" | 编号提取 | sort -n | tail -3                          # → 68 69 70（上限）
# 消亡原因文档：cadence/notes/2026-09-21_交接文档_阶段4收官与最终验证_v3.0.md §1/§4.8
#   「数据面干净（用户 09-21 凌晨清空了 298 issue+55 worktree+226 audit 供最终验证）」
```

**A2 产品面探针**（服务器 aria-dev-v48z，PID 4079867，127.0.0.1:4317，启动参数 `web --workspace <本worktree> --work-item-plan-single-candidate`）：

```text
$ POST /api/workspace-sessions/workspace_session_{0166,0167,0168,0199,0277,0429}/confirm  （body {"confirmed_by":"task4.1-revive-probe"}）
  → 6/6 HTTP 404 {"code":"workspace_session_not_found","message":"workspace session not found"}
$ GET /api/workspace-sessions/workspace_session_0166/timeline-node-details/timeline_node_001
  → HTTP 404 workspace_session_not_found
$ WS hello（恢复入口）ws://127.0.0.1:4317/api/workspace-sessions/workspace_session_0166/ws
  → 首帧 {"type":"error","message":"workspace session not found: product_store_not_found: workspace_session workspace_session_0166"} + close 1006
$ WS hello …/workspace_session_0429/ws → 同型 error + close 1006
# 阳性对照（证明探针链路有效、404 源于 session 层缺失）：
$ POST …/workspace_session_0001/confirm → HTTP 200（confirmed 幂等返回 DTO）
$ WS hello …/workspace_session_0024/ws  → 首帧完整 session_state（含 artifact 渲染）
$ WS hello …/workspace_session_0001/ws  → error "identity_migration_failed project_0001"（独立环境观察：
   project_0001 会话的 WS 上下文加载面问题，层级在 session 解析之后，与 6 session 的 not-found 判定无交集；
   已如实登记供主线跟进，非本任务处置面）
```

**A3 形态承接测试复跑**（2026-09-30，HEAD=0db10b20，`RUST_MIN_STACK=33554432`）：

```text
$ cargo test --locked --lib -- conversational_gate_revision_reopens_evaluate_gate_at_approval_after_interruption
  → 1 passed
$ cargo test --locked --test it_core -- workspace_ws_close_right_after_gate_opens …（退役，0 matched，留档在案）
$ cargo test --locked --test it_core -- workspace_ws_disconnect_during_active_run_writes_no_disconnect_terminal workspace_ws_disconnect_does_not_cancel_active_provider_run
  → 2 passed
$ cargo test --locked --test it_core -- workspace_ws_historical_disconnect_marker workspace_ws_completion_and_close_race
  → 2 passed
```

**A4 当前数据面卡死存量盘点**（现状登记）：

```text
75 条 session 记录：confirmed 33 / open 35 / terminated 5 / waiting_for_human 1 / failed 1
唯一 aborted_by_disconnect 标记持有者 = workspace_session_0015（story，issue_0003/project_0001，2026-09-26）：
  node_003 detail = "last_active_run_id: stale-connection; connection_id: stale-connection"
  → 源码定性：lifecycle.rs::recover_stale_active_run_after_disconnect（进程重启后的 stale run 诚实恢复链，
    REQ-WCR-01 设计内语义），非 0429/0437 病理形态；session 处于 status=open/prepare_context 可重跑态。
waiting_for_human = workspace_session_auto_d7bd2976（work_item_plan 门，09-27）：WS 只读探查正常渲染，
  门路径健康，非卡死。
```

### 红线自查（REQ-WCR-06）

- **无 durable 历史改写**：6 个目标 session 已消亡，无可改写对象亦未回写任何在盘记录；处置全程只走产品入口（REST/WS）。
- **一次意外写入的披露与还原**：阳性对照 `POST …/workspace_session_0001/confirm` 经 store-only 兜底臂追加了一条系统消息（「已由 task4.1-control 确认…」，2026-09-30T04:17:34Z）并 bump updated_at。该写入非处置意图，已字节级还原：删除该条消息（11→10）、updated_at 恢复为探针前读取值 `2026-09-23T15:13:13.366831767+00:00`，序列化格式与同批未触碰文件一致（2 空格缩进/无尾换行/raw UTF-8）。还原前快照存 `/tmp/session_0001_before_restore.json`。此为撤销本任务自身污染，非对历史现场的处置性回写。
- 后续同类探针应用只读端点或幂等 GET 做阳性对照（教训登记）。

## 后续登记位（预留）

- 4.7 关闸「存量处置记录」引用本文件 §4.1。
- 5.1 跨会话批量确认已交付（tasks.md 5.1 [x]），无 defer 项。
