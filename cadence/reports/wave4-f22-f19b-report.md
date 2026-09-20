# Wave4 F-22/F-19b 修复报告：choice 悬置等待界 + review 面看门狗

- **缺陷**：F-22（P1，中止后重跑楔死）+ F-19b（P1，看门狗覆盖面）——cadence/notes/2026-09-19_测试发现登记_阶段4监控.md §v28 轮
- **引擎线（本报告）；F-21 plan 门终止 UI 由前端线并行（wave4-f21）**。

## 0. 0482/0483 解剖结论（推翻登记时的两个前提假设）

### 证据链（全部来自落盘工件 + 实况探针）

1. **0482**（story_spec_0003 / pi 重跑 / timeline_node_004）：流式 503 字后最后一条执行事件是
   `call_40df5b9758c041159d1823a6 command started 'ask_user'`——**pi 调用了 ask_user 工具后再无任何事件**。
   流尾原文：「有一处**验收口径歧义**需要用户裁定，按纪律用 `ask_user` 确…」。codex 首跑（node_002）同形：
   流尾「…这属于验收边界，需要用户确认」后 app-server 静默。
2. **0483**（story_spec_0004 / **全新会话，无任何 abort**）：同样楔死在 `ask_user` started、node 永久 active、
   status=running。→ **F-22 的「abort 后状态/ledger 残留」假设被证伪**：楔死与 abort 无关，全新会话同样复现
   （同族=同因，非 F-07 supersede 残留）。真正区分 0476/0477（正常）与 0482/0483（楔死）的变量是
   **provider 是否发起结构化提问**，不是会话历史。
3. **实况探针**（本机 pi 0.85.1，按 daemon argv `--mode rpc -e aria-ask.ts` 拉起，实跑 180s）：
   pi 在 ask_user 工具启动 3ms 后发出
   `{"type":"extension_ui_request","id":"…","method":"select","title":"…","options":["甲","乙"]}`，
   然后静默等待 `extension_ui_response`（探针 180s 超时被杀）。线格式与适配器 `parse_pi_select_request`
   逐字段吻合——**适配器→ChoiceRequest→引擎转发链无缺口**。
4. **v28 二进制确含 F-19 看门狗**：0482 `provider_start_ledger` 有两条带 provider/started_at 的登记
   （该字段正是 3fc22497 引入）。看门狗在二进制内、provider 静默 12–15+ 分钟却零触发 → 反证
   `pending_choice_requests` 非空（permission 面为 auto 且 permission_events=[]）→ **引擎收到了
   ChoiceRequest 并按设计挂起看门狗**。

### 根因（单一，覆盖 F-22 与 F-19b 全部三连楔死）

provider 经结构化提问（pi `ask_user` / codex requestUserInput / claude AskUserQuestion）等待人工应答时：

- 引擎看门狗按 F-19 设计**挂起**（「等待人工不是 provider 楔死」）；
- choice 卡经 router `broadcast → try_send_live_event` 送达是**单次 try_send，无重发无恢复**——
  队列满即标记 degraded 丢弃、连接抖动即丢；丢失后用户永远看不到问题；
- **choice 悬置没有任何超时界**（权限有 adapter 侧 PERMISSION_TIMEOUT=15min 收口，choice 无对称物；
  pi 适配器 `await_pi_choice_response` 只监听命令通道死等）；
- 结果：卡片丢失/无人应答 → run 永久楔死（引擎看门狗挂起 + 适配器死等 + pi 子进程内 ctx.ui.select 挂起），
  时间线冻结在 ask_user started、会话永久 running、UI「零增长零出站」——与实测逐项吻合。

测试会话的「F-19b：story/design 面疑无看门狗覆盖（P0Watchdog 只覆盖 coding_ws）」前提有误：
看门狗自 3fc22497 起就在 workspace（story/design）author/reviewer 驱动面，**但 (a) choice 悬置使其按设计失效
且无界，(b) review 驱动循环（review/drive.rs）确实完全无看门狗臂**——(a)(b) 即本次两修。

## ① F-22/F-19b-a：choice 悬置等待界（author 面 drive_provider_session）

- `PROVIDER_CHOICE_WAIT_TIMEOUT`（provider_drive/watchdog.rs）：**生产 900s（对齐 ApprovalBridge
  PERMISSION_TIMEOUT 人工界）/ test 150ms**。依据：权限悬置由 adapter 超时收口，choice 悬置此前无任何界；
  900s 是既有产品对「等一个人」的显式定价，choice 不应更宽（卡片丢失场景 15min 转可诊断失败严格优于
  永久楔死）。
- 驱动循环新增 `choice_wait_timer` 臂：`pending_choice_requests` **空→非空起算/重置**；清空后再出现则
  重新计时。触发处置与看门狗触发块逐项一致：Abort 入会话 kill 链 + engine cancel + flush 流缓冲 +
  失败节点带稳定原因码 `provider_choice_wait_timeout`（含 pending id 列表）+ `EngineEvent::Error` +
  `finish_failed_run` → story/design 面回 `Open`/`prepare_context` **可重新开始生成**。
- 多 choice 连发语义：首条起算、应答尽清空后再问重新计时——与「一轮提问集 15 分钟内答复」的交互直觉一致。

