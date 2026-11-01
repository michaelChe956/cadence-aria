# Tasks: 逻辑代码库多仓库支持

> 高层工作包。精确文件、命令、测试与提交步骤由 superpowers:writing-plans 在 Plan 中展开；本文件只定义工作包与契约映射。
>
> **发布形态（用户决策）**：全量 **experimental + supervised** 发布，不再分 P0/P1。本决策消解「聚合初始化在前需真实 provider、envelope gate 在后」的分期冲突——envelope 与聚合初始化在同一发布内按依赖顺序生效。
>
> **实施顺序按依赖图（非分期）**，见文末依赖图与 §7.2 spike 前置。每个 WP 在 Plan 阶段补充 feature flag、依赖、估算与回滚点（受本契约约束，不得重新定义验收）。
>
> **Spike 前置**：进入实施 Plan 前必须完成 design.md §7.2 的四项 spike（身份域/迁移 journal、CodeGraph 2–3 仓、聚合初始化稳定 step ID、50 合成样本与预算阈值），结果回写本契约后再编写完整实施 Plan。

## WP0 领域边界、身份与数据迁移
- 建立 `ProjectCodebaseManifest`、`CodebaseMemberRecord`、`RepositoryCheckoutRecord`、`AggregateIndexRecord` 与闭合身份模型：`LogicalRepositoryId`（逻辑身份，selection/target/involved_refs 的语义类型）→ `RepositoryCheckoutId`（可用 checkout）→ `RepositoryRecord.id`（物理兼容投影）三层映射；`target_repository_id` 语义类型为 `LogicalRepositoryId`。
- 数据迁移：单仓 Project 默认 member、`repo_id` 投影 focus/primary、双读双写窗口、删除引用约束、稳定 UUID + tombstone 替代 `len+1`（替代当前 `RepositoryStore::create` 的 `repository_N` ID 分配）；迁移经 journal/version marker，可重放可恢复。
- 关联契约：REQ-REG-01、REQ-REG-04、REQ-REG-08、REQ-REG-09、REQ-TGT-01（身份支撑）、REQ-ENV-07（指针 locator 支撑）。

## WP1 批量成员登记（本地已 clone）
- 批量登记 API/UI：manifest 导入、目录扫描发现、预检分类展示、批次状态、幂等、取消/重启恢复、单项重试、TOCTOU 复验。
- 首版只登记本地已 clone 目录，不做远程 clone/fetch；新建 attach-only 的 `LogicalCodebaseRegistrationCoordinator` 与独立 API，不复用当前 `create_repository` handler（避免触发 Claude 初始化与 GitFinalize）。
- 关联契约：REQ-REG-01、REQ-REG-02、REQ-REG-03、REQ-REG-04、REQ-REG-07、REQ-REG-09。

## WP2 聚合初始化与既有能力改造
- `AggregateInitializationOperation`（独立 store/DTO/run coordinator，不复用固定六步单仓 operation，保留现有 operation 字节级兼容）：聚合根准入 preflight、聚合级规则/MCP/OpenSpec/示例生成、机器级 CadenceSkills 与全局工具一次准备、前端差异化 profile。
- **5 个稳定 step ID（spike 3 定案）**：`machine_skills`（机器级，非 provider turn）/ `aggregate_preflight`（确定性 Cadence 代码，member snapshot，非 provider turn）/ `pre_check`（1 Claude turn，聚合根）/ `rule_and_mcp_config`（1 Claude turn，聚合根，合并旧 rule_config+mcp_configuration）/ `openspec_and_examples`（1 Claude turn，聚合根）；GitFinalize 从 coordinator 调用图切断，不对成员仓 git 操作；`RepositoryInitializationRunRegistry` 泛化为 `operation_kind + project_id + operation_id`。
- 改造既有 capability：`repository-initialization-progress`（聚合 operation、不向成员仓提交推送）、`non-interrupt-repository-bootstrap`（聚合根执行一次）；传统单仓登记保持原契约。
- 关联契约：REQ-REG-02（登记零副作用）、REQ-REG-05、REQ-REG-06、REQ-INIT-01、REQ-BOOT-01。

## WP3 聚合 CodeGraph 索引生命周期
- 聚合索引建立（**exact version 钉定 v1.5.0**，spike 2 实测：#1295 在 v1.5.0 已修复）、快照采集（Cadence 侧 `git rev-parse HEAD` + dirty，CodeGraph 无 per-member revision API）、按需 sync + 定时兜底、single-writer、last-known-good、stale/degraded 状态与诊断。
- **索引范围控制（spike 2 颠覆性结论）**：CodeGraph 1.5.0 按目录递归扫描、**不按 git 边界识别成员**、CLI 无 allowlist；范围控制由 Cadence 生成并维护聚合根 `codegraph.json` 的 `exclude`（denylist），显式排除非成员目录 + `**/.worktrees/` + `**/.aria/` + 构建产物；CodeGraph 内建排除 `.git`/`build`/`node_modules`/`dist` 但不内建排除 `.worktrees`/`.aria`；index 后用 `codegraph files` + 负查询验证边界。
- 关联契约：REQ-IND-01 ~ REQ-IND-04。

