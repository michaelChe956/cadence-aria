# Proposal: use-mainland-mirror-for-bootstrap

## Why

代码库初始化链路的两处外部资源访问仍指向海外源，大陆网络环境下不稳定：

1. Cadence-skills 源仓库硬编码为 GitHub 地址（`https://github.com/michaelChe956/Cadence-skills`），首次克隆与后续 `fetch --all` 都访问 GitHub；仓库已镜像到 Gitee（`https://gitee.com/michaelChe-World/Cadence-skills.git`）。
2. `pre_check` 步骤发给 Claude Code 的提示词为 `/pre-check --no-interrupt`，未声明使用大陆镜像，命令执行期间可能选择海外资源。

## What Changes

- Cadence-skills 源地址切换为 Gitee：`https://gitee.com/michaelChe-World/Cadence-skills.git`。新克隆直接使用该地址。
- 存量克隆迁移：已存在的 Cadence-skills 克隆在更新前检测 `origin` 远程地址，与目标地址不一致时先执行 `git remote set-url origin <Gitee 地址>`，再走原有 fetch/pull 更新流程；不删除目录、不重新克隆。
- `pre_check` 步骤的 Claude Code 提示词改为 `/pre-check --no-interrupt 用大陆镜像`；其余三条命令（`/rule-config`、`/mcp-configuration`、`/project-rules-examples`）保持不变。

## Non-goals

- 不改变六步固定步骤的数量、顺序与状态机语义。
- 不处理 Gitee 与 GitHub 历史分叉导致的 `pull --ff-only` 失败（沿用既有 `update_failed` 错误路径，由用户删除本地克隆后重试自愈）。
- 不为其他三条初始化命令追加镜像参数。
- 不引入配置项；源地址仍为代码内常量。

## Capabilities

### Modified Capabilities

- `non-interrupt-repository-bootstrap`: `pre_check` 命令提示词追加 `用大陆镜像` 参数。
- `repository-initialization-progress`: `cadence_skills` 准备步骤的源仓库地址切换为 Gitee，并新增存量克隆 origin 迁移行为。

### New Capabilities

- 无

## Impact

- 后端：`src/product/cadence_skills/manager.rs`（源地址常量与 `update_source` 前的 origin 检测/set-url）；`src/product/repository_store/types.rs`（`PreCheck` 命令字符串）。
- 测试：`manager.rs` 单元测试、`repository_store` 单元测试、`tests/it_web/web_repository_initialization/`、前端 `client.test.ts`/`types.test.ts` 中的命令字符串断言。
- 前端：无功能变化；仅测试断言中的命令字符串同步。
- 不新增第三方依赖。
