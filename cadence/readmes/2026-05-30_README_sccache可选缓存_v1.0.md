# sccache 可选编译缓存使用说明

- **日期**：2026-05-30
- **版本**：v1.0
- **适用项目**：cadence-aria
- **目标受众**：项目开发者

---

## 1. 结论先行

sccache 在本项目中**为可选项，默认不启用**，且**不写入版本控制的项目配置**（`.cargo/config.toml`）。原因见下方实测收益分析——简言之，sccache 对本项目日常迭代**零收益**，仅在全量/跨 worktree 构建时省下依赖编译时间，价值有限。需要的开发者可在本地通过环境变量自行启用。

## 2. 为什么不写进 `.cargo/config.toml`

曾尝试在 `.cargo/config.toml` 加入：

```toml
[build]
rustc-wrapper = "sccache"
```

但该配置是**项目级、会被所有读到仓库的环境使用，包括 CI**。两个问题：

1. **会破坏 CI**：CI（`.github/workflows/ci.yml`）未安装 sccache。实测确认 cargo 在找不到 wrapper 时是**直接报错并非零退出**（不是降级警告），会导致 CI 的 clippy / check / test 三步全部失败。
2. **强制未安装者**：本地未装 sccache 的开发者，`cargo build` 同样会直接失败。

因此该配置已撤销，sccache 改为本地按需启用。

## 3. 实测收益边界（2026-05-30）

| 场景 | sccache 是否起作用 | 实测 |
|------|-------------------|------|
| **改一行代码后重编**（日常最高频） | ❌ 无作用 | 本 crate 重编时 sccache 调用增量 = **0**。本 crate 走 Rust 原生 incremental 编译，sccache 对开启 incremental 的编译单元一律**回退、不缓存**（non-cacheable）。耗时约 12s，sccache 帮不上 |
| **`cargo clean` 后全量重建** | ✅ 有作用 | 依赖（deps）命中约 53%，全量构建约 39s。这是 sccache 唯一发挥作用的场景，但属低频操作 |
| **跨 worktree 首次构建** | ✅ 有作用 | 新 worktree 首次构建可命中已缓存的依赖产物 |

> 关键限制：cargo 的 `dev`/`test` profile 默认 `incremental = true`，本 crate `cadence_aria`（编译大头）正走此路径，故对 sccache 始终 non-cacheable。详见技术方案 `cadence/designs/2026-05-30_技术方案_target目录瘦身_v1.0.md` 第 5.1.1 节。

## 4. 如何在本地启用（可选）

### 4.1 安装 sccache

```bash
cargo install sccache
# 或用包管理器，例如：
# brew install sccache        # macOS
# apt install sccache         # 部分 Linux 发行版
```

### 4.2 启用方式：环境变量（推荐，不污染项目配置）

在你的 shell 配置（`~/.zshrc` / `~/.bashrc`）或当前会话中设置：

```bash
export RUSTC_WRAPPER=sccache
```

此后在本项目执行 `cargo build` / `cargo test` 即会经过 sccache。验证：

```bash
sccache --show-stats | grep -E "Cache hits|Compile requests"
```

### 4.3 临时单次启用

不想长期开启时，只在单条命令前加前缀：

```bash
RUSTC_WRAPPER=sccache cargo build
```

### 4.4 关闭

取消环境变量即可：

```bash
unset RUSTC_WRAPPER
```

## 5. 注意事项

- sccache 缓存默认在 `~/.cache/sccache`，有大小上限（默认 10G）+ LRU 淘汰，不会无限增长。
- 若想让 sccache 也能缓存本 crate（而非仅依赖），需额外关闭 incremental（`export CARGO_INCREMENTAL=0`），代价是失去 Rust 原生增量快编。这是一个取舍，不建议默认开启，详见技术方案第 5.1.1 节的方案 A/B 对比。
- CI 已通过 `Swatinem/rust-cache@v2` 缓存 target 产物，无需在 CI 引入 sccache。
