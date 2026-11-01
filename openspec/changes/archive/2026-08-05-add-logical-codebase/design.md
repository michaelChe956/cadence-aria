# 设计：逻辑代码库多仓库支持

> 依据：`cadence/designs/2026-08-05_方案设计_逻辑代码库多仓库支持_v1.0.md`（v1.6，用户确认 + 三方评审收敛）；经 OpenSpec proposal 三司会审（researcher/worker/reviewer）修订；v1.7：合并 P0/P1 全量 experimental+supervised 发布（用户决策，消解分期冲突）、身份域闭合、envelope 多入口覆盖、验收阈值量化。

## 1. 核心架构边界

- **Project = 逻辑代码库作用域**（产品不变量 D1）。不新增顶层 `LogicalCodebaseRecord`；`ProjectRecord` 承载逻辑代码库语义。
- **公共父目录假设（D3）**：成员仓库均位于同一非 git 公共父目录下；父目录作聚合 CodeGraph 索引根与规划 provider 驱动目录。聚合资产由 Cadence `.aria` 领域存储管理。因果链：非 git 父目录 → 无 super-repo `.gitignore` → 子仓源码非 gitignored → codegraph 按目录递归扫描并索引（**非按 git 边界识别成员**，成员边界由 Cadence 据 manifest 生成 denylist 并验证，见 §4.2/spike 2）。
- **读聚合、写单仓（D2）**：规划类产物使用聚合上下文（全集可检索，不注入全部源码）；每个 Work Item / Coding attempt 有且仅有一个 `target_repository_id`；主 checkout 不被直接修改。
- **聚合根绝不注册为 RepositoryRecord**；`RepositoryRecord` 始终代表一个物理 git 仓。
- **身份三分（闭合定义）**：
  - `LogicalRepositoryId`（稳定 UUID，逻辑身份，selection/involved_refs/target 的语义类型）
  - `RepositoryCheckoutId`（可用 checkout 实例，执行时解析为可写物理 checkout）
  - `RepositoryRecord.id`（现有物理仓兼容投影，迁移期权威，逻辑代码库稳定后为只读兼容字段）
  - 映射链：`LogicalRepositoryId` →（member 解析）→ `RepositoryCheckoutId` →（物理定位）→ `RepositoryRecord.id` + canonical_path + git_dir_identity
  - `target_repository_id` 字段的语义类型为 `LogicalRepositoryId`（不可歧义）；attempt 冻结快照含三层完整映射（logical/checkout/physical + revision + policy_digest）
  - `repo_hash` 仅表达 checkout path，不作逻辑身份或去重凭据
- **权威数据源（C3）**：`.aria/projects/{id}/logical-codebase/manifest.json` 为唯一领域权威；`catalog/repos.yaml` 为可读投影/导入输入；根 CLAUDE.md/AGENTS.md/MCP 配置为 revisioned projection。

## 2. 关键决策与权衡

