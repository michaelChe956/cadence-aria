//! Task 1.5（REQ-BOOT-03、D2、REQ-REG-09）：root recipe receipt 与
//! filesystem auditor——recipe 副作用的可审计生产安全边界。
//!
//! 每条 root recipe 命令执行前后，[`RootRecipeFilesystemAuditor`] 对
//! canonical 聚合根做全量快照并 diff：变更只允许落在 allowlist
//! （`.aria/aggregate/**`，canonical 相对路径）内；未知路径、成员
//! `.git`/worktree 变化、symlink 逃逸、绝对/父路径越界与不可观测写入
//! 一律 fail-closed，拒绝事实与证据随 receipt durable 保留——允许记录，
//! 不允许静默。auditor 对聚合根只读，绝不覆盖或回滚用户文件。
//!
//! [`RootRecipeReceiptStore`] 把每条命令的审计事实
//! （[`RootRecipeCommandReceipt`]）与最终 receipt（[`RootRecipeReceipt`]，
//! 仅在四命令全部审计通过且关联 policy/rule digest 后由 `finalize`
//! 落盘）持久化到 per-LC scope 的 `aggregate-recipe-receipts/`，原子写
//! 含 rename 后父目录 fsync，用户冲突不覆盖。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id};

use super::aggregate_initialization::AggregateInitializationStepKind;
use super::aggregate_initialization_store::root_recipe_command_index;

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

// ---------------------------------------------------------------------------
// Filesystem auditor
// ---------------------------------------------------------------------------

/// Root recipe 文件系统审计器：命令前后对 canonical root 做全量快照并
/// diff。对 root 只读；任何 IO 失败（不可观测）fail-closed 报错。
#[derive(Debug, Clone)]
pub struct RootRecipeFilesystemAuditor {
    allowlist: Vec<String>,
}

impl Default for RootRecipeFilesystemAuditor {
    fn default() -> Self {
        Self::new()
    }
}

impl RootRecipeFilesystemAuditor {
    pub fn new() -> Self {
        Self {
            allowlist: default_allowlist(),
        }
    }

    /// 本次审计使用的 allowlist（canonical 相对路径前缀）。
    pub fn allowlist(&self) -> &[String] {
        &self.allowlist
    }

    /// 命令执行前：冻结 canonical root 并做全量快照。root 不存在、不可
    /// 读或命令身份不符合固定命令索引时 fail-closed。
    pub fn before_command(
        &self,
        operation_id: &str,
        canonical_root: &Path,
        step: AggregateInitializationStepKind,
        command_index: usize,
        command: &str,
    ) -> Result<RootRecipeCommandWatch, ProductStoreError> {
        validate_relative_id(operation_id)?;
        validate_command_identity(step, command_index, command)?;
        let canonical = canonical_root.canonicalize().map_err(|error| {
            ProductStoreError::Io(format!(
                "audit canonicalize {}: {error}",
                canonical_root.display()
            ))
        })?;
        if !canonical.is_dir() {
            return Err(ProductStoreError::Io(format!(
                "audit root {} is not a directory",
                canonical.display()
            )));
        }
        let before = snapshot_root(&canonical)?;
        Ok(RootRecipeCommandWatch {
            operation_id: operation_id.to_string(),
            canonical_root: canonical,
            step,
            command_index,
            command: command.to_string(),
            before,
        })
    }

    /// 命令执行后：再次快照并 diff。拒绝不报错——拒绝事实（verdict=
    /// `Rejected`、rejection_reason、逐变更证据、逃逸证据）随返回的
    /// receipt 保留，由调用方 durable append；只有不可观测的 IO/身份
    /// 漂移才 fail-closed 报错。
    pub fn after_command(
        &self,
        watch: RootRecipeCommandWatch,
        recorded_at: String,
    ) -> Result<RootRecipeCommandReceipt, ProductStoreError> {
        if recorded_at.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "root_recipe_command_receipt",
                reason: "recorded_at must not be empty".to_string(),
            });
        }
        let canonical = watch.canonical_root.canonicalize().map_err(|error| {
            ProductStoreError::Io(format!(
                "audit re-canonicalize {}: {error}",
                watch.canonical_root.display()
            ))
        })?;
        if canonical != watch.canonical_root {
            return Err(ProductStoreError::InvalidRecord {
                kind: "root_recipe_command_receipt",
                reason: format!(
                    "canonical root identity drifted: {} != {}",
                    canonical.display(),
                    watch.canonical_root.display()
                ),
            });
        }
        let after = snapshot_root(&canonical)?;

        let before_map: BTreeMap<&str, &RootRecipeSnapshotEntry> = watch
            .before
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry))
            .collect();
        let after_map: BTreeMap<&str, &RootRecipeSnapshotEntry> = after
            .entries
            .iter()
            .map(|entry| (entry.path.as_str(), entry))
            .collect();
        let mut paths: BTreeSet<&str> = before_map.keys().copied().collect();
        paths.extend(after_map.keys().copied());

        let mut observed_changes = Vec::new();
        for path in paths {
            let before_entry = before_map.get(path).copied();
            let after_entry = after_map.get(path).copied();
            let change_kind = match (before_entry, after_entry) {
                (Some(_), None) => RootRecipeChangeKind::Removed,
                (None, Some(_)) => RootRecipeChangeKind::Added,
                (Some(before), Some(after_entry)) => {
                    if entries_equivalent(before, after_entry) {
                        continue;
                    }
                    RootRecipeChangeKind::Modified
                }
                (None, None) => continue,
            };
            let (classification, escapes_root, link_target) =
                classify_change(path, before_entry, after_entry, &self.allowlist);
            let allowed = classification == RootRecipeChangeClass::AllowlistedArtifact;
            observed_changes.push(RootRecipeObservedChange {
                path: path.to_string(),
                change_kind,
                classification,
                allowed,
                before_digest: before_entry.and_then(|entry| entry.content_digest.clone()),
                after_digest: after_entry.and_then(|entry| entry.content_digest.clone()),
                link_target: link_target.or_else(|| {
                    before_entry
                        .and_then(|entry| entry.link_target.clone())
                        .or_else(|| after_entry.and_then(|entry| entry.link_target.clone()))
                }),
                escapes_root,
            });
        }

        // allowlist 作用域内的逃逸 symlink（即使未变化）一律作为证据拒绝。
        let mut escape_evidence: BTreeSet<String> = BTreeSet::new();
        for snapshot in [&watch.before, &after] {
            for entry in &snapshot.entries {
                if entry.kind == RootRecipeSnapshotEntryKind::Symlink
                    && entry.escapes_root
                    && allowlist_scope_contains(&entry.path, &self.allowlist)
                {
                    escape_evidence.insert(entry.path.clone());
                }
            }
        }

        let verdict =
            if observed_changes.iter().all(|change| change.allowed) && escape_evidence.is_empty() {
                RootRecipeCommandVerdict::Allowed
            } else {
                RootRecipeCommandVerdict::Rejected
            };
        let rejection_reason = match verdict {
            RootRecipeCommandVerdict::Allowed => None,
            RootRecipeCommandVerdict::Rejected => {
                Some(rejection_summary(&observed_changes, &escape_evidence))
            }
        };

        Ok(RootRecipeCommandReceipt {
            operation_id: watch.operation_id,
            canonical_root: canonical,
            step: watch.step,
            command_index: watch.command_index,
            command: watch.command,
            allowlist: self.allowlist.clone(),
            before_snapshot: watch.before,
            after_snapshot: after,
            observed_changes,
            escape_evidence: escape_evidence.into_iter().collect(),
            verdict,
            rejection_reason,
            recorded_at,
        })
    }
}

