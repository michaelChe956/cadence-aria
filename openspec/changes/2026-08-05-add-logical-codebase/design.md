# 设计：逻辑代码库多仓库支持

> 依据：`cadence/designs/2026-08-05_方案设计_逻辑代码库多仓库支持_v1.0.md`（v1.6，用户确认 + 三方评审收敛）；经 OpenSpec proposal 三司会审（researcher/worker/reviewer）修订。

## 1. 核心架构边界

- **Project = 逻辑代码库作用域**（产品不变量 D1）。不新增顶层 `LogicalCodebaseRecord`；`ProjectRecord` 承载逻辑代码库语义。
- **公共父目录假设（D3）**：成员仓库均位于同一非 git 公共父目录下；父目录作聚合 CodeGraph 索引根与规划 provider 驱动目录。聚合资产由 Cadence `.aria` 领域存储管理。因果链：非 git 父目录 → 无 super-repo `.gitignore` → 子仓非 gitignored → codegraph 自动发现并索引，无需 `includeIgnored`。
- **读聚合、写单仓（D2）**：规划类产物使用聚合上下文（全集可检索，不注入全部源码）；每个 Work Item / Coding attempt 有且仅有一个 `target_repository_id`；主 checkout 不被直接修改。
- **聚合根绝不注册为 RepositoryRecord**；`RepositoryRecord` 始终代表一个物理 git 仓。
- **身份三分**：LogicalRepositoryIdentity（稳定 UUID）/ RepositoryCheckoutIdentity（checkout path）/ AggregateIndexIdentity（index artifact）。`repo_hash` 仅表达 checkout path，不作逻辑身份或去重凭据。
- **权威数据源（C3）**：`.aria/projects/{id}/logical-codebase/manifest.json` 为唯一领域权威；`catalog/repos.yaml` 为可读投影/导入输入；根 CLAUDE.md/AGENTS.md/MCP 配置为 revisioned projection。

## 2. 关键决策与权衡

| 决策 | 选择 | 权衡 |
|---|---|---|
| D3 父目录 | 非 git 目录 | 规避外层 git 语义陷阱与 includeIgnored 依赖；聚合资产由 `.aria` 管理。**CodeGraph 版本钉定**，避开 v1.4.1（#1295 includeIgnored 回归） |
| D8 worktree | 每仓 `.worktrees/aria-issues/{issue}`，主 checkout 不动 | **由 Cadence 显式 `git worktree add` + cwd 钉死**，不依赖 provider 自带 worktree 沙箱（Claude #64315/#42034/#44220、Codex #14338/#15505 为已知坑）；仅 shared worktree 键升级为 `(project, issue, repository)` |
| D9 依赖图 | 不做服务拓扑建模 | 规划由 provider 从聚合索引按需检索（**启发式符号/字符串近似，非 Java 语义级**）；未来可演进 Nx Polygraph 式依赖图 |
| D12 沙箱 | P1 coding = experimental + supervised（用户确认） | 先做「配置目标 + 不暴露兄弟仓」行为级隔离（两级状态：`best_effort_configured` / `production_verified_readonly`）；OS 级沙箱后置；Claude plan 模式仅为 prompt 层，硬只读需 hook 或 Codex read-only |
| D13 索引刷新 | 按需 sync + 定时兜底 | 快照由 Cadence 用 `git rev-parse HEAD` 采集（CodeGraph 无 per-member revision API）；索引反映主 checkout 快照状态，worktree 不入索引 |
| D14 指针 | P1 随 coding 上线，独立 worktree/branch 受控发布 + ReviewRequest | P0 保持零副作用；指针写入是受控副作用，非「零副作用」 |
| 初始化 | 机器一次 + 聚合根一次（5.5） | provider 必要内容集中到聚合根；每仓只放最小指针；GitFinalize 逐仓提交仅保留给传统单仓路径 |
| Codex | 保持 app-server；当前 unsupported，改造后 experimental | `danger-full-access` 必须改为受限写配置（`workspace-write` 或等价权限 profile，按当时稳定形态钉定）+ 版本钉定 + 越界测试；权限 profiles 官方标 Beta，列为已知风险 |
| 交付 | P1 = 推分支 + ReviewRequest | 当前代码只有 GitBranchOnly 交付，无外部 PR 集成；自动 PR 不在本 change 范围（用户确认） |

