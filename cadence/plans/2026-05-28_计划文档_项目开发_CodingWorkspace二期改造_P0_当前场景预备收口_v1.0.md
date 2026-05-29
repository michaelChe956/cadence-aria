# CodingWorkspace 二期 P0：当前场景预备收口

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-28
- 版本：v1.0
- 前置：无
- 产出：爬楼梯测试场景所需的 Work Item 上下文、验证命令和 Provider 快速切换链路
- 设计评审：`cadence/designs-reviews/2026-05-28_设计评审_CodingWorkspace二期改造_v1.0.md`

---

## 一、目标

先保证当前测试场景能在现有 CodingWorkspace 架构上闭环，不提前引入 5 角色模型、StageGate runner 或 TestAgentLoop 大改造。

测试场景：

- issue：实现爬楼梯问题
- 测试代码库：`/Users/michaelche/Documents/git-folder/github-folder/naruto`
- 目标函数：`climb_stairs(n: i32) -> i32`
- 覆盖输入：`n=1、n=2、n=3、n=5、n=10`

---

## 二、任务清单

| # | 任务 | 测试先行 | 验收 |
|---|------|----------|------|
| 1 | 修复 fenced artifact 提取，支持 artifact 代码块内部包含普通代码块 | 单元测试 | Work Item 中的 `bash` 验证命令不被截断 |
| 2 | 从 Work Item markdown 的 `## 验证命令` 中提取 fenced code block 命令 | 单元测试 | `uv run python -m unittest discover -s tests -v` 被解析 |
| 3 | Coding session state 返回 `work_item_markdown` 和 `verification_commands` | WS 集成测试 | 连接 Coding WS 即可看到上下文和验证命令 |
| 4 | Coding prompt 注入 Work Item 上下文与验证命令 | Engine 测试 | provider prompt 包含 Work Item 内容 |
| 5 | Coding provider prompt 通过 execution event 暴露 | Engine 测试 | 前端可看到完整 Provider Prompt |
| 6 | `provider_select` 在 PrepareContext 阶段支持 author/reviewer 快速切换 | WS 集成测试 | 切换后 session state 返回新 provider |

---

## 三、验收标准

1. `cargo test --locked --test product_test_executor` 通过。
2. `cargo test --locked --test product_coding_workspace_engine` 通过。
3. `cargo test --locked --test web_coding_ws_handler` 通过。
4. `cargo test --locked -j 1` 通过。
5. 手动或集成场景中，Coding provider 收到的 prompt 包含爬楼梯 Work Item 和 unittest 验证命令。

---

## 四、不做的事

- 不引入 5 角色 Provider 对外协议。
- 不实现 StageGate 倒计时。
- 不实现 TestAgentLoop tool_use 白名单。
- 不调整前端 ChatEntryList 展示架构。