// ---------------------------------------------------------------------------
// Receipt store
// ---------------------------------------------------------------------------

/// Per-LC scoped durable receipt store（与
/// `AggregateInitializationOperationStore::for_lc` 同一布局规则）：
/// `aggregate-recipe-receipts/{operation_id}/commands/{index}.json` 保存
/// 每命令审计事实，`aggregate-recipe-receipts/{operation_id}.json` 保存
/// 最终 receipt。变更在 scope 级 `.aggregate-recipe-receipt.lock` 上
/// 串行化；冲突不覆盖。
#[derive(Debug, Clone)]
pub struct RootRecipeReceiptStore {
    paths: ProductAppPaths,
    lc_id: Option<String>,
}

impl RootRecipeReceiptStore {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths, lc_id: None }
    }

    /// Scopes receipt facts to one logical codebase subtree（v1.3 per-LC
    /// 布局；legacy 别名 codebase 保持 legacy project-scoped root）。
    pub fn for_lc(paths: ProductAppPaths, lc_id: impl Into<String>) -> Self {
        Self {
            paths,
            lc_id: Some(lc_id.into()),
        }
    }

    fn scope_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(project_id)?;
        crate::product::logical_codebase::lc_scope_root(&self.paths, project_id, &self.lc_id)
    }

    fn receipts_root(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .scope_root(project_id)?
            .join("aggregate-recipe-receipts"))
    }

    fn lock_path(&self, project_id: &str) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .scope_root(project_id)?
            .join(".aggregate-recipe-receipt.lock"))
    }

    fn receipt_path(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(operation_id)?;
        Ok(self
            .receipts_root(project_id)?
            .join(format!("{operation_id}.json")))
    }

    fn commands_root(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(operation_id)?;
        Ok(self
            .receipts_root(project_id)?
            .join(operation_id)
            .join("commands"))
    }

    fn command_receipt_path(
        &self,
        project_id: &str,
        operation_id: &str,
        command_index: usize,
    ) -> Result<PathBuf, ProductStoreError> {
        Ok(self
            .commands_root(project_id, operation_id)?
            .join(format!("{command_index:02}.json")))
    }

    /// 追加一条命令审计事实（含拒绝——拒绝也要 durable 保留证据）。同
    /// command_index 同内容重放幂等；同 index 不同内容冲突不覆盖。
    pub fn append_command(
        &self,
        project_id: &str,
        receipt: RootRecipeCommandReceipt,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(project_id)?;
        self.validate_command_receipt(&receipt)?;
        with_exact_exclusive_lock(&self.lock_path(project_id)?, || {
            let path = self.command_receipt_path(
                project_id,
                &receipt.operation_id,
                receipt.command_index,
            )?;
            if path.exists() {
                let existing: RootRecipeCommandReceipt = read_json(&path)?;
                if existing == receipt {
                    return Ok(());
                }
                return Err(receipt_conflict(
                    &receipt.operation_id,
                    receipt.command_index,
                ));
            }
            write_receipt_durable(&path, &receipt)
        })
    }

    /// Lists the operation's command audit facts, oldest command first.
    pub fn list_commands(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<Vec<RootRecipeCommandReceipt>, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        let root = self.commands_root(project_id, operation_id)?;
        if !root.exists() {
            return Ok(Vec::new());
        }
        let mut receipts = Vec::new();
        for entry in std::fs::read_dir(&root)
            .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", root.display())))?
        {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", root.display()))
            })?;
            let path = entry.path();
            if path.extension().and_then(|value| value.to_str()) != Some("json") {
                continue;
            }
            let receipt: RootRecipeCommandReceipt = read_json(&path)?;
            if receipt.operation_id != operation_id {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "root_recipe_command_receipt",
                    reason: format!(
                        "receipt at {} does not belong to operation {operation_id}",
                        path.display()
                    ),
                });
            }
            receipts.push(receipt);
        }
        receipts.sort_by_key(|receipt| receipt.command_index);
        Ok(receipts)
    }

    /// 产出最终 receipt：只接受固定命令索引的四条命令全部审计通过
    ///（verdict `Allowed`、身份一致、canonical root 唯一）且 policy/rule
    /// digest 非空；否则 fail-closed。同参数重放幂等；目标文件已有不同
    /// 内容（用户冲突）冲突不覆盖。
    pub fn finalize(
        &self,
        project_id: &str,
        operation_id: &str,
        policy_digest: &str,
        rule_digest: &str,
        finalized_at: String,
    ) -> Result<RootRecipeReceipt, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(operation_id)?;
        if finalized_at.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "root_recipe_receipt",
                reason: "finalized_at must not be empty".to_string(),
            });
        }
        if policy_digest.trim().is_empty() || rule_digest.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "root_recipe_receipt",
                reason: "policy/rule digest must be present to freeze receipt identity".to_string(),
            });
        }
        with_exact_exclusive_lock(&self.lock_path(project_id)?, || {
            let receipts = self.list_commands(project_id, operation_id)?;
            if receipts.len() != root_recipe_command_index().len() {
                return Err(ProductStoreError::InvalidRecord {
                    kind: "root_recipe_receipt",
                    reason: format!(
                        "operation {operation_id} has {} audited command receipt(s); \
                         the frozen root recipe requires exactly four",
                        receipts.len()
                    ),
                });
            }
            let mut canonical_root: Option<PathBuf> = None;
            let mut commands = Vec::new();
            for (step, command_index, command) in root_recipe_command_index() {
                let receipt = receipts
                    .iter()
                    .find(|receipt| receipt.command_index == command_index)
                    .ok_or_else(|| ProductStoreError::InvalidRecord {
                        kind: "root_recipe_receipt",
                        reason: format!(
                            "command {command_index} ({command}) has no audited receipt"
                        ),
                    })?;
                if receipt.step != step || receipt.command != command {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "root_recipe_receipt",
                        reason: format!(
                            "command {command_index} receipt identity does not match \
                             the frozen root recipe command index"
                        ),
                    });
                }
                if receipt.operation_id != operation_id {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "root_recipe_receipt",
                        reason: format!(
                            "command {command_index} receipt belongs to operation {}",
                            receipt.operation_id
                        ),
                    });
                }
                if receipt.verdict != RootRecipeCommandVerdict::Allowed {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "root_recipe_receipt",
                        reason: format!(
                            "command {command_index} ({command}) was audited {:?}; \
                             a rejected recipe cannot be finalized",
                            receipt.verdict
                        ),
                    });
                }
                match &canonical_root {
                    None => canonical_root = Some(receipt.canonical_root.clone()),
                    Some(root) if root != &receipt.canonical_root => {
                        return Err(ProductStoreError::InvalidRecord {
                            kind: "root_recipe_receipt",
                            reason: format!(
                                "command {command_index} audited a different canonical \
                                 root: {} != {}",
                                root.display(),
                                receipt.canonical_root.display()
                            ),
                        });
                    }
                    Some(_) => {}
                }
                commands.push(RootRecipeCommandSummary {
                    command_index,
                    step,
                    command: command.to_string(),
                    verdict: receipt.verdict,
                    change_count: receipt.observed_changes.len(),
                    before_snapshot_digest: receipt.before_snapshot.snapshot_digest.clone(),
                    after_snapshot_digest: receipt.after_snapshot.snapshot_digest.clone(),
                });
            }
            let receipt = RootRecipeReceipt {
                operation_id: operation_id.to_string(),
                canonical_root: canonical_root
                    .expect("the frozen command index always yields a canonical root"),
                commands,
                policy_digest: policy_digest.to_string(),
                rule_digest: rule_digest.to_string(),
                finalized_at,
            };
            let path = self.receipt_path(project_id, operation_id)?;
            if path.exists() {
                let existing: RootRecipeReceipt = read_json(&path)?;
                if existing == receipt {
                    return Ok(existing);
                }
                return Err(ProductStoreError::Conflict {
                    kind: "root_recipe_receipt",
                    id: operation_id.to_string(),
                });
            }
            write_receipt_durable(&path, &receipt)?;
            Ok(receipt)
        })
    }

    /// 读取最终 receipt（纯投影，零副作用）。identity 漂移 fail-closed。
    pub fn get(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<Option<RootRecipeReceipt>, ProductStoreError> {
        let path = self.receipt_path(project_id, operation_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let existing: RootRecipeReceipt = read_json(&path)?;
        if existing.operation_id != operation_id {
            return Err(ProductStoreError::InvalidRecord {
                kind: "root_recipe_receipt",
                reason: format!(
                    "receipt at {} does not belong to operation {operation_id}",
                    path.display()
                ),
            });
        }
        Ok(Some(existing))
    }

    /// Durable 边界上的 receipt 事实校验：命令身份符合固定索引、allowlist
    /// 形状安全（绝不扩大为整个 root）、快照与 canonical root 自洽、
    /// verdict 与拒绝原因一致。
    fn validate_command_receipt(
        &self,
        receipt: &RootRecipeCommandReceipt,
    ) -> Result<(), ProductStoreError> {
        validate_relative_id(&receipt.operation_id)?;
        validate_command_identity(receipt.step, receipt.command_index, &receipt.command)?;
        validate_allowlist(&receipt.allowlist)?;
        if !receipt.canonical_root.is_absolute() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "root_recipe_command_receipt",
                reason: format!(
                    "canonical root {} must be an absolute canonical path",
                    receipt.canonical_root.display()
                ),
            });
        }
        if receipt.before_snapshot.canonical_root != receipt.canonical_root
            || receipt.after_snapshot.canonical_root != receipt.canonical_root
        {
            return Err(ProductStoreError::InvalidRecord {
                kind: "root_recipe_command_receipt",
                reason: "snapshot canonical root does not match the receipt".to_string(),
            });
        }
        if receipt.recorded_at.trim().is_empty() {
            return Err(ProductStoreError::InvalidRecord {
                kind: "root_recipe_command_receipt",
                reason: "recorded_at must not be empty".to_string(),
            });
        }
        match receipt.verdict {
            RootRecipeCommandVerdict::Allowed => {
                if receipt.rejection_reason.is_some() {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "root_recipe_command_receipt",
                        reason: "allowed receipt must not carry a rejection reason".to_string(),
                    });
                }
            }
            RootRecipeCommandVerdict::Rejected => {
                if receipt
                    .rejection_reason
                    .as_deref()
                    .map(str::trim)
                    .unwrap_or("")
                    .is_empty()
                {
                    return Err(ProductStoreError::InvalidRecord {
                        kind: "root_recipe_command_receipt",
                        reason: "rejected receipt must carry a rejection reason".to_string(),
                    });
                }
            }
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Validation & audit helpers
// ---------------------------------------------------------------------------

