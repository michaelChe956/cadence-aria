# CodingWorkspace 二期 P3：Test Agent Loop

## 文档信息

- 文档类型：计划文档
- 分支：`product-workbench-issue-lifecycle`
- 制定日期：2026-05-28
- 版本：v1.0
- 前置：P2（AttemptRunner 与 Gate 状态基础）
- 产出：Provider 工具事件协议 + Tester 白名单基础 + TestingReport 生成
- 设计文档：`cadence/designs/2026-05-28_技术方案_CodingWorkspace二期改造_v1.0.md` §6
- 设计评审：`cadence/designs-reviews/2026-05-28_设计评审_CodingWorkspace二期改造_v1.0.md`

---

## 一、目标

先补齐 Test Agent Loop 所需的结构化工具协议。当前 `StreamingProviderAdapter` 对 CodingWorkspace 只暴露 `Text/Done/Error`，engine 不能拦截 tool_use，因此不能直接实现白名单 Agent Loop。

1. Provider session 显式暴露 `ToolCall` / `ToolResult`
2. Runner 层执行白名单拦截
3. 现有后端测试命令执行器继续作为基础验证能力
4. TestingReport 汇总命令结果与 Tester 分析
5. 终止条件：全部通过 / 发现 bug / 超时 / 连续失败

---

## 二、任务清单

### 2.1 Provider 工具事件协议（src/cross_cutting/streaming_provider.rs 或新接口）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 1.1 | 定义 CodingWorkspace 可消费的 `ProviderToolCall` / `ProviderToolResult` 事件 | 序列化测试 | 工具事件结构稳定 |
| 1.2 | Provider session 支持 runner 返回 tool_result 后继续运行 | 单元测试 | 可完成一轮工具调用 |
| 1.3 | 兼容现有 Text/Done/Error streaming provider | 单元测试 | 旧 provider 不受影响 |

### 2.2 Tester Provider 基础（src/product/coding_workspace_engine.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 2.1 | 构建 Tester system prompt：包含项目上下文、变更文件列表、可用测试命令 | 单元测试 | prompt 包含必要信息 |
| 2.2 | Agent Loop 主循环：发送 prompt → 解析 tool_call → runner 执行或拒绝 → 返回 tool_result | 集成测试 | 循环正确执行 |
| 2.3 | 保留后端命令执行器作为 fallback | 单元测试 | 无工具协议 provider 仍可测试 |

### 2.3 Tool 白名单机制（src/product/ 新文件或 runner 内）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | 定义 Tester 可用 tool 列表：`run_command` / `read_file` / `list_files` / `search_code` | 单元测试 | 白名单正确定义 |
| 3.2 | tool_use 拦截逻辑：非白名单 tool → 返回 tool_result error（"Tester 不允许修改文件"） | 单元测试 | write_file 被拦截 |
| 3.3 | 连续 3 次违反约束 → 终止 Agent Loop + 输出 warning | 单元测试 | 3 次后终止 |

### 2.4 测试命令推断（src/product/coding_workspace_engine.rs）

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 3.1 | 实现 `infer_test_commands` 函数：检测 Cargo.toml → `cargo test` | 单元测试 | Rust 项目正确推断 |
| 3.2 | 检测 package.json scripts.test → `pnpm test` | 单元测试 | Node 项目正确推断 |
| 3.3 | 检测 pytest.ini / pyproject.toml [tool.pytest] → `pytest` | 单元测试 | Python 项目正确推断 |
| 3.4 | 将推断结果注入 Tester system prompt | 单元测试 | prompt 包含推断命令 |

### 2.5 TestingReport 生成与终止条件

| # | 任务 | 测试先行 | 验收 |
|---|------|---------|------|
| 4.1 | Agent Loop 结束后解析 Tester 最终输出为 TestingReport | 单元测试 | report 结构正确 |
| 4.2 | 超时终止（默认 5 分钟）：tokio::time::timeout → 输出已完成结果 + timeout warning | 单元测试 | 超时后正确终止 |
| 4.3 | 连续 3 次 tool_use 执行失败 → 终止 + 输出错误 summary | 单元测试 | 连续失败后终止 |
| 4.4 | Agent Loop 期间实时推送 CodingChatEntry（tool_call / tool_result） | 集成测试 | 前端实时看到测试过程 |

---

## 三、验收标准

1. `cargo test` 全部通过
2. 手动测试：Coding 完成后进入 Testing → Tester Provider 自动执行测试命令 → 前端看到 tool_call 气泡
3. 手动测试：Tester 尝试 write_file → 被拦截 → 前端看到错误提示
4. 手动测试：测试全部通过 → TestingReport 显示 NoIssue
5. 手动测试：测试有失败 → TestingReport 包含 BugReport 列表

---

## 四、不做的事

- Rework 分析官逻辑（P4）
- 前端 ChatEntryList 复用展示（P5）
- CodeReview / InternalPrReview（P6）
