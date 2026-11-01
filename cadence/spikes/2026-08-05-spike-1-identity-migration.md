# Spike 1：身份域与迁移 journal 设计

- Change：`2026-08-05-add-logical-codebase`
- 日期：2026-08-05
- 性质：实施前设计 spike；本文是 Rust/JSON/迁移协议草案，**没有修改产品代码或 OpenSpec**。

## 1. 基线结论与约束

本 spike 以当前磁盘持久化实现为准，而不是仅依据 change 的目标模型。

| 观察点 | 当前事实 | 对设计的影响 |
|---|---|---|
| 仓库 ID | `RepositoryStore::create` 以 `list().len()` 调用 `next_sequential_id("repository", existing_len)`，产生 `repository_0001` 等；`delete` 只从 `repos.json` 的 Vec 移除记录（`src/product/repository_store.rs:67-133`）。 | 删除后下一次创建可重用 ID；不能把它继续当作稳定逻辑身份。 |
| 物理仓存储 | 每项目单一 `projects/{project_id}/repos.json`；`RepositoryRecord.id` 是 API、Issue、workspace 与众多测试使用的字符串投影。 | 迁移不能改写其旧值，也不能让新读路径把它解释为逻辑仓 ID。 |
| 物理路径标识 | `repo_hash` 是 canonical path 的 SHA-256 前 12 位（`src/product/id.rs:3-5`），并非 Git 或逻辑仓身份。 | 不能用 `repo_hash` 做 member 去重、重定位或迁移幂等键。 |
| 单仓外键 | `IssueRecord.repo_id: Option<String>`、`IssueRuntimeBindingRecord.repo_id: String`、`StorySpecRecord.repository_id`、`LifecycleWorkItemRecord.repository_id`、`IssueSharedWorktree.repository_id` 都保存物理 `RepositoryRecord.id`（`models/project.rs`、`models/lifecycle.rs`）。 | 新语义字段必须与这些兼容投影并存，并逐条回填。 |
| Attempt | `CodingExecutionAttempt` 只有 work item/branch/worktree/provider 快照，没有 target logical/checkout/physical/revision/policy 快照；它还实现了手写 `Deserialize`（`src/product/coding_models/execution.rs:90-184`）。 | 新字段必须同时进入公开 struct、私有 `CodingExecutionAttemptSerde` 和重建逻辑；不能只在公开 struct 加 `#[serde(default)]`。 |
| 反向引用 | 现有 `RepositoryStore::delete` 完全不检查引用。binding/故事/work item/issue shared worktree/attempt 均是项目 issue 子树内的独立 JSON。 | 删除必须先全量扫描权威 JSON，不能依赖尚未存在的索引。 |
| 原子性 | `write_json` 是单文件 temp+sync+rename（`src/product/json_store.rs:33-106`），跨文件没有事务、也没有项目级迁移锁。 | journal 必须记录每个可重放写入的确定身份；完成 marker 必须最后写。 |

还确认了两项会影响实施排期的事实：

1. `IssueSharedWorktree` 当前每个 `(project, issue)` 只有一个 `issue-shared-worktree.json`，并非设计目标的 `(project, issue, repository)`；其 `repository_id` 是直接反向引用，且同 issue 跨仓并行前必须先迁移该路径/键（`src/product/lifecycle_store/worktree.rs:18-79`）。
2. Attempt 没有直接 `repository_id`。删除扫描须通过 Attempt 的 `work_item_id`（group attempt 还要检查其 `current_work_item_id`、unit/plan binding）解析到 `LifecycleWorkItemRecord.repository_id`，不能只扫描 attempt 顶层 JSON。

## 2. 三层身份与 Rust 草案

### 2.1 术语、不变量与分配规则

| 层 | 类型 | 谁分配 / 生命周期 | 允许用于 |
|---|---|---|---|
| 逻辑成员 | `LogicalRepositoryId` | 首次将一个 source 加入 logical codebase 时，v4 UUID；由 member/tombstone ledger 保留 | Issue selection、involved refs、`target_repository_id`、member 关系 |
| checkout 实例 | `RepositoryCheckoutId` | 主 checkout 登记或 worktree checkout 创建时，v4 UUID | 执行时路径解析、availability/revision、attempt target snapshot |
| 物理兼容投影 | `RepositoryRecord.id: String` | 已存值永不改写；新记录使用 UUID-backed `repository_<uuid-simple>`；删除/恢复由 ledger 控制 | 旧 API、旧 JSON、仍未迁移调用者、`repos.json` 物理记录定位 |

