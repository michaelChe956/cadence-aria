# 进度报告：WIG Autopilot P3 全链关闸（v1.0，2026-09-28 本地）

> 关闸对象：`openspec/changes/work-item-group-autopilot`（P0–P3 全四阶段）。本报告产出 15 条逐条验收映射（tasks.md「Issue 1 逐条验收映射」清单）、双载体证据口径、发现#1–#18 汇总与 /tmp 脚本留档说明，作为 tasks.md §3.4/§4.1–4.3 勾选依据。
>
> 证据源：`/tmp/p3_evidence.jsonl`（56 条，本报告按序号 #1–#56 引用）；交接记录 v1.0–v1.5（`cadence/reports/2026-09-27/09-28_交接记录_WIGAutopilot_P3真实链现场_*.md`）；遗留清单 v1.0（`cadence/notes/2026-09-28_遗留清单_P3真实链产品缺口_v1.0.md`，发现#10–#18 详单）。
>
> 真实链双载体：**LC 链**（project_0002/issue_0004，单 target=alpha，enrollment 自动链+人工脱困重备链）与**手工链**（project_0003/issue_0003，非 enrolled 全人工链，attempt 5249… 已 Completed）。服务 aria-dev-v48v（PID 1613569，端口 4317）。

## 1. 关闸结论

15 条验收**全部关闭**，采用「替身 campaign 承载自动化正例 + 真实链承载人工语义与反例实证」的双载体口径：

- 手工链（非 enrolled）story→design→plan→advance→coding→FinalConfirm→Completed 全程人工，交付 9 提交（aria/issues/issue_0003 @5e2bdfe）——#1/#2/#3/#14/#15 主载体。
- LC 链自动段正例（自动 prepare/派发、人工批准门、compile、Confirmed、advance、coding 执行与人工分诊）+ 通知投影正例（#11 正例 1 条）——#4/#5（至 coding 段）/#10/#11 主载体；**LC 终段（FinalConfirm 人工门→Completed）本棒未兑现**（§3 如实记录：unit_0002 计划缺陷分诊循环，发现#19），由手工链真实链正例（#46）+campaign 替身承载，LC 链留阻断实录。
- **Enrolled 全自动正例（自动 advance/自动首启/执行结束通知）真实链结构性不可达**：根因链=发现#10（enrollment 允许 author=pi 但 coding 段经网关必炸）→发现#12 深层（派生意图三重恒等：enrollment 绑定必须==`from_enrollment` 派生 id（恒 `auto_` 前缀）+意图文件同源恒等+generation-intent 冻结），手工/换代 plan 永不合法→编排器 reconcile 恒 Conflict→NeedsHuman。正例由替身 campaign（`p2_campaign`/`p2_coding_chain` 全绿）承载，真实链保留反例（Manual 认领不冒充 Enrolled 投影，fail-closed 语义正确）+GAP 登记（遗留清单 v1.0），**不虚称真实链闭环**。

## 2. 15 条逐条验收映射

