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

fn snapshot_root(
    canonical_root: &Path,
    budget: &RootRecipeSnapshotBudget,
) -> Result<RootRecipeFilesystemSnapshot, ProductStoreError> {
    let mut entries = Vec::new();
    let mut scale = SnapshotScale::default();
    walk_root(
        canonical_root,
        canonical_root,
        &mut entries,
        budget,
        &mut scale,
    )?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    let snapshot_digest = digest_snapshot_entries(&entries);
    Ok(RootRecipeFilesystemSnapshot {
        canonical_root: canonical_root.to_path_buf(),
        entries,
        snapshot_digest,
    })
}

/// 快照规模的累计计数（条目/字节），由 [`RootRecipeSnapshotBudget`] 施加
/// 上限：超限即 fail-closed，绝不静默截断观测面。
#[derive(Default)]
struct SnapshotScale {
    entries: usize,
    bytes: u64,
}

impl SnapshotScale {
    fn charge_entry(
        &mut self,
        budget: &RootRecipeSnapshotBudget,
        path: &Path,
    ) -> Result<(), ProductStoreError> {
        self.entries += 1;
        if self.entries > budget.max_entries {
            return Err(ProductStoreError::Io(format!(
                "audit snapshot budget exceeded at {}: {} entries > max_entries {} (fail-closed)",
                path.display(),
                self.entries,
                budget.max_entries
            )));
        }
        Ok(())
    }

    fn charge_bytes(
        &mut self,
        budget: &RootRecipeSnapshotBudget,
        path: &Path,
        len: u64,
    ) -> Result<(), ProductStoreError> {
        self.bytes = self.bytes.saturating_add(len);
        if self.bytes > budget.max_bytes {
            return Err(ProductStoreError::Io(format!(
                "audit snapshot budget exceeded at {}: {} cumulative bytes > max_bytes {} (fail-closed)",
                path.display(),
                self.bytes,
                budget.max_bytes
            )));
        }
        Ok(())
    }
}

/// 全量递归快照：不跟随 symlink；目录不可读、文件不可读、特殊文件类型
/// 均视为不可观测而 fail-closed 报错（绝不静默跳过）。快照规模受预算门
/// 约束（Task 1.5 carry → Task 3.4），超限同样 fail-closed。
fn walk_root(
    root: &Path,
    dir: &Path,
    entries: &mut Vec<RootRecipeSnapshotEntry>,
    budget: &RootRecipeSnapshotBudget,
    scale: &mut SnapshotScale,
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
        scale.charge_entry(budget, &path)?;
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
            walk_root(root, &path, entries, budget, scale)?;
        } else if file_type.is_file() {
            let bytes = std::fs::read(&path).map_err(|error| {
                ProductStoreError::Io(format!("audit read {}: {error}", path.display()))
            })?;
            scale.charge_bytes(budget, &path, bytes.len() as u64)?;
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
