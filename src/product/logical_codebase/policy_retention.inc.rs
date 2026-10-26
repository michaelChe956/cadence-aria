// ---------------------------------------------------------------------------
// R4:政策 revision 滚动保留(owner 2026-10-26:「历史版本不需要保留太多」)。
//
// 发布成功后只保留最近 `POLICY_REVISION_RETENTION_WINDOW` 个 revision 的
// `policy/{project}/{lc}/{rev}` 正文文件；但「引用安全阀」优先于保留窗口：
// 任何仍被 durable 事实引用的 revision 一律不删。清理尽力而为，删除失败
// 只 warn，绝不阻断发布，也不影响 artifact/回执/正文的三重一致性。
// ---------------------------------------------------------------------------

/// 滚动保留窗口:最近 N 个 revision 的正文文件保留在权威根下。
/// owner 裁决 2026-10-26(「历史版本不需要保留太多」);调整只需改此常量。
pub(crate) const POLICY_REVISION_RETENTION_WINDOW: usize = 3;

/// 引用扫描的单文件大小上限与文件数上限(防病态目录拖垮发布路径)。
const RETENTION_SCAN_MAX_FILE_BYTES: u64 = 1024 * 1024;
const RETENTION_SCAN_MAX_FILES: usize = 5000;
/// 引用扫描跳过的非产品数据目录。
const RETENTION_SCAN_SKIP_DIRS: &[&str] = &["node_modules", "target", ".git"];

/// 政策 revision 的 durable 引用集合:被引用的 revision 与 digest 各成一组。
#[derive(Debug, Default)]
struct PolicyRevisionReferences {
    revisions: std::collections::BTreeSet<u64>,
    digests: std::collections::BTreeSet<String>,
}

impl PolicyRevisionReferences {
    fn add_revision(&mut self, revision: u64) {
        self.revisions.insert(revision);
    }

    fn add_digest(&mut self, digest: &str) {
        if digest.starts_with("sha256:") {
            self.digests.insert(digest.to_string());
        }
    }
}

impl AggregatePolicyArtifactStore {
    fn policy_revision_dir(
        &self,
        canonical_root: &Path,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> PathBuf {
        canonical_root
            .join("policy")
            .join(project_id)
            .join(logical_codebase_id)
    }

    /// 清理失败只 warn:发布已提交的一致性不受影响。
    fn retain_policy_revisions_best_effort(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        canonical_root: &Path,
    ) {
        if let Err(error) =
            self.retain_policy_revisions(project_id, logical_codebase_id, canonical_root)
        {
            tracing::warn!(
                %error,
                "policy revision retention failed; the published artifact is unaffected"
            );
        }
    }

    /// keep = 最近 N 个 ∪ 被 durable 事实引用 ∪ 当前 artifact;其余删除。
    /// 只动本 project 本 LC 的 `policy/{project}/{lc}/`，绝不跨 LC。
    fn retain_policy_revisions(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        canonical_root: &Path,
    ) -> Result<(), ProductStoreError> {
        let dir = self.policy_revision_dir(canonical_root, project_id, logical_codebase_id);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            // 目录不存在 = 没有历史正文可清理。
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    dir.display()
                )));
            }
        };
        let mut revisions: Vec<(u64, PathBuf)> = Vec::new();
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", dir.display()))
            })?;
            let Some(revision) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<u64>().ok())
            else {
                continue;
            };
            let path = entry.path();
            let meta = std::fs::symlink_metadata(&path).map_err(|error| {
                ProductStoreError::Io(format!("stat {}: {error}", path.display()))
            })?;
            if meta.is_symlink() || !meta.is_file() {
                continue;
            }
            revisions.push((revision, path));
        }
        if revisions.len() <= POLICY_REVISION_RETENTION_WINDOW {
            return Ok(());
        }
        revisions.sort_by(|left, right| right.0.cmp(&left.0));
        let references = self.collect_policy_revision_references(project_id, logical_codebase_id)?;
        for (index, (revision, path)) in revisions.into_iter().enumerate() {
            if index < POLICY_REVISION_RETENTION_WINDOW || references.revisions.contains(&revision) {
                continue;
            }
            // digest 引用(attempts/receipts 只记 digest)同样受保护。
            if let Ok(bytes) = std::fs::read(&path)
                && references.digests.contains(&sha256_hex(&bytes))
            {
                continue;
            }
            #[cfg(test)]
            if let Some(error) = publish_faults::trip(&path, FaultPhase::RetentionDelete) {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "policy revision retention skipped a file (injected fault)"
                );
                continue;
            }
            if let Err(error) = std::fs::remove_file(&path) {
                tracing::warn!(
                    path = %path.display(),
                    %error,
                    "policy revision retention could not remove a revision file"
                );
            }
        }
        Ok(())
    }

    /// 收集本 project 全部 durable 事实对本 LC 政策 revision 的引用。
    fn collect_policy_revision_references(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<PolicyRevisionReferences, ProductStoreError> {
        let mut references = PolicyRevisionReferences::default();
        if let Some(current) = self.get(project_id)? {
            references.add_revision(current.revision);
            references.add_digest(&current.digest);
        }
        if let Some(record) = self.republication(project_id)? {
            references.add_revision(record.revision);
            if record.prior_revision > 0 {
                references.add_revision(record.prior_revision);
            }
            references.add_digest(&record.digest);
            references.add_digest(&record.prior_digest);
            references.add_digest(&record.superseded_receipt_policy_digest);
            references.add_digest(&record.superseded_receipt_rule_digest);
        }
        // 新回执的 base 链(当前 revision 的发布回执指向的前驱)。
        if let Some(current) = self.get(project_id)?
            && let Some(output) = self.current_publication_output(project_id, &current)?
        {
            references.add_revision(output.base_policy.revision);
            references.add_digest(&output.base_policy.digest);
        }
        // 项目级 durable 事实(attempts/sessions/waiting facts/未完成 run)。
        let mut scanned = 0usize;
        scan_policy_references(
            &self.paths.project_root(project_id),
            project_id,
            logical_codebase_id,
            &mut references,
            &mut scanned,
        );
        Ok(references)
    }

    /// 当前 artifact 对应的发布回执(同 digest 的那条)。
    /// 该 artifact 是否由发布流水线产出（存在冻结的 publication output）。
    /// 只有这种 artifact 才参与「是否落后于当前规则源」的新鲜度判定；
    /// 手工/迁移写入的存量 artifact 不做该判定，避免误判为过期。
    pub(crate) fn has_publication_record(
        &self,
        project_id: &str,
        current: &AggregatePolicyArtifact,
    ) -> Result<bool, ProductStoreError> {
        Ok(self.current_publication_output(project_id, current)?.is_some())
    }

    fn current_publication_output(
        &self,
        project_id: &str,
        current: &AggregatePolicyArtifact,
    ) -> Result<Option<AggregatePolicyPublicationOutput>, ProductStoreError> {
        let root = self.scope_root(project_id)?.join("aggregate-initializations");
        let entries = match std::fs::read_dir(&root) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    root.display()
                )));
            }
        };
        for entry in entries.flatten() {
            let path = entry.path().join(POLICY_PUBLICATION_FILE);
            if !path.is_file() {
                continue;
            }
            let Ok(output) = read_json::<AggregatePolicyPublicationOutput>(&path) else {
                continue;
            };
            if output.artifact.digest == current.digest && output.project_id == project_id {
                return Ok(Some(output));
            }
        }
        Ok(None)
    }
}

