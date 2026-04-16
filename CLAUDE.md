# CLAUDE.md

本文件为 Claude Code (claude.ai/code) 在此仓库中工作提供指导。

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

### 5. MCP Server 使用规则
- **各 MCP 工具的使用规范** → 详见 `.claude/rules/mcp-servers.md`

### 6. 项目个性化规则（强制规则）

详见 `cadence/project-rules/README.md`。

## 项目配置

> 以下内容由初始化脚本根据项目环境自动检测生成，非通用规则。
> 待办：技术栈确定后需补充，详见 `cadence/notes/2026-04-16_技术栈待补充.md`。

### 包管理器规则
- **前端项目**：必须使用 `pnpm` 作为包管理器
- **Python 项目**：必须使用 `uv` 作为包管理器
- **禁止使用**：npm（前端）、pip（Python）、yarn（前端）

### 项目技术栈
- **语言**：待确定
- **包管理器**：待确定
- **测试命令**：待确定
- **检查命令**：待确定
- **格式化命令**：待确定
- **覆盖率阈值**：80%（默认值）

## 当前日期

Today's date is 2026/04/16。