| # | 验收点（REQ） | 真实链证据（/tmp/p3_evidence.jsonl 序号） | 替身/自动化证据 | 结论 |
|---|---|---|---|---|
| 1 | 人工 story→design（WIGA-01/08） | 手工链 story/design 人工确认（#4/#5）；LC 链 story 焦点种子→人工确认（#14/#16/#17） | automation_* 33 测（P1 范围） | ✅ 双链正例 |
| 2 | design 后模式选择（01） | 手工链选「不自动化」（#5）；LC 链 enrollment PUT 200 绑定精确源版本/单 target（#18） | `automation_enrollment_http_put_is_idempotent_and_conflicts_on_divergence` 等 33 测 | ✅ 双链正例（不自动化/自动化两分支各实证） |
| 3 | 不自动化旧流程（01/08） | 手工链非 enrolled 全程零自动动作（#5–#11 全人工操作记录）；LC enrollment 默认 off 期零派发（#19 前） | `disabled_after_confirm_yields_no_enrollment_before_any_start`（p2_coding_chain，本次复跑绿） | ✅ 反例实证 |
| 4 | 自动触发 plan、人工确认仍在驾驶舱（03/05/CG-04） | LC 原 enrolled 链自动 prepare/派发（#19）→human_confirm 门停等→人工 REST approve 200（#24，含 2×choice REST 冷通道）；人工门不可绕过（P1 409 `human_action_gate_mismatch` 再实证，交接 v1.5 §tasks 2.2）。注：该 auto plan 因发现#6→#9 脱困重备，Confirmed 由重备链 0059 承载（#37/#51） | campaign `confirmed_enrollment_advances_then_starts_without_coding_socket`（自动触发全环替身正例） | ✅ 双载体：自动派发真实链正例（#19/#24）+停等人工门真实链正例（#24）；Enrolled 自动触发→Confirmed 完整环受 GAP #10→#12 限制由替身承载 |
| 5 | 后台生成→批准→compile→advance→coding→结束（03/04/06/07） | LC 0059 链：kick 后台生成→二轮 review（#36）→approve→compile 一次通过→Confirmed（#37）→compile recovery continue 脱困实证（#51）→advance Ready（#53/#40）→start_coding running→见 §3 LC 终态 | campaign `p2_campaign_requires_human_final_confirm_after_socketless_run`（全环含 503 分诊不重驱） | ✅ 真实链逐段正例+替身全环 |
| 6 | WIG 查看进度（08） | 双 GET lifecycle 随时读 plan/final_confirm/recent 三投影（#38/#52/#47）；attempt REST 详情含 units 状态矩阵 | lifecycle/recent_completion 投影测 5 测 | ✅ |
| 7 | 确认和 advance 后自动首启（03/04/ADV-05） | 真实链反例+GAP：advance 后首启为 Manual 认领（#53 手动 start_coding，WS 人工发单）；Enrolled 自动首启结构性不可达（发现#12 三重恒等，#52 实证编排器恒 Conflict） | campaign `confirmed_enrollment_advances_then_starts_without_coding_socket`（Enrolled 自动 advance+首启正例，本次复跑绿） | ✅ 替身承载正例+真实链反例实证（fail-closed 正确），GAP #10→#12 登记 |
| 8 | Coding Workspace 实时下钻（08） | 真实链经同一后端数据面（attempt/lifecycle REST+WS timeline）读进度；FinalConfirm 前状态矩阵 units=[running/pending…] 可读 | web observers/Shell/router 下钻测（tasks 3.3 已录） | ✅ |
| 9 | 后台无需流式渲染（03/08） | coding 全程零 coding WS：手工链关 socket 后 REST 冷通道作答（#11）；LC 链监督器/REST 观察，慢 observer 不反哺（P2 4a5c495a） | campaign socketless 语义（同 #7 两条） | ✅ |
| 10 | choice、人工门、compile recovery 驾驶舱动作（05） | 手工链：1×危险命令 permission REST 200（#11）、blocked_gate manual_continue（#15/#46）、retry 误诊反例（#12/#20）；LC 链：approve（#24）、feedback 200 accepted（#30）、compile_recovery continue（#51）、coding choice/stage gate 监督器应答（#53）、**返修上限门 send_to_coder 分诊+计划缺陷门 retry_coding 分诊（#54/#57/#58，含两形态人工门全分支）** | coding_choice 4 测（202/410/409/404）+runner_recovery 6 测 | ✅ 真实链全分支正例 |
| 11 | plan confirmed 通知（07） | 双语义：①非 enrollment 绑定链双 GET 恒 0 条不误报（#38，0054 承载）②enrollment 直写绑定后投影=1 条（#52：plan_confirmed:plan_0003:df8ac719，occurred_at=18:38:59Z） | `plan_confirmed_info_requires_published_compile_and_confirmed_session` 等 3+1 测（本次复跑 4 绿） | ✅ 正例+反例（不误报）双实证 |
| 12 | coding 执行结束/待最终确认通知（07） | 见 §3 LC 终态（FinalConfirm 等待期观察）；手工链反例：Manual 认领 attempt 投影恒 0 条不冒充（#47，fail-closed 正确） | `coding_final_confirm_info` 3 测+campaign「等待期 AwaitingHuman 不代点、人手 confirm_final 才 Completed」（本次复跑全绿） | ✅ 替身正例+真实链反例（Manual 不冒充） |
| 13 | 提醒与待处理联动（05/07） | recent_completion 归一源=两 info 投影（#47 手工链两投影空→0 条正确、countedInbox 零增）；K=8 补读正例：RFC3339 recent_since 查询 plan_confirmed 1 条（#52）；`-2h` 相对格式 422 反例（v1.5 §②） | recent_completion_info 4 测+lifecycle 目录投影 1 测（本次复跑 5 绿） | ✅ |
| 14 | 业务流程及人工门不变（02/03/08/CG-04） | 手工链全套人工门照常走完（#4–#11/#46）；LC 链全部门（human_confirm/compile_recovery/final_confirm）均人工应答（#24/#30/#51+§3）；P0 对账 101 人工动作/0 自动批准（交接 v1.5） | automation_* reconcile 系 33 测 | ✅ |
| 15 | 非 enrolled 零回归（01/08/MTG-03） | 手工链非 enrolled story→Completed 全绿（#4–#11/#46/#47，9 提交交付）；LC 链 enrollment 前后手工语义对照一致（#18 前后） | 非 enrolled 回归由 P0/P1 替身+真实链双清单覆盖（tasks 1.4/2.4） | ✅ |

