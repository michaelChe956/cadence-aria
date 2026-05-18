# product-workbench-issue-lifecycle 端到端测试方案

## 文档信息

- 文档类型：计划文档 / E2E 测试方案
- 版本：v1.0
- 日期：2026-05-18
- 适用分支：`product-workbench-issue-lifecycle`
- 工作区：`.worktrees/product-workbench-issue-lifecycle`
- 背景方案：`cadence/designs/2026-05-17_技术方案_真实流式Provider接入设计_v1.0.md`
- 方案范围：只设计端到端测试，不执行测试，不改动生产代码。

## 1. 分支当前进度理解

### 1.1 产品主线

当前分支已经把 Web 首页切到 Issue 生命周期主流程：

- `/` 重定向到 `/workbench`。
- `/workbench` 渲染 `IssueLifecycleWorkbench`，主区域为四列看板：Issue、Story Spec、Design Spec、Work Item。
- `/workbench/workspace/:sessionId` 渲染 `WorkspacePage`，作为全屏对话式 Workspace。
- Issue 创建必须绑定 Repository。
- Story、Design、Work Item 都通过产品生命周期 API 创建对应实体和 Workspace Session。
- Story/Design/Work Item 的 provider 交互主路径已经转向 WebSocket，而不是旧 HTTP `run-next`。

关键文件：

- `web/src/router.tsx`
- `web/src/app-shell.tsx`
- `web/src/components/lifecycle/IssueLifecycleWorkbench.tsx`
- `web/src/pages/WorkspacePage.tsx`
- `src/web/app.rs`
- `src/web/handlers.rs`
- `src/product/lifecycle_store.rs`

### 1.2 Provider 与 Workspace 主线

当前分支已经实现真实流式 provider 接入的主要结构：

- `src/web/workspace_ws_handler.rs` 负责 `/api/workspace-sessions/{session_id}/ws`。
- `src/product/workspace_engine.rs` 负责 session stage、message、artifact、checkpoint、provider event 转换。
- `src/cross_cutting/streaming_provider.rs` 定义 `StreamingProviderAdapter` session API、`ProviderEvent`、`ProviderCommand`、fake provider。
- `src/cross_cutting/provider_registry.rs` 负责按 `ProviderName` 选择 provider。
- `src/cross_cutting/claude_code_provider.rs` 处理 Claude Code `stream-json`。
- `src/cross_cutting/codex_provider.rs` 处理 Codex `app-server` JSON-RPC。
- WebSocket 协议已包含 `permission_request`、`permission_response`、`provider_status`、`execution_event`。
- 前端 store/hook 已处理权限队列、provider 状态和执行事件。

关键文件：

- `src/web/workspace_ws_types.rs`
- `src/web/workspace_ws_handler.rs`
- `src/product/workspace_engine.rs`
- `src/cross_cutting/approval_bridge.rs`
- `src/cross_cutting/claude_code_provider.rs`
- `src/cross_cutting/codex_provider.rs`
- `web/src/hooks/useWorkspaceWs.ts`
- `web/src/state/workspace-ws-store.ts`

### 1.3 现有测试基础

现有 Playwright E2E 覆盖较窄：

- `web/e2e/issue-lifecycle-workspace.spec.ts`：确认默认页面是四列生命周期工作台。
- `web/e2e/fake-workbench.spec.ts`：通过 API seed Story Workspace，进入 Workspace，用 fake provider 流式生成并确认。
- `web/e2e/workbench-visual.spec.ts`：桌面与移动宽度下检查主界面和 Workspace 无横向溢出。

现有后端集成测试覆盖较深：

- `tests/web_lifecycle_api.rs` 覆盖 Project/Repository/Issue/Story/Design/WorkItem API 与确认约束。
- `tests/workspace_ws_integration.rs` 覆盖 WebSocket session state、fake stream、rollback、provider 选择持久化、reconnect、interrupt、abort、Claude fixture 权限确认、Codex fixture command execution event。
- `tests/web_provider_execution_events.rs` 覆盖旧 provider event 与敏感信息脱敏。

因此 E2E 不应重复验证 Claude/Codex 私有协议细节，而应验证浏览器用户路径、前端状态呈现和 API/WebSocket/UI 串联。

## 2. 测试目标与非目标

### 2.1 目标

