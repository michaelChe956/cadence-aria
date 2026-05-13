---
name: start-cadence-aria-service
description: Use when the user asks to start Cadence Aria development services, dev server, hot reload, HMR, 前后端热编译, 启动当前服务, 启动开发服务, or the local Aria web workbench from source instead of web/dist.
---

# Start Cadence Aria Service

## 目标

启动当前仓库的开发模式服务：Rust 后端热重编译重启，React/Vite 前端 HMR。开发服务必须从源码运行，不要用旧的 `web/dist` 静态产物作为前端来源。

## 规则

- 后端使用宿主机 Rust/Cargo，不要默认使用 Docker。
- 前端使用 `pnpm`，不要使用 `npm` 或 `yarn`。
- 不要手写文件轮询 watcher。后端使用 `cargo-watch`；前端使用 Vite dev server。
- 不要把 `cargo run -- web` 托管 `web/dist` 当作开发模式；只有用户明确要求生产静态托管时才这样做。
- 如果端口被本仓库旧服务占用，先停止旧进程；如果是不明进程，先向用户确认。

## 启动流程

1. 确认工具可用：

```bash
cargo watch --version
pnpm --version
```

如果缺少 `cargo-watch`，安装一次：

```bash
cargo install cargo-watch --locked
```

2. 在仓库根目录启动后端热重启服务：

```bash
cargo watch -w src -w Cargo.toml -w Cargo.lock -x "run --locked -- web --workspace . --host 127.0.0.1 --port 4317"
```

3. 在 `web/` 目录启动前端 HMR：

```bash
pnpm dev
```

4. 验证服务和代理链路：

```bash
curl --noproxy '*' -sS http://127.0.0.1:4317/api/health
curl --noproxy '*' -sS -I http://127.0.0.1:5173/
curl --noproxy '*' -sS http://127.0.0.1:5173/api/health
```

期望结果：

- 后端 `/api/health` 返回 `{"status":"ok"}`。
- 前端根路径返回 `200 OK`。
- 前端 `/api/health` 通过 Vite proxy 返回 `{"status":"ok"}`。

## 汇报

向用户汇报前端地址 `http://127.0.0.1:5173` 和后端地址 `http://127.0.0.1:4317`。如果 Vite 选择了其他端口，以实际输出为准。
