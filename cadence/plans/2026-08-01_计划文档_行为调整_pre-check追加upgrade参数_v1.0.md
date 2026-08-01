# pre-check 追加 `--upgrade` 参数实施 Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

- **OpenSpec Change**：`openspec/changes/2026-08-01-add-upgrade-flag-to-pre-check-bootstrap/`（契约已获批）
- **映射工作包**：任务 1 -> 工作包 1（requirement：`non-interrupt-repository-bootstrap` / 无中断 Claude Code 初始化命令）；任务 2 -> 工作包 2（全量验证）

**Goal:** 将代码库初始化第二步 `pre_check` 的 Claude Code 提示词从 `/pre-check --no-interrupt 用大陆镜像` 改为 `/pre-check --no-interrupt --upgrade 用大陆镜像`，其余三条命令不变。

**Architecture:** `RepositoryInitializationStepKind::command()` 的 `PreCheck` 分支返回的静态字符串追加 `--upgrade` 参数（位于 `--no-interrupt` 之后、`用大陆镜像` 之前）；`command()` 仍为 `Option<&'static str>`，无架构变化。引用该字符串的 Rust 单元/集成测试与前端测试断言同步更新。

**Tech Stack:** Rust（edition 2024，宿主机 Cargo）、Tokio；前端 TypeScript + Vitest（仅测试断言同步）。

## Global Constraints

- `pre_check` 提示词逐字为：`/pre-check --no-interrupt --upgrade 用大陆镜像`
- `/rule-config --no-interrupt`、`/mcp-configuration --no-interrupt`、`/project-rules-examples --no-interrupt` 三条命令逐字不变，SHALL NOT 附带 `--upgrade`
- 🔴 禁止给任何 `cargo` 命令加 `-j 1`；并行度由 `.cargo/config.toml` 托管
- 定向单测必须带 `--lib`：`cargo test --locked --lib <过滤名>`；禁止不带 `--lib` 的过滤命令
- 标准验证四命令：`cargo fmt --check`、`cargo clippy --all-targets --all-features --locked -- -D warnings`、`cargo check --locked`、`cargo test --locked`
- 所有命令在 worktree 根目录执行（worktree 由 `superpowers:using-git-worktrees` 在执行时创建）
- 测试模块受 `#[cfg(all(test, unix))]` 约束，本机为 Linux，直接可跑

---

### Task 1: pre_check 命令追加 `--upgrade` 参数（TDD）

**Files:**
- Modify: `src/product/repository_store/types.rs:104`（`PreCheck` 命令字符串）
- Test: `src/product/repository_store/initializer/tests.rs:296,327`
- Test: `src/product/repository_store/operation/tests.rs:98`
- Test: `tests/it_web/web_repository_initialization/operation_http.rs:62,72,160`
- Test: `web/src/api/client.test.ts:50,247`
- Test: `web/src/api/types.test.ts:551,600`

**Interfaces:**
- Consumes: 无前置任务依赖
- Produces: `RepositoryInitializationStepKind::command()` 的 `PreCheck` 分支返回 `Some("/pre-check --no-interrupt --upgrade 用大陆镜像")`；前端仅消费该字符串，无类型变化

- [ ] **Step 1: 更新全部测试断言（红灯）**

将下列文件中出现的 `/pre-check --no-interrupt 用大陆镜像` 逐字替换为 `/pre-check --no-interrupt --upgrade 用大陆镜像`（Rust 文件为字符串字面量，TS 文件同为字符串字面量；注意不要把 `/rule-config` 等其它三条命令误改）：

- `src/product/repository_store/initializer/tests.rs`（296、327 行附近，共 2 处）
- `src/product/repository_store/operation/tests.rs`（98 行附近，1 处）
- `tests/it_web/web_repository_initialization/operation_http.rs`（62、72、160 行附近，共 3 处）
- `web/src/api/client.test.ts`（50、247 行附近，共 2 处）
- `web/src/api/types.test.ts`（551、600 行附近，共 2 处）

替换完成后用以下命令确认：旧字符串应仅残留在 `src/product/repository_store/types.rs:104`（实现，待 Step 3 改），新字符串应出现 10 处（测试断言）：

```bash
rg -n '/pre-check --no-interrupt 用大陆镜像' --glob '!node_modules' --glob '!.git' -g '*.rs' -g '*.ts' -g '*.tsx'
rg -c '/pre-check --no-interrupt --upgrade 用大陆镜像' --glob '!node_modules' --glob '!.git' -g '*.rs' -g '*.ts' -g '*.tsx'
```

- [ ] **Step 2: 运行后端测试确认失败**

Run: `cargo test --locked --lib repository_store`
Expected: FAIL--`initializer`/`operation` 测试断言的命令字符串与实现（仍未改）不符。

- [ ] **Step 3: 最小实现**

`src/product/repository_store/types.rs:104` 改为：

```rust
Self::PreCheck => Some("/pre-check --no-interrupt --upgrade 用大陆镜像"),
```

- [ ] **Step 4: 运行后端测试确认通过**

Run: `cargo test --locked --lib repository_store`
Run: `cargo test --locked --test it_web web_repository_initialization`
Expected: 两条均 PASS。

- [ ] **Step 5: 运行前端测试确认通过**

Run: `cd web && pnpm test client.test types.test`
Expected: PASS（Vitest 按文件名子串过滤到两个断言所在文件，`pnpm test` 脚本本身已带 `--run`）。

- [ ] **Step 6: Commit**

```bash
git add src/product/repository_store/types.rs src/product/repository_store/initializer/tests.rs src/product/repository_store/operation/tests.rs tests/it_web/web_repository_initialization/operation_http.rs web/src/api/client.test.ts web/src/api/types.test.ts
git commit -m "feat: add --upgrade flag to pre-check bootstrap command"
```

---

### Task 2: 全量验证与 OpenSpec tasks 勾选

**Files:**
- Modify: `openspec/changes/2026-08-01-add-upgrade-flag-to-pre-check-bootstrap/tasks.md`（勾选全部工作包）

**Interfaces:**
- Consumes: Task 1 的提交
- Produces: 全量验证证据；勾选后的 tasks.md

- [ ] **Step 1: 后端标准四命令**

Run（worktree 根目录，依次执行，全部必须通过）:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo check --locked
cargo test --locked
```

Expected: 全部通过；`cargo test --locked` 全量绿。

- [ ] **Step 2: 前端验证**

Run:

```bash
cd web && pnpm tsc -b
cd web && pnpm test
```

Expected: 类型检查通过、Vitest 全量绿。

- [ ] **Step 3: 勾选 OpenSpec tasks 并提交**

将 `openspec/changes/2026-08-01-add-upgrade-flag-to-pre-check-bootstrap/tasks.md` 中全部 `- [ ]` 勾选为 `- [x]`，然后：

```bash
git add openspec/changes/2026-08-01-add-upgrade-flag-to-pre-check-bootstrap/
git commit -m "chore: tick tasks for add-upgrade-flag-to-pre-check-bootstrap"
```

- [ ] **Step 4: 推送分支**

```bash
git push origin <功能分支名>
```
