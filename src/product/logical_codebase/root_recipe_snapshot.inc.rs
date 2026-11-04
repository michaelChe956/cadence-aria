// ---------------------------------------------------------------------------
// Audit snapshot: metadata diff + streaming reads (REQ-AUD-01/02/03)
// ---------------------------------------------------------------------------
//
// 落点见 cadence/plans/2026-11-03_计划文档_实施_块2_聚合冷启动与审计预算
// _v1.0.md §7。四条口径：
//
// 1. **两遍扫描只记元数据**（REQ-AUD-01）：第一遍冻结命令执行前的元数据，
//    第二遍取命令执行后的元数据；两份元数据差分出「本次变更清单」，**只有
//    清单里的文件才读内容算哈希**。全仓两遍扫描保留（不改成只扫改动路径、
//    不引入 inotify），「recipe 写到 allowlist 之外」的检测能力完整保留。
// 2. **`.git` 子树不进快照观测面**（REQ-AUD-02）：路径任一段为 `.git` →
//    该子树的条目不进快照、不计条目/字节、内容不读（与聚合索引排除清单
//    `**/.git/` 同一口径，仓库内不出现两套 `.git` 口径）。但它的**元数据**
//    仍被单独采集为「成员 git 面」——只用于判定「命令试图改 `.git`」这一
//    越界行为（`classify_member_git` 的 MemberGit / MemberWorktree 分类
//    保留；`.git` 永不出现在大项分组里）。
// 3. **流式读**（REQ-AUD-03）：64 KiB 分块，边读边累加字节、边算 sha256，
//    越限立刻停（内存峰值 = 块大小量级），绝不先整读后计费。
// 4. **预算口径 = 本次变更量**：`bytes` 与 `entries` 都只对**进入变更清单的
//    路径**计；未变化路径与 `.git` 子树一律不计字节、也不计条目（否则
//    >20 000 文件的存量仓仍会因 `max_entries` 失败）。

/// 审计快照的元数据事实：路径 + 大小 + 修改时间（+ 条目类型）。
///
/// Plan §7.2 的契约字段是 `path`/`size`/`mtime_ns`；此处另带
/// `kind`/`link_target`/`special_type`/`escapes_root`/`in_git`——差分必须能
/// 识别「类型变了」「链接目标变了」，且必须能把「成员 git 面」与快照观测
/// 面分开（`.git` 子树不进快照、不计预算、不读内容，但仍要能判定
/// 「命令试图改 `.git`」这一越界行为）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SnapshotMeta {
    /// canonical 相对路径（相对审计根，正斜杠分隔）。
    path: PathBuf,
    /// 条目类型（文件/目录/链接/非常规）。
    kind: RootRecipeSnapshotEntryKind,
    /// 文件字节数（目录/链接/非常规条目为 0）。
    size: u64,
    /// 修改时间（Unix epoch 纳秒；早于 epoch 为负；取不到时为 0）。
    mtime_ns: i128,
    /// symlink 目标（仅链接条目）。
    link_target: Option<String>,
    /// 非常规条目的具体类型名（socket/fifo/…）。
    special_type: Option<String>,
    /// symlink 词法解析后是否逃出审计根。
    escapes_root: bool,
    /// 是否属「成员 git 面」（路径任一段为 `.git`）。
    in_git: bool,
}

/// 一次元数据差分得到的单条变更（路径 + 变更类型 + 前后元数据）。
#[derive(Debug, Clone)]
struct MetaChange<'a> {
    path: &'a Path,
    kind: RootRecipeChangeKind,
    before: Option<&'a SnapshotMeta>,
    after: Option<&'a SnapshotMeta>,
}

/// 变更清单的计费累计（bytes/entries 只对进入变更清单的路径计）。
#[derive(Debug, Default)]
struct AuditCharge {
    /// 已流式读到的字节数（越限判据）。
    bytes: u64,
    /// 已进入变更清单并计过的路径数。
    entries: usize,
    /// 大项分组累计（键见 [`budget_group_key`]）。
    groups: BTreeMap<String, u64>,
    /// 各成员仓各自的贡献。
    members: BTreeMap<String, MemberCharge>,
}

