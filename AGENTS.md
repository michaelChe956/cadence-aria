# AGENTS.md

本文件为 Codex 及其他 AI Agents 在此仓库中工作提供指导。

## 默认角色

- **谨慎执行者**：优先阅读 issue、现有代码和约束，再按指令完成实现、验证与结果汇报。

## 强制规则

> **🔴 必须遵守 - 无例外**
> 详细规则见 `.claude/rules/` 目录下的各规则文件。
> 用户自定义规则见 `cadence/project-rules/` 目录。

### 1. 语言规则
- **必须使用中文回答** → 详见 `.claude/rules/language.md`

### 2. 代码使用规则
- **遵循 TDD 和代码规范** → 详见 `.claude/rules/code-usage.md`

### 3. 文档存储规则
- **Cadence 产物文档必须存放在 `cadence` 目录下；Claude Code 框架规则保留在 `.claude/rules` 目录下** → 详见 `.claude/rules/document-storage.md`

### 4. Markdown 格式规则
- **代码块嵌套使用 4 反引号/3 反引号** → 详见 `.claude/rules/markdown-format.md`

### 5. MCP Server 与工具使用规则
- **各 MCP 工具及相关自动化工具的使用必须遵循项目规范** → 详见 `.claude/rules/mcp-servers.md`

### 6. 项目个性化规则（强制规则）

详见 `cadence/project-rules/README.md`。

其中 **Rust 构建/测试/检查命令规范**（标准命令、🔴 禁止 `-j 1`、定向快反馈命令）详见 `cadence/project-rules/build-test-commands.md`，本地与 CI 必须遵循。

### 7. 代码阅读规则
- **结构化优先，使用 `ast-grep outline` 避免盲读** → 详见 `.claude/rules/code-reading.md`

## 与 CLAUDE.md 的关系

- 用户在当前任务中的明确指令优先级最高。
- `CLAUDE.md` 面向 Claude Code。
- `AGENTS.md` 面向 Codex 及其他通用 AI Agents。
- 两者如有表述差异，应优先遵循本仓库中的实际规则文件，即 `.claude/rules/` 与 `cadence/project-rules/`。

## Agent 执行要求

- 开始任务前，应先读取 `CLAUDE.md`，并按需查看 `.claude/rules/` 与 `cadence/project-rules/` 中的相关规则文件。
- 若当前工作区的 `cadence/project-rules/README.md` 存在“已启用项目规则”章节，必须读取该章节列出的每一个规则文件，并按其内容执行。
- 使用 worktree 开发时，创建或切换到 worktree 后，必须重新读取该 worktree 内的 `AGENTS.md`、`CLAUDE.md`、`.claude/rules/` 与 `cadence/project-rules/README.md`；不得沿用主工作区读取到的旧规则上下文。
- 执行 issue 时，应先读取 issue 与相关上下文，再修改文件。
- 完成任务后，必须汇报测试或验证结果。
