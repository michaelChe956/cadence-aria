# Tasks: 逻辑代码库多仓库支持

> 高层工作包。精确文件、命令、测试与提交步骤由 superpowers:writing-plans 在 Plan 中展开；本文件只定义工作包与契约映射。实施顺序：P0 先行，P1 后置。每个 WP 在 Plan 阶段补充 feature flag、依赖、估算与回滚点（受本契约约束，不得重新定义验收）。

## P0 工作包（preview/experimental）

### WP0 领域边界、身份与数据迁移
- 建立 `ProjectCodebaseManifest`、`CodebaseMemberRecord`、`RepositoryCheckoutRecord`、`AggregateIndexRecord` 及身份模型（logical/checkout/index 三分）。
- 数据迁移：单仓 Project 默认 member、`repo_id` 投影 focus/primary、双读双写窗口、删除引用约束、稳定 UUID + tombstone 替代 `len+1`。
- 关联契约：REQ-REG-01、REQ-REG-04、REQ-REG-08、REQ-REG-09、REQ-TGT-01（身份支撑）、REQ-ENV-07（指针 locator 支撑）。

### WP1 批量成员登记（本地已 clone）
- 批量登记 API/UI：manifest 导入、目录扫描发现、预检分类展示、批次状态、幂等、取消/重启恢复、单项重试、TOCTOU 复验。
- 首版只登记本地已 clone 目录，不做远程 clone/fetch。
- 关联契约：REQ-REG-01、REQ-REG-02、REQ-REG-03、REQ-REG-04、REQ-REG-07、REQ-REG-09。

### WP2 聚合初始化与既有能力改造
- `AggregateInitializationOperation`：聚合根准入 preflight、聚合级规则/MCP/OpenSpec/示例生成、机器级 CadenceSkills 与全局工具一次准备、前端差异化 profile。
- 改造既有 capability：`repository-initialization-progress`（聚合 operation、不向成员仓提交推送）、`non-interrupt-repository-bootstrap`（聚合根执行一次）；传统单仓登记保持原契约。
- 关联契约：REQ-REG-05（MODIFIED）、REQ-REG-07。

### WP3 聚合 CodeGraph 索引生命周期
- 聚合索引建立（版本钉定）、成员 allowlist 范围与排除、快照采集（Cadence 侧）、按需 sync + 定时兜底、single-writer、last-known-good、stale/degraded 状态与诊断。
- 关联契约：REQ-IND-01 ~ REQ-IND-04。

## P1 工作包（experimental + supervised）

### WP4 聚合规划上下文
- `RepositoryContextSet` resolver、`IssueCodebaseSelection`、`PlanningContextSnapshot`、唯一 `PlanningContextResolver`（REQ-PLN-07）、规划 provider 从聚合根 best-effort 只读启动、inventory/profile/index revision 注入与预算截断。
- Story/Design 聚合视野改造（involved_repositories、改动顺序）。
- 关联契约：REQ-PLN-01 ~ REQ-PLN-07。

### WP5 聚合计划到单仓执行的安全桥（拆分实施）
- **WP5a target 编译链**：`target_repository_id` 贯穿 Outline/Draft/compile/validator/runtime；校验集为 Issue selection 有效成员；缺失 target 即 blocker（REQ-TGT-01 ~ REQ-TGT-05）。
- **WP5b worktree/group 迁移**：attempt target 冻结；shared worktree `(project, issue, repository)` 键；mixed-target group 一律拒绝；改造 `coding-attempt-deletion`、`work-item-group-deletion`（REQ-COD-01 ~ REQ-COD-04）。
- **WP5c 交付**：每仓 branch/commit/push + ReviewRequest（GitBranchOnly），自动 PR 后置（REQ-COD-06）。
- **WP5d 证据服务**：跨仓只读证据检索中介（ACL/snapshot/token 预算/审计，启发式）（REQ-COD-05）。
- **WP5e envelope 与路由**：SessionPolicyEnvelope + ValidatedSessionLaunchPolicy（覆盖两条 provider 栈 + 禁止 fallback）+ 路由级 fail-closed + resume 一致 + 配置来源隔离（REQ-ENV-01 ~ REQ-ENV-06）。
- **WP5f 最小指针**：每仓指针发布（独立 worktree/branch + ReviewRequest）+ 改造 `project-rule-aware-prompts`（REQ-ENV-07）。

### WP6 测试、迁移演练与可观测性（贯穿质量泳道）
- 批次幂等/并发/零副作用；聚合索引覆盖与 freshness；target 贯穿与 blocker；per-repo worktree 锁；envelope 路由 fail-closed（两条栈 + fallback 关闭）；旧 `repo_id` 兼容迁移；2–3 仓端到端与 50 仓预算测试；provider 版本钉定安全测试；OpenSpec strict validate 通过为发布门。
- 关联契约：全部 REQ 的验收门；每 WP 单独设置验收与回滚映射，不以「覆盖全部」替代。

## 明确不在本 change 范围（YAGNI）

- 远程代码托管平台导入、动态 clone/fetch、自动创建远端 PR/MR。
- 自动依赖图/服务拓扑（D9）。
- 完整多仓 coordinator、跨仓原子 commit/PR/回滚状态机、mixed-target 自动拆分。
- codegraph DB merge/federation、Sourcegraph/scip-java 部署。
- Codex exec adapter、通用动态 provider 评分器、OS 级沙箱（后置）。
