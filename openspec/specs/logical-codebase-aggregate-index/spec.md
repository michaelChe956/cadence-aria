# logical-codebase-aggregate-index Specification

## Purpose

在非 git 公共父目录建立一份统一 CodeGraph 索引覆盖全部成员仓，管理索引范围、快照、freshness 与生命周期。

## Requirements

### Requirement: 聚合索引建立（REQ-IND-01）
系统 SHALL 在非 git 聚合根建立一份统一 CodeGraph 索引（exact version 钉定 v1.5.0）；CodeGraph 按目录递归扫描而非按 git 边界识别成员（spike 2 实测：非 git 目录 `not-a-repo` 同被纳入），主路径不依赖 `includeIgnored`（D3 非 git 父目录）。索引构建成功以「`codegraph files` 成员覆盖 + 代表性跨子仓查询命中 + 非成员/`.worktrees`/`.aria`/构建产物负查询不命中」为验收，而非仅 DB 文件存在。

#### Scenario: 非 git 聚合根建索引
- **WHEN** 完成成员登记并在非 git 聚合根初始化索引
- **THEN** 聚合根产生一份 `.codegraph/codegraph.db`，覆盖全部 manifest 成员；索引构建成功以「成员覆盖 + 代表性查询命中 + 非成员/excluded 负查询不命中」为验收，而非仅 DB 文件存在

### Requirement: 索引范围与排除（REQ-IND-02）
系统 SHALL 由 Cadence 在聚合根生成并维护 `codegraph.json` 的 `exclude`（denylist，根相对 gitignore 模式；spike 2 实测：CLI 无 allowlist 参数，`exclude` 是唯一范围控制机制），至少覆盖：非成员目录、`**/.worktrees/`、`**/.aria/`、构建产物目录与凭据目录；CodeGraph 内建排除 `.git`/`build`/`node_modules`/`dist` 但 `.worktrees`/`.aria` 非内建（spike 2 实测：删子仓 `.gitignore` 后 `.worktrees`/`.aria` 被误索引）。成员增删后 SHALL 原子更新该文件并 sync/全量 index；index 后用 `codegraph files` 与负查询验证边界，发现非成员路径即失败而非静默接受。同一源码主 checkout 与 worktree 不重复索引。

#### Scenario: 排除执行 worktree
- **WHEN** 成员仓内存在 `.worktrees/**` 或构建产物
- **THEN** 索引 SHALL 排除这些路径，查询结果不命中未提交的 coding 分支内容

### Requirement: 索引快照与 freshness（REQ-IND-03）
系统 SHALL 在每次 init/sync 前后由 Cadence 采集各成员仓 `git rev-parse HEAD` 与 dirty 状态填充 `AggregateIndexMemberSnapshot`（CodeGraph 不提供 per-member revision API）；按需 sync（AI 用前检查）+ 定时兜底刷新；stale/degraded 状态由 Cadence 对比快照 revision 推断并显式呈现。

#### Scenario: 成员主 checkout 被外部更新
- **WHEN** 成员主 checkout 在索引后发生外部提交/切换
- **THEN** 系统 SHALL 标记索引为 stale（对比快照 revision），规划时明确提示「索引可能过时」；按需 sync 或定时兜底触发刷新

#### Scenario: 快照不可变证据
- **WHEN** 索引建立或刷新
- **THEN** 系统 SHALL 记录 AggregateIndexMemberSnapshot（member/checkout/revision/included/indexed_at）作为不可变证据，供规划与审计引用

### Requirement: 索引生命周期与故障恢复（REQ-IND-04）
系统 SHALL 提供聚合索引 single-writer 锁、last-known-good 保留、重建失败恢复与 stale/degraded/superseded 状态转换；规划使用冻结快照，不承诺实时。

#### Scenario: 索引重建失败
- **WHEN** 聚合索引重建或刷新失败
- **THEN** 系统 SHALL 保留 last-known-good 索引并标记 degraded，规划时返回可审计告警，不阻塞只读规划

#### Scenario: 并发写保护
- **WHEN** 多个 sync/rebuild 请求并发
- **THEN** 系统 SHALL 通过 single-writer 锁串行化，避免索引损坏或半写状态