| 决策 | 选择 | 权衡 |
|---|---|---|
| D3 父目录 | 非 git 目录 | 规避外层 git 语义陷阱与 includeIgnored 依赖；聚合资产由 `.aria` 管理。**CodeGraph exact version 钉定 v1.5.0**（spike 2 实测：#1295 在 v1.5.0 已修复；CLI 无 allowlist，范围控制靠 `codegraph.json` exclude denylist） |
| D8 worktree | 每仓 `.worktrees/aria-issues/{issue}`，主 checkout 不动 | **由 Cadence 显式 `git worktree add` + cwd 钉死**，不依赖 provider 自带 worktree 沙箱（Claude #64315/#42034/#44220、Codex #14338/#15505 为已知坑）；仅 shared worktree 键升级为 `(project, issue, repository)` |
| D9 依赖图 | 不做服务拓扑建模 | 规划由 provider 从聚合索引按需检索（**启发式符号/字符串近似，非 Java 语义级**）；未来可演进 Nx Polygraph 式依赖图 |
| D12 沙箱 | coding = experimental + supervised（用户确认，全量发布） | 先做「配置目标 + 不暴露兄弟仓」行为级隔离（两级状态：`best_effort_configured` / `production_verified_readonly`）；OS 级沙箱后置；Claude plan 模式仅为 prompt 层，硬只读需 hook 或 Codex read-only |
| D13 索引刷新 | 按需 sync + 定时兜底 | 快照由 Cadence 用 `git rev-parse HEAD` 采集（CodeGraph 无 per-member revision API）；索引反映主 checkout 快照状态，worktree 不入索引 |
| D14 指针 | 随 coding 上线，独立 worktree/branch 受控发布 + ReviewRequest | 登记阶段保持零副作用；指针写入是受控副作用，非「零副作用」 |
| 初始化 | 机器一次 + 聚合根一次（5.5） | provider 必要内容集中到聚合根；每仓只放最小指针；GitFinalize 逐仓提交仅保留给传统单仓路径 |
| Codex | 保持 app-server；当前 unsupported，改造后 experimental | `danger-full-access` 必须改为受限写配置（首选稳定路径 `sandbox_mode = "workspace-write"`，Beta permission profile 列为后续演进）+ 版本钉定 + 越界测试；权限 profiles 官方标 Beta，列为已知风险 |
| 交付 | 推分支 + ReviewRequest | 当前代码只有 GitBranchOnly 交付，无外部 PR 集成；自动 PR 不在本 change 范围（用户确认） |

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
- **5 个稳定 step ID（spike 3 定案）**：`machine_skills`（CadenceSkills 准备，机器级，非 provider turn）/ `aggregate_preflight`（确定性 Cadence 代码，preflight + member snapshot，非 provider turn）/ `pre_check`（1 Claude turn，聚合根）/ `rule_and_mcp_config`（1 Claude turn，聚合根，合并旧 `rule_config`+`mcp_configuration`）/ `openspec_and_examples`（1 Claude turn，聚合根）。step ID 是持久化协议，不是显示文案。
- **GitFinalize 切割点（spike 3）**：聚合模式从 coordinator 调用图切断 GitFinalize，不映射为聚合第六步；聚合初始化不对成员仓执行任何 git add/commit/push，仅写 Aria-owned 聚合资产。
- 聚合根准入 preflight（REQ-REG-09）：canonical 非 git、仅含 manifest 成员、排除 `.worktrees/**`/`.git/**`/`.aria/**`/构建产物/凭据。
- 每仓最小指针发布随 coding 上线，经独立 worktree/branch + ReviewRequest，处理已有 CLAUDE.md/AGENTS.md 合并冲突，可回滚。

### 4.2 聚合 CodeGraph 索引（C1/C2 / logical-codebase-aggregate-index）

- 在非 git 聚合根 `codegraph init` 一次；主路径无需 includeIgnored（D3）。**spike 2 实测结论**：CodeGraph 1.5.0 按目录递归扫描，不按 git 边界识别成员（非 git 目录也会被纳入），CLI 无 allowlist 参数；范围控制必须由 Cadence 生成聚合根 `codegraph.json` 的 `exclude`（denylist），显式排除非成员目录 + `**/.worktrees/` + `**/.aria/` + 构建产物；CodeGraph 内建排除 `.git`/`build`/`node_modules`/`dist` 但不内建排除 `.worktrees`/`.aria`（删子仓 `.gitignore` 后会被误索引）。**CodeGraph exact version 钉定 v1.5.0**。
- 索引范围由 Cadence 维护 `codegraph.json` exclude（denylist）；执行 worktree 不入索引；成员增删后原子更新该文件并 sync；index 后用 `codegraph files` 与负查询验证边界。
- freshness：快照由 Cadence 采集（`git rev-parse HEAD` + dirty）；按需 sync + 定时兜底；stale/degraded 由 Cadence 对比快照推断并显式呈现；single-writer 锁 + last-known-good。

## 5. 安全与 provider（experimental + supervised）

### 5.1 SessionPolicyEnvelope（覆盖全部逻辑代码库 provider 入口）

- 两层：持久化 `AggregatePolicyArtifact` + 不可变 `SessionPolicyEnvelope`。
- **适用范围（闭合：现有代码的实际入口不止两条栈，envelope 必须覆盖全部）**：逻辑代码库流程的以下真实 provider 启动入口，全部须经统一 `LogicalCodebaseProviderGateway` 构造 `ValidatedSessionLaunchPolicy` 后启动：
  - 同步栈：`ProviderAdapter::run(AdapterInput)`（work-item split 引擎 `engine.rs:131-145` 直接调用）
  - 流式栈：`StreamingProviderAdapter::start(StreamingProviderInput)`（聚合初始化、聚合规划 provider_drive `provider_drive.rs:91-119`、coding provider_stream、review）
  - **gateway 是逻辑代码库 provider 的唯一建造入口**；同步/流式 adapter 前均完成 policy/capability/target/canonical cwd/git-dir/config digest/resume fingerprint 复验
  - 真实 provider 的 legacy fallback（`run_streaming` 默认 bridge、coding retry `allow_legacy_stream_fallback: true`）在逻辑代码库调用中固定为 false；裸 `StreamingProviderInput`/`AdapterInput` 不得直接构造启动
  - Fake/测试路径经 registry 分层或编译期构造限制隔离，不依赖运行时 `if provider != Fake`
