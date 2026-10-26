// ---------------------------------------------------------------------------
// 根政策发布:来源收集、正文构造与 durable 写入(Task 1,I1)
// ---------------------------------------------------------------------------

/// 正文聚合的固定标题(Global Constraints 冻结:LF,源字节不 trim)。
const AGGREGATE_POLICY_HEADER: &str = "# 聚合政策\n\n";
/// 根规则目录(canonical root 相对路径)。
const POLICY_RULES_DIR: &str = ".claude/rules";
/// 根政策聚合必须包含的最小规则来源。
const REQUIRED_POLICY_RULE: &str = ".claude/rules/language.md";
/// R6：政策源第二项（owner 裁定纳入；文件缺失时跳过，不视为错误）。
const POLICY_CLAUDE_ENTRY_FILE: &str = "CLAUDE.md";
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

/// R3:单一来源的稳定性指纹——长度、mtime(秒+纳秒)与 inode。任一变化
/// 都说明文件在合成窗口内被替换/改写,拼出的正文可能是混合内容。
#[derive(Debug, Clone, PartialEq, Eq)]
struct SourceStat {
    len: u64,
    modified_secs: i64,
    modified_nanos: i64,
    inode: u64,
}

/// 来源快照指纹:入口/规则相对路径 + 稳定性指纹,按相对路径字节序。
type SourceFingerprint = Vec<(String, SourceStat)>;

/// 来源合成错误:`Store` 是真实 IO/契约错误(直接上抛),`Unstable` 是
/// 「读窗口内来源被改写」(重试一次后仍不稳定则报可识别错误)。
enum SourceSnapshotError {
    Store(ProductStoreError),
    Unstable(String),
}

impl From<ProductStoreError> for SourceSnapshotError {
    fn from(error: ProductStoreError) -> Self {
        Self::Store(error)
    }
}

/// 读取来源文件原字节。`cfg(test)` 下挂确定性 IO seam(仅测试编译,
/// 生产无此路径),不可读场景不依赖 chmod。
/// R6：读取可选来源；文件不存在返回 `None`（与必需源区分）。
fn read_optional_source_snapshot(
    path: &Path,
) -> Result<Option<(Vec<u8>, SourceStat)>, SourceSnapshotError> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) => {
            if meta.is_symlink() || !meta.is_file() {
                return Err(invalid_publication(format!(
                    "policy source {} must be a regular file, not a symlink/directory",
                    path.display()
                ))
                .into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(invalid_publication(format!(
                "policy source {}: {error}",
                path.display()
            ))
            .into())
        }
    }
    let (bytes, stat) = read_source_snapshot(path)?;
    Ok(Some((bytes, stat)))
}

fn read_source_bytes(path: &Path) -> Result<Vec<u8>, ProductStoreError> {
    #[cfg(test)]
    if let Some(error) = publish_faults::trip(path, FaultPhase::SourceRead) {
        return Err(error);
    }
    std::fs::read(path)
        .map_err(|error| ProductStoreError::Io(format!("read {}: {error}", path.display())))
}

/// 来源文件的稳定性指纹(`std::fs::metadata` 跟随 symlink;symlink 本身
/// 在上层遍历中已拒绝)。
fn stat_source(path: &Path) -> Result<SourceStat, ProductStoreError> {
    use std::os::unix::fs::MetadataExt;

    let meta = std::fs::metadata(path)
        .map_err(|error| ProductStoreError::Io(format!("stat {}: {error}", path.display())))?;
    Ok(SourceStat {
        len: meta.len(),
        modified_secs: meta.mtime(),
        modified_nanos: meta.mtime_nsec(),
        inode: meta.ino(),
    })
}

/// 读取来源字节并校验读窗口前后 stat 一致:同一 `(len, mtime, inode)`
/// 才能证明这一次读拿到的是确定性字节。
fn read_source_snapshot(
    path: &Path,
) -> Result<(Vec<u8>, SourceStat), SourceSnapshotError> {
    let before = stat_source(path)?;
    let bytes = read_source_bytes(path)?;
    #[cfg(test)]
    publish_faults::mutate_source_after_read(path);
    let after = stat_source(path)?;
    if before != after {
        return Err(SourceSnapshotError::Unstable(format!(
            "{} changed while it was being read",
            path.display()
        )));
    }
    Ok((bytes, after))
}

