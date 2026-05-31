# product-workbench-issue-lifecycle E2E 测试方案、用例与矩阵

## 文档信息

- 文档类型：计划文档 / E2E 测试方案
- 版本：v1.1
- 日期：2026-05-19
- 适用分支：`product-workbench-issue-lifecycle`
- 工作区：`.worktrees/product-workbench-issue-lifecycle`
- 目标代码库：`/Users/michaelche/Documents/git-folder/github-folder/naruto`
- 方案范围：只定义测试方案、测试用例和测试矩阵；不执行测试，不修改生产代码。

## 1. 分支进度理解

当前分支已经把产品入口从旧执行工作台切到 Issue 生命周期工作台：

- `/` 进入 `/workbench`，主页面为四列看板：`Issue`、`Story Spec`、`Design Spec`、`Work Item`。
- Project、Repository、Issue、Story、Design、Work Item 都有产品索引和生命周期 API。
- Issue 创建必须绑定 Repository。
- Story、Design、Work Item 通过 `*:generate` API 创建对应实体和 Workspace Session。
- `/workbench/workspace/:sessionId` 是全屏对话式 Workspace，通过 WebSocket 驱动流式 provider 交互。
- WebSocket 已覆盖 `session_state`、`stream_chunk`、`message_complete`、`artifact_update`、`stage_change`、`provider_status`、`permission_request`、`execution_event`。
- `HEAD` 已修复人工 E2E 中暴露的几个关键问题：Workspace 有显式“开始生成”按钮；fake provider 生成候选 Markdown，不回显系统上下文；卡片“选中”和“打开 Workspace”已拆开；Story/Design 会写入版本并回填 preview。

当前仍需明确的边界：

- Web 生命周期工作台当前主要验证 Story/Design/Work Item 文档链路，不等价于完整代码开发闭环。
- Coding Workspace 的代码 diff、测试结果、review/rework/final UI 闭环仍是后续边界。
- 如果要证明 `naruto` 中实际落地 Python 代码并运行测试，应通过 `aria task run` 真实 provider L4 门禁，或等待 Coding Workspace 接入后再纳入浏览器 E2E。

## 2. 测试目标

1. 验证用户能在浏览器中完成 Project、Repository、Issue 到 Story/Design/Work Item 的生命周期主链路。
2. 验证必选用例“爬楼梯问题”在 `naruto` Repository 上能被完整带入 Workspace 上下文，并产生可确认的 Story/Design/Work Item 产物。
3. 验证 Workspace WebSocket 的用户可见行为：开始生成、流式输出、artifact 更新、人工确认、返回看板同步、回退、中止、刷新恢复。
4. 验证真实 streaming provider 在 UI 层的关键可见行为：权限卡片、允许/拒绝、provider 状态、Codex command execution event。
5. 验证错误和约束路径：缺 Repository、Repository 被删除、非法 session、provider 不可用。
6. 区分浏览器 E2E、Rust 集成测试、真实 provider L4 门禁的覆盖边界，避免重复测试私有协议细节。

## 3. 非目标

- 不在 PR 必跑 E2E 中调用真实 Claude Code 或 Codex 账号。
- 不在浏览器 E2E 中证明 Claude `stream-json` 或 Codex `app-server` 的全部协议分支；协议细节保留在 Rust 集成测试。
- 不把 fake provider 主链路当作真实代码开发完成证明。
- 不手工修改 `naruto` 代码来制造通过结果。
- 不执行本文中列出的测试命令；本文只作为方案与矩阵。

## 4. 测试对象与目标数据

### 4.1 目标 Repository

`naruto` 当前状态：

- 路径：`/Users/michaelche/Documents/git-folder/github-folder/naruto`
- 分支：`main`
- 工作区：干净
- 代码结构：基本为空，只有 README、OpenSpec 配置、规则目录和空 `.aria/runtime`

测试含义：

- 浏览器生命周期测试应把 `naruto` 当作“从零开始”的目标仓库。
- 不能假设已有 Python 工程、测试框架或源码目录。
- 爬楼梯用例中的 Python 实现和测试应作为 Story/Design/Work Item 的明确产物要求；真实代码落地只在 L4 门禁判断。