- 三落点：产品层 envelope、adapter 输入 `ValidatedSessionLaunchPolicy`（不可缺省）、prompt 可见性层。
- 硬门（最小闭环先做，其余后置）：① 只读/单写绑定可执行边界（配置目标 + pre/post 检测）；⑤ resume 时 policy/target/版本/capability 一致才允许；⑦ 禁止无政策 fallback、禁止降级 full-access。
- 配置来源隔离：Aria-owned settings/MCP bundle；审计 user/project/local/env/子仓 MCP 合并优先级；managed-settings 优先级高于注入时列为已知 gap，并在 run 审计显式标注（检测 `/status` Setting sources 是否含 managed settings）。

### 5.2 Provider 路由（路由级 fail-closed）

- 规划：Claude 只读模式（best-effort 只读）；目标 coding：Claude single-target 配置（experimental）；Codex：显式 experimental opt-in。**Codex 当前硬编码 `danger-full-access`（`codex_provider/mod.rs:31`/`session.rs:77,101`），必须经 gateway 路由级阻断，不能仅 UI 隐藏选项**；在完成受限写配置（首选稳定路径 `sandbox_mode = "workspace-write"`，Beta permission profile 列为后续演进）+ 版本钉定 + 越界测试前保持 unsupported。
- capability 记录 exact version / adapter dialect / evidence 状态（`declared` / `fixture_verified` / `production_verified`，三态持久化并参与路由判定，`declared` 级路由标注未验证）；能力不满足 → 阻塞，不降级。
- 越界矩阵 fixture 须纳入已知上游旁路：Claude plan 模式 bypass 可用（#41758）、PreToolUse deny 被忽略（#37210/#43407）；Codex `apply_patch` 绕过 writable_roots（#24214，针对 exec 路径，app-server 需独立验证）。
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

## 7. 发布形态与验收（C9/C10）

### 7.1 单一全量发布（用户决策：合并 P0/P1，不再分期）

- 本 change 全量按 **experimental + supervised** 发布，不再分 P0/P1 两个 preview/experimental 阶段。该决策消解了「WP2 聚合初始化在前期需真实 provider，而 envelope gate 在后期」的分期冲突——envelope（WP5e）与聚合初始化（WP2）在同一发布内按依赖顺序排布，envelope 是整个逻辑代码库流程从首次启动起就生效的门。
- WP 顺序按依赖图（非分期）。**关键时序约束（spike 3 + 二审 worker F-4 修正）**：WP2 的 `pre_check`/`rule_and_mcp_config`/`openspec_and_examples` 三个 step 会启动真实 provider，SHALL 经 `LogicalCodebaseProviderGateway` 启动；故 WP5e（gateway + envelope）SHALL 在 WP2 的 provider step 可运行前就绪。WP0 SHALL 生成/登记 bootstrap `AggregatePolicyArtifact`（envelope 解析的事实来源），消除「WP2 provider 先需 envelope、envelope 又先需 policy artifact」的 bootstrap 环。WP2 的 `machine_skills`/`aggregate_preflight`（非 provider turn 的确定性预检）可在 WP5e 就绪前先行。
- 依赖图：`WP0（含 bootstrap policy artifact）→ WP1/WP3 并行 + WP5e`；`WP5e → WP2（provider step 被 gateway feature gate）`；`WP0 + WP3 → WP4`；`WP0 + WP4 → WP5a`；`WP5a + WP0 → WP5b`；`WP0 + WP3 + WP4 → WP5d`；`WP5a + WP5b + WP5e → WP5c`；`WP0 + WP5b + WP5c + WP5e → WP5f`；WP6 贯穿质量泳道（每个 WP 同步增设测试与回滚项，非末尾集中验证）。
- feature flag 贯穿全程：逻辑代码库能力在 flag 关闭时整体不可用，回退单仓行为（REQ-REG-08）。

### 7.2 Spike 前置（已完成，结论回写本契约）

以下 spike 已于 2026-08-08 完成，报告见 `cadence/spikes/2026-08-05-spike-{1,2,3,4}-*.md`，关键结论已回写本设计：