以下规则是实现门：

- `target_repository_id` 的 Rust 语义类型只能是 `LogicalRepositoryId`；禁止以同名 `String` 暗含物理 ID。
- `RepositoryRecord.id` 不是 logical ID，不能从其字符串格式推导任何一层身份。
- 一个 active member 必须有且仅有一个 `logical_repository_id`；一个 active checkout 必须归属一个 active member；主 checkout 还必须指向一个 active `RepositoryRecord.id`。
- checkout 的移动是 **同一 checkout 记录更新 `canonical_path` 的受控操作**；未知 source 不能因 `repo_hash` 相同而合并。
- 重新发现 tombstoned source 必须走显式 `reactivate_tombstoned_source`，使用 ledger 中的确定映射；普通 `create` 遇到 tombstone 返回可操作冲突，避免静默复活历史身份。

### 2.2 Rust newtype 及领域记录草案

现有 `uuid` 依赖只启用了 `v4`。若以下 transparent newtype 直接序列化 `Uuid`，实施时需将 Cargo feature 扩为 `uuid = { ..., features = ["v4", "serde"] }`；这是编译前置条件，本文不修改依赖。

```rust
use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// 外部 JSON 为单一 UUID 字符串，例如 "018f..."；不可接受 repository_0001。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct LogicalRepositoryId(pub Uuid);

/// 外部 JSON 为单一 UUID 字符串；代表可解析的 checkout 实例而非逻辑成员。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Ord, PartialOrd, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RepositoryCheckoutId(pub Uuid);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberStatus {
    Active,
    Removed,
    Tombstoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutKind {
    Main,
    IssueWorktree,
    Imported,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CheckoutAvailability {
    Available,
    Missing,
    Unresolved,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RepositoryType {
    Backend,
    Frontend,
    Mixed,
    Unknown,
}

/// 这是保守的“同一 source instance”证据，不是 repo_hash，也不是 Git 全局 UUID。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositorySourceIdentity {
    pub scheme: String,                 // "git_dir_and_origin_v1" 或 "git_dir_only_v1"
    pub key_digest: String,             // sha256(canonical_git_dir + NUL + canonical_origin_or_empty)
    pub canonical_git_dir: PathBuf,     // 诊断/人工确认，不作单独等价判断
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub canonical_origin: Option<String>,
    pub first_seen_path_hash: String,   // 仅证据；不得作为主键
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CodebaseMemberRecord {
    pub logical_repository_id: LogicalRepositoryId,
    /// 旧 API/记录的物理兼容投影；字段名明确不表达 logical identity。
    pub physical_repository_id: String,
    pub alias: String,
    pub role: String,
    pub ordinal: u32,
    pub source_identity: RepositorySourceIdentity,
    #[serde(default)]
    pub repo_type: RepositoryType,
    #[serde(default)]
    pub tech_stack: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owner: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_ref: Option<String>,
    #[serde(default)]
    pub checkout_ids: Vec<RepositoryCheckoutId>,
    #[serde(default)]
    pub status: MemberStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryCheckoutRecord {
    pub checkout_id: RepositoryCheckoutId,
    pub logical_repository_id: LogicalRepositoryId,
    pub physical_repository_id: String,
    pub kind: CheckoutKind,
    pub canonical_path: PathBuf,
    pub checkout_path_hash: String,
    pub git_dir_identity: String,       // source identity 的 git-dir 部分摘要
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,       // Cadence 观察到的 HEAD；不是 source identity
    #[serde(default)]
    pub availability: CheckoutAvailability,
    pub observed_at: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 仅为读旧数据/旧 API 加的兼容投影；保留 `id: String` 原字段及其 JSON 名称。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryRecord {
    pub id: String,
    // ... 保持当前全部既有字段和顺序语义 ...
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logical_repository_id: Option<LogicalRepositoryId>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub primary_checkout_id: Option<RepositoryCheckoutId>,
    #[serde(default)]
    pub identity_schema_version: u16,   // 0 = legacy；1 = 已回填
}
```

`Uuid::new_v4()` 只发生在 allocation/reactivation 决策中；绝不能按数组长度、文件数或 `repo_hash` 生成。为保持现有 `validate_relative_id` 与 URL 路由约束，新 physical projection 可用 `format!("repository_{}", Uuid::new_v4().simple())`；logical/checkout JSON 仍是无前缀 UUID 字符串。

