---
name: start-aria-e2e-worktree
description: Use when the user asks to guide Cadence Aria full end-to-end testing from a worktree branch, says worktrees/worktree branch, gives a branch in brackets like 【fix_author_confirm_followup】, or sends a “目标/要求” prompt for aria 全流程端到端测试.
---

# Start Aria E2E Worktree

## 目标

把真实 E2E 测试的启动准备固定成稳定流程：先进入指定 worktree/branch，读取该 worktree 的规则和进度，再从源码启动当前 worktree 的 Aria 前后端服务，健康检查通过后再指导用户页面测试。

## 触发后顺序

1. 提取用户给出的 branch；优先识别 `【branch_name】`。没有 branch 时只问一个问题：要使用哪个 worktree branch？
2. 在主仓库根目录确认 worktree：
   - `git worktree list`
   - 目标路径默认是 `.worktrees/<branch>`。
   - 如果不存在，先 `git fetch origin <branch>`，再 `git worktree add .worktrees/<branch> origin/<branch>`。
   - 如果已存在，进入该 worktree 后执行 `git fetch origin <branch>`；工作区干净时可 `git pull --ff-only`，有本地改动时不要 pull/reset/stash，报告状态后继续按当前 worktree 测。
3. 切换所有后续命令的 `workdir` 到目标 worktree 根目录，不要继续在主工作区执行项目命令。
4. 重新读取目标 worktree 内的规则：
   - `AGENTS.md`
   - `CLAUDE.md`
   - `.claude/rules/` 中本任务相关规则
   - `cadence/project-rules/README.md` 及其“已启用项目规则”列出的文件
5. 确认进度和现场：
   - `git status --short --branch`
   - `git log --oneline -5`
   - 必要时查看 `.aria/projects`、最近 workspace session、当前 diff。
   - 不使用旧测试项目、旧代码库、旧 issue；用户说自己准备时，只把 Aria 服务启动好并让用户在页面添加。
6. 启动前检查工具：
   - `cargo watch --version`
   - `pnpm --version`（在 `web/` 下也可）
   - 缺 `cargo-watch` 时按仓库规则使用宿主机 `cargo install cargo-watch --locked`，不要用 Docker。

## 服务清理

启动前先检查 4317/5173 和旧开发进程：

```bash
pgrep -af "cargo-watch|cargo run|target/debug/aria|pnpm dev|vite"
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
```

如果端口已有本仓库旧服务或重复 watcher：

- 停掉旧的 `cargo-watch`、`target/debug/aria`、`pnpm dev`、`vite` 进程，再从目标 worktree 重启。
- 普通 sandbox 下 `kill` 可能失败或误报；如果 `pgrep` 仍显示旧进程，按工具规范用 `require_escalated` 重新执行同一组 `kill`。
- 如果进程不是本仓库或来源不明，先问用户，不要直接杀。

## 启动命令

后端在 worktree 根目录启动，必须是源码开发模式：

```bash
cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

前端在 `web/` 目录启动：

```bash
pnpm dev --port 5173
```

启动时使用长期 `exec_command` session。后端必须等到看到类似 `aria web listening on http://127.0.0.1:4317`；前端必须等到 Vite 输出 Local 地址。

## 健康检查

启动后必须跑三项：

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

期望：

- 后端返回 `{"status":"ok"}`。
- 前端 `/` 返回 `200 OK`。
- 前端 `/api/health` 经 Vite proxy 返回 `{"status":"ok"}`。
- `pgrep -af "cargo-watch|cargo run|target/debug/aria|pnpm dev|vite"` 只应保留一套当前 worktree 服务。

## 进入 E2E 指导

健康检查通过后，回复用户：

- 当前 worktree 路径和 branch。
- 服务地址：前端 `http://127.0.0.1:5173`，后端 `http://127.0.0.1:4317`。
- 下一步只给第一个测试阶段：让用户打开 `/workbench`，确认显示 `Issue 生命周期工作台`。

不要主动要求用户使用默认 naruto 仓库或默认 issue；如果用户说测试项目、代码库、issue 自己准备，就让用户在页面中按真实路径添加。