### 4.2 必选测试 Issue

Issue 标题：

```text
爬楼梯问题
```

Issue 描述：

```text
实现爬楼梯问题：给定 n 阶楼梯，每次可以爬 1 或 2 阶，返回到达楼顶的不同方法数。请使用 python 实现函数 climb_stairs(n: i32) -> i32，并补充测试覆盖 n=1、n=2、n=3、n=5、n=10。
```

业务验收值：

| n | 期望返回 |
|---|----------|
| 1 | 1 |
| 2 | 2 |
| 3 | 3 |
| 5 | 8 |
| 10 | 89 |

Python 落地解释：

- 原始需求中的 `i32` 保留在 Issue 和产物中。
- Python 实现建议映射为 `def climb_stairs(n: int) -> int`。
- 测试用例必须覆盖 `n=1`、`n=2`、`n=3`、`n=5`、`n=10`。

## 5. 测试分层

| 层级 | 类型 | 目标 | 是否进 PR 必跑 |
|------|------|------|----------------|
| L0 | Rust 单元/集成测试 | API、Store、WebSocket 协议、provider fixture、状态机 | 是 |
| L1 | 前端组件/Hook 测试 | Zustand store、hook 消息解析、页面按钮行为 | 是 |
| L2 | Playwright fake provider E2E | 浏览器主链路和 UI 状态同步 | 是 |
| L3 | Playwright fixture provider E2E | 权限卡片、provider status、execution event | 建议 nightly 或手动 |
| L4 | 真实 provider 本机 E2E | `naruto` 真实代码落地、测试执行、最终报告 | 发布前人工门禁 |

本方案的核心是 L2/L3/L4。L0/L1 继续作为已有测试基础，不在 Playwright 中重复协议内部细节。

## 6. 环境与数据策略

### 6.1 浏览器 E2E 环境

沿用现有 Playwright 启动方式：

```bash
pnpm --dir web test:e2e
```

现有配置会启动：

- API：`node ./e2e/start-api.mjs`
- Web：`pnpm dev --port 5173`
- API 端口：`127.0.0.1:4317`
- Web 端口：`127.0.0.1:5173`

### 6.2 Repository 数据策略

两类 Repository 策略分开使用：

| 策略 | 用途 | Repository path |
|------|------|-----------------|
| 临时 repo | CI 稳定 smoke、视觉和恢复测试 | `start-api.mjs` 创建的临时 git repo |
| `naruto` repo | 必选爬楼梯业务链路、真实仓库上下文验证 | `/Users/michaelche/Documents/git-folder/github-folder/naruto` |

PR 必跑建议默认使用临时 repo，避免污染用户真实仓库。专门的 `naruto` 爬楼梯用例在本机或手动门禁执行，执行前必须确认 `naruto` 工作区干净。

### 6.3 Provider 策略

| Provider | 用途 | 自动化建议 |
|----------|------|------------|
| `fake` | P0 主链路、稳定 smoke、快速反馈 | PR 必跑 |
| `claude_code` fixture | 权限请求 UI、允许/拒绝 | Nightly/手动 |
| `codex` fixture | command execution event UI | Nightly/手动 |
| 真实 Claude/Codex | 真实代码落地和测试证据 | 发布前人工门禁 |

要稳定运行 L3，需要后续支持通过环境变量覆盖 provider 命令，例如：

```text
ARIA_E2E_PROVIDER_FIXTURES=1
ARIA_CLAUDE_COMMAND=<repo>/tests/fixtures/provider/claude_stream_json_fixture.sh
ARIA_CODEX_COMMAND=<repo>/tests/fixtures/provider/codex_app_server_current_fixture.sh
```

当前生产默认 registry 仍指向 `claude` 和 `codex`，所以 L3 自动化应先补测试基础设施，不应直接依赖本机登录态。

## 7. P0 主链路详细用例

### E2E-P0-01 默认进入生命周期工作台

前置条件：

- Aria Web 服务启动。

步骤：

1. 打开 `/`。
2. 等待主页面加载。

断言：

