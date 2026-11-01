# 交接记录：WIG Autopilot P3 Task 5 真实链现场（软预算交接 v1.1，2026-09-28 本地）

> 前棒快照：`cadence/reports/2026-09-27_交接记录_WIGAutopilot_P3真实链现场_v1.0.md`（先读）。本棒在 v1.0 基础上推进了手工链与 LC 载体铺底；两模式尚未全通，关闸报告/tasks.md 勾选/validate/commit 均未做。服务：aria-dev-v48t（PID 1411548，git 861a9030，端口 4317，`--work-item-plan-single-candidate`）。证据文件：`/tmp/p3_evidence.jsonl`（本棒已记 9 条）。

## 本棒已完成

### 手工模式（project_0003/issue_0003，至 coding 重启门）
- **story**：前棒已 confirmed（0044/`story_spec_0001`）。
- **design**：`design-specs:generate`（REST 建 0045+design_spec_0001）→ `kick.mjs` start_generation（T12：发完即关 WS）→ autodrive 2×choice REST 200 delivered（DEPLOYED_AT 严格校验/前端不本地化）→ `author_confirm` 门 → 人手 `POST /confirm {"confirmed_by":"human-p3-realchain"}` → confirmed（2026-09-27T16:07Z）。
- **模式选择=不自动化**：`GET automation-enrollment` → `null`（无 PUT，Issue1 #2/#3 证据已记）。
- **plan 第一轮（0046，已终止，含真实链发现#1）**：prepare→kick→author/reviewer 完成；reviewer preflight 3 Error（`acceptance_path_not_in_baseline`：WI-001/002/003 AC 引用新建文件不在基线树）→ 委托返修接力被断连竞争取消（`aria-cancellation handler_run_supersede`，本棒双 kick 误操作+autodrive 轮询断连所致）→ F2 孤儿恢复落降级门（phase=generate、无快照）：approve=fail-closed conflict（正确）、feedback=engine error `human gate snapshot is missing`（**观察：降级门缺反馈通路，F2 注释与引擎行为不符**）、start_generation=`WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID` → 人手 abandon 终止（`human-actions abandon` 200 accepted）。
- **plan 第二轮（0047，Confirmed）**：重备 `issue_work_item_plan_0002` → 单次 kick + **常驻 driver WS（`/tmp/drive2.mjs`，模拟真实驾驶舱页面）** → cross_review（reviewer 3 Error 判返修）→ running（**返修接力走通**：author 修订轮+findings 回灌）→ reviewer Pass → `human_confirm` 门 **snapshot=present** → 人手 `human-actions approve`（expected_gate_id=timeline_node_006）200 accepted → compile → **Confirmed+phase completed**（16:21Z）。时间线 6 节点（start/author/reviewer/author/reviewer/gate）。
- **advance**：WS `advance`（driver）→ `advance_completed`，创建 `coding_attempt_52493b8deb38410c8aec48ccf6293658`，worktree `/home/michaelche/workspace/github/naruto/.worktrees/aria-issues/issue_0003`。
- **coding**：`coding_kick.mjs`（coding_hello→start_coding→关 socket）→ pi 真写码：staged `server.js`/`status.html`/`test/status-server.test.js`/`test/status-page.test.js`/`test/status-integration.test.js`；1×危险命令 permission choice REST 200 delivered（rm -rf 探针→Yes）；3 coding units（unit_0001 running）。
- **coder blocked（真实链发现#2）**：coder 误诊 plan defect——称 CHECK-001 `node --test test/` 不可满足（Node 24 目录参数问题）；**实际计划命令是文件参数 `node --test test/status-server.test.js`，人工实测 10/10 通过**（coder 自己跑了目录形式）。人手分诊：coding WS `gate_response`（retry_coding+extra_context 澄清文件参数形式）→ 受理，会话转出 blocked，出现 **`coding_stage_gate_0002`（stage_gate）等 `stage_gate_confirm`**。

