# Aria Web 工作台与逐节点交互 Runtime 设计

## 文档信息

- 文档类型：技术方案
- 版本：v1.0
- 日期：2026-05-09
- 适用仓库：`cadence-aria`
- 目标命令：`aria web`
- 第一版范围：本地单机、单 workspace、真实逐节点确认闭环
- 后续演进：桌面端壳与多 workspace 管理
- 相关背景文档：
  - `cadence/designs/2026-05-07_技术方案_Aria_TUI工作台与可回退交互Runtime设计_v1.0.md`
  - `cadence/analysis-docs/2026-05-07_状态记录_Aria_Fibonacci真实E2E_main本地Rust与TUI准备_v1.0.md`
- 参考项目：`/Users/michaelche/Documents/git-folder/github-folder/vibe-kanban`

## 背景

当前 Aria CLI 已具备 `daemon status/run`、`repl`、`task run --non-interactive` 和初步 `tui` 入口。TUI 设计与实现已经沉淀了 `interactive` core 的早期模型，包括 `WorkspaceProjection`、`TaskSession`、`InteractionTurn`、`NodeRun`、`RuntimeCheckpoint`、policy、checkpoint/rollback 和 provider step 暂停点。

Fibonacci 真实 E2E 案例显示，仅靠 CLI 末尾报告不足以解释真实运行状态：整体状态为 `blocked_by_gate`，但业务源码和测试已经通过，最终阻塞来自归档 worktask 的 write scope contract 问题。Web 工作台需要把节点输入输出、使用过程、文档沉淀物、provider 运行、diff、测试结果、gate 诊断和回退点放在同一个可操作界面里。

用户确认的第一版边界：

- 采用本地单机 Web 工作台，不做云端/多用户。
- 通过 `aria web --workspace <PATH>` 绑定一个 workspace。
- Web 页面必须能完成真实逐节点暂停、确认、执行、观察、回退、编辑 prompt 后重跑。
- Claude Code 和 Codex 节点执行前必须展示 Codex-like 输入框，由用户确认执行。
- 回退交互参考 vibe-kanban 的 workspace 思路：对每次回答不满意时可恢复到上一个版本，但不照搬其视觉设计。
- 桌面端作为后续演进目标，不进入第一版实施范围。

## 目标

1. 新增 `aria web --workspace <PATH> [--host HOST] [--port PORT]`，启动本地 Web 服务并托管前端工作台。
2. 页面包含 TUI 规划的全部信息域：Overview、Timeline、IO、Artifacts、Changes、Diagnostics、Action 输入。
3. 支持新建任务、继续已有任务、逐节点推进、provider 节点前暂停确认。
4. 清晰展示每个节点的输入、输出、使用过程、文档沉淀物、provider run、stdout/stderr、结构化输出、报告和 diff。
5. 支持 checkpoint 级回退：恢复 Git workspace 与 Aria runtime 边界，后续历史标记为 `dropped=true`。
6. 保持 `aria task run --non-interactive` 行为兼容。
7. 前端视觉采用现代高密度工作台风格，优先排查效率和执行安全感。

## 非目标

- 第一版不做多 workspace 列表或项目管理。
- 第一版不做云端部署、多用户权限、团队协作或账号系统。
- 第一版不做桌面应用打包，桌面壳保留为后续演进。
- 第一版不承诺回退 provider 在仓库外产生的副作用。
- 第一版不做任意文件级撤销，只做 turn/checkpoint 级恢复。
- 第一版不替代 IDE，只展示与当前 Aria task 相关的 diff、文件内容和产物。

## 总体方案

采用“Rust 本地 Web 服务 + React/Vite SPA + Interactive Runtime Core”的方案。

`aria web` 不通过 shell 包装 `aria task run`。原因是现有 `task run --non-interactive` 无法满足真实逐节点暂停确认。Web 服务应直接复用并扩展 `interactive` core、provider adapter、checkpoint/rollback 和 task orchestration，把现有非交互流程拆为可暂停的 step runner。

分层如下：