## 3. 数据模型

### 3.1 新增实体（权威记录在 `.aria/projects/{id}/logical-codebase/`）

- `ProjectCodebaseManifest`：project_id、logical_codebase_id、provider_context_root、layout、active_aggregate_index_id、context_policy、membership_revision。
- `CodebaseMemberRecord`：logical_repository_id、repository_id（兼容）、alias、role、ordinal、source_identity、repo_type、tech_stack、owner、tags、default_ref。
- `RepositoryCheckoutRecord`：checkout_id、logical_repository_id、canonical_path、checkout_path_hash、kind、revision、availability。
- `AggregateIndexRecord`：aggregate_index_id、aggregate_root、indexer/format_version、codegraph_version（钉定）、membership_revision、config_digest、status、indexed_at。
- `AggregateIndexMemberSnapshot`：aggregate_index_id、logical_repository_id、checkout_id、checkout_revision、included、indexed_at（不可变证据，由 Cadence 采集）。
- `IssueCodebaseSelection`：project_id、issue_id、selection_policy（all_members/explicit）、included/excluded/focus_repository_ids（focus 可为多值、必须在 include 内、exclude 优先）、snapshot_ref。
- `PlanningContextSnapshot`：membership_revision、每仓 checkout revision/dirty/availability、index revision、policy digest、access fingerprint。
- `AggregatePolicyArtifact`：policy_revision、正文、content_digest、生成时间。
- `SessionPolicyEnvelope`：见 §5。

### 3.2 产物模型变化

| 产物 | 现状 | 改为 |
|---|---|---|
| StorySpec | `repository_id` 单值 | `logical_codebase_ref` + `involved_repositories` + 可选 `focus_repository` |
| DesignSpec | 回落 `issue.repo_id` | 显式 `logical_codebase_ref` + `involved_repositories` |
| WorkItem | `repository_id` 单值（全塞 primary） | `target_repository_id` 必填（校验集 = Issue selection 有效成员）+ `depends_on` 排序 |
| CodingAttempt | 单 worktree/branch | 冻结 `target_repository_id` + checkout + policy digest |
| IssueSharedWorktree | `(project, issue)` 单记录 | `(project, issue, repository)` 键，同仓串行/异仓并行 |

### 3.3 迁移契约（C5 / REQ-REG-08）

- 既有单仓 Project 自动生成默认 member；`IssueRecord.repo_id` 迁移为 focus/primary 投影，双读双写窗口内保持兼容；双写冲突时以新字段（selection）为权威。
- 新增字段 serde `default`；旧数据读取走兼容投影。
- `RepositoryStore::delete` 先做反向引用检查（member/binding/attempt/worktree/index）；缺失引用约束时阻断或 tombstone。
- 删除重导 ID：稳定 UUID + tombstone/source identity 映射，禁止 `len+1` 顺序 ID。
- feature flag 关闭时回退到单仓行为；已生成的多仓 Work Item/attempt 标记兼容投影或阻塞，不静默改写到错误仓库。

## 4. 初始化与索引生命周期

### 4.1 聚合初始化（C4 / REQ-REG-05 MODIFIED）

- 新增 `AggregateInitializationOperation`（manifest revision、成员状态机、幂等 key、取消清理、审计），不复用固定六步单仓 operation；传统单仓登记保持原契约。
- 聚合根准入 preflight（REQ-REG-09）：canonical 非 git、仅含 manifest 成员、排除 `.worktrees/**`/`.git/**`/`.aria/**`/构建产物/凭据。
- 每仓最小指针发布在 P1，经独立 worktree/branch + ReviewRequest，处理已有 CLAUDE.md/AGENTS.md 合并冲突，可回滚。

### 4.2 聚合 CodeGraph 索引（C1/C2 / logical-codebase-aggregate-index）

- 在非 git 聚合根 `codegraph init` 一次；主路径无需 includeIgnored（D3）；**CodeGraph 版本钉定**避开 #1295。
- 索引范围基于 manifest 成员 allowlist；执行 worktree 不入索引。
- freshness：快照由 Cadence 采集（`git rev-parse HEAD` + dirty）；按需 sync + 定时兜底；stale/degraded 由 Cadence 对比快照推断并显式呈现；single-writer 锁 + last-known-good。

## 5. 安全与 provider（P1 experimental + supervised）