### 2.3 Attempt target 快照草案

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptTargetSnapshot {
    pub logical_repository_id: LogicalRepositoryId,
    pub checkout_id: RepositoryCheckoutId,
    pub physical_repository_id: String,
    pub canonical_path: PathBuf,
    pub git_dir_identity: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub revision: Option<String>,
    pub policy_digest: String,
    pub membership_revision: u64,
    pub captured_at: String,
    /// "created" | "migration_observed"；migration_observed 绝不允许 resume 写入。
    pub capture_source: String,
}

pub struct CodingExecutionAttempt {
    // 保持现有字段
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_snapshot: Option<AttemptTargetSnapshot>,
}

#[derive(Deserialize)]
struct CodingExecutionAttemptSerde {
    // 与公开 struct 同步保留所有现有字段
    #[serde(default)]
    target_snapshot: Option<AttemptTargetSnapshot>,
}
```

新建 attempt 采用 `Some` 且校验快照四层映射在创建时一致；`None` 仅允许旧 JSON 读入。运行中/可 resume 的 legacy attempt 若无法获得历史真实性快照，必须标记 `target_snapshot_missing` 并拒绝 logical-codebase resume，不能把当前 HEAD 伪装成创建时 HEAD。

## 3. 持久化形态（JSON schema 草案）

### 3.1 路径、权威性与版本

新增权威根目录：

```text
.aria/projects/{project_id}/logical-codebase/
├── manifest.json
├── members/{logical_repository_uuid}.json
├── checkouts/{checkout_uuid}.json
├── identity-registry.json
├── migration-journal.json
└── aggregate-indexes/{aggregate_index_id}.json        # 后续 aggregate-index 工作包写入
```

实际 `ProductAppPaths` 当前只提供 `project_root()` 和 `repository_initializations_root()`；实施须添加显式 `logical_codebase_root(project_id)`，不能散落手写 join。`repos.json` 在双读/双写窗口仍是物理仓兼容投影，**不是**新领域权威。

所有时间为 RFC 3339 字符串；所有 ID 字段必须经 `validate_relative_id`/UUID parse 校验后才组成路径。JSON 示例展示字段名、类型和默认策略，非完整 JSON Schema draft-2020 文档。

### 3.2 `manifest.json`

```json
{
  "schema_version": 1,
  "project_id": "project_0001",
  "logical_codebase_id": "f2d3c...-....",
  "provider_context_root": "/workspace/non-git-parent",
  "layout": "common_non_git_parent",
  "membership_revision": 7,
  "member_ids": ["018f...-...."],
  "active_aggregate_index_id": null,
  "context_policy_digest": "sha256:...",
  "created_at": "2026-08-05T00:00:00Z",
  "updated_at": "2026-08-05T00:00:00Z"
}
```

| 字段 | 类型 | serde / 校验 |
|---|---|---|
| `schema_version` | `u16` | 必填；当前只接受 `1`。 |
| `project_id` | `String` | 必填，必须等于目录 project id。 |
| `logical_codebase_id` | UUID string | 必填，属于 Project 级 codebase，不替代 member logical ID。 |
| `provider_context_root` | path string | 必填；canonical、绝对、非 git，且成员路径满足布局。 |
| `layout` | enum string | 必填，v1 为 `common_non_git_parent`。 |
| `membership_revision` | `u64` | 必填，任何 member/checkout 可见性变更单调加一。 |
| `member_ids` | UUID string array | `#[serde(default)]` 只用于 crash recovery 的早期草稿；完成 manifest 必须与 member 文件一一对应。 |
| `active_aggregate_index_id` | string/null | `#[serde(default)]`。 |
| `context_policy_digest` | string | `#[serde(default)]`，旧迁移填 `""`，但 logical provider 启动必须拒绝空 digest。 |

### 3.3 member 记录：`members/{logical_repository_id}.json`

```json
{
  "logical_repository_id": "018f...-....",
  "physical_repository_id": "repository_0001",
  "alias": "api",
  "role": "service",
  "ordinal": 10,
  "source_identity": {
    "scheme": "git_dir_and_origin_v1",
    "key_digest": "sha256:...",
    "canonical_git_dir": "/workspace/api/.git",
    "canonical_origin": "ssh://git@example/acme/api.git",
    "first_seen_path_hash": "7e3a..."
  },
  "repo_type": "unknown",
  "tech_stack": [],
  "owner": null,
  "tags": [],
  "default_ref": null,
  "checkout_ids": ["8af0...-...."],
  "status": "active",
  "created_at": "...",
  "updated_at": "..."
}
```

