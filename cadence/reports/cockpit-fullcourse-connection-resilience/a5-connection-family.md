# A5 断连矩阵连接族①②④ + 1.1 四种 close + 1.3 readback 复跑报告

- 日期：2026-09-30
- 服务器：aria-dev-v49a（127.0.0.1:4317，worktree `feat-b-0808-add-monorepo` @ af5653ba，前端 index-DacrUKlO.js）
- 日志基线：`/tmp/aria-dev-v49a.log` 基线行 1138（2026-09-30T06:36:20Z 起为本轮证据窗口）
- 证据脚本（本轮产物，均在 /tmp）：`a5_wslog.mjs`（CDP 常驻采集器）、`a5_ctl.mjs`（CDP 控制器）、`a5_observer.mjs`、`a5_driver_gate.mjs`、`a5_serveridle.mjs`、`a5_tcpdrop.mjs`（前次遗留，本轮复用）
- 证据日志：`/tmp/a5_wslog2.log`（浏览器帧级采集）、`/tmp/a5_observer_0081.log`、`/tmp/a5_observer_0081_c.log`、`/tmp/a5_driver_gate.log`、`/tmp/a5_serveridle_v49a.out`

## 0. 结论速览

| 项 | 结论 | 判定 |
|---|---|---|
| ① intensive-throttling 隐藏 tab | 隐藏 11 分钟，25s ping 被节流至精确 60.000s（±2ms），服务端 90s idle 未误关，run 全程持续（1869+ 流块），无 aborted_by_disconnect | ✅ 通过 |
| ② human-gate 静默 >60s | 门开启后静默 138s（07:25:15→07:27:33）：前端未发自关 4000（线上仅 25s ping），服务端未 idle close；终态=业务结果（review 通过 → confirmed/completed） | ✅ 通过 |
| ④ driver+observer 多连接 | 关 observer：driver ping 25.000s 无中断、run 持续；driver 灾难性死亡（renderer 挂死）：reviewer run 照常完成、服务端在 run 守卫解除后 idle 清死连接并释放 lease、node/HTTP 接管答门至终态 | ✅ 通过 |
| 1.1 四种 close 唯一归因 | server_idle / close_frame 4000 / close_frame 1000（含 1001 变体）/ eof(TCP drop) 各至少一次，诊断字段互斥可辨 | ✅ 通过 |
| 1.3 readback 真实复跑 | durable_session_readback 两轮零错 ✅；stage3_group_snapshot_readback 被 **campaign 驱动脚本自身 bug 确定性阻断**（新缺口 GAP-1，见 §5），产品侧等价 readback 链已用只读脚本复算零错 | ⚠️ 半通过（驱动脚本缺陷已登记） |

## 1. 矩阵①：intensive-throttling 隐藏 tab

**环境**：专属 Chrome 150（headed，Xorg :100），`--enable-features=IntensiveWakeUpThrottling:grace_period_seconds/10`（经 Chromium `third_party/blink/common/features.cc` 确认 feature=`IntensiveWakeUpThrottling`、param=`grace_period_seconds`）。未携带任何 `--disable-background-timer-throttling` 类反制 flag。

**载体**：issue_0011（naruto 状态页 footer 健康摘要行）/ story 会话 workspace_session_0081，author=pi。页面路由 `/workbench/workspace/workspace_session_0081`（driver ws）。CDP 采集器对页面注入 WebSocket 包装器记录全部帧。

**隐藏窗口**：06:48:40.264Z 隐藏（激活另一标签页，`VIS hidden`）→ 06:59:39.176Z 恢复可见（`VIS visible`）。

**ping 实际间隔全表**（`/tmp/a5_wslog2.log` 去重后 29 个 ping）：

```
可见期:      25.000 ×5（06:46:49→06:48:29）
隐藏瞬间:    +10.659（visibilitychange→sendPing）
过渡期:      +14.644, +25.000, +75.000
稳态节流:    60.001 60.001 59.998 60.002 60.000 60.000 60.000 59.998 60.002（06:51:34→06:59:34，连续 9 个 60s）
恢复瞬间:    +4.266（visibilitychange→sendPing）
可见期:      25.000 ×…（恢复后精确 25s）
```

