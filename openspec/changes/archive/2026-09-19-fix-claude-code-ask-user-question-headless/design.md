# Design: fix-claude-code-ask-user-question-headless

## Context

见 proposal.md - Why。claude code headless 下 AskUserQuestion 的注册由本地 `isEnabled()` 决定：headless 且未配置 permission prompt tool 时禁用。aria `build_args` 仅在 Supervised 加 `--permission-prompt-tool=stdio`；默认 Auto 下工具缺失。此外 aria 发送非法的 `set_permission_mode: "supervised"`，并在 assistant tool_use 时手工注入 tool_result 造成同一 tool_use_id 两条结果。

## Goals / Non-Goals

- Goals：Auto 模式恢复 AskUserQuestion 注册；权限决策权保持在 aria；AskUserQuestion 结果单一所有权；权限模式映射合法。
- Non-Goals：不改 ApprovalBridge 决策逻辑；不改 text_fallback 二级兜底；不引入版本探测/最低版本门槛；不处理 --channels 场景（当前未使用）。

## Decisions

1. **build_args 始终加 `--permission-prompt-tool=stdio`，删除 `mode` 参数**：该 flag 是宿主交互能力注册，与 aria 权限策略解耦。
2. **权限模式映射：`Auto`/`Supervised` 都 → `"default"`**（用户已确认，否决 `auto`）：`default` 使 claude 不预判，所有权限请求经 stdio callback 进 ApprovalBridge；`auto` 会让 claude classifier 抢在 aria 前自动批准部分普通工具，破坏 aria 权限合同。
3. **wire 值用 `"default"`，不卡版本**（用户已确认）：`default` 是 SDK/wire 稳定标识，本机 2.1.220/2.1.237 实测均接受；代码附注释说明新版 UI 显示名为 manual；真实 CLI smoke test 兜底。
4. **AskUserQuestion 结果所有权单一化**：移除 assistant tool_use 手工注入 `tool_result` 的路径（删 `write_tool_result`、`ResolvedAskUserQuestion.input`、`ask_user_question_tool_result_content` 等）；`control_request` 是唯一回答入口；原生 tool_result 到达时 `remove` 缓存并处理 `is_error`。
5. **不支持只发 assistant tool_use 不发 control_request 的旧版**（用户已确认）：视为协议不兼容，按协议错误处理，不恢复双重回答路径。
6. **提交顺序 C→B→A**：C（结果所有权）先在现有 Supervised 路径验证；B（模式映射）次之；A（注册 stdio）最后把完整链路暴露给默认 Auto。

## Risks / Trade-offs

- [普通工具多一轮 control_request/response + Auto approval 审计] → 为保持 aria 权限权威的可接受代价，用户无感知；用 provider 级测试锁定该行为。
- [claude 显式 deny/组织策略仍可能先于 callback 拦截] → 不绕过安全规则，aria 只处理 claude 转发来的请求。
- [旧版只发 tool_use 会卡住/报错] → 按决策 5 报协议错误；smoke test 覆盖当前版本事件序列。
- [`default` 将来被移除] → 注释标明，届时跟进；当前两版实测接受。
- [Auto 无人值守时 choice 阻塞] → `ApprovalBridge::request_choice` 不检查权限模式，A 落地后默认 Auto 的 run 一旦触发 AskUserQuestion 会阻塞等待用户。这是既有行为，不是本次引入；由 provider timeout 与 cancel token 兜底，用户在 UI 回答或取消。tasks 3.3 的回归测试明确断言「发送 ChoiceResponse 前无 Completed」，锁定该语义；不新增超时/自动降级逻辑。

## Migration Plan

单仓内改动，无数据迁移；回滚即 revert 三个提交。

## Open Questions

（无——三个歧义点均已与用户确认）
