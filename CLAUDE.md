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

其中 **Rust 构建/测试/检查命令规范**（标准命令、🔴 禁止 `-j 1`、定向快反馈命令）详见 `cadence/project-rules/build-test-commands.md`，本地与 CI 必须遵循。

### 7. 代码阅读规则
- **结构化优先，使用 `ast-grep outline` 避免盲读** → 详见 `.claude/rules/code-reading.md`

## 项目配置

> 以下内容由初始化脚本根据项目环境自动检测生成，非通用规则。
> 待办：技术栈确定后需补充：
> 1. 更新「项目技术栈」和「包管理器规则」部分
> 2. 补充具体的测试命令、检查命令、格式化命令
> 3. 根据最终技术栈调整覆盖率阈值（当前默认 80%）

### 包管理器规则
- **前端项目**：必须使用 `pnpm` 作为包管理器
- **Python 项目**：必须使用 `uv` 作为包管理器
- **禁止使用**：npm（前端）、pip（Python）、yarn（前端）

### 项目技术栈
- **语言**：Rust（edition 2024，工具链固定 `rust-toolchain.toml` 的 1.95.0）
- **包管理器**：Cargo
- **测试命令**：`cargo test --locked`（🔴 禁止 `-j 1`；定向单测用 `cargo test --locked --lib <过滤名>`）
- **检查命令**：`cargo check --locked` / `cargo clippy --all-targets --all-features --locked -- -D warnings`
- **格式化命令**：`cargo fmt --check`（应用：`cargo fmt`）
- **覆盖率阈值**：80%（默认值）
- **命令规范**：详见 `cadence/project-rules/build-test-commands.md`

## 当前日期

Today's date is 2026/04/16。
