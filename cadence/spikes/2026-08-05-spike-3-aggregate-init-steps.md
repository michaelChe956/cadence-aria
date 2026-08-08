# Spike 3：聚合初始化稳定 step ID 设计

- Change：`2026-08-05-add-logical-codebase`
- 日期：2026-08-05
- 性质：实施前设计 spike；本文只产生设计草案，没有修改产品代码或 OpenSpec。

## 1. 基线结论与设计边界

当前单仓初始化并不是可直接“改成五步”的通用 operation：它有已经对外暴露的固定 6-step JSON/API 契约和强状态机不变量。

| 基线事实 | 证据 | 结论 |
|---|---|---|
| 固定 6 个枚举值 | `RepositoryInitializationStepKind::ALL = [CadenceSkills, PreCheck, RuleConfig, McpConfiguration, ProjectRulesExamples, GitFinalize]`，`src/product/repository_store/types.rs:80-120`。 | 聚合 operation 必须使用新 enum，禁止扩展或重排旧 `ALL`。 |
| 仅 4 个 Claude turn | initializer 过滤 `step.command()`；CadenceSkills/GitFinalize 返回 `None`，另四个命令逐个启动 Claude session（`initializer.rs:36-98`）。 | 新的“是否 provider turn”应由 aggregate step 定义，而不是复用 `command_index`。 |
| 旧 operation 强制长度/顺序 | `validate_initial_operation` 要 `steps.len()==6`；`has_supported_step_layout` 只接受完整 6 或 legacy 5；`step_index` 从旧 `ALL` 取 index；所有前步必须 completed（`operation.rs:377-644`）。 | 在旧 struct 上添加聚合步骤会让已持久化 JSON 不可读或违反运行不变量。 |
| 旧收尾的持久化时点特殊 | Repository 在 `GitFinalize` 之前已写入 `repos.json`，然后旧 coordinator 在仓根 `git add -A/commit/push`；checkpoint 允许收尾失败仍把 operation 标为 completed+warning（`registration.rs:518-575`, `git_finalize.rs`）。 | 聚合模式不能将旧 GitFinalize 误映射为聚合第六步；要从 coordinator 调用图上切断。 |
| registry 只按字符串 operation ID 去重 | `RepositoryInitializationRunRegistry` 保存 `HashSet<String>`；HTTP worker 创建和 GET recovery 都只传 `operation_id`（`repository_initialization_run_registry.rs`, `web/handlers/repository_registration.rs:261-337`）。 | 新旧 operation ID 若碰撞会错误地互相阻塞/误判中断；registry 必须加 operation kind。 |
| profile 不是技术探测器 | `RepositoryProfile` 由规划 provider 产出 languages/frameworks/package_managers 等；产品中尚未发现 `package.json`、pnpm 或 Vite 的静态 repo type 探测（`models/verification.rs`）。 | “frontend profile”必须加入确定性 preflight 探测，不能假设 Java 六步能推导。 |

### 1.1 本 spike 的定位

- 新增 `AggregateInitializationOperation`：为 Project（逻辑代码库）聚合根初始化，不注册聚合根为 `RepositoryRecord`，不进入成员仓执行 `git add/commit/push`。
- 保持 `RepositoryInitializationOperation`、`RepositoryInitializationOperationDto`、单仓注册路由与其持久化 JSON **字节级兼容**：既有文件反序列化、枚举字符串、steps 数组顺序、API DTO 都不变化。
- aggregate 操作只在 logical-codebase feature flag 开启且 manifest 已建立时可启动；它不替代普通单仓注册的初始化。
- “machine”指开发者本机的 Cadence skills 安装/链接根，不是任一个 member checkout。

## 2. 五个稳定 step ID

### 2.1 enum、记录和稳定性规则

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateInitializationStepKind {
    MachineSkills,
    AggregatePreflight,
    PreCheck,
    RuleAndMcpConfig,
    OpenspecAndExamples,
}

