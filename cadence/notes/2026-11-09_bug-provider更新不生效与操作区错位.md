# bug：provider「更新」点了没效果 + 操作区按钮错位（2026-11-09）

> 报告形态：能力卡（Provider 管理面板）里 Claude Code / Codex / Pi 三家都显示
> 「版本落后 → 新版本」与【更新】按钮，点下去版本号不变；同一张卡里
> 「核验 <provider>」与【更新】两个按钮上下错位。

## 1. 根因（实测，非推测）

### 1.1 更新不生效：安装落点前缀 ≠ PATH 上生效的 CLI 所在前缀

- `src/product/provider_installation/runner.rs` 的 `InstallJobExecution::run`
  把安装前缀固定成 `npm_prefix()` = `NPM_PREFIX` 环境变量或
  `$HOME/.npm-global`（REQ-PCI-03 的「可写前缀」，原意是避开 sudo）。
- 本机 CLI 实际由 fnm 装在
  `/home/michaelche/.local/share/fnm/node-versions/v24.17.0/installation`
  （`bin/claude` → `../lib/node_modules/@anthropic-ai/claude-code/bin/claude`），
  该目录在 PATH 上（经 `fnm/aliases/default` 软链）。
- 结果：点击【更新】真的跑了
  `npm install -g --prefix ~/.npm-global @anthropic-ai/claude-code@latest`，
  新版本落进 `~/.npm-global/bin`（实测已是 2.1.319），而该目录**不在 PATH 上**
  ⇒ 用户执行的仍是旧版 2.1.307，界面版本号也不变。

### 1.2 按钮错位：操作区垂直居中 + 安装块是多行

- `ProviderCapabilityCard.tsx` 的操作区用 `items-center`：兄弟节点
  「核验 <provider>」按钮相对**整个安装块**居中。
- 安装块（`ProviderInstallActionButton`）是多行（按钮行 + 状态文案行），
  ⇒ 「核验」按钮被下移约 13px，与【更新】按钮不在同一顶边 ⇒ 视觉错位。
- 附带问题：安装块是 flex item（`min-width: auto`），长状态文案按
  max-content 把整块撑到 481px 宽，溢出卡片行宽。

## 2. 修法

### 2.1 后端：解析「生效前缀」再安装

新增 `src/product/provider_installation/effective_prefix.rs`：

1. **PATH 上当前生效的 CLI 所在前缀**（`<prefix>/bin/<cmd>`）——优先；
   更新它，用户手上的命令才真的变成新版本。
2. `npm prefix -g`（npm 自己的全局前缀）——CLI 未安装时的新装落点。
3. 兜底 `NPM_PREFIX` / `$HOME/.npm-global`（保持 REQ-PCI-03 的「可写前缀」）。

只取**可写**候选（不可写 ⇒ npm 必然 EACCES）。`path_hint` 语义同步收紧：
PATH 上生效的命令若不来自本次安装的前缀，提示「把 `<prefix>/bin` 放到 PATH
前面」，不再只在命令完全不在 PATH 时提示。

### 2.2 前端：操作区顶端对齐 + 安装块可收缩

- `ProviderCapabilityCard.tsx`：操作区 `items-center` → `items-start`
  （两个按钮共享顶边）；安装块包裹层加 `min-w-0 max-w-full`。
- `ProviderInstallActionButton.tsx`：根容器加 `min-w-0 max-w-full`；
  状态 / 失败 / 手动命令文案加 `break-words`（长文案折行而不是撑宽父行）。

## 3. 验证

- 后端：`cargo test --lib provider_installation`（43 项全绿，含新增回归用例：
  安装必须落到 PATH 上生效的 CLI 所在前缀）；`cargo test --lib` 全量
  6071 项全绿。
- 前端：`pnpm vitest --run`（272 文件 / 2779 项全绿，含 3 条新布局契约用例）。
- 界面：vite 预览 + Playwright 截图核对（修复前「核验」与「更新」顶边相差
  13px；修复后同一 y=160，状态文案落在按钮左缘）。
- 真实端到端（`aria web` 真服务 + 真 HTTP）：
  - `POST /api/providers/codex/install` → 作业 `completed`，
    `installed_version=0.167.0`、`path_hint=null`；
    `codex --version` 由 0.166.0 变 0.167.0；随后
    `GET /api/providers/install-targets` 显示 codex `action=none`（更新按钮消失）。
  - `POST /api/providers/pi/install` → `installed_version=1.5.0`；
    `pi --version` 由 1.3.0 变 1.5.0。
  - `claude` 按解析出的生效前缀实跑
    `npm install -g --prefix <fnm installation> @anthropic-ai/claude-code@latest`
    ⇒ `claude --version` 由 2.1.307 变 2.1.319。

### 部署口径（2026-11-09 第二次同款故障的沉淀）

- 本次修复最初「看起来没生效」，真因是**启动方式错了**：手工直起二进制 ⇒
  `ARIA_API_TRUSTED_ORIGINS` 没进进程环境 ⇒ owner 访问报
  `[request origin not allowed]`（端口在听但服务不可用）。
- 正确口径（已固化，见 `cadence/project-rules/local-service-deploy.md`）：
  1. `pnpm -C web build` → `CARGO_TARGET_DIR=/tmp/deploy-target cargo build --release --locked`
  2. `install -m755 scripts/aria-web-start.sh /tmp/aria-deploy/start.sh`（仓内源即真源）
  3. `bash /tmp/aria-deploy/start.sh`（TTY + 白名单 + 停旧实例；用 harness
     具名长驻服务 + pty 托管）
  4. `bash /tmp/aria-deploy/start.sh --check` → 6 项自检
- 发布态实测（4317，`/tmp/deploy-target/release/aria`）：6 项自检全过；
  `grep -a` 指纹「PATH 上生效的」「放到 PATH 前面」新件命中 1 / 回滚件命中 0；
  内嵌资源名 `assets/index-BeP4oStK.js` 与 `web/dist/assets/` 一致；
  owner 的访问路径（`arch-code:4317` / `localhost:4317` / 经 5173 代理）均 200。
- 界面实测（harness 内置浏览器，4317）：「核验 Codex」与【更新】同顶边
  （两者 y=367.5、height=36）；点【更新】⇒ codex 0.166.0 → 0.167.0，
  「版本落后」标签与【更新】按钮随之消失。

## 5. 闭环状态（2026-11-09）

- **owner 实机复测通过**：Claude Code / Codex / Pi / Kimi Code 四家的【更新】按钮逐个点过，
  版本号与「版本落后」标签按预期变化，无异常。
- 相关产出清单：
  - 契约层：Kimi 支持更新走独立 OpenSpec change `kimi-provider-update-support`
    （**已归档**，主规格已同步 REQ-PCI-07）；
  - 计划：`cadence/plans/2026-11-09_实施计划_kimi支持更新.md`（含完成记录）；
  - 说明：`cadence/notes/2026-11-09_kimi支持更新_版本来源与安装来源分离.md`；
  - 启动与开发期规则：`scripts/aria-web-start.sh` +
    `cadence/project-rules/local-service-deploy.md`（4317 启动唯一入口）+
    `cadence/project-rules/dev-test-port-policy.md`（开发期 hooks 错位的取证与处置）。
- **不提 PR**：该功能分支还有其它会话的在途内容，按 owner 决定不合并出 PR。