| 层级 | 职责 | 第一版实现 |
|------|------|------------|
| Web 前端 | 工作台 UI、节点导航、输入确认、artifact/diff 预览、回退确认 | React + Vite + TypeScript |
| Web API | HTTP API、SSE 事件流、静态资源托管、错误标准化 | Rust `axum` |
| Interactive Runtime Core | projection、session、turn、node run、policy、checkpoint、rollback | 复用并扩展现有 `src/interactive` |
| Step Runner | 逐节点推进、provider 节点暂停、确认后执行 | 从 `task_run` orchestration 中拆出 |
| Existing Runtime | ProviderAdapter、OpenSpec、artifact/report、runtime event log | 保持兼容并复用 |
| Workspace | `.aria/runtime`、OpenSpec、源码、测试、Git worktree | 事实数据源 |

启动形态：

```bash
aria web --workspace /path/to/project
```

默认只监听 `127.0.0.1`，端口可自动选择或通过 `--port` 指定。生产构建后的前端静态资源由 Rust 服务托管。开发阶段可使用 Vite dev server 代理 API。

## 页面信息架构

Web 第一屏就是工作台，不做 landing page。

建议整体布局为“左侧节点流程 + 中央当前节点工作区 + 右侧证据面板 + 底部执行输入框”。

### 顶部状态栏

展示：

- workspace path
- task_id
- change_id
- phase/status
- 当前节点和当前 worktask
- policy preset
- provider mode
- git branch/head/dirty
- 运行状态、SSE 连接状态

当状态为 `blocked_by_gate` 时，顶部不只显示失败，还应区分：

- E2E overall
- business code
- unit tests
- coverage gate
- archive/integration gate
- root cause

Fibonacci 样本应展示为：

```text
E2E overall: blocked_by_gate
Business code: generated
Unit tests: passed
Coverage gate: passed
Archive worktask: failed
Root cause: cadence/ write scope missing
```

### 左侧 Flow Rail

展示 N00-N28 节点流：

- 当前节点
- completed/running/blocked/failed/dropped 状态
- provider 类型：Claude Code、Codex、internal
- 重复执行次数和 rework 次数
- 关键产物数量
- gate/diagnostic 标记

点击节点后，中央区域切换到该节点详情。

### 中央 Node Workspace

按 tabs 展示当前节点：

| Tab | 内容 |
|-----|------|
| Overview | 节点目标、角色、provider、状态、耗时、失败路由、completion criteria |
| Inputs | canonical inputs、projection refs、context files、allowed write scope、prompt snapshot |
| Run | provider 调用过程、stdout/stderr、structured output、manual gate、retry 信息 |
| Outputs | artifact refs、生成文档、reports、源码、测试、traceability refs |
| Diff | 本节点 changed_files、checkpoint 前后 diff、文件级摘要 |

### 右侧 Evidence Panel

常驻展示当前任务和当前节点的证据链：

- OpenSpec proposal/spec/design/tasks
- Aria artifacts
- reports
- provider run records
- testing report
- final/blocked report
- node-events.jsonl
- 生成源码和测试

点击条目后在同一栏预览 Markdown、JSON、source、test、log。JSON 默认折叠长字段，Markdown 支持目录和锚点。

### 底部 Action Composer

Claude Code/Codex 节点暂停时出现 Codex-like 输入区：

- 显示待发送 prompt，可编辑或追加指令。
- 显示 input summary、output schema、allowed write scope。
- 支持确认执行、请求修改 prompt、停止、回退上一轮。
- 执行中显示 provider streaming output 和当前 node/run 状态。

内部节点或自动执行阶段，底部区域显示当前自动动作、事件流摘要和可停止操作。

### Changes / Workspace 交互

参考 vibe-kanban 的 workspace 版本思路：

- 每轮用户确认形成一个 turn。
- 每轮 turn 对应一个 checkpoint 和一组 changed files。
- 用户能看到回答、文件变化、diff 和产物。
- 对不满意的回答，可“恢复到该轮前 checkpoint”，后续历史灰显为 dropped。
- 回退后自动回到对应节点的 Action Composer，允许编辑 prompt 后重跑。

## 逐节点执行设计

现有 orchestration 需要拆成可暂停状态机。核心循环为：

1. 计算下一节点。
2. 若内部节点可自动执行，则执行并写入 node run/event/artifacts。
3. 若 provider 节点需要确认，则构造 `PendingProviderStep`，创建 checkpoint，返回 `paused_for_approval`。
4. 用户确认后执行 provider。
5. 写入 provider run、turn、node run、artifacts、reports、events。
6. 刷新 projection，并继续等待下一次 `advance`。