## WP4 聚合规划上下文
- `RepositoryContextSet` resolver、`IssueCodebaseSelection`、`PlanningContextSnapshot`、唯一 `PlanningContextResolver`（REQ-PLN-07）、规划 provider 从聚合根 best-effort 只读启动、inventory/profile/index revision 注入与预算截断（阈值由 spike 定值）。
- Story/Design 聚合视野改造（involved_repositories、改动顺序）。
- 关联契约：REQ-PLN-01 ~ REQ-PLN-07。

## WP5 聚合计划到单仓执行的安全桥（拆分实施）
- **WP5a target 编译链**：`target_repository_id`（语义类型 `LogicalRepositoryId`）贯穿 Outline/Draft/compile/validator/runtime；校验集为 Issue selection 有效成员；缺失 target 即 blocker（REQ-TGT-01 ~ REQ-TGT-05）。改造 `compile_support.rs` 现有「从第一个 Story 推导单一 repository_id 填全部 item」的默认路径。
- **WP5b worktree/group 迁移**：attempt target 冻结（含三层身份映射快照）；shared worktree `(project, issue, repository)` 键，路径重分片（`issues/{issue}/shared-worktrees/{repository_id}.json`），锁 API 增加 repository 参数，旧 `issue-shared-worktree.json` 经 journal 迁移（验 record.repository_id 一致性→原子写新 record→legacy tombstone/redirect→无活动引用时清除旧文件）；mixed-target group 一律拒绝；改造 `coding-attempt-deletion`、`work-item-group-deletion`（REQ-COD-01 ~ REQ-COD-04、REQ-DEL-01/02、REQ-GRP-01/02）。
- **WP5c 交付**：每仓 branch/commit/push + ReviewRequest（GitBranchOnly），自动 PR 后置（REQ-COD-06）。
- **WP5d 证据服务**：跨仓只读证据检索中介（ACL/snapshot/token 预算/审计，启发式）（REQ-COD-05）。
- **WP5e envelope 与路由**：统一 `LogicalCodebaseProviderGateway` 为逻辑代码库 provider 唯一建造入口（覆盖同步栈 `ProviderAdapter::run` + 流式栈 `StreamingProviderAdapter::start` 全部入口），SessionPolicyEnvelope + ValidatedSessionLaunchPolicy（逻辑代码库调用固定 `allow_legacy_stream_fallback=false`，禁止裸 input 启动，Fake 经 registry/编译期隔离）+ 路由级 fail-closed + resume 一致 + 配置来源隔离 + managed-settings 检测标注（REQ-ENV-01 ~ REQ-ENV-06）。Codex 当前 `danger-full-access` 经 gateway 路由阻断，不限 UI 隐藏。
- **WP5f 最小指针**：每仓指针发布（独立 worktree/branch + ReviewRequest）+ 改造 `project-rule-aware-prompts`（REQ-ENV-07、REQ-PROMPT-01/02）。

## WP6 测试、迁移演练与可观测性（贯穿质量泳道）
- 按 §7.4 测试分层执行：L1 deterministic unit / L2 integration（2–3 仓端到端）/ L3 CLI contract fixture / L4 production verification（opt-in）。
- 批次幂等/并发/零副作用；聚合索引覆盖与 freshness；target 贯穿与 blocker；per-repo worktree 锁；envelope 路由 fail-closed（全部入口 + fallback 关闭）；旧 `repo_id` 兼容迁移；50 合成仓预算测试；provider 版本钉定 + 越界矩阵 fixture（Claude #41758/#37210/#43407、Codex #24214）；OpenSpec strict validate 通过为发布门。
- 每个 WP 同步增设测试与回滚项（非末尾集中验证）。
- 关联契约：全部 REQ 的验收门。

## 依赖图（实施顺序，非分期）

**关键时序约束**：WP2 的 provider step（pre_check/rule_and_mcp_config/openspec_and_examples）须经 LogicalCodebaseProviderGateway，故 WP5e SHALL 在 WP2 provider step 可运行前就绪；WP0 SHALL 生成 bootstrap AggregatePolicyArtifact（消除 bootstrap 环）。WP2 的 machine_skills/aggregate_preflight（非 provider turn）可在 WP5e 就绪前先行。

```
WP0（含 bootstrap policy artifact）
  ├─→ WP1
  ├─→ WP3 ─┬─→ WP4 ──┐
  └─→ WP5e │         │
            │         │
  WP5e ──→ WP2(provider step gated)
            │         │
            └─→ WP5d ←┘ (WP0 + WP3 + WP4)

WP4 + WP0 ──→ WP5a ──┬─→ WP5b ──→ WP5c（+ WP5e）
                     │
WP5b + WP5c + WP5e ──┴─→ WP5f（+ WP0）

WP6：贯穿所有 WP（每个 WP 同步测试与回滚）
```

## 明确不在本 change 范围（YAGNI）

- 远程代码托管平台导入、动态 clone/fetch、自动创建远端 PR/MR。
- 自动依赖图/服务拓扑（D9）。
- 完整多仓 coordinator、跨仓原子 commit/PR/回滚状态机、mixed-target 自动拆分。
- codegraph DB merge/federation、Sourcegraph/scip-java 部署。
- Codex exec adapter、通用动态 provider 评分器、OS 级沙箱（后置）。
