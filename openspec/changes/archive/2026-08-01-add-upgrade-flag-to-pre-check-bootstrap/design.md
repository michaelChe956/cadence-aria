# Design: add-upgrade-flag-to-pre-check-bootstrap

## Context

代码库初始化的第二步 `pre_check` 由 `RepositoryInitializationStepKind::command()` 提供提示词，经 Claude Code Provider 在目标 Git 根作为工作目录执行。`command()` 返回 `Option<&'static str>` 静态字符串，所有初始化共用同一条命令，不区分场景。当前 `PreCheck` 分支返回 `/pre-check --no-interrupt 用大陆镜像`。`from_command_index` 等按步骤枚举的逻辑只依赖步骤顺序，不依赖命令字符串内容。

## Decisions

### 决策 1：仅 `pre_check` 追加 `--upgrade`，无条件固定

- **选择**：`PreCheck` 命令改为 `/pre-check --no-interrupt --upgrade 用大陆镜像`，所有代码库初始化均带 `--upgrade`；其余三条命令不变。
- **理由**：本仓库不解释 `--upgrade` 语义，只保证发送的字符串与 Cadence-skills 最新命令契约一致；`--upgrade` 的行为由 Cadence-skills 的 `/pre-check` 命令定义。与上一个 change（`use-mainland-mirror-for-bootstrap`）追加"用大陆镜像"的处理方式一致。
- **放弃的备选**：按"新建/升级"场景动态切换命令。该方案要求本仓库先引入初始化模式概念（当前不存在），`command()` 须从静态字符串改为按模式生成，架构影响大且当前无需求（YAGNI）。

### 决策 2：`command()` 仍是静态字符串，不做配置化/场景化

- **理由**：YAGNI；当前无按场景切换命令的需求，与现状（静态常量）保持一致，减少测试面；`from_command_index` 等按步骤枚举的逻辑不受影响。

## 影响面与测试策略

- `types.rs`：`PreCheck` 分支命令字符串更新。
- 引用该字符串的 Rust 单元/集成测试与前端测试断言同步更新。
- 验证：仓库标准四命令（fmt/clippy/check/test）+ 前端 `pnpm tsc -b`、`pnpm test`；定向单测带 `--lib`，禁止 `-j 1`。
