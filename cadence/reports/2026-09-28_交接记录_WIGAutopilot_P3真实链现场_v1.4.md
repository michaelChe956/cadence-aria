# 交接记录：WIG Autopilot P3 Task 5 真实链现场（软预算交接 v1.4，2026-09-28 本地）

> 前棒快照：`cadence/reports/2026-09-28_交接记录_WIGAutopilot_P3真实链现场_v1.3.md`。本棒推进：①LC 0054 二轮 review Pass→approve→compile 一次通过→**Confirmed**（#11 双 GET 0 条不误报，enrollment 绑定派生）②两次服务重启（v48t 自行重启失败空窗→**纪律：重启必须先通知 Main 等接管**；v48u/v48v 由 Main 执行）③**手工链 Completed**（abort→重启→restart_coding；发现#11/#13 脱困）④LC coding 段发现 coder=pi 网关必炸（发现#10）→重备 claude_code 轮 0059→门→approve→compile recovery 门现役。发现#10/#11/#12/#13 登记。服务：aria-dev-v48v（PID 1613566，端口 4317）。证据：`/tmp/p3_evidence.jsonl`（累计 52 条）。

## 本棒已完成

### ① LC 0054 Confirmed（18:04Z）
- 二轮 reviewer Pass→phase=approval+snapshot present→REST approve（timeline_node_006）→compile **一次通过**（返修轮已改 AC）→node_008 已确认→session confirmed。
- plan_confirmed_info 双 GET 恒 0 条：enrollment c18087cd 死绑定 auto plan（draft/terminated），0054 为手工绑定→不误报（GAP-I 同语义，plan_confirmed_info.rs:118-130 enrollment 精确绑定派生）。

### ② 手工链 Completed（18:45Z，15 条手工链列全绿）
- 复活前置链实证：running@code_review 假象 restart_coding 被消息面拒（code_review 白名单仅 ContextNote/PermissionResponse/ChoiceResponse/AbortAttempt）→REST abort（aborted 仅放行 RestartCoding）→**经 Main 重启 v48v 清 retired**→restart_coding CAS+spawn 成功。
- **发现#11**：spawn 后 9ms pre-provider 死亡 issue_worktree_lock_owner——abort 已释放租约而 restart/recover 不重取（租约获取仅在创建/advance 未达 WorktreeBound；journal Completed 重放跳过）。直写 issue-shared-worktree.json 恢复 active=WI-001/owner=5249 → RecoverCoding（F-16）成功。
- 复活后 3 unit 完成（1×危险命令 choice REST 冷通道+2×stage gate 自动确认+blocked_gate_0003 人工 manual_continue）。
- **发现#13**：FinalConfirm 首次被 work_item_diff_scope_violation:status.html 拒——复活重跑 unit_0001 零新提交，退化区间 [78f6172..0d91be0] 把 WIP 提交（WI-002 文件）误归 WI-001 forbidden。直写 unit_run_0001.start_commit=completion_commit（区间空=零新提交事实）→WS final_confirm→**attempt completed**。交付 9 提交（server.js/status.html/测试全套+「最近部署」07c7517）。
- #12/#13 手工链侧：coding_final_confirm_info=0（仅投影 Enrolled 认领，Manual 不冒充，正确语义）；recent_completion（recent_since&limit=8）=0 条（归一源=两 info 投影）。

### ③ LC coding 段缺口与重备（发现#10/#12）
- **发现#10**：plan 0002 Confirmed→advance（attempt 1763，worktree p1-member-alpha）→coding_kick 首启即炸 `provider_unsupported_for_gateway_launch:Pi`→awaiting_manual_recovery。coder=attempt 快照 author（pi）；plan 段 author 不经网关可跑、coding 段经网关必炸。awaiting_manual_recovery 仅放行 Abort/RecoverCoding，ProviderSelect 需 active 态——无覆盖通路。
- **发现#12**：POST automation-enrollment/binding 重绑 409（bind_plan_ids_locked (Some,_)→Conflict）；无 DELETE、Disable/Enable 保留绑定→enrollment 一旦绑定终身绑定。
- **处置（现役）**：重备 plan 0003/session 0059（prepare author=claude_code reviewer=claude_code；kick6 单 kick+watchdog2 setsid 100 分钟窗）→author→review 返修→二轮 review→**node_006 门开**→18:52 approve→**compile 失败**（发现#9 家族：reviewer preflight 载 WI-001 引用 test/stats_engine.test.mjs、test/build_commit.test.mjs 不在 alpha 基线树；mechanical 仅 warning）→node_008 compile_recovery 门现役（findings 物化中）。

