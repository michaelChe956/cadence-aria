## 1. 提交 C：AskUserQuestion 结果所有权归 control_request

- [ ] 1.1 先重写失败测试：把 `claude_provider_deduplicates_assistant_then_control_ask_user_question` 改为 `claude_provider_answers_ask_user_question_only_via_control_response`（断言 ChoiceRequest 的 request_id 来自 control_request；aria 不得写 stdin tool_result；由 fixture 输出原生 tool_result 后正常完成），运行 `cargo test -p cadence-aria --lib claude_provider_answers_ask_user_question_only_via_control_response` 确认红
- [ ] 1.2 新增失败测试：`claude_provider_reuses_choice_for_duplicate_control_request_until_native_tool_result`（重复 control_request 复用缓存、只问一次用户、无 aria 注入 tool_result），运行同上确认红
- [ ] 1.3 新增失败测试：`claude_provider_tool_use_without_control_request_is_protocol_error`（只收到 assistant tool_use、收不到 control_request 时按协议错误处理，不注入手工 tool_result），运行同上确认红
- [ ] 1.4 实现：`stream.rs` 移除 assistant tool_use 分支的手工提问与 `write_tool_result`；`mod.rs` 删 `write_tool_result`；`ask_user_question.rs` 删 `ask_user_question_tool_result_content`/`render_answer_value`；`ResolvedAskUserQuestion` 只留 answers；原生 tool_result 到达时**先判 `is_error` 再 `remove` 缓存**并处理协议错误
- [ ] 1.5 更新 fixture 与用例：`claude_ask_user_question_fixture.sh`、`claude_ask_user_question_tool_error_fixture.sh`（等 control_response 后输出原生 tool_result）；重写 `claude_provider_ask_user_question_tool_use_emits_protocol_error_on_bridge_failure`（原语义为 tool_use 路径 + bridge 失败，C 删除后必红）及 `claude_ask_user_question_tool_use_bridge_failure_fixture.sh`（改为无 control_request 的 tool_use → 协议不兼容）；运行 `cargo test -p cadence-aria --lib claude_provider` 全绿
- [ ] 1.6 提交：`fix(claude): let native control flow own AskUserQuestion tool results`

## 2. 提交 B：权限模式映射为合法 wire 值

- [ ] 2.1 先写失败测试（`src/cross_cutting/claude_code_provider/tests/permissions.rs`）：`claude_supervised_permission_mode_maps_to_default`、`claude_auto_permission_mode_uses_default_so_aria_remains_authoritative`、`claude_initial_messages_send_only_valid_permission_modes`（握手 JSON 的 mode 恒为 "default"、永不为 "supervised"），运行 `cargo test -p cadence-aria --lib claude_permission_mode` 确认红
- [ ] 2.2 实现：新增 `permission_mode_for_claude(&ProviderPermissionMode) -> &'static str`，Auto/Supervised → `"default"`；替换初始握手 JSON
- [ ] 2.3 新增 provider 级回归：`claude_auto_mode_routes_permission_request_through_auto_approval_bridge`（普通工具 control_request 在 Auto 下自动批准、不出现面向用户的 PermissionRequest、出现 Auto approval 事件），运行 `cargo test -p cadence-aria --lib claude_provider` 全绿
- [ ] 2.4 提交：`fix(claude): map aria permission policy to valid callback mode`

## 3. 提交 A：所有模式注册 stdio 权限回调

- [ ] 3.1 先写失败测试（`src/cross_cutting/claude_code_provider/tests/args.rs`）：`claude_args_always_include_stdio_permission_prompt`（Auto/Supervised 均含且仅含一次 flag），运行 `cargo test -p cadence-aria --lib claude_args` 确认红
- [ ] 3.2 实现：`build_args` 始终加 `--permission-prompt-tool=stdio`，删除 `mode` 参数；同步改 `args.rs:10,23` 两处 `build_args(...)` 签名，并复核 `tests/process.rs:17`、`tests/streaming.rs:154,225` 的 argv 断言是否受影响
- [ ] 3.3 新增回归：`claude_auto_mode_registers_stdio_and_waits_for_ask_user_question`（argv 含 flag；AskUserQuestion 先出 ChoiceRequest、发送 ChoiceResponse 前无 Completed——锁定「Auto 下 AskUserQuestion 仍等待用户」），运行 `cargo test -p cadence-aria --lib claude_provider` 全绿
- [ ] 3.4 提交：`fix(claude): register stdio permission callback in auto mode`

## 4. 整体验证

- [ ] 4.1 `cargo fmt`、`cargo clippy --workspace --all-targets`、`cargo test --workspace` 全部通过
- [ ] 4.2 真实 claude CLI（2.1.237）smoke：带 `--permission-prompt-tool=stdio` 验证 tool_use→control_request→control_response→tool_result 事件序列，记录输出作为新鲜证据
