# Proposal: batch1-coding-autopilot-mainline

## Why

owner 于 2026-10-15 裁定批 1 范围（phase-ledger 12:30/12:50）：起点是 plan 已完成、coding 开始之后；目标是整个 coding 过程纯自动化——让已开始 coding 的 plan 自动推进，需人时通知，全做完后一次人工确认。批 1 主流程七条（owner 18:50 收敛指令确认）：

1. WI 自动往下跑：上游完成，下游自动开始，不等上游最终确认。
2. 下游能读到上游代码（跨仓注入）：coder prompt 带真实上游路径/SHA/契约，沙箱只读挂载真实上游。
3. 遇分诊/choice 页面有清晰卡片可处理、继续：三按钮分诊卡 + 大白话摘要；choice 按题号作答走统一 REST。
4. coder/reviewer 挂起自动重试一次，仍不行停下并通知：reviewer 5 分钟 / coder 10 分钟 / 工具未返回 30 分钟（owner 19:10 裁定 D-3=A）。
5. 需用户处理的事都发通知：durable 事实投影 + 驾驶舱全局消费，跨 plan 可见。
6. 全部完成后一个按钮确认：plan 级聚合最终确认，逐仓调用既有确认，首失败停。
7. 一个页面看到所有 WI：只读 plan 总览 + attempt 状态仪表盘覆盖。

owner 裁定摘要：分诊卡三按钮与验证处理折叠入「技术详情」（13:30）；块 2 D-1~D-5 全部按推荐 A（18:50）；边角料第二档全部暂不做、P2-f 暂停（19:00、19:35）；总览页采用列表+单详情切换（19:45）；批 1 取消双审、接地审查 6 条 concern 吸收进实施（20:45）。范围矛盾经 controller 裁决收敛（X1/X2/X3，见 design.md）。

设计依据（本 change 不复制其内容，只引用）：`cadence/designs/2026-10-15_方案设计_批量1总览_v1.0.md` 及块 1~4 设计文档。

## What Changes

- **跨仓上游交付注入（块 3）**：复用跨 attempt resolver，把上游 owner 的 worktree 路径 + handoff SHA + 契约/能力注入 coder 实际 prompt（fresh/delta/rework/teaching 各路径）；Kimi Auto terminal 修复 bwrap 挂载顺序，tmpfs 后只读挂 authority、仅本仓 target 可写；上游完成发布 handoff 时 refs 为空则从冻结 output_contracts 登记 contracts/capabilities；上游不可读报 operational finding，走新环境阻塞门。
- **分诊与 choice 页面（块 2）**：新门 `coding_operational_blocker`（三动作：再试/说明情况/终止）+ coder 路径 OpenOperationalGate 落门与校验放宽；新动作 `retry_coding_with_context`（补上下文提交即重跑）；分诊卡大白话化（triage_summary 投影），旧门 `coding_output_human_triage` 保持两动作、用新卡展示（X2）；「验证处理」修 evidence_refs 为空不可用 bug 并折叠进二级「高级：验证处理」；choice 帧/快照带 questions、Coding 页改 REST 逐题提交、无 runner 时人话提示（无恢复按钮，X1）。
- **挂起检测（块 4）**：共享 provider stream 加可重置 idle watchdog（reviewer 300s / coder 600s / 工具 in-flight max(角色,1800s)），真实活动重置、choice/permission 人工等待暂停；首次挂起同 provider fresh 重试一次，仍挂起 attempt Blocked + `coder_provider_stalled`/`code_review_provider_stalled` 门 + stalled 通知；不切 provider、不改 transport 3 次预算；挂起门复用块 2 三按钮卡（C5）。
- **平台通知（块 4+1）**：不建通知表，durable 事实投影 `CodingAttentionItemDto`；lifecycle 暴露 `coding_attention_items`，驾驶舱全局消费（不受 watched 限制）、逐 key 提醒、点击按 target+focus 下钻；plan 达到 ready/partial 只聚一条 `plan_final_confirm`。
- **聚合确认（块 1）**：新增无 body `POST …/work-item-plans/{plan_id}/coding-final-confirm`，按 target 顺序逐个调用既有 `handle_final_confirm`（不改引擎），跳过 Completed、首失败停并报告失败仓；GET plan 进度新增 `final_confirm` 四态与 `attention_items`。
- **一页总览（块 1）**：新只读页 `/workbench/…/coding-plans/$planId` 列全部 WI/仓库/attempt 状态/依赖/待办，徽标直达 attempt 详情（focus.gate_id/choice_id）并可返回；仪表盘补 attempt 11 态独立状态条。
- **验证处理入口修复（块 2）**：coder 输出门 evidence_refs 为空时前端回退 `gate_id#0`；`coding_operational_blocker` 加入前后端验证处理资格白名单。
- **主流程包实施口径**：17 个主流程包 × 7 波，全部 glm53-task（anthropic-task 提名 0）；TDD 先红后绿；同波无共享文件才并行；统一验收命令与单仓冒烟见实施计划。

