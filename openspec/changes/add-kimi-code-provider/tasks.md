## 1. Provider 目录与可用性

- [ ] 1.1 将 Kimi 纳入活跃流式 Provider 的名称（`ProviderName::KimiCode`，wire `"kimi_code"`）、健康检查（`kimi_version_command()` = `kimi --version`，最低 0.34.0）、状态接口（`handlers/providers.rs` DTO：display_name=`Kimi Code`，install_hint=`Install Kimi Code CLI and ensure \`kimi\` is available on PATH.`）和前端选择目录（`REAL_PROVIDER_CATALOG` + `PROVIDER_ORDER`）。
- [ ] 1.2 给 `ProviderType` 加 `KimiCode` 变体并保证所有 `ProviderName → ProviderType` 穷尽映射（image_create/models.rs、coding_workspace_engine/tool_format.rs、workspace_engine/mappings.rs、work_item_split_engine/types.rs）补分支；Task Runner 四入口（provider_factory、step_runner、web/runtime/provider.rs、web/runtime/utils.rs）对 Kimi 补稳定拒绝分支（`Err(incompatible_output)` 或稳定文本，禁止 `unreachable!`）并补单元测试；断言 `adapter_compatibility` 兼容性矩阵无 Kimi 条目。
- [ ] 1.3 保持仓库初始化的 Claude Code 专用选项与执行路径不受 Kimi 影响（前端 `CreateRepositoryDialog` 显式过滤 Kimi，加 capability policy 注释）。

## 2. Kimi ACP 流式会话适配

- [ ] 2.1 新增 `src/cross_cutting/kimi_code_provider/`（mod.rs/parse.rs/session.rs/tests.rs），冻结 `.pi-subagents/spike/acp/` 的 0.34.0 真实 ACP 往返为 `tests/fixtures/*.jsonl`。实现 `KimiCodeProvider` + `impl StreamingProviderAdapter`：复用 `JsonRpcPeer`，spawn `kimi acp`（cwd = 代码库目录），驱动 initialize → session/new → session/prompt 状态机，终态由 `session/prompt` result `stopReason` 判定（end_turn→Completed，其他/error→Failed）。覆盖：文本流（AgentMessageChunk 计入 full_output，AgentThoughtChunk 不计入）、工具调用（tool_call/tool_call_update）、退出码（0/1/75）、Abort（进程终止+不双发终态）、异常退出→Failed、版本<0.34.0 启动门禁报错。不变量：Completed/Failed 一次且仅一次；provider_session_id 仅 session/new 后上报。注册到生产/测试 `default_provider_registry()`。
- [ ] 2.2 实现 Supervised 审批往返：`session/new` 的 `permissionMode` 按 Aria 模式映射（Auto→`auto`，Supervised→`default`）；收到 `session/request_permission` 映射为既有 `PermissionRequest`（经 ApprovalBridge）；用户响应映射回 ACP result（approve_once/allow_always/reject）；reject 后工具不执行且会话继续。Auto 模式不发审批。回归测试：approve 完整往返、reject 后会话继续、Auto 不发 request_permission。

## 3. Workspace 角色配置与执行

- [ ] 3.1 普通 Workspace 的 Author/Reviewer 支持 Kimi；Kimi 默认权限 `Auto`，UI 可切 `Supervised`（与 Pi 的 Auto-only 区分）；`workspace_engine/mappings.rs` 不强制 Kimi 为 Auto，保留用户选择。
- [ ] 3.2 前后端 Kimi interaction guidance 一致（`workspace_context/prompts.rs` + `workspace-ws-store-guidance.ts`），声明 Kimi 支持 structured permission request。

## 4. Coding Workspace 角色配置与执行

- [ ] 4.1 Coding Workspace 的 Coder、Code Reviewer、Internal Reviewer 支持 Kimi；默认 `Auto`，可切 `Supervised`（`coding_models/provider_config.rs` 不强制 Auto-only）。
- [ ] 4.2 `work_item_projection/render/kimi_code.rs` 新增（仿 `render/pi.rs`），含专属 label、renderer version、Supervised tool hint、structured-output wrapper；接入 `render/mod.rs` + `renderer_for()`。

## 5. image-create 与边界

- [ ] 5.1 image-create 支持 Kimi：`image_create/models.rs` `From<ProviderName>` 加 Kimi 分支；前端 `web/src/api/types/image-create.ts` dropdown 加 Kimi；回归测试用脚本化 provider 跑通 Kimi 路径。
- [ ] 5.2 显式排除：`workspace_engine/provider_drive.rs`（artifact retry）与 `workspace_engine/review/drive.rs`（review repair）排除 Kimi，写明注释（第一阶段不实证 resume 稳定性）。

## 6. 用户界面与运行可见性

- [ ] 6.1 普通 Workspace 与 Coding Workspace 的 Provider 配置展示 Kimi；Kimi 提供 Auto/Supervised 控制（与 Claude Code/Codex 一致，区别于 Pi 的 Auto-only）。
- [ ] 6.2 前端穷尽 union/match 补 Kimi：`api/types/provider.ts`（`RealProviderName`）、`ProviderConfigPanel.tsx`、`CodingProviderConfigPanel.tsx`、`ChatWorkspacePageParts.tsx`、`workspace-ws-message-handler.ts`，及相关 `.test.tsx` fixture union；运行事件与界面状态呈现不可用原因（含版本过低、未登录运行错误）与失败状态。

## 7. 回归验证与质量门禁

- [ ] 7.1 后端：provider_health（Kimi 可用/缺失/超时/版本过低，snapshot 持久化，并行 probe）、provider_registry（stable order）、provider_availability_gate（entry 存在/缺失）、handlers/providers（DTO）、task-run 四入口拒绝（不 panic）、**凭证缺失运行错误映射**（模拟 ACP 认证错误/stderr → 清晰运行错误，提示 `kimi login`）。
- [ ] 7.2 前端：provider-options.test（catalog/order/可用禁用）、ProviderConfigPanel.test（Auto+Supervised）、CreateRepositoryDialog.test（Kimi 被过滤）、WebSocket parser、Chat fallback 含 kimi_code。
- [ ] 7.3 质量门禁：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo test --locked`、`cd web && pnpm tsc -b && pnpm test`。