/// 命令身份必须命中固定命令索引（auditor 与 durable store 双侧校验）。
fn validate_command_identity(
    step: AggregateInitializationStepKind,
    command_index: usize,
    command: &str,
) -> Result<(), ProductStoreError> {
    let matches = root_recipe_command_index().into_iter().any(
        |(expected_step, expected_index, expected_command)| {
            expected_step == step && expected_index == command_index && expected_command == command
        },
    );
    if matches {
        return Ok(());
    }
    Err(ProductStoreError::InvalidRecord {
        kind: "root_recipe_command_receipt",
        reason: format!(
            "command {command_index} ({command}) on step {} does not match the frozen \
             root recipe command index",
            step.as_str()
        ),
    })
}

/// Allowlist 形状校验：非空、每条都是安全相对子路径（拒绝绝对路径、
/// `..`、`.`、空——allowlist 不得扩大为整个 root）。
fn validate_allowlist(allowlist: &[String]) -> Result<(), ProductStoreError> {
    if allowlist.is_empty() {
        return Err(ProductStoreError::InvalidRecord {
            kind: "root_recipe_command_receipt",
            reason: "allowlist must not be empty".to_string(),
        });
    }
    for entry in allowlist {
        if entry.trim().is_empty() || entry == "." {
            return Err(ProductStoreError::InvalidRecord {
                kind: "root_recipe_command_receipt",
                reason: format!(
                    "allowlist entry {entry:?} must be a proper subpath, not the whole root"
                ),
            });
        }
        let path = Path::new(entry);
        if path.is_absolute()
            || path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir
                        | Component::RootDir
                        | Component::Prefix(_)
                        | Component::CurDir
                )
            })
        {
            return Err(ProductStoreError::PathEscape(entry.clone()));
        }
    }
    Ok(())
}