### 创建/继续任务

用户在 Web 输入：

- request
- change-id
- policy preset
- provider mode
- timeout

服务创建 task/session，并初始化 OpenSpec 和 runtime state。已有 task 可直接 browse/continue。

### Policy Preset

沿用 TUI 设计：

| Preset | 行为 |
|--------|------|
| `manual-all` | 所有 provider 节点暂停确认 |
| `manual-write` | 写 runtime 或 workspace 的节点暂停确认，纯只读节点自动 |
| `auto-review` | 规划和编码确认；review/testing/final summary 自动 |
| `non-interactive` | 接近当前 CLI 行为，不暂停确认 |

第一版默认 `manual-write`。用户可在单节点临时 override。

### Provider 节点确认

暂停时返回：

- node_id
- provider_type
- runtime_role
- adapter_role
- prompt
- input_summary
- canonical input refs
- context files
- output_schema
- allowed_write_scope
- forbidden_actions
- verification_commands
- checkpoint_id

前端展示并允许编辑 prompt。确认后，服务把最终 prompt 传给 provider adapter。

### 事件流

使用 SSE 作为第一版事件通道。浏览器断线后通过 `/api/projection` 恢复。

事件类型：

| 事件 | 说明 |
|------|------|
| `projection_updated` | projection 可刷新 |
| `node_started` | 节点开始 |
| `node_completed` | 节点完成 |
| `node_failed` | 节点失败 |
| `paused_for_approval` | provider 节点等待确认 |
| `provider_output` | stdout/stderr 增量或摘要 |
| `artifact_written` | 产物写入 |
| `gate_blocked` | gate 或 contract 阻塞 |
| `checkpoint_created` | checkpoint 创建 |
| `rollback_previewed` | 回退预览生成 |
| `rollback_completed` | 回退完成 |
| `error` | 标准化错误 |

## 数据模型

复用 TUI 设计的数据模型，并为 Web 补充字段。

### `WorkspaceProjection`

Web 的主要读取结构：

- `workspace_root`
- `active_task_id`
- `active_session_id`
- `overview`
- `sessions`
- `timeline`
- `artifact_index`
- `diagnostics`
- `available_actions`
- `pending_provider_step`
- `selected_node_context`
- `git_summary`
- `event_cursor`

`projection.json` 是可重建缓存，不作为唯一事实来源。

### `InteractionTurn`

每次用户确认 provider 节点前创建：

- `turn_id`
- `session_id`
- `node_id`
- `provider_type`
- `prompt_snapshot`
- `input_summary`
- `checkpoint_before`
- `provider_run_id`
- `output_artifact_refs`
- `changed_files`
- `status`
- `dropped`
- `created_at`
- `updated_at`

### `RuntimeCheckpoint`

每个可回退点记录：

- `checkpoint_id`
- `task_id`
- `session_id`
- `turn_id`
- `git_head`
- `dirty_summary`
- `state_snapshot_ref`
- `projection_snapshot_ref`
- `artifact_boundary`
- `provider_run_boundary`
- `node_run_boundary`
- `created_at`

### `ArtifactIndexEntry`

统一索引输入输出和沉淀文档：

- `artifact_ref`
- `artifact_kind`
- `producer_node`
- `path`
- `summary`
- `status`
- `content_type`
- `traceability_refs`
- `dropped`

## 回退设计

回退语义为“恢复到该轮之前”。

流程：

1. 用户在 Timeline、Node Workspace 或 Changes 视图选择某个 turn/checkpoint。
2. 调用 rollback preview。
3. 服务返回：
   - 目标 checkpoint
   - 将 reset 到的 git SHA
   - 当前 dirty 状态
   - 将 dropped 的 turn/node/provider/artifact 数量
   - 可能丢弃的文件变化
4. 如果 dirty，用户必须显式勾选允许丢弃未提交变更。
5. 服务执行 rollback：
   - 恢复 Git worktree 到 checkpoint `git_head`
   - 恢复 runtime state/projection 边界
   - 后续 turn/node/provider/artifact 标记 `dropped=true`
   - 刷新 projection