/// 收集根政策来源字节集(R3):一次遍历锁定同一根目录下的全部必需来源,
/// 逐文件读窗口前后 stat 一致 + 全量指纹复算;任一次不一致重试一次,
/// 仍不一致报 `policy_sources_unstable`——绝不拼出混合正文。
fn collect_policy_sources(canonical_root: &Path) -> Result<Vec<PolicySource>, ProductStoreError> {
    let mut detail = String::from("no consistent snapshot");
    for _ in 0..2 {
        let (sources, fingerprint) = match collect_policy_sources_once(canonical_root) {
            Ok(snapshot) => snapshot,
            Err(SourceSnapshotError::Store(error)) => return Err(error),
            Err(SourceSnapshotError::Unstable(reason)) => {
                detail = reason;
                continue;
            }
        };
        match fingerprint_policy_sources(canonical_root) {
            Ok(fresh) if fresh == fingerprint => return Ok(sources),
            Ok(_) => detail = "policy sources changed during composition".to_string(),
            Err(error) => detail = error.to_string(),
        }
    }
    Err(invalid_publication(format!(
        "policy_sources_unstable: {detail}; refusing to compose a mixed policy body"
    )))
}

/// 单次来源收集:入口 `AGENTS.md` 第一,`.claude/rules/` 下全部常规 `.md`
///(含嵌套子目录)按相对路径字节序排列。symlink、非普通入口、非法 UTF-8、
/// 空/纯白入口与缺 `language.md` 一律 fail-closed。
fn collect_policy_sources_once(
    canonical_root: &Path,
) -> Result<(Vec<PolicySource>, SourceFingerprint), SourceSnapshotError> {
    let entry_path = canonical_root.join(ROOT_RULE_ENTRY_FILE);
    let entry_meta = std::fs::symlink_metadata(&entry_path).map_err(|error| {
        invalid_publication(format!("root rule entry {}: {error}", entry_path.display()))
    })?;
    if entry_meta.is_symlink() || !entry_meta.is_file() {
        return Err(invalid_publication(format!(
            "root rule entry {} must be a regular file, not a symlink/directory",
            entry_path.display()
        ))
        .into());
    }
    let (entry_bytes, entry_stat) = read_source_snapshot(&entry_path)?;
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
        ))
        .into());
    }

    let mut sources = vec![PolicySource {
        relative_path: ROOT_RULE_ENTRY_FILE.to_string(),
        bytes: entry_bytes,
    }];
    let mut fingerprint: SourceFingerprint = vec![(ROOT_RULE_ENTRY_FILE.to_string(), entry_stat)];
    // R6(owner 2026-10-26)：政策源追加 `CLAUDE.md`（存在才纳入）。
    // 顺序固定在 `AGENTS.md` 之后、`.claude/rules/*` 之前，保证合成确定性。
    if let Some((claude_bytes, claude_stat)) =
        read_optional_source_snapshot(&canonical_root.join(POLICY_CLAUDE_ENTRY_FILE))?
    {
        std::str::from_utf8(&claude_bytes).map_err(|_| {
            invalid_publication(format!(
                "policy source {POLICY_CLAUDE_ENTRY_FILE} is not valid UTF-8"
            ))
        })?;
        sources.push(PolicySource {
            relative_path: POLICY_CLAUDE_ENTRY_FILE.to_string(),
            bytes: claude_bytes,
        });
        fingerprint.push((POLICY_CLAUDE_ENTRY_FILE.to_string(), claude_stat));
    }
    let mut rules: Vec<String> = Vec::new();
    collect_rule_paths(&canonical_root.join(POLICY_RULES_DIR), "", &mut rules)?;
    rules.sort();
    for relative_path in rules {
        let path = canonical_root.join(&relative_path);
        let (bytes, stat) = read_source_snapshot(&path)?;
        std::str::from_utf8(&bytes).map_err(|_| {
            invalid_publication(format!("policy source {relative_path} is not valid UTF-8"))
        })?;
        sources.push(PolicySource {
            relative_path: relative_path.clone(),
            bytes,
        });
        fingerprint.push((relative_path, stat));
    }

    if !sources
        .iter()
        .any(|source| source.relative_path == REQUIRED_POLICY_RULE)
    {
        return Err(invalid_publication(format!(
            "policy sources must include {REQUIRED_POLICY_RULE}"
        ))
        .into());
    }
    Ok((sources, fingerprint))
}

