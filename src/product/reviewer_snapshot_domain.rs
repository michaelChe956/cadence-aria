//! 零写对账域(owner 2026-10-23 裁定「乙」/REQ-ENV-REVIEW-B):对账域恰为
//! tracked ∪ untracked 非 gitignore 忽略——`git status` 常规口径;ignored
//! 产物(`node_modules/`、`target/`、构建输出)**不采集、不恢复、不删除、
//! 不判违规**。取代 2026-10-16 v2.0 设计 :54「零写对账不能忽略 ignored」。
//!
//! 已接受代价(报告/审计面须显式披露 [`DOMAIN_BOUNDARY_DISCLOSURE`],
//! 不得声称零写覆盖全部文件):评审对 ignored 产物与 git 不可见形态
//! (FIFO/套接字等 UTF-8 名特殊文件、子仓内部内容——父仓 gitlink 对其
//! 不透明)的修改不被发现、不被恢复、不构成违规;对账域内(tracked ∪
//! untracked 非 ignored)仍严格零写,临时验证产物按「正常验证恢复」语义
//! 不变。排除理由:前端仓实测 ignored 产物使基线达 12,397 条 / 131 MB,
//! 叠加游标持久化写放大后恢复吞吐 1.3 条/秒、写入 13 GB、ETA 2 小时,
//! 不可运维。
//!
//! 域在**基线时刻冻结**(契约 Scenario「对账域由基线冻结的 ignore 规则
//! 界定」):
//! - capture 以受信 Git 桥 `git ls-files --cached --others
//!   --exclude-standard -z` 精确枚举域内文件/symlink(git 是口径唯一
//!   权威:tracked 文件即使命中 ignore 模式仍在域内;FIFO 与空目录对
//!   git 不可见——空目录仍记为 Directory 条目以保 Coder 现场,特殊
//!   文件整条跳过);
//! - 同时把三类 ignore 规则的**内容**(每个非忽略目录的 `.gitignore`、
//!   `.git/info/exclude`、生效 core.excludesFile)与**基线时刻已被
//!   ignore 的路径清单**(`ls-files --others --ignored
//!   --exclude-standard --directory`)冻结进 manifest;
//! - 恢复/清扫/终验对 manifest 外新路径的域判定只认冻结内容——Reviewer
//!   期间改写 `.gitignore`(域内 tracked 文件,自身按域内差异恢复)或
//!   新建 `.gitignore` 都无法缩小对账域;Coder 既有 ignored 产物按冻结
//!   清单豁免,冻结规则只裁决 Reviewer 期间新增路径。
//!
//! 匹配语义复用 Bug2 的 gitignore 子集解析
//! (`logical_codebase::root_recipe_receipt`,2026-10-16):深层覆盖
//! 浅层、同目录 `.gitignore` 覆盖 `.git/info/exclude`、`!` 否定层内最后
//! 命中者胜、被忽略目录不下降;已知简化见该文件头注(倾向多忽略→
//! 域偏小→残留存活,不会误删)。
//!
//! 本模块同时承载 worktree 走查机器(fstatat AT_SYMLINK_NOFOLLOW +
//! openat 锚定,不追随 symlink 到 owner 文件),自 reviewer_snapshot.rs
//! 拆出以守 large_file_guard 行数门禁(同 reviewer_snapshot_cursor 先例)。