1. 覆盖用户从项目创建到 Issue 生命周期四列推进的端到端主路径。
2. 覆盖全屏 Workspace 的 WebSocket 对话、流式输出、artifact、checkpoint、人工确认与返回看板后的状态同步。
3. 覆盖真实流式 provider 接入在前端可见层面的关键行为：provider 状态、执行事件、权限确认卡片、允许/拒绝响应。
4. 覆盖中断、回退、重连、provider 不可用等用户能感知的稳定性边界。
5. 覆盖桌面与移动布局，防止高密度工作台和全屏 Workspace 出现横向溢出、遮挡或主流程入口不可达。
6. 建立稳定、可复现、可并入 CI 的 Playwright 数据隔离和 provider fixture 策略。

### 2.2 非目标

- 不在 E2E 中调用真实本机 Claude Code 或 Codex 账号执行长任务。
- 不用 E2E 重新证明 Claude `stream-json` 或 Codex JSON-RPC 的每个协议分支；这些应保留在 Rust 集成测试和 provider fixture 测试中。
- 不在本轮 E2E 覆盖完整 Coding Workspace 的 diff、测试结果、review/rework 循环；目标设计明确该能力仍是后续演进边界。
- 不在 E2E 中验证旧 TUI，分支已删除 TUI 主线。
- 不执行测试操作；本文仅为方案。

## 3. 测试环境与数据策略

### 3.1 启动方式

沿用现有 Playwright 配置：

```bash
pnpm --dir web test:e2e
```

现有 `web/playwright.config.ts` 会启动：

- API：`node ./e2e/start-api.mjs`
- Web：`pnpm dev --port 5173`

`web/e2e/start-api.mjs` 当前会：

- 创建临时 git repo。
- 写入 `README.md` 与 `.gitignore`。
- `cargo run --manifest-path ../Cargo.toml --locked -- web --workspace <tmp> --host 127.0.0.1 --port 4317`。

建议保留临时 repo 策略，避免污染用户仓库和 `~/.aria`。

### 3.2 Provider fixture 策略

E2E 分三类 provider：

| 类型 | 用途 | 稳定性要求 |
|------|------|------------|
| `fake` | P0 主流程、快速稳定 smoke | 默认必须可用 |
| fixture `claude_code` | 权限确认卡片、provider status、message complete | 不依赖真实 Claude 账号 |
| fixture `codex` | execution event、command output、cwd 展示 | 不依赖真实 Codex 账号 |

建议新增 E2E profile，而不是直接依赖默认 `claude` / `codex` 命令：

```text
ARIA_E2E_PROVIDER_FIXTURES=1
ARIA_CLAUDE_COMMAND=<repo>/tests/fixtures/provider/claude_stream_json_fixture.sh
ARIA_CODEX_COMMAND=<repo>/tests/fixtures/provider/codex_app_server_current_fixture.sh
```

如果生产代码暂未支持环境变量覆盖 provider 命令，则 E2E 方案应先落一项测试基础设施任务：让 `serve_web` / `WebAppState` 的默认 provider registry 读取上述环境变量。这样浏览器 E2E 可以稳定覆盖真实 streaming provider 的前端表现，而不触发真实外部 CLI。

### 3.3 数据创建原则

- 每个测试使用唯一 Project 名称，例如 `E2E Lifecycle ${Date.now()}`。
- Repository 使用 `/api/workspaces` 返回的临时 repo path。
- 跨列主流程可以用 UI 创建 Project/Repository/Issue，也可以用 `page.request` 快速 seed 前置状态。
- P0 用户主路径优先使用 UI 操作；需要绕开冗长前置条件的 P1/P2 场景可使用 API seed。
- 每个测试只断言自身创建的数据，避免依赖列表顺序。

## 4. 推荐测试分层

### 4.1 Playwright E2E 负责

- 用户能否完成业务路径。
- 页面、按钮、表单、路由、WebSocket hook、Zustand store 与后端 API 是否串起来。
- 用户能看到的 provider status、权限卡片、执行事件、错误提示是否正确。
- 断线重连、返回看板、刷新页面后的状态是否可恢复。
- 主要 viewport 下界面是否可用。

### 4.2 Rust 集成测试继续负责

- Claude/Codex 私有协议解析。
- `ApprovalBridge` 的 pending map、cancel、abort、receiver close。
- `WorkspaceEngine` 的 checkpoint、rollback、provider event 桥接。
- `workspace_ws_handler` 的低层 WebSocket 消息序列。
- provider 进程管理、命令缺失、非 0 exit、stderr 聚合。