- 页面进入 `/workbench`。
- `Issue 生命周期工作台` main region 可见。
- `Issue 列`、`Story Spec 列`、`Design Spec 列`、`Work Item 列`均可见。
- 页面不出现旧工作台文案 `AI Coding Workbench`。

### E2E-P0-02 创建 Project 与 Repository

前置条件：

- 浏览器打开 `/workbench`。
- 测试选择 Repository path：
  - CI smoke 使用临时 repo path。
  - `naruto` 专项使用 `/Users/michaelche/Documents/git-folder/github-folder/naruto`。

步骤：

1. 点击创建 Project。
2. 输入唯一 Project 名称，例如 `E2E Naruto Climb Stairs <timestamp>`。
3. 提交 Project。
4. 点击创建 Repository。
5. 输入 Repository 名称，例如 `naruto`。
6. 输入 Repository path。
7. 提交 Repository。

断言：

- Project sidebar 出现新 Project。
- Repository 列表出现 `naruto` 或对应临时 repo 名称。
- `新建 Issue` 按钮从 disabled 变为可用。

### E2E-P0-03 创建“爬楼梯问题” Issue

前置条件：

- 已创建 Project。
- 已注册 Repository。

步骤：

1. 点击 `新建 Issue`。
2. 选择 Repository。
3. 填写标题 `爬楼梯问题`。
4. 填写必选 Issue 描述。
5. 提交。

断言：

- Issue 列出现 `爬楼梯问题` 卡片。
- 卡片 preview 包含 `给定 n 阶楼梯`。
- Story/Design/Work Item 列暂不出现派生卡片。

### E2E-P0-04 Issue 生成 Story Spec 并确认

前置条件：

- Issue 列存在 `爬楼梯问题`。

步骤：

1. 点击 `爬楼梯问题` Issue 卡片主体，选中 Issue。
2. 点击 `生成 Story Spec`。
3. 自动进入 `/workbench/workspace/:sessionId`。
4. 等待 Workspace 顶部显示 `Story Spec`。
5. 断言系统上下文消息包含：
   - `爬楼梯问题`
   - `climb_stairs`
   - `n=1`
   - `n=2`
   - `n=3`
   - `n=5`
   - `n=10`
   - Repository 路径
6. 点击 `开始生成`。
7. 等待 `Artifact` 非空。
8. 等待 `确认通过` 按钮出现。
9. 点击 `确认通过`。
10. 点击返回。

断言：

- Workspace 经历 `运行中`、`交叉审查`、`人工确认`、`已完成`阶段。
- Story Spec 列出现新卡片。
- Story 状态为 `confirmed`。
- Story preview 包含 `REQ` 或 `AC`，且能追踪到爬楼梯需求。
- 输入框变为 `会话已完成` 并 disabled。

### E2E-P0-05 confirmed Story 生成 Design Spec 并确认

前置条件：

- `爬楼梯问题` 的 Story Spec 状态为 `confirmed`。

步骤：

1. 返回 `/workbench`。
2. 选中 confirmed Story Spec 卡片主体。
3. 点击 `生成 Design Spec`。
4. 进入 Design Workspace。
5. 点击 `开始生成`。
6. 等待 Artifact 非空。
7. 点击 `确认通过`。
8. 返回看板。

断言：

- 未 confirmed Story 时不应出现 `生成 Design Spec`。
- confirmed Story 后按钮可见。
- Design Spec 列出现新卡片。
- Design 状态为 `confirmed`。
- Design 卡片关联来源 Story。
- Design preview 包含设计范围、关键决策或组件/API 信息。

### E2E-P0-06 confirmed Design 生成 Work Item 并确认

前置条件：

- `爬楼梯问题` 的 Design Spec 状态为 `confirmed`。

步骤：

1. 选中 confirmed Design Spec 卡片主体。
2. 点击 `生成 Work Item`。
3. 进入 Work Item Workspace。
4. 点击 `开始生成`。
5. 等待 Artifact 非空。
6. 点击 `确认通过`。
7. 返回看板。

断言：

