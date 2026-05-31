# Rust 构建/测试/检查命令规范（强制规则）

> **🔴 强制规则 - 无例外**
>
> 本规则规定本仓库 `cargo` 构建、测试、检查、格式化命令的统一用法。所有本地验证、手工验收、CI 门禁、AI Agent 自动化均须遵循。

## 文档信息

- 文档类型：项目个性化规则
- 规则状态：**已启用**
- 适用范围：cadence-aria 全部分支与 worktree
- 目标读者：项目开发者、维护者、AI Agent
- 工具链来源：仓库根目录 `rust-toolchain.toml`（当前 `1.95.0`），唯一声明来源
- 并行度托管：`.cargo/config.toml` 的 `[build] jobs = 8`

## 1. 标准命令（全量验证）

在仓库根目录或当前 worktree 根目录执行，宿主机直接运行（不使用 Docker）：

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

这四条即手工验收与 CI 门禁的标准命令，**本地与 CI 必须完全一致**。

## 2. 🔴 禁止携带 `-j 1`

- **禁止** `cargo test --locked -j 1`，**禁止**给任何 `cargo` 命令显式加 `-j 1`。
- 原因：本机 24 核，`-j 1` 强制单核串行编译，实测使 `cargo test` 从 **~1.5 分钟膨胀到 6 分钟**（其余 23 核全程闲置）。
- 并行度已由 `.cargo/config.toml` 的 `jobs = 8` 统一托管，命令行**无需也不应**再写 `-j`。命令行 `-j` 会覆盖 config，属退化操作。

> **历史背景（已被实测排除）**：`-j 1` 源于早期"规避冷构建并发下偶发集成测试依赖解析错误"的保守设置（见已退役的 `2026-04-29_README_Rust与Docker开发指南_v1.2.md`）。2026-05-30 实测：默认并行与 `-j 8` 下 553 个测试全绿、无依赖解析错误、内存峰值约 3GB（余量充足、无 OOM）。该担忧不再成立，故解除 `-j 1` 约束。

## 3. 定向快反馈命令（保留）

针对单元测试做局部快速验证时，使用以下定向命令（这些只跑单测、本就不应带 `-j 1`）：

```bash
# 定向运行 src/ 内单元测试，必须限制 --lib 目标，避免遍历全部集成测试二进制
cargo test --locked --lib <测试过滤名>

# approval_bridge 单测专用别名（等价 cargo test --locked --lib approval_bridge）
cargo test-approval-bridge
```

> **禁止**用 `cargo test --locked <测试过滤名>`（不带 `--lib`）作为快速反馈命令——它仍会编译并遍历所有集成测试二进制，失去"快"的意义。

## 4. 性能预期（实测基线，2026-05-30）

改一处 `src` 代码后的增量场景：

| 命令 | 预期耗时 |
|------|---------|
| `cargo check --locked` | 数秒 |
| `cargo clippy --all-targets --all-features --locked -- -D warnings` | 约 10 秒内 |
| `cargo test --locked`（含运行 553 个测试） | 约 1.5 分钟 |

冷构建（`cargo clean` / 切分支后首次）需重编重型 git 依赖，属低频，可能超 2 分钟；如需加速可本地按需启用 sccache（见 `cadence/readmes/2026-05-30_README_sccache可选缓存_v1.0.md`），不写入版本库。

## 5. 其他约束

- 工具链以 `rust-toolchain.toml` 为唯一来源；缺组件时按其修复宿主机环境，不改用 Docker 绕过。
- 新增或升级依赖时必须同步更新 `Cargo.lock`，并确保第 1 节四条命令全部通过。
