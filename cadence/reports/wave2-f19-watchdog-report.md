# Wave2 F-19 修复报告：provider 楔死看门狗 + ledger 登记 + 生成期 Abort 入口

- **缺陷**：F-19（P0 族，cadence/notes/2026-09-19_测试发现登记_阶段4监控.md §F-19）
- **现象链**（v27 / story_session_0459 / codex app-server PID 2735053）：单击「开始生成」→ app-server 子进程拉起 → 前 9 条 skill 探索命令后**完全静默 27min**：CPU 时间采样零增长、出站 TCP 0 条、durable streaming 恒 137 字、`provider_start_ledger` 为空；驱动循环 select 无超时臂，run 任务持引擎锁永挂；运行中页面无任何 中止/终止/放弃 控件（ChatInputBar 仅在 prepare_context/author_confirm 渲染），用户只能换会话或等重启。
- **三件最小修**（B 线=引擎+store+一条前端渲染条件；与 A 线 F-18/F-20 面零交集，ChatCockpitPage 编辑区段经 IRC 与 P0StoryUI 确认不冲突）：

## ① provider start 写 ledger 登记（含 provider/时间戳）

- `ProviderStartLedgerEntry`（work_item_plan_policy/types.rs）新增 `#[serde(default, skip_serializing_if=…)] provider: Option<String>` 与 `started_at: Option<String>`——旧 JSON 与 wire 输出逐字节不变（既有 `session_state_preserves_nonempty_provider_start_ledger` 钉），campaign driver 的规范化只读两冻结字段、自动丢弃新增可选字段。
- `LifecycleStore::claim_provider_start_with_details(session_id, key, provider, started_at)`（lifecycle_store/workspace.rs）：与既有 `claim_provider_start` 同语义（排它锁原子落盘、同 key 幂等），条目带 provider 名（ProviderName serde 文本：`codex`/`pi`/`claude_code`…）与 RFC3339 登记时间。`claim_provider_start` 改为其薄委托。
- `WorkspaceEngine::register_provider_start_in_ledger(role, provider)`（provider_drive.rs）：story/design/workitem 聊天面在 provider 子进程拉起前登记 `workspace_{role}:{session_id}:{n}`（对照 SC 面 `single_candidate_author:{session}:{n}` 先例），成功后镜像进引擎内存（SessionState wire 投影来源）。**范围闸**：`WorkspaceType::WorkItemPlan` 一律跳过——Plan 面（Legacy+SC）ledger 键索引由既有预留路径按 len 派生（routing_scope/SC reserve/human_gate），追加条目会漂移键算术（回归中 `single_candidate_revise_route…` 红测证实并收窄）。
- 失败语义=诊断面 best-effort：登记失败仅 `tracing::warn` 不阻断 run（对照 tool-policy audit 的 fail-closed 是承重契约；此处若 fail-closed 会因 store 抖动杀死正常生成，与观测目的不成比例）。
- 登记位点三处（各一行）：author（provider_drive.rs handle_author_message_with_prompt_mode）、reviewer（review/drive.rs drive_review_session）、revision（review/drive.rs drive_revision_session）——retry/repair 子 run 复用同一 run 语义不重复登记。

## ② 挂起看门狗：拉起后零活动 N 分钟 → 可诊断终态

- `PROVIDER_IDLE_WATCHDOG_TIMEOUT`（provider_drive.rs）：
  - **生产 600s（10min 级）依据**：①实测 codex 楔死静默 27min（F-19 三层证据），10min≈确认时间的 1/3 即转可诊断终态；②流式 provider 正常推理逐 token 产出事件（pi 2 分钟全程流式、codex 命令流实时渲染，见监控 M-02/T5 包），合法静默间隙 ≪10min；③上界对照：`DEFAULT_PROVIDER_TIMEOUT_SECS=3h` 是整 run 总上限，对「子进程活着但不干活」无效（27min 楔死全程在总上限内，即 F-19 成因）；④下界对照：ApprovalBridge `PERMISSION_TIMEOUT=900s`（15min）是人工等待界，看门狗须低于它以先于权限超时保护非权限性楔死；resume-stall 60s 仅覆盖 resume 会话首事件。`#[cfg(test)]` 150ms（先例：PERMISSION_TIMEOUT 900s/30ms、CLAUDE_POLICY_HANDSHAKE_TIMEOUT 60s/200ms）。