use std::collections::{BTreeMap, HashSet};
use std::ffi::OsStr;
use std::io::{Read as _, Write as _};
use std::os::fd::{AsRawFd as _, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::cross_cutting::git_command::{args, run_git};

use crate::product::logical_codebase::root_recipe_receipt::{
    GitignoreLevel, parse_ignore_rules_from_str,
};
use crate::product::reviewer_snapshot::{
    BaselineEntryKind, ReviewerBaselineManifest, ReviewerSnapshotError, SnapshotBudget,
    fstatat_no_follow, io_error_string, map_git_error, open_root_dir_no_follow, openat_dir,
    openat_read_no_follow, read_dir_sorted, readlinkat_target, sha256_hex, unlinkat_dir,
    unlinkat_file,
};

/// 报告/审计面的域边界披露(REQ-ENV-REVIEW-B:不得声称零写覆盖全部文件)。
pub(crate) const DOMAIN_BOUNDARY_DISCLOSURE: &str = "对账域=tracked∪untracked非ignored\
(git status 口径,基线时刻冻结);ignored 产物不采集/不恢复/不删除/不判违规\
(已知边界,owner 2026-10-23 裁定乙)";

/// 冻结 ignore 规则载体的 schema(独立版本)。
pub(crate) const IGNORE_RULES_FREEZE_SCHEMA: &str = "reviewer_ignore_rules_freeze/1";

// ---------------------------------------------------------------------------
// 受信枚举:域内文件与基线时刻已被 ignore 的路径
// ---------------------------------------------------------------------------

fn ls_files_z(worktree: &Path, extra: &[&str]) -> Result<Vec<String>, ReviewerSnapshotError> {
    let mut argv: Vec<&str> = vec!["ls-files", "-z"];
    argv.extend(extra.iter().copied());
    let record = run_git(worktree, &args(&argv)).map_err(map_git_error)?;
    let mut out: Vec<String> = record
        .stdout
        .split('\0')
        .filter(|value| !value.is_empty())
        .map(str::to_string)
        .collect();
    out.sort();
    Ok(out)
}

/// 域内文件/symlink 清单(tracked + untracked 非 ignored;git 口径权威)。
pub(crate) fn enumerate_domain_files(
    worktree: &Path,
) -> Result<HashSet<String>, ReviewerSnapshotError> {
    Ok(
        ls_files_z(worktree, &["--cached", "--others", "--exclude-standard"])?
            .into_iter()
            .collect(),
    )
}

/// 基线时刻已被 ignore 的路径清单(全 ignored 的目录以 `/` 收尾折叠)。
pub(crate) fn enumerate_ignored_paths(
    worktree: &Path,
) -> Result<Vec<String>, ReviewerSnapshotError> {
    ls_files_z(
        worktree,
        &["--others", "--ignored", "--exclude-standard", "--directory"],
    )
}

// ---------------------------------------------------------------------------
// ignore 规则采集(基线时刻磁盘/配置内容)
// ---------------------------------------------------------------------------

/// 三类 ignore 规则源的基线内容(采集自磁盘;冻结进 manifest 后恢复期
/// 只认冻结副本)。
pub(crate) struct IgnoreSources {
    /// 各非忽略目录 `.gitignore` 内容(键=相对正斜杠路径,根为 "")。
    pub(crate) gitignores: BTreeMap<String, String>,
    /// `.git/info/exclude` 内容(缺失为空)。
    pub(crate) info_exclude: String,
    /// 生效 core.excludesFile 全局排除内容(缺失为空)。
    pub(crate) global_exclude: String,
}

fn read_lossy(path: &Path) -> String {
    std::fs::read(path)
        .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
        .unwrap_or_default()
}

fn expand_home(path: &str) -> PathBuf {
    if let Some(rest) = path.strip_prefix("~/")
        && let Ok(home) = std::env::var("HOME")
    {
        return PathBuf::from(home).join(rest);
    }
    PathBuf::from(path)
}

fn read_global_exclude(worktree: &Path) -> String {
    let configured = match run_git(worktree, &args(&["config", "--get", "core.excludesFile"])) {
        Ok(record) => record.stdout.trim().to_string(),
        // 未配置(exit 1)或读失败:按 git 默认路径尽力而为
        Err(_) => String::new(),
    };
    let path = if configured.is_empty() {
        std::env::var("XDG_CONFIG_HOME")
            .ok()
            .map(|root| PathBuf::from(root).join("git/ignore"))
            .or_else(|| {
                std::env::var("HOME")
                    .ok()
                    .map(|home| PathBuf::from(home).join(".config/git/ignore"))
            })
    } else {
        Some(expand_home(&configured))
    };
    path.map(|path| read_lossy(&path)).unwrap_or_default()
}

/// 采集 ignore 规则源:自根向下逐层(复用 Bug2 匹配器判定目录是否被
/// 忽略——被忽略目录整体不下降,与 git 一致;不追 symlink 目录;不进
/// `.git`)读取每个非忽略目录的 `.gitignore`,外加 `.git/info/exclude`
/// 与全局排除。读失败按空规则降级(git 同样尽力而为);采集缺失只会
/// 让恢复期域**偏大**(Reviewer 残留被清除),不会误删 Coder 既有
/// ignored 产物(后者由冻结清单豁免,见 [`IgnoreRulesFreeze`])。
pub(crate) fn collect_ignore_sources(worktree: &Path, gitdir: &Path) -> IgnoreSources {
    let mut gitignores = BTreeMap::new();
    let mut levels: Vec<GitignoreLevel> = Vec::new();
    collect_ignore_dir(worktree, "", &mut levels, &mut gitignores);
    IgnoreSources {
        gitignores,
        info_exclude: read_lossy(&gitdir.join("info/exclude")),
        global_exclude: read_global_exclude(worktree),
    }
}

fn ignored_by_levels(levels: &[GitignoreLevel], rel: &str, is_dir: bool) -> bool {
    levels
        .iter()
        .rev()
        .find_map(|level| level.verdict(rel, is_dir))
        .unwrap_or(false)
}

fn collect_ignore_dir(
    dir_path: &Path,
    scope: &str,
    levels: &mut Vec<GitignoreLevel>,
    gitignores: &mut BTreeMap<String, String>,
) {
    let gitignore_path = dir_path.join(".gitignore");
    let content = read_lossy(&gitignore_path);
    let has_rules = !content.trim().is_empty();
    if has_rules {
        gitignores.insert(scope.to_string(), content);
        let patterns = parse_ignore_rules_from_str(gitignores.get(scope).expect("刚插入"));
        if !patterns.is_empty() {
            levels.push(GitignoreLevel::from_patterns(scope.to_string(), patterns));
        }
    }
    let Ok(entries) = std::fs::read_dir(dir_path) else {
        return;
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name() != ".git")
        .filter_map(|entry| entry.file_name().into_string().ok())
        .collect();
    names.sort();
    for name in names {
        let child = dir_path.join(&name);
        let Ok(metadata) = std::fs::symlink_metadata(&child) else {
            continue;
        };
        if !metadata.is_dir() {
            continue;
        }
        let rel = if scope.is_empty() {
            name.clone()
        } else {
            format!("{scope}/{name}")
        };
        if ignored_by_levels(levels, &rel, true) {
            continue;
        }
        let pushed = levels.len();
        collect_ignore_dir(&child, &rel, levels, gitignores);
        levels.truncate(pushed);
    }
    // 根作用域:无自身 pop(调用方一次性使用)
}

// ---------------------------------------------------------------------------
// 冻结载体与冻结规则匹配
// ---------------------------------------------------------------------------

/// 冻结进 manifest 的 ignore 证据:规则内容 + 基线时刻已被 ignore 的
/// 路径清单。恢复期域判定**只认**这份冻结内容(Reviewer 期间改写
/// `.gitignore`/新建 `.gitignore`/改全局排除都无法缩小对账域)。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct IgnoreRulesFreeze {
    pub(crate) schema: String,
    #[serde(default)]
    pub(crate) gitignores: BTreeMap<String, String>,
    #[serde(default)]
    pub(crate) info_exclude: String,
    #[serde(default)]
    pub(crate) global_exclude: String,
    /// 基线时刻已被 ignore 的路径(git 枚举,全忽略目录以 `/` 收尾)。
    /// Coder 既有 ignored 产物按此豁免删除;规则只裁决新增路径。
    #[serde(default)]
    pub(crate) ignored_paths: Vec<String>,
}