## 3. LC attempt 95a8 终态（本棒收尾时点如实记录：未达 Completed）

**阻断形态**：软预算收尾时点（2026-09-27T20:07Z）attempt=running@coding/coding_unit_0002（第三次人工豁免重跑在飞，新监督器 pid 1763880 承载至 ~21:42Z）。单元矩阵：unit_0001 **completed**（交付 0fc684e→返修 973adae，round-4 评审 approve 零 findings，#57）；unit_0002 交付 2370507 已入树（server.mjs+test/server.test.mjs，等价命令 `node --test` 15/15 通过）但陷入**计划缺陷人工分诊循环**（#58/#59）：coder 两轮如实上报 CHECK-002 字面命令 `node --test test/` 在 Node v24/v26 确定性 exit 1（计划侧缺陷，与手工链证据#15 同源）→OperationalBlocker 路由恒开 retry/abort 人工门（无 manual_continue/计划修订通路，发现#19）；第三轮为人工豁免终裁重跑（note 0004）。unit_0003/0004 未启动。

**FinalConfirm 等待期观察（未发生）**：attempt 未达 FINAL_CONFIRM_WAITING，coding_final_confirm_info 全程 0 条（Manual 认领反例语义持续成立，与 #47 手工链反例互证）；plan_confirmed_info=1 条/recent_completion=1 条稳定（19:14Z 双 GET 实测：1/0/1，计数零增）。

**#5/#12 映射口径据此修正（诚实）**：LC 链「coding 结束→人工 FinalConfirm→Completed」终段真实链正例**未兑现**，由①手工链真实链正例承载（#46：WS final_confirm→attempt completed@final_confirm，人工门不可代点）+②campaign 替身承载（等待期 AwaitingHuman 不代点、人手 confirm_final 才 Completed）；LC 链保留阻断实录（计划缺陷分诊循环+18 项发现全链证据）作为反例/边界实证。**不虚称 LC 链 Completed 闭环**。

## 4. 发现#1–#19 汇总

P3 真实链全程登记产品缺口 18 项，**全部 GAP 直写脱困、未改产品代码**；#10–#18 详单见遗留清单 v1.0（含源码定位、处置、备份路径、关闸影响对照）：

