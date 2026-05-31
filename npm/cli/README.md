# @cadence-aria/cli

通过 npx 一键在本地启动 Aria 工作台：

```bash
npx @cadence-aria/cli
```

无参运行会启动本地 Web 工作台（绑定 `127.0.0.1`，自动选空闲端口）并打开浏览器。

## 用法

- `npx @cadence-aria/cli`：启动 web 工作台并开浏览器。
- `npx @cadence-aria/cli --no-open`：启动 web 但不开浏览器（远程/无头环境）。
- `npx @cadence-aria/cli web --port 3000`：指定端口（不自动开浏览器）。
- `npx @cadence-aria/cli task run ...` / `daemon` / `repl`：透传给底层 aria 二进制。

## 平台支持

预编译二进制覆盖：linux-x64、darwin-x64、darwin-arm64。其它平台请使用源码构建或查阅 GitHub Release。

## 可选外部依赖

真实 provider 执行需要 `codex` / `claude` CLI 在 PATH 中；缺失不影响工作台界面与演示。