**断言核验**：
- ping 实际间隔：稳态 60.000s（1 wake-up/min 节流生效；grace≈10s 后进入），最大过渡间隔 75s < 90s idle 阈值。
- 服务端不误关：隐藏窗口内 0081 driver 连接零 `[aria-connection-diagnostic]` 记录（该连接自 06:46:24 建立后从未被关闭）。
- run 持续：隐藏期 observer 侧收到 1869 个 stream_chunk（`/tmp/a5_observer_0081.log`：counts stream_chunk=1869），choice_request 2 次实时送达（其一在隐藏期由页面自动应答后 run 继续）。
- 无 aborted_by_disconnect：issue_0011 durable 全目录 grep `aborted_by_disconnect` 零命中；timeline 终态节点 `completed/completed`，唯一 failed 节点（node_002 author_run）失败原因=业务（`choice_wait_timeout`，见 §6 备注），非断连。

**附注（本轮唯一 run 失败，业务归因）**：第一次 author run（06:48 起）在可见期因第二个 choice 无人应答于 07:15:46 触发 `provider_choice_wait_timeout` 取消（服务端日志 `[aria-cancellation] ... choice_wait_timeout`），node_002 标 failed=业务失败；随后重跑成功至终态。此失败发生在隐藏窗口结束 16 分钟后且归因明确为 choice 超时，不影响①结论。

## 2. 矩阵②：human-gate 静默 >60s

**载体**：同 0081。author run 完成后 stage=author_confirm / status=waiting_for_human（07:24:45–07:25:15 之间开门；probe 序列见 bg_10：07:24:45 running → 07:25:15 author_confirm）。

**静默窗口**：07:25:15 → 07:27:33（≥138s，页面可见、零交互；此期间 provider 无任何输出——author 已完成、reviewer 未启动）。

**断言核验**：
- 前端不自发 close 4000：窗口内线上仅有 5 个 25.000s ping（07:25:34/59、07:26:24/49、07:27:14），无 `CLOSE_CALL`/`CLOSED`（代码依据：`useWorkspaceWs.ts` 静默看门狗以 pong 刷新 `lastMessageAtRef`，且 `ACTIVE_PROVIDER_STAGES`/human_confirm 在飞守卫在 run 期抑制；实测 pong 恒流）。
- 服务端不 idle close：窗口内 0081 无任何新诊断行。
- 终态=业务结果：07:28:04 页面「确认并评审」→ cross_review（claude_code reviewer 真跑）→ 07:31:42 review_complete（schema gate 全过、无返修项）→ 二次 author_confirm 门 → 07:38:31 HTTP confirm → **stage=completed / status=confirmed**（story spec 定稿）。全链无断连覆盖。

**门等待期服务端语义补充证据**：无 ping 的静默客户端在门等待期（无 active run）会被 90s idle 关闭——observer#2（未实现 ping）于 07:33:17 被 `server_idle` 关闭（run 结束+90s）。即：②的"服务端不 idle close"依赖客户端 ping 持续（浏览器 driver 恒满足）；守卫（active run）只在 run 期阻止 idle，门等待期不阻止——与代码 `workspace_idle_activity_guard` 语义一致。

## 3. 矩阵④：driver + observer 多连接

**④-a/b 关 observer 零影响**：
- 06:58:44–07:01:29 observer（role=observer）在 author run 进行中订阅（1869 流块）；07:01:29.457 计划性 clean close(1000)。
- 服务端：`07:01:29.457 close_frame code=1000 role=observer provider_drive_depth=1`（run 在场）。
- driver 零影响：observer 关闭前后页面 ping 精确 25.000s 无中断（07:00:59→07:01:24→07:01:49→07:02:14），无重连、无诊断异常、run 继续。