/// path 是否落在 allowlist 作用域内：等于某个前缀、位于前缀之下，或
/// 是前缀自身的祖先脚手架目录（recipe 需要能创建 `.aria`/
/// `.aria/aggregate` 脚手架）。
fn allowlist_scope_contains(path: &str, allowlist: &[String]) -> bool {
    allowlist.iter().any(|prefix| {
        if path == prefix || path.starts_with(&format!("{prefix}/")) {
            return true;
        }
        let mut ancestor = String::new();
        for component in prefix.split('/') {
            if !ancestor.is_empty() {
                ancestor.push('/');
            }
            ancestor.push_str(component);
            if ancestor == path {
                return true;
            }
        }
        false
    })
}

/// 成员 `.git`/worktree 判定：路径任一段为 `.git` 即成员 Git 面；其下
/// 再出现 `worktrees` 段为成员 worktree 面（分类优先于 allowlist：任何
/// 位置出现的 `.git` 都不因位于 allowlist 内而放行）。
fn classify_member_git(path: &str) -> Option<RootRecipeChangeClass> {
    let mut saw_git = false;
    let mut saw_worktrees = false;
    for component in path.split('/') {
        if component == ".git" {
            saw_git = true;
            saw_worktrees = false;
        } else if saw_git && component == "worktrees" {
            saw_worktrees = true;
        }
    }
    if saw_worktrees {
        Some(RootRecipeChangeClass::MemberWorktree)
    } else if saw_git {
        Some(RootRecipeChangeClass::MemberGit)
    } else {
        None
    }
}

fn classify_change<'a>(
    path: &str,
    before: Option<&'a RootRecipeSnapshotEntry>,
    after: Option<&'a RootRecipeSnapshotEntry>,
    allowlist: &[String],
) -> (RootRecipeChangeClass, bool, Option<String>) {
    let symlink_entry = [before, after]
        .into_iter()
        .flatten()
        .find(|entry| entry.kind == RootRecipeSnapshotEntryKind::Symlink);
    let escapes_root = symlink_entry.is_some_and(|entry| entry.escapes_root);
    let link_target = symlink_entry.and_then(|entry| entry.link_target.clone());

    if escapes_root && allowlist_scope_contains(path, allowlist) {
        return (
            RootRecipeChangeClass::SymlinkEscape,
            escapes_root,
            link_target,
        );
    }
    if let Some(class) = classify_member_git(path) {
        return (class, escapes_root, link_target);
    }
    if allowlist_scope_contains(path, allowlist) {
        return (
            RootRecipeChangeClass::AllowlistedArtifact,
            escapes_root,
            link_target,
        );
    }
    (
        RootRecipeChangeClass::UnknownPath,
        escapes_root,
        link_target,
    )
}

