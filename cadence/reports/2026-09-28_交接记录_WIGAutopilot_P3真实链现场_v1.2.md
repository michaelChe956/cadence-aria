# 交接记录：WIG Autopilot P3 Task 5 真实链现场（软预算交接 v1.2，2026-09-28 本地）

> 前棒快照：`cadence/reports/2026-09-28_交接记录_WIGAutopilot_P3真实链现场_v1.1.md`（先读）。本棒推进：手工链 coding 至 unit_0001 review 循环并遭遇发现#4/#5（attempt 现 running@code_review 复活态）；LC 链 story/design 确认→enrollment→自动 plan 全程跑至人工门（降级门 conflict 待解）。服务：aria-dev-v48t（PID 1411548 持续存活，git 861a9030，端口 4317）。证据：`/tmp/p3_evidence.jsonl`（累计 24 条，本棒 11 条）。

## 本棒已完成

### LC enrolled 链（project_0002/issue_0004）——已到 plan 人工门（降级 conflict 待解）
- **story**：REST 建 story_spec_0001+session_0051 → 铺底 involved+focus=alpha（产品缺口复现，P1 同款直写法）→ kick → 1×choice REST 答（页脚口径）→ author_confirm → 人手 `/confirm human-p3-lc` 200 confirmed（v1）。
- **design**：REST 建 design_spec_0001+session_0052 → 铺底 involved=[alpha]（同款缺口）→ kick → author_confirm → 人手 `/confirm` 200 confirmed（v1）。
- **enrollment**：PUT 200（enrollment_id=c18087cd-5da6-481d-a712-52f91409c226，policy_revision=1，source=story@v1+design@v1 精确版本，author=pi/reviewer=claude_code，单 target alpha，selection_key=p3-lc-fullchain-1）。
- **自动 plan（零人工派发实证）**：编排器自动 prepare `issue_work_item_plan_auto_c18087cd…` + auto session running → author_run(pi) completed → reviewer_run(claude_code 网关) completed → **human_confirm 门（timeline_node_003 active）**。1×危险命令 permission choice REST 冷通道 200 delivered（17:06）。
- **⚠️当前卡点（发现#6，#1 家族复现于 enrolled 自动链）**：auto 会话零 WS 消费者 → 事件队列满（event_seq 3639）→ `degraded-diagnostics.jsonl` 记 `outbound_queue_full`（17:07:22）→ 降级门无 snapshot（timeline_node_details 仅 001/002）→ 人手 approve 两次均 `product_store_conflict: human_gate_close`。lifecycle 里 plan 仍 draft、session waiting_for_human。

### 手工链（project_0003/issue_0003）——attempt 复活 running@code_review，监督在跑
- retry run#2 完成 unit_0001 编码（worktree 3×提交：32362c7/78f6172/7844df0，WI-001 全交付）→ 内审循环：review run_0003 failed（要求修改）→ coder 修复 run_0004 → review run_0005 blocked（CHECK-001 字面命令 `node --test test/` 目录形式 Node v24.17.0 实测 exit 1=计划缺陷；等价证据 22/22+10/10 exit 0 已登记）→ **gate_0002 人手分诊 manual_continue**（quality bypass+context note，理由=接受等价证据）。
- **发现#4**：manual_continue 后 runner 断头——gate_response 重启白名单（runner.rs:282）不含 manual_continue；attach 自动重启（socket/resumption.rs:112）不含 CodeReview 阶段；消息面 Running@CodeReview 禁 StartCoding/RestartCoding/RecoverCoding → 链停摆。
- **脱困尝试与发现#5**：清树（WIP 提交 0d91be0 保留 WI-002 预交付 3 文件+备份 /tmp/p3_wt_backup/，node_modules/lock 移除）→ REST abort 200（aborted）→ WS restart_coding → CAS 转 Running 但 spawn 报 `coding_runner_already_started`——根因=registry `retired_attempts`（abort_attempt 插入、**无清除路径**，insert_cancellable 拒绝）→ **同进程内 abort 过的 attempt 永无法 RestartCoding（F-44 语义被击穿）**。期间 17:16 短暂全 skipped/aborted，**17:21 复查 attempt=running@code_review、unit_0001=running（复活，触发者未定位——第一棒 socket 关闭或引擎异步收尾，需查）**。监督 `/tmp/p3_sup2.log`（p3_supervisor.mjs，40 分钟窗）在跑。

