// ---------------------------------------------------------------------------
// 根政策发布:来源收集、正文构造与 durable 写入(Task 1,I1)
// ---------------------------------------------------------------------------

/// 正文聚合的固定标题(Global Constraints 冻结:LF,源字节不 trim)。
const AGGREGATE_POLICY_HEADER: &str = "# 聚合政策\n\n";
/// 根规则目录(canonical root 相对路径)。
const POLICY_RULES_DIR: &str = ".claude/rules";
/// 根政策聚合必须包含的最小规则来源。
const REQUIRED_POLICY_RULE: &str = ".claude/rules/language.md";
/// operation 目录下的不可变发布输出文件名。
const POLICY_PUBLICATION_FILE: &str = "policy-publication.json";
/// save/ensure_bootstrap/发布三方共用的 scope 锁文件名。
const AGGREGATE_POLICY_LOCK_FILE: &str = ".aggregate-policy.lock";

/// 根政策来源字节:canonical root 相对路径(正斜杠)+ 原字节。
struct PolicySource {
    relative_path: String,
    bytes: Vec<u8>,
}

fn invalid_publication(reason: impl Into<String>) -> ProductStoreError {
    ProductStoreError::InvalidRecord {
        kind: "aggregate_policy_publication",
        reason: reason.into(),
    }
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(bytes))
}

/// 读取来源文件原字节。`cfg(test)` 下挂确定性 IO seam(仅测试编译,
/// 生产无此路径),不可读场景不依赖 chmod。
fn read_source_bytes(path: &Path) -> Result<Vec<u8>, ProductStoreError> {
    #[cfg(test)]
    if let Some(error) = publish_faults::trip(path, FaultPhase::SourceRead) {
        return Err(error);
    }
    std::fs::read(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))
}

/// 收集根政策来源字节集:入口 `AGENTS.md` 第一,`.claude/rules/` 下全部
/// 常规 `.md`(含嵌套子目录)按相对路径字节序排列。symlink、非普通
/// 入口、非法 UTF-8、空/纯白入口与缺 `language.md` 一律 fail-closed。
fn collect_policy_sources(canonical_root: &Path) -> Result<Vec<PolicySource>, ProductStoreError> {
    let entry_path = canonical_root.join(ROOT_RULE_ENTRY_FILE);
    let entry_meta = std::fs::symlink_metadata(&entry_path).map_err(|error| {
        invalid_publication(format!("root rule entry {}: {error}", entry_path.display()))
    })?;
    if entry_meta.is_symlink() || !entry_meta.is_file() {
        return Err(invalid_publication(format!(
            "root rule entry {} must be a regular file, not a symlink/directory",
            entry_path.display()
        )));
    }
    let entry_bytes = read_source_bytes(&entry_path)?;
    let entry_text = std::str::from_utf8(&entry_bytes).map_err(|_| {
        invalid_publication(format!(
            "root rule entry {} is not valid UTF-8",
            entry_path.display()
        ))
    })?;
    if entry_text.trim().is_empty() {
        return Err(invalid_publication(format!(
            "root rule entry {} has no effective content",
            entry_path.display()
        )));
    }

    let mut sources = vec![PolicySource {
        relative_path: ROOT_RULE_ENTRY_FILE.to_string(),
        bytes: entry_bytes,
    }];
    let mut rules: Vec<(String, Vec<u8>)> = Vec::new();
    collect_rules_dir(&canonical_root.join(POLICY_RULES_DIR), "", &mut rules)?;
    rules.sort_by(|left, right| left.0.cmp(&right.0));
    for (relative_path, bytes) in rules {
        std::str::from_utf8(&bytes).map_err(|_| {
            invalid_publication(format!("policy source {relative_path} is not valid UTF-8"))
        })?;
        sources.push(PolicySource {
            relative_path,
            bytes,
        });
    }

    if !sources
        .iter()
        .any(|source| source.relative_path == REQUIRED_POLICY_RULE)
    {
        return Err(invalid_publication(format!(
            "policy sources must include {REQUIRED_POLICY_RULE}"
        )));
    }
    Ok(sources)
}

