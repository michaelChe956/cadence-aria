# Task 1.2 实施报告：outbound turn/advance 事件族与 stage 准入矩阵

## 状态

DONE

## BASE / HEAD commit

- BASE：`0acda84ed25d810c3995c0e2c98afe59c390dab5`
- HEAD：`5f8ebac71abfbe816dbc45bdb84dfb27b107509b`
- Commit message：`feat(ws): define conversational gate and advance events`

## 改动文件清单

- `src/web/workspace_ws_types/out.rs`：新增精确形状的七个 outbound variant，并沿用既有 tagged snake_case serde。
- `src/web/workspace_ws_types/tests.rs`：注册 outbound 事件测试模块，并保留旧 protocol error fixture 测试。
- `src/web/workspace_ws_types/tests/conversational_gate_events.rs`：新增七个事件的 type 前缀、关键字段、无 markdown 内联及 serde roundtrip 断言。
- `src/web/workspace_ws_handler/protocol.rs`：新增按 `(flow_kind, stage, message)` 判定的准入 helper；SC 仅在 HumanConfirm 接受 HumanGateFeedback、Confirm、HumanConfirm(Terminate)，SC Advance 允许 Completed；legacy 矩阵保持原分支；新增 stage-specific ProtocolError 与 AdvanceRejected 构造 helper。
- `src/web/workspace_ws_handler/socket.rs`：真实 socket stage 白名单改用 flow-aware helper；新 feedback 使用 `WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID`，advance 前置 stage 拒绝使用 `advance_rejected`。
- `src/web/workspace_ws_handler/decisions/inbound.rs`：dispatch boundary 先校验 command_id，再校验 flow/stage；合法命令暂返回未接线占位错误，不启动 provider、不写 store。
- `src/web/workspace_ws_handler/tests.rs`：注册 stage 准入测试模块。
- `src/web/workspace_ws_handler/tests/conversational_gate_protocol.rs`：新增穿过 dispatch 的 blank command_id 回归测试。
- `src/web/workspace_ws_handler/tests/conversational_gate_stage.rs`：新增 SC/legacy/stage 矩阵与零副作用协议错误测试。
- `src/web/workspace_ws_handler/tests/single_candidate_scope_rejection.rs`：仅将既有测试 fixture helper 调整为同级测试模块可复用。

## 测试证据

### 失败测试先行

- `cargo test --locked --lib workspace_ws_types::tests::conversational_gate_events -- --list` 首次在新增测试后匹配 1 项，并因七个 variant 尚未实现而 RED（编译错误逐项报告 variant 缺失）。

### 目标过滤集（均先 list 且 N >= 1）

- `cargo test --locked --lib workspace_ws_types::tests::conversational_gate_events -- --list`：1 项；随后运行通过，`1 passed`。
- `cargo test --locked --lib workspace_ws_handler::tests::conversational_gate_stage -- --list`：2 项；随后运行通过，`2 passed`。
- `cargo test --locked --lib workspace_ws_handler::tests::conversational_gate_protocol -- --list`：3 项；随后运行通过，`3 passed`。
- `cargo test --locked --lib workspace_ws_types::tests::outbound -- --list`：1 项（既有 protocol error fixture）；随后运行通过，`1 passed`。

### 其他验证

- `cargo test --locked`：通过，`400 passed; 0 failed; 12 ignored`；集成测试与 doc-tests 也通过。
- `cargo fmt --check`：通过。
- `cargo check --locked`：通过。
- `cargo clippy --all-targets --all-features --locked -- -D warnings`：通过。
- `git diff --check`：通过。
- `rg -n 'HumanConfirmDecision::RequestChange|WorkItemDraftDecision' src`：通过；legacy 枚举与分支仍存在。
- 追加写盘后 `wc -l`：`conversational_gate_events.rs` 118 行、`conversational_gate_stage.rs` 90 行、`conversational_gate_protocol.rs` 89 行。

## 自审

- [x] 七个 outbound Rust variant 名称、字段及 wire type 前缀与 brief 完全一致。
- [x] payload 仅携带引用/原因字段，事件测试断言不含 `markdown`。
- [x] flow-aware stage 矩阵不依赖 node title/ID；SC feedback/Confirm/Terminate 仅 HumanConfirm，SC Advance 仅 Completed，legacy RequestChange/旧 HumanConfirm 分支保持。
- [x] Running/CrossReview/Revision/Completed 对 SC feedback 统一拒绝，错误码为 `WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID`。
- [x] Advance stage 前置拒绝编码为 `WsOutMessage::AdvanceRejected`，而非普通 Error。
- [x] 真实 socket 路径已使用新矩阵，Task 1.1 两个 inbound variant 不再被既有白名单提前拒绝（合法 stage 进入 dispatch）。
- [x] dispatch boundary 覆盖空串/空白 command_id；新增测试经 `handle_workspace_inbound_message` 断言 `INVALID_COMMAND_ID`。
- [x] 本任务未新增 durable 写入或 provider 启动；合法命令仍为未接线占位。
- [x] 未删除或重解释 legacy 消息/枚举。
- [x] 未使用 `-j` 参数。

## 遗留 / concerns

- `HumanGateFeedback` 与 `Advance` 的业务 durable service、provider turn、AdvanceRecord/group 初始化仍属于后续 Task 2.x/5.x；本任务只实现协议事件、准入和占位 dispatch。
- `cargo test --locked --lib workspace_ws_types::tests::outbound` 原过滤名在本仓库当前测试命名下只匹配 outbound protocol fixture 1 项，已先 list 确认非零并通过；未扩大过滤语义。

## Acceptance evidence

- changed-files：上述 10 个代码/测试文件及本报告。
- tests-added：七事件 roundtrip；SC/legacy/stage matrix；stage-specific protocol error；dispatch blank command_id regression。
- commands-run：见“测试证据”。
- residual-risks：仅后续 durable turn/advance 业务实现未接线，符合 Task 1.2 范围。
- no-staged-files：代码与报告提交后 `git status --short` 为空。