## 下一棒续跑指令（按优先级）
1. **LC 门解困（先做，链最靠近终点）**：给 auto session 常驻 observer/driver WS（参考 drive2.mjs 改 session id=workspace_session_auto_c18087cd-5da6-481d-a712-52f91409c226）让队列消费/引擎恢复；若门快照 materialize→重试 human-actions approve（expected_gate_id=timeline_node_003）；若仍降级无快照→按 P1 既证路径 abandon+re-prepare（编 enrollment 派生 intent 会 fail-closed 需先 Disable 再 Enable 同 selection_key 或走计划重备；P2 3.4 的脱困载体 project_0003/issue_0001 有完整先例）。
2. **LC 后续（门过=compile→Confirmed）**：观察 plan_confirmed_info（15 条 #11）双 GET；**重启检查点**：plan Confirmed 且零活跃 run（同时确认手工链无活跃 run 或已终态）→ 通知 Main → 重启 v48t（命令见下）→ 验证 reconcile 不重派/保持停等 → 后台自动 advance→Ready→自动首启→coding choices REST 冷通道作答（lc_auto_watch.mjs 可复用，注意其 GATE 判定）→ amendment 零 socket → FinalConfirm 等待期记录「待最终确认」info+计数零增（#12/#13）→ 人手 REST `/api/coding-attempts/{aid}/execution-plan/confirm` → issue Completed（#5/#6/#7/#14 收口）。
3. **手工链**：先看 /tmp/p3_sup2.log——若 runner 真活了等 review 自然推进（choice 由监督答）；若复活是假象（无新 run 派发），因发现#5 同进程 Restart 无望，唯一通路=服务重启（清 retired registry）后 attach（CodeReview 不在 attach 自动重启名单→需再 abort→restart；或重启后直接 RestartCoding 若 abort 态）。注意重启会打断 LC 链——务必与 LC 重启检查点合并执行（先 LC plan Confirmed，再统一重启，重启后双链各自恢复）。最终 FinalConfirm 人手 REST 同上；「待最终确认」横幅观察同 LC。
4. **关闸**（两链 Completed 后）：15 条映射（tasks.md 行 32：#1-#3/#15=手工链列、#4-#14=LC 链列为主）→ 写 `cadence/reports/2026-09-27_进度报告_WIGAutopilot_P3全链关闸_v1.0.md`（含发现#1-#6 登记：#1 SC 返修孤儿门缺反馈；#2 coder 误诊 retry_coding 可脱困；#3 勿双 kick；#4 manual_continue runner 断头；#5 retired_attempts 击穿 RestartCoding；#6 enrolled 零消费者 outbound_queue_full 降级门）→ 勾 tasks.md §3.4/§4.1-4.3 → `openspec validate work-item-group-autopilot` → 关闸 commit → 删 /tmp 临时脚本（p3_*.mjs/lc_*.mjs/coding_*.mjs 等本棒脚本，kick/drive2/autodrive/gate_resp/gateinfo 保留到关闸后一并清）。
5. **K=8 外补读观察**（#4.1）：FinalConfirm/Completed 后 GET `/api/issues/issue_0004/lifecycle?project_id=project_0002` 看 recent_completion_info（注意 lifecycle 必须带 project_id 参数，缺参 400）。

## 关键命令
- 服务重启：`cd <worktree> && nohup ./target/debug/aria web --workspace <worktree绝对路径> --host 127.0.0.1 --port 4317 --work-item-plan-single-candidate >> /tmp/aria-dev-v48t.log 2>&1 &`（重启前必通知 Main）。
- lifecycle：`GET /api/issues/{issue_id}/lifecycle?project_id={project_id}`；coding attempt：`GET /api/coding-attempts/{aid}`（pending_gates 按当前 stage 过滤，落盘真相在 `.aria/.../coding-attempts/{aid}/blocked-gates|stage-gates/`）。
- stage_gate 均为 5 秒 TTL 瞬态门（过期=自动继续，非阻塞，勿再当人工门等）。

## 纪律
常驻 driver WS 勿双 kick（发现#3）；provider 长跑分段轮询；重启通知 Main；软预算交接按本快照。