**④-c driver 关闭只影响订阅/lease，run 持续**（意外获得更强形态：driver 灾难性死亡）：
- 07:28:35 导航离开致页面 renderer 挂死（CDP eval 超时；React unmount 未执行、ping 停止、ws 悬挂——即 driver 非 clean 死亡）。
- reviewer run 照常执行：git 检查（07:29:17–07:31:19）、893 流块、07:31:42 review_complete + 业务门开启（observer#2 全程旁观，`/tmp/a5_observer_0081_c.log`）。
- 服务端处置：reviewer 结束后守卫解除，07:33:14.626 对死 driver 连接发 `server_idle` 关闭（`depth=0`），lease-diagnostics 记录 `release from_holder=9d44212e… reason=connection_closed`（07:33:14.626）。
- 接管与终态：07:37:09 node driver（role=driver hello，ping 25s）重连成功；story 门 approve 走 HTTP `POST /api/workspace-sessions/{id}/confirm`（代码注释明确：story/design 门 approve 走 HTTP 端点，Confirm 帧不放行——ws 发 confirm 被拒 `INVALID_MESSAGE_FOR_STAGE` 属预期协议边界）；07:38:31 → **completed/confirmed**。
- 结论：driver 关闭全程只摘除 attachment/撤销 lease（代码 `handle_connection_closed`：绝不取消 run、不写 durable 终态），run 与业务终态零影响。

## 4. 1.1 四种人为 close 唯一归因（v49a 新证据）

| 场景 | 触发方式 | 服务端诊断（`/tmp/aria-dev-v49a.log` 行号） | 归因字段 |
|---|---|---|---|
| server idle | 静默 observer（hello 后零出站）连 0076 | **L2457**：`receiver_exit=server_idle, idle_timeout_triggered=true, close_code=null, role=observer`；客户端 +90.0s 收 close（`/tmp/a5_serveridle_v49a.out`：code=1005 clean） | server_idle+idle=true |
| 前端 4000 | 页面 ws.close(4000,'stale_socket_silence_sim')（completed 会话 0081，07:39:58.160） | `07:39:58.160 close_frame code=4000 reason=stale_socket_silence_sim role=driver` | close_frame+code=4000 |
| 卸载 1000 | SPA 重载 unmount（0081 driver，06:39:44.025） | **L1448**：`close_frame code=1000 role=driver idle=false`；lease-diagnostics 同刻 `release reason=connection_closed` | close_frame+code=1000 |
| （卸载变体 1001） | 浏览器导航关闭 ws（06:39:45.043） | **L1449**：`close_frame code=1001 role=driver` | close_frame+code=1001 |
| TCP drop | 裸 TCP 握手+hello 后 FIN 不发 close 帧（/tmp/a5_tcpdrop.mjs @0076，06:49:12） | **L2286**：`receiver_exit=eof, close_code=null, idle_timeout_triggered=false, role=observer` | eof+idle=false+code=null |

四种归因互斥：server_idle（idle=true）/ close_frame（带 code）/ eof（无 code 且 idle=false），日志可唯一区分。前次 v48z 会话的 TCP drop 证据（receiver_exit=eof/close_code=null/idle=false）与本轮一致。

## 5. 1.3 readback 真实复跑

**运行**（project_0003/repository_0001，ARIA_EXPECTED_FLOW_KIND=single_candidate，ARIA_RUN_POLICY=interactive）：
- rep1（pi，无 advance 脚本）：**completed=true**，session confirmed，`durable_session_readback`（trigger=stage_completed）零错；issue_0010/plan_0001/session_0080。
- rep2/rep3/rep4：pi 产出未过结构化校验（结构化 section 重复/缺字段/IR 引用未知 REQ），campaign fail-closed 终止——provider 内容抖动，非基础设施缺陷。
- rep5（pi，ARIA_HUMAN_SCRIPT="confirm;advance"）：**completed=true**，session confirmed，human_confirm 门脚本 confirm，advance 发出并完成（attempt `coding_attempt_8a639836…`，workspace_entry 发放），`durable_session_readback` 零错；issue_0015/session_0088。

