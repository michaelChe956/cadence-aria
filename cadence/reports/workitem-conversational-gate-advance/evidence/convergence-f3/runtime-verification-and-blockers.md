# F3 运行验证证据与阻断诊断（2026-09-06，controller）

## F3 运行面实证（跨 3 次 pi 跑+1 次 kimi 跑）

- 真实策略会话共 34 个（session_0160:6 / 0161:6 / 0163:22；pi=33，role=work_item_splitter×16+reviewer×17 均衡）
- durable 分区 .aria/tool-policy-run-audit/ 全部不变量保持：每文件 provider_start 恰首行、schema_version=1、seq=0 起、role 用真实 AdapterRole、dialect=pi-rpc、provider_session_id 齐备、argv 逐字=冻结值 [`--mode rpc -e <ask-ext> --session-id <uuid> --exclude-tools edit,write`]
- 高水位 role-run-seq 分配跨 22 run 连续（0..21）无重复；进程重启后 monotonic（跨 4 个 session 验证）
- 引擎全阶段健康：作者轮/评审轮/revision 循环（0163 达 7 轮 review-revision）/human gate 前全部 completed；策略角色全程零写面逃逸（评审产出 findings、作者产出计划，无文件写）
- 手工复跑 reviewer argv：pi CLI 正常启动（exit 0，extension UI 请求流正常）——CLI/argv 因果排除

## 3 连跑阻断根因（两次独立诊断，均非 F3 回归）

### 阻断 1：auto_if_valid 流 × 服务器 idle-timeout（run1b 735s/run1c 515s 同签名）
- driver 在 auto 流仅发 hello+start_generation 后全程静默（ws.jsonl 出站仅 2 条）
- 服务器 spawn_idle_timeout_task（socket.rs:40-58，timeout=test_controls.server_idle_timeout()，tick=5s）在「客户端消息静默超时 && !is_active_run()」时经 ws_sender.close() 发空 close frame→undici 报 code=1005 wasClean:true（与观测逐字吻合）
- review 段不计入 current_run 注册（is_active_run=false），第二轮 review 启动时计时器可触发；interactive 流因 driver 持续发消息刷新 last_seen 而免疫（3.5 完整案例 10/10 全 interactive）
- F3 diff 未触碰 socket.rs/idle/gate-close 任何代码（git diff --stat src/web/ 仅 +1/+5 行 sink 接线）

### 阻断 2：interactive 流 driver 命令竞态（run1d 2072s）
- 7 轮 review-revision 全部完成、到达 human gate confirm；driver 于 08:09:30.266 同毫秒并发发送 confirm+human_gate_feedback → product_store_conflict: human_gate_close → session 0163 aborted（timeline node_016 human_confirm failed/017 aborted_by_disconnect）
- 属 F7 观察项族（3.6 计划 F7 首条「confirm 失败上抛…stage_gate 口径」）与 driver 测量脚本竞态，非 F3 面

### 判别与排除记录
- kimi 判别跑死于已知 F2 族（作者空交付 compile error，155s），对 WS 问题判别力作废但未引入新形态
- 消息尺寸/流量排除（最大 29.6KB，总 1.65MB/9722 条，node ws 默认限制内）；服务器零 panic/零 error 日志；无子进程泄漏

## 原始现场
- 失败跑 result/ws.jsonl：/tmp/aria-f3-20260906/{run1,run1b,run1c,run1d,kimi-discrim}/
- 服务器日志：/tmp/aria-f3-deploy.log（PID 3949323 存活，health ok）
- 审计分区快照：.aria/tool-policy-run-audit/workspace_session_016{0,1,3}/（git-ignored，磁盘持久）