- `logical_repository_id` 必须等于文件名 UUID；`physical_repository_id` 必须命中 active `repos.json` record 或对应 tombstone。
- `repo_type`、`tech_stack`、`owner`、`tags`、`default_ref`、`checkout_ids`、`status` 都使用 `#[serde(default)]`，以允许后续 enrichment 和最小迁移。
- 对旧记录没有 member 文件的情形，reader 先走 journal/legacy projection 构造 **内存** member；写路径不得仅写内存投影。

### 3.4 checkout 记录：`checkouts/{checkout_id}.json`

```json
{
  "checkout_id": "8af0...-....",
  "logical_repository_id": "018f...-....",
  "physical_repository_id": "repository_0001",
  "kind": "main",
  "canonical_path": "/workspace/api",
  "checkout_path_hash": "7e3a...",
  "git_dir_identity": "sha256:...",
  "revision": "abc123...",
  "availability": "available",
  "observed_at": "...",
  "created_at": "...",
  "updated_at": "..."
}
```

`revision: Option<String>`、`availability`（default `unresolved`）可 serde default；`checkout_id`、logical/physical IDs、path、kind、时间必须存在。`issue_worktree` 记录是执行资产，不写回 `RepositoryRecord.primary_checkout_id`；后者永远指 main checkout。

### 3.5 identity registry / tombstone：`identity-registry.json`

这是一张不能由 `repos.json` 重建的 ledger，用来防止删除后的 source 被 `len+1` 或路径 hash 静默冒认。

```json
{
  "schema_version": 1,
  "entries": [
    {
      "source_identity": { "scheme": "...", "key_digest": "sha256:...", "canonical_git_dir": "/...", "canonical_origin": null, "first_seen_path_hash": "..." },
      "logical_repository_id": "018f...-....",
      "physical_repository_id": "repository_...",
      "primary_checkout_id": "8af0...-....",
      "state": "active",
      "created_by_key": "identity-migration:project_0001:repository_0001",
      "deleted_at": null,
      "delete_operation_id": null,
      "reactivated_at": null
    }
  ],
  "updated_at": "..."
}
```

`state` 为 `active | tombstoned`。字段 `deleted_at`、`delete_operation_id`、`reactivated_at` 均 `#[serde(default, skip_serializing_if = "Option::is_none")]`。v1 migration 建立 active entries（不是伪造 tombstone）；删除原子顺序的最后一步将相同 entry 变为 tombstoned。遇到同一 `source_identity.key_digest` 但证据字段不完全相同，返回 `source_identity_collision`，要求人工 adoption，不可自动 merge。

**source identity 的边界：** Git 没有可通用读取的永久 repo UUID。方案把 canonical git-dir 与规范化 origin（无 origin 时仅 git-dir）合成“同一 checkout instance”证据，因此目录移动或 clone 到另一台机器不会自动视为同一 source；必须显式 adoption，并记录审计。这个保守限制比用 `repo_hash` 误合并更安全。

## 4. migration journal 与版本 marker

### 4.1 数据结构和位置

