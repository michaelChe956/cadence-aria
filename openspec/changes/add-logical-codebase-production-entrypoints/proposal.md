# Proposal: add-logical-codebase-production-entrypoints

## Why

多仓库支持（BCD 全阶段）已完成实现与测试，但双轮只读生产审计（cadence/notes/2026-08-18_生产入口审计_*.md）确认 6 项"功能已实现但生产无入口"的缺口：登记链无 Web 入口（Task 17 自认未接入但 ledger 标 complete）、聚合索引构建无生产触发、新建 issue 无 selection 初始写入方、聚合初始化 POST 只 begin 不 execute（幽灵入口）、Web 默认 LogicalCodebaseFeature disabled、前端无登记/初始化 UI。结果：人工端到端测试（REG-01→…→PLN-01）无法开始，it_web 全绿掩盖了 Web 层断头（fixture 直调 coordinator 绕过路由）。

## What Changes

1. Project 增加 multi_repo opt-in（创建时勾选，不可切换）；`RepositoryStore::for_project` 按项目启用 LogicalCodebaseFeature；多仓 project 下 legacy repositories 端点受限（GET 投影 / POST、DELETE 409）。
2. 登记生产入口：5 个同步 HTTP 端点（preflight 冻结快照持久化 / 提交 / 查询 / resume / cancel），串接 AggregateRootPreflight 与 LogicalCodebaseRegistrationCoordinator；首批提交原子创建 manifest，root 不一致 409。
3. 聚合初始化行为修复：POST begin → 共享 registry（含 CancellationToken）→ tokio::spawn(execute)；生产级依赖替换 Noop；cancel 经 token 在 step 边界生效；重启恢复标 interrupted-Failed。
4. 聚合索引生产触发三层：初始化成功尾步首建（single-writer 锁 + Building 先持久化）；resolver 读时 stale sync（修 assess 将 degraded 误报 active 的 bug）；手动同步 rebuild 端点（try-register 409）。
5. 多仓 project 创建 issue 时服务端自动写 all_members selection（补偿事务保证原子性）；DesignSpecRecord change_order blocker 校验落点补齐。
6. 前端：project 创建勾选、登记向导、聚合初始化卡片、索引卡片（均含 loading）。

## Capabilities

### New Capabilities

- `multi-repo-project-mode`：project 级多仓库模式 opt-in、feature 按项目构造、legacy 端点防护。

### Modified Capabilities

- `logical-codebase-registration`：新增 HTTP 生产入口契约（preflight 快照 store、同步批次、漂移语义、manifest 首批原子创建与 root 一致性）。
- `logical-codebase-aggregate-index`：新增生产触发契约（初始化尾步首建、读时 sync、手动 rebuild 端点、Building 持久化、首建漂移 Failed）。
- `logical-codebase-aggregate-planning`：issue 创建自动 all_members selection 与 repository_id=primary 语义、change_order blocker 落点。
- `non-interrupt-repository-bootstrap`：聚合初始化 execute 接线、token cancel、重启恢复语义。

## Impact

- 后端：src/web（app.rs 路由、handlers 新建 2 个/修改 3 个、state.rs、types.rs、support.rs 错误码）、src/product（project_store、repository_store for_project、logical_codebase 模块导出与多处小修、aggregate_index apply_index/assess、issue selection 补偿）。
- 前端：web/src（CreateProjectDialog、逻辑代码库页 3 卡片、新 API client/types）。
- 兼容：单仓 project 行为零变化（默认 disabled 不变；旧 JSON multi_repo serde default=false）。
- 侵入面：约 16-17 处生产 `RepositoryStore::new`、约 27 处 `CreateProjectInput` 构造点（Plan 中独立 task 逐点核对）。
- 完成定义：it_web 经真实 HTTP 复现 REG-01/INIT-01/IDX-01/PLN-01（含 change_order blocker）。