6. 前端灰显 dropped 历史，并把 Action Composer 定位到目标节点。

重要约束：

- 第一版只承诺 Git workspace 和 Aria runtime。
- 不静默执行 hard reset。
- checkpoint 缺失时不显示回退动作。
- 回退失败必须可诊断。

## Web API 契约

第一版 API 保持小而完整。

```text
GET  /api/health
GET  /api/projection?task_id=
GET  /api/tasks
POST /api/tasks
POST /api/tasks/{task_id}/advance
POST /api/tasks/{task_id}/confirm
POST /api/tasks/{task_id}/rollback/preview
POST /api/tasks/{task_id}/rollback
GET  /api/artifacts/{artifact_ref}
GET  /api/files/content?path=
GET  /api/files/diff?base_checkpoint=&path=
GET  /api/events
```

### `POST /api/tasks`

请求：

```json
{
  "request_text": "用 JavaScript 写一个函数...",
  "change_id": "aria-fibonacci-square",
  "policy_preset": "manual-write",
  "provider_mode": "real",
  "timeout_secs": 2400
}
```

响应：

```json
{
  "task_id": "task_0001",
  "session_id": "sess_task_0001",
  "change_id": "aria-fibonacci-square",
  "phase": "intake"
}
```

### `POST /api/tasks/{task_id}/advance`

响应可能为：

```json
{
  "status": "paused_for_approval",
  "pending_step": {
    "node_id": "N16",
    "provider_type": "codex",
    "prompt": "...",
    "input_summary": {},
    "output_schema": "schema://aria/artifacts/coding_report/v1",
    "allowed_write_scope": ["src/", "tests/"],
    "checkpoint_id": "ckpt_0001"
  }
}
```

或：

```json
{
  "status": "advanced",
  "projection_version": 42
}
```

### `POST /api/tasks/{task_id}/confirm`

请求：

```json
{
  "checkpoint_id": "ckpt_0001",
  "prompt": "最终确认后的 prompt"
}
```

响应：

```json
{
  "status": "provider_started",
  "node_id": "N16",
  "turn_id": "turn_0001"
}
```

### `POST /api/tasks/{task_id}/rollback/preview`

请求：

```json
{
  "checkpoint_id": "ckpt_0001"
}
```

响应：

```json
{
  "checkpoint_id": "ckpt_0001",
  "git_head": "abc1234",
  "dirty": false,
  "turns_to_drop": 3,
  "node_runs_to_drop": 7,
  "provider_runs_to_drop": 4,
  "artifacts_to_drop": 6,
  "files_may_change": ["src/fibonacciSquareSum.js"]
}
```

## 错误处理

所有 API 错误统一返回：

```json
{
  "code": "checkpoint_unsafe_dirty_worktree",
  "message": "worktree has uncommitted changes",
  "details": {}
}
```

错误分类：

| 分类 | 示例 |
|------|------|
| `provider_error` | provider 命令缺失、未登录、权限拒绝、超时、结构化输出解析失败、非零 exit code |
| `gate_blocked` | manual gate、rework limit、write scope 冲突、contract 阻塞 |
| `validation_failed` | artifact schema、OpenSpec、traceability、phase profile 校验失败 |
| `checkpoint_unsafe` | checkpoint 缺失、dirty worktree 未确认、git reset 失败、runtime 边界恢复失败 |
| `web_runtime_error` | 端口占用、workspace 不可写、静态资源缺失、SSE 断连 |

Web 必须把错误归类到 Diagnostics Panel，而不是只弹 toast。

## 技术选型

### 后端

- Rust `axum`
- `tokio`
- `serde` / `serde_json`
- SSE 优先，WebSocket 保留为后续需要双向 streaming 时的选项

选择理由：

- 与现有 Rust/tokio/serde 代码一致。
- `axum` 支持清晰的 routing、JSON extractor、SSE。
- 本地服务比 shell 包 CLI 更适合逐节点交互状态。

### 前端

- `pnpm`
- React
- Vite
- TypeScript
- TanStack Router
- Tailwind CSS
- Radix/shadcn 风格组件
- lucide icons

选择理由：

