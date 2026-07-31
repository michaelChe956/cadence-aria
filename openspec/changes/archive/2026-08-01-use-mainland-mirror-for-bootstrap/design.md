# Design: use-mainland-mirror-for-bootstrap

## Context

`CadenceSkillsManager` 负责 Cadence-skills 源的准备：源目录不存在时 `clone_source`；存在且含 `.git` 时 `update_source`（`fetch --all` → 检查上游 → `pull --ff-only`）；否则离线回退。源地址是 `manager.rs` 中的常量 `REPOSITORY_URL`，当前指向 GitHub。代码库初始化的第二步 `pre_check` 由 `RepositoryInitializationStepKind::command()` 提供提示词，经 Claude Code Provider 在目标仓库执行。

## Decisions

### 决策 1：存量克隆迁移用 `remote set-url`，不删除重克隆

- **选择**：`update_source` 执行任何网络操作前，先 `git remote get-url origin`；与目标 Gitee 地址不一致时执行 `git remote set-url origin <目标地址>`，随后走原有 fetch/pull 流程。
- **理由**：Gitee 是同一仓库的镜像，commit 历史一致，`pull --ff-only` 可正常工作；set-url 快、保留本地克隆、无删除风险。
- **放弃的备选**：检测到不一致即删除目录重新克隆。该方案破坏性强、迁移成本高，仅在两端历史分叉时才有优势；分叉场景下本设计沿用既有 `update_failed` 错误路径，用户删除本地克隆目录后重试即可自愈。
- **失败处理**：`get-url`/`set-url` 失败（含超时、取消）按 `update_failed` 返回，与既有 fetch/pull 失败语义一致；不新增错误码类别之外的对外契约。

### 决策 2：仅 `pre_check` 追加镜像参数

- **选择**：`PreCheck` 命令改为 `/pre-check --no-interrupt 用大陆镜像`，其余三条命令不变。
- **理由**：用户确认镜像参数只作用于 pre-check 阶段；`from_command_index` 等按步骤枚举的逻辑不受影响。

### 决策 3：源地址保持代码内常量，不做配置化

- **理由**：YAGNI；当前无按环境切换源的需求，与现状（常量）保持一致，减少测试面。

## 影响面与测试策略

- `manager.rs`：常量更新；`update_source` 前置 origin 迁移逻辑。单元测试以 RecordingRunner 断言：origin 不匹配时请求序列为 `get-url` → `set-url` → `fetch` → `rev-parse` → `pull`；origin 匹配时不发出 `set-url`。
- `types.rs`：命令字符串更新；引用该字符串的 Rust 单元/集成测试与前端测试断言同步更新。
- 验证：仓库标准四命令（fmt/clippy/check/test）+ 前端 `pnpm tsc -b`、`pnpm test`。