/// 递归收集 `.claude/rules/` 下的常规 `.md`(嵌套目录由调用方按相对
/// 路径字节序排序);目录与常规非 `.md` 跳过,symlink 一律拒绝。
fn collect_rules_dir(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<(String, Vec<u8>)>,
) -> Result<(), ProductStoreError> {
    let meta = match std::fs::symlink_metadata(dir) {
        Ok(meta) => meta,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(invalid_publication(format!(
                "policy rules directory {} is missing; {REQUIRED_POLICY_RULE} is required",
                dir.display()
            )));
        }
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "stat {}: {error}",
                dir.display()
            )));
        }
    };
    if meta.is_symlink() {
        return Err(invalid_publication(format!(
            "policy rules directory {} must not be a symlink",
            dir.display()
        )));
    }
    if !meta.is_dir() {
        return Err(invalid_publication(format!(
            "policy rules path {} must be a directory",
            dir.display()
        )));
    }
    for entry in std::fs::read_dir(dir)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", dir.display())))?
    {
        let entry = entry.map_err(|error| {
            ProductStoreError::Io(format!("read {} entry: {error}", dir.display()))
        })?;
        let path = entry.path();
        let child_meta = std::fs::symlink_metadata(&path)
            .map_err(|error| ProductStoreError::Io(format!("stat {}: {error}", path.display())))?;
        if child_meta.is_symlink() {
            return Err(invalid_publication(format!(
                "policy source {} must not be a symlink",
                path.display()
            )));
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        let child_prefix = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if child_meta.is_dir() {
            collect_rules_dir(&path, &child_prefix, out)?;
        } else if child_meta.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("md")
        {
            let bytes = read_source_bytes(&path)?;
            out.push((format!("{POLICY_RULES_DIR}/{child_prefix}"), bytes));
        }
    }
    Ok(())
}

/// 按 Global Constraints 冻结的格式聚合正文:固定标题 + 每来源
/// `## 来源:<相对路径>` + 完整原字节 + LF 分隔。源字节不 trim、不改
/// 换行;正文不含 revision、policy_id、时间或绝对 root。
fn build_policy_text(sources: &[PolicySource]) -> Result<String, ProductStoreError> {
    let mut text = String::from(AGGREGATE_POLICY_HEADER);
    for source in sources {
        let content = std::str::from_utf8(&source.bytes).map_err(|_| {
            invalid_publication(format!(
                "policy source {} is not valid UTF-8",
                source.relative_path
            ))
        })?;
        text.push_str("## 来源：");
        text.push_str(&source.relative_path);
        text.push_str("\n\n");
        text.push_str(content);
        text.push_str("\n\n");
    }
    Ok(text)
}

/// 同目录临时文件名(pid+纳秒),避免并发写入互相覆盖临时文件。
fn publication_temp_path(target: &Path) -> PathBuf {
    let parent = target
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let file_name = target
        .file_name()
        .map(|value| value.to_string_lossy().into_owned())
        .unwrap_or_else(|| "policy-publication.json".to_string());
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    parent.join(format!(".{file_name}.{}.{}.tmp", std::process::id(), nanos))
}

fn fsync_parent(target: &Path) -> Result<(), ProductStoreError> {
    if let Some(parent) = target.parent()
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

/// 逐级检查 `canonical_root` 到 target 的每个组件:既存必须目录且非
/// symlink;缺失目录逐级创建,绝不越过 canonical root。
fn ensure_publication_parents(
    target: &Path,
    canonical_root: &Path,
) -> Result<(), ProductStoreError> {
    let parent = target.parent().unwrap_or(canonical_root);
    let relative = parent
        .strip_prefix(canonical_root)
        .map_err(|_| {
            invalid_publication(format!(
                "publication target {} escapes canonical root {}",
                target.display(),
                canonical_root.display()
            ))
        })?
        .to_path_buf();
    let mut current = canonical_root.to_path_buf();
    for component in relative.components() {
        current.push(component);
        match std::fs::symlink_metadata(&current) {
            Ok(meta) if meta.is_symlink() => {
                return Err(invalid_publication(format!(
                    "publication parent {} must not be a symlink",
                    current.display()
                )));
            }
            Ok(meta) if !meta.is_dir() => {
                return Err(invalid_publication(format!(
                    "publication parent {} must be a directory",
                    current.display()
                )));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                std::fs::create_dir(&current).map_err(|error| {
                    ProductStoreError::Io(format!("create {}: {error}", current.display()))
                })?;
            }
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "stat {}: {error}",
                    current.display()
                )));
            }
        }
    }
    Ok(())
}