**GAP-1（新缺口，已登记）：campaign 驱动脚本 stage3 group snapshot readback 确定性失败**
- 现象：rep5 `ws.jsonl` 记 `stage3_group_snapshot_readback_failed`（无 error 字段=evidence null），任何一轮均无法产出 readback evidence。
- 根因：`workitem_run_campaign.mjs` 的 `requestJson`（L270-278，经 `parseResponse` L255-266）**返回解析后的 body 对象本身**；而 `stage3GroupSnapshotReadback` 内（L762-765）取 `snapshotResponse.body` → 恒 `undefined` → spread 为 `{}` → `snapshot.attempt` 缺失 → evidence 恒 null。非竞态（durable 写入全部先于 advance_completed 发射：attempt .763/units .780-.784/advance-record .816 vs 消息 .817，本地时间 14:52:59）。
- 掩蔽：`stage3_campaign_driver.test.mjs` L790-793 的 mock `requestJson` 返回 `{ body: … }`，与真实实现形状不一致，单测全绿掩盖生产路径。
- 影响面：仅诊断脚本（cadence/reports/workitem-coding-campaign/），产品服务端无关。
- 修复建议（2 行）：调用点 spread `...snapshotResponse`（或 requestJson 包 `{body}`，二选一并同步修 mock）。
- **产品侧等价验证（零错）**：对 rep5 的 attempt 以只读脚本复算 readback 链——REST 快照 200（attempt 匹配、3 个 binding revision）、durable advance record（status=ready、attempt 绑定、command_id 一致）、issue shared worktree lock owner=attempt、binding digest `3330292a…`——全部一致零错。即：readback 链路（产品面）健康，阻断点仅在驱动脚本取值 bug。

## 6. 其他登记

- **GAP-2（轻，观察项）**：门等待期无 ping 的 observer 会被 90s idle 关闭（07:33:17 observer#2）。属设计语义（守卫只保 active run），但对"静默观察者"类工具脚本需自带 ping——已在 observer 脚本中补齐。非缺陷，记录备忘。
- **过程性说明**：06:41–06:45 期间 0081 出现 3 次 driver `server_idle` 循环，系本轮采集器首版注入的 WebSocket 包装器丢失静态常量（`WebSocket.OPEN` 等）致应用层 ping 被抑制——**仪器 bug，非产品缺陷**；修复包装器并重载后 ping 立即恢复精确 25.000s。该时段证据不用于结论（但客观上再次验证了 server_idle 归因路径）。
- rep5 的 advance 产物 coding_attempt_8a63…：attempt created / WI-001 标 running 但 role_runs 为空（无 provider 启动，advance 不启 coding runner），无残留活进程，无清理负担。
- 服务器未重启（全程 aria-dev-v49a 单实例）；本轮全部写操作经产品 API/WS，零直写 durable。

## 7. 复现命令索引

```sh
# server idle（90s 静默）
node /tmp/a5_serveridle.mjs workspace_session_0076        # → /tmp/a5_serveridle_v49a.out
# TCP drop（握手+hello+FIN 无 close 帧）
node /tmp/a5_tcpdrop.mjs workspace_session_0076
# 归因行
grep aria-connection-diagnostic /tmp/aria-dev-v49a.log     # L1448/L1449/L2286/L2457 及 07:39:58 close_frame 4000
# 隐藏窗口 ping 间隔表
grep "SENT workspace_session_0081 .*ping" /tmp/a5_wslog2.log
# readback 复跑
cd <worktree> && ARIA_PROJECT_ID=project_0003 ARIA_REPOSITORY_ID=repository_0001 \
  ARIA_EXPECTED_FLOW_KIND=single_candidate ARIA_RUN_POLICY=interactive \
  node cadence/reports/workitem-coding-campaign/workitem_run_campaign.mjs pi 5 /tmp/a5_campaign2
```