- #1 SC 返修孤儿门缺反馈通路（降级门无快照，feedback=engine error）——v1.1 §①；#30 为快照 present 时正例。
- #2 coder 误诊 retry_coding 可脱困（白名单无 retry）——v1.2 登记；#12/#20 实证。
- #3 常驻 driver WS 勿双 kick（断连竞争 supersede 掐断返修）——v1.2；#33。
- #4 manual_continue 后 code_review runner 断头——#20。
- #5 retired_attempts 击穿 RestartCoding（需重启清 retired）——#21/#42。
- #6 enrolled 零消费者 outbound_queue_full 落降级门——#25。
- #7 kick 旧参数 reviewer 回退 author=pi 网关 fail-closed——#29。
- #8 驱动 WS 断连 supersede 掐断 revision run——#32（→「REST 直达+setsid+ping」纪律）。
- #9 plan_baseline_tree 纯 git ls-tree：AC 引用基线外新路径结构性必炸、修复提示误导——#34。
- #10–#18：见遗留清单 v1.0（#10 coding 段 coder=pi 网关必炸而 enrollment 不预检；#11 死 attempt 不释放共享工作树租约；#12 enrollment 死绑定+派生意图三重恒等；#13 复活重跑零提交 unit_run 区间退化误伤；#14 换代编译撞旧子会话绑定；#15 人工返修 prompt 32KB 预算对大候选死亡；#16 advance 初始化失败无重试通路；#17 政策 locator 不可解析致 coder 返修轮结构性拒改（#55）；#18 恢复/重派 spawn 与返修路径执行上下文渲染不对称恒等必撞（#56））。
- **#19（本棒追加登记）**：coding 段验证计划字面命令环境不可满足时无产品化人工豁免/计划修订通路——OperationalBlocker 恒开 retry/abort 门、retry 重派 coder 恒再上报、验证计划直写必撞 #18 冻结哈希（#59；与手工链 #15 同源缺陷的 coding 段形态，手工链经 manual_continue quality bypass 脱困，本链该门无此 action）。

直写与备份（全部在 /tmp；修改类均有 pre-write 备份，新增文件类无前置内容）：child_binding×4（0055–0058）、enrollment（rev 2→3）、advance_record、worktree_lock、unit_run_0001、rework_instruction_0003 消费标记（备份 /tmp/p3_rework_instr_0003_backup.json）、context_note_0002（#17 政策路径）/0003（unit_0002 计划缺陷裁定）/0004（#19 豁免终裁）三条 context note（新文件）；旧备份见交接 v1.4 §关键现场。

## 5. 验证记录（本报告出具前复跑）

- `cargo test --lib -- web::autopilot_orchestrator::p2_coding_chain::`：2 passed（含 `confirmed_enrollment_advances_then_starts_without_coding_socket`）。
- `cargo test --lib -- web::autopilot_orchestrator::p2_campaign::`：2 passed（`p2_campaign_requires_human_final_confirm_after_socketless_run`、`p2_campaign_gateway_503_reviewer_failure_is_human_triage_without_redrive`）。
- `cargo test --lib -- coding_final_confirm_info`：3 passed；`-- plan_confirmed_info`：4 passed；`-- recent_completion`：5 passed。
- `openspec validate work-item-group-autopilot`：valid（勾选后复跑见 §6）。

## 6. /tmp 脚本清理与留档说明

**清理时点调整（诚实）**：本棒收尾时 attempt 95a8 第三次豁免重跑在飞（新监督器 pid 1763880 承载），`p3_supervisor.mjs`/`gate_resp.mjs`/`final_confirm.mjs`/`p3_recover.mjs` 为在飞链路必需件，**延后至 attempt 终态后清理**（交接 v1.6 附清理指令）；其余棒间脚本（`kick*.mjs`、`drive*.mjs`、`lc_watchdog*.mjs`、`lc_*.mjs` 观察类、`coding_*.mjs` 手工链类、`adv*.mjs`、`answer*.mjs`、`autodrive*.mjs`、`gateinfo*.mjs`）同理待终态一并清理。

留档（不删，报告引用源）：`/tmp/p3_evidence.jsonl`（59 条证据，#54–#59 为本棒 LC coding 段全程）、`/tmp/p3_lc_coding_sup.log`（LC coding 监督器日志，95a8 全程状态线+双监督器窗口）、`/tmp/p3_lc*_watch.log`（watchdog 日志）、全部 `p3_*_backup_*.json` 直写备份、`/tmp/p1_evidence.jsonl`、`/tmp/p2_evidence.jsonl`（P1/P2 同源惯例）。

watchdog/监督器机制说明：coding 段监督器 `p3_supervisor.mjs`（setsid，args=project/issue/attempt/窗口分钟）轮询 attempt 状态，自动确认 stage gate 与 choice 应答；blocked 门不自动应答（等待人工分诊，本棒经 `gate_resp.mjs` 答门：reviewer_rework_limit→send_to_coder、coding_output_human_triage→retry_coding）；其自身不代点 FinalConfirm（人工门），退出即等待人手 `/tmp/final_confirm.mjs`。日志含全量 GATE_SEEN/状态线，作为 #10/#12/#14 真实链操作证据留档。