impl IgnoreRulesFreeze {
    /// capture 期冻结(sources=磁盘采集;ignored=git 权威枚举)。
    pub(crate) fn capture(
        sources: &IgnoreSources,
        ignored_paths: Vec<String>,
    ) -> Result<Self, ReviewerSnapshotError> {
        if ignored_paths.iter().any(|path| path.contains('\0')) {
            return Err(ReviewerSnapshotError::Git(
                "ls-files --ignored 输出含 NUL".to_string(),
            ));
        }
        Ok(Self {
            schema: IGNORE_RULES_FREEZE_SCHEMA.to_string(),
            gitignores: sources.gitignores.clone(),
            info_exclude: sources.info_exclude.clone(),
            global_exclude: sources.global_exclude.clone(),
            ignored_paths,
        })
    }

    /// 合成 manifest(合同测试)用的空冻结:无规则、无豁免清单。
    #[cfg(test)]
    pub(crate) fn synthetic() -> Self {
        Self {
            schema: IGNORE_RULES_FREEZE_SCHEMA.to_string(),
            gitignores: BTreeMap::new(),
            info_exclude: String::new(),
            global_exclude: String::new(),
            ignored_paths: Vec::new(),
        }
    }

    fn covers(&self, rel: &str) -> bool {
        self.ignored_paths.iter().any(|ignored| {
            if let Some(prefix) = ignored.strip_suffix('/') {
                rel.starts_with(prefix) && rel.as_bytes().get(prefix.len()) == Some(&b'/')
            } else {
                ignored == rel
            }
        })
    }
}

