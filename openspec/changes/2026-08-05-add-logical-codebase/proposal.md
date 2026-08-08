## Why

用户有大量存量 Java Web 微服务项目，一个业务项目包含 30–50 个 git 仓库（后端微服务 + 2–3 个前端工程），仓库之间存在 Dubbo/HTTP 调用关系，一个 Issue 可能横跨 10+ 个仓库改动。

当前「代码库添加」是逐仓库手工录入，且每个仓库都要单独跑一遍 Claude Code 初始化（4 次 Claude 会话 × 50 仓 = 120–200 次），`IssueRecord.repo_id` 只能绑定单个仓库，workspace / coding / worktree / provider 全链路均为单仓模型。用户无法把一个多仓库业务项目当作一个整体接入，AI 也无法在跨仓库需求上同时理解多个仓库。

用户已确认的解决方向：**把一个业务 Project 视为一个「逻辑代码库」，关联同一公共父目录下的全部真实 git 仓库；规划类产物（Issue/Story/Design/Work Item 计划）使用聚合上下文（全集可检索，AI 看全部仓库），执行类产物（Coding）保持单目标仓（一次只改一个，其他只看不碰）**。

## What Changes

- 引入「逻辑代码库」概念：`Project` 即逻辑代码库作用域，通过 manifest 登记同一非 git 公共父目录下的 30–50 个真实 git 仓库为成员（attach-only，P0 零 Git 副作用，不操作主 checkout）。
- 批量登记：支持扫描公共父目录发现子仓、manifest 导入、逐项预检（非 git/重复/嵌套/脏仓分类展示）、批次状态、幂等与单项重试；含旧数据迁移与聚合根准入。
- 聚合初始化：初始化 6 步从「每仓跑一遍」改为「机器一次 + 聚合根一次」；规则/MCP/OpenSpec/示例生成到聚合根；每仓最小安全指针在 P1 经独立 worktree/branch 受控发布并生成 ReviewRequest。
- 聚合 CodeGraph 索引：在非 git 公共父目录建一份统一索引覆盖全部成员仓（版本钉定）；按需 + 定时刷新，stale 状态可见，快照由 Cadence 采集。
- 聚合规划上下文：Story/Design/Work Item 计划由 provider 从聚合根只读启动（P1 best-effort），注入成员清单、目标仓 profile、索引摘要与政策 envelope；输出要求每个 Work Item 携带唯一 `target_repository_id`，不确定则 blocker。
- 单仓执行：每个 Work Item 在目标仓的 `.worktrees/aria-issues/{issue}` 中 coding（由 Cadence 显式建 worktree，主 checkout 不动）；跨仓只读证据经聚合索引检索注入（启发式）；不同仓可并行、同仓串行；MVP 一律拒绝 mixed-target group。
- 安全边界（P1 experimental + supervised）：SessionPolicyEnvelope 冻结每次 provider run 的政策/目标/配置；真实 provider 只接受 validated launch policy、禁止无政策 fallback；路由级 fail-closed（阻塞不降级）；OS 级沙箱后置。
- 交付：P1 每仓独立 branch/commit/push + ReviewRequest（人工评审），自动创建 GitHub/GitLab PR 不在本 change 范围。
- 前端工程纳入：成员含 repo_type，前端初始化 profile 与后端差异化，不套 Java 六步。

## Capabilities

### New Capabilities

- `logical-codebase-registration`：以 Project 为逻辑代码库，批量登记公共父目录下多个真实 git 仓库为成员（attach-only、预检、批次状态、幂等、迁移、聚合根准入）。
- `logical-codebase-aggregate-index`：在非 git 聚合根建统一 CodeGraph 索引，管理范围、快照、freshness 与生命周期。
- `logical-codebase-aggregate-planning`：规划类产物使用聚合上下文（RepositoryContextSet + IssueCodebaseSelection + PlanningContextSnapshot），Story/Design/Work Item 计划可跨仓理解并列出涉及仓库。
- `target-aware-work-item`：Work Item 携带唯一 `target_repository_id`（校验集为 Issue selection 有效成员），贯穿拆分/编译/校验/运行链路；缺失或非法 target 一律 blocker。
- `per-repo-coding-execution`：Coding 在目标仓独立 worktree 执行，shared worktree 键升级为 `(project, issue, repository)`，attempt 冻结 target 快照，mixed-target group 一律拒绝，交付为推分支 + ReviewRequest。
- `session-policy-envelope`：为逻辑代码库流程每次 provider run 冻结集中政策/目标/配置快照，适配器只接受 validated launch policy，路由级 fail-closed，禁止无政策 fallback（P1 experimental + supervised）。

### Modified Capabilities

- `repository-initialization-progress`：为逻辑代码库提供聚合初始化 operation（可轮询、幂等、可恢复），不再向成员仓执行 git 提交推送；传统单仓登记保持原契约。
- `non-interrupt-repository-bootstrap`：逻辑代码库场景下四个无中断命令在聚合根执行一次，逐仓本地化由确定性程序完成。
- `project-rule-aware-prompts`：逻辑代码库 prompt 以 envelope 校验的聚合政策为权威输入，每仓最小指针仅负责发现。
- `coding-attempt-deletion`：shared worktree 清理与删除判定升级为 `(project, issue, repository)` 键。
- `work-item-group-deletion`：mixed-target group 一律拒绝，删除清理按仓库键执行。

## Non-goals

- 不把聚合根注册为 RepositoryRecord；不把 50 个仓库合并为一个 git 仓库。
- 不做自动依赖图/服务拓扑分析（规划由 provider 从索引按需检索）；不绑定 Dubbo/HTTP 特定建模。
- 不支持远程代码托管平台导入与动态 clone/fetch（首版本地已 clone）。
- 不创建自动 GitHub/GitLab PR（P1 交付为推分支 + 人工评审）；不做跨仓原子提交/回滚。
- 不实现 OS 级沙箱（P1 为 best-effort 行为级隔离）；不做完整多仓 coordinator 与 mixed-target 自动拆分。
- 不实现 codegraph DB merge/federation、Sourcegraph/scip-java 部署、Codex exec adapter、通用动态 provider 评分器。
- 不把全部成员源码注入 provider context（全集可检索，但按预算注入摘要/证据）。

## Impact

- 后端：repository_store、初始化协调器、workspace context、coding 引擎、provider 抽象、数据模型与迁移、Git worktree/锁、聚合索引生命周期。
- 前端：代码库添加弹窗（批量/预检/批次）、Issue 创建（逻辑代码库 + 多仓选择）、Story/Design/WorkItem 展示（涉及仓库/按仓分组）、编码工作区（跨仓只读证据、per-repo 状态、ReviewRequest）。
- 测试：批次幂等/并发/零副作用、聚合索引覆盖与 freshness、target 贯穿与 blocker、per-repo worktree 锁、envelope 路由 fail-closed、旧 `repo_id` 兼容迁移；2–3 仓端到端与 50 仓预算测试；provider 版本钉定安全测试。
- 依赖：继续使用现有 Claude Code / Codex Provider 与 CodeGraph CLI；不新增第三方运行时依赖。
- 交付节奏：P0（registration + aggregate-index）为 preview/experimental；P1（aggregate-planning + target-aware-work-item + per-repo-coding-execution + session-policy-envelope）为 experimental + supervised；路由级 fail-closed 仅代表启动阻塞，不代表端到端 OS 级隔离。