/// 把字节写进 target 同目录的临时文件并 fsync(不建立目标)。失败清理
/// 临时文件;成功返回临时路径,由调用方完成 hard_link/rename 并清理。
fn stage_bytes_durable(target: &Path, bytes: &[u8]) -> Result<PathBuf, ProductStoreError> {
    use std::io::Write;

    let temp_path = publication_temp_path(target);
    let staged = (|| -> Result<(), ProductStoreError> {
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
            .map_err(|error| {
                ProductStoreError::Io(format!("create {}: {error}", temp_path.display()))
            })?;
        file.write_all(bytes).map_err(|error| {
            ProductStoreError::Io(format!("write {}: {error}", temp_path.display()))
        })?;
        file.flush().map_err(|error| {
            ProductStoreError::Io(format!("flush {}: {error}", temp_path.display()))
        })?;
        file.sync_all().map_err(|error| {
            ProductStoreError::Io(format!("sync {}: {error}", temp_path.display()))
        })
    })();
    if let Err(error) = staged {
        let _ = std::fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(temp_path)
}

/// 不可变目标 no-clobber 发布:临时文件 fsync 后 `hard_link`(目标已
/// 存在则 EEXIST,同文件系统原子不覆盖)→ 删临时 → 父目录 fsync。
/// 目标已存在时:普通文件且字节相同 → 复验复用;symlink/非普通/不同
/// 字节 → 冲突,绝不覆盖用户文件。
fn publish_bytes_no_clobber(
    target: &Path,
    bytes: &[u8],
    protected_root: Option<&Path>,
) -> Result<(), ProductStoreError> {
    let target_exists = match std::fs::symlink_metadata(target) {
        Ok(meta) => {
            if meta.is_symlink() {
                return Err(invalid_publication(format!(
                    "publication target {} must not be a symlink",
                    target.display()
                )));
            }
            if !meta.is_file() {
                return Err(invalid_publication(format!(
                    "publication target {} must be a regular file",
                    target.display()
                )));
            }
            true
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => false,
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "stat {}: {error}",
                target.display()
            )));
        }
    };
    if target_exists {
        let existing = std::fs::read(target).map_err(|error| {
            ProductStoreError::Io(format!("read {}: {error}", target.display()))
        })?;
        if existing == bytes {
            return Ok(());
        }
        return Err(ProductStoreError::Conflict {
            kind: "aggregate_policy_publication",
            id: target.display().to_string(),
        });
    }

    if let Some(root) = protected_root {
        ensure_publication_parents(target, root)?;
    } else if let Some(parent) = target.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }

    let temp_path = stage_bytes_durable(target, bytes)?;
    if let Err(error) = std::fs::hard_link(&temp_path, target) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(ProductStoreError::Io(format!(
            "hard_link {} -> {}: {error}",
            temp_path.display(),
            target.display()
        )));
    }
    std::fs::remove_file(&temp_path).map_err(|error| {
        ProductStoreError::Io(format!("remove {}: {error}", temp_path.display()))
    })?;
    fsync_parent(target)
}

/// 校验后的 current artifact 覆盖式 durable 替换(发布链中唯一允许普通
/// rename 覆盖的位置):临时文件 fsync → rename → 父目录 fsync。
fn write_artifact_durable(
    path: &Path,
    artifact: &AggregatePolicyArtifact,
) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(artifact)
        .map_err(|error| ProductStoreError::Json(error.to_string()))?;
    let temp_path = stage_bytes_durable(path, &bytes)?;
    if let Err(error) = std::fs::rename(&temp_path, path) {
        let _ = std::fs::remove_file(&temp_path);
        return Err(ProductStoreError::Io(format!(
            "rename {} to {}: {error}",
            temp_path.display(),
            path.display()
        )));
    }
    fsync_parent(path)
}

