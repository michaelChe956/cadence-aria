---
name: prepare-aria-worktree-dev
description: Use when the user asks to prepare or start a Cadence Aria worktree branch, gives a branch in brackets like 【bugfix_branch】, wants dev services started, or wants help observing issues while they operate manually.
---

# Prepare Aria Worktree Dev

## 目标

在指定 worktree 中准备 Cadence Aria 环境，摸清分支进度，启动后端和前端开发服务，然后停下等待用户手动操作。后续只根据用户贴出的日志、截图或现象协助分析；需要改代码时，先给方案，等用户确认后再改。

## 推荐输入

```text
准备并启动 worktree【bugfix_branch】，只启动服务，等我操作。
```

把 `bugfix_branch` 替换成目标分支即可。

## 顺序

1. 提取分支名：优先识别 `【branch_name】`；没有分支时只问“要使用哪个 worktree branch？”。
2. 在主仓库根目录确认 worktree：
   - 运行 `git worktree list`。
   - 目标路径默认 `.worktrees/<branch>`。
   - 不存在时执行 `git fetch origin <branch>`，再 `git worktree add .worktrees/<branch> origin/<branch>`。
   - 已存在时进入该 worktree；不要主动 pull、reset 或 stash，只汇报工作区和 ahead/behind 状态。需要更新分支时先问用户。
3. 后续所有命令都切换到目标 worktree 根目录。
4. 重新读取目标 worktree 内规则：`AGENTS.md`、`CLAUDE.md`、相关 `.claude/rules/`、`cadence/project-rules/README.md` 及其“已启用项目规则”列出的文件。
5. 摸底项目：
   - `git status --short --branch`
   - `git log --oneline -5 --decorate`
   - 查看 `Cargo.toml`、`web/package.json`、路由或启动相关文件，确认入口。
   - 向用户简短说明当前进度、最近改动重点、待观察重点。
6. 启动服务，仅做服务就绪检查，然后停下等待用户操作。

## 启动约束

- 后端和前端都必须以开发模式启动。
- 后端使用宿主机 Rust/Cargo，不使用 Docker。
- 前端使用 `pnpm`，不使用 `npm` 或 `yarn`。
- 缺少 `cargo-watch`、`pnpm` 或依赖时，先停下说明缺什么并请求确认；不要擅自安装依赖或改配置。
- 不主动创建 project、repo、issue、spec、work item 等业务数据。
- 不主动跑浏览器流程测试、Playwright、业务 API 或测试控制 API。
- 只允许服务就绪检查：后端 `/api/health`、前端 `/`、前端代理 `/api/health`。
- 端口被占用时先确认来源；来源不明或需要停止进程时先问用户。不要擅自 kill。
- 启动失败或核心功能崩溃时，收集一次关键日志后停下，不反复重试。

## 启动命令

先确认工具：

```bash
cargo watch --version
pnpm --version
```

后端在 worktree 根目录启动：

```bash
cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

前端在 `web/` 目录启动：

```bash
pnpm dev --port 5173
```

启动后只做就绪检查：

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

通过后汇报：

- worktree 路径和分支。
- 最近提交/当前进度摘要。
- 前端地址 `http://127.0.0.1:5173`，后端地址 `http://127.0.0.1:4317`。
- 建议用户从 `/workbench` 开始手动观察。
- 明确说明：服务已启动，接下来等待用户操作。

## 用户反馈问题后

1. 先复述现象和用户正在操作的位置。
2. 根据用户贴出的日志、截图或错误，定位可能层级：前端、后端、WebSocket、Provider、目标仓库或环境。
3. 需要更多信息时一次只问一个关键问题。
4. 给出原因判断、定位路径和修复方案。
5. 需要改代码、配置、依赖或删除文件时，先等用户确认。
6. 用户确认后再按仓库规则执行 TDD、修改和验证。
