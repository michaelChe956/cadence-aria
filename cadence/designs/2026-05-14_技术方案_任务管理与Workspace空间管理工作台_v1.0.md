# 任务管理与 Workspace 空间管理工作台技术方案

## 背景

当前 Aria Web 首页就是执行工作台。用户在同一个页面中填写任务请求和 change id，然后直接创建 Aria runtime task。这个模式适合验证单个 workspace 的执行链路，但缺少两个产品化入口：

- 任务管理工作台：用户先创建 issue，再决定何时启动执行。
- Workspace 空间管理：用户维护多个代码库路径，启动 issue 时选择一个 workspace，让 Aria 在该代码库中运行。

参考 `vibe-kanban` 的工作流后，本方案只保留最小必要概念：issue、workspace、start、执行工作台。不引入项目、组织、远端同步、PR 状态、多人协作或完整 Kanban。

## 目标

1. 默认首页变为任务管理工作台，支持新建 issue、查看 issue 列表、从 issue 点击 Start。
2. 增加 workspace 管理能力，用户可以登记本地代码库路径，并在 Start 时选择 workspace。
3. Start 后进入现有 Aria workspace workbench，继续复用当前 projection、SSE、推进、确认、回退、证据查看等执行能力。
4. 所有 MVP 元数据本地持久化，不依赖数据库、登录或远端服务。

## 非目标

- 不实现完整 Kanban 列、拖拽状态、多人分派、评论、附件、PR 关联。
- 不实现远端 workspace 同步。
- 不创建独立 worktree 管理器；workspace 就是用户登记的本地代码库路径。
- 不重写现有节点执行工作台，只抽取并复用它。

## 当前系统约束

- `src/web/app.rs` 当前路由已经包含 `/api/tasks`、`/api/projection`、`/api/events`、`/api/files/*` 等执行接口。
- `src/web/runtime.rs` 的 `WebRuntime` 当前绑定单一 `workspace_root`，task 创建、projection、git summary、文件读取和 diff 都默认使用这个路径。
- `web/src/app-shell.tsx` 当前承担整个首页，并直接创建 task。
- `vibe-kanban` 的可借鉴点是 issue 与 workspace 分离：issue 是任务入口，workspace 是执行空间。Aria MVP 只采用这个分层，不复制其复杂平台模型。

## 推荐方案

采用“本地任务中心 + 复用现有 workbench”方案。

后端把 Web 服务的根路径拆成两个概念：

- `app_root`：服务启动目录，用来存储任务中心元数据。
- `workspace_root`：用户登记的代码库路径，用来运行 Aria task。

前端把当前单页拆成两个视图：

- `TaskManagementWorkbench`：默认首页，管理 issue 和 workspace。
- `WorkspaceExecutionShell`：Start 后进入的执行视图，复用当前 `AppShell` 内的 workbench 逻辑。

## 数据模型

### Workspace

字段：

- `workspace_id`：稳定 ID，例如 `workspace_0001`。
- `name`：用户可读名称。
- `path`：本地代码库绝对路径。
- `default_policy_preset`：默认 `manual-write`。
- `default_provider_mode`：默认 `fake`，允许 `real`。
- `created_at`、`updated_at`。

校验：

- `path` 必须存在。
- `path` 必须是目录。
- `path` 必须能被 `git rev-parse --show-toplevel` 识别为 Git 仓库或仓库子目录。

### Issue

字段：

- `issue_id`：稳定 ID，例如 `issue_0001`。
- `title`：必填。
- `description`：可选。
- `status`：`draft`、`started`、`running`、`completed`、`blocked`。
- `workspace_id`：Start 后写入。
- `task_id`：Start 后由现有 Aria runtime 分配并写入。
- `change_id`：Start 时生成或由用户填写；默认由 title slug 派生。
- `created_at`、`updated_at`。

约束：

- 一个 issue MVP 阶段只绑定一个 workspace 和一个 task。
- 已经 started 的 issue 再次点击 Start 时直接进入已有 workbench，不重复创建 task。

## 本地存储

MVP 使用 JSON 文件持久化，路径位于 `app_root/.aria/runtime/web/`：

- `workspaces.json`：workspace registry。
- `issues.json`：issue registry。

现有执行产物继续存储在所选 workspace 下：

- `<workspace_root>/.aria/runtime/tasks/<task_id>/...`

这样任务中心元数据与代码库执行产物分离，避免在每个代码库中复制任务管理首页数据。

## API 设计

新增接口：

- `GET /api/workspaces`：列出 workspace。
- `POST /api/workspaces`：创建 workspace。
- `PATCH /api/workspaces/{workspace_id}`：更新名称、路径或默认设置。
- `GET /api/issues`：列出 issue。
- `POST /api/issues`：创建 issue。
- `POST /api/issues/{issue_id}/start`：选择 workspace 并启动执行；若 issue 已经绑定 task，则返回既有执行上下文。

Start 请求：