### 4.3 前端组件/Hook 测试继续负责

- `workspace-ws-store` 的状态归约。
- `useWorkspaceWs` 的消息解析和 JSON 发送。
- `WorkspacePage` 的权限卡片按钮行为。
- `IssueLifecycleWorkbench` 的组件级 API mock 分支。

## 5. E2E 用例矩阵

### 5.1 P0：必须进入 CI 的主路径

| ID | 用例 | 入口 | Provider | 核心断言 |
|----|------|------|----------|----------|
| E2E-P0-01 | 默认进入生命周期工作台 | `/` | 无 | 重定向到 `/workbench`，四列可见 |
| E2E-P0-02 | UI 创建 Project、Repository、Issue | `/workbench` | 无 | Project sidebar 更新，Issue 列出现新卡片 |
| E2E-P0-03 | Issue 生成 Story Workspace 并确认 | 看板 + Workspace | fake | Story 卡片出现，Workspace 流式输出，artifact 更新，确认后 Story 状态为 confirmed |
| E2E-P0-04 | Story confirmed 后生成 Design Workspace | 看板 + Workspace | fake | Design Spec 按钮仅在 Story confirmed 后出现，确认后 Design 状态为 confirmed |
| E2E-P0-05 | Design confirmed 后生成 Work Item Workspace | 看板 + Workspace | fake | Work Item 卡片出现，打开 Workspace，确认后 Plan 状态/Work Item 状态更新 |
| E2E-P0-06 | Workspace 返回看板状态同步 | Workspace 返回按钮 | fake | 返回后当前 Project 可选，四列展示最新 confirmed 状态 |
| E2E-P0-07 | 桌面和移动无横向溢出 | `/workbench` + Workspace | fake | `scrollWidth <= clientWidth`，主按钮无遮挡 |

### 5.2 P1：Provider streaming 与恢复能力

| ID | 用例 | 入口 | Provider | 核心断言 |
|----|------|------|----------|----------|
| E2E-P1-01 | Supervised 权限允许后继续完成 | Workspace | claude fixture | 出现权限卡片，点击“允许”，卡片消失，收到 message complete，进入人工确认 |
| E2E-P1-02 | Supervised 权限拒绝后展示错误或可恢复状态 | Workspace | claude fixture | 点击“拒绝”，卡片消失，页面展示 provider 失败/错误，输入框恢复可用或阶段回到准备上下文 |
| E2E-P1-03 | Codex execution event 展示命令与输出 | Workspace | codex fixture | 执行 tab 显示 `Command`、`pwd`、cwd、stdout、completed |
| E2E-P1-04 | Provider 选择持久化 | Workspace provider 面板 | fake/codex | 修改 Author 后刷新或重连，Provider 文案保持选择 |
| E2E-P1-05 | 流式中止不创建 assistant checkpoint | Workspace | fake long prompt | 点击“中止”，无 `message_complete` 可见结果，输入框恢复，返回后无部分 assistant 消息 |
| E2E-P1-06 | 用户新消息打断旧流 | Workspace | fake long prompt | 发送第二条消息后最终流内容来自第二条，不出现旧流完成 |
| E2E-P1-07 | 回退到历史 checkpoint | Workspace | fake | 两轮生成后点击第一条 assistant 的回退，第二轮消息和 artifact 被移除 |
| E2E-P1-08 | 刷新/重连恢复 session state | Workspace | fake | 生成后刷新页面，历史消息、artifact、checkpoint 回退按钮仍存在 |

### 5.3 P2：错误、约束与兼容路径

| ID | 用例 | 入口 | Provider | 核心断言 |
|----|------|------|----------|----------|
| E2E-P2-01 | 没有 Repository 时不能新建 Issue | `/workbench` | 无 | 新建 Issue 按钮 disabled 或 dialog 无法提交 |
| E2E-P2-02 | 删除 Repository 后生成 Story 失败可见 | 看板 | 无 | UI 展示错误，不创建孤立 Story 卡片 |
| E2E-P2-03 | provider unavailable 展示错误 | Workspace | 缺失 provider | 错误信息包含 provider unavailable，阶段不进入 completed |
| E2E-P2-04 | 非法 WebSocket session | `/workbench/workspace/bad` | 无 | 页面展示 session not found 错误 |
| E2E-P2-05 | 旧 HTTP `run-next` 兼容入口不影响主流程 | API seed + UI | fake | 主页面仍打开 WebSocket Workspace，不回到旧执行工作台 |

