## ADDED Requirements

### Requirement: 聚合索引生产触发

聚合索引构建具备三层生产触发：聚合初始化成功后自动首建、规划读取发现 stale 时按需同步、手动重建端点兜底。定时轮询兜底不在本变更范围。

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

### Requirement: 构建写入的 single-writer 与快照

#### Scenario: 首建与重建共用互斥
- **WHEN** 首次构建、重建或同步并发执行
- **THEN** 全部写入路径经统一 single-writer 互斥；跨命令前后采集成员快照，重建/同步期间漂移将新记录标 stale（不静默丢弃），active 记录引用命令前快照