/// 由冻结(或采集)内容构建的匹配器:层序=浅→深(同深度 global <
/// exclude < `.gitignore`),裁决自深向浅、层内最后命中者胜——与
/// Bug2 遍历栈及 git 优先级一致。
pub(crate) struct FrozenIgnoreRules {
    /// 排序后序:浅→深;同深度 exclude/global 先于 `.gitignore`
    /// (find_map 自深向浅,故同深度后者先被裁决)。
    levels: Vec<GitignoreLevel>,
}

impl FrozenIgnoreRules {
    pub(crate) fn from_sources(sources: &IgnoreSources) -> Self {
        let mut levels = Vec::new();
        let mut push = |content: &str, scope: &str| {
            let patterns = parse_ignore_rules_from_str(content);
            if !patterns.is_empty() {
                levels.push(GitignoreLevel::from_patterns(scope.to_string(), patterns));
            }
        };
        push(&sources.global_exclude, "");
        push(&sources.info_exclude, "");
        for (scope, content) in &sources.gitignores {
            push(content, scope);
        }
        Self::sorted(levels)
    }

    pub(crate) fn from_freeze(freeze: &IgnoreRulesFreeze) -> Self {
        let sources = IgnoreSources {
            gitignores: freeze.gitignores.clone(),
            info_exclude: freeze.info_exclude.clone(),
            global_exclude: freeze.global_exclude.clone(),
        };
        Self::from_sources(&sources)
    }

    fn sorted(mut levels: Vec<GitignoreLevel>) -> Self {
        // 深度升序(稳定排序保持同深度 [global, exclude, gitignore] 入序;
        // find_map 逐层自深向浅,同深度取序末=gitignore 优先,与 git 一致)。
        levels.sort_by_key(|level| level.scope.matches('/').count());
        Self { levels }
    }

    pub(crate) fn is_ignored(&self, rel: &str, is_dir: bool) -> bool {
        self.levels
            .iter()
            .rev()
            .find_map(|level| level.verdict(rel, is_dir))
            .unwrap_or(false)
    }
}

// ---------------------------------------------------------------------------
// 走查期域判定
// ---------------------------------------------------------------------------

/// 走查时每个条目的域判定。
pub(crate) enum ReconciliationDomain<'a> {
    /// capture 期:文件/symlink 以 git 枚举为准;目录以采集规则判
    /// 忽略(空目录不入 git 口径,但保留为 Directory 条目以保 Coder
    /// 现场,恢复不误删)。
    Captured {
        files: &'a HashSet<String>,
        rules: &'a FrozenIgnoreRules,
    },
    /// 恢复/清扫/终验期:manifest 条目恒在域内;基线 ignored 清单
    /// 恒在域外;其余(Reviewer 新增路径)按冻结规则裁决。
    Frozen {
        rels: &'a HashSet<String>,
        dirs: &'a HashSet<String>,
        freeze: &'a IgnoreRulesFreeze,
        rules: &'a FrozenIgnoreRules,
    },
}