## 6. P0 主流程详细脚本设计

### 6.1 E2E-P0-02：UI 创建 Project、Repository、Issue

步骤：

1. 打开 `/workbench`。
2. 点击 Project sidebar 的创建 Project 按钮。
3. 输入唯一 Project 名称并提交。
4. 点击创建 Repository。
5. 使用 `/api/workspaces` 取得临时 repo path，填入 Repository 表单。
6. 点击新建 Issue。
7. 选择 Repository，填写标题和描述。
8. 提交后等待 Issue 列出现该标题。

断言：

- `Project 切换` navigation 中出现新 Project。
- Repository 列表出现新 Repository。
- Issue 列 region 包含新 Issue 标题。
- Story/Design/Work Item 列暂不出现派生卡片。
- 页面没有 `AI Coding Workbench`、旧 task workbench 文案。

### 6.2 E2E-P0-03：Story Workspace 生成并确认

步骤：

1. 在 Issue 列点击刚创建的 Issue 卡片。
2. 点击“生成 Story Spec”。
3. 自动进入 `/workbench/workspace/:sessionId`。
4. 等待 TopBar 显示 `Story Spec`。
5. 输入“请生成 Story Spec 和验收标准”并回车。
6. 等待流式内容出现，等待“确认通过”按钮出现。
7. 切到 Artifact tab，确认 artifact 非空。
8. 点击“确认通过”。
9. 断言输入框 placeholder 为“会话已完成”且 disabled。
10. 点击“返回”。
11. 选择当前 Project，确认 Story Spec 列显示该 Story，状态为 `confirmed`。

断言：

- URL 从 `/workbench` 变为 `/workbench/workspace/<id>`。
- `Author: fake | Reviewer: codex` 可见。
- 至少出现一条 assistant 消息和一个回退按钮。
- Stage 标签最终为“已完成”或卡片状态为 `confirmed`。

### 6.3 E2E-P0-04：Design Workspace 生成并确认

步骤：

1. 在 Story Spec 列点击 confirmed Story 卡片。
2. 断言 header 出现“生成 Design Spec”。
3. 点击后进入 Workspace。
4. 输入“请生成前端设计方案”。
5. 等待确认按钮，确认通过。
6. 返回看板。

断言：

- Design Spec 列出现新卡片。
- Design 卡片来源关联当前 Issue。
- Design 确认后状态为 `confirmed`。
- 如果 Story 未 confirmed，生成 Design 按钮不可见。

### 6.4 E2E-P0-05：Work Item Workspace 生成并确认

步骤：

1. 在 Design Spec 列点击 confirmed Design 卡片。
2. 断言 header 出现“生成 Work Item”。
3. 点击后进入 Workspace。
4. 输入“请生成可执行 Plan 和工作项拆分”。
5. 等待 artifact 与确认按钮。
6. 点击确认通过并返回看板。

断言：

- Work Item 列出现新卡片。
- Work Item 记录关联 Story 与 Design。
- Workspace 类型显示 `Work Item`。
- 当前实现中确认 Work Item 会把 `plan_status` 更新为 `confirmed`，E2E 应断言卡片或生命周期 API 中该状态。

## 7. P1 Provider 可见行为详细设计

### 7.1 权限确认允许

前置：

- 使用 fixture Claude provider 创建 author 为 `claude_code` 的 Story Workspace。
- E2E API server 需要注册 `tests/fixtures/provider/claude_stream_json_fixture.sh`。

步骤：

1. 进入 Workspace。
2. 输入触发 provider 的消息。
3. 等待权限卡片出现，内容包含 `Bash`。
4. 断言执行 tab 显示 `Provider: 等待权限` 或等待权限 badge。
5. 点击“允许”。
6. 断言权限卡片移除。
7. 等待 assistant message complete，确认按钮出现。

断言：

- 浏览器实际发送 `permission_response`。如果不直接拦截 WebSocket，可通过后续完成状态证明。
- 卡片消失后不会重复出现同一 permission id。
- 最终 artifact 非空，stage 为人工确认。

### 7.2 权限拒绝

步骤：

1. 同权限允许流程，但点击“拒绝”。
2. 等待 provider 返回失败或结束。

