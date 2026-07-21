# MCP Configuration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` or `superpowers:subagent-driven-development` to execute this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 以不泄露或覆盖现有凭据为前提，确认项目的 Claude、Codex 与 pi MCP 配置满足 Cadence 基础能力要求。

**Architecture:** `.mcp.json` 是 Claude 与 pi-mcp-adapter 的唯一共享来源；`.codex/config.toml` 仅保留具有 command 的 stdio Server。既有同名配置在普通模式下保持不变，只验证必需结构和本地忽略策略。

**Tech Stack:** JSON、TOML、jq、Python `tomllib`、pi-mcp-adapter。

## Global Constraints

- 不读取、输出、复制或改写 API Key、Token、Authorization Header 等私密值。
- 普通模式下同名冲突保持现有配置；仅报告 Server 名称和占位符状态。
- HTTP MCP 只保留在 `.mcp.json`，不写入 Codex TOML。
- `.mcp.json`、`.codex/config.toml` 与 `.worktrees/` 必须保持在 `.gitignore`。

---

### Task 1: 核验 MCP 规则与 Claude 入口引用

**Files:**
- Inspect: `.claude/rules/mcp-servers.md`
- Inspect: `CLAUDE.md`

- [x] **Step 1: 核验规则段落覆盖**

Run: `rg -n '^### (Time MCP|Context7 MCP|Sequential Thinking MCP|CodeGraph MCP|MiniMax Token Plan MCP)' .claude/rules/mcp-servers.md`

Expected: 基础、CodeGraph、MiniMax 与智普相关规则段落均存在。

- [x] **Step 2: 核验 Claude 摘要引用**

Run: `rg -n 'mcp-servers\\.md' CLAUDE.md`

Expected: CLAUDE.md 引用 `.claude/rules/mcp-servers.md`。

### Task 2: 核验 Claude、Codex 与 pi 配置集合

**Files:**
- Inspect: `.mcp.json`
- Inspect: `.codex/config.toml`

- [x] **Step 1: 验证 `.mcp.json` 结构和必需 Server**

Run: `jq -e '.mcpServers | type == "object"' .mcp.json`

Expected: JSON 解析成功，并包含 `time`、`context7`、`sequential-thinking`、`codegraph`、`zai-mcp-server`、`web-search-prime`、`web-reader`、`zread` 和 `MiniMax`。

- [x] **Step 2: 验证 Codex 仅同步 stdio Server**

Run: `python3 -c 'import tomllib; tomllib.load(open(".codex/config.toml", "rb"))'`

Expected: TOML 解析成功，包含六个 stdio Server；不包含三个 HTTP 智普 Server。

- [x] **Step 3: 保留现有非占位凭据**

Run: `jq -e '.mcpServers.MiniMax.env | has("MINIMAX_API_KEY") and has("MINIMAX_API_HOST")' .mcp.json`

Expected: 仅验证环境变量键存在；不输出或替换其值。

- [x] **Step 4: 核验 pi 复用路径**

Run: `test -d /home/michaelche/.pi/agent/npm/node_modules/pi-mcp-adapter`

Expected: pi-mcp-adapter 已安装，pi 直接复用 `.mcp.json`，无需额外配置文件。

### Task 3: 核验忽略规则与幂等结果

**Files:**
- Inspect: `.gitignore`

- [x] **Step 1: 验证本地配置忽略项**

Run: `grep -qxF '.worktrees/' .gitignore && grep -qxF '.mcp.json' .gitignore && grep -qxF '.codex/config.toml' .gitignore`

Expected: 三条忽略规则均存在，且没有对应的反向规则。

- [x] **Step 2: 运行结构和空白复核**

Run: `jq -e '.mcpServers | type == "object"' .mcp.json && python3 -c 'import tomllib; tomllib.load(open(".codex/config.toml", "rb"))' && git diff --check`

Expected: JSON、TOML 与 Git 空白检查均通过。

## 执行记录

- 9 个必需 Server 均已存在；`.mcp.json` 与 `.codex/config.toml` 已通过结构校验。
- `.claude/rules/mcp-servers.md` 与当前框架模板存在差异，但已覆盖全部必需 MCP 规则段落；普通模式下保持项目现有规则不变。
- `zai-mcp-server` 使用占位符；MiniMax 已存在非占位凭据，按普通模式保留，未输出其值。
- pi-mcp-adapter 已安装，`.worktrees/`、`.mcp.json`、`.codex/config.toml` 均在 `.gitignore` 中且没有反向规则。
