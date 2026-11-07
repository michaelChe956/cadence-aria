/// Root recipe 副作用审计的固定 allowlist（相对 canonical root）。
///
/// Task 3.5 carry ②（Task 1.8 §五.2）：按真实 Claude Code（2.1.283）四命令
/// recipe 在空的非 Git canonical root 的实测落盘证据逐项冻结（spike 与
/// 全链 E2E，见 cadence/reports/2026-10-02_验收报告_LC根初始化全链E2E）：
/// `/pre-check` 写 `.agents`/`.claude`/`.kimi-code`/`.pi`/`openspec`；
/// `/rule-config` 写 `AGENTS.md`/`CLAUDE.md`/`.claude/rules`/`.agents/rules`
/// /`.omp`/`cadence`；`/mcp-configuration` 写 `.mcp.json`/`.codex`/`.gitignore`；
/// `/project-rules-examples` 写 `cadence/project-rules/examples`。聚合
/// artifact 只允许落在 `.aria/aggregate/**`。绝不扩大为整个 root，也绝不
/// 放行成员仓路径（D2；成员 `.git` 分类优先于 allowlist）。2026-10-02
/// E2E 增补：`.codegraph`/`codegraph.json` 为产品自管聚合索引面
/// （aggregate_index/exclude.rs 根扫描白名单同款），索引建立后 codegraph
/// daemon 与 recipe 共存属真实部署事实，按此补齐。r49(claude r48 现场)
/// 增补：`.mcp.json.cadence-backup-*` 为 Claude CLI 修改 `.mcp.json` 时
/// 自动落盘的管理性备份（CLI 自身行为，同 workspace 重试必触发）；尾缀
/// `*` 通配仅匹配同目录单文件（不跨 `/`，见 allowlist_scope_contains）。
pub const ROOT_RECIPE_ALLOWLIST: &[&str] = &[
    ".aria/aggregate",
    ".codegraph",
    "codegraph.json",
    "AGENTS.md",
    "CLAUDE.md",
    ".mcp.json",
    ".mcp.json.cadence-backup-*",
    ".gitignore",
    ".claude",
    ".agents",
    ".omp",
    ".codex",
    ".kimi-code",
    ".pi",
    "cadence",
    "openspec",
];

/// 根规则通用入口文件（Task 1.6，REQ-BOOT-03）：AGENTS.md 是四家 provider
/// 的通用入口；CLAUDE.md 仅是兼容副本，不进入 readiness 身份（副本策略
/// 差异不得制造伪漂移）。
pub const ROOT_RULE_ENTRY_FILE: &str = "AGENTS.md";

/// Task 1.5 carry → Task 3.4：快照规模的预算（条目/字节上限）。超限
/// fail-closed 报错——绝不静默截断观测面，也绝不让 receipt 体积随 root
/// 规模无界增长；预算内的审计语义与无预算时完全一致。
///
/// **口径（REQ-AUD-01，2026-11-03 块2）**：预算计的是**本次变更量**，不是
/// 整仓存量——两遍扫描只记元数据，差分出变更清单后，`entries` 与 `bytes`
/// **都只对进入变更清单的路径**计；未变化路径与 `.git` 子树（整棵跳过，
/// REQ-AUD-02）一律不计。存量体量不再压垮预检（未被忽略的构建产物再多，
/// 窗口内零改动即零计费）。
///
/// 默认值钉定依据 Task 3.4 大 LC 实测（36 成员 fixture：1776 条目/
/// 1.02 MiB），按 ≥10× 条目/≥60× 字节余量取整：数十成员 LC 及其常规
/// 打包历史远在预算内；失控规模（海量 loose 对象/巨型文件）在审计前
/// 即被阻断（见验收报告 2026-10-01_验收报告_LC根初始化大LC与写边界）。
/// Bug2（2026-10-16）：预算只对 gitignore 观测面生效——被 `.gitignore`
/// （含嵌套成员仓）与 `.git/info/exclude` 忽略的路径不计条目/字节。
/// **数值不变**（20 000 条目 / 64 MiB）——口径变了，阈值没变。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RootRecipeSnapshotBudget {
    /// 变更清单条目（目录/文件/symlink）总数上限。
    pub max_entries: usize,
    /// 变更清单累计读取字节上限（仅按被哈希的文件内容计）。
    pub max_bytes: u64,
}

impl RootRecipeSnapshotBudget {
    pub const DEFAULT: Self = Self {
        max_entries: 20_000,
        max_bytes: 64 * 1024 * 1024,
    };
}

fn default_allowlist() -> Vec<String> {
    ROOT_RECIPE_ALLOWLIST
        .iter()
        .map(|entry| (*entry).to_string())
        .collect()
}

// ---------------------------------------------------------------------------
// Filesystem snapshot
// ---------------------------------------------------------------------------

/// 快照条目类型。symlink 只记录链接目标，绝不跟随（防逃逸/防循环）。
/// special 记录非常规条目（Unix socket/fifo/device 等）：路径+类型可
/// 观测、无内容 digest——IPC 产物是产品自管索引面（`.codegraph` daemon
/// 等）的运行时面目，可观测但不 fail-closed 断审（2026-10-02 E2E 缺陷
/// 回归：audit cannot observe unsupported entry type）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootRecipeSnapshotEntryKind {
    File,
    Dir,
    Symlink,
    Special,
}

impl RootRecipeSnapshotEntryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
            Self::Symlink => "symlink",
            Self::Special => "special",
        }
    }
}