fn entries_equivalent(before: &RootRecipeSnapshotEntry, after: &RootRecipeSnapshotEntry) -> bool {
    before.kind == after.kind
        && before.content_digest == after.content_digest
        && before.link_target == after.link_target
}

fn rejection_summary(
    changes: &[RootRecipeObservedChange],
    escape_evidence: &BTreeSet<String>,
) -> String {
    let count = |class: RootRecipeChangeClass| {
        changes
            .iter()
            .filter(|change| change.classification == class)
            .count()
    };
    let mut summary = format!(
        "rejected: {} disallowed change(s) (unknown_path={}, member_git={}, \
         member_worktree={}, symlink_escape={})",
        changes.iter().filter(|change| !change.allowed).count(),
        count(RootRecipeChangeClass::UnknownPath),
        count(RootRecipeChangeClass::MemberGit),
        count(RootRecipeChangeClass::MemberWorktree),
        count(RootRecipeChangeClass::SymlinkEscape),
    );
    if !escape_evidence.is_empty() {
        summary.push_str(&format!(
            "; symlink escape evidence: {}",
            escape_evidence
                .iter()
                .cloned()
                .collect::<Vec<String>>()
                .join(", ")
        ));
    }
    summary
}

fn snapshot_root(canonical_root: &Path) -> Result<RootRecipeFilesystemSnapshot, ProductStoreError> {
    let mut entries = Vec::new();
    walk_root(canonical_root, canonical_root, &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let snapshot_digest = digest_snapshot_entries(&entries);
    Ok(RootRecipeFilesystemSnapshot {
        canonical_root: canonical_root.to_path_buf(),
        entries,
        snapshot_digest,
    })
}

/// 全量递归快照：不跟随 symlink；目录不可读、文件不可读、特殊文件类型
/// 均视为不可观测而 fail-closed 报错（绝不静默跳过）。
fn walk_root(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<RootRecipeSnapshotEntry>,
) -> Result<(), ProductStoreError> {
    let reader = std::fs::read_dir(dir)
        .map_err(|error| ProductStoreError::Io(format!("audit read {}: {error}", dir.display())))?;
    for entry in reader {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("audit read {} entry: {error}", dir.display()))
        })?;
        let path = entry.path();
        let relative = relative_audit_path(root, &path)?;
        let file_type = entry.file_type().map_err(|error| {
            ProductStoreError::Io(format!("audit stat {}: {error}", path.display()))
        })?;
        if file_type.is_symlink() {
            let target = std::fs::read_link(&path).map_err(|error| {
                ProductStoreError::Io(format!("audit readlink {}: {error}", path.display()))
            })?;
            entries.push(RootRecipeSnapshotEntry {
                path: relative,
                kind: RootRecipeSnapshotEntryKind::Symlink,
                content_digest: None,
                link_target: Some(target.to_string_lossy().into_owned()),
                escapes_root: symlink_escapes_root(root, &path, &target),
            });
        } else if file_type.is_dir() {
            entries.push(RootRecipeSnapshotEntry {
                path: relative,
                kind: RootRecipeSnapshotEntryKind::Dir,
                content_digest: None,
                link_target: None,
                escapes_root: false,
            });
            walk_root(root, &path, entries)?;
        } else if file_type.is_file() {
            let bytes = std::fs::read(&path).map_err(|error| {
                ProductStoreError::Io(format!("audit read {}: {error}", path.display()))
            })?;
            entries.push(RootRecipeSnapshotEntry {
                path: relative,
                kind: RootRecipeSnapshotEntryKind::File,
                content_digest: Some(digest_bytes(&bytes)),
                link_target: None,
                escapes_root: false,
            });
        } else {
            return Err(ProductStoreError::Io(format!(
                "audit cannot observe unsupported entry type at {} (fail-closed)",
                path.display()
            )));
        }
    }
    Ok(())
}

fn relative_audit_path(root: &Path, path: &Path) -> Result<String, ProductStoreError> {
    let relative = path.strip_prefix(root).map_err(|_| {
        ProductStoreError::PathEscape(format!(
            "{} escapes audit root {}",
            path.display(),
            root.display()
        ))
    })?;
    let mut result = String::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => {
                if !result.is_empty() {
                    result.push('/');
                }
                result.push_str(&value.to_string_lossy());
            }
            other => {
                return Err(ProductStoreError::PathEscape(format!(
                    "unexpected component {other:?} under audit root {}",
                    root.display()
                )));
            }
        }
    }
    Ok(result)
}

/// symlink 是否词法逃逸 canonical root：以链接父目录为基点解析目标
///（不触盘），消解 `.`/`..` 后不在 root 之下即逃逸；绝对目标同理。
fn symlink_escapes_root(root: &Path, link_path: &Path, target: &Path) -> bool {
    let base = link_path.parent().unwrap_or_else(|| Path::new("/"));
    let joined = if target.is_absolute() {
        target.to_path_buf()
    } else {
        base.join(target)
    };
    let mut normalized = PathBuf::new();
    for component in joined.components() {
        match component {
            Component::ParentDir => {
                normalized.pop();
            }
            Component::CurDir => {}
            other => normalized.push(other.as_os_str()),
        }
    }
    !normalized.starts_with(root)
}

