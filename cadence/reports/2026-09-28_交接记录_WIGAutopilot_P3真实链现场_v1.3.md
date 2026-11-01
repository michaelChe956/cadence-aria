# 交接记录：WIG Autopilot P3 Task 5 真实链现场（软预算交接 v1.3，2026-09-28 本地）

> 前棒快照：`cadence/reports/2026-09-28_交接记录_WIGAutopilot_P3真实链现场_v1.2.md`（先读）。本棒推进：①复活触发者查明（第一优先完成）②LC 发现#6 解困：abandon 降级门链→重备三轮（0053 因发现#8/#9 耗尽 repair 预算 abandoned；0054 现役返修接力中）③发现#7/#8/#9 登记。服务：aria-dev-v48t（PID 1411548 持续存活，端口 4317）。证据：`/tmp/p3_evidence.jsonl`（累计 34 条，本棒 7 条）。

## 本棒已完成

### ①复活触发者查明（v1.2 第一优先，已闭合）
- **结论：复活=前棒自己的 WS restart_coding CAS（17:16:10Z aborted→running），spawn 因 retired_attempts（发现#5）被拒 → running 态零 runner 活动=假象复活**；非引擎异步收尾、非第一棒 socket 关闭。
- 证据链：attempt updated_at=17:16:10（CAS 写入时刻）；role-runs 最后活动 coding_role_run_0005 blocked completed 16:55:02Z，17:15 后零新 role-run/unit-run/stage-gate 文件；源码唯一 aborted→running 路径=`coding_attempt_store/admission.rs:59/394` restart_coding（F-44）。
- 手工链恢复确认依赖服务重启（清 retired registry），与 LC 重启检查点合并执行。

### ②LC 链解困（发现#6 降级门 → 三轮重备）
- **发现#6 无害化解尝试失败**：observer WS 挂上（17:24 baseline 送达）但 node_003 快照未 materialize、degraded_exit 未记录；approve 仍 500 `product_store_conflict: human_gate_close`。
- **根因定位（lifecycle_store/workspace.rs:648-682）**：confirm CAS 要求 `single_candidate_phase∈{Approval,Evaluate} 且 human_gate_snapshot.is_some()`；降级 session phase=generate+snapshot=None → approve 永久 fail-closed。
- **abandon 通路（代码证实+实证）**：relaxed_terminate 放行面（phase∈{None,Prepare,Generate,Failed}；approval+snapshot present 分支同样放行）→ human-actions abandon 200 accepted（node_003）。
- **enrollment 通路死路证实**：Disable/Enable 保留 plan/session 绑定（issue_automation_store.rs:558-572）；ensure_plan_binding Unchanged；terminated session admission=NeedsHuman（plan_generation.rs:158）。
- **重备 0053 轮**（manual `work-item-plans:prepare` → issue_work_item_plan_0001）：
  - kick 首轮用旧参数（reviewer:null+reviewer_enabled:false）→ 落盘 reviewer 回退 author=pi（lifecycle.rs:837-840）→ LC 网关 fail-closed `provider_unsupported_for_gateway_launch:Pi`（provider_gateway.rs:138-151）→ node_005 failed（**发现#7**）。
  - Failed 会话显式重臂（lifecycle.rs:764-767）+ kick5（reviewer=claude_code+enabled=true）→ author/reviewer/human_confirm 门快照 present → **approve 200 触发 compile** → 4×`acceptance_path_not_in_baseline`（AC 引用新建测试路径不在基线树）→ 门重开（#10 compile recovery 实证）→ **feedback 200 accepted**（发现#1 家族反例：快照 present 时反馈通路正常）。
  - 三轮 revision run 均被连接断开 supersede 掐断收尾（**发现#8**）；第四轮反馈纠正方向（AC 不引用基线外路径）后 `HUMAN_GATE_BUDGET_EXHAUSTED`（attempts=3）→ abandon 0053。
  - **发现#9**：plan_baseline_tree=纯 git ls-tree 基线分支清单（plan_preflight.rs:131-178），exclusive_scopes 不进基线树——修复提示文案误导；空仓载体（alpha 仅 package.json+src/cross_repo_greeting.ts）AC 引用新路径结构性必炸。
- **重备 0054 轮**（现役）：issue_work_item_plan_0002 + workspace_session_0054（prepare 参数同 enrollment，reviewer=claude_code）→ kick5 参数单 kick → author_run（node_002）→ reviewer_run（node_003，4 Error 判返修）→ **node_004 author 返修接力 active（18:00:48）**。watchdog2（ping 心跳+断线重连+setsid）稳定挂载，gen=1 无断连。