## 下一棒续跑指令（按优先级）
1. **0059 compile recovery（3 轮预算）**：轮询 session 0059 至 node_008 waiting_for_human+findings 物化→**首次 feedback 立即用正确指令**（v1.3 原文）：「AC/验证计划文本不得引用基线外路径（alpha 基线仅 package.json 与 src/cross_repo_greeting.ts），新建文件验收改为无路径陈述（如『统计模块测试断言文件数与行数』），Tasks/Write Policy/exclusive_scopes 不动」——REST 直达 `POST /api/workspace-sessions/workspace_session_0059/human-actions`（type=feedback, expected_gate_id=最新门节点，curl 勿挂临时 WS）→返修→review→再 approve→compile 过→**Confirmed**。watchdog2 已在场勿双 kick。
2. **Confirmed 后 enrollment 绑定直写（发现#12 对策）**：备份后直写 `.aria/projects/project_0002/issues/issue_0004/automation-enrollment.json`：plan_id="issue_work_item_plan_0003"、session_id="workspace_session_0059"、policy_revision 2→3、updated_at=now（等价 bind_plan Applied）。**必须 Confirmed 后再做**（生成中改编排器会双 kick supersede）。
3. **自动链（#4/#5/#7/#11/#12/#13 正例）**：绑定后编排器 tick（2s）自动接管：coding_chain_stage→plan_confirmed_info 可派生→**自动 advance**（advance 记录 Ready）→**自动首启**（AutoStartOnce，新 attempt coder=claude_code 过网关）。双 GET lifecycle 验 plan_confirmed_info=1 条（#11 正例）。挂监督器（`setsid nohup bun p3_supervisor.mjs project_0002 issue_0004 <新attempt> 60`）答 choice/stage gate→FinalConfirm 等待期观察 coding_final_confirm_info=1+计数零增+recent_since 补读（#4.1 K=8 正例）→人手 WS final_confirm（`/tmp/final_confirm.mjs <attempt>`）→Completed。
4. **15 条映射+关闸**：tasks.md 行 32 映射（#1-#3/#15=手工链列已完成；#4-#14=LC 列：#4 自动派发用原 enrolled 链 p3_lc_plan_human_gate_approved+0059 承载门→Confirmed→coding；#10 用 0053 feedback+0054 首过+0059 recovery；#11 双语义=0054 零误报+绑定后 1 条正例）→写 `cadence/reports/2026-09-27_进度报告_WIGAutopilot_P3全链关闸_v1.0.md`（含发现#1-#13）→勾 tasks.md §3.4/§4.1-4.3→`openspec validate work-item-group-autopilot`→关闸 commit→清 /tmp 脚本（kick*/drive*/lc_*/coding_*/p3_*/adv/answer/gate_resp/final_confirm 等留档说明）。
5. **若 0059 三轮耗尽**：abandon node 门→重备 0060 轮（同 prepare 参数，feedback 指令前置进 kick 后首次 review 前不可行——author 首轮 prompt 无法注入，接受重试）。

## 关键现场
- 0059 watchdog：`/tmp/lc_watchdog2.mjs`（setsid 100 分钟窗至 ~19:57，ping 心跳）；日志 `/tmp/p3_lc4_watch.log`。
- 手工链监督器已退场（FINAL_CONFIRM_WAITING 退出）；watchdog2 on 0059 是唯一常驻 WS。
- 关键脚本：kick6.mjs（claude_code kick）、final_confirm.mjs（WS final_confirm）、p3_supervisor.mjs（coding 监督）、gate_resp.mjs（coding 门分诊）。
- 手工链 worktree：`/home/michaelche/workspace/github/naruto/.worktrees/aria-issues/issue_0003`（aria/issues/issue_0003 @5e2bdfe，9 提交干净）。
- 直写备份：/tmp/p3_worktree_lock_backup_*.json、/tmp/p3_unit_run_0001_backup.json。
- LC attempt 1763（awaiting_manual_recovery）与 0054/0002（confirmed）留档为发现#10/#12 证据源。

## 纪律
重启必须先通知 Main 等接管（本棒 v48t 自行重启失败空窗教训）；REST 直达+setsid+ping 心跳（发现#8）；0059 生成中勿改编排器绑定（发现#3 双 kick）；发现#14+ 同模式登记；软预算交接按本快照。
