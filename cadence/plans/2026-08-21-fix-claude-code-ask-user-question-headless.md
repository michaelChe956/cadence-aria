# fix-claude-code-ask-user-question-headless 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:test-driven-development 逐任务执行。每步先写失败测试、跑红、再实现、跑绿、提交。步骤使用 checkbox（`- [ ]`）跟踪。

**Goal:** 修复 claude code headless 下 AskUserQuestion 未注册（A）+ 权限模式非法值（B）+ 重复 tool_result（C），恢复 Auto 模式结构化提问。

**Architecture:** 三处改动集中在 `claude_code_provider`：C 让 control_request 成为 AskUserQuestion 唯一回答入口；B 把 aria 权限模式映射为 claude `default`；A 始终注册 stdio 权限回调。

**Tech Stack:** Rust（package `cadence-aria`）；测试命令 `cargo test -p cadence-aria --lib <filter>`；🔴 禁止 `-j 1`。

**Spec:** `openspec/changes/fix-claude-code-ask-user-question-headless/`（proposal/specs/design/tasks，已获批）。

## Global Constraints

- 三提交顺序 C→B→A，每个提交独立可回归，最后一个提交才把完整链路暴露给默认 Auto。
- 已确认决策：Auto/Supervised 都映射 claude `default`；wire 值用 `default`；结果所有权单一化。
- ApprovalBridge 决策逻辑不改；text_fallback 不动。
- 命令统一 `cargo test -p cadence-aria --lib`；整体 `cargo test --workspace`。

---

### Task 1（提交 C）：AskUserQuestion 结果所有权归 control_request

**Files:**
- Modify: `src/cross_cutting/claude_code_provider/stream.rs`、`src/cross_cutting/claude_code_provider/mod.rs`、`src/cross_cutting/claude_code_provider/ask_user_question.rs`
- Test: `src/cross_cutting/claude_code_provider/tests/ask_user_question.rs`
- Fixture: `tests/fixtures/provider/claude_ask_user_question_fixture.sh`、`claude_ask_user_question_tool_error_fixture.sh`、`claude_ask_user_question_tool_use_bridge_failure_fixture.sh`

**关键现状（file:line 以实际为准）：**
- `stream.rs` assistant `tool_use(AskUserQuestion)` 分支当前会 `bridge.request_choice` → 缓存 → `ClaudeCodeProvider::write_tool_result(&stdin, ...)`（须删除手工注入）。
- `control_request(can_use_tool, AskUserQuestion)` 分支已会复用缓存并 `write_choice_control_response`（保留）。
- `mod.rs::write_tool_result` 拼 `{"type":"user","message":{...,"content":[{"type":"tool_result",...}]}}`（须删除）。
- `ask_user_question.rs::ask_user_question_tool_result_content` / `render_answer_value`（须删除，仅被手工注入使用）。
- `ResolvedAskUserQuestion { input, answers }` → 只留 `answers`。
- 原生 `user.tool_result` 处理处当前 `resolved_ask_user_questions.contains_key(...)` 判定，须改为 `remove(...)` 并在 remove 前先判 `is_error`。

- [ ] **1.1 重写测试**：`claude_provider_deduplicates_assistant_then_control_ask_user_question` → `claude_provider_answers_ask_user_question_only_via_control_response`。fixture 顺序：初始 user → assistant `tool_use` → `control_request(can_use_tool, request_id=ask_req_001, tool_use_id=toolu_question)`；断言 ChoiceRequest 的 request_id == `ask_req_001`；收到 ChoiceResponse 前 fixture 若读到 stdin `"tool_result"` 立即失败；须收到 `control_response` 且 `updatedInput.answers` 正确；然后 fixture 自输出原生 tool_result 与 result；测试断言只一次 ChoiceRequest 且成功完成。运行 `cargo test -p cadence-aria --lib claude_provider_answers_ask_user_question_only_via_control_response` 期望 FAIL。
- [ ] **1.2 新增测试**：`claude_provider_reuses_choice_for_duplicate_control_request_until_native_tool_result`。fixture：第一个 control_request（tool_use_id=toolu_question）→ 收到 control_response 后再发第二个 control_request（同 tool_use_id、不同 request_id）→ 应复用缓存收到 control_response → 前端只一次 ChoiceRequest → 输出原生成功 tool_result + result。运行期望 FAIL。
- [ ] **1.3 新增测试**：`claude_provider_tool_use_without_control_request_is_protocol_error`。fixture：只发 assistant `tool_use(AskUserQuestion)`，不发 control_request；断言 aria 不注入 tool_result、最终报协议错误。运行期望 FAIL。
- [ ] **1.4 实现**：删手工注入路径；删 `write_tool_result` 与 `ask_user_question_tool_result_content`/`render_answer_value`；`ResolvedAskUserQuestion` 只留 answers；原生 tool_result 到达时**先判 is_error 再 remove**，is_error 则发协议错误并终止。
- [ ] **1.5 改 fixture**：`claude_ask_user_question_fixture.sh`、`claude_ask_user_question_tool_error_fixture.sh` 改为等 control_response 后输出原生 tool_result；`claude_ask_user_question_tool_use_bridge_failure_fixture.sh` 及用例 `claude_provider_ask_user_question_tool_use_emits_protocol_error_on_bridge_failure`（ask_user_question.rs:538）重写为「无 control_request 的 tool_use → 协议不兼容」。运行 `cargo test -p cadence-aria --lib claude_provider` 全绿。
- [ ] **1.6 提交**：`git commit -m "fix(claude): let native control flow own AskUserQuestion tool results"`（仅相关文件）