- 驱动循环（drive_provider_session select）新增看门狗臂：**事件/命令任一活动即重置**；**等待人工输入挂起**——PermissionRequest 置 `waiting_for_permission`、ChoiceRequest 悬置以 `pending_choice_requests` 非空判据，人工等待由 adapter 侧 PERMISSION_TIMEOUT 与其 PermissionTimeout 事件收口，不算 provider 楔死；应答回达即重置计时。
- 触发处置（对照 handle_permission_timeout 先例）：Abort 命令入会话 kill 链（stub 断言已收到）+ engine cancel（触发 adapter `cancel.cancelled()` → `child.start_kill()`）+ flush 流缓冲 + 失败节点带稳定原因码 `provider_idle_watchdog` + `EngineEvent::Error`（UI 可见）+ `finish_failed_run`。
- **终态语义论证（与 F-16 AMR 协调）**：coding 面转 `awaiting_manual_recovery`（abort-only+RecoverCoding 恢复通道）因 attempt 是一次性 admission 门控资源；story/design 面是**对话式可重跑面**——`finish_failed_run` 落 `Open`+`prepare_context`，用户重新点「开始生成」即恢复（既有 provider-error 路径同款收口，part_08 先例），无需新造 AMR 等价态。诊断留痕=失败节点 summary + Error 事件 + ledger 条目（start 已发生+时间戳）三点交叉。

## ③ story/design 生成期 Abort 入口

- 矩阵面确认：`workspace_ws_handler/protocol.rs` Running/CrossReview/Revision 臂均已放行 `WsInMessage::Abort`（生成期 Abort 协议层本就合法，缺口纯在前端渲染条件）；服务端 `decisions/inbound.rs` Abort 分支→`abort_active_run`→cancel/Abort kill 链既有。
- `ChatCockpitPage.tsx`：ChatInputBar 渲染条件由 `{prepare_context, author_confirm}` 扩至 `{+running, +cross_review, +revision}`——ChatInputBar 的 `BUSY_STAGES` 形态自带「输入禁用+仅中止钮」，中止钮接 `workspaceWs.abort`→`{type:"abort"}`。Legacy 页（ChatWorkspacePageLegacy）本就恒渲染输入条，无需改。

## TDD 证据（红→绿）

| 测试 | 红 | 绿 |
|---|---|---|
| `lifecycle_store::tests::claim_provider_start_with_details_registers_provider_and_timestamp_idempotently`（登记含 provider/时间戳+幂等+旧 JSON 兼容） | E0599 无方法/E0609 无字段 | ✔ |
| `workspace_engine::tests::provider_idle_watchdog::author_provider_start_registers_ledger_entry_with_provider_and_timestamp`（durable+内存镜像，key 形先例） | 同上 | ✔ |
| `…::idle_watchdog_aborts_silent_provider_run_with_diagnosable_failure`（静默 4×窗口→Error 原因码+prepare_context+Open+失败节点+Abort 命令送达 stub） | 无常量编译红 | ✔ |
| `…::idle_watchdog_suspends_while_permission_awaits_human_response`（权限悬置 4×窗口不触发；应答后正常收口 author_confirm） | 无常量编译红 | ✔ |
| `ChatCockpitPage.generation.test.tsx › exposes the abort entry during a running story generation`（running 渲染中止钮→点击调 ws abort） | getByRole 找不到中止钮 | ✔（14/14 文件全绿） |

## 定向回归（全绿）

- `cargo test --lib`：**3411 passed / 0 failed**（含 watchdog cfg(test)=150ms 全局生效下的全部既有测试——无既有测试依赖「事件通道恒开静默>150ms」语义）。
- 分面：workspace_engine:: 1200✔ ｜ lifecycle_store:: 92✔ ｜ workspace_ws_handler:: 118✔（范围闸修正后）｜ 前端 cockpit 面 5 文件 63✔。
- `cargo fmt`（定向 14 文件）+ `cargo clippy --lib -D warnings` 干净。

## 修改文件清单（本次 commit）

- 引擎/store：`src/product/work_item_plan_policy/types.rs`、`src/product/lifecycle_store/workspace.rs`、`src/product/lifecycle_store/human_gate.rs`、`src/product/lifecycle_store/tests.rs`、`src/product/lifecycle_store/workspace_single_candidate.rs`、`src/product/lifecycle_store/tests/human_gate_close_promotion.rs`、`src/product/workspace_engine/provider_drive.rs`、`src/product/workspace_engine/review/drive.rs`、`src/product/workspace_engine/review/routing_scope.rs`、`src/product/workspace_engine/tests/recovery_task7.rs`、`src/product/workspace_engine/tests/single_candidate_prompt/verification_scope.rs`、`src/web/workspace_ws_types/out.rs`（后七者含构造点补 `provider:None, started_at:None` 与 fmt 归位）
- 新增测试：`src/product/workspace_engine/tests/provider_idle_watchdog.rs`（+ tests.rs 挂载）、`src/product/lifecycle_store/tests.rs` 追加 store 级用例、`src/product/workspace_engine/tests.rs` 挂 mod
- 前端：`web/src/pages/ChatCockpitPage.tsx`（渲染条件）、`web/src/pages/ChatCockpitPage.generation.test.tsx`（红测）
- 本报告。

## 遗留与边界

- F-19 的 codex 侧「楔死根因」（app-server 为何静默：零出站=模型请求都没发出）未在本线追——看门狗保证 10min 可诊断脱困，根因属 provider 外部（待 v28 复现观测）。
- F-18（typed abandon 接线）与 F-20（AuthorConfirm 门卡）由 A 线（P0StoryUI/W2dF18Fix）并行修复，本报告不含。
