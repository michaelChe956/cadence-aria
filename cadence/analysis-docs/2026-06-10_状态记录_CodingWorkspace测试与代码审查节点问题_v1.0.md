# Coding Workspace 测试与代码审查节点问题状态记录

记录日期：2026-06-10
工作分支：`bugfix_test_branch`
工作区：`.worktrees/bugfix_test_branch`

## 背景

本记录用于保存 2026-06-09 端到端测试中发现的 Coding Workspace 流程问题，后续再讨论修复方案。当前只记录问题、证据和待讨论点，不包含实现决策。

## 问题 1：执行测试节点覆盖不足

用户期望 Coding Workspace 的“执行测试”节点至少覆盖：

- 单元测试。
- 如果是 API 改动，需要做 API 贯通测试。
- 如果是前端改动，至少保证前端不报错。
- 基础渗透测试、安全漏洞扫描或安全风险检查。

当前实际行为：

- 测试命令优先从 Work Item 的 `## 验证命令` 抽取。
- 如果抽取不到命令，则回退到自动发现。
- 自动发现当前只覆盖：
  - Rust：`cargo test --locked`
  - Python：`uv run pytest`
  - Node/package：`pnpm test` 或 `pnpm -C <dir> test`
- 如果 tester provider 支持 tool calls，会把命令交给 Tester Agent 调用 `run_command`；否则后端直接执行命令。
- `TestingReport` 只按命令退出码判断 `passed` / `failed` / `blocked`。

本次真实 attempt 证据：

- `testing_report_0001.json` 中只执行了两条命令：
  - `cargo test --locked`
  - `pnpm -C web test`
- 两条均通过。
- 未执行 `pnpm -C web build`。
- 未执行 API 贯通测试。
- 未执行安全扫描、漏洞扫描或基础渗透测试。

相关代码位置：

- `src/web/coding_ws_handler.rs`：`test_specs_for_attempt`
- `src/product/test_executor.rs`：`discover_test_commands`
- `src/product/tester_agent_loop.rs`：`build_tester_system_prompt`

额外观察：

- 当前 Work Item 虽然写了“全量验证”命令，但命令抽取逻辑遇到 `定向快反馈：` 这类小标题后退出“验证命令”块，导致没有抽取到后续 `cargo fmt`、`cargo clippy`、`cargo check`、`pnpm build` 等命令。
- 因为没有抽取到 Work Item 命令，系统回退到了自动发现路径，最终只跑了 `cargo test --locked` 和 `pnpm -C web test`。

待讨论点：

- 测试节点是否需要固定质量门禁策略，而不是只依赖 Work Item 命令和 Tester Agent 自觉补充。
- 如何定义 API 改动、前端改动、安全风险改动的检测规则。
- 前端“不报错”最低标准是 `pnpm build`、启动 smoke test，还是 Playwright 页面冒烟。
- 安全检查最低标准采用哪些命令或内置检查。

## 问题 2：代码审查节点卡住

当前现象：

- Coding Attempt 当前状态为 `blocked`。
- 当前 stage 为 `code_review`。
- timeline 中 `coding_node_0005` 状态为 `blocked`，summary 为 `code review 被阻塞`。
- 前端表现为停在代码审查节点，但没有可继续的 gate 操作。

直接原因：

- reviewer 实际输出中包含 `verdict=request_changes` 和 10 条 findings。
- 但 reviewer 输出的 findings 不符合后端 schema：
  - 缺少必填 `severity`。
  - 没有按 schema 输出 `message`。
  - 没有输出 `required_action`。
  - 没有输出 `source_stage=code_review`。
- 后端 `parse_review_payload` 解析失败后进入 `blocked_review_payload`。
- 最终 `code_review_0001.json` 被保存为：
  - `verdict: blocked`
  - `findings: []`
  - `summary: review 输出不是有效 JSON，已阻塞并等待人工确认: ...`

相关代码位置：

- `src/product/coding_workspace_engine.rs`：`execute_code_review_with_commands`
- `src/product/coding_workspace_engine.rs`：`build_code_review_prompt`
- `src/product/coding_workspace_engine.rs`：`parse_review_payload`
- `src/product/coding_models.rs`：`ReviewFinding` / `FindingSeverity`

前端/流程层观察：

- `blocked` 后没有创建 open stage gate。
- `build_coding_session_state` 只会把 open stage gates 转成 `pending_gates`。
- 当前 `pending_gates=[]`。
- `CodingWorkspacePage` 的 `GatePanel` 在没有 gate 时直接返回 `null`。
- 因此前端没有展示“采纳返修 / 重试审查 / 人工确认继续”等恢复动作，只剩“发送上下文”“中止”“删除”等通用入口。

待讨论点：

- 代码审查 prompt 是否需要更明确的 JSON schema 示例和禁止 markdown fence。
- 后端是否应兼容 reviewer 常见字段别名，例如 `summary` / `failure_scenario` 转为 `message`，缺失 `severity` 时是否降级或按默认 severity 处理。
- 解析失败是否应该保留原始 findings，而不是直接变成 `findings=[]`。
- `blocked` 状态是否必须创建可操作 gate，让前端能恢复流程。
- `request_changes` 和 `blocked` 的流程语义是否需要区分：
  - `request_changes`：应进入返修分析或自动返修。
  - `blocked`：应提供人工处理入口。

## 当前结论

- 测试节点问题主要是后端测试策略不足，当前只是命令执行器，不是完整质量门禁。
- 代码审查卡住的主因是 reviewer 输出 schema 不合规导致后端解析失败；前端没有恢复 gate 放大了“卡住”的体验。
- 明日优先讨论两个方向：
  - 测试节点质量门禁范围和命令生成策略。
  - 代码审查输出 schema、解析容错和 blocked 恢复入口。
