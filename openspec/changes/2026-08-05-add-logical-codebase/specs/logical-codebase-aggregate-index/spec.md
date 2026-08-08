# logical-codebase-aggregate-index Specification

## Purpose

在非 git 公共父目录建立一份统一 CodeGraph 索引覆盖全部成员仓，管理索引范围、快照、freshness 与生命周期。

## ADDED Requirements

### Requirement: 聚合索引建立（REQ-IND-01）
系统 SHALL 在非 git 聚合根建立一份统一 CodeGraph 索引覆盖全部 manifest 成员仓；主路径不依赖 `includeIgnored`（D3 非 git 父目录自动发现子仓）；CodeGraph 版本钉定且避开已知 `includeIgnored` 回归版本（如 v1.4.1，#1295）。

#### Scenario: 非 git 聚合根建索引
- **WHEN** 完成成员登记并在非 git 聚合根初始化索引
- **THEN** 聚合根产生一份 `.codegraph/codegraph.db`，覆盖全部 manifest 成员；索引构建成功以「成员 allowlist 覆盖 + 代表性查询命中」为验收，而非仅 DB 文件存在

### Requirement: 索引范围与排除（REQ-IND-02）
系统 SHALL 基于 manifest 成员 allowlist 构造索引范围，排除执行 worktree（`.worktrees/**`）、`.git/**`、`.aria/**`、构建产物与凭据目录；同一源码主 checkout 与 worktree 不重复索引。

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