- 未 confirmed Design 时不应出现 `生成 Work Item`。
- Work Item 列出现新卡片。
- Work Item 关联 Story 和 Design。
- Work Item 中能看到实现、测试、验证命令相关计划语义。
- 当前版本口径下，确认后 `plan_status=confirmed` 或 UI 等价状态可见。

### E2E-P0-07 返回看板状态同步

前置条件：

- 已完成 Story、Design、Work Item 三段确认。

步骤：

1. 从 Workspace 点击返回。
2. 在 Project sidebar 重新点击当前 Project。
3. 点击刷新。

断言：

- Issue 列仍展示 `爬楼梯问题`。
- Story Spec 列展示 confirmed Story。
- Design Spec 列展示 confirmed Design。
- Work Item 列展示对应 Work Item。
- 页面无旧执行工作台入口混入主流程。

## 8. P1 Workspace 稳定性用例

### E2E-P1-01 刷新恢复 session state

步骤：

1. 创建 Story Workspace。
2. 点击 `开始生成`。
3. 等待 assistant 消息、artifact、checkpoint 出现。
4. 刷新页面。

断言：

- WebSocket 重连后推送 `session_state`。
- 历史 user/assistant 消息保留。
- Artifact 保留。
- 回退按钮仍存在。
- 阶段恢复为刷新前等价状态。

### E2E-P1-02 消息级回退

步骤：

1. 在同一 Workspace 完成第一轮生成。
2. 发送第二条用户消息并完成第二轮生成。
3. 点击第一条 assistant 消息的 `回退`。

断言：

- 第二轮 user/assistant 消息被移除。
- Artifact 回到第一轮快照。
- checkpoint 列表与 UI 保持一致。
- 回退后仍可继续发送新消息。

### E2E-P1-03 中止流式生成

步骤：

1. 进入 Workspace。
2. 点击 `开始生成`。
3. 流式输出期间点击 `中止`。

断言：

- provider 状态变为 `已中止` 或阶段回到可恢复状态。
- 不出现可确认的 completed assistant checkpoint。
- 输入框恢复可用。
- 返回看板后不应把部分输出误标为 confirmed。

### E2E-P1-04 新消息打断旧流

步骤：

1. 点击 `开始生成`。
2. 流式输出未完成时输入第二条消息。

断言：

- 旧 run 被取消。
- 最终 artifact 与 assistant 输出来自第二条消息。
- 不出现旧 run 的 late `message_complete` 污染当前状态。

## 9. P1 Provider 可见行为用例

### E2E-P1-05 Claude fixture 权限允许

前置条件：

- E2E provider registry 使用 `claude_stream_json_fixture.sh`。
- Workspace author provider 为 `claude_code`。

步骤：

1. 进入 Story Workspace。
2. 点击 `开始生成`。
3. 等待权限卡片出现。
4. 点击 `允许`。

断言：

- 权限卡片显示 `Bash` 或 fixture tool name。
- provider 状态显示 `等待权限`。
- 点击允许后权限卡片消失。
- 后续收到 assistant 完成消息。
- 最终进入人工确认。

### E2E-P1-06 Claude fixture 权限拒绝

前置条件：

- 同 E2E-P1-05。

步骤：

1. 进入 Story Workspace。
2. 点击 `开始生成`。
3. 等待权限卡片出现。
4. 点击 `拒绝`。

断言：

- 权限卡片消失。
- UI 展示 provider 失败或可恢复错误。
- 输入框恢复可用。
- 不进入 `已完成`。
- 不创建 confirmed Story。

### E2E-P1-07 Codex fixture execution event

前置条件：

- E2E provider registry 使用 `codex_app_server_current_fixture.sh`。
- Workspace author provider 为 `codex`。

步骤：

1. 进入 Story Workspace。
2. 打开右侧 `执行` tab。
3. 点击 `开始生成`。

断言：

- 执行事件出现 `Command`。
- command 为 `pwd`。
- cwd 为 Repository path。
- stdout 包含 Repository path。
- provider 状态最终为 `已完成`。
- Workspace 可进入人工确认。

## 10. P2 错误与约束用例

### E2E-P2-01 无 Repository 时禁止创建 Issue

步骤：

1. 创建 Project。
2. 不创建 Repository。
3. 观察 `新建 Issue` 按钮。