断言：

- 权限卡片移除。
- 页面出现错误或 provider 状态为失败。
- 用户输入框恢复可用，允许重新发送消息。
- 不创建 confirmed 状态，不出现“会话已完成”。

### 7.3 Codex 执行事件

前置：

- 使用 fixture Codex provider 创建 author 为 `codex` 的 Story Workspace。
- E2E API server 注册 `tests/fixtures/provider/codex_app_server_current_fixture.sh`。

步骤：

1. 进入 Workspace。
2. 发送消息。
3. 保持右侧“执行”tab。
4. 等待 command row 出现。

断言：

- `Provider: 运行中` 后进入 `已完成`。
- 执行事件包含 `Command` 类型、`pwd` 命令、cwd 为临时 repo path。
- stdout 区域包含临时 repo path。
- 最终可以进入人工确认。

## 8. 稳定性与选择器规范

### 8.1 选择器优先级

优先使用：

1. `getByRole` + accessible name。
2. `getByPlaceholder`。
3. `getByText` 只用于稳定业务文案。
4. 必要时新增 `aria-label`，避免依赖 CSS class。

不建议：

- 依赖卡片 DOM 层级。
- 依赖列表第一个元素，除非测试数据已唯一化。
- 使用固定 timeout 代替等待业务状态。

### 8.2 等待策略

- 对 API seed：使用 `expect(response).toBeOK()` 后再进入 UI。
- 对 WebSocket 流：等待具体 UI 变化，如“确认通过”按钮、权限卡片、执行事件行。
- 对返回看板：返回后显式点击当前 Project，再断言四列。
- 对 mobile viewport：先切换 viewport，再断言 header、关键按钮和无横向溢出。

### 8.3 失败诊断

建议 Playwright 配置开启：

```typescript
use: {
  trace: "retain-on-failure",
  screenshot: "only-on-failure",
  video: "retain-on-failure",
}
```

CI 中保留：

- Playwright trace。
- API server stdout/stderr。
- 临时 repo `.aria` 目录压缩包，仅失败时保留。

## 9. 建议文件组织

建议把 E2E 拆成以下文件：

```text
web/e2e/
├── helpers/
│   ├── lifecycle-fixtures.ts
│   ├── workspace-fixtures.ts
│   └── assertions.ts
├── issue-lifecycle-workspace.spec.ts
├── lifecycle-happy-path.spec.ts
├── workspace-streaming.spec.ts
├── workspace-permissions.spec.ts
├── workspace-recovery.spec.ts
└── workbench-visual.spec.ts
```

职责：

- `lifecycle-fixtures.ts`：Project/Repository/Issue/Story/Design/WorkItem seed。
- `workspace-fixtures.ts`：创建指定 author provider 的 Workspace Session。
- `assertions.ts`：`expectNoHorizontalOverflow`、`expectCurrentProjectVisible`、`expectWorkspaceCompleted`。
- `lifecycle-happy-path.spec.ts`：P0 业务主流程。
- `workspace-streaming.spec.ts`：fake stream、confirm、artifact。
- `workspace-permissions.spec.ts`：Claude fixture permission。
- `workspace-recovery.spec.ts`：rollback、abort、interrupt、reload。

## 10. CI 分组建议

### 10.1 PR 必跑

```bash
pnpm --dir web test:e2e -- --project=chromium web/e2e/issue-lifecycle-workspace.spec.ts web/e2e/lifecycle-happy-path.spec.ts web/e2e/workspace-streaming.spec.ts
```

特点：

- 只使用 fake provider。
- 总时长应控制在 2 到 4 分钟。
- 覆盖主业务路径和最关键回归。

### 10.2 Nightly 或手动门禁

```bash
ARIA_E2E_PROVIDER_FIXTURES=1 pnpm --dir web test:e2e -- web/e2e/workspace-permissions.spec.ts web/e2e/workspace-recovery.spec.ts web/e2e/workbench-visual.spec.ts
```

特点：

- 使用 Claude/Codex fixture provider。
- 覆盖权限、执行事件、恢复能力和响应式布局。

### 10.3 不建议默认跑真实 CLI E2E

真实 Claude Code / Codex CLI 端到端可以作为人工验收脚本，不建议进入默认 CI：

- 依赖本机登录态或 API key。
- 输出不可完全稳定。
- 时长和费用不可控。
- provider 产品协议变化会造成外部噪声。

