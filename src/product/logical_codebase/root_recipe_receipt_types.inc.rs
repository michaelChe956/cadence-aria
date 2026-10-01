/// Root recipe 副作用审计的固定 allowlist（相对 canonical root）：聚合
/// artifact 只允许落在 `.aria/aggregate/**`（含 allowlist 前缀自身的
/// 脚手架目录），绝不扩大为整个 root，也绝不放行成员仓路径（D2）。
pub const ROOT_RECIPE_ALLOWLIST: &[&str] = &[".aria/aggregate"];

/// 根规则通用入口文件（Task 1.6，REQ-BOOT-03）：AGENTS.md 是四家 provider
/// 的通用入口；CLAUDE.md 仅是兼容副本，不进入 readiness 身份（副本策略
/// 差异不得制造伪漂移）。
pub const ROOT_RULE_ENTRY_FILE: &str = "AGENTS.md";

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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RootRecipeSnapshotEntryKind {
    File,
    Dir,
    Symlink,
}

impl RootRecipeSnapshotEntryKind {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::File => "file",
            Self::Dir => "dir",
            Self::Symlink => "symlink",
        }
    }
}

/// canonical root 下一个路径的快照事实：canonical 相对路径（正斜杠）、
/// 类型、内容 digest（文件）或链接目标（symlink）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecipeSnapshotEntry {
    pub path: String,
    pub kind: RootRecipeSnapshotEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub content_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link_target: Option<String>,
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
#[derive(Debug, Clone)]
pub struct RootRecipeCommandWatch {
    pub operation_id: String,
    pub canonical_root: PathBuf,
    pub step: AggregateInitializationStepKind,
    pub command_index: usize,
    pub command: String,
    pub before: RootRecipeFilesystemSnapshot,
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

/// 整个 root recipe 的最终 receipt：仅在四命令全部审计通过且关联
/// policy/rule digest 后由 [`RootRecipeReceiptStore::finalize`] 落盘。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RootRecipeReceipt {
    pub operation_id: String,
    pub canonical_root: PathBuf,
    pub commands: Vec<RootRecipeCommandSummary>,
    pub policy_digest: String,
    pub rule_digest: String,
    pub finalized_at: String,
}
