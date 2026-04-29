# Rust 与 Docker 开发指南

## 文档信息

- 文档类型：项目个性化规则
- 规则状态：已启用，强制执行
- 适用范围：Aria 一期 Rust/Cargo 实现分支
- 目标读者：项目开发者、维护者、AI Agent
- 当前推荐镜像：`rust:1-bookworm`
- 当前固定工具链：`rust-toolchain.toml` 中的 `1.95.0`

## 背景

当前分支使用 Rust/Cargo 实现 Aria P1。如果宿主机没有安装 Rust 工具链，必须使用 Docker 作为统一开发环境。

仓库根目录的 `rust-toolchain.toml` 是唯一工具链声明来源，固定 Rust 版本并声明 `rustfmt`、`clippy` 组件。不要在本规则文档中另行指定不同 Rust 版本。

命令默认从仓库根目录或当前 worktree 根目录执行。

## 格式检查

`cargo fmt --check` 必须通过。`rust-toolchain.toml` 已声明 `rustfmt` 组件，首次运行时 rustup 可能下载组件。

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -e RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup \
  -e RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo fmt --check
```

## 应用格式化

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -e RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup \
  -e RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo fmt
```

命令必须使用 `-u "$(id -u):$(id -g)"`，避免 `cargo fmt` 重写文件后留下 root-owned 文件。

## Clippy 检查

`cargo clippy` 必须覆盖全部 target 和 feature，并将 warning 作为失败处理：

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -e RUSTUP_DIST_SERVER=https://mirrors.tuna.tsinghua.edu.cn/rustup \
  -e RUSTUP_UPDATE_ROOT=https://mirrors.tuna.tsinghua.edu.cn/rustup/rustup \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo clippy --all-targets --all-features --locked -- -D warnings
```

## 编译检查

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo check --locked
```

## 运行测试

```bash
docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -e CARGO_NET_RETRY=10 \
  -e CARGO_HTTP_TIMEOUT=120 \
  -v "$PWD":/work -w /work \
  rust:1-bookworm cargo test --locked -j 1
```

测试默认使用 `-j 1`，用于规避冷构建并发状态下偶发的集成测试依赖解析错误。若后续确认默认并发冷构建稳定，可再调整本规则。

## 运行 CLI

```bash
mkdir -p /tmp/aria-workspace

docker run --rm -u "$(id -u):$(id -g)" \
  -e CARGO_HOME=/work/.cargo-home \
  -v "$PWD":/work \
  -v /tmp:/host-tmp \
  -w /work \
  rust:1-bookworm cargo run --locked -- daemon status --workspace /host-tmp/aria-workspace
```

干净 workspace 的预期输出：

```text
daemon_not_found
```

## 缓存与权限

- `CARGO_HOME=/work/.cargo-home` 会把 Cargo 缓存放在 worktree 内。
- `.cargo-home/` 已在 `.gitignore` 中忽略。
- 以 `-u "$(id -u):$(id -g)"` 运行 `cargo fmt`、`cargo clippy`、`cargo check`、`cargo test` 和 `cargo run`，可避免 `target/` 与缓存目录变成 root-owned。
- 如果必须用 root 运行会改写文件的命令，命令结束前应显式 `chown` 回当前用户。

## CI 门禁

GitHub Actions 必须至少执行以下命令：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked -j 1
```

新增或升级依赖时必须同步更新 `Cargo.lock`，并确保上述门禁通过。
