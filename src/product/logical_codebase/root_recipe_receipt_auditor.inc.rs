// ---------------------------------------------------------------------------
// Filesystem auditor
// ---------------------------------------------------------------------------

/// Root recipe 文件系统审计器：命令前后对 canonical root 做全量快照并
/// diff。对 root 只读；任何 IO 失败（不可观测）fail-closed 报错。
#[derive(Debug, Clone)]
pub struct RootRecipeFilesystemAuditor {
    allowlist: Vec<String>,
    budget: RootRecipeSnapshotBudget,
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
            budget: RootRecipeSnapshotBudget::DEFAULT,
        }
    }

    /// 本次审计使用的 allowlist（canonical 相对路径前缀）。
    pub fn allowlist(&self) -> &[String] {
        &self.allowlist
    }

    /// Task 1.5 carry → Task 3.4：以显式快照规模预算构造（测试/运维面）；
    /// 生产接线走 [`Self::new`] 的默认预算。
    pub fn with_snapshot_budget(budget: RootRecipeSnapshotBudget) -> Self {
        Self {
            allowlist: default_allowlist(),
            budget,
        }
    }

    /// 本次审计的快照规模预算（条目/字节上限，超限 fail-closed）。
    pub fn snapshot_budget(&self) -> RootRecipeSnapshotBudget {
        self.budget
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
        let before = snapshot_root(&canonical, &self.budget)?;
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
        let after = snapshot_root(&canonical, &self.budget)?;

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