/// 复算来源指纹(不读字节,只枚举 + stat):与读取窗口内记录的指纹比较,
/// 判断整批来源是否仍是同一时刻的同一组文件。
fn fingerprint_policy_sources(canonical_root: &Path) -> Result<SourceFingerprint, ProductStoreError> {
    let mut paths = vec![ROOT_RULE_ENTRY_FILE.to_string()];
    if canonical_root.join(POLICY_CLAUDE_ENTRY_FILE).is_file() {
        paths.push(POLICY_CLAUDE_ENTRY_FILE.to_string());
    }
    let mut rules: Vec<String> = Vec::new();
    collect_rule_paths(&canonical_root.join(POLICY_RULES_DIR), "", &mut rules)?;
    rules.sort();
    paths.extend(rules);
    paths
        .into_iter()
        .map(|relative_path| {
            let stat = stat_source(&canonical_root.join(&relative_path))?;
            Ok((relative_path, stat))
        })
        .collect()
}

/// 递归收集 `.claude/rules/` 下的常规 `.md` 相对路径(嵌套目录由调用方按
/// 相对路径字节序排序);目录与常规非 `.md` 跳过,symlink 一律拒绝。
fn collect_rule_paths(
    dir: &Path,
    prefix: &str,
    out: &mut Vec<String>,
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
            collect_rule_paths(&path, &child_prefix, out)?;
        } else if child_meta.is_file()
            && path.extension().and_then(|value| value.to_str()) == Some("md")
        {
            out.push(format!("{POLICY_RULES_DIR}/{child_prefix}"));
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

/// 覆盖式 durable JSON 写:临时文件 fsync → rename → 父目录 fsync。
/// 只用于「单文件即完整状态」的记录(current artifact、重新发布记录)。
fn write_json_durable<T: serde::Serialize>(
    path: &Path,
    value: &T,
) -> Result<(), ProductStoreError> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        std::fs::create_dir_all(parent).map_err(|error| {
            ProductStoreError::Io(format!("create {}: {error}", parent.display()))
        })?;
    }
    let bytes = serde_json::to_vec_pretty(value)
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

/// 校验后的 current artifact 覆盖式 durable 替换(发布链中唯一允许普通
/// rename 覆盖的位置)。
fn write_artifact_durable(
    path: &Path,
    artifact: &AggregatePolicyArtifact,
) -> Result<(), ProductStoreError> {
    write_json_durable(path, artifact)
}

/// 来源摘要集(不重复保存正文;完整正文只存在候选 artifact 中)。
fn source_digests_of(sources: &[PolicySource]) -> Vec<AggregatePolicySourceDigest> {
    sources
        .iter()
        .map(|source| AggregatePolicySourceDigest {
            relative_path: source.relative_path.clone(),
            digest: sha256_hex(&source.bytes),
        })
        .collect()
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
    // base 引用两种合法形态:严格前驱(base.revision + 1 == candidate)或
    // 「复用当前 revision」(base 逐字段等于 candidate——正文未变的幂等
    // 重放不会新增 revision)。
    let reuse_current = output.base_policy.policy_id == artifact.policy_id
        && output.base_policy.revision == artifact.revision
        && output.base_policy.digest == artifact.digest;
    let advances_from_base = output
        .base_policy
        .revision
        .checked_add(1)
        .is_some_and(|expected| expected == artifact.revision);
    if !reuse_current && !advances_from_base {
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
