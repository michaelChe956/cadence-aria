# F-16 修复报告：`awaiting_manual_recovery` 显式恢复通道 + pre-provider 死因可考化

- 工作树：`.worktrees/feat-b-0808-add-monorepo`
- 谱系：F-14（4d097c1d，runner pre-provider 死亡 fail-closed）的伴随面——KimiUpgrade v25 取证
  （`.superpowers/sdd/2026-09-04_计划文档_3.6全量收敛轮_v1.0/stage4-kimi-upgrade-report.md` §3）
  证实 `awaiting_manual_recovery` 态无任何 retry/recover 通道（wire 三探测全拒
  `coding_message_not_allowed`；源码四重一致：socket 状态门 abort-only / 状态机仅可转 Aborted /
  admission 拒 re-admit×2 / HTTP 仅 abort），「人工恢复」名不副实只能 abort。
- 结论：**恢复通道 = 新 wire 动作 `RecoverCoding`，仅人工恢复态放行 → 重走 admission CAS 回
  Running → socket 分支复用 StartCoding 同款 spawn 重启 runner；死因 = eprintln stdout + durable
  chat-entry 尾帧双通道。F-14 fail-closed 语义零回归（一般 admission 对 AMR 依旧拒绝）。**

## 1. 设计取舍：为什么是新动作而不是放行既有 retry_coding

- `gate_response{retry_coding}` 需要一个 open blocked gate（`handle_blocked_gate_response` 以
  gate_id 查 open 集合）；F-16 现场（coding_attempt_0556a410）gate 已过期、无 open gate，放行该
  形态需在 gates.rs 做 gateless 特例——面更大且语义混淆。
- `StartCoding` 是 stage 门控（仅 PrepareContext/ReviewRequest）的「开始编码」语义；恢复是
  **状态门控**的显式人工动作。独立变体使白名单可审计：AMR 分支枚举 `AbortAttempt |
  RecoverCoding`，其余状态对 `RecoverCoding` 一律拒绝（无旁路）。
- 状态机语义选最小面：进入 Running 的唯一通道仍是 admission CAS（`valid_status_transition`
  直达表对 AMR 保持 abort-only，仅补注释）；恢复经 admission.rs 新显式入口
  `recover_attempt_from_manual_recovery` 复用既有 admit（路由/快照/policy 重验）+ ticket CAS
  消费两段校验，不新开任何直达写。

## 2. 状态机四处同步

| 面 | 位置 | 变更 |
|---|---|---|
| WS 状态门 | `socket.rs is_coding_ws_message_allowed`（AMR 分支） | `AbortAttempt \| RecoverCoding`（其余消息照旧拒绝） |
| 状态机直达表 | `attempt.rs valid_status_transition` | **零改动**（AMR→仅 Aborted 不变；仅补注释指明恢复唯一通道=admission CAS） |
| store admission | `admission.rs` | `admit/transition_to_executable_locked` 参数化 `allow_manual_recovery`；新增 `recover_attempt_from_manual_recovery`（单锁内：AMR 源校验 → admit 重验 → ticket CAS 消费 → 清 reason → 重锚 marker）；既有 3 个调用点传 `false`，两处 AMR 硬拒绝（`ATTEMPT_AWAITING_MANUAL_RECOVERY`）对自动路径原样保留 |
| HTTP | app.rs 路由 | **未加**（assignment 标可选；abort 已有 HTTP 面，recover 以 WS wire 为准，登记为后续轮 UI/HTTP 接线项） |

恢复成功副作用：`version += 1`、`admission_ticket_consumed_at = now`（重锚会话）、
`manual_recovery_reason = None`（恢复即结束本次人工恢复会话）。

## 3. socket 循环分支（socket.rs）

`RecoverCoding` 分支（StartCoding 分支旁）：`recover_attempt_from_manual_recovery` →
`spawn_coding_runner`（StartCoding/resumption 同款 spawn 路径）→ 发恢复后 session state 快照。
失败 fail-visible：`coding_recover_failed` protocol error（admission 重验拒绝时错误串直达
wire）。spawn 被注册表拒（已有活 runner）时回 `coding_runner_already_started`。

## 4. 死因可考（KimiUpgrade §3#5 双通道）

- **stdout**：`runner/task.rs` F-14 死亡分支加 aria-cancellation 同款
  `#[cfg(not(test))] eprintln!`（`[aria-runner-death] ... trigger=pre_provider_failure
  project_id={} issue_id={} attempt_id={} reason={} error={}`；测试构建保留 tracing——与
  `provider_stream/cancellation.rs` 先例同款双态）。runner 死亡即使 WS 无人收帧、tty 无落盘，
  服务 stdout 也可回收死因串。
- **durable 尾帧**：新增 `CodingAttemptStore::append_manual_recovery_diagnostic`（System/
  SystemEvent `manual_recovery_transition`，message=`{reason}: {error}`，metadata 含
  reason+failure_detail）落 attempt `chat-entries/`。写入点两处：
  - `runner/task.rs` F-14 死亡分支（**先于**状态转换写入——转换本身失败也留死因证据）；
  - `socket/resumption.rs` B 兜底 `fail_over_to_manual_recovery`（转换成功后写，失败仅 warn）。