断言：

- `新建 Issue` disabled。
- 不能提交无 Repository 的 Issue。

### E2E-P2-02 Repository 删除后生成 Story 失败可见

步骤：

1. 创建 Project、Repository、Issue。
2. 删除 Repository。
3. 选中 Issue 并尝试生成 Story Spec。

断言：

- UI 展示 `repository_not_found` 或等价错误。
- 不创建孤立 Story 卡片。
- 当前 Issue 不被误标为 completed。

### E2E-P2-03 provider unavailable

步骤：

1. 通过 API seed 一个 author provider 不可用的 Workspace。
2. 进入 Workspace。
3. 点击 `开始生成`。

断言：

- 页面展示 `provider unavailable`。
- 阶段不进入 completed。
- 输入框或错误状态允许用户恢复。

### E2E-P2-04 非法 Workspace session

步骤：

1. 打开 `/workbench/workspace/workspace_session_missing`。

断言：

- 页面展示 session not found 或 WebSocket 连接失败错误。
- 不空白、不无限 loading。

## 11. L4 真实 provider 门禁用例

### E2E-L4-01 `naruto` 爬楼梯真实代码落地

该用例不进入 PR 必跑，只作为发布前人工门禁。

前置条件：

- `naruto` 工作区干净。
- Claude Code/Codex 登录态可用。
- 当前分支构建通过。
- 明确不手工修改 `naruto`。

建议入口：

```bash
cargo run --locked -- task run \
  --workspace /Users/michaelche/Documents/git-folder/github-folder/naruto \
  --request "实现爬楼梯问题：给定 n 阶楼梯，每次可以爬 1 或 2 阶，返回到达楼顶的不同方法数。请使用 python 实现函数 climb_stairs(n: i32) -> i32，并补充测试覆盖 n=1、n=2、n=3、n=5、n=10。" \
  --change-id aria-climb-stairs-python \
  --providers real \
  --timeout 2400 \
  --report json \
  --non-interactive
```

审计断言：

- Aria 生成 `.aria/runtime/tasks/<task_id>`。
- Aria 生成或更新 OpenSpec change。
- `naruto` diff 包含 Python 实现文件和测试文件。
- 实现中存在 `climb_stairs`。
- 测试覆盖 `n=1`、`n=2`、`n=3`、`n=5`、`n=10`。
- runtime 中存在 testing report，且记录了 Aria 触发的测试命令。
- final summary 或 blocked report 可解释最终状态。

失败处理：

- 如果 provider timeout，归类为 provider 执行失败，不手工补代码。
- 如果测试失败，应让 Aria 进入 rework；若没有 rework，记录为 workflow bug。
- 如果没有生成测试证据，不视为 L4 通过。

## 12. 总矩阵

