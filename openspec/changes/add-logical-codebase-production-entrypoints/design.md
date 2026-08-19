# Design: add-logical-codebase-production-entrypoints

> 完整设计见 `cadence/designs/2026-08-18_方案设计_多仓库生产入口补齐_v1.2.md`（经 v1.0 terra 审核 7B/14M/5m 与 v1.1 reviewer 审阅 2B/6M/9m 两轮迭代后用户批准）。本文件为契约内摘要。

## 核心决策（D1-D8）

| # | 决策 |
|---|---|
| D1 | 多仓库模式=project 创建时 opt-in，不可切换（单向升级留后续 change） |
| D2 | 登记 UI=逻辑代码库页向导弹窗 |
| D3 | 初始化=begin → 共享 registry lease（含 CancellationToken）→ tokio::spawn(execute)，step 边界检查取消 |
| D4 | 多仓 issue 自动写 all_members selection（补偿事务原子性） |
| D5 | 索引三层触发：初始化尾步首建 + resolver 读时 stale sync + 手动同步 rebuild；定时轮询出界 |
| D6 | 登记与索引重建全同步（前端 loading） |
| D7 | 首批登记提交原子创建 manifest（root 一致性 409 拒绝） |
| D8 | preflight 冻结快照磁盘持久化（TTL 24h，重启存活） |

## 关键技术点

- `RepositoryStore::for_project(paths, project)`：multi_repo→LogicalCodebaseFeature enabled；project 查找失败→404（不静默降级）。侵入面：28 文件中约 16-17 处生产 `RepositoryStore::new` + 约 27 处 `CreateProjectInput` 构造点，独立 task 逐点核对。
- registry 升级：扩展现有 `InitializationRunRegistry`（key 已泛型化）值类型为 `HashMap<key, CancellationToken>`，单仓旧调用方无感；另加 per-project 索引重建 try-register 注册表产生 409。
- apply_index 状态机：先 `store.create` 持久化 Building（并发 GET 可见 rebuilding）→ 命令 → 成功 replace_active / 失败转 Failed；首建（Initialize 模式）期间漂移记 Failed，重建/同步期间漂移记 stale。
- 漂移语义对齐既有 coordinator：revision 漂移=item 级 needs_attention；identity 变化=整批 409 registration_batch_conflict。
- 错误码全表（15 个新增稳定码，含 aggregate_root_* 5 个 422/409 映射）见设计文档 §6。
- production `AggregateInitializationDependencies`：真实 skills 准备 + AggregateRootPreflight + index 接线，替换 NoopSkills/NoopPreflight。

## 测试策略

完成定义=it_web 经**真实 HTTP** 复现 REG-01/INIT-01/IDX-01/PLN-01（含 change_order blocker）；单仓 project 行为零变化回归；lib/it_web/前端/tsc/clippy/fmt/specs 全门禁基线不降。

## 已知限制（park）

- replace_active 跨文件非事务：single-writer 解决并发，崩溃窗口保留（与既有行为一致，不恶化）。
- poll_due 定时兜底出界；REQ-IND"定时"场景以手动重建+读时 sync 满足。

## 已知限制（merge 前标注，2026-08-19）

- **多仓 WorkItemPlan 目前止于 Draft**：schema-v2 finalize 不持久化 per-target LifecycleWorkItemRecord，且多 target plan 的 plan 级 start_generation 被 repository_routing_ambiguous 拒绝——"多 target plan → coding"生产链路待专项 change（见 ledger park-1/park-2 与最终审查报告）。
- aggregate_initialization_in_progress / aggregate_initialization_state_rejected 两码暂落默认 500，待后续补 409 映射。
