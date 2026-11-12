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
   - **必须确保 worktree 处于受 Git 管控的本地分支**：进入 worktree 后检查 `git status --short --branch`，若显示 `detached HEAD` 或没有本地 `<branch>` 分支，必须执行 `git switch -C <branch> origin/<branch>`，切出本地分支并关联远程。
3. 后续所有命令都切换到目标 worktree 根目录。
4. 重新读取目标 worktree 内规则：`AGENTS.md`、`CLAUDE.md`、相关 `.claude/rules/`、`cadence/project-rules/README.md` 及其“已启用项目规则”列出的文件。
5. 摸底项目：
   - `git status --short --branch`
   - `git log --oneline -5 --decorate`
   - 查看 `Cargo.toml`、`web/package.json`、路由或启动相关文件，确认入口。
   - 向用户简短说明当前进度、最近改动重点、待观察重点。
6. 启动服务，仅做服务就绪检查，然后停下等待用户操作。

## 启动约束

- 后端启动环境**必须**显式配置 `ARIA_API_TRUSTED_ORIGINS`（至少含前端 dev origin，如 `http://127.0.0.1:5173` 与 `http://localhost:5173`）；未配置时**不得声称服务可用**。优先用 `bash scripts/aria-dev-start.sh` 启动，它会自动带上全套受信 origin 并做门禁探测。
- 服务就绪检查必须包含**受守卫端点**的门禁探测：用浏览器同款 `Host`+`Origin` 经前端代理访问一个 `/api/**` 只读端点，HTTP 200 才算就绪。**不得用 `/api/health` 代替**——它在后端 Host 校验之前放行，探测不到来源校验故障（后果与故障史见下文「为什么必须配受信 origin」）。
- 后端使用宿主机 Rust/Cargo，不使用 Docker。
- 前端使用 `pnpm`，不使用 `npm` 或 `yarn`。
- 缺少 `cargo-watch`、`pnpm` 或依赖时，先停下说明缺什么并请求确认；不要擅自安装依赖或改配置。
- 不主动创建 project、repo、issue、spec、work item 等业务数据。
- 不主动跑浏览器流程测试、Playwright、业务 API 或测试控制 API。
- 只允许服务就绪检查：后端 `/api/health`、前端 `/`、前端代理 `/api/health`，以及上一条的受守卫端点门禁探测。
- 端口被占用时先确认来源；来源不明或需要停止进程时先问用户。不要擅自 kill。
- 启动失败或核心功能崩溃时，收集一次关键日志后停下，不反复重试。

## 启动命令

先确认工具：

```bash
cargo watch --version
pnpm --version
```

### 1. 先构建前端产物

后端 `build.rs` 通过 `rust-embed` 在编译期嵌入 `web/dist`，干净 checkout 后必须先构建前端产物，否则后端编译会 panic：

```bash
cd web/
pnpm install   # 仅当 node_modules 缺失且用户确认后执行
pnpm build
```

### 2. 用唯一入口脚本启动双服务（首选）

```bash
bash scripts/aria-dev-start.sh
```

脚本自动完成：导出 `ARIA_API_TRUSTED_ORIGINS`（后端+前端端口 × `127.0.0.1`/`localhost`/hostname 全套 origin，外部已设值追加不覆盖）、`NO_PROXY`/`no_proxy`、flock 串行预编译、后台启动 cargo watch 后端与 vite 前端、等待两者就绪，并做**门禁探测**（浏览器同款 `Host`+`Origin` 经前端代理访问受守卫只读端点，非 200 即非零退出并打印可复制的修复命令）。

换端口（如避开共享实例 4317/5173）：`ARIA_PORT=4327 ARIA_WEB_PORT=5273 bash scripts/aria-dev-start.sh`；只探测已运行的服务不启动：`bash scripts/aria-dev-start.sh --check`。

### 3. 手工启动（仅当脚本不可用；必须自己补齐它的全部门禁）

后端**必须**先导出含前端 dev origin 的 `ARIA_API_TRUSTED_ORIGINS`，再在 worktree 根目录以后台任务启动，**必须设置无超时**（dev server 需要长期运行）：