## 11. 与现有测试的覆盖边界

| 能力 | 已有覆盖 | E2E 补充 |
|------|----------|----------|
| 四列默认工作台 | `issue-lifecycle-workspace.spec.ts` | 保留并增强 URL/旧文案断言 |
| fake provider Story 生成 | `fake-workbench.spec.ts` | 扩成 Story → Design → Work Item 主链路 |
| Workspace WebSocket 协议 | `tests/workspace_ws_integration.rs` | 浏览器层确认 UI 状态和用户操作 |
| 权限确认 bridge | `approval_bridge` 与 `workspace_ws_integration` | 浏览器权限卡片与按钮 |
| Codex command event | `workspace_ws_integration` | 执行 tab 可视化 |
| rollback/abort/interrupt | `workspace_ws_integration` | 浏览器回退按钮、中止按钮、输入框恢复 |
| 响应式布局 | `workbench-visual.spec.ts` | 加入完整主路径后的 mobile 检查 |

## 12. 风险与缓解

| 风险 | 影响 | 缓解 |
|------|------|------|
| 默认 provider registry 指向真实 `claude`/`codex` | CI 不稳定 | 增加 E2E fixture provider 环境变量 |
| 流式输出时序不稳定 | 偶发失败 | 等待业务 UI 状态，不等待固定毫秒 |
| lifecycle 数据跨测试污染 | 卡片误匹配 | 每测唯一 Project，断言精确标题 |
| UI 文案频繁调整 | 选择器脆弱 | 使用 role、aria-label、稳定表单 label |
| E2E 过度覆盖协议细节 | 慢且重复 | 协议细节留给 Rust 集成测试 |
| WebSocket 失败不易诊断 | 排查成本高 | 保留 trace、server logs、失败时 `.aria` artifacts |

## 13. 开放问题

以下问题需要流程侧确认，确认后可调整用例优先级：

1. P0 主链路是否必须用纯 UI 创建 Project/Repository/Issue，还是允许 API seed 后专注验证 Workspace 主流程？
2. Story → Design → Work Item 是否要求一条 E2E 完整串完，还是拆成三个较短用例以降低失败定位成本？
3. Supervised 权限拒绝后的产品期望是“provider failed 回到准备上下文”，还是“provider 继续并展示拒绝原因”？当前 E2E 只能按实现断言错误/可恢复状态。
4. Work Item 当前确认映射为 `plan_status=confirmed`，是否已经代表本版本的 Work Item 完成口径？
5. 是否需要一条人工验收专用的真实 Claude/Codex CLI smoke，不进入 CI，只在发布前本机执行？

## 14. 分阶段落地建议

### 阶段一：P0 fake provider 主链路

新增 `lifecycle-happy-path.spec.ts`，覆盖：

- UI 创建 Project/Repository/Issue。
- Issue → Story Workspace → confirm。
- confirmed Story → Design Workspace → confirm。
- confirmed Design → Work Item Workspace → confirm。
- 返回看板后四列状态正确。

### 阶段二：Workspace 恢复与操作安全

新增 `workspace-recovery.spec.ts`，覆盖：

- reload 后 session state 恢复。
- rollback 到第一轮 checkpoint。
- abort 不创建 partial assistant message。
- 第二条用户消息打断第一条长流。

### 阶段三：Provider fixture 可见行为

新增 `workspace-permissions.spec.ts`，覆盖：

- Claude fixture permission allow。
- Claude fixture permission deny。
- Codex fixture execution event。

该阶段前置条件是 API server 能在 E2E profile 下注册 fixture provider 命令。

### 阶段四：视觉与诊断强化

增强 `workbench-visual.spec.ts`：

- 对四列看板、Workspace、权限卡片、执行 tab 分别做 desktop/mobile overflow 检查。
- CI 开启失败 trace/screenshot/video。

## 15. 验收标准

方案落地后，E2E 测试套件应满足：

- PR 必跑套件覆盖 P0 fake provider 主链路并稳定通过。
- Provider fixture 套件能在无真实 Claude/Codex 登录态的 CI 中运行。
- 每个用例有唯一测试数据，不依赖执行顺序。
- 任一失败能通过 Playwright trace、API server log、临时 `.aria` 数据定位。
- E2E 总体不重复 Rust 集成测试的协议细节，只验证浏览器端用户可见结果。