fn digest_snapshot_entries(entries: &[RootRecipeSnapshotEntry]) -> String {
    let mut hasher = Sha256::new();
    for entry in entries {
        hasher.update(entry.path.as_bytes());
        hasher.update([0]);
        hasher.update(entry.kind.as_str().as_bytes());
        hasher.update([0]);
        hasher.update(entry.content_digest.as_deref().unwrap_or("-").as_bytes());
        hasher.update([0]);
        hasher.update(entry.link_target.as_deref().unwrap_or("-").as_bytes());
        hasher.update([0]);
        hasher.update(if entry.escapes_root { b"1" } else { b"0" });
        hasher.update(b"\n");
    }
    format!("sha256:{:x}", hasher.finalize())
}

fn digest_bytes(content: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(content))
}

/// Task 1.6（REQ-BOOT-03）：canonical root 根规则材料的只读摘要——receipt
/// `finalize`（生产接线 Task 1.8）与 bootstrap readiness 投影共用此函数，
/// 保证两侧 rule digest 语义唯一。`Ok(None)` 表示根规则入口尚不存在
///（recipe 未生成或被移除）；不可读 fail-closed，绝不静默当作缺失。
pub fn root_rule_digest(canonical_root: &Path) -> Result<Option<String>, ProductStoreError> {
    let entry = canonical_root.join(ROOT_RULE_ENTRY_FILE);
    match std::fs::read(&entry) {
        Ok(bytes) => Ok(Some(digest_bytes(&bytes))),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(ProductStoreError::Io(format!(
            "read root rule {}: {error}",
            entry.display()
        ))),
    }
}

fn receipt_conflict(operation_id: &str, command_index: usize) -> ProductStoreError {
    ProductStoreError::Conflict {
        kind: "root_recipe_command_receipt",
        id: format!("{operation_id}:{command_index}"),
    }
}

/// 原子 durable 写（Task 1.3 教训）：create 临时文件 → 写 JSON → flush →
/// `sync_all` → rename → **父目录 fsync**（`json_store::write_json` 缺最后
/// 一步，本模块补齐）。失败清理临时文件，目标文件绝不被部分覆盖。
fn write_receipt_durable<T: Serialize>(path: &Path, value: &T) -> Result<(), ProductStoreError> {
    use std::io::Write;

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }

    let temp_path = receipt_temp_path(path);
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp_path)
        .map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", temp_path.display()))
        })?;
    let write_result = (|| {
        serde_json::to_writer_pretty(&mut file, value).map_err(|error| {
            ProductStoreError::Json(format!(
                "write {} via {}: {error}",
                path.display(),
                temp_path.display()
            ))
        })?;
        file.flush().map_err(|error| {
            ProductStoreError::Io(format!("flush {}: {error}", temp_path.display()))
        })?;
        file.sync_all().map_err(|error| {
            ProductStoreError::Io(format!("sync {}: {error}", temp_path.display()))
        })
    })();
    drop(file);
    write_result?;

    if let Err(error) = std::fs::rename(&temp_path, path) {
        let cleanup = std::fs::remove_file(&temp_path)
            .err()
            .map(|cleanup_error| format!("; cleanup failed: {cleanup_error}"))
            .unwrap_or_default();
        return Err(ProductStoreError::Io(format!(
            "rename {} to {}: {error}{cleanup}",
            temp_path.display(),
            path.display()
        )));
    }
    // rename 的目录项变更本身也要落盘（1.3 fsync 缺口补齐）。
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        let dir = std::fs::File::open(parent).map_err(|error| {
            ProductStoreError::Io(format!("open {}: {error}", parent.display()))
        })?;
        dir.sync_all().map_err(|error| {
            ProductStoreError::Io(format!("fsync {}: {error}", parent.display()))
        })?;
    }
    Ok(())
}