impl AggregateInitializationStepKind {
    /// 此列表、顺序与 JSON 名称在 v1 后冻结；新增步骤须引入 layout_version=2，不能插入这里。
    pub const V1: [Self; 5] = [
        Self::MachineSkills,
        Self::AggregatePreflight,
        Self::PreCheck,
        Self::RuleAndMcpConfig,
        Self::OpenspecAndExamples,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MachineSkills => "machine_skills",
            Self::AggregatePreflight => "aggregate_preflight",
            Self::PreCheck => "pre_check",
            Self::RuleAndMcpConfig => "rule_and_mcp_config",
            Self::OpenspecAndExamples => "openspec_and_examples",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateInitializationOperationStatus {
    Created,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateInitializationStepStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateInitializationStepRecord {
    pub step_id: AggregateInitializationStepKind,
    pub status: AggregateInitializationStepStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}
```

稳定 step ID 是持久化协议，不是显示文案，也不是 Claude command index。UI 使用 `step_id` 做 i18n 映射；实现不得以中文 label、枚举 debug name 或 provider prompt 文本作为存储 ID。重试复用同一 operation 的同一个 step key；显式“重新初始化”创建新的 operation ID，不能将 completed step 改回 pending。

### 2.2 每步定义

| Step ID | 职责 / 允许副作用 | Claude provider turn | cwd | 输入 → 输出 | 幂等键 |
|---|---|---|---|---|---|
| `machine_skills` | 调用现有 `CadenceSkillsManager::prepare`，准备/更新用户级 Cadence skills 与 managed link。它不向任何 member 或 aggregate root 写规则文件。 | 否。现有 prepare 走 Git/文件链接，不经 `StreamingProviderAdapter`。 | 不适用（manager 使用其受控 source/home；不能假装 aggregate cwd）。 | 输入：skills source version、home identity、feature config。输出：`CadenceSkillsPreparationSummary`、skills digest/paths/warnings。 | `aggregate-init:{project}:{operation}:machine_skills:{skills_input_digest}`；重放先比较已保存 source/link digest。 |
| `aggregate_preflight` | 确认 manifest、公共非 git 聚合根、成员 allowlist、成员 canonical path/git root、无 worktree/.git/.aria/构建/secret 索引越界；采集 member revision/dirty/availability snapshot；创建 Aria-owned aggregate 工作区骨架。 | 否。必须是确定性 Cadence 代码，禁止 provider 依据 prompt 决定 allowlist。 | 聚合根用于路径/非 git 检查；所有 Git 命令 cwd 钉到逐个 member main checkout。 | 输入：manifest revision、provider_context_root、member/checkout records、排除规则版本。输出：immutable `preflight.json`，含 `membership_revision`、每 member checkout/revision/availability、aggregate asset paths、diagnostics。 | `aggregate-init:{project}:{operation}:aggregate_preflight:{manifest_digest}:{member_revisions_digest}`。 |
| `pre_check` | 在聚合根执行一次 provider 的受控 readiness/pre-check；只能写 Aria-owned aggregate assets，不能对 members 执行初始化或 Git 操作。 | 是，**1 turn**，Claude Code executor。prompt 为 `/pre-check --no-interrupt --upgrade 用大陆镜像` 的聚合专用封装；须通过 logical provider gateway/policy envelope。 | 聚合根。 | 输入：preflight ref、policy digest、provider capability/version、machine skills digest。输出：sanitized provider transcript/command summary、managed pre-check projection digest。 | `aggregate-init:{operation}:pre_check:{preflight_digest}:{policy_digest}:{provider_capability_digest}`。 |
| `rule_and_mcp_config` | 合并旧 `rule_config` 与 `mcp_configuration` 为一个原子**逻辑 step**：在聚合 assets 生成/更新 root rule projection 与 Aria-owned MCP bundle，并记录 managed source/digest。禁止直接修改各 member 的 CLAUDE/AGENTS/MCP。 | 是，**1 turn**。provider 可执行组合 prompt（如先 `/rule-config --no-interrupt` 再 `/mcp-configuration --no-interrupt`），但只产生一个 step record；若中途失败，runner 在 step 内用 staging dir 回滚/重放。 | 聚合根。 | 输入：preflight ref、pre-check output、policy artifact、现有 aggregate managed files digest。输出：`rule-mcp-config.json`（两个子 action 摘要、文件清单、combined digest）。 | `aggregate-init:{operation}:rule_and_mcp_config:{precheck_digest}:{policy_digest}:{managed_projection_digest}`。 |
| `openspec_and_examples` | 合并旧 `project_rules_examples` 的规则示例生成与聚合 OpenSpec 指针/模板安装；只发布 aggregate root 的 Aria-managed projection。member 最小指针属于 coding 上线后的独立 worktree/ReviewRequest workflow，不在此步骤。 | 是，**1 turn**。 | 聚合根。 | 输入：rule+mcp output、manifest/preflight、policy digest、OpenSpec template/skill versions。输出：`openspec-examples.json`、managed files/digest、指针发布待办（而非成员文件变更）。 | `aggregate-init:{operation}:openspec_and_examples:{rule_mcp_digest}:{template_digest}:{policy_digest}`。 |

**本期固定结论：** 每个有 provider 的 step 各一个 session/turn，因此 aggregate v1 共 3 个 Claude turns（`pre_check`、`rule_and_mcp_config`、`openspec_and_examples`），不是旧单仓 4 个。Machine skills 和 preflight 都是 Cadence 自有确定性步骤，必须在 provider 之前失败关闭。

### 2.3 `rule_and_mcp_config` 内部原子性

合并不等于失去诊断。该 step 的 `output_ref` 指向一个拥有 `rule_config` 与 `mcp_configuration` 子 action 的 result：

```json
{
  "step_id": "rule_and_mcp_config",
  "rule_config": {"status": "completed", "changed_paths": [".aria/aggregate/CLAUDE.md"]},
  "mcp_configuration": {"status": "completed", "changed_paths": [".aria/aggregate/mcp.json"]},
  "managed_projection_digest": "sha256:...",
  "staging_dir_cleaned": true
}
```

coordinator 必须：在 `aggregate/.aria-staging/{operation}/rule-and-mcp/` 建准备目录 → 运行组合 provider turn → 校验只写允许 staging 范围 → 原子 rename/replace managed projection → 写 output → 标 completed。provider failed、cancel 或输出越界时删除 staging directory；已 publish 版本由 digest 幂等识别。不得把旧的两个 step record 或旧 `command_index=2/3` 泄漏为 aggregate API 约定。

## 3. 与现有六步的映射及 GitFinalize 切割

### 3.1 映射矩阵

| 单仓固定六步 | 聚合处理 | 理由 |
|---|---|---|
| `cadence_skills` | → `machine_skills` | 复用 `CadenceSkillsPreparation`/`CadenceSkillsManager::prepare`；机器资源同一 operation 只做一次。 |
| `pre_check` | → `pre_check` | 语义保留，但 cwd 从 git root 改为受控 non-git aggregate root，且受 policy envelope 审核。 |
| `rule_config` | → `rule_and_mcp_config` 的子 action | 文件归属上移至 aggregate managed projection。 |
| `mcp_configuration` | → `rule_and_mcp_config` 的子 action | 同一 provider turn/staging transaction，与规则配置一起校验和发布。 |
| `project_rules_examples` | → `openspec_and_examples` 的一部分 | 保留项目规则示例；补入聚合 OpenSpec 初始化/指针模板。 |
| `git_finalize` | **不映射到 aggregate step** | 旧语义是仓根 `git add -A`、commit、可选 push；aggregate root 保证非 git，且绝不能借此进入每个 member。 |
| （无旧对应） | `aggregate_preflight` | 新增的安全门，必须在任何 provider turn 之前建立成员/根/排除快照。 |

### 3.2 coordinator 的明确切割点

当前 `RepositoryRegistrationCoordinator::execute_initialization` 同时管理准备、Claude initializer、`repositories.create_repository` 与 `git_finalize`；因此绝不能通过给 `ClaudeRepositoryInitializer` 增加 flag 后继续调用旧 coordinator。切割应位于 **web handler 之下、repository registration coordinator 之旁**：

```text
POST aggregate initialization
  -> AggregateInitializationCoordinator::begin(...)
  -> AggregateInitializationCoordinator::execute(...)
       -> AggregateInitializationOperationStore
       -> CadenceSkillsPreparation (machine_skills)
       -> AggregatePreflightService (aggregate_preflight)
       -> AggregateInitializer / LogicalCodebaseProviderGateway (3 provider steps)
       -> AggregateAssetPublisher (only .aria aggregate assets)

POST single repository registration (unchanged)
  -> RepositoryRegistrationCoordinator::begin_initialization(...)
  -> RepositoryRegistrationCoordinator::execute_initialization(...)
       -> ClaudeRepositoryInitializer (old 4 turns)
       -> RepositoryStore::create_repository(...)
       -> git_finalize(git_root)  // old behavior only
```

`AggregateInitializationCoordinator` 的依赖 trait 草案：

```rust
pub trait AggregatePreflightService: Send + Sync {
    fn inspect(
        &self,
        project_id: &str,
        manifest: &ProjectCodebaseManifest,
        cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError>;
}

#[async_trait]
pub trait AggregateInitializer: Send + Sync {
    async fn run_step(
        &self,
        step: AggregateInitializationStepKind,
        input: AggregateStepInput,
        progress: Arc<dyn AggregateInitializationProgress>,
        cancellation: CancellationToken,
    ) -> Result<AggregateStepOutput, AggregateInitializationError>;
}

pub struct AggregateInitializationCoordinator {
    operations: AggregateInitializationOperationStore,
    manifests: Arc<dyn LogicalCodebaseManifestStore>,
    skills: Arc<dyn CadenceSkillsPreparation>,
    preflight: Arc<dyn AggregatePreflightService>,
    initializer: Arc<dyn AggregateInitializer>,
    publisher: Arc<dyn AggregateAssetPublisher>,
    // 没有 RepositoryPersistence::create_repository；没有 git_finalize runner。
}
```

这一层级的负向测试应断言 aggregate coordinator 的依赖图中不存在 `RepositoryStore::create_repository` 和 `RepositoryRegistrationCoordinator::git_finalize` 调用，并用 fake bounded command runner 验证未向任何 member cwd 发出 `git add`/`git commit`/`git push`。

### 3.3 member 最小指针的边界

设计文档 §4.1 已规定 member 最小指针随 coding 上线、经独立 worktree/branch + ReviewRequest 发布。aggregate init 的 `openspec_and_examples` 只能产出 `member_pointer_plan`（成员、目标文件、base revision、预计内容 digest），不可直接写 member checkout。这样既不污染当前注册时“可能 GitFinalize”的行为，也不会在公共非 git 根制造一个伪 super-repo commit。

## 4. `AggregateInitializationOperation` 状态机

### 4.1 持久化 DTO/record 草案

建议路径：

```text
.aria/projects/{project_id}/logical-codebase/
├── aggregate-initializations/{operation_id}.json
├── aggregate-initializations/{operation_id}/outputs/{step_id}.json
└── aggregate/.aria-staging/{operation_id}/...     # 运行时残留，受 cleanup 管理
```

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateInitializationOperationInput {
    pub project_id: String,
    pub expected_manifest_revision: u64,
    pub requested_by: String,
    pub policy_digest: String,
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateMemberInitializationProjection {
    pub logical_repository_id: LogicalRepositoryId,
    pub primary_checkout_id: RepositoryCheckoutId,
    pub status: AggregateMemberInitializationStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preflight_ref: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub updated_at: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateMemberInitializationStatus {
    Pending,
    Validated,
    Ready,
    Blocked,
    Removed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AggregateInitializationOperation {
    pub operation_id: String,               // aggregate_initialization_<uuid>
    pub operation_kind: String,             // fixed "aggregate_initialization"
    pub layout_version: u16,                // fixed 1
    pub input: AggregateInitializationOperationInput,
    pub status: AggregateInitializationOperationStatus,
    pub steps: Vec<AggregateInitializationStepRecord>, // exact V1 order
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_step: Option<AggregateInitializationStepKind>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failed_step: Option<AggregateInitializationStepKind>,
    #[serde(default)]
    pub member_projections: Vec<AggregateMemberInitializationProjection>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancellation: Option<AggregateCancellationRecord>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<AggregateInitializationErrorRecord>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}
```

新 operation 接受 `Cancelled`，因为 aggregate 的 staging/lock/managed projection 残留需要可辨识终态；不能挪用旧 enum（旧 enum 没有 cancelled）。所有 top-level option/vec 使用 serde default 只服务于新 operation 的未来字段演进；`layout_version`、project、operation ID、steps 及 input 在 v1 读取时必填并校验。

### 4.2 严格顺序与状态转移

**初始不变量：** `Created` 有正好五个 step，顺序严格等于 `V1`，全为 pending，`current_step/failed_step/error/cancellation/completed_at` 全空；member projections 为 manifest 成员，均 pending。

**开始：** `Created -> Running` 仅能发生一次；coordinator 用 input `idempotency_key` 创建时去重：同 project/key/input digest 返回同 operation，不同 digest 返回 conflict。

**推进：**

```text
pending(i) --mark_step_running--> running(i)
  前提：operation=running；i 前全部 completed；i 后全部 pending；current_step=None
running(i) --mark_step_completed--> completed(i)
  前提：current_step=i；output digest/ref 已持久化；对 publish step staging 已清理
completed(i) -> pending(i+1) 可启动
completed(last) -> operation completed
```

- `mark_step_running` 对已 `running(i)` 的相同 step/idempotency key 返回已记录 operation（network retry no-op）；对已 `completed(i)` 返回 completed result；任何跳步、回退或并行 running 都报 `IdentityMismatch`/`invalid_transition`。
- `aggregate_preflight` 完成时将 member projection 从 pending 置 validated（或任何成员问题则 step failed，问题成员 blocked，其他仍 pending）；后三个聚合 asset steps 成功后，所有 validated member 才可置 ready。
- operation 完成不表示“每个 member 文件已被初始化”；它表示聚合资产已 ready，member pointer plan 已准备。`member_pointer_plan` 发布的状态属于其后独立 workflow。

### 4.3 失败、取消和恢复

**失败：** provider 或确定性动作失败时，当前 `running` step 写 failed（有 started/completed timestamp），operation 写 `Failed` 和 structured error；后续 steps 保持 pending，未运行。retry 的语义是 `retry_failed_step`：创建新 operation（审计更清楚）或通过显式 `resume_operation` 将失败 step 重置 pending，二者只能选一。建议 v1 使用**新 operation**，旧 operation immutable terminal；其 input 包含 `supersedes_operation_id`。这是最简的审计/幂等模型。

**取消：** HTTP cancel endpoint 在 registry 找到 operation cancellation token，首先将 persistent `cancellation.requested_at/reason` 写入，再 signal token。worker 收到后：

1. provider turn：向 session 发送 Abort；等待有限时间，不把取消视为 completed；
2. staging：删除 operation-owned staging path，失败写 `cleanup_pending_paths`；
3. locks：RAII drop + 持久化锁 owner 清理；
4. managed publisher：只有未 atomic publish 的 staging 可删；已 publish 的 digest 完整 projection 留存，不能“猜测回滚”覆盖此前版本；
5. running step 标 `Cancelled`，operation 标 `Cancelled`，后续 pending 保持 pending；成员若已 validated 留 validated，若无安全 preflight 留 pending，绝不标 ready。

**进程中断恢复：** GET/read 时，若 operation 为 Created/Running 而 keyed registry 无 active lease，`recover_interrupted_operation` 读取 `current_step`：

- 先检查该 step 的 output/ref/publisher receipt 与 idempotency key，完整且 digest 匹配则安全地 mark completed 并继续由显式 resume 启动；
- 否则清理 operation staging，当前 step 标 failed，operation `Failed`，reason `aggregate_initialization_interrupted`；
- 不自动重启 provider；重新执行需用户/API 明确 resume/new request。

这借鉴当前 repository operation 的 recovery 思路，但不能照抄其 GitFinalize special case：aggregate 没有“repository already persisted yet finalization failed”这一合法 completed-with-warning 状态。

### 4.4 operation store 接口草案

```rust
pub struct AggregateInitializationOperationStore { paths: ProductAppPaths }

impl AggregateInitializationOperationStore {
    pub fn create_idempotent(
        &self,
        operation: AggregateInitializationOperation,
    ) -> Result<AggregateInitializationOperation, ProductStoreError>;
    pub fn get(&self, project_id: &str, operation_id: &str)
        -> Result<AggregateInitializationOperation, ProductStoreError>;
    pub fn mark_running(&self, project_id: &str, operation_id: &str, at: String) -> Result<_, _>;
    pub fn mark_step_running(&self, project_id: &str, operation_id: &str,
        step: AggregateInitializationStepKind, key: String, at: String) -> Result<_, _>;
    pub fn checkpoint_step_output(&self, project_id: &str, operation_id: &str,
        step: AggregateInitializationStepKind, output: AggregateStepOutputRef) -> Result<_, _>;
    pub fn mark_step_completed(&self, project_id: &str, operation_id: &str,
        step: AggregateInitializationStepKind, at: String) -> Result<_, _>;
    pub fn finish_failed(&self, project_id: &str, operation_id: &str,
        step: AggregateInitializationStepKind, error: AggregateInitializationErrorRecord, at: String) -> Result<_, _>;
    pub fn cancel(&self, project_id: &str, operation_id: &str,
        request: AggregateCancellationRecord, at: String) -> Result<_, _>;
    pub fn recover_interrupted(&self, project_id: &str, operation_id: &str, at: String) -> Result<_, _>;
}
```

与旧 `RepositoryInitializationOperationStore` **独立文件、独立 enum、独立 validation**。不能通过 generic `Vec<StepRecord>` 放松旧 store 的固定长度检查，因为那会扩大单仓 JSON 的接受面。

## 5. 与旧 `RepositoryInitializationOperation` 的隔离

### 5.1 字节级兼容约束

以下对象保持不变：

- Rust：`RepositoryInitializationOperation`、`RepositoryInitializationStepKind::ALL`、`RepositoryInitializationStepRecord`、旧 operation store 的 `has_supported_step_layout`/GitFinalize checkpoint 规则；
- JSON：`projects/{project}/repository-initializations/{operation}.json` 的顶层字段、6 条 step IDs、次序与 enum snake_case；
- HTTP：现有 repository registration `POST`/`GET` 的 request/response DTO；前端 `RepositoryInitializationStepId` 继续为 6-member union；
- 行为：传统单仓路径继续在 repository persist 之后跑 git finalize，并保留 warning/checkpoint recovery 语义。

aggregate API 使用独立 DTO，例如：

```rust
pub struct AggregateInitializationOperationDto {
    pub operation_id: String,
    pub operation_kind: String, // "aggregate_initialization"
    pub status: String,
    pub steps: Vec<AggregateInitializationStepDto>,
    pub current_step: Option<String>,
    pub failed_step: Option<String>,
    pub member_statuses: Vec<AggregateMemberInitializationDto>,
    pub error: Option<ApiError>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}
```

不要在 `RepositoryInitializationOperationDto` 中加 `operation_kind: Option` 或将 step ID union 扩展为 11 项；即使 serde 可兼容，也会改变旧 API 响应字节并让 UI 错把 aggregate 状态渲染为一个仓库注册。

### 5.2 run registry 泛化

替换当前 registry 的 string key：

```rust
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum InitializationOperationKind {
    Repository,
    Aggregate,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct InitializationRunKey {
    pub kind: InitializationOperationKind,
    pub project_id: String,
    pub operation_id: String,
}

#[derive(Clone, Default)]
pub struct InitializationRunRegistry {
    active: Arc<StdMutex<HashSet<InitializationRunKey>>>,
}

impl InitializationRunRegistry {
    pub fn register(&self, key: InitializationRunKey) -> Option<InitializationRunLease>;
    pub fn is_active(&self, key: &InitializationRunKey) -> bool;
}
```

迁移方式：先把当前 `RepositoryInitializationRunRegistry` 改名/内部 delegate 到 `InitializationRunRegistry`，但保留其 public `register(operation_id)`/`is_active(operation_id)` facade 并固定生成 `{ Repository, project_id? }`。更好的实现是同步改旧 handler 让它传 project ID，避免旧 registry 跨项目同 operation ID 的理论碰撞。aggregate handler 始终传 `{Aggregate, project, operation}`。lease Drop 删除完整 key，确保同字符串 ID 的 repository 和 aggregate worker 可并行而不互相触发 GET recovery。

Registry 仍是进程内优化，不是权威运行记录；GET recovery 必须以各自 persistent operation store 判断，而不能仅依赖 `is_active`。

### 5.3 coordinator/HTTP 的隔离

- 新 route 推荐：`POST /api/projects/{project_id}/logical-codebase/initializations`、`GET .../{operation_id}`、`POST .../{operation_id}/cancel`。
- 新 `AggregateInitializationDependencies` 不接受 `RepositoryPersistence`，避免自然地落入 `RepositoryRegistrationCoordinator`。
- `WebAppState` 同时持有泛化 registry 和 aggregate dependency factory；旧 `repository_registration_dependencies` 不承载 aggregate 服务。
- aggregate operation 的 `operation_id` 前缀为 `aggregate_initialization_`；即使 registry kind 漏接线，也增加人工诊断性，不能作为唯一隔离。

## 6. 前端 repo_type 差异化 profile

### 6.1 检测位置与确定性规则

在 `aggregate_preflight` 内新增 `RepositoryTypeDetector`。它只读取 manifest member 的 **main checkout 根目录**（不向目录递归搜索、不读取 `.worktrees`、不跟随根外 symlink），结果写入 immutable preflight snapshot 及 member record 的经审计 projection。它不是现有 provider 产出的 `RepositoryProfile` 替代物：后者仍面向 Issue/规划，且现状没有自动检测 package manifest。

```rust
pub trait RepositoryTypeDetector: Send + Sync {
    fn detect(&self, checkout_root: &Path) -> Result<RepositoryTypeEvidence, ProductStoreError>;
}

pub struct RepositoryTypeEvidence {
    pub repo_type: RepositoryType,       // frontend/backend/mixed/unknown
    pub signals: Vec<String>,            // e.g. root_package_json, pnpm_lock, vite_config
    pub package_manager: Option<String>, // pnpm/npm/yarn/unknown
    pub framework: Option<String>,       // vite/react/next/unknown
    pub files_examined: Vec<String>,
    pub detector_version: String,
}
```

优先级（仅根目录）：

1. `package.json` 存在且可解析 JSON；读取 `packageManager`、`scripts`、dependencies/devDependencies；
2. `pnpm-lock.yaml` 或 `pnpm-workspace.yaml` → pnpm signal；
3. `vite.config.{ts,js,mts,mjs,cjs,cts}`，或 `package.json` deps/devDeps 包含 `vite` → Vite signal；
4. 典型 frontend deps（`react`、`vue`、`@angular/core`、`svelte`）或 scripts（`vite`/`next`）→ frontend signal；
5. Java signals（`pom.xml`、`build.gradle`、`build.gradle.kts`、`settings.gradle*`）→ backend signal；两边都有 → mixed；都没有/JSON 无法解析 → unknown + diagnostic。

不执行 `package.json` scripts、`pnpm install`、Vite、Node 或 Java 命令；预检的“检测”不可产生安装/构建副作用。检测结果与 manifest revision、checkout revision 一同进入 preflight digest，因此 checkout 改变会使旧 init 不可误复用。

### 6.2 profile 到 step 的行为矩阵

| repo type | `machine_skills` | `aggregate_preflight` | `pre_check` | `rule_and_mcp_config` | `openspec_and_examples` |
|---|---|---|---|---|---|
| backend（含 Java） | 执行 | 执行；收集 Maven/Gradle evidence | 使用 aggregate/Java-aware precheck profile | 生成 backend rules + MCP bundle | backend OpenSpec/examples profile |
| frontend | 执行 | 执行；验证 package.json，采集 pnpm/Vite signal；不可把 Java precheck 配置当作 prerequisite | **替换**为 frontend precheck profile；例如只校验 Node/pnpm/version/lockfile 证据，绝不跑 Java 命令 | **替换输入 template** 为 frontend（pnpm/Vite）规则/MCP 配置 | **替换输入 template** 为 frontend OpenSpec/examples；不产生 Java 示例 |
| mixed | 执行 | 执行；记录两类证据，若无明确 profile 返回 blocked | 使用 composite profile，provider prompt 明确按成员/目录边界处理 | 生成组合但 namespaced 的 rules/MCP；冲突则 block | 组合 examples，需显式 scope |
| unknown | 执行 | 成功但 member 为 `blocked`（或 operation preflight failed，取决于该 member 是否 included） | 不启动 | 不启动 | 不启动 |

聚合 operation 是 Project 范围，因此成员类型不一致时不能“只因一个 frontend 就跳过 Java 六步”：实际 rule 是由 Issue/aggregate initialization request 的 `included_member_ids` 与其 profile 决定。如果 request 不提供 scope，v1 应要求所有 active members 得到同一个可解析 profile，否则 fail closed 并提示分组初始化；不可以选择第一个 repository 的类型作为全局类型。

**关键区别：** 五个 stable ID 不随 frontend 改变或消失；变化的是 step 内选择的 `AggregateInitializationProfile` 与 provider prompt/template。这样 operation layout、进度 UI、恢复和审计不因成员技术栈漂移而改变。

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AggregateInitializationProfile {
    JavaBackend,
    FrontendPnpmVite,
    Mixed,
}
```

`AggregateInitializationProfile` 及 evidence digest 进入 `AggregateInitializationOperationInput`；启动前预检决定它，provider turn 只能消费，不能自行降级。未覆盖的 frontend（例如 npm/yarn/non-Vite）在 v1 应以 `Unknown/Blocked` 报错或纳入明确的新 profile，不能套用 Java 六步或盲目执行 pnpm。

### 6.3 前端 UI/DTO

aggregate UI 显示五个固定步骤、当前 step、member projection 和 profile/evidence（例如 `frontend_pnpm_vite`, `package_json`, `pnpm_lock`, `vite_config_ts`）；不要复用单仓“已注册 repository + GitFinalize warning”组件。frontend precheck 的失败详情应指明缺失 `package.json`、pnpm 或 Vite signal，而不是显示 Java command failure。

## 7. 实施测试清单

1. **旧 operation compatibility：** 用现存 6-step JSON fixtures 执行 deserialize/get/mark/recover，断言字节序列化和 DTO step IDs 未变；尝试向旧 store 写 aggregate layout 必须失败。
2. **aggregate layout：** create 产生恰好 `[machine_skills, aggregate_preflight, pre_check, rule_and_mcp_config, openspec_and_examples]`；所有跳步、两条 running、重排、错误 completed timestamp 均被拒绝。
3. **执行映射：** fake skills/preflight/provider 断言执行顺序为 5 steps、恰好 3 provider turns；rule/mcp 在一个 step 内生成两个子 action output。
4. **无成员 GitFinalize：** fake command runner 记录 cwd/argv；aggregate operation 不得调用 `git add`、`git commit`、`git push`，不得在 member root 写文件；单仓 registration fixture 仍保留原 GitFinalize 行为。
5. **失败/取消/重启：** 每个 step 强制失败、provider cancellation、publish 前/后 crash；断言后续 pending、staging cleanup、persistent recovery 以及不自动重启 provider。
6. **registry：** 同 `operation_id` 的 Repository/Aggregate 以及不同 project 可同时 register；重复完整 key 被拒绝，lease drop 精确释放 key。
7. **profile：** package.json+pnpm lock+vite config 检出 `FrontendPnpmVite`，Java root 检出 backend，二者共存检出 mixed；无根 package.json 的 frontend request 失败关闭；检测绝不运行 package script。
8. **成员状态投影：** member preflight failure 使 status blocked，不可把 operation completed 和 member ready 混同；成功只表示聚合 assets ready，不表示 member pointer 已发布。

## 8. 风险与需在 Plan 固化的决定

1. 现有 `/pre-check --no-interrupt --upgrade 用大陆镜像` 及其他 3 条 prompt 的实际语义由 provider skill 决定，仓库内没有可静态验证的 Java/前端分支；实现前须为三条 aggregate prompt 建立 versioned template 与 fixture。
2. `CadenceSkillsManager::prepare` 使用全局 `PREPARE_LOCK`（`cadence_skills/manager.rs:62-91`），它保证本进程 machine 步骤串行，但 aggregate operation 仍需持久化 idempotency/结果，不能把内存锁当作 crash recovery。
3. 当前 `write_json` 只保证单文件原子 replace；rule/MCP 的合并发布必须通过 staging+manifest/digest receipt 处理跨文件恢复，不能宣称两文件事务。
4. 当前代码没有 repository type 静态探测，且 “frontend” 不必然等同 pnpm/Vite；v1 仅把 package.json/pnpm/Vite 识别定义为明确 profile，其他生态需要新的受测 profile，不应进行隐式 fallback。
5. 取消 HTTP/API、Aria-managed aggregate asset 的精确路径、以及 member pointer publish operation 需要在实施 Plan 与 OpenSpec 后续条目中锁定；本 spike 已明确它们不属于旧 six-step operation 和不进入 member GitFinalize。
