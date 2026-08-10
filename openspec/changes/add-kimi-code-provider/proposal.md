## Why

产品当前支持 Claude Code、Codex、Pi 三个真实流式 Provider。接入 Kimi Code（月之暗面官方编码 CLI）能提供第四个可供用户选择的执行端，使日常 Workspace（Story/Design/Work Item 的 Author/Reviewer）与 Coding Workspace（Coder/Code Reviewer/Internal Reviewer）以及图片创作（image-create）均可使用本机已安装的 Kimi。

Kimi Code CLI 通过 `kimi acp` 子命令提供基于 stdin/stdout 的 ACP（Agent Client Protocol）JSON-RPC 服务端，已实测（Phase 0 Spike，Kimi 0.34.0）支持会话强控（list/resume/close/delete/fork）、多模态 prompt、以及宿主可拦截的工具审批（`session/request_permission`），可扩展性与会话控制能力优于 JSONL 单发模式。

## What Changes

- 在日常产品执行链路中新增 Kimi Code Provider：Story、Design、Work Item Workspace 的 Author / Reviewer，以及 Coding Workspace 的 Coder、Code Reviewer、Internal Reviewer 均可选择 Kimi。
- 为 Kimi 提供健康检查（`kimi --version`，最低 0.34.0）、可用性展示、ACP 流式会话、工具审批往返、取消与会话标识持有；与现有 Provider Registry 和 WebSocket 运行链路集成。
- Kimi 同时支持 `Auto` 与 `Supervised` 权限模式（ACP `default` = Supervised，`auto`/`yolo` = Auto），默认 `Auto`，与 Claude Code/Codex 对齐。
- 图片创作（image-create）支持 Kimi（image-create 复用 streaming provider 会话，无需独立图像 API）。
- Provider 启动或运行失败时直接报告失败，不在运行期自动切换到其他 Provider；第一阶段不实现同 Provider 内部重试（artifact retry / resume retry）。
- 仓库初始化仅 Claude Code 可用，Kimi（同 Pi）被显式过滤，不展示或不允许用于初始化。
- Task Runner（task-run）及其 CLI/API、专用协议、旧的 Fake Provider Workspace Runner 的可调度 Provider 范围与运行行为不变；为满足流式链路共享类型约束，`ProviderType` 与 `ProviderName` 增加 `KimiCode` 变体，但 Kimi 在 task-run 所有入口被显式拒绝，返回稳定错误（不使用 `unreachable!`，不引入新的 panic 风险）。
- structured-output repair / review repair 第一阶段排除 Kimi（不实证 Kimi resume 稳定性，留作后续 enhancement）。
- Kimi 第一阶段**不支持需求歧义结构化提问**（`ChoiceRequest` 等价物）。Phase 0 Spike 已验证：ACP 协议下 Kimi 仅有工具审批（`session/request_permission`），无独立的开方式提问/选择方法；agent 遇到歧义时以纯文本提问（`agent_message_chunk`）。因此 Kimi 不复用 Pi 的 `aria-ask.ts` 提问扩展，需求歧义走文本 fallback。

## Capabilities

### New Capabilities

- `kimi-code-provider-integration`: 在活跃的 Workspace、Coding Workspace 与 image-create 工作流中选择、执行 Kimi Code Provider（Auto / Supervised 模式）。

### Modified Capabilities

- 无（Kimi 作为独立 capability 接入，不改写既有 provider 行为）。

## Impact

- Provider 名称与前后端 API contract、Provider 健康检查（新增 `kimi_version_command`）、可选 Provider 目录与排序。
- Workspace 与 Coding Workspace 的 WebSocket 流式 Provider Registry、ACP 会话控制与现有授权桥接（Kimi 走 ACP `session/request_permission` 复用 ApprovalBridge）。
- 普通 Workspace 与 Coding Workspace 的 Provider 配置持久化、默认权限模式（Kimi 默认 Auto，可切 Supervised）。
- 新增 Kimi ACP 适配模块（`kimi_code_provider/`，含 mod/parse/session/tests 与冻结自 0.34.0 的 JSON-RPC fixture）；不修改全局 Kimi 配置或会话目录（复用用户默认 `~/.kimi-code/`）。
- image-create 的 `ProviderName → ProviderType` 映射与前端 dropdown。
- task-run 边界：四入口显式拒绝 Kimi，返回稳定错误文案。
- 凭证前置：Kimi 不从环境变量（如 `KIMI_API_KEY`）自动读取凭证，必须写入 `~/.kimi-code/config.toml` 或完成 `kimi login`；运行期若凭证缺失，从 ACP 错误/stderr 捕获并映射为清晰运行错误。
- 前端 provider catalog、权限控件、仓库初始化过滤、WebSocket parser、guidance 文案。
