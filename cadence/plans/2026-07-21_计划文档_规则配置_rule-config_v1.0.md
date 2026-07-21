# Rule Configuration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans` or `superpowers:subagent-driven-development` to execute this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 以不覆盖项目既有规则和配置为前提，补齐 Cadence 的 OpenSpec/Superpowers 路由规则，并验证项目级工具与 OpenSpec 契约。

**Architecture:** 保留现有通用规则、技术栈说明和 MCP 配置，仅新增缺失的 L1 协作规则与 L0 入口路由。OpenSpec 配置以现有 YAML 为基础，在临时工作区完成候选合并与四类指令验证后原子发布。

**Tech Stack:** Markdown、YAML、OpenSpec CLI、CodeGraph CLI、Rust、TypeScript/React。

## Global Constraints

- 所有用户可见说明和新增文档使用中文。
- 不覆盖既有普通规则、项目技术栈、MCP 配置或用户自定义内容。
- `cadence/` 保持纳入版本控制；`.codegraph/` 继续保持忽略。
- Playwright 规则未获明确请求，保持跳过。
- OpenSpec 候选必须通过 proposal、design、specs、tasks 四类 `openspec instructions --json` 验证后才能发布。

---

### Task 1: 记录配置计划并核验前置状态

**Files:**
- Create: `cadence/plans/2026-07-21_计划文档_规则配置_rule-config_v1.0.md`
- Inspect: `CLAUDE.md`、`AGENTS.md`、`.claude/rules/`、`openspec/config.yaml`

- [x] **Step 1: 确认项目类型和模板完整性**

Run: `rg --files -g '*.{rs,ts,tsx}' | head -n 1 && test -f Cargo.toml && test -f web/package.json`

Expected: 输出至少一个源码文件，且 `Cargo.toml` 与 `web/package.json` 存在。

- [x] **Step 2: 记录不覆盖的增量边界**

Run: `rg -n 'cadence-managed:openspec-superpowers-routing' CLAUDE.md AGENTS.md || true`

Expected: 当前入口不存在 L0 标记，因此只在首个 `## 强制规则` 前插入模板区块。

### Task 2: 补齐 L1 协作规则与 L0 入口路由

**Files:**
- Create: `.claude/rules/openspec-superpowers-workflow.md`
- Modify: `CLAUDE.md`
- Modify: `AGENTS.md`

- [x] **Step 1: 创建当前 v1 L1 协作规则**

Source: `/home/michaelche/.agents/Cadence-skills/cadence-init/skills/rule-config/references/rules/openspec-superpowers-workflow.md`

Expected: 目标文件包含 `cadence-framework-rule:openspec-superpowers-workflow:v1` 标记。

- [x] **Step 2: 在两个入口插入相同的 L0 受管区块**

Source: `/home/michaelche/.agents/Cadence-skills/cadence-init/skills/rule-config/references/rules/agent-routing-kernel.md`

Expected: 两个文件均包含成对的 `cadence-managed:openspec-superpowers-routing:v1` 开始和结束标记，且区块外原内容保持不变。

- [x] **Step 3: 验证路由引用完整性**

Run: `test -f .claude/rules/openspec-superpowers-workflow.md && rg -n 'cadence-managed:openspec-superpowers-routing:v1:start|openspec-superpowers-workflow.md' CLAUDE.md AGENTS.md`

Expected: 命令以 0 退出，两个入口均有 L0 开始标记和 L1 规则引用。

### Task 3: 核验 CodeGraph 和可选规则策略

**Files:**
- Inspect: `.codegraph/`、`.mcp.json`、`.codex/config.toml`、`.gitignore`

- [x] **Step 1: 检查本地索引状态**

Run: `test -d .codegraph && codegraph status`

Expected: CodeGraph 报告项目已初始化。

- [x] **Step 2: 检查 Claude 与 Codex 的 MCP 配置**

Run: `rg -n '"codegraph"|\[mcp_servers\.codegraph\]' .mcp.json .codex/config.toml`

Expected: 两个配置均匹配 CodeGraph server。

- [x] **Step 3: 确认默认跳过项和忽略策略**

Run: `test ! -e .claude/rules/playwright.md && grep -qxF '.codegraph/' .gitignore && ! grep -qxF 'cadence/' .gitignore`

Expected: Playwright 规则未创建，`.codegraph/` 被忽略，`cadence/` 未被忽略。

### Task 4: 保守合并并验证 OpenSpec 契约

**Files:**
- Modify: `openspec/config.yaml`
- Inspect: `/home/michaelche/.agents/Cadence-skills/cadence-init/skills/rule-config/references/openspec/config.yaml`

- [x] **Step 1: 基于现有配置创建候选并追加缺失 context 与 artifact rules**

Candidate must preserve `schema: spec-driven`，新增模板的五行 context 与 proposal、design、specs、tasks 各两条规则；不得创建 `rules.apply`。

- [x] **Step 2: 在临时工作区验证四类 OpenSpec 指令**

Run: `openspec instructions proposal --change cadence-config-validation --json && openspec instructions design --change cadence-config-validation --json && openspec instructions specs --change cadence-config-validation --json && openspec instructions tasks --change cadence-config-validation --json`

Expected: 四条命令均以 0 退出，输出包含公共 context 和对应 artifact rules。

- [x] **Step 3: 原子发布并做最终复核**

Run: `git diff --check && git status --short`

Expected: 没有空白错误；变更仅包含本计划、L1 规则、两个入口 L0 区块、OpenSpec 配置及此前 pre-check 生成的 `.pi/`。

## 执行记录

- 已确认 968 个源码文件及 `Cargo.toml`、`web/package.json`，项目类型为 Coding 项目。
- 已创建 L1 规则，并在独立审查后将两个入口的 L0 受管区块校正为与 v1 模板逐字一致。
- CodeGraph 本地索引已存在；状态显示 1,124 个文件、19,196 个节点，Claude 与 Codex 的本地 MCP 配置均已核验。
- 在 `openspec/.cadence-rule-config.1upR1N/` 临时工作区创建 `cadence-config-validation` change，以 `--change cadence-config-validation` 成功运行 proposal、design、specs、tasks 四类 `openspec instructions --json`；四项均读取公共 context 与对应 artifact rules。临时工作区已在原子发布后清理。