### LC enrolled 模式（project_0002/issue_0004，铺底完成、链未起）
- 建 issue：`logical_codebase_id=logical_codebase_30e57115e989b289b90b012414e82583`，`repository_id=repository_7eb291bcf7594cd19a1faef93e145e4e`（alpha 物理仓 ID；925195d0… 是 logical ID，REST 要物理 ID）。
- selection 铺底（GAP-A）：`.aria/projects/project_0002/issues/issue_0004/codebase-selection.json` 改 `explicit`/仅 alpha（925195d0-71af-4901-b34e-f398c50c74c7），建 issue 时 seed=all_members 已覆盖。
- **未做**：story/design 生成确认 → `PUT automation-enrollment`（reviewer=claude_code，author=pi，源=精确版本）→ 自动链（prepare/绑定/派发→驾驶舱门→plan_confirmed info→**重启检查点：通知 Main**→不开 Coding Workspace 后台 advance→Ready→自动首启→coding choice REST→amendment 零 socket→FinalConfirm 等待→人手确认→Completed）。

## 下一棒续跑指令（精确）

1. **手工链续**：coding WS 发 `{"type":"stage_gate_confirm","stage":"coding"}`（参考 `/tmp/gate_resp.mjs` 改消息类型；或 coding_hello→stage_gate_confirm→关 socket）→ coder 重启（注意：retry 已清 coder 对话，extra_context 是否入 prompt 未验证）→ 若再报同款目录形式误诊，再分诊 retry_coding（计划命令本身可满足）→ 3 units 跑完 → review 阶段 choice REST 作答（`/tmp/coding_auto.mjs` 修过 terminal 判定，可复用）→ `FinalConfirm` 人工（WS `final_confirm` 或 REST `execution-plan/confirm`）→ attempt completed + issue Completed。
2. **观察点**：FinalConfirm 等待期「编码执行完成，待最终确认」信息（Task 4 横幅/recent_completion_info 字段、通知去重、计数零增）。
3. **LC 链**：按上面「未做」顺序全链；plan confirmed 后做重启检查点（通知 Main，plan confirmed 且零活跃 run）。
4. **关闸**：15 条映射（tasks.md 行 32 的 #1–#15 两列：不自动化=手工链、单 target 自动化=LC 链）→ 写 `cadence/reports/2026-09-27_进度报告_WIGAutopilot_P3全链关闸_v1.0.md` → 勾 tasks.md §3.4/§4.1–4.3（§4.1/4.2 按 Task 3/4 实测可勾）→ `openspec validate` → 关闸 commit → 删临时脚本。
5. **真实链发现登记**（进关闸报告或 GAP 清单）：#1 SC 返修接力孤儿门缺反馈通路（abandon+re-prepare 脱困可用）；#2 coder 对 Node 24 `node --test <dir>` 的误诊分诊路径可用（retry_coding+stage_gate_confirm）。

## 驱动脚本（/tmp，均本棒实测可用）
`kick.mjs`（start_generation 一脚关）、`kick2/kick3.mjs`（带应答监听）、`drive2.mjs`（**常驻 driver+REST 答题，推荐**）、`autodrive.mjs`（轮询版，接力窗口慎用）、`adv.mjs`（WS advance）、`coding_kick.mjs`（coding 首启一脚关）、`coding_auto.mjs`（coding REST 冷通道答题）、`gate_resp.mjs`（coding 门响应）、`gateinfo.mjs`（会话态快照）。日志：`/tmp/p3_design_autodrive.log`、`/tmp/p3_plan2_kick.log`、`/tmp/p3_plan2_drive.log`、`/tmp/p3_coding_auto.log`、`/tmp/p3_coding_auto2.log`、`/tmp/aria-dev-v48t.log`。

## 纪律提醒
- **绝不全双 kick**（本棒事故根因）；接力/返修窗口保持常驻 driver WS。
- provider 长跑耐心（分段 55s 轮询）；coder 单元 5–10 分钟正常。
- 需重启通知 Main（检查点=plan confirmed 后无活跃 run）；绝对路径；软预算交接按本快照模式。
