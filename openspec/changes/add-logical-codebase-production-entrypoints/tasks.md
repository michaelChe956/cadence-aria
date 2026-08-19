# Tasks: add-logical-codebase-production-entrypoints

## 1. Project 多仓模式基础

- [x] 1.1 ProjectRecord/CreateProjectInput/CreateProjectRequest/ProjectDto 增加 multi_repo（serde default=false），project_store 读写与旧数据兼容测试
- [x] 1.2 新增 RepositoryStore::for_project（multi_repo→feature enabled；project 缺失→ProjectNotFound），lib 单测映射与失败语义
- [x] 1.3 替换约 16-17 处生产 RepositoryStore::new 为 for_project（逐点核对清单入 task 报告；测试 fixture 保持 new），跑全量 lib 确认单仓零回归
- [x] 1.4 legacy 端点防护：多仓 project 下 POST/DELETE /repositories 与 GET /repository-initializations/{oid} → 409 legacy_repository_endpoint_on_multi_repo；GET /repositories 返回成员投影；单仓误调登记/索引端点 → 409 logical_codebase_feature_disabled（it_web 负向用例）

## 2. 登记生产入口

- [x] 2.1 新建 RegistrationPreflightSnapshotStore（磁盘 preflights/{id}.json：冻结 RegistrationCandidate 全量 + aggregate_root + created_at；TTL 24h 读取时惰性过期），lib 单测读写/TTL/重启存活
- [x] 2.2 mod.rs 补导出 RegistrationBatch* 系列；新建 handlers/logical_codebase_registration.rs：preflight 端点（AggregateRootPreflight::validate → coordinator.preflight → 快照落盘），aggregate_root_* 5 个错误码 HTTP 映射入 support.rs
- [x] 2.3 提交端点：load 快照→重建 ConfirmedRegistrationBatchInput（勾选 needs_attention 项→include_needs_attention=true）→同步执行；revision 漂移 item 级 needs_attention、identity 变化 409 registration_batch_conflict；首批 manifest 原子创建与 root 一致性 409 aggregate_root_mismatch（lib 单测 + it_web）
- [x] 2.4 GET/resume/cancel 端点（cancel 仅 Queued/PartialFailed，终态 409 not_cancelable），it_web 全链：7 类预检分类→提交→幂等→resume→漂移两态
- [x] 2.5 前端登记向导 + registration API client/types（分类展示、勾选确认、同步提交 loading、批次结果/partial 恢复）

## 3. 聚合初始化执行接线

- [x] 3.1 扩展 InitializationRunRegistry：值升级 HashMap<key, CancellationToken>（单仓调用方无感），lib 单测注册/触发/Drop 清理
- [x] 3.2 production AggregateInitializationDependencies（真实 skills 准备 + AggregateRootPreflight + index 接线，替换 Noop），注入 AppState 共享
- [x] 3.3 POST 改为 begin → registry lease → tokio::spawn(execute)；cancel 取 token 在 step 边界生效；GET 对 Running 无 lease → recover_interrupted（Failed+interrupted）；重触发新幂等键（it_web：轮询推进 Completed、cancel 边界、重启恢复）

## 4. 聚合索引生产触发

- [x] 4.1 apply_index 状态机：store.create 先持久化 Building → 命令 → replace_active/转 Failed；build 包 single-writer 锁；前后双快照，重建/同步漂移→stale、首建漂移→Failed（lib 单测）
- [x] 4.2 修 AggregateIndexFreshnessService::assess 对 Degraded 误报 active 的 bug（保留 last-known-good 告警）
- [x] 4.3 PlanningContextResolver 读路径接 sync_if_stale（spawn_blocking；degraded 不重建仅注入告警）（lib 单测 + it_web stale→sync）
- [x] 4.4 handlers/aggregate_index.rs：POST rebuild（同步 spawn_blocking + try-register 409 aggregate_index_rebuild_in_progress）+ GET active（四态映射：Failed 有 good→degraded 无→missing）；初始化 execute 成功尾步 spawn 首建（it_web：初始化后 active 出现、并发 409）
- [x] 4.5 前端索引卡片 + aggregate-index API client（状态徽标、stale 提示、重建按钮 loading）

## 5. issue selection 与规划解锁