/// 单个成员仓的累计贡献。
#[derive(Debug, Default)]
struct MemberCharge {
    bytes: u64,
    entries: usize,
}

/// 流式读取的结果：内容摘要，或「越限已中止」。
#[derive(Debug, Clone)]
enum StreamDigest {
    /// 完整读完并算出的内容摘要。
    Digest(String),
    /// 累计字节越限，已立刻停止读取（不再读剩余内容）。
    BudgetExceeded,
}

/// 内容读取的分块大小（64 KiB）——流式读的内存峰值量级。
const AUDIT_READ_CHUNK: usize = 64 * 1024;

// ---------------------------------------------------------------------------
// Metadata scan
// ---------------------------------------------------------------------------

/// 全仓元数据扫描：**只记元数据，不读文件内容**（REQ-AUD-01）。
///
/// `.git` 子树**不进快照观测面**（不进条目、不计条目/字节、不读内容，
/// REQ-AUD-02）；但它的元数据仍被单独采集到「成员 git 面」——只用于判定
/// 「命令试图改 `.git`」这一越界行为（`classify_member_git` 分类保留）。
/// gitignore 感知口径不变（被 `.gitignore` / `.git/info/exclude` 忽略的
/// 路径不进观测面）。
fn scan_meta(root: &Path) -> Result<Vec<SnapshotMeta>, ProductStoreError> {
    let mut metas = Vec::new();
    let mut ignores = AuditIgnoreStack::default();
    walk_meta(root, root, &mut metas, &mut ignores)?;
    metas.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(metas)
}

/// 递归遍历一个目录，只采集元数据（不读文件内容）。
fn walk_meta(
    root: &Path,
    dir: &Path,
    metas: &mut Vec<SnapshotMeta>,
    ignores: &mut AuditIgnoreStack,
) -> Result<(), ProductStoreError> {
    let pushed = ignores.enter_dir(root, dir)?;
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
        // 被忽略路径(含目录整体)不进观测面（gitignore 感知口径不变）。
        if ignores.is_ignored(&relative, file_type.is_dir()) {
            continue;
        }
        // `.git` 子树：**不进快照观测面**（条目/字节/内容都不算），只采集
        // 元数据供「成员 git 面」判定用（REQ-AUD-02）。
        if is_git_path(&relative) {
            collect_git_face(root, &path, metas)?;
            continue;
        }
        let meta = meta_of(root, &path, &relative, &file_type, false)?;
        let is_dir = meta.kind == RootRecipeSnapshotEntryKind::Dir;
        metas.push(meta);
        if is_dir {
            walk_meta(root, &path, metas, ignores)?;
        }
    }
    ignores.leave_dir(pushed);
    Ok(())
}

/// 采集「成员 git 面」的元数据：`.git` 子树只记元数据（路径/大小/mtime），
/// 不读任何内容、不计条目/字节、不进快照——存在的唯一目的是判定
/// 「命令试图改 `.git`」这一越界行为（`MemberGit` / `MemberWorktree`）。
fn collect_git_face(
    root: &Path,
    dir: &Path,
    metas: &mut Vec<SnapshotMeta>,
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
        let meta = meta_of(root, &path, &relative, &file_type, true)?;
        let is_dir = meta.kind == RootRecipeSnapshotEntryKind::Dir;
        metas.push(meta);
        if is_dir {
            collect_git_face(root, &path, metas)?;
        }
    }
    Ok(())
}

