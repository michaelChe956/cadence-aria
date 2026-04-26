# 横切能力文档：Provider Adapter 接口规范

**文档信息**
- **能力标识**：CC10
- **创建日期**：2026-04-22
- **版本**：v1.0
- **负责人**：Codex

> **评审后实施裁定**：本文件保留为上游横切能力背景说明。Aria 一期实际 Rust 类型、JSON schema、字段命名、`ProviderContextPackage -> AdapterInput` 映射、`AdapterOutput` 落盘规则，以 `cadence/designs/2026-04-26_技术方案_Aria一期评审后实施规格补齐_v1.4.md` 第 4.7.3 章为准。所有跨进程序列化字段统一使用 `snake_case`。

## 1. 能力标识

- **能力 ID**：CC10
- **能力名称**：provider_adapter_spec
- **能力类型**：横切（执行层）

## 2. 能力目的

定义 Aria 与 AI Provider（Claude Code、Codex）之间的统一适配接口规范。一期所有 provider 调用通过 `spawn + CLI` 实现，adapter 层屏蔽不同 provider 的 CLI 差异，为后续升级到 SDK 接入预留空间。

## 3. Adapter 接口定义

### 3.1 统一输入（AdapterInput）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `provider_type` | enum | 是 | `claude_code` / `codex` |
| `role` | enum | 是 | `executor` / `reviewer` / `orchestrator` |
| `worktree_path` | string | 是 | 执行目录（worktree 绝对路径） |
| `prompt` | string | 是 | 提示词（包含上下文和指令） |
| `context_files` | array | 否 | 需要传入的文件路径列表 |
| `output_schema` | object | 否 | 期望的结构化输出格式描述 |
| `timeout` | integer | 否 | 超时秒数（默认 600） |
| `max_retries` | integer | 否 | 最大重试次数（默认由 `effective_policy` 决定） |

### 3.2 统一输出（AdapterOutput）

| 字段 | 类型 | 必填 | 说明 |
|------|------|------|------|
| `exit_code` | integer | 是 | 进程退出码（0 = 成功） |
| `stdout` | string | 是 | 标准输出（截断上限 100KB） |
| `stderr` | string | 是 | 标准错误输出（截断上限 50KB） |
| `duration_ms` | integer | 是 | 执行耗时（毫秒） |
| `structured_output` | object | 否 | 解析后的结构化输出 |
| `parse_error` | string | 否 | 结构化解析失败时的错误信息 |
| `files_modified` | array | 否 | 检测到的文件变更列表 |

## 4. Provider 差异映射

### 4.1 Claude Code

| 维度 | 值 |
|------|---|
| CLI 命令 | `claude` |
| 输入方式 | `--prompt` 参数或 stdin pipe |
| 输出格式 | 纯文本（Markdown） |
| worktree 支持 | 通过 cwd 切换 |
| 会话管理 | 支持 `--session-id` |
| 角色映射 | orchestrator / executor |

### 4.2 Codex

| 维度 | 值 |
|------|---|
| CLI 命令 | `codex` |
| 输入方式 | `--prompt` 参数或 stdin pipe |
| 输出格式 | 纯文本（Markdown） |
| worktree 支持 | 通过 cwd 切换 |
| 会话管理 | 支持 `--session` |
| 角色映射 | executor / reviewer |

## 5. CLI 输出解析规则

### 5.1 结构化提取

adapter 从 CLI stdout 中提取结构化数据的规则：

1. 检查 stdout 中是否包含 JSON 代码块（```json ... ```）
2. 如果包含，解析第一个 JSON 代码块作为 `structured_output`
3. 如果不包含，尝试从 Markdown 结构中提取关键信息（标题、列表、表格）
4. 如果都无法解析，`structured_output` 为 null，`parse_error` 记录原因

### 5.2 截断规则

- stdout 超过 100KB 时，保留前 80KB 和最后 20KB，中间标记 `[...truncated...]`
- stderr 超过 50KB 时，保留前 40KB 和最后 10KB，中间标记 `[...truncated...]`
- 截断时在 `parse_error` 中记录 `output_truncated`

## 6. 并发控制

- adapter 维护一个 provider 并发池
- Claude Code 默认并发上限：2（可通过配置调整）
- Codex 默认并发上限：4（可通过配置调整）
- 超过并发上限的请求排队等待
- 排队超时（默认 300 秒）后标记为 `timeout` 并进入 retry/gate

## 7. 超时策略

- 默认超时：600 秒（10 分钟）
- 执行阶段（N16-N19）超时：900 秒（15 分钟）
- 评审阶段（N08, N18）超时：600 秒（10 分钟）
- 规划阶段（N05, N07, N11）超时：600 秒（10 分钟）
- 超时后：
  1. 发送 SIGTERM 信号
  2. 等待 30 秒
  3. 若仍未退出，发送 SIGKILL
  4. 记录 `timeout` 到 provider run record
  5. 按 `retryable_failure` 处理

## 8. 与节点文档的关联规则

- 所有 Agent 业务节点（类型包含"Agent 业务"的节点）通过本规范与 provider 交互
- 节点文档中"Provider 执行契约"章节的指令，通过本规范的 `prompt` 字段传入
- 节点文档中"输出产物"章节的格式，通过本规范的 `output_schema` 字段描述
- adapter 不负责校验产物格式，产物校验由 `artifact_validate` 横切能力负责