/// 递归扫描 JSON durable 事实里的政策引用(`policy_id` 精确前缀、`policy_digest`
/// 与 `digest` 的 sha256 摘要)。发布回执(内嵌完整正文)与重新发布记录由调用
/// 方显式处理，此处跳过，避免它们把每个历史 revision 都钉住。
fn scan_policy_references(
    root: &Path,
    project_id: &str,
    logical_codebase_id: &str,
    references: &mut PolicyRevisionReferences,
    scanned: &mut usize,
) {
    let policy_prefix = format!("policy/{project_id}/{logical_codebase_id}/");
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_symlink() {
                continue;
            }
            if meta.is_dir() {
                let skip = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| RETENTION_SCAN_SKIP_DIRS.contains(&name));
                if !skip {
                    stack.push(path);
                }
                continue;
            }
            if !meta.is_file()
                || meta.len() > RETENTION_SCAN_MAX_FILE_BYTES
                || path.extension().and_then(|value| value.to_str()) != Some("json")
                || path.file_name().and_then(|name| name.to_str())
                    == Some(POLICY_PUBLICATION_FILE)
                || path.file_name().and_then(|name| name.to_str())
                    == Some(POLICY_REPUBLICATION_FILE)
            {
                continue;
            }
            if *scanned >= RETENTION_SCAN_MAX_FILES {
                tracing::warn!(
                    limit = RETENTION_SCAN_MAX_FILES,
                    "policy revision retention stopped scanning durable facts at the file cap"
                );
                return;
            }
            *scanned += 1;
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            if !text.contains("\"policy_id\"") && !text.contains("\"policy_digest\"") {
                continue;
            }
            let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
                continue;
            };
            collect_references_from_value(&value, &policy_prefix, references);
        }
    }
}

fn collect_references_from_value(
    value: &serde_json::Value,
    policy_prefix: &str,
    references: &mut PolicyRevisionReferences,
) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, child) in map {
                match (key.as_str(), child) {
                    ("policy_id", serde_json::Value::String(id)) => {
                        if let Some(revision) = id
                            .strip_prefix(policy_prefix)
                            .and_then(|rest| rest.parse::<u64>().ok())
                        {
                            references.add_revision(revision);
                        }
                    }
                    ("policy_digest", serde_json::Value::String(digest)) => {
                        references.add_digest(digest);
                    }
                    ("digest", serde_json::Value::String(digest)) => {
                        references.add_digest(digest);
                    }
                    _ => {}
                }
                collect_references_from_value(child, policy_prefix, references);
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_references_from_value(item, policy_prefix, references);
            }
        }
        _ => {}
    }
}