`migration-journal.json` 是 per-project、schema v1 一次迁移的可恢复状态机；未出现时项目处于 `legacy_only`。journal 创建即为迁移开始标记，`manifest.schema_version=1` 加 `read_mode=logical_authoritative` 才表示切换完成。

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IdentityMigrationPhase {
    Scanning,
    Mapping,
    WritingAuthority,
    BackfillingCompatibility,
    DualReadWrite,
    SwitchingReads,
    LegacyFallbackRemoved,
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryIdentityMapping {
    pub legacy_repository_id: String,
    pub source_identity_digest: String,
    pub logical_repository_id: LogicalRepositoryId,
    pub primary_checkout_id: RepositoryCheckoutId,
    /// 已有记录保留原值；新记录为 UUID-backed repository_<uuid>。
    pub physical_repository_id: String,
    pub idempotency_key: String,
    #[serde(default)]
    pub authority_written: bool,
    #[serde(default)]
    pub compatibility_backfilled: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdentityMigrationJournal {
    pub journal_version: u16,             // 1
    pub migration_id: String,             // identity-migration:{project_id}:v1
    pub project_id: String,
    pub target_schema_version: u16,       // 1
    pub phase: IdentityMigrationPhase,
    pub source_repos_digest: String,      // sorted legacy repos.json canonical JSON digest
    pub mappings: Vec<RepositoryIdentityMapping>,
    #[serde(default)]
    pub completed_keys: Vec<String>,      // 诊断/审计；写入依 mapping bool 校验
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_mode: Option<String>,        // legacy_projection | dual | logical_authoritative
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}
```

所有阶段写前都取得项目专属排他锁：`logical-codebase/.identity-migration.lock`。锁只保护当前进程间迁移；每一持久化动作仍以 journal mapping 的 `idempotency_key` 实现 crash 后重放。每次重放先读目标文件，再验证 `(project_id, logical id, physical id, source digest)`；完全一致即 no-op，不一致即 `IdentityMismatch`/`migration_conflict` 并停在 `Failed`，绝不覆盖。

### 4.2 阶段、入口、幂等键和恢复点

| 阶段 | 实施入口及动作 | 幂等键 | 成功后恢复点 / 失败处理 |
|---|---|---|---|
| 0. 发现/加锁 | `RepositoryStore::ensure_identity_schema(project_id)` 在 `list/create/delete/find_by_path` 前调用；不存在 journal 时创建 `Scanning` journal。 | `identity-migration:{project}:v1` | journal 存在即读取并续跑；并发调用等待/返回 migration in progress。 |
| 1. 扫描 | 读取 `repos.json`，按原 `id` 排序；扫描 issue 子树，建立待回填的外键清单；记录 `source_repos_digest`。 | `scan:{migration_id}:{source_repos_digest}` | journal 落盘后才进入 Mapping。若 legacy `repos.json` 在非本锁持有者下变化，digest 不匹配则 Failed，要求重新扫描/人工合并。 |
| 2. 映射 | 对每个 legacy `RepositoryRecord` canonicalize path、读取 git-dir/origin，生成 UUID 并**先写入 journal mapping**。已存在 registry active entry 时复用其 mapping；未映射才 `Uuid::new_v4()`。 | `map:{project}:{legacy_physical_id}:{source_identity_digest}` | UUID 已落盘，进程崩溃后绝不重新生成；Git 不可读取时记录具体 repository error，保持 Mapping。 |
| 3. 写 manifest/member/checkout/registry | 写 manifest（成员全集）、每个 member、main checkout 和 identity registry active entry；以文件内身份校验实现逐文件 no-op。 | `authority:{migration_id}:{legacy_physical_id}` | 单文件完成后设 mapping `authority_written=true`。所有 mappings 完成才 phase 前进；manifest 的 revision/成员集合必须等于 journal。 |
| 4. 回填兼容投影 | 回写 `repos.json` 仅加 `logical_repository_id`、`primary_checkout_id`、`identity_schema_version=1`；再写 Issue selection/新语义字段，旧 `repo_id`/`repository_id` 保留。 | `backfill:{migration_id}:{record_kind}:{record_id}` | 每条记录成功可重放；attempt 见下文特殊规则。完成后 `read_mode=dual`。 |
| 5. 双读双写 | 新写路径先写 logical authority，再在同一项目锁内写旧物理投影；读路径新字段优先、旧字段仅 fallback，并记录 fallback metric。每次写后做映射一致性断言。 | 业务 command/request ID，例如 `repository-registration:{operation_id}` | 任一双写失败返回失败且保留 journal/dirty 诊断；不得只让其中一侧被后续请求当成完成。补偿由同一 command 重放。 |
| 6. 读路径切换 | `IdentityMigrationVerifier::verify(project)` 重新扫描所有 authority/compatibility 对、无 fallback、无 active legacy attempt 后，将 journal `read_mode=logical_authoritative`。 | `switch:{migration_id}:{source_repos_digest}:{membership_revision}` | marker 是最后一个文件写入；切换前崩溃仍为 dual。切换后出现不一致立即 fail-closed，不回退到猜测式 legacy 解析。 |
| 7. 删旧 fallback | 删除的是**代码 fallback 分支和 legacy-only reader**，不是 `RepositoryRecord.id`、`IssueRecord.repo_id` 等兼容字段；这些字段需跨版本保留。标记 `legacy_fallback_removed`。 | `remove-fallback:{migration_id}` | 只有全项目验证、旧数据支持窗口到期、恢复工具可从 authority 重建投影后执行；保留只读 import/repair 工具。 |

### 4.3 外键回填及 Attempt 的特殊处理

`backfill` 的目标如下；所有新字段均保留原物理字段作为兼容投影。

| 现有记录 | 回填新权威数据 | 旧字段处理 |
|---|---|---|
| `RepositoryRecord` | `logical_repository_id`、`primary_checkout_id` | `id` 原样保留。 |
| `IssueRecord` | `issues/{issue}/codebase-selection.json`：`included=[logical]`、`focus=[logical]`、`selection_policy=explicit` | `repo_id` 原样保留，双写期间由 selection 投影更新。 |
| `IssueRuntimeBindingRecord` | `logical_repository_id`、`checkout_id`（新增 optional fields） | `repo_id` 原样保留。 |
| `StorySpecRecord` | `logical_codebase_ref`、`involved_repository_ids=[logical]`、`focus_repository_id=Some(logical)` | `repository_id` 原样保留。 |
| `LifecycleWorkItemRecord` | `target_repository_id=Some(logical)` | `repository_id` 原样保留；新创建在 feature flag 开启时要求 Some。 |
| `IssueSharedWorktree` | `target_repository_id`、`checkout_id`；同时迁到仓维路径 | `repository_id` 原样保留，旧单文件只作 migration source。 |
| `RepositoryProfile` / plan 产物 | `logical_repository_id` 或 profile member snapshot | 原 `repository_id` 原样保留，避免 profile 指向错误仓。 |
| `CodingExecutionAttempt` | `target_snapshot` | 不覆盖原 worktree/branch/provider fields。 |

对 attempt：由 work item 解析出的 physical ID 只可生成 `capture_source="migration_observed"` 快照。已完成/失败/中止 attempt 可以保存该审计快照（`revision` 不可得时为 `null`）；`is_active()` 的 attempt 不得当作安全可 resume。它们在切换验证中产生 blocker，要求先终止/人工恢复为带新快照的 attempt。对 group attempt，还须读取 group initialization journal/units/plan binding 验证唯一 target；发现 mixed/无法解析则阻断 switch，不能选择第一个 work item 猜测。

## 5. `RepositoryStore::create/delete` 改造点

### 5.1 输入、输出与 service 边界草案

```rust
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CreateRepositoryInput {
    pub project_id: String,
    pub name: String,
    pub path: PathBuf,
    pub default_policy_preset: Option<String>,
    pub default_provider_mode: Option<String>,
    /// 调用方持久化的 command/operation id；注册流程传 repository initialization operation_id。
    pub idempotency_key: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteRepositoryCommand {
    pub operation_id: String,
    pub expected_updated_at: Option<String>,
    /// 默认为 false；无任何“忽略反向引用”的 force 删除路径。
    pub allow_tombstone_reactivation: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepositoryDeletionReceipt {
    pub physical_repository_id: String,
    pub logical_repository_id: LogicalRepositoryId,
    pub checkout_id: RepositoryCheckoutId,
    pub tombstone_operation_id: String,
    pub deleted_at: String,
}

impl RepositoryStore {
    pub fn create(&self, input: CreateRepositoryInput)
        -> Result<RepositoryRecord, ProductStoreError>;

    pub fn delete(
        &self,
        project_id: &str,
        physical_repository_id: &str,
        command: DeleteRepositoryCommand,
    ) -> Result<RepositoryDeletionReceipt, ProductStoreError>;

    pub fn resolve_logical_repository(
        &self,
        project_id: &str,
        logical_id: LogicalRepositoryId,
    ) -> Result<(CodebaseMemberRecord, RepositoryCheckoutRecord, RepositoryRecord), ProductStoreError>;
}
```

`RepositoryPersistence::create_repository`、registration coordinator 以及 HTTP `DELETE` 都要随之接入 command/idempotency key；当前 Web 删除 handler 仅把 path 参数传给 `store.delete`（`src/web/handlers/product_resources.rs:100-109`），因此需要新增请求 operation ID（推荐 `Idempotency-Key` header）及返回 receipt。不能为了保留旧 handler 静默自动生成随机 key，否则客户端重试仍会重复创建/删除。

### 5.2 create 的明确流程

1. 取得项目 identity lock，调用 `ensure_identity_schema`；canonicalize 传入路径，读取 `git rev-parse --git-dir` 和规范化 origin，构造 `RepositorySourceIdentity`。
2. 在 registry 查 source：`active` 返回 `repository_already_registered`；`tombstoned` 返回 `repository_source_tombstoned`，客户端必须调用有审计的 reactivation command；无记录才分配新的三层 UUID/physical projection。
3. 以 `idempotency_key` 查询 command receipt。若同 key 的 input digest 一致，返回原 `RepositoryRecord`；不同则 `idempotency_key_reused`。
4. 先写 authority（member、main checkout、registry active entry、membership revision），再写 `repos.json`。`RepositoryRecord` 由 UUID-backed physical ID 创建，并带 identity compatibility fields。
5. 写 registration/create receipt。若在步骤 4 的两个文件之间中断，下一次相同 key 用 registry/mapping 校验并补齐，不能分配新 UUID。

现有 repository registration 在 initializer 成功后才调用 `create_repository`，再进行 GitFinalize（`registration.rs:491-575`）。新 `idempotency_key` 应使用该 operation id；这样 persist 重试不会因一次 GitFinalize/网络失败产生第二个 member。

### 5.3 delete 的反向引用检查与实现位置

`RepositoryStore` 负责交易边界、tombstone 和 `repos.json`，但不应把所有目录遍历隐藏在一个匿名 closure。新增内部 `RepositoryReferenceScanner`，由 `RepositoryStore::delete` 调用，输入 `(project_id, physical_repository_id, resolved logical_id)`，输出结构化 `RepositoryReferenceReport { blockers: Vec<RepositoryReference> }`。它必须直接读取权威 JSON（文件索引仅可作为加速，且命中/漏失均要回退扫描）。

| 检查类别 | 当前/新增位置 | 判定 |
|---|---|---|
| active member | `logical-codebase/members/*.json` + registry | `physical_repository_id` 或 logical ID 仍为 `active` 即 blocker；应先走 member removal 工作流。 |
| Issue | `IssueStore::list(project)`（当前枚举 `issues/*/issue.json`） | `repo_id == physical`，或 selection/focus 包含 logical，即 blocker。 |
| runtime binding | 对每个 Issue 调 `RuntimeBindingStore::list(project, issue)`；当前路径为 `issues/{issue}/bindings/*.json` | `repo_id == physical`，或新增 logical/checkout field 命中，即 blocker。 |
| StorySpec / WorkItem / Profile | `LifecycleStore::list_story_specs/list_work_items` 及 repository profile list，逐 issue | `repository_id == physical` 或新 logical target 命中，即 blocker。 |
| coding attempt | 新增 `CodingAttemptStore::list_attempts_for_project(project)`，基于当前 `coding_attempts` 真实 JSON 扫描；再检查 `target_snapshot`，legacy 则解析 work item/group binding。 | 任一 attempt（含 completed history，除非未来有独立 archive policy）指向该 target 即 blocker。不能仅查 active attempt。 |
| shared worktree | 旧 `get_issue_shared_worktree` 与新 `(issue, logical)` 路径都扫描 | physical/logical/checkout 任一命中，或工作目录仍存在，均 blocker。 |
| aggregate index | `logical-codebase/aggregate-indexes/*.json` 的 member snapshots/active pointer | 包含 logical ID、checkout ID 或 source digest 的 active/stale/last-known-good index 均 blocker；须先 supersede/retire index。 |
| 运行中初始化/任务 | repository-initializations 的 running operation、未来 aggregate operation、worktree lock 文件 | 当前 registry 仅进程内，重启后不能证明无运行；持久 operation 是删除前置检查。 |

若 report 非空，返回 `ProductStoreError::Conflict { kind: "repository_references", id }`，Web error details 带稳定的 `{kind, record_id, path}` 列表；**没有** `force=true` 旁路。检查通过后顺序为：写 delete intent receipt → 更新 registry entry 为 tombstoned → 将 member/checkouts 状态改为 removed/tombstoned（若其 removal 已获批准）→ 从 `repos.json` 移除物理 record → 写 completed receipt。每步可用 `operation_id` 重放；对外把 tombstone receipt 返回成功的唯一证据。

## 6. serde 兼容清单与读取/回填规则

### 6.1 要加 `#[serde(default)]` 的字段

| Struct / 文件 | 新字段 | 默认与读取后的行为 |
|---|---|---|
| `RepositoryRecord` | `logical_repository_id: Option<_>`、`primary_checkout_id: Option<_>`、`identity_schema_version: u16` | `None/None/0` 表示 legacy；只在 migration journal 允许时根据 `repos.json` 映射投影。 |
| `IssueRuntimeBindingRecord` | logical ID、checkout ID | `None`；未回填时由 `repo_id` 只读解析，写前必须先迁移。 |
| `StorySpecRecord` | `logical_codebase_ref`、`involved_repository_ids`、`focus_repository_id` | `None/[]/None`；fallback 到 `repository_id` 仅限 dual mode。 |
| `LifecycleWorkItemRecord` | `target_repository_id: Option<_>` | `None`；legacy fallback 到 `repository_id`，logical coding 写操作拒绝 None。 |
| `IssueSharedWorktree` | logical target、checkout ID、path schema version | `None/None/0`；旧路径只作为 import source。 |
| `RepositoryProfile` / plan records | logical repository ref、membership revision snapshot | `None`；不允许在 logical planning 中把空值当默认 member。 |
| `CodingExecutionAttempt` 和其 `CodingExecutionAttemptSerde` | `target_snapshot: Option<_>` | `None`；如 §4.3，active/resume fail-closed。 |
| 新的 manifest/member/checkout/registry/journal structs | 见 §3/§4 所列 extension fields | default 只能支持“尚未 enrich”；身份主键、project_id、path 等核心字段不 default。 |

`skip_serializing_if = "Option::is_none"` 只用于可选新增字段；在一轮迁移尚未回填的旧对象上避免无意义 JSON 噪音。完成回填后 normal write 会持久化 `Some(...)`；不得以 skip serialize 代替 migration。

### 6.2 双读优先级与回填算法

1. 先读 manifest/member/checkout 和 `RepositoryRecord.logical_repository_id`；四者全部一致才产生 logical resolution。
2. 若 journal `read_mode == dual` 且新字段缺失，以旧 physical field 在 registry 查唯一 mapping；找到一个才返回带 `resolution_source=legacy_projection` 的内存结果，并记录 metric/audit。
3. 若 0 或多个 mapping，返回 `identity_resolution_missing/ambiguous`；不能取 `repositories[0]` 或用 `repo_hash` 猜测。
4. 任何写入先要求 logical resolution；更新新权威记录后在同 key 下投影旧 physical fields。新数据不得只写 `repo_id`/`repository_id`。
5. journal 为 `logical_authoritative` 后，业务 reader 不再调用步骤 2；只保留受控 repair/import CLI。`

## 7. 可实施测试与验收建议

实施 Plan 应至少覆盖：

- 删除 `repository_0001` 后新增仓不再得到同 ID；新 ID、logical ID、checkout ID 均 UUID-backed 且独立。
- 从旧 `repos.json`（无新字段）迁移，模拟在 authority 的每个文件写后进程中断；相同 journal 重跑产生完全相同 UUID、manifest/member/checkout，不重复成员。
- 回填含 Issue、binding、Story、WorkItem、worktree、profile、completed attempt 的 fixture；缺 active attempt target 快照的 fixture 必须阻止切换。
- 反向扫描分别命中 member/binding/attempt/worktree/index，以及旧物理字段和新逻辑字段；所有命中都使 delete 返回 Conflict 且不改变 `repos.json`。
- 仅在引用为零时 delete 写 tombstone；同 source 的 reactivation 需要显式 command，普通 create 不会暗中复活。
- 旧 JSON serde round-trip 和手写 `CodingExecutionAttempt` Deserialize：旧文件可读，新 attempt 会序列化完整 target snapshot。

## 8. 结论、未决风险

本设计将稳定性放在 UUID ledger 与 journal 已持久化的 mapping 上，而不是当前可变的 array length、path hash 或一次性内存推断。它可以在当前单文件原子写能力下恢复跨文件迁移，但必须以项目锁、逐项 idempotency key 和最后 marker 实现，不能宣称跨文件原子事务。

残余风险：当前项目没有 Git 提供的永久 source UUID，目录移动/重复 clone 的身份 adoption 必须显式处理；旧完成 attempt 是否应长期阻止物理仓删除还需要产品定义 archive/retention 政策。在该政策未获批前，本 spike 的保守建议是所有 attempt/index 历史引用均阻断删除。