## Capabilities

### New Capabilities

无；全部新增行为并入既有 capability，不拆新 capability。

### Modified Capabilities

- `work-item-group-coding-execution`：新增下游启动不依赖上游最终确认（REQ-GCE-06）、上游完成契约登记（REQ-GCE-07）。
- `per-repo-coding-execution`：新增上游交付注入 prompt 段（REQ-COD-07）、上游不可读走环境阻塞门（REQ-COD-08）。
- `kimi-acp-client-services`：新增终端沙箱只读上游挂载顺序（REQ-KACS-IDLE-RO）。
- `coding-run-operability`：新增分诊卡大白话与动作集（REQ-CRO-09）、`retry_coding_with_context` 动作语义（REQ-CRO-10）、choice 逐题 REST 作答（REQ-CRO-11）、coding 待办事实投影（REQ-CRO-12）。
- `coding-verification-triage`：新增 coder 输出门验证处理入口可用（REQ-CVT-06）。
- `coding-provider-transport-retry`：新增 provider idle 挂起检测与一次自动重试（REQ-PTR-IDLE）。
- `multi-target-group-coding`：新增 plan 级聚合最终确认（REQ-MTG-11）、只读 plan 总览与待办直达（REQ-MTG-12）、仪表盘 attempt 全态状态条（REQ-MTG-13）。
- `web-workbench-autopilot`：新增驾驶舱全局消费 coding 待办（REQ-UI37-19）。

## Non-Goals

owner 已裁定的 10 条不做清单（原文搬入，批 1 全部不做）：

1. P2-f choice「恢复运行」（RecoverChoice/作废重跑/`mark_open_choice_gates_stale`/message_policy 新臂）不做：服务重启后卡在 choice 的 attempt 仅显示「AI 当前没有在运行，这个问题暂时无法回答」，无恢复按钮，走已有手动恢复路径（19:00 暂停、19:35 owner 复确认）。
2. B1-e 聚合确认补齐不做：回执/防重放（command_id/generation）/并发/原子性/跨仓事务/半写补齐均为已知风险，批 1 不处理。
3. B4-a 挂起与通知边界包不做：各 adapter 取消退出屏障、并发工具事件粒度、混合失败预算跨重启、残留 run 接管、落门原子性、legacy/旧持久数据兼容矩阵。
4. B4-b 通知边界包不做：个别 issue 读取失败、后台节流、处理后旧消息点击、多标签/重载重复提醒、已读历史、关闭应用后的 OS 送达、长历史扫描性能。
5. 块 3 固定 SHA 独立快照不做：运行期 owner worktree 注入（方案 A）已定，独立物化快照为第二档。
6. 目录被删自动恢复不做：上游目录被外部删除/继续前进为已知风险，仅走环境阻塞门人工处理。
7. 旧 handoff 补齐不做：存量空 refs/output 历史数据不回填、不迁移；完成登记以新产生的交付验收。
8. plan 生成阶段补全 blocker_rules 不做：D-2 选 A 系统端放宽，不改 plan 生成/编译（`work_item_plan_compiler`、`work_item_contract` 不动）。
9. 批 2 范围不混入：自动化入口（「生成模式：自动化」）、enrollment、多 target 授权、启动前 provider 检查、plan 阶段自动批准、plan 级终止语义全部属批 2/批 3。
10. 不建通知表/ack/已读历史：通知是 durable 事实投影，事实关闭投影即消失；不做离线推送与性能第二档。

## Impact

- 服务端引擎（`coding_workspace_engine`）：gates/blocked_gate 分派、coding/rework 落门分支、plan_defect_routing 校验闭包、provider_stream/provider_retry/provider_failure idle watchdog、cross_attempt_handoff/prompts 上游注入、group_completion 契约登记。
- 模型与存储（`coding_models/gate.rs`、`coding_attempt_store`）：新动作变体、`CodingGateRequired.triage_summary`、choice gate facts 只读取数面。
- Web 服务端（`src/web`）：新 `plan_final_confirm.rs`、`coding_attention_info.rs`、lifecycle/plan_start DTO、POST 路由、WS protocol choice 帧。
- 前端（`web/src`）：分诊卡组件与叙事函数、choice REST 提交 hook、plan 总览页与聚合确认组件、路由 focus 导航、仪表盘 attempt 状态条、驾驶舱全局待办消费。
- 兼容性：全部 DTO/帧字段 additive（serde default）；旧 gate JSON 反序列化 `triage_summary=None` 回落 description；老门不下发新动作、前端按 `available_actions` 渲染退化两按钮；WS `choice_response` 后端处理保留。
- 批 2 接口：契约 `batch1-batch2-interface.md` A1~A6 全部 CONFIRMED；批 2 复用 A1 DTO/target、A4 动作、A5 门，不重新设计。
