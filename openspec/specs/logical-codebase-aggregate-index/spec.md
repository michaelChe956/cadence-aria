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

### Requirement: 聚合索引生产触发

逻辑代码库聚合初始化 SHALL 在缺少 active aggregate index 时按冷启动依赖顺序推进：先完成可用的 identity、manifest/checkout、rules/policy 与成员 index 材料，再触发首次 aggregate index 构建。首建准入 MUST NOT 要求一个尚不存在的 active index，也 MUST NOT 通过 GET/规划只读请求隐式启动 provider 或索引。每个步骤的状态、checkpoint、失败原因和合法准备/继续/重试操作 SHALL 可查询；步骤成功后自动推进到下一步骤，最终将可验证的聚合索引置为 active/PlanningReady。

#### Scenario: 初始化后自动首建

- **WHEN** 聚合初始化成功完成
- **THEN** 自动触发首次索引构建；首建失败不回滚初始化，索引状态为 missing 附失败原因，可手动重建

#### Scenario: 首建期间漂移

- **WHEN** 首次构建期间成员 HEAD 或 dirty 状态变化
- **THEN** 该索引记录标记 Failed（而非 stale），避免出现无可读 active 索引

#### Scenario: 规划读时 stale 同步

- **WHEN** 规划上下文解析读取到 stale 索引
- **THEN** 执行按需同步后继续；degraded 索引不自动重建，仅向规划上下文注入可审计告警；degraded 记录不会被新鲜度评估误报为 active

#### Scenario: 手动重建

- **WHEN** 调用 POST /api/projects/{pid}/logical-codebase/aggregate-indexes/rebuild
- **THEN** 同步执行重建并返回最终记录（前端展示 loading）；同 project 已有重建进行中时返回 409 aggregate_index_rebuild_in_progress

#### Scenario: 构建状态可见

- **WHEN** 构建进行中查询 GET /api/projects/{pid}/logical-codebase/aggregate-indexes/active
- **THEN** 返回 state=rebuilding（Building 记录先于索引命令持久化）；状态映射覆盖 active/stale/degraded/rebuilding/missing（Failed 有 last-known-good 时呈现 degraded，否则 missing）

#### Scenario: 缺少 active index 的 LC 首建

- **WHEN** 新逻辑代码库已完成身份与成员材料但没有 active aggregate index
- **THEN** 系统 SHALL 接受首建并按缺失步骤生成成员 index 与 aggregate index，不因 active index 缺失拒绝成员准入，也不要求用户手工 seed

#### Scenario: 首建步骤中断后继续

- **WHEN** 首次 aggregate index 构建或其前置步骤失败
- **THEN** 系统 SHALL 保留已完成 checkpoint、失败状态及原因；用户从产品操作面点击允许的继续/重试后 SHALL 只执行未完成或安全可重做步骤，完成后自动推进，不重复已完成 provider turn

#### Scenario: GET 仅投影 index 状态

- **WHEN** 客户端查询 active aggregate index、成员 index 或 LC 初始化状态
- **THEN** 系统 SHALL 返回 missing/rebuilding/failed/degraded/active 等 durable 投影和允许动作，MUST NOT 因查询触发索引命令、provider、checkout 或状态突变

### Requirement: 构建写入的 single-writer 与快照

全部索引写入路径必须经统一 single-writer 互斥，并保留命令前后成员快照证据。

#### Scenario: 首建与重建共用互斥
- **WHEN** 首次构建、重建或同步并发执行
- **THEN** 全部写入路径经统一 single-writer 互斥；跨命令前后采集成员快照，重建/同步期间漂移将新记录标 stale（不静默丢弃），active 记录引用命令前快照

### Requirement: 构建状态可见与失败可恢复

系统 SHALL 将聚合 index 的缺失、构建中、失败、降级和 active 状态与所属 LC authority、成员/checkout 快照及操作身份关联展示。首次构建失败时 SHALL 保留可诊断失败事实；存在 last-known-good 时继续只读投影并标记 degraded，无 last-known-good 时标记 missing/failed；任何自动继续或用户重试均不得伪造 active。重试成功后才可基于验证过的成员覆盖、代表性查询与排除边界将记录转换为 active。

#### Scenario: 首建失败不伪造 active

- **WHEN** 首次 index 构建因成员材料缺失、范围错误或成员 HEAD 漂移失败
- **THEN** 系统 SHALL 返回失败原因和修复/重试操作，active index 保持不存在；不得以仅有数据库文件或 capability 记录宣称成功

#### Scenario: 保留可读的 last-known-good

- **WHEN** 已有 active index 的后续刷新失败
- **THEN** 系统 SHALL 保留 last-known-good 供只读规划，状态标记为 degraded 并向规划上下文注入可审计告警；不得把 degraded 当作新的 active 或静默丢弃失败事实