/// canonical root 下一个路径的快照事实：canonical 相对路径（正斜杠）、
/// 类型、内容 digest（文件）或链接目标（symlink）。
///
/// **新快照不再填内容摘要**（REQ-AUD-01，2026-11-03 块2）：两遍扫描只记
/// 元数据，内容哈希只对**变更清单**里的文件计算并落在
/// [`RootRecipeObservedChange::after_digest`]。本字段保留（旧 receipt 仍
/// 带内容摘要，反序列化必须继续成功）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecipeSnapshotEntry {
    pub path: String,
    pub kind: RootRecipeSnapshotEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
    /// special 条目的具体类型（socket/fifo/char_device/block_device/other）；
    /// 其余条目为 None。旧 receipt 反序列化缺省 None（向后兼容）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub special_type: Option<String>,
    /// symlink 词法解析后逃出 canonical root 时为 `true`（快照时判定）。
    #[serde(default)]
    pub escapes_root: bool,
}

/// canonical root 的一次全量快照（条目按路径升序，含整体 digest）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecipeFilesystemSnapshot {
    pub canonical_root: PathBuf,
    pub entries: Vec<RootRecipeSnapshotEntry>,
    pub snapshot_digest: String,
}

// ---------------------------------------------------------------------------
// Observed changes
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootRecipeChangeKind {
    Added,
    Removed,
    Modified,
}

/// 变更分类（证据同时保留在 [`RootRecipeObservedChange`] 的 digest/target
/// 字段中）：只有 `allowlisted_artifact` 是合法变更，其余全部拒绝。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootRecipeChangeClass {
    AllowlistedArtifact,
    UnknownPath,
    MemberGit,
    MemberWorktree,
    SymlinkEscape,
}

/// 单个变更的完整证据：路径、前后 digest、链接目标与分类。
///
/// **口径（REQ-AUD-01，2026-11-03 块2）**：元数据差分法只对变更清单里的
/// 文件读「现在」的内容 → `after_digest` 有值（新增/修改的文件），
/// `before_digest` 恒为 `None`（读旧内容就得读完整个仓，等于退回旧口径）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecipeObservedChange {
    pub path: String,
    pub change_kind: RootRecipeChangeKind,
    pub classification: RootRecipeChangeClass,
    pub allowed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
    #[serde(default)]
    pub escapes_root: bool,
}

// ---------------------------------------------------------------------------
// Command receipt
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootRecipeCommandVerdict {
    Allowed,
    Rejected,
}

/// `before_command` 返回的命令观察句柄：冻结 operation/root/step/command
/// 与 before 快照，`after_command` 消费它产出 receipt。
///
/// `before_meta` 是第一遍全量扫描的**元数据**（路径 + 大小 + mtime）——
/// 第二遍扫描后与它差分出变更清单（REQ-AUD-01）。它不落盘（watch 不
/// 序列化），只在审计窗口内传递。
#[derive(Debug, Clone)]
pub struct RootRecipeCommandWatch {
    pub operation_id: String,
    pub canonical_root: PathBuf,
    pub step: AggregateInitializationStepKind,
    pub command_index: usize,
    pub command: String,
    pub before: RootRecipeFilesystemSnapshot,
    pub before_meta: Vec<SnapshotMeta>,
}

/// 单条 root recipe 命令的审计事实。拒绝也产出 receipt（verdict=
/// `Rejected` + 证据），由调用方 append 到 store durable 保留——允许
/// 记录，不允许静默。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecipeCommandReceipt {
    pub operation_id: String,
    pub canonical_root: PathBuf,
    pub step: AggregateInitializationStepKind,
    pub command_index: usize,
    pub command: String,
    pub allowlist: Vec<String>,
    pub before_snapshot: RootRecipeFilesystemSnapshot,
    pub after_snapshot: RootRecipeFilesystemSnapshot,
    pub observed_changes: Vec<RootRecipeObservedChange>,
    /// allowlist 作用域内逃逸 symlink 的路径（即使未被本命令改动也拒绝：
    /// recipe 自有子树内不允许存在逃逸）。
    pub escape_evidence: Vec<String>,
    pub verdict: RootRecipeCommandVerdict,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rejection_reason: Option<String>,
    pub recorded_at: String,
}

/// 最终 receipt 中的单命令摘要。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecipeCommandSummary {
    pub command_index: usize,
    pub step: AggregateInitializationStepKind,
    pub command: String,
    pub verdict: RootRecipeCommandVerdict,
    pub change_count: usize,
    pub before_snapshot_digest: String,
    pub after_snapshot_digest: String,
}

/// 整个 root recipe 的最终 receipt。两种合法形态（owner 2026-10-29）：
///
/// - 完整 recipe receipt：四命令全部审计通过（`commands` 四条，
///   `skills_initialization = None`）；
/// - 跳过式 receipt：聚合根已做过 Cadence-skills 初始化、四条命令确实
///   没必要跑（`commands` 为空，`skills_initialization` 携带 `skipped`
///   判定依据与两个判定文件摘要）。
///
/// 两种形态互斥：任一命令审计事实都与「跳过」是反证，反之亦然。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecipeReceipt {
    pub operation_id: String,
    pub canonical_root: PathBuf,
    pub commands: Vec<RootRecipeCommandSummary>,
    pub policy_digest: String,
    pub rule_digest: String,
    pub finalized_at: String,
    /// 跳过式 receipt 的判定依据；完整 recipe receipt 为 `None`（旧记录
    /// 反序列化缺省 `None`，语义不变）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skills_initialization: Option<AggregateSkillsInitialization>,
}