### 5.1 SessionPolicyEnvelope（覆盖两条 provider 调用栈）

- 两层：持久化 `AggregatePolicyArtifact` + 不可变 `SessionPolicyEnvelope`。
- **适用范围**：逻辑代码库流程的两条栈——规划栈（`ProviderAdapter::run(AdapterInput)`）与 coding/初始化栈（`StreamingProviderAdapter::start(StreamingProviderInput)`）；真实 provider 的 legacy fallback 与裸 input 启动被关闭（REQ-ENV-02）。
- 三落点：产品层 envelope、adapter 输入 `ValidatedSessionLaunchPolicy`（不可缺省）、prompt 可见性层。
- 硬门（最小闭环 3 条先做，其余后置）：① 只读/单写绑定可执行边界（P1 为配置目标 + pre/post 检测）；⑤ resume 时 policy/target/版本/capability 一致才允许；⑦ 禁止无政策 fallback、禁止降级 full-access。
- 配置来源隔离：Aria-owned settings/MCP bundle；审计 user/project/local/env/子仓 MCP 合并优先级；managed-settings 优先级高于注入时列为已知 gap。

### 5.2 Provider 路由（路由级 fail-closed）

- 规划：Claude 只读模式（P1 best-effort 只读）；目标 coding：Claude single-target 配置（P1 experimental）；Codex：显式 experimental opt-in（改造受限写配置前为 unsupported）。
- capability 记录 exact version / adapter dialect / evidence 状态（declared / fixture_verified / production_verified）；能力不满足 → 阻塞，不降级。
- 明确：路由级 fail-closed ≠ 端到端 coding 已达 fail-closed supported。

### 5.3 Coding 执行

- 每 Work Item → 目标仓 `.worktrees/aria-issues/{issue}`；**由 Cadence 显式建 worktree + cwd 钉死**；attempt 冻结 target 快照（REQ-COD-01/02）。
- 跨仓只读证据经聚合索引检索注入（中介查询 + ACL + snapshot + token 预算，启发式），Coder 不持有聚合根（REQ-COD-05）。
- 不同仓并行、同仓串行；MVP 一律拒绝 mixed-target group（REQ-COD-04）。
- 交付：每仓 branch/commit/push + ReviewRequest（GitBranchOnly），自动 PR 后置（REQ-COD-06）。

## 6. 失败处理与状态机（C8）

- 成员登记：discovered → validated → registered → indexed → ready/failed（幂等重试，取消/补偿）。
- 聚合索引：building → active → stale/degraded → superseded；重建失败保留 last-known-good。
- 单仓执行：worktree-created → coding → committed → pushed → ReviewRequest-created（幂等重试，人工恢复入口）。
- 无跨仓原子事务；partial failure 显式呈现，不以部分交付伪装全局完成。

## 7. 分期与验收（C9/C10）

- **P0（preview/experimental）**：WP0（身份/manifest/迁移）、WP1（批量登记）、WP2（聚合初始化 + 既有 capability MODIFIED）、WP3（聚合索引生命周期）。
- **P1（experimental + supervised）**：WP4（聚合规划）、WP5（单仓执行桥 + envelope 路由 + 指针）、WP6（贯穿质量泳道）。
- **量化阈值（PoC 阶段定值，先定边界）**：prompt inventory/evidence 的 token/byte 预算；索引刷新周期与 stale threshold；50 仓索引建立/刷新时间预算；sandbox 越界矩阵；provider 版本矩阵；这些以「P0/P1 交付前经 2–3 仓与 50 仓实测确定并写入 Plan」为契约，不得由 Plan 发明新验收。
- 每个 WP 标注 P0/P1、feature flag、依赖、估算、回滚点（写入 Plan 前由本契约约束）。

## 8. YAGNI（推迟）

- codegraph DB merge / federation / SQLite ATTACH；Sourcegraph / scip-java 部署。
- 自动依赖图/服务拓扑（D9；未来演进参照 Nx Polygraph）。
- 远程代码托管平台导入；动态 clone/fetch（首版本地已 clone）；自动创建远端 PR/MR。
- 完整多仓 coordinator、跨仓原子 commit/PR/回滚状态机、mixed-target 自动拆分。
- Codex exec adapter；通用动态 provider 评分器；完整第三方 policy engine；OS 级沙箱（后置）。
- session 内政策热更新；完整规则治理平台。