/// 发布前的 operation 状态核验(锁外):同 project、Running、
/// current_step=OpenspecAndExamples,manifest/operation/参数三方 canonical
/// root 一致——不放宽生产状态核验,也不允许跨 root 发布。
fn validate_publication_operation(
    operation: &AggregateInitializationOperation,
    manifest: &LogicalCodebaseManifest,
    canonical_root: &Path,
) -> Result<(), ProductStoreError> {
    if operation.project_id != manifest.project_id {
        return Err(ProductStoreError::IdentityMismatch {
            kind: "aggregate_policy_publication",
            id: operation.operation_id.clone(),
        });
    }
    if operation.status != AggregateInitializationOperationStatus::Running
        || operation.current_step != Some(AggregateInitializationStepKind::OpenspecAndExamples)
    {
        return Err(invalid_publication(format!(
            "operation {} must be Running at step OpenspecAndExamples to publish policy",
            operation.operation_id
        )));
    }
    ensure_canonical_matches(&manifest.provider_context_root, canonical_root, "manifest")?;
    ensure_canonical_matches(
        &operation.input.provider_context_root,
        canonical_root,
        "operation",
    )?;
    Ok(())
}

/// canonicalize `candidate` 并要求等于发布 canonical root——manifest/
/// operation/参数三方 root 一致,不允许跨 root 发布。
fn ensure_canonical_matches(
    candidate: &Path,
    canonical_root: &Path,
    label: &str,
) -> Result<(), ProductStoreError> {
    let resolved = std::fs::canonicalize(candidate).map_err(|error| {
        ProductStoreError::Io(format!("canonicalize {}: {error}", candidate.display()))
    })?;
    if resolved != canonical_root {
        return Err(invalid_publication(format!(
            "{label} provider_context_root {} canonicalizes to {}, not the publication canonical root {}",
            candidate.display(),
            resolved.display(),
            canonical_root.display()
        )));
    }
    Ok(())
}

/// 重入输出的身份复验:project/operation/UUID/root/scope 引用一致,
/// base 三元引用与候选 revision、policy_id、digest 自洽——冻结候选被
/// 篡改后不得重放。
fn validate_publication_identity(
    output: &AggregatePolicyPublicationOutput,
    manifest: &LogicalCodebaseManifest,
    operation_id: &str,
    canonical_root: &Path,
    scope_lc_id: &str,
) -> Result<(), ProductStoreError> {
    if output.project_id != manifest.project_id || output.operation_id != operation_id {
        return Err(invalid_publication(format!(
            "publication output identity does not match operation {operation_id} of project {}",
            manifest.project_id
        )));
    }
    if output.logical_codebase_id != manifest.logical_codebase_id.to_string() {
        return Err(invalid_publication(
            "publication output logical-codebase UUID does not match the manifest",
        ));
    }
    if output.lc_id != scope_lc_id {
        return Err(invalid_publication(
            "publication output does not belong to this logical-codebase scope",
        ));
    }
    if output.canonical_root != canonical_root {
        return Err(invalid_publication(format!(
            "publication output canonical root {} does not match {}",
            output.canonical_root.display(),
            canonical_root.display()
        )));
    }
    let artifact = &output.artifact;
    if artifact.project_id != manifest.project_id
        || artifact.logical_codebase_id != output.logical_codebase_id
    {
        return Err(invalid_publication(
            "publication artifact identity does not match the manifest",
        ));
    }
    artifact.validate_digest()?;
    if output
        .base_policy
        .revision
        .checked_add(1)
        .is_none_or(|expected| expected != artifact.revision)
    {
        return Err(invalid_publication(
            "publication base reference does not line up with the candidate revision",
        ));
    }
    // base revision 0 表示无前驱;非 0 时 base policy_id/digest 形状自洽。
    if output.base_policy.revision == 0 {
        if !output.base_policy.policy_id.is_empty() || !output.base_policy.digest.is_empty() {
            return Err(invalid_publication(
                "publication base reference must be empty for revision 0",
            ));
        }
    } else {
        let expected_base_id = format!(
            "policy/{}/{}/{}",
            artifact.project_id, artifact.logical_codebase_id, output.base_policy.revision
        );
        if output.base_policy.policy_id != expected_base_id
            || !output.base_policy.digest.starts_with("sha256:")
        {
            return Err(invalid_publication(
                "publication base reference identity is malformed",
            ));
        }
    }
    let expected_policy_id = format!(
        "policy/{}/{}/{}",
        artifact.project_id, artifact.logical_codebase_id, artifact.revision
    );
    if artifact.policy_id != expected_policy_id {
        return Err(invalid_publication(
            "publication artifact policy_id does not match its identity",
        ));
    }
    Ok(())
}
