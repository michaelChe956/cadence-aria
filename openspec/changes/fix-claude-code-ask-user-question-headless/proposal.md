# Proposal: fix-claude-code-ask-user-question-headless

## Why

claude code 在 headless（`-p --input-format stream-json`）模式下，只有启动参数携带 `--permission-prompt-tool=stdio` 时才向模型注册 AskUserQuestion 工具。aria 的 `ClaudeCodeProvider::build_args` 仅在 Supervised 模式加该参数，而 workspace 默认 author 是 Auto，导致结构化提问工具从未注册：模型只能自行按 Issue 推导固化口径，或输出文本说明「当前工具环境未注册 AskUserQuestion」。历史会话（≤8/3）能正常发起 AskUserQuestion，根因即启动参数差异，而非 claude code 升级或服务端门控（已由最小变量 A/B 实验证实：同网关同模型，仅增删 stdio flag 即可在「工具不存在」与「tool_use→control_request→control_response→tool_result 完整成功」之间翻转）。

## What Changes

- `ClaudeCodeProvider::build_args` 始终添加 `--permission-prompt-tool=stdio`（Auto 与 Supervised 一致），删除 `mode` 参数。
- 新增 `permission_mode_for_claude()`：Aria 的 `Auto` 与 `Supervised` 都映射为 claude code wire 值 `"default"`，替代当前非法的 `"supervised"`；claude 不再用自身 classifier 预判普通工具，权限决策权保留在 aria `ApprovalBridge`（Auto 自动批准普通工具，AskUserQuestion 始终等待用户）。
- 移除 assistant `tool_use(AskUserQuestion)` 时 aria 手工注入 `user.tool_result` 的路径：`control_request(can_use_tool)` 成为唯一回答入口，CLI 生成原生 tool_result；原生 tool_result 到达时消费 `resolved_ask_user_questions` 缓存并处理 `is_error` 协议错误。
- 不支持「只发 assistant tool_use、不发 control_request」的旧版 claude code：视为协议不兼容，明确报协议错误，不恢复双重回答路径。
- 保持 `text_fallback` 作为二级兜底不变。

## Capabilities

### New Capabilities
- `claude-code-structured-interaction`: claude code headless 模式下结构化提问（AskUserQuestion）与权限回调（permission prompt tool）的注册、模式映射与结果所有权协议。

### Modified Capabilities

（无）

## Impact

- 代码：`src/cross_cutting/claude_code_provider/{mod,stream,ask_user_question}.rs`
- 测试：`src/cross_cutting/claude_code_provider/tests/{args,permissions,ask_user_question}.rs`、`tests/fixtures/provider/claude_ask_user_question*.sh`
- 行为：默认 Auto 模式下 claude code author 重新获得 AskUserQuestion 工具；普通工具权限仍由 aria ApprovalBridge 决定（多一轮 control_request/response 与 Auto approval 审计事件，用户无感知）。
- 对外接口无变化。
