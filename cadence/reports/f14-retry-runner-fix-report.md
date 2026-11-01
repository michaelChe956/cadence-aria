# F-14 修复报告：blocked gate 放行（retry_coding）后 runner 空转——attempt 僵尸 Running 态 fail-closed

- 工作树：`.worktrees/feat-b-0808-add-monorepo`
- 现场：`coding_attempt_0556a410`（issue_0278，scope=work_item_group，WI-003）
- 结论：**resume 拉起链没有断**；真正断点在被拉起的 runner 于「stage gate 过期 → provider 启动」窗口死亡后，attempt 停留 `running`，attach 侧反复重启注定失败的 runner，形成「stage gate 反复创建过期 + provider 零启动」静默死循环。修复 = runner 失败且 attempt 仍 Running 时 fail-closed 转 `awaiting_manual_recovery`（resumption B 兜底同款语义）。

## 1. 现场证据链（磁盘取证，修正原假设）

原假设「gate_response 处理路径没有重新启动 runner（should_resume_runner_after_gate_response 无消费点）」**不成立**：

- `should_resume_runner_after_gate_response`（`src/web/coding_ws_handler/runner.rs:162`）的消费点在
  `src/web/coding_ws_handler/socket.rs:557-572`：gate_response 处理尾段在 `handle_blocked_gate_response`
  返回 Running 后直接 `spawn_coding_runner(...)`。该链自 f52c454e（2026-07-15）即存在，早于部署二进制
  （target/release/aria，2026-09-18 16:14 UTC 构建）。
- 磁盘时间线证明链路确实点火：
  - `22:15:24.088` gate_response{retry_coding} → admit（`admission_ticket_consumed_at` 写入）→ attempt `running`/`coding`；
  - `22:15:24.273` **stage_gate_0006 创建**（仅比放行晚 185ms）——全库唯一创建点是
    `await_stage_gate`（runner 循环内，`gates.rs:71`），证明 runner 已被拉起并走到 coding gate；
  - `22:15:29.273` gate 0006 过期；此后 attempt `updated_at` 冻结在 22:15:24.088、无 coding_node_0007、
    无 coding_role_run_0006、aria（PID 784706）零子进程——runner 死亡点被夹在
    `runner.rs:328-345`（gate 过期返回 → `provider_for` → `execute_coding_with_commands_outcome` 首行）
    的窄窗口内，且失败只经 WS 发一条瞬时 protocol error，**不留任何持久痕迹**；
  - `22:21:41` 重连 attach → `ensure_runner_for_resumed_attempt` 再次重启 → gate 0007 → 同样死亡；
  - 修复取证期间（22:28）本人以 WS 探针重连同一路径：gate 0008 过期后 92ms 创建 role_run_0006、
    kimi provider 子进程存活并开始编码——同代码同盘面路径可走通，证实 22:15/22:21 的死亡是
    瞬态环境失败（服务 stdout 已丢失，具体错误不可回溯），但**结构缺陷**使其变成死循环。
- 历史背景：driver 侧 60min 硬超时（ws.jsonl 18:44 `Coding 硬超时 3600000ms`）后服务端 role_run_0005
  于 18:51 完成，分诊门拦截 → blocked gate 0001。22:15 人工放行即进入上述循环。

## 2. 根因（一段话）

`run_coding_runner_task_body`（`src/web/coding_ws_handler/runner/task.rs`）对
`execute_start_coding_flow` 的 Err 只做两件事：Blocked/WaitingForHuman 时补发快照，否则发一条
`coding_protocol_error` 事件——attempt 若仍处 `running`（runner 在进入任何受管状态前死亡），
既不转人工恢复也不终结：`running`+`coding`+注册表空 恰好满足 attach 半启动重启判定
（`resumption.rs:108`），于是每次重连都重启一个注定失败的 runner、再开一个 5s stage gate、
再过期、再死亡——「gate 反复创建过期、provider 零启动、无人知晓」；且转人工恢复的 B 兜底
（`resumption.rs:159`）只覆盖「重启失败」，不覆盖「重启成功后 pre-provider 死亡」。

## 3. 修复（coding_ws_handler 面内，最小切口）

`src/web/coding_ws_handler/runner/task.rs`（错误分支，`Aborted`/可恢复态之外）：

- 最新 attempt 为 `running` 时，先 `transition_to_awaiting_manual_recovery(attempt_id,
  "coding_runner_failed_while_running")`（复用既有 store API：同锁清 admission 标记、持久化
  stable reason、幂等），再补发人工恢复态快照，最后保留原 protocol error 事件（fail-visible，
  转换失败仅 warn 不吞错）。
- 效果：`awaiting_manual_recovery` 不在半启动重启集合内 → attach 不再重启（循环打断）；
  状态与 reason 落盘对所有 socket 可见；白名单随之收紧为 abort-only（与 3b 第 2 死点 B 兜底一致）。
- 不触及 attach 侧建 gate 路径与 gate_response→spawn 既有链（观察面/控制流零变化）。

## 4. TDD 与验证

- 红测：`src/web/coding_ws_handler/tests/runner_recovery.rs::
  runner_dying_before_provider_moves_running_attempt_to_manual_recovery`
  ——admitted running/coding attempt + coder 指向未注册 provider（registry 空）复刻「gate 过期后
  runner 死于 provider 启动前」；修前红（`left: Running`），修后绿。断言含：状态/reason/admission
  标记清空、`ensure_runner_for_resumed_attempt` 返回 NotNeeded、注册表零重启、
  `coding_start_failed` protocol error 仍可见。
- 契约重钉：`tests/it_web/web_coding_ws_handler/part_02.rs::
  coding_ws_start_coding_pushes_engine_stage_and_timeline_events` 原断言钉住的正是僵尸行为
  （fixture worktree prepare 确定性失败后停留 Running）——改为轮询 settle 后断言
  `awaiting_manual_recovery` + reason + stage 不变；stage/timeline 事件断言保持。
- 回归：`cargo test --lib coding_ws` 108 绿；`--test it_web web_coding` 159 绿；
  `--lib` 全量 3369 绿；`--test it_web` 全量绿；`cargo fmt` 已过；`cargo clippy --lib -- -D warnings`
  与 `cargo clippy --test it_web -- -D warnings` 零告警。

## 5. 现场保留 attempt 0556a410 验证路径（需重建部署后生效）

修复属服务端代码，须重建并重启 aria（`cargo build --release` → 重启 `aria web ...`）后验证：

1. **该 attempt 已于取证期间自然走通**（22:28 gate 0008 → role_run_0006 → kimi provider 实际在跑，
   现场自愈），故对本 attempt 的直接复验应观察其自然完成或人工 abort；
2. 复现回归路径：任一 attempt 走到 blocked gate → 放行 `retry_coding` → 人为令 provider 启动失败
   （如临时移除 provider CLI/清注册）→ 期望：5s gate 过期后 attempt 落
   `awaiting_manual_recovery`（reason=`coding_runner_failed_while_running`）、WS 收到
   `coding_protocol_error`、反复重连不再新增 stage gate；
3. 若部署前仍有遗留 `running`+`coding` 僵尸 attempt，重连一次即触发修复分支收敛（或直接 abort）。

## 6. Commit

- `fix(coding-ws): F-14 runner pre-provider 死亡 fail-closed 转人工恢复，打断 attach 重连空转`
  （文件：`src/web/coding_ws_handler/runner/task.rs`、`src/web/coding_ws_handler/tests.rs`、
  `src/web/coding_ws_handler/tests/runner_recovery.rs`、`tests/it_web/web_coding_ws_handler/part_02.rs`）
