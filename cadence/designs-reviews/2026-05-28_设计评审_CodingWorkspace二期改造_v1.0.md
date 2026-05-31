# CodingWorkspace 二期改造设计评审

> 版本：v1.0 | 日期：2026-05-28

## 一、评审结论

原设计方向成立，但 P1-P7 不能按原顺序直接实施。当前 CodingWorkspace 的 WebSocket handler 采用单连接内顺序执行模式，Provider 配置仍是 author/reviewer 双角色，Testing 阶段也没有可被 engine 拦截的结构化 tool_use 协议。因此需要先补一个 P0 预备收口，并重排 P1-P7 的底层依赖。

## 二、必须修正的问题

### 2.1 StageGate 与当前 WS 执行模型冲突

当前 `StartCoding` 会在 `handle_coding_socket` 内同步驱动完整执行流。执行流运行期间，同一个 WebSocket receive loop 无法继续处理 `StageGateConfirm`、`ProviderSelect`、`AbortAttempt` 等客户端消息。

修正方式：

- 新增 `AttemptRunner` 后台任务承载执行流。
- WebSocket handler 只负责接收客户端消息并写入 runner 命令通道。
- StageGate 状态必须持久化，包含 `gate_id`、`stage`、`role`、`expires_at`、`provider_snapshot`、`status`。
- 重连时由 session state 恢复 gate 剩余时间和当前 provider。

### 2.2 Provider 角色模型需要兼容迁移

原计划直接把 `ProviderConfigSnapshot` 替换为 5 角色配置，风险过高，会影响 HTTP snapshot、WS session state、前端类型和既有 Workspace provider 选择链路。

修正方式：

- 保留现有 `provider_config_snapshot.author/reviewer/review_rounds` 对外协议。
- 新增 Coding 内部角色配置模型，并从旧快照派生默认值。
- 默认映射：
  - `coder = author`
  - `tester = author`
  - `analyst = reviewer.unwrap_or(author)`
  - `code_reviewer = reviewer.unwrap_or(author)`
  - `internal_reviewer = reviewer.unwrap_or(author)`
- 前端 5 角色面板等 StageGate runner 稳定后再接入。

### 2.3 TestAgentLoop 缺少结构化工具协议

当前 `StreamingProviderAdapter` 对 CodingWorkspace 暴露 `Text/Done/Error`。Provider 内部工具调用被转换为 execution event，engine 不能拦截 `write_file` / `edit_file` 并返回 tool_result，因此无法实现真正的 Tester tool 白名单。

修正方式：

- 先扩展 CodingWorkspace 可用的 Provider session 协议，显式暴露 `ToolCall` / `ToolResult`。
- 白名单拦截放在 runner 层，而不是放在 provider 内部日志事件层。
- 在该协议完成前，Testing 阶段继续使用后端命令执行器；LLM Tester 可先只做测试命令建议和报告分析。

### 2.4 InternalPrReview 流程以现有代码为准

现有稳定链路为：

```text
Coding -> Testing -> CodeReview -> ReviewRequest(push) -> InternalPrReview -> FinalConfirm
```

InternalPrReview 需要基于 commit、review request 和 diff 做内部审查，因此应保留在 ReviewRequest 之后。后续 P6/P7 文档应以该顺序为准。

## 三、修正后的执行顺序

| 阶段 | 名称 | 核心交付 |
|------|------|----------|
| P0 | 当前场景预备收口 | Work Item 上下文、验证命令、prompt 可见、PrepareContext provider_select |
| P1 | 兼容式角色模型 | 5 角色内部模型 + author/reviewer 兼容层 |
| P2 | AttemptRunner 与 StageGate | 后台执行任务、命令通道、Gate 持久化、倒计时交互 |
| P3 | Tester 工具协议 | ToolCall/ToolResult 协议、白名单、测试报告 |
| P4 | Rework 分析官 | AnalystVerdict、ContextNote 注入、路由 |
| P5 | 前端 UX 对齐 | 消息气泡、Timeline、角色颜色 |
| P6 | CodeReview 与 InternalPrReview | Review provider 与现有 ReviewRequest 顺序对齐 |
| P7 | 集成验收与 E2E | 全流程与边界场景 |

## 四、当前已完成的 P0 内容

- Work Item markdown 已加入 Coding session state。
- Work Item 中的 `## 验证命令` 可解析为 `verification_commands`。
- `provider_select` 可在 PrepareContext 阶段更新 author/reviewer provider。
- Coding provider prompt 已作为 execution event 输出，便于确认 prompt 内容。
- 支持 fenced artifact 中包含内层代码块的 Work Item 提取。