/// 一个路径的元数据事实（不读内容）。
fn meta_of(
    root: &Path,
    path: &Path,
    relative: &str,
    file_type: &std::fs::FileType,
    in_git: bool,
) -> Result<SnapshotMeta, ProductStoreError> {
    let metadata = std::fs::symlink_metadata(path).map_err(|error| {
        ProductStoreError::Io(format!("audit stat {}: {error}", path.display()))
    })?;
    let kind = if file_type.is_symlink() {
        RootRecipeSnapshotEntryKind::Symlink
    } else if file_type.is_dir() {
        RootRecipeSnapshotEntryKind::Dir
    } else if file_type.is_file() {
        RootRecipeSnapshotEntryKind::File
    } else {
        RootRecipeSnapshotEntryKind::Special
    };
    let link_target = if kind == RootRecipeSnapshotEntryKind::Symlink {
        let target = std::fs::read_link(path).map_err(|error| {
            ProductStoreError::Io(format!("audit readlink {}: {error}", path.display()))
        })?;
        Some(target.to_string_lossy().into_owned())
    } else {
        None
    };
    let special_type = if kind == RootRecipeSnapshotEntryKind::Special {
        Some(special_entry_kind(file_type).to_string())
    } else {
        None
    };
    let escapes_root = match (&link_target, kind) {
        (Some(target), RootRecipeSnapshotEntryKind::Symlink) => {
            symlink_escapes_root(root, path, Path::new(target))
        }
        _ => false,
    };
    Ok(SnapshotMeta {
        path: PathBuf::from(relative),
        kind,
        size: if kind == RootRecipeSnapshotEntryKind::File {
            metadata.len()
        } else {
            0
        },
        mtime_ns: metadata_mtime_ns(&metadata),
        link_target,
        special_type,
        escapes_root,
        in_git,
    })
}

/// 路径任一段为 `.git` → 属「成员 git 面」（不进快照观测面）。
fn is_git_path(relative: &str) -> bool {
    relative.split('/').any(|segment| segment == ".git")
}

/// 文件修改时间（Unix epoch 纳秒；早于 epoch 为负；取不到时为 0）。
fn metadata_mtime_ns(metadata: &std::fs::Metadata) -> i128 {
    match metadata.modified() {
        Ok(time) => match time.duration_since(std::time::UNIX_EPOCH) {
            Ok(delta) => delta.as_nanos() as i128,
            Err(error) => -(error.duration().as_nanos() as i128),
        },
        Err(_) => 0,
    }
}

// ---------------------------------------------------------------------------
// Metadata diff
// ---------------------------------------------------------------------------

/// 两份元数据的差分（REQ-AUD-01）：只有「新增/消失/大小或 mtime 变化/
/// 类型或链接目标变化」的路径进入变更清单；其余路径既不读内容也不计费。
fn diff_meta<'a>(
    before: &'a [SnapshotMeta],
    after: &'a [SnapshotMeta],
) -> Vec<MetaChange<'a>> {
    let before_map: BTreeMap<&'a Path, &'a SnapshotMeta> = before
        .iter()
        .map(|meta| (meta.path.as_path(), meta))
        .collect();
    let after_map: BTreeMap<&'a Path, &'a SnapshotMeta> = after
        .iter()
        .map(|meta| (meta.path.as_path(), meta))
        .collect();
    let mut paths: BTreeSet<&'a Path> = before_map.keys().copied().collect();
    paths.extend(after_map.keys().copied());
    let mut changes = Vec::new();
    for path in paths {
        let before_entry = before_map.get(&path).copied();
        let after_entry = after_map.get(&path).copied();
        let kind = match (before_entry, after_entry) {
            (Some(_), None) => RootRecipeChangeKind::Removed,
            (None, Some(_)) => RootRecipeChangeKind::Added,
            (Some(before_entry), Some(after_entry)) => {
                if meta_equivalent(before_entry, after_entry) {
                    continue;
                }
                RootRecipeChangeKind::Modified
            }
            (None, None) => continue,
        };
        changes.push(MetaChange {
            path,
            kind,
            before: before_entry,
            after: after_entry,
        });
    }
    changes
}