1. **身份域与迁移 journal 设计（spike 1）**：三层身份 newtype（`LogicalRepositoryId`/`RepositoryCheckoutId`/`RepositoryRecord.id`）、`RepositorySourceIdentity` + tombstone ledger、旧 JSON `#[serde(default)]` 兼容、可重放迁移阶段（扫描→映射→写 manifest/member/tombstone→回填兼容投影→双读双写→读路径切换→删旧 fallback）。**实测冲突点**：`CodingExecutionAttempt` 无 target 快照且手写 `Deserialize`（`coding_models/execution.rs:90-184`），新字段须同时改公开 struct + 私有 `CodingExecutionAttemptSerde` + 重建逻辑，不能只加 `#[serde(default)]`。**新发现**：git 不提供永久 source UUID，目录移动/重复 clone 的身份 adoption 须显式处理（`reactivate_tombstoned_source`）。详见 spike 1 报告。
2. **CodeGraph 2–3 仓 spike（spike 2）**：v1.5.0 实测。**颠覆性结论**：CodeGraph 按目录递归扫描，**不按 git 边界识别成员**（非 git 目录 `not-a-repo` 被误索引）；CLI 无 allowlist 参数；`.worktrees`/`.aria` 非内建排除（删子仓 `.gitignore` 后被误索引）。D3 「非 git 父目录建统一索引」技术能力成立（跨子仓 callers/callees 命中），但范围控制必须由 Cadence 维护 `codegraph.json` exclude denylist。exact version 钉定 v1.5.0。详见 spike 2 报告。
3. **聚合初始化稳定 step ID（spike 3）**：5 个 step 定案（`machine_skills`/`aggregate_preflight`/`pre_check`/`rule_and_mcp_config`/`openspec_and_examples`）；`rule_config`+`mcp_configuration` 合并为单逻辑 step；GitFinalize 从 coordinator 调用图切断；`RepositoryInitializationRunRegistry` 泛化为 `operation_kind + project_id + operation_id`。详见 spike 3 报告。
4. **50 合成样本与预算阈值（spike 4）**：50 仓 fixture（1409 文件）实测，阈值见 §7.3。详见 spike 4 报告。

### 7.3 验收阈值（spike 4 实测量化，可判定，不得由 Plan 发明新验收）

基于 spike 4（50 合成仓、1409 文件、CodeGraph 1.5.0、Arch Linux/i5-13500H/46GiB RAM，非网络盘）实测：

- **索引性能**：50 仓首次索引目标 ≤ 10s（实测 1.36s），软告警 > 30s，硬失败/转异步 > 120s；3–5 文件增量 sync 目标 ≤ 3s（实测 0.42s），软告警 > 10s，硬失败 > 30s；sync 失败保留 stale 标记，不报「已更新」。
- **内存防护**：预算 512 MiB（实测 `ru_maxrss` ~413 MiB），告警 ≥ 768 MiB；CI/目标机用 GNU time 重测校准。
- **DB 容量**：50 仓基线 10 MiB（实测 9.2–9.8 MiB），告警 > 25 MiB；真实仓按「可索引源码数/图节点数」记新基线，不按仓数机械限额。
- **inventory 注入预算**：默认最多 4 KiB / ~1,400 token 紧凑清单，上限 8 KiB / ~2,700 token；超出只注入目标成员 + 计数/摘要并按需查询全量（实测 50 项详细 JSON 9857B / ~3,286 token，不能无截断每轮塞全量）。
- **stale 检测周期**：30s 轮询兜底；文件变更/成员表变化即时标记 stale；活跃写入 2s 静默去抖。不依赖 codegraph watcher 作唯一正确性保障。
- **provider capability matrix**：Claude/Codex exact version + adapter dialect + evidence 三态（`declared`/`fixture_verified`/`production_verified`），未达 `production_verified` 不得宣称物理只读。
- **越界矩阵 fixture**：Claude plan bypass（#41758）、PreToolUse deny 忽略（#37210/#43407）、Codex apply_patch 绕过（#24214）、shell/apply_patch/绝对路径/父目录/symlink/resume 越界。

### 7.4 测试分层

- L1 deterministic unit：临时 git repo、fake streaming adapter、bounded command runner、fixture provider process、文件快照。
- L2 integration：2–3 仓端到端（登记→初始化→索引→规划→coding→推分支）。
- L3 CLI contract fixture：CodeGraph 真实 CLI（spike 2 输出）。
- L4 production verification（opt-in 隔离）：真实 Claude/Codex 账号调用、真实 50 仓性能；不作为普通单测前提，证据记为 `production_verified`。

## 8. YAGNI（推迟）

- codegraph DB merge / federation / SQLite ATTACH；Sourcegraph / scip-java 部署。
- 自动依赖图/服务拓扑（D9；未来演进参照 Nx Polygraph）。
- 远程代码托管平台导入；动态 clone/fetch（首版本地已 clone）；自动创建远端 PR/MR。
- 完整多仓 coordinator、跨仓原子 commit/PR/回滚状态机、mixed-target 自动拆分。
- Codex exec adapter；通用动态 provider 评分器；完整第三方 policy engine；OS 级沙箱（后置）。
- session 内政策热更新；完整规则治理平台。