## ② F-19b-b：review 驱动循环补看门狗臂（review/drive.rs）

- `drive_reviewer_provider_session_once` 此前 select **只有 cancel/命令/事件三臂，零超时**——reviewer
  静默即永久悬置（F-19 修复只覆盖了 author 驱动面）。补齐对照 author 面先例：
  - 零活动看门狗（同 `PROVIDER_IDLE_WATCHDOG_TIMEOUT`）：事件/命令活动即重置；权限悬置
    （`waiting_for_permission`，PermissionRequest 置位/应答清除）与 choice 悬置（pending id 集）期间挂起；
  - choice 等待界（同 ①，pending 集空→非空起算）；
  - 触发处置走 review 面既有收口：Abort + cancel + flush 后返回
    `Failed(ReviewProviderRunFailure::Provider(原因码消息))` → `finish_review_provider_run_failure`
    统一 Error 事件 + 节点失败 + `finish_failed_run`（回 prepare_context 可重跑，同 author 面恢复语义）。

## F-22 恢复语义说明（重跑链路核实）

abort→rerun 引擎链路本身无缺陷：0482 重跑确实正常起跑并流式 2385 字（ws 层每 run `use_run_token`
换新 CancellationToken，旧 cancel 不残留）；重跑「再次楔死」= pi 再次遇到同一验收歧义再次 ask_user，
撞同一无界 choice 悬置。本修后该形态 15min 转可诊断失败回 prepare_context，可无限次重跑（测试 ③ 钉
「失败→重跑→应答→author_confirm」全链）。

## TDD 证据（红→绿，provider_idle_watchdog.rs）

| 测试 | 红 | 绿 |
|---|---|---|
| ③ `choice_wait_timeout_converts_lost_choice_into_diagnosable_rerunnable_failure`（0482/0483 实测形态 stub：TextDelta→ToolCall(ask_user)→ChoiceRequest(ProviderChoice)→死等） | **永久挂死**（生产楔死的确定性复现：看门狗挂起+无界悬置，测试进程零 CPU 需手动终止） | 1.38s：ChoiceRequest 转发 ✔→超界 Error 带码 ✔→回 prepare/Open ✔→节点 summary 带码 ✔→**重跑到 choice→应答→author_confirm** ✔ |
| ③b `choice_response_within_wait_window_completes_run_without_choice_timeout` | （对照绿——证明引擎转发链本来就通，楔死纯在无界悬置） | 窗口内应答正常完成、应答送达 provider ✔、无超界误伤 ✔ |
| ④ `review_idle_watchdog_aborts_silent_reviewer_with_diagnosable_failure` | Elapsed panic（drive_review_session 永挂） | Abort 送达 ✔ + Error 带码 ✔ + 回 prepare_context ✔ |
| ⑤ `review_choice_wait_timeout_converts_lost_reviewer_choice_into_diagnosable_failure` | Elapsed panic | Error 带 `provider_choice_wait_timeout` ✔ + 回 prepare_context ✔ |

既有不回归：F-19 原三项（ledger 登记/静默看门狗/权限挂起）全绿。

## 回归与验证

- `cargo test --lib`：**3417 passed / 0 failed**（3 ignored 为既有）。
- `cargo test --test it_core workspace_ws_integration`：**44 passed / 0 failed**（含既有 choice 卡全链
  ③组——Hello 期间不阻塞、stale 拒绝、abort-after-choice；choice 窗口内应答路径不受影响）。
- `cargo clippy --lib`：零告警；`rustfmt` 四文件归一。
- 实况探针（非 CI 资产，一次性）：`/tmp/pi-ask-probe/probe.mjs` 验证 pi 0.85.1 select 线格式（报告留档，
  探针已弃）。

## 变更面

- `src/product/workspace_engine/provider_drive/watchdog.rs`：+`PROVIDER_CHOICE_WAIT_TIMEOUT`（依据注释含 0482/0483 实测）。
- `src/product/workspace_engine/provider_drive.rs`：导出新常量；drive 循环 +choice_wait_timer 臂与 ChoiceRequest
  空转非空重置。
- `src/product/workspace_engine/review/drive.rs`：review 驱动循环 +看门狗臂 +choice 等待界 +权限/choice
  悬置跟踪（PermissionRequest/Response、ChoiceRequest/Response 四臂置位清除）。
- `src/product/workspace_engine/tests/provider_idle_watchdog.rs`：+AskUserGateStreamingProvider stub 与
  ③/③b/④/⑤ 四测试（392 行）。

## 遗留（不在本修范围，登记备查）

1. **choice 卡送达可靠性**（前端/广播面）：丢卡根因（degraded 丢弃/渲染）未动——等待界把「永久楔死」
   降级为「15min 可诊断失败」，卡片丢失本身仍会发生；后续可做 pending choice 重连重发
   （`pending_author_choice_request_message` 现仅恢复 TextFallback choice，不恢复 provider choice）。
2. WorkItemPlan 驱动循环（`drive_work_item_plan_provider_session_to_output`）无看门狗——F-19 时已论证
   该面键算术独立，未纳入本次范围。