impl ReconciliationDomain<'_> {
    pub(crate) fn contains(&self, rel: &str, is_dir: bool) -> bool {
        match self {
            Self::Captured { files, rules } => {
                if is_dir {
                    !rules.is_ignored(rel, true)
                } else {
                    files.contains(rel)
                }
            }
            Self::Frozen {
                rels,
                dirs,
                freeze,
                rules,
            } => {
                if rels.contains(rel) || dirs.contains(rel) {
                    return true;
                }
                if freeze.covers(rel) {
                    return false;
                }
                !rules.is_ignored(rel, is_dir)
            }
        }
    }
}

/// 恢复期冻结域的自持载体(不借用 manifest,游标推进可安全改写)。
pub(crate) struct FrozenDomain {
    pub(crate) rels: HashSet<String>,
    pub(crate) dirs: HashSet<String>,
    freeze: IgnoreRulesFreeze,
    rules: FrozenIgnoreRules,
}

impl FrozenDomain {
    pub(crate) fn from_manifest(manifest: &ReviewerBaselineManifest) -> Self {
        let mut rels = HashSet::new();
        let mut dirs = HashSet::new();
        for entry in &manifest.entries {
            rels.insert(entry.rel_path.clone());
            if entry.kind == BaselineEntryKind::Directory {
                dirs.insert(entry.rel_path.clone());
            }
            let mut end = entry.rel_path.len();
            while let Some(idx) = entry.rel_path[..end].rfind('/') {
                dirs.insert(entry.rel_path[..idx].to_string());
                end = idx;
            }
        }
        let freeze = manifest.ignore_rules_freeze.clone();
        let rules = FrozenIgnoreRules::from_freeze(&freeze);
        Self {
            rels,
            dirs,
            freeze,
            rules,
        }
    }

    pub(crate) fn domain(&self) -> ReconciliationDomain<'_> {
        ReconciliationDomain::Frozen {
            rels: &self.rels,
            dirs: &self.dirs,
            freeze: &self.freeze,
            rules: &self.rules,
        }
    }
}

// ---------------------------------------------------------------------------
// worktree 走查机器(自 reviewer_snapshot.rs 迁入;语义不变处未改动)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct WalkedEntry {
    pub(crate) rel_path: String,
    pub(crate) kind: BaselineEntryKind,
    pub(crate) mode: u32,
    pub(crate) size: u64,
    pub(crate) content_sha256: Option<String>,
    pub(crate) symlink_target: Option<String>,
    /// 套接字/FIFO/设备/非 UTF-8 名:capture 拒绝;restore 视作可删残留。
    pub(crate) unsupported: bool,
}

#[derive(Default)]
struct WalkStats {
    entries: usize,
    bytes: u64,
}

pub(crate) struct BlobSink {
    pub(crate) dir: PathBuf,
}

impl BlobSink {
    /// CAS 写入(create-new;同 sha 已存在=去重,hash 由 restore 预检兜底)。
    pub(crate) fn store(&self, sha_hex: &str, bytes: &[u8]) -> Result<(), ReviewerSnapshotError> {
        let path = self.dir.join(sha_hex);
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Ok(());
            }
            Err(error) => {
                return Err(io_error_string(
                    &format!("blob create {}", path.display()),
                    error,
                ));
            }
        };
        file.write_all(bytes)
            .map_err(|error| io_error_string("blob write", error))?;
        file.sync_all()
            .map_err(|error| io_error_string("blob sync", error))?;
        Ok(())
    }
}

pub(crate) enum WalkPurpose<'a> {
    Capture {
        budget: &'a SnapshotBudget,
        sink: &'a mut BlobSink,
    },
    Observe {
        delete_extras: bool,
        manifest_rels: Option<&'a HashSet<String>>,
        removed: &'a mut Vec<String>,
    },
}