### Task 2（提交 B）：权限模式映射为合法 wire 值

**Files:**
- Modify: `src/cross_cutting/claude_code_provider/mod.rs`
- Test: `src/cross_cutting/claude_code_provider/tests/permissions.rs`

- [ ] **2.1 新增失败测试**（`tests/permissions.rs`）：
  - `claude_supervised_permission_mode_maps_to_default`：`permission_mode_for_claude(&Supervised) == "default"`
  - `claude_auto_permission_mode_uses_default_so_aria_remains_authoritative`：`permission_mode_for_claude(&Auto) == "default"`
  - `claude_initial_messages_send_only_valid_permission_modes`：解析初始握手第二行 JSON，`request.subtype == "set_permission_mode"`、`request.mode == "default"`、`!= "supervised"`
  运行 `cargo test -p cadence-aria --lib claude_permission_mode` 期望 FAIL。
- [ ] **2.2 实现**：新增 `fn permission_mode_for_claude(&ProviderPermissionMode) -> &'static str`（Auto|Supervised => `"default"`）；替换 `mod.rs` 初始握手 JSON 中 `mode` 取值；附注释「新版 CLI UI 显示为 manual，wire 兼容名为 default」。
- [ ] **2.3 新增回归**：`claude_auto_mode_routes_permission_request_through_auto_approval_bridge`：fixture 发 Bash `can_use_tool`，断言不出现 `ProviderEvent::PermissionRequest`、出现 Auto approval 执行事件、回写 `behavior:"allow"`。运行 `cargo test -p cadence-aria --lib claude_provider` 全绿。
- [ ] **2.4 提交**：`git commit -m "fix(claude): map aria permission policy to valid callback mode"`

### Task 3（提交 A）：所有模式注册 stdio 权限回调

**Files:**
- Modify: `src/cross_cutting/claude_code_provider/mod.rs`
- Test: `src/cross_cutting/claude_code_provider/tests/args.rs`、`tests/process.rs`、`tests/streaming.rs`

- [ ] **3.1 新增失败测试**（`tests/args.rs`）：`claude_args_always_include_stdio_permission_prompt`：对 Auto 与 Supervised，`build_args` 输出中 `--permission-prompt-tool=stdio` 恰好出现一次。运行 `cargo test -p cadence-aria --lib claude_args` 期望 FAIL。
- [ ] **3.2 实现**：`build_args` 删 `mode` 参数、始终 push `--permission-prompt-tool=stdio`；改 `args.rs:10,23` 两处调用；复核 `tests/process.rs:17`、`tests/streaming.rs:154,225` argv 断言是否受影响并修正。
- [ ] **3.3 新增回归**：`claude_auto_mode_registers_stdio_and_waits_for_ask_user_question`：argv 含 flag；AskUserQuestion 先出 ChoiceRequest、发送 ChoiceResponse 前无 Completed（锁定 Auto 下仍等待用户）。运行 `cargo test -p cadence-aria --lib claude_provider` 全绿。
- [ ] **3.4 提交**：`git commit -m "fix(claude): register stdio permission callback in auto mode"`

### Task 4：整体验证

- [ ] 4.1 `cargo fmt`；`cargo clippy --workspace --all-targets`；`cargo test --workspace` 全部通过
- [ ] 4.2 真实 claude CLI（2.1.237）smoke：`--permission-prompt-tool=stdio` 下验证 tool_use→control_request→control_response→tool_result 序列，记录输出作为新鲜证据
