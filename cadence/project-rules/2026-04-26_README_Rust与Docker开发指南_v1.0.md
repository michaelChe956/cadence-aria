# Rust 与 Docker 开发指南

## 文档信息

- 文档类型：项目个性化规则
- 规则状态：已启用，强制执行
- 适用范围：Aria 一期 Rust/Cargo 实现分支
- 目标读者：项目开发者、维护者、AI Agent
- 当前推荐镜像：`rust:1-bookworm`

## 背景

当前分支使用 Rust/Cargo 实现 Aria P1。如果宿主机没有安装 Rust 工具链，必须使用 Docker 作为统一开发环境。

命令默认从仓库根目录或当前 worktree 根目录执行。

## 格式检查

`rust:1-bookworm` 可能需要下载 `rustfmt` 组件。为避免默认源下载过慢，建议使用清华 TUNA rustup 镜像：

```bash
docker run --rm \
  -e RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup \
  -e RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup \
  -v "$PWD":/work -w /work \
  rust:1-bookworm \
  sh -c 'set -e; rustup component add rustfmt; cargo fmt --check'
```

## 应用格式化

```bash
docker run --rm \
  -e RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup \
  -e RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup \
  -v "$PWD":/work -w /work \
  rust:1-bookworm \
  sh -c 'set -e; rustup component add rustfmt; cargo fmt; chown -R "$(stat -c %u:%g /work)" /work/src /work/tests /work/Cargo.toml /work/Cargo.lock /work/.gitignore /work/README.md /work/cadence/project-rules'
```

`chown` 用于避免容器内 `cargo fmt` 重写文件后留下 root-owned 文件。

## 编译检查

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo check
```

## 运行测试

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo test
```

## 运行 CLI

```bash
mkdir -p /tmp/aria-workspace

docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -v "$PWD":/work \
  -v /tmp:/host-tmp \
  -w /work \
  rust:1-bookworm cargo run -- daemon status --workspace /host-tmp/aria-workspace
```

干净 workspace 的预期输出：

```text
daemon_not_found
```

## 缓存与权限

- `CARGO_HOME=/work/.cargo-home` 会把 Cargo 缓存放在 worktree 内。
- `.cargo-home/` 已在 `.gitignore` 中忽略。
- 以 `-u "$(id -u):$(id -g)"` 运行 `cargo check` 和 `cargo test`，可避免 `target/` 与缓存目录变成 root-owned。
- 如果必须用 root 运行会改写文件的命令，命令结束前应显式 `chown` 回当前用户。