| ID | 优先级 | 层级 | 场景 | 数据 | Provider | 自动化状态 | 核心断言 |
|----|--------|------|------|------|----------|------------|----------|
| E2E-P0-01 | P0 | L2 | 默认工作台 | 临时 repo | 无 | PR 必跑 | `/workbench` 四列可见 |
| E2E-P0-02 | P0 | L2 | 创建 Project/Repository | 临时 repo 或 `naruto` | 无 | PR 必跑/本机专项 | Repository 注册后 Issue 入口可用 |
| E2E-P0-03 | P0 | L2 | 创建爬楼梯 Issue | `naruto` | 无 | 本机专项 | Issue 卡片展示标题和描述 |
| E2E-P0-04 | P0 | L2 | Issue → Story confirmed | `naruto` | fake | 本机专项 | Story confirmed，preview 有 REQ/AC |
| E2E-P0-05 | P0 | L2 | Story → Design confirmed | `naruto` | fake | 本机专项 | Design confirmed，关联 Story |
| E2E-P0-06 | P0 | L2 | Design → Work Item confirmed | `naruto` | fake | 本机专项 | Work Item 出现，plan confirmed |
| E2E-P0-07 | P0 | L2 | 返回看板同步 | `naruto` | fake | 本机专项 | 四列链路状态保留 |
| E2E-P0-08 | P0 | L2 | 响应式布局 | 临时 repo | fake | PR 必跑 | 无横向溢出 |
| E2E-P1-01 | P1 | L2 | 刷新恢复 | 临时 repo | fake | Nightly | session state 恢复 |
| E2E-P1-02 | P1 | L2 | checkpoint 回退 | 临时 repo | fake | Nightly | 消息和 artifact 截断 |
| E2E-P1-03 | P1 | L2 | 中止流式输出 | 临时 repo | fake | Nightly | 不生成 partial confirmed |
| E2E-P1-04 | P1 | L2 | 新消息打断旧流 | 临时 repo | fake | Nightly | 旧 run 不污染新状态 |
| E2E-P1-05 | P1 | L3 | Claude 权限允许 | 临时 repo | claude fixture | 手动/Nightly | 权限卡片允许后完成 |
| E2E-P1-06 | P1 | L3 | Claude 权限拒绝 | 临时 repo | claude fixture | 手动/Nightly | 拒绝后错误可恢复 |
| E2E-P1-07 | P1 | L3 | Codex command event | 临时 repo | codex fixture | 手动/Nightly | `pwd`、cwd、stdout 可见 |
| E2E-P2-01 | P2 | L2 | 无 Repository | 临时 repo | 无 | PR 必跑 | Issue 创建入口 disabled |
| E2E-P2-02 | P2 | L2 | Repository 删除 | 临时 repo | 无 | Nightly | 生成 Story 失败可见 |
| E2E-P2-03 | P2 | L2 | provider unavailable | 临时 repo | 缺失 provider | Nightly | 错误显示且不 completed |
| E2E-P2-04 | P2 | L2 | 非法 session | 临时 repo | 无 | PR 必跑 | 不空白，有错误 |
| E2E-L4-01 | P0 门禁 | L4 | `naruto` 爬楼梯真实代码落地 | `naruto` | real | 发布前人工 | Python 实现、测试、testing report 均存在 |

## 13. CI 与人工门禁建议

### 13.1 PR 必跑

目标：快速发现主流程断裂。

建议范围：

- `E2E-P0-01`
- `E2E-P0-02` 临时 repo 版本
- `E2E-P0-08`
- `E2E-P2-01`
- `E2E-P2-04`

Provider：

- 只用 `fake` 或无 provider。

### 13.2 Nightly

目标：覆盖稳定性和 fixture provider 可见行为。

建议范围：

- `E2E-P1-01` 到 `E2E-P1-07`
- `E2E-P2-02`
- `E2E-P2-03`

前置：

- provider fixture 命令可注入。
- Playwright trace、screenshot、video 失败保留。

### 13.3 发布前人工门禁

目标：证明真实 provider 能在真实仓库完成业务闭环。

建议范围：

- `E2E-L4-01`

通过标准：

- 不是只看到 UI confirmed。
- 必须看到 `naruto` 代码 diff、测试文件、Aria testing report 和最终报告。

## 14. 选择器与诊断规范

选择器优先级：

1. `getByRole` + accessible name。
2. `getByPlaceholder`。
3. 稳定业务文案。
4. 必要时补 `aria-label`，不依赖 CSS class。

等待策略：

- 不使用固定 sleep 判断 provider 完成。
- 等待具体 UI 状态：`确认通过`、Artifact 非空、状态标签、权限卡片、执行事件行。
- 每个测试数据使用唯一 Project 名称，避免匹配旧数据。

失败诊断：

- Playwright trace retain-on-failure。
- screenshot only-on-failure。
- API server stdout/stderr 保留。
- L4 失败时保留 `.aria/runtime`、OpenSpec change、`git diff --stat`。

## 15. 开放确认项

1. `naruto` 爬楼梯专项是否允许使用真实主工作区，还是必须先创建 `naruto` 测试 worktree。
2. `E2E-P0-03` 到 `E2E-P0-07` 是否要进入 PR 必跑；如果进入，需要避免真实 `naruto` 路径依赖。
3. L4 真实门禁是继续使用 `aria task run`，还是等 Coding Workspace UI 完成后再用浏览器驱动。
4. Provider fixture 命令注入是否作为测试基础设施的前置任务优先落地。