- 符合仓库前端包管理规则。
- React/Vite 适合本地 SPA 与后续桌面壳复用。
- TanStack Router 的类型化路由和 search params 适合恢复节点、tab、artifact、turn 选择。
- Tailwind/Radix 适合构建自定义、高密度、可访问的工作台 UI。

### 视觉方向

视觉关键词：高密度、清晰、冷静、科技感、可诊断。

约束：

- 第一屏即工作台。
- 不做营销页。
- 不照搬 vibe-kanban 的视觉。
- 不使用大面积单色深蓝/紫色渐变。
- 不使用纯装饰性背景。
- 卡片只用于重复项、模态框和工具面板，不做卡片套卡片。
- 危险操作必须有影响预览和明确确认。

## 测试策略

遵循 TDD。

### Rust 单元测试

覆盖：

- Web API handler。
- projection 构建。
- interactive controller。
- policy pause/confirm。
- checkpoint rollback preview。
- checkpoint rollback 执行。
- step runner 的 provider pause seam。
- 错误码映射。

### Rust 集成测试

覆盖：

- `aria web --check --workspace <PATH>` 预检。
- handler-level fake provider 闭环。
- 任务创建、advance、paused_for_approval、confirm、artifact 写入、projection 刷新。
- rollback preview 和 rollback。
- `aria task run --non-interactive` 不回归。

### 前端测试

覆盖：

- Action Composer 状态。
- Flow Rail 节点选择和 dropped 灰显。
- Artifact Viewer。
- Rollback Preview Dialog。
- Diagnostics 分类展示。
- URL search params 恢复。

### E2E 验收

1. fake provider 完整跑通新建任务、逐节点确认、产物展示、回退重跑。
2. Fibonacci 真实样本可浏览，并正确识别：
   - overall `blocked_by_gate`
   - business code generated
   - unit tests passed
   - archive worktask failed
   - root cause 为 write scope/contract 阻塞
3. 本地 provider 凭据缺失时，Web 显示 provider authorization/command diagnostics。

## 验收标准

1. `aria web --workspace <PATH>` 能启动本地 Web 服务并显示工作台。
2. Web 能浏览已有 task，包含 Overview、Timeline、IO、Artifacts、Changes、Diagnostics。
3. Web 能新建 task 并真实逐节点执行。
4. Claude Code/Codex 节点执行前暂停，展示 prompt/input/schema/scope，由用户确认。
5. provider 执行过程和产物能实时展示。
6. 每个节点的输入、输出、使用过程和文档沉淀物可追踪。
7. 用户能回退到上一轮或指定 checkpoint，后续历史标记 dropped，编辑 prompt 后重跑。
8. Fibonacci 样本能清晰区分业务测试通过与归档 gate 阻塞。
9. `aria task run --non-interactive` 行为不回归。

## 实施顺序建议

1. 扩展 interactive projection，使 Web 能完整浏览 TUI 所需信息。
2. 拆出真实 step runner seam，支持 provider 节点暂停与确认。
3. 实现 checkpoint rollback preview，补足边界计数和 dirty preflight。
4. 新增 `web` 后端模块和 `aria web` CLI 入口。
5. 实现最小前端工程和 API client。
6. 实现工作台主布局：状态栏、Flow Rail、Node Workspace、Evidence Panel、Action Composer。
7. 接入 SSE 事件流。
8. 实现 rollback dialog 和 dropped 历史展示。
9. 用 fake provider 和 Fibonacci 样本完成验收。

## 后续演进

- 桌面端壳，复用同一 React 工作台和 Rust 本地服务。
- 多 workspace/task 列表。
- WebSocket 双向 streaming。
- 更完整的 diff viewer 和文件树。
- Provider 会话续接和跨 turn conversation 恢复。
- 更细粒度 artifact schema viewer。
- 任务模板和 policy preset 管理。

## 参考资料

- React 官方文档：`https://react.dev/`
- Vite 官方文档：`https://vite.dev/`
- TanStack Router 文档：`https://tanstack.com/router/latest`
- Tailwind CSS 文档：`https://tailwindcss.com/docs`
- axum 文档：`https://docs.rs/axum/latest/axum/`
- vibe-kanban 本地参考：`/Users/michaelche/Documents/git-folder/github-folder/vibe-kanban`