### 本棒新发现登记（#7/#8/#9，同模式续 v1.2 的 #1-#6）
- **#7** start_generation 的 reviewer 回退（`unwrap_or(author)`）对 LC 归属链是隐性毒配置：reviewer_enabled:false+reviewer:null → 落盘 reviewer=pi → Evaluate 轮 LC 网关必炸。对照：P1 手工链 0047 reviewer=pi 直走成功因走 Legacy routing（无 LC gateway）。
- **#8** 临时 driver WS 关闭的 handler_run_supersede 会静默丢弃已完成的 provider 修订产物（provider 输出 16KB+usage 已记但收尾被取消）；另：工具 bash 生命周期会杀未 setsid 的常驻进程（receiver_exit=eof）+ 服务端 ~2 分钟 idle 清理（server_idle）——常驻脚本必须 setsid+ping 心跳（`{type:"ping"}` 全阶段允许）。
- **#9** acceptance_path_not_in_baseline 与空仓/全新文件载体结构性冲突+修复提示文案误导（详见上文②）。

## 下一棒续跑指令（按优先级）
1. **0054 续跑（watchdog 已在场，勿另挂 driver）**：观察 `/tmp/p3_lc3_watch.log`（40 分钟窗至 ~18:38）→ 返修轮（node_004）→ reviewer 二轮 → human_confirm 门（快照应 present）→ **REST approve**（expected_gate_id=最新 human_confirm 节点；curl 直达，勿挂临时 WS——发现#8）→ compile：若 preflight 仍报 acceptance_path（pi 大概率没改对），**首次 feedback 立即用正确指令**：「AC/验证计划文本不得引用基线外路径（alpha 基线仅 package.json 与 src/cross_repo_greeting.ts），新建文件验收改为无路径陈述（如『统计模块测试断言文件数与行数』），Tasks/Write Policy/exclusive_scopes 不动」——3 轮预算内过。compile 过 → **Confirmed**。
2. **Confirmed 后**：plan_confirmed_info 双 GET（15 条 #11，注意 GET `/api/issues/issue_0004/lifecycle?project_id=project_0002`）→ **重启检查点**：确认 0054 零活跃 run、手工链无活跃 run（现 running@code_review 为假象）→ **通知 Main** → 重启 v48t：`cd <worktree> && nohup ./target/debug/aria web --workspace <worktree绝对路径> --host 127.0.0.1 --port 4317 --work-item-plan-single-candidate >> /tmp/aria-dev-v48t.log 2>&1 &`。
3. **重启后 LC coding 段**：lc_auto_watch.mjs 可复用（注意其 autoSess 选择=`_auto_` 前缀或最后一个 session——**0054 不带 _auto_ 前缀，须确认它选到 0054**，必要时手改）→ 编排器自动 advance→Ready→自动首启→coding choices REST 冷通道作答 → amendment 零 socket → FinalConfirm 等待期「待最终确认」info+计数零增（#12/#13）→ 人手 REST `/api/coding-attempts/{aid}/execution-plan/confirm` → issue Completed（#5/#6/#7/#14）。
4. **重启后手工链**：retired registry 已清 → `coding_restart.mjs`（WS restart_coding）应能 spawn → review 循环续跑（choice/门由监督答，gate_resp.mjs 手动分诊如需）→ FinalConfirm 人手 REST 同上 → Completed。worktree 现场：分支 aria/issues/issue_0003 @0d91be0（WIP 保留 WI-002 预交付），备份 /tmp/p3_wt_backup/。
5. **关闸**（两链 Completed 后）：15 条映射（tasks.md 行 32：#1-#3/#15=手工链列、#4-#14=LC 链列为主；#4 自动派发证据用原 enrolled 链 p3_lc_plan_human_gate_approved，门→compile→Confirmed→coding→FinalConfirm 由 0054 承载，#10 approve+feedback+compile recovery 本棒已实证）→ 写 `cadence/reports/2026-09-27_进度报告_WIGAutopilot_P3全链关闸_v1.0.md`（含发现#1-#9）→ 勾 tasks.md §3.4/§4.1-4.3 → `openspec validate work-item-group-autopilot` → 关闸 commit → 清 /tmp 临时脚本（kick*/drive*/lc_*/coding_*/p3_* 等）。
6. **K=8 外补读观察**（#4.1）：FinalConfirm/Completed 后 GET lifecycle 看 recent_completion_info（带 project_id）。

## 关键现场
- 0054 watchdog：`/tmp/lc_watchdog2.mjs`（setsid 运行中，ping 心跳，40 分钟窗）；日志 `/tmp/p3_lc3_watch.log`。**勿双 kick/勿挂第二个 driver**。
- 0053（abandoned）与 auto_c18087cd（terminated）仅作证据源。
- 手工链 attempt coding_attempt_52493b8deb38410c8aec48ccf6293658：running@code_review（假象，updated_at 17:16:10 后无变化）；监督 p3_supervisor.mjs 40 分钟窗已过期。
- 关键脚本：kick5.mjs（带 reviewer kick）、lc_watchdog2.mjs（常驻）、approve/feedback 用 curl REST 直达、coding_restart.mjs（重启后手工链）。

## 纪律
常驻 driver WS 勿双 kick（发现#3）；临时操作一律 REST 直达（发现#8）；常驻脚本 setsid+ping 心跳（发现#8）；重启必须先通知 Main；发现#10+ 同模式登记；软预算交接按本快照。