fn receipt_temp_path(path: &Path) -> PathBuf {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = path
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "root-recipe-receipt.json".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nanos))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::json_store::{ProductStoreError, read_json};
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepKind;
    use crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index;

    const PROJECT_ID: &str = "project_0001";
    const LC_ID: &str = "logical_codebase_0001";
    const OPERATION_ID: &str = "aggregate_initialization_0001";
    const RECORDED_AT: &str = "2026-10-01T00:00:00Z";
    const POLICY_DIGEST: &str = "sha256:policy-1";
    const RULE_DIGEST: &str = "sha256:rule-1";

    /// 非 Git 聚合根 + 一个成员仓（含 `.git/HEAD`）的最小 fixture；receipt
    /// store 以 per-LC scope 构造（与 1.4 的 store 同一布局规则）。
    struct ReceiptFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        store: RootRecipeReceiptStore,
        root: std::path::PathBuf,
    }

    impl ReceiptFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let paths = ProductAppPaths::new(temp.path());
            let store = RootRecipeReceiptStore::for_lc(paths.clone(), LC_ID);
            let raw_root = temp.path().join("aggregate-root");
            std::fs::create_dir_all(raw_root.join(".aria/aggregate")).unwrap();
            std::fs::create_dir_all(raw_root.join("members/repo-a/.git")).unwrap();
            std::fs::write(
                raw_root.join("members/repo-a/.git/HEAD"),
                "ref: refs/heads/main\n",
            )
            .unwrap();
            std::fs::write(raw_root.join("members/repo-a/README.md"), "# member\n").unwrap();
            let root = raw_root.canonicalize().unwrap();
            Self {
                _temp: temp,
                paths,
                store,
                root,
            }
        }

        /// 以「只写 allowlist 内一个新 artifact」的方式跑一条命令的完整审计。
        fn run_allowed_command(
            &self,
            auditor: &RootRecipeFilesystemAuditor,
            command_index: usize,
            artifact_relative: &str,
            artifact_content: &str,
        ) -> RootRecipeCommandReceipt {
            let (step, command) = command_spec(command_index);
            let watch =
                auditor.before_command(OPERATION_ID, &self.root, step, command_index, command);
            let watch = watch.unwrap();
            std::fs::write(
                self.root.join(".aria/aggregate").join(artifact_relative),
                artifact_content,
            )
            .unwrap();
            auditor
                .after_command(watch, RECORDED_AT.to_string())
                .unwrap()
        }
    }

    /// 固定命令索引的第 command_index 条（1..=4）。
    fn command_spec(command_index: usize) -> (AggregateInitializationStepKind, &'static str) {
        let (step, _, command) = root_recipe_command_index()[command_index - 1];
        (step, command)
    }

    fn change_of<'a>(
        receipt: &'a RootRecipeCommandReceipt,
        path: &str,
    ) -> (&'a RootRecipeChangeClass, bool) {
        let change = receipt
            .observed_changes
            .iter()
            .find(|change| change.path == path)
            .unwrap_or_else(|| panic!("no observed change for {path}"));
        (&change.classification, change.allowed)
    }

    #[test]
    fn receipt_auditor_rejects_unknown_paths_and_symlink_escape() {
        let fixture = ReceiptFixture::new();
        let auditor = RootRecipeFilesystemAuditor::new();

        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();

        // allowlist 内新 artifact：唯一合法变更。
        std::fs::write(
            fixture.root.join(".aria/aggregate/precheck-report.json"),
            "{\"ok\":true}",
        )
        .unwrap();
        // 根级未知路径（allowlist 之外）。
        std::fs::write(fixture.root.join("AGENTS.md"), "# user file\n").unwrap();
        // 成员 `.git` 变化。
        std::fs::write(
            fixture.root.join("members/repo-a/.git/HEAD"),
            "ref: refs/heads/changed\n",
        )
        .unwrap();
        // 成员 worktree 变化。
        std::fs::create_dir_all(fixture.root.join("members/repo-a/.git/worktrees/wt")).unwrap();
        std::fs::write(
            fixture.root.join("members/repo-a/.git/worktrees/wt/HEAD"),
            "abc123\n",
        )
        .unwrap();
        // allowlist 内指向根之外的 symlink（逃逸）。
        std::os::unix::fs::symlink("../../..", fixture.root.join(".aria/aggregate/escape-link"))
            .unwrap();

        let receipt = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Rejected);
        assert!(
            !receipt
                .rejection_reason
                .as_deref()
                .unwrap_or_default()
                .is_empty()
        );

        let (class, allowed) = change_of(&receipt, ".aria/aggregate/precheck-report.json");
        assert_eq!(class, &RootRecipeChangeClass::AllowlistedArtifact);
        assert!(allowed, "allowlisted artifact change must be allowed");

        let (class, allowed) = change_of(&receipt, "AGENTS.md");
        assert_eq!(class, &RootRecipeChangeClass::UnknownPath);
        assert!(!allowed, "unknown root-level path must be rejected");

        let (class, allowed) = change_of(&receipt, "members/repo-a/.git/HEAD");
        assert_eq!(class, &RootRecipeChangeClass::MemberGit);
        assert!(!allowed, "member .git change must be rejected");

        let (class, allowed) = change_of(&receipt, "members/repo-a/.git/worktrees/wt/HEAD");
        assert_eq!(class, &RootRecipeChangeClass::MemberWorktree);
        assert!(!allowed, "member worktree change must be rejected");

        let (class, allowed) = change_of(&receipt, ".aria/aggregate/escape-link");
        assert_eq!(class, &RootRecipeChangeClass::SymlinkEscape);
        assert!(
            !allowed,
            "symlink escaping the canonical root must be rejected"
        );
        assert!(
            receipt
                .escape_evidence
                .iter()
                .any(|path| path == ".aria/aggregate/escape-link")
        );

        // allowlist 不扩大为整个 root。
        assert_eq!(receipt.allowlist, vec![".aria/aggregate".to_string()]);
        // auditor 只读：用户/成员文件原样保留。
        assert_eq!(
            std::fs::read_to_string(fixture.root.join("members/repo-a/README.md")).unwrap(),
            "# member\n"
        );
        assert!(fixture.root.join("AGENTS.md").exists());

        // 拒绝事实 durable 保留（允许记录，不允许静默）。
        fixture
            .store
            .append_command(PROJECT_ID, receipt.clone())
            .unwrap();
        // 任一命令被拒绝时 finalize fail-closed。
        let error = fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::InvalidRecord { .. }),
            "finalize must fail closed on a rejected command: {error:?}"
        );
    }

    #[test]
    fn receipt_records_before_after_snapshot_and_policy_rule_identity() {
        let fixture = ReceiptFixture::new();
        let auditor = RootRecipeFilesystemAuditor::new();

        let artifacts = [
            ("precheck-report.json", "{\"ok\":true}"),
            ("claude-rules.md", "# rules"),
            ("mcp.json", "{\"mcp\":true}"),
            ("openspec-examples.json", "[]"),
        ];
        for (offset, (artifact, content)) in artifacts.iter().enumerate() {
            let command_index = offset + 1;
            let (expected_step, expected_command) = command_spec(command_index);
            let receipt = fixture.run_allowed_command(&auditor, command_index, artifact, content);

            assert_eq!(receipt.verdict, RootRecipeCommandVerdict::Allowed);
            assert!(receipt.rejection_reason.is_none());
            assert_eq!(receipt.operation_id, OPERATION_ID);
            assert_eq!(receipt.canonical_root, fixture.root);
            assert_eq!(receipt.step, expected_step);
            assert_eq!(receipt.command, expected_command);
            assert_eq!(receipt.command_index, command_index);
            assert_eq!(receipt.allowlist, vec![".aria/aggregate".to_string()]);

            // before/after 快照 + digest 均被冻结。
            assert!(!receipt.before_snapshot.entries.is_empty());
            assert!(!receipt.after_snapshot.entries.is_empty());
            assert!(
                receipt
                    .before_snapshot
                    .entries
                    .iter()
                    .any(|entry| entry.path == "members/repo-a/.git/HEAD")
            );
            assert!(
                receipt
                    .before_snapshot
                    .snapshot_digest
                    .starts_with("sha256:")
            );
            assert_ne!(
                receipt.before_snapshot.snapshot_digest,
                receipt.after_snapshot.snapshot_digest
            );
            assert!(receipt.observed_changes.iter().all(|change| change.allowed));

            fixture.store.append_command(PROJECT_ID, receipt).unwrap();
        }

        let finalized = fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap();
        assert_eq!(finalized.operation_id, OPERATION_ID);
        assert_eq!(finalized.canonical_root, fixture.root);
        assert_eq!(finalized.policy_digest, POLICY_DIGEST);
        assert_eq!(finalized.rule_digest, RULE_DIGEST);
        assert_eq!(finalized.commands.len(), 4);
        let commands: Vec<&str> = finalized
            .commands
            .iter()
            .map(|summary| summary.command.as_str())
            .collect();
        assert_eq!(
            commands,
            vec![
                "/pre-check --no-interrupt --upgrade 用大陆镜像",
                "/rule-config --no-interrupt",
                "/mcp-configuration --no-interrupt",
                "/project-rules-examples --no-interrupt",
            ]
        );
        assert!(
            finalized
                .commands
                .iter()
                .all(|summary| summary.verdict == RootRecipeCommandVerdict::Allowed)
        );

        // receipt durable 写入 per-LC scope 的 aggregate 路径族。
        let receipt_path = fixture
            .paths
            .logical_codebases_root(PROJECT_ID)
            .join(LC_ID)
            .join("aggregate-recipe-receipts")
            .join(format!("{OPERATION_ID}.json"));
        assert!(receipt_path.exists());

        // get 往返 + finalize 幂等重放。
        assert_eq!(
            fixture
                .store
                .get(PROJECT_ID, OPERATION_ID)
                .unwrap()
                .unwrap(),
            finalized
        );
        let replay = fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap();
        assert_eq!(replay, finalized);
    }

    #[test]
    fn receipt_does_not_overwrite_user_conflict() {
        let fixture = ReceiptFixture::new();
        let auditor = RootRecipeFilesystemAuditor::new();

        // 命令 1 首次审计落盘。
        let first = fixture.run_allowed_command(&auditor, 1, "precheck-report.json", "v1");
        fixture
            .store
            .append_command(PROJECT_ID, first.clone())
            .unwrap();
        // 同内容重放幂等。
        fixture
            .store
            .append_command(PROJECT_ID, first.clone())
            .unwrap();

        // 同 command_index 的不同审计事实：冲突不覆盖。
        std::fs::write(
            fixture.root.join(".aria/aggregate/precheck-report.json"),
            "v2",
        )
        .unwrap();
        std::fs::write(fixture.root.join("CLAUDE-user.md"), "user content").unwrap();
        let (step, command) = command_spec(1);
        let watch = auditor
            .before_command(OPERATION_ID, &fixture.root, step, 1, command)
            .unwrap();
        let conflicting = auditor
            .after_command(watch, RECORDED_AT.to_string())
            .unwrap();
        assert_ne!(conflicting, first);
        let error = fixture
            .store
            .append_command(PROJECT_ID, conflicting)
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { .. }),
            "conflicting append must fail closed: {error:?}"
        );
        // 磁盘上的既有记录保持首次审计事实。
        let persisted: RootRecipeCommandReceipt = read_json(
            &fixture
                .store
                .command_receipt_path(PROJECT_ID, OPERATION_ID, 1)
                .unwrap(),
        )
        .unwrap();
        assert_eq!(persisted, first);

        // 其余三命令补齐并 finalize。
        for command_index in 2..=4 {
            let receipt = fixture.run_allowed_command(
                &auditor,
                command_index,
                &format!("artifact-{command_index}.json"),
                "ok",
            );
            fixture.store.append_command(PROJECT_ID, receipt).unwrap();
        }
        fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap();

        // 用户手工修改最终 receipt：再次 finalize 冲突且用户内容原样保留。
        let receipt_path = fixture
            .store
            .receipt_path(PROJECT_ID, OPERATION_ID)
            .unwrap();
        let user_text = {
            let value: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&receipt_path).unwrap()).unwrap();
            let mut user_value = value.clone();
            user_value["policy_digest"] = serde_json::json!("sha256:user-edited");
            let text = serde_json::to_string_pretty(&user_value).unwrap();
            std::fs::write(&receipt_path, &text).unwrap();
            text
        };
        let error = fixture
            .store
            .finalize(
                PROJECT_ID,
                OPERATION_ID,
                POLICY_DIGEST,
                RULE_DIGEST,
                RECORDED_AT.to_string(),
            )
            .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::Conflict { .. }),
            "finalize over a user-modified receipt must fail closed: {error:?}"
        );
        assert_eq!(std::fs::read_to_string(&receipt_path).unwrap(), user_text);
        // get 返回用户修改后的事实（不覆盖、不回滚）。
        let current = fixture
            .store
            .get(PROJECT_ID, OPERATION_ID)
            .unwrap()
            .unwrap();
        assert_eq!(current.policy_digest, "sha256:user-edited");
    }
}