/// 两份元数据是否等价（无变更）。
///
/// - 目录：**只看类型**——目录的 mtime 会在任意子条目增删时变化，把它当
///   「变更」会把「文件写入」放大成「父目录也变了」的噪声（目录内容变化
///   由子条目各自承载）。
/// - 普通文件：大小或 mtime 变化即视为变更（REQ-AUD-01 的元数据口径）。
/// - symlink：链接目标变化即变更；非常规条目：类型变化即变更。
fn meta_equivalent(before: &SnapshotMeta, after: &SnapshotMeta) -> bool {
    if before.kind != after.kind {
        return false;
    }
    match before.kind {
        RootRecipeSnapshotEntryKind::Dir => true,
        RootRecipeSnapshotEntryKind::File => {
            before.size == after.size && before.mtime_ns == after.mtime_ns
        }
        RootRecipeSnapshotEntryKind::Symlink => before.link_target == after.link_target,
        RootRecipeSnapshotEntryKind::Special => before.special_type == after.special_type,
    }
}

// ---------------------------------------------------------------------------
// Change description + budget charging
// ---------------------------------------------------------------------------

/// 变更清单 → 观察到的变更证据（分类 + 内容哈希），并按**本次变更量**计费。
///
/// 计费口径（REQ-AUD-01）：`bytes` 与 `entries` 都只对进入变更清单的路径
/// 计；内容读取为流式（边读边累加、越限立刻停）。
fn describe_changes(
    root: &Path,
    changes: &[MetaChange<'_>],
    budget: &RootRecipeSnapshotBudget,
    allowlist: &[String],
) -> Result<(Vec<RootRecipeObservedChange>, AuditCharge), ProductStoreError> {
    let mut charge = AuditCharge::default();
    // 分组统计先按变更清单的已知体量算全——越限中止时也要给出完整的大项
    // 排行（文案要回答「谁占得多」，半张榜没有意义）。`.git` 子树不参与
    // 排行与计费（REQ-AUD-02：`.git` 永不入列）。
    for change in changes {
        if change_is_git_face(change) {
            continue;
        }
        let relative = change_path_string(change.path);
        let kind = change
            .after
            .or(change.before)
            .map(|meta| meta.kind)
            .unwrap_or(RootRecipeSnapshotEntryKind::File);
        let size = change
            .after
            .map(|meta| meta.size)
            .or_else(|| change.before.map(|meta| meta.size))
            .unwrap_or(0);
        let group = budget_group_key(&relative, kind);
        *charge.groups.entry(group).or_insert(0) += size;
        let member = charge
            .members
            .entry(budget_member(&relative, kind))
            .or_default();
        member.bytes += size;
        member.entries += 1;
    }

    let mut observed = Vec::with_capacity(changes.len());
    for change in changes {
        let relative = change_path_string(change.path);
        let git_face = change_is_git_face(change);
        if !git_face {
            charge.entries += 1;
            if charge.entries > budget.max_entries {
                return Err(budget_failure(&relative, &charge, budget));
            }
        }
        let before_entry = change.before.map(entry_of_meta);
        let after_entry = change.after.map(entry_of_meta);
        let (classification, escapes_root, link_target) = classify_change(
            &relative,
            before_entry.as_ref(),
            after_entry.as_ref(),
            allowlist,
        );
        let allowed = classification == RootRecipeChangeClass::AllowlistedArtifact;
        // 只有「现在还存在」的普通文件才读内容算哈希（流式、边读边计费）；
        // `.git` 子树**永不读内容**（REQ-AUD-02）。
        let after_digest = match change.after {
            Some(meta)
                if meta.kind == RootRecipeSnapshotEntryKind::File && !git_face =>
            {
                let path = root.join(change.path);
                match stream_file_digest(&path, budget, &mut charge.bytes)? {
                    StreamDigest::Digest(digest) => Some(digest),
                    StreamDigest::BudgetExceeded => {
                        return Err(budget_failure(&relative, &charge, budget));
                    }
                }
            }
            _ => None,
        };
        observed.push(RootRecipeObservedChange {
            path: relative,
            change_kind: change.kind,
            classification,
            allowed,
            // 变更前的**内容**摘要不再可得：元数据差分法只对变更文件读
            // 「现在」的内容（读旧内容就得读完整个仓，等于退回旧口径）。
            before_digest: None,
            after_digest,
            link_target: link_target.or_else(|| {
                before_entry
                    .and_then(|entry| entry.link_target.clone())
                    .or_else(|| after_entry.and_then(|entry| entry.link_target.clone()))
            }),
            escapes_root,
        });
    }
    Ok((observed, charge))
}

/// 该变更是否属「成员 git 面」（`.git` 子树）——这类变更**不计条目/字节、
/// 不读内容、不进大项排行**（REQ-AUD-02），但仍会被判定为越界
/// （`MemberGit` / `MemberWorktree`）。
fn change_is_git_face(change: &MetaChange<'_>) -> bool {
    change
        .after
        .or(change.before)
        .map(|meta| meta.in_git)
        .unwrap_or(false)
}

/// 分组键：`<成员仓目录名>/<一级目录或 <文件>>`（如 `pms/target`、
/// `busi/target`、`root/AGENTS.md`）。`.git` 永不入列——`.git` 整棵子树在
/// 计费之前就已被跳过（REQ-AUD-02）。
fn budget_group_key(relative: &str, kind: RootRecipeSnapshotEntryKind) -> String {
    let mut segments = relative.split('/');
    let first = segments.next().unwrap_or("");
    match segments.next() {
        Some(second) => format!("{first}/{second}"),
        // 顶层目录本身就是成员仓目录 → 它自己的组；顶层文件归 `root`。
        None if kind == RootRecipeSnapshotEntryKind::Dir => first.to_string(),
        None => format!("root/{first}"),
    }
}

/// 成员仓归属：顶层文件归 `root`，其余按第一段目录名。
fn budget_member(relative: &str, kind: RootRecipeSnapshotEntryKind) -> String {
    let mut segments = relative.split('/');
    let first = segments.next().unwrap_or("");
    if segments.next().is_none() && kind != RootRecipeSnapshotEntryKind::Dir {
        return "root".to_string();
    }
    first.to_string()
}

/// 预算超限的结构化失败（REQ-AUD-04）：上限、累计字节/条目、越线路径、
/// 大项分组累计与各成员仓贡献一次带齐。
fn budget_failure(
    over_line_path: &str,
    charge: &AuditCharge,
    budget: &RootRecipeSnapshotBudget,
) -> ProductStoreError {
    let mut top_groups: Vec<BudgetGroup> = charge
        .groups
        .iter()
        .map(|(key, bytes)| BudgetGroup {
            key: key.clone(),
            bytes: *bytes,
        })
        .collect();
    top_groups.sort_by(|left, right| {
        right
            .bytes
            .cmp(&left.bytes)
            .then_with(|| left.key.cmp(&right.key))
    });
    top_groups.truncate(10);
    let member_summaries: Vec<MemberBudgetSummary> = charge
        .members
        .iter()
        .map(|(member, totals)| MemberBudgetSummary {
            member: member.clone(),
            bytes: totals.bytes,
            entries: totals.entries,
        })
        .collect();
    ProductStoreError::AuditSnapshotBudgetExceeded(AuditSnapshotBudgetFailure {
        max_bytes: budget.max_bytes,
        cumulative_bytes: charge.bytes,
        over_line_path: over_line_path.to_string(),
        max_entries: budget.max_entries,
        cumulative_entries: charge.entries,
        top_groups,
        member_summaries,
    })
}

// ---------------------------------------------------------------------------
// Streaming content read
// ---------------------------------------------------------------------------

/// 流式读取文件内容：64 KiB 分块，**边读边累加字节、边算 sha256**，累计
/// 越限立刻停止读取（REQ-AUD-03：内存峰值 = 块大小量级，绝不先整读后计费）。
fn stream_file_digest(
    path: &Path,
    budget: &RootRecipeSnapshotBudget,
    charged: &mut u64,
) -> Result<StreamDigest, ProductStoreError> {
    let file = std::fs::File::open(path).map_err(|error| {
        ProductStoreError::Io(format!("audit read {}: {error}", path.display()))
    })?;
    let mut reader = std::io::BufReader::with_capacity(AUDIT_READ_CHUNK, file);
    let mut hasher = Sha256::new();
    let mut buffer = vec![0u8; AUDIT_READ_CHUNK];
#[cfg(test)]
let mut chunks = 0usize;
#[cfg(test)]
let mut read_bytes = 0u64;
    let outcome = loop {
        let read = std::io::Read::read(&mut reader, &mut buffer).map_err(|error| {
            ProductStoreError::Io(format!("audit read {}: {error}", path.display()))
        })?;
        if read == 0 {
            break StreamDigest::Digest(format!("sha256:{:x}", hasher.finalize()));
        }
#[cfg(test)]
{
    chunks += 1;
    read_bytes += read as u64;
}
        *charged += read as u64;
        hasher.update(&buffer[..read]);
        if *charged > budget.max_bytes {
            break StreamDigest::BudgetExceeded;
        }
    };
    #[cfg(test)]
    read_probe::record(&path.to_string_lossy(), chunks, read_bytes);
    Ok(outcome)
}

// ---------------------------------------------------------------------------
// Snapshot projection
// ---------------------------------------------------------------------------

/// 元数据 → 快照条目（新快照不再带内容摘要：内容哈希只在变更清单上算；
/// `content_digest` 字段保留给旧 receipt 反序列化）。
fn entry_of_meta(meta: &SnapshotMeta) -> RootRecipeSnapshotEntry {
    RootRecipeSnapshotEntry {
        path: meta.path.to_string_lossy().into_owned(),
        kind: meta.kind,
        content_digest: None,
        link_target: meta.link_target.clone(),
        special_type: meta.special_type.clone(),
        escapes_root: meta.escapes_root,
    }
}

/// 元数据 → 一次全量快照（条目按路径升序，含整体 digest）。
///
/// **成员 git 面不进快照**（REQ-AUD-02）：`.git` 子树的条目既不进快照、
/// 也不计入条目/字节——它只用于判定「命令试图改 `.git`」这一越界行为。
fn snapshot_from_meta(root: &Path, metas: &[SnapshotMeta]) -> RootRecipeFilesystemSnapshot {
    let entries: Vec<RootRecipeSnapshotEntry> = metas
        .iter()
        .filter(|meta| !meta.in_git)
        .map(entry_of_meta)
        .collect();
    let snapshot_digest = digest_snapshot_entries(&entries);
    RootRecipeFilesystemSnapshot {
        canonical_root: root.to_path_buf(),
        entries,
        snapshot_digest,
    }
}

/// 变更路径的相对路径字符串（canonical 相对、正斜杠）。
fn change_path_string(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}

// ---------------------------------------------------------------------------
// Test probe
// ---------------------------------------------------------------------------

/// 测试探针：记录每次流式读取发生的路径与读取块数——证明「只对变更文件读
/// 内容」以及「越限立刻停（读取次数有上限）」。
///
/// **线程局部**：Rust 测试用例各自跑在自己的线程上 ⇒ 探针天然按用例隔离，
/// 不会把别的用例的读取事件算到自己头上（用例并行时尤其重要）。
#[cfg(test)]
pub(crate) mod read_probe {
    use std::cell::RefCell;

    /// 一次流式读取的观测记录。
    #[derive(Debug, Clone)]
    pub(crate) struct ReadEvent {
        pub path: String,
        pub chunks: usize,
        pub bytes: u64,
    }

    std::thread_local! {
        static READS: RefCell<Vec<ReadEvent>> = const { RefCell::new(Vec::new()) };
    }

    /// 清空探针（每个用例开始前调用）。
    pub(crate) fn reset() {
        READS.with(|reads| reads.borrow_mut().clear());
    }

    /// 记录一次读取。
    pub(crate) fn record(path: &str, chunks: usize, bytes: u64) {
        READS.with(|reads| {
            reads.borrow_mut().push(ReadEvent {
                path: path.to_string(),
                chunks,
                bytes,
            });
        });
    }

    /// 已发生的读取列表（按发生顺序）。
    pub(crate) fn reads() -> Vec<ReadEvent> {
        READS.with(|reads| reads.borrow().clone())
    }
}