```bash
export ARIA_API_TRUSTED_ORIGINS="http://127.0.0.1:5173,http://localhost:5173"
cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
# run_in_background=true, disable_timeout=true
```

然后轮询后端 `/api/health`，直到返回 `{"status":"ok"}`：

```bash
until curl --noproxy '*' -sS http://127.0.0.1:4317/api/health | grep -q '"status":"ok"'; do sleep 2; done
```

> **必须先等后端就绪再启动前端**。前端 dev server 会反向代理 `/api` 到 `127.0.0.1:4317`；若后端未就绪，前端会长时间报 `ECONNREFUSED` 代理错误，背景任务可能因默认超时（约 10 分钟）被 kill。

再启动前端：在后端就绪后，于 `web/` 目录以后台任务启动前端 dev server，**同样设置无超时**：

```bash
pnpm dev --port 5173
# run_in_background=true, disable_timeout=true
```

### 4. 就绪检查与硬性门禁探测

只做以下检查；**最后一条是硬性门禁，非 200 不得声称服务可用**：

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
# 硬性门禁：浏览器同款 Host+Origin 经前端代理访问受守卫只读端点，必须 200
curl --noproxy '*' -sS -o /dev/null -w '%{http_code}\n' \
  -H "Host: 127.0.0.1:5173" -H "Origin: http://127.0.0.1:5173" \
  http://127.0.0.1:5173/api/image-create/sessions
```

门禁返回 403（`forbidden_host` / `forbidden_origin`）时服务**不可用**：按提示导出含前端 dev origin 的 `ARIA_API_TRUSTED_ORIGINS` 后重启后端再重测，不得向用户报告「已启动」。

通过后汇报：

- worktree 路径和分支。
- 最近提交/当前进度摘要。
- 前端地址 `http://127.0.0.1:5173`，后端地址 `http://127.0.0.1:4317`。
- 建议用户从 `/workbench` 开始手动观察。
- 明确说明：服务已启动并通过门禁探测，接下来等待用户操作。

## 为什么必须配受信 origin（故障史，勿删）

后端控制面以 Host/Origin 来源校验为入站边界（`src/web/api_auth.rs`），只放行「真实 bind 地址推导 + 显式配置」的 Host/Origin；vite dev 代理会把浏览器的 `Host: 127.0.0.1:5173` **原样**转给后端。未把前端 dev origin 配进 `ARIA_API_TRUSTED_ORIGINS` 的后果：

- 所有受守卫的 `/api/**` 与 `/ws/**` 全部 403（`forbidden_host` / `forbidden_origin`），前端整页「PROVIDER 状态不可读」。
- `/api/health` 在 Host 校验之前放行 ⇒ 健康检查全绿——「端口在听、健康正常、整站不可用」，报错离原因十万八千里。

两次真实故障：

- 2026-10-30（发布实例）：绕过 `scripts/aria-web-start.sh` 手工启动 ⇒ 脚本导出的 `ARIA_API_TRUSTED_ORIGINS` 没进进程环境 ⇒ owner 一访问就报 `request origin not allowed`（详见 `scripts/aria-web-start.sh` 头注释）。
- 2026-11-12（开发态）：按本 Skill 旧版裸命令起 4317+5173、未配受信 origin ⇒ vite 代理原样转发浏览器 Host ⇒ 后端全部 `/api/**` 403 `forbidden_host`，前端整页「状态读取失败: request host not allowed」。

唯一可靠自检 = 受守卫端点的门禁探测（第 4 步的 curl，或 `bash scripts/aria-dev-start.sh --check`）；`/api/health` 探不出这类故障，禁止用它代替。

## 用户反馈问题后

1. 先复述现象和用户正在操作的位置。
2. 根据用户贴出的日志、截图或错误，定位可能层级：前端、后端、WebSocket、Provider、目标仓库或环境。
3. 需要更多信息时一次只问一个关键问题。
4. 给出原因判断、定位路径和修复方案。
5. 需要改代码、配置、依赖或删除文件时，先等用户确认。
6. 用户确认后再按仓库规则执行 TDD、修改和验证。
