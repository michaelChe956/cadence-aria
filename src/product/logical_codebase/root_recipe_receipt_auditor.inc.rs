// ---------------------------------------------------------------------------
// Filesystem auditor
// ---------------------------------------------------------------------------

/// Root recipe 文件系统审计器：命令前后对 canonical root 做**全量两遍**
/// 扫描并 diff。对 root 只读；任何 IO 失败（不可观测）fail-closed 报错。
///
/// 口径（REQ-AUD-01/02/03，详见 root_recipe_snapshot.inc.rs）：两遍都**只
/// 记元数据**（路径 + 大小 + mtime）不读内容；差分出变更清单后**只对变更
/// 文件**流式读内容算哈希；`.git` 子树整棵跳过；预算只对本次变更量计。
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

    /// 命令执行前：冻结 canonical root 并做**第一遍全量元数据扫描**（不读
    /// 文件内容）。root 不存在、不可读或命令身份不符合固定命令索引时
    /// fail-closed。
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
        // 第一遍：**只记元数据**（路径 + 大小 + mtime），不读文件内容
        // （REQ-AUD-01）。`.git` 子树在遍历时整棵跳过（REQ-AUD-02）。
        let before_meta = scan_meta(&canonical)?;
        let before = snapshot_from_meta(&canonical, &before_meta);
        Ok(RootRecipeCommandWatch {
            operation_id: operation_id.to_string(),
            canonical_root: canonical,
            step,
            command_index,
            command: command.to_string(),
            before,
            before_meta,
        })
    }

    /// 命令执行后：**第二遍全量元数据扫描**并与第一遍差分；只对变更清单
    /// 里的文件流式读内容算哈希。拒绝不报错——拒绝事实（verdict=
    /// `Rejected`、rejection_reason、逐变更证据、逃逸证据）随返回的
    /// receipt 保留，由调用方 durable append；只有不可观测的 IO/身份
    /// 漂移与预算超限才 fail-closed 报错。
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
        // 第二遍：同样**只记元数据**；两份元数据差分出变更清单，只有清单里
        // 的文件才流式读内容算哈希（REQ-AUD-01/03）。全仓两遍扫描保留。
        let after_meta = scan_meta(&canonical)?;
        let after = snapshot_from_meta(&canonical, &after_meta);
        let changes = diff_meta(&watch.before_meta, &after_meta);
        let (observed_changes, _charge) =
            describe_changes(&canonical, &changes, &self.budget, &self.allowlist)?;

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
