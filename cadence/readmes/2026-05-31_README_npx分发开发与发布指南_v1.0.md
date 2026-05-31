# README：Aria npx 分发开发与发布指南

- 文档类型：开发文档（README）
- 版本：v1.0
- 创建日期：2026-05-31
- 对应设计：`cadence/designs/2026-05-31_技术方案_Aria_npx本地化分发_v1.3.md`

## 1. 构建顺序契约（必读）

`web/dist` 被 `web/.gitignore` 忽略、不在版本控制中。`aria` 二进制通过 `rust-embed` 在**编译期**嵌入 `web/dist`，且 `build.rs` 会在其缺失时**硬失败**。因此：

> **任何 `cargo build` / `cargo test` / `cargo check` 前，必须先 `pnpm -C web build`。**

干净 checkout、切分支、CI 首次构建均适用。

## 2. 本地开发逃生口 ARIA_WEB_DIST

运行时默认从嵌入资源服务前端。开发/调试时若想改前端而不重编 Rust：

```bash
ARIA_WEB_DIST=$(pwd)/web/dist cargo run -- web --workspace .
```

设置后改从该磁盘目录读取（未设置或目录不存在则走嵌入）。playwright e2e（`web/e2e/start-api.mjs`）已自动注入该变量。

## 3. Rust 验证命令

遵循 `cadence/project-rules/build-test-commands.md`：

```bash
pnpm -C web build   # 前置，必须
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

🔴 禁止给 cargo 加 `-j 1`。

## 4. npm 包结构与本地冒烟

- 主包 `npm/cli`（仅 JS launcher，声明 `bin: aria` 与 optionalDependencies）。
- 平台子包 `npm/cli-<platform>`（含预编译二进制 `bin/aria`，**不声明 bin**）。
- 组包：`node scripts/pack-npm.mjs --version <v> --main-only` + `--platform <key> --binary <path>`。
- 本地冒烟：`node scripts/smoke-npx.mjs`（需先 `pnpm -C web build && cargo build --release`）。

launcher 单测：`node --test "npm/cli/test/*.test.mjs"`（注意用 glob；node v25 下 `node --test <目录>` 会报错）。

### launcher 行为要点

- 无参 或 恰好只传 `web` → 默认 web 模式：自选空闲端口、`web --port <p> --host 127.0.0.1`、就绪后开浏览器。
- `web` 后跟任何 flag（`--check`/`--port`/`--host`/`--workspace` 等）或其它子命令 → 原样透传、不劫持端口、不开浏览器。
- `--no-open` 由 launcher 消费并关闭开浏览器。

## 5. 发布（Tag 触发）

推送 `v<x.y.z>` tag 触发 `.github/workflows/release.yml`：

1. build：ubuntu/macos-13/macos-14 各自原生编译，tar 打包上传 artifact（tar 保执行位）。
2. release：下载解包 → chmod +x + test -x → 组包 → 体积 gate → npm publish（子包先、主包后）→ GitHub Release（附 tar.gz + SHA256SUMS）。

### 发布前置条件（维护者提供）

- npm `@cadence-aria` scope 已创建。
- 仓库 secret `NPM_TOKEN`（具 `@cadence-aria` 发布权限）。
- GitHub Release 用默认 `GITHUB_TOKEN` + `permissions: contents: write`，无需额外 PAT。
- 版本来源：tag `v0.1.0` → 包版本 `0.1.0`。

### publish 失败处理（非原子）

多包 publish 无事务。若中途失败（如 3 个子包发了 2 个），**不要覆盖同版本**（npm 不允许）。改用新 patch 版本（如 `v0.1.1`）整体重发。

## 6. 平台支持与风险

- 预编译矩阵：linux-x64、darwin-x64、darwin-arm64。
- `macos-13`（Intel runner）有退役风险；下线后改用 arm64 runner 交叉编译 x86_64-apple-darwin（需验证 git 重依赖交叉编译）。
- linux-arm64、Windows 原生为后续增量；非支持平台 launcher 给出清晰错误。

## 7. 实测数据与待确认（开放事项）

- **release 二进制实测约 6.3MB**（linux-x64，strip+lto+opt-level=s），远低于体积 gate 90MB 初值。
- 首个版本号（建议 `0.1.0`）。
- 启动检测提示中 codex/claude 安装指引文案/链接（当前为占位文案）。
- release profile `opt-level`（`s` vs `z`）、`panic = "abort"` 最终取值。
- 体积 gate 阈值（workflow 中初值 90MB，按多平台实测校准）。
