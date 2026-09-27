# 交接记录：WIG Autopilot P3 Task 5 真实链现场（软预算交接，2026-09-27 23:5x）

> 本文件是 Task 5 真实链**进行中**的交接快照，不是关闸报告。`cadence/reports/2026-09-27_进度报告_WIGAutopilot_P3全链关闸_v1.0.md` 仍须按计划 Step 5 在两模式真实链实测通过后才创建/回填；§3.4/§4.1–4.3 未勾选。

## 已完成（本棒）

- Task 3（commit `b4f419ad`）：hook TTL 过滤+occurred_at DESC+最近到期单次失效定时器；CockpitShell 三分消费（actionableCount 等价替换告警条/标题/favicon、completionIdentity 候选队列每身份一次 5s 逐条、gate 优先留队）；legacy info 静默基线保持。聚焦 4 文件 73 测全绿+tsc 0 错。
- Task 4（commit `861a9030`）：CodingWorkspacePage 等待横幅「编码执行完成，待最终确认」（完成横幅只在人手 completed 后出现）；`useCodingWorkspaceWs.actions.test.tsx` 补断线重连 `coding_hello.last_seen_node_id` 续读且快照重放不发第二个 start_coding（文档性用例）。Task 4 聚焦 9 文件 117 测全绿+tsc 0 错。
- Task 5 替身定向（全部本次实测绿）：
  - `cargo test --locked --lib recent_completion_info`（4 passed）
  - `cargo test --locked --lib coding_choice`（9 passed）
  - `cargo test --locked --lib p2_campaign`（2 passed）
  - `cargo test --locked --lib coding_start`（14 passed）
  - `cargo test --locked --lib coding_amendment_delivery`（9 passed；含零 socket Unsent→Delivered、慢 observer 不反压）
  - web 聚焦（observers/Shell/Workbench.drawer/CodingWorkspacePage）全绿。
  - 五红线反例均有既有行为测试：`automation_reconcile_ignores_disabled_enrollment`（无 opt-in 不自动）、`automation_p1_crash_windows_keep_one_plan_and_never_reissue_provider`（单 plan/单 start）、`confirmed_enrollment_advances_then_starts_without_coding_socket`（无页面首启）、`p2_campaign_requires_human_final_confirm_after_socketless_run`（人手 FinalConfirm）、`automation_enrollment_http_put_rejects_zero_or_multi_target`+`automation_target_requires_exactly_one_logical_member`（多 target 禁令）。
- 部署：Main 已上 v48t（PID 1411548，git revision `861a9030`，前端 index-O_ehYGjg.js，exe md5 前缀 418b7585，health ok，`recent_completion_info` 字段实测在）。provider 状态：claude_code 2.1.283/codex/pi 全 available。

## 真实链现场

### 手工模式（进行中，未完）
- 载体：`project_0003/issue_0003`（P3 手工模式关闸载体：状态 API 扩展 last_deployed_at；repo=naruto repository_0001 已初始化）。
- 弃用记录：project_0001 新建 issue_0006 后 story 生成 500 `identity_migration_failed`（历史失败迁移 journal，环境级）；project_0004 新建仓初始化卡 pre_check（operation `repository_initialization_8cc37e4afcf94534bbd536c48139279e`，已 cancel）。均写入本记录备查，不算链内失败。
- 已到：story 段**完成**——`workspace_session_0044`（author=pi）5 次 choice REST 200 delivered（分支策略/取值契约/测试落点/前端校验门），GATE_REACHED `author_confirm` 停等，人手 `POST /confirm {"confirmed_by":"human-p3-final-b"}` 后 session=confirmed、`story_spec_0001` confirmed（2026-09-27T15:57Z 前后）。
- 剩余步骤：design-specs:generate 同流程 → 人手 confirm → **模式选择=不自动化（即不 PUT enrollment，直接手工）** → `work-item-plans:prepare` 人工生成/批准/compile（recovery 如触发）→ 人工 advance→StartCoding（`coding_kick.mjs` 或 WS start_coding）→ coding choice REST 作答 → 人手 FinalConfirm → Completed。期间至少关/开一次 driver/observer 页面核对重放。

### LC enrolled 模式（未开始，调研已完）
- 载体方案：`project_0002` 新建 issue 带 `logical_codebase_id="logical_codebase_30e57115e989b289b90b012414e82583"`；`repository_id` 需过 `validate_logical_codebase_primary`（active member 校验；manifest primary=None，取 alpha=925195d0-71af-4901-b34e-f398c50c74c7 试首个 active member）。
- 注意：创建会自动 seed **all_members** selection（alpha+beta 两成员→多 target）；enrollment 前须把 `logical-codebases/{lc}/selections/{issue}.json` 改为 explicit/仅 alpha（P2 棒对 issue_0003 的同款人工铺底，GAP-A 实践，见 `.aria/projects/project_0002/issues/issue_0003/codebase-selection.json` 样例）。
- enrollment：`PUT /api/projects/project_0002/issues/{i}/automation-enrollment`（reviewer=claude_code，author=pi；源=精确 story/design 版本）。
- 链路：编排器自动 prepare/绑定/派发（P2 已实证该段可走）→ 驾驶舱人手 plan choice/门/compile recovery/approve → plan_confirmed info（此时做**重启检查点**：通知 Main 重启，plan confirmed 且零活跃 run，不中断已认领 provider）→ 不开 Coding Workspace 后台独立 advance→Ready→自动单发首启→真实 Claude coding → coding choice REST 完整答案+真回执 → 人工 amendment 零 socket 确认+重连真投递 → readiness+FinalConfirm 等待/info（0 待处理增量）→ 人手 Final Confirm → durable Completed/同 key「已最终确认」。
- 驱动脚本（前棒遗留可用）：/tmp/kick.mjs、/tmp/autodrive.mjs、/tmp/coding_kick.mjs、/tmp/coding_watch.mjs、/tmp/gateinfo.mjs；证据文件 /tmp/p3_evidence.jsonl（新）、/tmp/p2_evidence.jsonl（P2 沿革）。

### 关闸剩余（下一棒）
1. 两模式跑完并逐条记账（Issue 1 #1–#15 两列）。
2. 写 `cadence/reports/2026-09-27_进度报告_WIGAutopilot_P3全链关闸_v1.0.md`（替身与真实清单独立；15 行映射 PASS/FAIL/BLOCKED+日志引用）。
3. 据实勾 `openspec/.../tasks.md` §3.4/§4.1–4.3（任一 required real row 不过则保持未勾+记阻断）；§4.1/§4.2 按本棒 Task 3/4 实测可勾（P3 计划 Step 5 允许据实回填）。
4. 更新 P2 计划 Task 10 的 LC 补证记录。
5. validate+关闸 commit（按计划 Step 5 文件清单）；删临时脚本/凭据。

## 证据
- /tmp/p3_evidence.jsonl（本棒起记录；story kick 已发生）
- /tmp/aria-dev-v48t.log（v48t 服务日志，含诊断流）
- 本文件+git log（b4f419ad、861a9030）