- 取舍说明：attempt json 新增 diagnostic 字段方案因需改 `CodingExecutionAttempt` 结构体——
  波及 ~26 个构造点（含并行线在途文件 coding_attempt_repository.rs / builder.rs，冲突面），
  故选 assignment 给出的 chat-entries 选项：零模型改动、UI 聊天时间线可直接展示。

## 5. TDD（红→绿）

红（仅加协议变体脚手架后，通道未实现）：
- `tests.rs::recover_coding_message_allowed_only_in_awaiting_manual_recovery`：红=
  `AwaitingManualRecovery 必须放行显式恢复动作（stage=Coding）`（is_coding_ws_message_allowed=false）。
- `it_web part_02::coding_ws_recover_coding_revives_awaiting_manual_recovery_attempt`（真 WS
  wire 契约）：红= `RecoverCoding must re-admit the attempt and restart the runner (version 2 ->
  2)`——复刻 KimiUpgrade 三探测形态（消息可达但被状态门拒绝、零 durable 副作用）。

绿后断言面：
- wire：发 `recover_coding` → admission CAS（version +1）→ runner 重启 → 同一确定性失败再次
  fail-closed 回 AMR（version 再 +1）、reason 重设；全程无 `coding_message_not_allowed` 帧。
- 状态门：`RecoverCoding` 仅 AMR 放行（Running/Blocked/WaitingForHuman/Created/PrepareContext/
  Aborted 全拒）。
- store（admission_tests.rs +4）：恢复 CAS 全语义（Running/version+1/marker 重锚/reason 清除）；
  非	AMR 源 fail-closed（`attempt_not_awaiting_manual_recovery`）；**F-14 零回归钉**
  （`admit_and_transition_attempt_to_executable` 对 AMR 仍拒 `attempt_awaiting_manual_recovery`）；
  诊断尾帧落盘断言。
- lib 集成（runner_recovery.rs +1）：F-14 死亡 → 状态门放行 → attach 依旧 NotNeeded（F-14 面）→
  恢复 CAS → spawn 重启 → 同款死亡 → AMR + **两条** durable 诊断尾帧（各含原始错误串）+ 第二次
  `coding_start_failed` 仍可见。

## 6. 验证

- `cargo test --lib coding_ws`：111 绿（F-14 基线 108 + 3 新）。
- `cargo test --lib coding_attempt_store::admission`：27 绿（+4 新）。
- `cargo test --lib manual_recovery` / `resumption`：绿（补跑确认 F-14 面零回归）。
- `cargo test --test it_web web_coding`：160 绿（基线 159 + 1 新 wire 契约）。
- `cargo fmt`：过（仅本文件面）。
- clippy：本文件面（coding_ws_handler / coding_attempt_store / coding_models）零 findings；
  全树 `cargo clippy --lib` / `--test it_web` 当前被**并行线在途文件**（workspace_engine
  contract_autorepair/compile_parse 重构，非本 change 文件面）的 6 个编译错挡住——C2T1 已澄清
  非其文件，归属另线；待其落地后全树 clippy 由 Main 收口轮统一跑。
- 前端：`web/src/api/types/coding.ts` 消息联合类型补 `recover_coding`（类型面 parity；UI 恢复
  按钮未接——按预授权「wire 层先行，UI 登记后续」）。

## 7. 现场遗留 attempt 的恢复路径（需重建部署后生效）

对 F-16 现场（0556a410 已 aborted 终态，不适用）；对后续轮任何落入 AMR 的 attempt：
1. WS attach → 观察 `coding_session_state`（status=awaiting_manual_recovery + reason）+
   chat-entries 中 `manual_recovery_transition` 尾帧（死因串）；
2. 排除死因（如 provider 注册/CLI 恢复）后发 `{"type":"recover_coding"}`；
3. 期望：admission 重验通过 → attempt 回 Running（version+1、marker 重锚、reason 清除）→
   runner 重启（5s stage gate 照常）；若重验失败（快照/policy 漂移）→
   `coding_recover_failed` 错误帧，attempt 保持 AMR（fail-closed 不动）。

## 8. Commit

文件（显式）：
- `src/web/coding_ws_handler/protocol.rs`：`RecoverCoding` 变体
- `src/web/coding_ws_handler/socket.rs`：状态门 AMR 分支 + RecoverCoding 循环分支
- `src/web/coding_ws_handler/socket/resumption.rs`：B 兜底补 durable 诊断尾帧
- `src/web/coding_ws_handler/runner/task.rs`：死亡分支 eprintln 双态 + 诊断尾帧
- `src/web/coding_ws_handler/tests.rs`：状态门红绿测试
- `src/web/coding_ws_handler/tests/runner_recovery.rs`：lib 集成红绿测试
- `src/product/coding_attempt_store/admission.rs`：恢复通道（参数化 + recover 入口）
- `src/product/coding_attempt_store/admission_tests.rs`：store 契约测试 ×4
- `src/product/coding_attempt_store/attempt.rs`：直达表注释（零行为改动）
- `src/product/coding_attempt_store/context.rs`：append_manual_recovery_diagnostic
- `tests/it_web/web_coding_ws_handler/part_02.rs`：wire 契约测试
- `web/src/api/types/coding.ts`：TS 协议 parity
- `cadence/reports/f16-recovery-channel-fix-report.md`：本报告