- [x] 5.1 create_product_issue 多仓分支补偿事务（预校验→建 issue→save all_members→失败删 issue 422 issue_selection_write_failed / 删失败 orphan 告警 500）；repository_id=primary 须属 active member（lib 单测 + it_web）
- [x] 5.2 DesignSpecRecord change_order 注释修正与 blocker 校验落点（缺 change_order→blocker change_order_required_for_logical_codebase，不进 compile）
- [x] 5.3 it_web 端到端：多仓 project→登记→初始化→索引→建 issue→Story/Design/WorkItemPlan 全链真实 HTTP（含 change_order blocker 用例）——本变更完成定义

## 6. 前端收尾与全门禁

- [x] 6.1 前端聚合初始化卡片（触发/步骤进度轮询/取消，loading）+ CreateProjectDialog 多仓库模式勾选 + 前端测试补齐
- [x] 6.2 全门禁：cargo fmt/clippy -D warnings/lib/it_web 全量、前端 vitest/tsc、openspec validate --specs 26/26 + 本 change 通过；人工测试矩阵 P0 主线冒烟（REG-01→INIT-01→IDX-01→PLN-01/02/03，/tmp/test-demo）


## 7. 代码库级模式修正（v1.3 Rework，2026-08-19 追加）

- [x] R1 模型与存储：LogicalCodebaseRecord/store + logical-codebases/{lc_id}/ 子树；删除 ProjectRecord.multi_repo（旧数据兼容忽略）；迁移工具（旧 logical-codebase/ → 首 LC）
- [x] R2 统一 codebases 列表端点（混合单仓/逻辑）+ LC CRUD（POST 创建/GET 详情/DELETE 软删零 git 副作用）
- [x] R3 登记端点换形 /logical-codebases/{lc_id}/ + guard 改 require_logical_codebase（404 logical_codebase_not_found）
- [x] R4 初始化/索引端点换形 + 母 change 旧端点（members/pointer/initializations）保留默认首 LC 兼容别名
- [x] R5 routing/resolver/gateway/selection 按 issue 所属 codebase 解析（补偿事务按 LC；issue 增加 logical_codebase_id 归属；单仓 issue 零变化）
- [x] R6 废除多仓 project 防护语义（legacy_repository_endpoint_on_multi_repo 不再产出）+ 单仓端点全回归
- [ ] R7 前端：代码库混合列表 + 添加代码库弹窗模式单选（单仓→既有流程/多仓→建 LC→登记向导 auto_discover）
- [ ] R8 前端：逻辑代码库页按 LC 分区（初始化/索引/指针）+ issue 创建代码库选择 + 恢复创建 project 弹窗（撤 UX 2.1 单选）
- [ ] R9 P0 e2e 更新为新寻址全链（建 LC→登记→初始化→索引→逻辑 issue→Story/Design→Draft）+ 单仓回归。编码/交付链残留切换点清单（按 issue 所属 lc_id 寻址，单仓/无 LC 回退 legacy）：
  - [ ] gateway 调用点：gateway_factory build → build_for_lc（workspace_ws_handler/socket.rs、coding_ws_handler/runner/task.rs、aggregate_initialization/production_dependencies.inc.rs）——R5 fix round 1 已接线，R9 仅 e2e 覆盖
  - [ ] build_attempt_target_snapshot（target_snapshot.rs:34）：LogicalCodebaseStore/AggregatePolicyArtifactStore 仍 project 级 + RepositoryStore::for_project（:50）；签名无 issue 上下文，需按 issue lc_id 扩展
  - [ ] cross_target_check（coding_workspace_engine/cross_target_check.rs:67,289）：成员 checkout 采集仍 LogicalCodebaseStore::new project 级
  - [ ] validate_logical_group_selection（handlers/coding/group.rs:450）：list_members 仍 LogicalCodebaseStore::new project 级
  - [ ] coding.rs:367 / coding/group.rs:323：build_attempt_target_snapshot 调用点未透传 issue lc_id
  - [ ] resolve_coding_attempt_repository（coding_attempt_repository.rs:34）：物理解析仍 RepositoryStore::for_project（:42）；调用点 coding_ws_handler/context.rs:89、handlers/coding.rs:458
  - [ ] RepositoryStore::for_project 残留清理：target_snapshot.rs:50、coding_attempt_repository.rs:42、coding/group.rs:411/428 改按 codebase 判定
- [ ] R10 全门禁 + 验收记录更新