/// `libc::mode_t` 的宽度**随平台而变**（Linux `u32` / macOS `u16`）。
/// 统一收敛到 `u32` 后再比较，避免各处按 Linux 宽度假设 → macOS 编译不过
/// （现场 2026-11-01：release workflow 在 darwin-arm64 上编译失败）。
#[inline]
pub(crate) fn mode_bits(stat_mode: libc::mode_t) -> u32 {
    stat_mode as u32
}

pub(crate) fn classify(stat_mode: u32) -> Option<BaselineEntryKind> {
    // 常量的宽度随平台而变（macOS 为 u16）→ 统一转成 u32 后再当 pattern 用
    // （Rust 不允许在 pattern 里写 `as` 表达式，故取局部 const）。
    const S_IFMT: u32 = libc::S_IFMT as u32;
    const S_IFREG: u32 = libc::S_IFREG as u32;
    const S_IFDIR: u32 = libc::S_IFDIR as u32;
    const S_IFLNK: u32 = libc::S_IFLNK as u32;
    match stat_mode & S_IFMT {
        S_IFREG => Some(BaselineEntryKind::File),
        S_IFDIR => Some(BaselineEntryKind::Directory),
        S_IFLNK => Some(BaselineEntryKind::Symlink),
        _ => None,
    }
}

/// 目录是否含 `.git` 条目(嵌套仓/submodule;含根级子项)。
fn dir_contains_git(parent: &OwnedFd, name: &OsStr) -> bool {
    openat_dir(parent, name)
        .ok()
        .and_then(|child| fstatat_no_follow(&child, OsStr::new(".git")).ok())
        .is_some()
}

pub(crate) fn walk_worktree(
    root: &Path,
    purpose: &mut WalkPurpose<'_>,
    domain: Option<&ReconciliationDomain<'_>>,
) -> Result<Vec<WalkedEntry>, ReviewerSnapshotError> {
    let root_fd = open_root_dir_no_follow(root).map_err(|error| {
        ReviewerSnapshotError::ReadFailure(format!(
            "open worktree root {}: {error}",
            root.display()
        ))
    })?;
    let mut out = Vec::new();
    let mut stats = WalkStats::default();
    walk_dir(&root_fd, "", true, purpose, domain, &mut out, &mut stats)?;
    out.sort_by(|left, right| left.rel_path.cmp(&right.rel_path));
    Ok(out)
}

fn raw_rel_name(name: &OsStr) -> String {
    format!("raw:{}", hex::encode(name.as_bytes()))
}