- `workspace_id`：必填。
- `policy_preset`：可选，默认取 workspace 设置。
- `provider_mode`：可选，默认取 workspace 设置。
- `timeout_secs`：可选，默认沿用当前 `2400`。

Start 响应：

- `issue_id`
- `workspace_id`
- `task_id`
- `session_id`
- `status`

现有接口调整：

- `/api/projection` 支持 `workspace_id` 与 `task_id` 参数。
- `/api/tasks/{task_id}/advance|confirm|stop|rollback` 必须带 `workspace_id` 查询参数；后端同时校验 `issue registry` 中的 `task_id -> workspace_id` 关系，防止跨 workspace 误操作。
- `/api/files/content` 与 `/api/files/diff` 需要在选中 workspace 中解析路径，继续保留路径安全校验。
- `/api/events` MVP 可继续作为全局事件流，事件 payload 带上 `workspace_id` 与 `issue_id`。

## 前端交互

默认首页布局：

- 桌面端左侧区域、移动端顶部区域：workspace 管理列表和新增 workspace 表单。
- 主区域：issue 列表、新建 issue 表单、Start 按钮。
- issue 行展示 title、status、workspace name、task id、updated time。

Start 流程：

1. 用户点击 issue 的 Start。
2. 如果没有 workspace，提示先新增 workspace。
3. 如果有 workspace，展示 workspace 选择器。
4. 用户选择 workspace 并确认。
5. 前端调用 `POST /api/issues/{issue_id}/start`。
6. 成功后进入 `WorkspaceExecutionShell`，该视图持有 `workspace_id` 和 `task_id`。

执行视图：

- 顶部保留返回任务管理工作台按钮。
- 展示当前 issue title、workspace name、task id。
- 原有推进、确认、回退、Provider stream、Flow rail、Node workspace、Evidence panel 保持功能不变。

## 后端结构

建议新增模块：

- `src/web/workspace_registry.rs`：workspace JSON registry、路径校验。
- `src/web/issue_registry.rs`：issue JSON registry、状态更新。
- `src/web/task_index.rs`：`task_id -> issue_id/workspace_id/workspace_root` 索引能力，可由 issue registry 派生。

调整模块：

- `src/web/state.rs`：保存 `app_root`，并让 runtime 调用按 workspace 动态创建 `WebRuntime`。
- `src/web/runtime.rs`：保留现有单 workspace runtime，调用方按 workspace path 实例化 `WebRuntime`。
- `src/web/handlers.rs`：新增 workspace/issue handler，现有 task/projection handler 支持 workspace 解析。

## 错误处理

需要明确返回的错误码：

- `workspace_not_found`
- `workspace_path_missing`
- `workspace_path_not_directory`
- `workspace_path_not_git_repo`
- `issue_not_found`
- `issue_missing_workspace`
- `issue_start_failed`
- `task_workspace_not_found`

UI 行为：

- 表单级错误显示在当前 panel 内。
- 全局执行错误沿用现有 error banner。
- 已启动但 task 文件缺失时，issue 行显示 blocked，并提供“重新选择 workspace 后恢复”的后续扩展点；MVP 只展示错误，不自动修复。

## 测试策略

后端测试：

- workspace registry 可以创建、更新、列出 workspace。
- workspace path 校验覆盖不存在路径、非目录、非 Git repo、合法 Git repo。
- issue registry 可以创建、列出、Start 后写入 workspace/task。
- Start 已启动 issue 不重复创建 task。
- projection 可以根据 `workspace_id + task_id` 读取正确 workspace。

前端测试：

- 默认渲染任务管理工作台，而不是直接渲染执行 workbench。
- 新增 workspace 后出现在 workspace 列表。
- 新增 issue 后出现在 issue 列表。
- issue Start 时必须选择 workspace。
- Start 成功后进入执行视图，并显示 issue、workspace、task 信息。
- 执行视图中的推进、确认、SSE 刷新能力继续可用。

端到端验证：

- 启动 Web 服务。
- 添加当前仓库作为 workspace。
- 新建 issue。
- Start issue。
- 进入执行工作台。
- 使用 fake provider 推进到确认节点并完成一次确认。

## 迁移与兼容

- 首次启动时，如果 `workspaces.json` 不存在，可以自动把 `app_root` 注册为默认 workspace，名称为当前目录名。
- 如果 `issues.json` 不存在，返回空 issue 列表。
- 现有 `/api/tasks` 可以继续用于执行视图，但任务管理首页优先使用 `/api/issues`。

## 验收标准

1. 用户打开前端默认看到任务管理工作台。
2. 用户可以添加至少一个本地 Git workspace。
3. 用户可以新建 issue。
4. 用户点击 Start 时必须选择 workspace。
5. Start 成功后进入现有 workspace workbench。
6. workbench 的推进、确认、回退、projection 和事件流仍然可用。
7. 重启服务后 workspace 和 issue 列表仍然存在。