#[allow(clippy::too_many_arguments)]
fn walk_dir(
    dir: &OwnedFd,
    prefix: &str,
    is_root: bool,
    purpose: &mut WalkPurpose<'_>,
    domain: Option<&ReconciliationDomain<'_>>,
    out: &mut Vec<WalkedEntry>,
    stats: &mut WalkStats,
) -> Result<(), ReviewerSnapshotError> {
    let names = read_dir_sorted(dir).map_err(|error| {
        ReviewerSnapshotError::ReadFailure(format!("readdir {prefix:?}: {error}"))
    })?;
    let delete_extras = matches!(
        purpose,
        WalkPurpose::Observe {
            delete_extras: true,
            ..
        }
    );
    for name in names {
        let name_str = name.to_str().map(str::to_string);
        let rel = name_str.clone().map(|value| {
            if prefix.is_empty() {
                value
            } else {
                format!("{prefix}/{value}")
            }
        });

        let stat = fstatat_no_follow(dir, &name).map_err(|error| {
            ReviewerSnapshotError::ReadFailure(format!(
                "fstatat {}: {error}",
                name.to_string_lossy()
            ))
        })?;
        let mode = mode_bits(stat.st_mode) & 0o7777;
        let size = stat.st_size.max(0) as u64;
        let classified = classify(mode_bits(stat.st_mode));

        // root `.git`:目录=Git 观察域另行冻结,不入 worktree 清单(恢复
        // 也永不触碰);gitfile=身份锚点条目;symlink/特殊文件=不支持。
        let effective_kind = if is_root && name == ".git" {
            match classified {
                Some(BaselineEntryKind::Directory) => {
                    continue;
                }
                Some(BaselineEntryKind::File) => Some(BaselineEntryKind::Gitfile),
                other => other,
            }
        } else {
            classified
        };

        // 对账域外跳过(owner 2026-10-23 裁定乙,REQ-ENV-REVIEW-B):
        // - ignored 产物(按 capture 枚举/冻结规则)——不采集、不恢复、
        //   不删除、不报告;
        // - git 不可见形态(FIFO/套接字/设备,UTF-8 名)——git 口径外,
        //   同按域外处置,不做不支持项拒绝;
        // - root `.git` 锚点(任何形态)永属域内。
        // 非 UTF-8 名无法可靠判域,维持 fail-closed:capture 拒绝、
        // observe 按可删残留处置。
        if let (Some(domain), Some(rel_value)) = (domain, rel.as_ref()) {
            let in_domain = if is_root && name == ".git" {
                true
            } else {
                match classified {
                    Some(BaselineEntryKind::Directory) => domain.contains(rel_value, true),
                    Some(_) => domain.contains(rel_value, false),
                    None => false,
                }
            };
            if !in_domain {
                continue;
            }
        }

        // Observe+delete:manifest 外多余项=Reviewer 残留,删除;Coder 原样
        // 保留域全部在 manifest 内。root `.git`(任何形态)永不由 sweep 处置。
        let in_manifest = rel.as_ref().is_some_and(|value| match purpose {
            WalkPurpose::Observe {
                manifest_rels: Some(set),
                ..
            } => set.contains(value),
            _ => false,
        });
        if delete_extras && !in_manifest && !(is_root && name == ".git") {
            let label = rel.clone().unwrap_or_else(|| raw_rel_name(&name));
            match effective_kind {
                Some(BaselineEntryKind::Directory) => {
                    let child = openat_dir(dir, &name).map_err(|error| {
                        ReviewerSnapshotError::ReadFailure(format!("openat dir {label}: {error}"))
                    })?;
                    // 整删已定罪的残留目录树:内容不再过域滤(其内部若
                    // 有基线文件,父目录必然在 manifest 内,到不了这里)。
                    walk_dir(&child, &label, false, purpose, None, out, stats)?;
                    unlinkat_dir(dir, &name)
                        .map_err(|error| io_error_string("rmdir extra", error))?;
                }
                _ => {
                    unlinkat_file(dir, &name)
                        .or_else(|_| unlinkat_dir(dir, &name))
                        .map_err(|error| io_error_string("unlink extra", error))?;
                }
            }
            if let WalkPurpose::Observe { removed, .. } = purpose {
                removed.push(label);
            }
            continue;
        }

        // 不支持项/非 UTF-8 名:capture 整单拒绝;observe 记为可删残留
        if effective_kind.is_none() || rel.is_none() {
            let label = rel.clone().unwrap_or_else(|| raw_rel_name(&name));
            match purpose {
                WalkPurpose::Capture { .. } => {
                    return Err(ReviewerSnapshotError::UnsupportedEntry(format!(
                        "{label}: 不支持项(非 UTF-8 名或 socket/FIFO/设备)"
                    )));
                }
                WalkPurpose::Observe { .. } => {
                    out.push(WalkedEntry {
                        rel_path: label,
                        kind: BaselineEntryKind::File,
                        mode,
                        size,
                        content_sha256: None,
                        symlink_target: None,
                        unsupported: true,
                    });
                    continue;
                }
            }
        }
        let kind = effective_kind.expect("上方已判空");
        let rel = rel.expect("上方已判空");

        // 预算先行:超限整单拒绝,不截断称完整
        if let WalkPurpose::Capture { budget, .. } = purpose {
            if stats.entries + 1 > budget.max_entries {
                return Err(ReviewerSnapshotError::BudgetExceeded(format!(
                    "目录项数 {} 超预算 {}",
                    stats.entries + 1,
                    budget.max_entries
                )));
            }
            if stats.bytes + size > budget.max_total_bytes {
                return Err(ReviewerSnapshotError::BudgetExceeded(format!(
                    "字节 {} + {size} 超预算 {}",
                    stats.bytes, budget.max_total_bytes
                )));
            }
        }

        match kind {
            BaselineEntryKind::Directory => {
                // 嵌套仓(submodule/自建 repo):父仓 gitlink 口径对其内部
                // 不透明——capture/终验/sweep(域内)只记目录条目不下降,
                // 保 Coder 子仓现场;manifest 外残留的整删走上方 extras
                // 分支(递归不带域滤,内部含 `.git` 也整树清除)。
                let opaque_nested_repo = domain.is_some() && dir_contains_git(dir, &name);
                stats.entries += 1;
                out.push(WalkedEntry {
                    rel_path: rel.clone(),
                    kind,
                    mode,
                    size: 0,
                    content_sha256: None,
                    symlink_target: None,
                    unsupported: false,
                });
                if !opaque_nested_repo {
                    let child = openat_dir(dir, &name).map_err(|error| {
                        ReviewerSnapshotError::ReadFailure(format!("openat dir {rel}: {error}"))
                    })?;
                    walk_dir(&child, &rel, false, purpose, domain, out, stats)?;
                }
            }
            BaselineEntryKind::File | BaselineEntryKind::Gitfile => {
                let mut file = openat_read_no_follow(dir, &name).map_err(|error| {
                    ReviewerSnapshotError::ReadFailure(format!("open {rel}: {error}"))
                })?;
                let open_stat: libc::stat = {
                    let mut value: libc::stat = unsafe { std::mem::zeroed() };
                    if unsafe { libc::fstat(file.as_raw_fd(), &mut value) } != 0 {
                        return Err(ReviewerSnapshotError::ReadFailure(format!(
                            "fstat {rel}: {}",
                            std::io::Error::last_os_error()
                        )));
                    }
                    value
                };
                if mode_bits(open_stat.st_mode) & (libc::S_IFMT as u32) != libc::S_IFREG as u32 {
                    return Err(ReviewerSnapshotError::ReadFailure(format!(
                        "{rel}: 打开后类型漂移"
                    )));
                }
                let mut bytes = Vec::with_capacity(open_stat.st_size.max(0) as usize);
                file.read_to_end(&mut bytes).map_err(|error| {
                    ReviewerSnapshotError::ReadFailure(format!("read {rel}: {error}"))
                })?;
                if bytes.len() as i64 != open_stat.st_size {
                    return Err(ReviewerSnapshotError::ReadFailure(format!(
                        "{rel}: 读取期间大小漂移"
                    )));
                }
                let sha = sha256_hex(&bytes);
                if let WalkPurpose::Capture { sink, .. } = purpose {
                    sink.store(&sha, &bytes)?;
                }
                stats.entries += 1;
                stats.bytes += bytes.len() as u64;
                out.push(WalkedEntry {
                    rel_path: rel,
                    kind,
                    mode: mode_bits(open_stat.st_mode) & 0o7777,
                    size: bytes.len() as u64,
                    content_sha256: Some(sha),
                    symlink_target: None,
                    unsupported: false,
                });
            }
            BaselineEntryKind::Symlink => {
                let target_os = readlinkat_target(dir, &name).map_err(|error| {
                    ReviewerSnapshotError::ReadFailure(format!("readlink {rel}: {error}"))
                })?;
                let Some(target) = target_os.to_str().map(str::to_string) else {
                    match purpose {
                        WalkPurpose::Capture { .. } => {
                            return Err(ReviewerSnapshotError::UnsupportedEntry(format!(
                                "{rel}: symlink target 非 UTF-8"
                            )));
                        }
                        WalkPurpose::Observe { .. } => {
                            out.push(WalkedEntry {
                                rel_path: rel,
                                kind,
                                mode: 0o777,
                                size: 0,
                                content_sha256: None,
                                symlink_target: None,
                                unsupported: true,
                            });
                            continue;
                        }
                    }
                };
                stats.entries += 1;
                out.push(WalkedEntry {
                    rel_path: rel,
                    kind,
                    mode: 0o777,
                    size: 0,
                    content_sha256: None,
                    symlink_target: Some(target),
                    unsupported: false,
                });
            }
        }
    }
    Ok(())
}
