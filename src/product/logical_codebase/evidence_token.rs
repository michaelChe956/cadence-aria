// C-4 跨仓只读证据中介：attempt 级会话令牌（Task 3）。
//
// 契约 REQ-COD-05「受控接口」；设计 §4.1。attempt 启动/恢复时幂等生成/重写
// 32 字节随机令牌（解决 issue 级 worktree 跨 attempt 复用下旧令牌残留）：
// 原文只写入 worktree `.aria/evidence-token`（0600），Aria 侧 attempt 分区只存
// SHA-256 哈希（`evidence-token.json`：attempt_id/token_hash/created_at）。
// 同时向仓库公共 exclude（`<common-gitdir>/info/exclude`，经
// `git rev-parse --git-common-dir` 解析——repo_path 是 linked worktree 时其
// `.git` 是 gitdir 指针文件，直接 join 会写进 git 不读的路径）幂等追加
// 平台自产本地产物目录忽略行（单一事实源 `local_artifacts` 模块；写入以
// `<common-gitdir>/aria-exclude.lock` 文件锁互斥，temp+rename 原子落盘），
// 确保注入文件不进 `git add -A`、不进 commit（令牌明文外泄向量闭合），且
// 执行 agent 从一开始 `git status` 就是干净的——**不需要自己去改
// `.gitignore`**（`.gitignore` 是被跟踪文件，改它会撞 Work Item 写入范围门禁
// `work_item_diff_scope_violation`）。全仓排除这些目录语义正确：它们都是
// Aria/工具链在整仓（主 checkout 与所有 worktree）的注入/索引/验证产物目录。
//
// 本 Task 只实现函数，不接挂钩（挂钩在 T8）。函数签名相对简报补充显式依赖注入
// `paths: &ProductAppPaths`（attempt 分区路径来源）与 `repo_path: &Path`
// （worktree 路径经 `worktree_path_for_attempt(repo_path, attempt)` 推导），
// 与 evidence 模块既有 `EvidenceIndexQuery::new` 的显式依赖注入惯例一致。

use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::cross_cutting::document_ops::compute_sha256;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::ExclusiveFileLock;
use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionAttempt};
use crate::product::coding_workspace_engine::worktree_path_for_attempt;
use crate::product::json_store::{read_json, write_json};
use crate::product::local_artifacts;
use crate::product::logical_codebase::evidence_index::EvidenceError;

/// worktree 注入目录名：`.aria`（exclude 以目录语义忽略整个注入目录）。
const ARIA_DIR_NAME: &str = ".aria";
/// exclude 写入临界区的锁文件名，落公共 gitdir 根（见 [`ensure_repo_exclude`]）：
/// 主 checkout 与所有 linked worktree 解析到同一 gitdir → 同一把锁。
const ARIA_EXCLUDE_LOCK_FILE: &str = "aria-exclude.lock";
/// 令牌文件相对 worktree 根的路径：`.aria/evidence-token`。
const EVIDENCE_TOKEN_FILE: &str = "evidence-token";
/// attempt 分区令牌哈希记录文件名。
///
/// `pub(crate)`：T6 `resolve_attempt_by_token` 需按此名扫描各 attempt 分区。
pub(crate) const EVIDENCE_TOKEN_RECORD_FILE: &str = "evidence-token.json";
/// 令牌原始字节数（32 字节 → 64 位 hex）。
const TOKEN_BYTES: usize = 32;

/// C2 Task 10（REQ-ENV-C2-POLICY）：受限政策读取授权 claims——绑定
/// attempt＋role＋policy digest，附过期时间；仅在该次返修 run 运行态期间
/// 有效（运行态由 `validate_evidence_token` 的 Running 校验承担）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvidenceTokenClaims {
    pub attempt_id: String,
    pub role: String,
    pub policy_digest: String,
    pub expires_at: String,
}

/// Aria 侧 attempt 分区的令牌哈希记录（存 SHA-256，非明文）。
///
/// 字段名与设计 §4.1 一致（snake_case，serde 显式声明）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvidenceTokenRecord {
    pub attempt_id: String,
    pub token_hash: String,
    pub created_at: String,
    /// C2 Task 10 additive：受限政策读取授权；旧记录无 claims（serde
    /// default）→ 不支持受限读取，不得误放行。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claims: Option<EvidenceTokenClaims>,
}

/// 生成/重写 attempt 级会话令牌：写 worktree `.aria/evidence-token`（0600）、
/// 向仓库公共 exclude（`<common-gitdir>/info/exclude`）幂等追加平台自产
/// 本地产物目录忽略行（`local_artifacts` 单一事实源）、写 attempt 分区哈希
/// 记录，返回 64 位 hex 令牌原文。同 attempt 二次调用生成新令牌并覆盖旧值
/// （旧令牌立即失效）。
///
/// `paths` 提供 Aria 侧 attempt 分区根（`issue_lifecycle_root/coding-attempts/{id}`）；
/// `repo_path` 为物理仓库根（主 checkout 或 linked worktree 均可，公共 gitdir
/// 解析见 [`ensure_repo_exclude`]），worktree 路径经 `worktree_path_for_attempt` 推导。
///
/// 调用时机：`execute_worktree_prepare` 在 worktree 创建完成后立即调用本函数，
/// 故 exclude 在执行 agent 启动前即已就位——agent 一开始 `git status` 就是干净的，
/// 不需要自己去改 `.gitignore`（那会撞 Work Item 写入范围门禁）。
pub fn issue_evidence_token(
    paths: &ProductAppPaths,
    repo_path: &Path,
    attempt: &CodingExecutionAttempt,
) -> Result<String, EvidenceError> {
    let token = generate_token();
    let worktree = worktree_path_for_attempt(repo_path, attempt);

    // 先落 exclude：令牌文件写失败（worktree 目录缺失等）不得连带跳过 exclude，
    // 否则 agent 会看到未忽略的本地产物噪音。
    ensure_repo_exclude(repo_path)?;
    write_token_file(&worktree, &token)?;

    let record = EvidenceTokenRecord {
        attempt_id: attempt.id.clone(),
        token_hash: compute_sha256(token.as_bytes()),
        created_at: Utc::now().to_rfc3339(),
        claims: None,
    };
    write_json(&attempt_record_path(paths, attempt), &record).map_err(|error| {
        EvidenceError::Io {
            message: format!("write evidence token record: {error}"),
        }
    })?;

    Ok(token)
}

/// 校验提交的令牌：attempt 必须 Running（否则 `evidence_forbidden`）；比对
/// attempt 分区存储的 SHA-256 哈希（缺失/不匹配 → `evidence_unauthorized`）。
pub fn validate_evidence_token(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    token: &str,
) -> Result<(), EvidenceError> {
    if attempt.status != CodingAttemptStatus::Running {
        return Err(EvidenceError::Forbidden);
    }

    let record_path = attempt_record_path(paths, attempt);
    if !record_path.exists() {
        return Err(EvidenceError::Unauthorized);
    }
    let record: EvidenceTokenRecord =
        read_json(&record_path).map_err(|error| EvidenceError::Io {
            message: format!(
                "read evidence token record {}: {error}",
                record_path.display()
            ),
        })?;

    let token_hash = compute_sha256(token.as_bytes());
    if constant_time_hash_eq(&record.token_hash, &token_hash) {
        Ok(())
    } else {
        Err(EvidenceError::Unauthorized)
    }
}

/// Compare digest strings without an early exit so a matching prefix does not
/// reveal how many leading characters were correct. Hashes are expected to be
/// fixed-width SHA-256 hex, but length is folded into the result defensively.
fn constant_time_hash_eq(left: &str, right: &str) -> bool {
    let mut difference = left.len() ^ right.len();
    for index in 0..left.len().max(right.len()) {
        difference |= (left.as_bytes().get(index).copied().unwrap_or(0)
            ^ right.as_bytes().get(index).copied().unwrap_or(0)) as usize;
    }
    difference == 0
}

/// C2 Task 10：为该 attempt 的下一次返修 run 签发受限政策读取授权（旋转
/// 旧令牌——attempt 记录被覆盖，旧授权立即失效；新记录携带 claims：绑定
/// attempt＋role＋policy digest＋过期时间）。worktree 路径未知时只写 attempt
/// 分区记录（worktree 令牌文件由调用方停等处理）；`.aria/` exclude 已在
/// attempt 首次 worktree 创建时幂等追加，无需重复写入。
pub fn issue_policy_reauthorization(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    role: &str,
    policy_digest: &str,
    expires_at: String,
) -> Result<String, EvidenceError> {
    let token = generate_token();
    if let Some(worktree) = attempt.worktree_path.as_deref() {
        write_token_file(worktree, &token)?;
    }
    let record = EvidenceTokenRecord {
        attempt_id: attempt.id.clone(),
        token_hash: compute_sha256(token.as_bytes()),
        created_at: Utc::now().to_rfc3339(),
        claims: Some(EvidenceTokenClaims {
            attempt_id: attempt.id.clone(),
            role: role.to_string(),
            policy_digest: policy_digest.to_string(),
            expires_at,
        }),
    };
    write_json(&attempt_record_path(paths, attempt), &record).map_err(|error| {
        EvidenceError::Io {
            message: format!("write evidence token record: {error}"),
        }
    })?;
    Ok(token)
}

/// C2 Task 10：读取 attempt 分区令牌记录的受限读取授权 claims。旧记录无
/// claims（serde default）→ `Ok(None)`：不支持受限读取，不得误放行；记录
/// 缺失 → `Unauthorized`（与 `validate_evidence_token` 同口径）。
pub(crate) fn load_evidence_token_claims(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<Option<EvidenceTokenClaims>, EvidenceError> {
    let record_path = attempt_record_path(paths, attempt);
    if !record_path.exists() {
        return Err(EvidenceError::Unauthorized);
    }
    let record: EvidenceTokenRecord =
        read_json(&record_path).map_err(|error| EvidenceError::Io {
            message: format!(
                "read evidence token record {}: {error}",
                record_path.display()
            ),
        })?;
    Ok(record.claims)
}

/// 生成 32 字节随机令牌并编码为 64 位小写 hex（两个 v4 UUID 拼接）。
fn generate_token() -> String {
    let mut bytes = [0u8; TOKEN_BYTES];
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    bytes[..16].copy_from_slice(first.as_bytes());
    bytes[16..].copy_from_slice(second.as_bytes());
    hex::encode(bytes)
}

/// Aria 侧 attempt 分区令牌哈希记录路径：
/// `issue_lifecycle_root/{project}/{issue}/coding-attempts/{attempt_id}/evidence-token.json`
/// （与 `coding_attempt_store` 的 `attempt_dir` 模式一致）。
fn attempt_record_path(paths: &ProductAppPaths, attempt: &CodingExecutionAttempt) -> PathBuf {
    paths
        .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join(EVIDENCE_TOKEN_RECORD_FILE)
}

/// 写 worktree `.aria/evidence-token`（目录自动创建，0600）。
fn write_token_file(worktree: &Path, token: &str) -> Result<(), EvidenceError> {
    let dir = worktree.join(ARIA_DIR_NAME);
    std::fs::create_dir_all(&dir).map_err(|error| EvidenceError::Io {
        message: format!("create {}: {error}", dir.display()),
    })?;
    let path = dir.join(EVIDENCE_TOKEN_FILE);
    write_private_file(&path, token.as_bytes())
}

#[cfg(unix)]
fn write_private_file(path: &Path, content: &[u8]) -> Result<(), EvidenceError> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};

    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| EvidenceError::Io {
            message: format!("create {}: {error}", path.display()),
        })?;
    file.write_all(content).map_err(|error| EvidenceError::Io {
        message: format!("write {}: {error}", path.display()),
    })?;
    file.sync_all().map_err(|error| EvidenceError::Io {
        message: format!("sync {}: {error}", path.display()),
    })?;
    drop(file);
    // 兜底 chmod，确保不受 umask 影响。
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
        EvidenceError::Io {
            message: format!("chmod {}: {error}", path.display()),
        }
    })?;
    Ok(())
}

#[cfg(not(unix))]
fn write_private_file(path: &Path, content: &[u8]) -> Result<(), EvidenceError> {
    std::fs::write(path, content).map_err(|error| EvidenceError::Io {
        message: format!("write {}: {error}", path.display()),
    })
}

/// 向仓库公共 exclude 幂等追加平台自产本地产物目录忽略行（单一事实源
/// [`local_artifacts`]，附一段说明注释 [`local_artifacts::notice_text`]）。
///
/// 落点 `<common-gitdir>/info/exclude`（B1）：exclude 属整个仓库（主 checkout
/// 与所有 linked worktree 共读，`--git-path info/exclude` 也解析到 common
/// dir）。公共 gitdir 经 `git rev-parse --git-common-dir` 解析——repo_path 是
/// linked worktree 时其 `.git` 是 gitdir 指针文件而非目录，直接 join 会把
/// exclude 写进 git 根本不读的路径（见 [`common_git_dir`]）。
///
/// 语义：
/// - **幂等**：缺失的行才追加，重复调用不重复写、不重复行；说明注释在首次
///   追加时写一行，行齐全时不重写（不产生写抖动）。
/// - **互斥**（P1-2）：read→算缺失→write 整个临界区持
///   `<common-gitdir>/aria-exclude.lock` 排它文件锁——同一仓库的并发 attempt
///   解析到同一锁文件，并发追加的重复行/互相覆盖在此闭合；落盘用
///   temp+rename 原子替换（见 [`write_exclude_atomically`]）。
/// - **不覆盖用户已有内容**：用户自己写在 `info/exclude` 里的行原样保留，
///   平台只在末尾追加缺失行。
/// - **不动 `.gitignore`**：`.gitignore` 是被跟踪文件，改它会撞 Work Item 写入
///   范围门禁。
fn ensure_repo_exclude(repo_path: &Path) -> Result<(), EvidenceError> {
    let git_dir = common_git_dir(repo_path);
    let exclude_path = git_dir.join("info").join("exclude");
    let lock_path = git_dir.join(ARIA_EXCLUDE_LOCK_FILE);

    let _lock = ExclusiveFileLock::acquire_at(&lock_path).map_err(|error| EvidenceError::Io {
        message: format!("lock {}: {error}", lock_path.display()),
    })?;

    if let Some(parent) = exclude_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| EvidenceError::Io {
            message: format!("create {}: {error}", parent.display()),
        })?;
    }

    let mut content = match std::fs::read_to_string(&exclude_path) {
        Ok(content) => content,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(error) => {
            return Err(EvidenceError::Io {
                message: format!("read {}: {error}", exclude_path.display()),
            });
        }
    };

    let missing: Vec<String> = local_artifacts::exclude_lines()
        .filter(|line| {
            !content
                .lines()
                .any(|existing| same_ignore_pattern(existing, line))
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    let notice = local_artifacts::notice_text();
    if !content.lines().any(|existing| existing.trim() == notice) {
        content.push_str(notice);
        content.push('\n');
    }
    for line in missing {
        content.push_str(&line);
        content.push('\n');
    }

    write_exclude_atomically(&exclude_path, &content)
}

/// 解析 repo_path 的公共 gitdir（B1）：`git rev-parse --git-common-dir`。
///
/// 主 checkout 返回相对路径 `.git`（相对 repo_path 解析）；linked worktree
/// 返回主仓库 gitdir 的绝对路径。git 不可用/不是仓库（轻量 fixture、裸目录）
/// 时回退 `repo_path/.git`，与主 checkout 的解析结果一致。
fn common_git_dir(repo_path: &Path) -> PathBuf {
    let output = std::process::Command::new("git")
        .args(["rev-parse", "--git-common-dir"])
        .current_dir(repo_path)
        .output();
    match output {
        Ok(output) if output.status.success() => {
            let value = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if value.is_empty() {
                repo_path.join(".git")
            } else if Path::new(&value).is_absolute() {
                PathBuf::from(value)
            } else {
                repo_path.join(value)
            }
        }
        _ => repo_path.join(".git"),
    }
}

/// 锁内原子写 exclude：先写同目录临时文件再 rename 替换（P1-2）。读者
/// （git）任一时刻看到的都是完整的旧文件或完整的新文件，不存在半截内容。
/// 临时文件名固定：所有写者都持 `aria-exclude.lock`，无并发写者；崩溃残留
/// 会被下一次写入覆盖清理。
fn write_exclude_atomically(exclude_path: &Path, content: &str) -> Result<(), EvidenceError> {
    let temp_path = exclude_path.with_file_name("exclude.aria.tmp");
    std::fs::write(&temp_path, content.as_bytes()).map_err(|error| EvidenceError::Io {
        message: format!("write {}: {error}", temp_path.display()),
    })?;
    std::fs::rename(&temp_path, exclude_path).map_err(|error| EvidenceError::Io {
        message: format!(
            "rename {} -> {}: {error}",
            temp_path.display(),
            exclude_path.display()
        ),
    })
}

/// 比较两条 ignore 模式是否等价：尾斜杠差异不算不同（git 语义下 `.aria` 与
/// `.aria/` 对目录的忽略效果一致），空白差异也不算不同。
fn same_ignore_pattern(existing: &str, desired: &str) -> bool {
    existing.trim().trim_end_matches('/') == desired.trim_end_matches('/')
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::process::Command as StdCommand;

    use tempfile::TempDir;

    use super::*;
    use crate::product::coding_models::{
        CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    };
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const ATTEMPT_ID: &str = "coding_attempt_0001";

    fn attempt_fixture(status: CodingAttemptStatus) -> CodingExecutionAttempt {
        CodingExecutionAttempt {
            id: ATTEMPT_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            scope: CodingAttemptScope::WorkItem,
            status,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            admission_kind: crate::product::coding_models::CodingAdmissionKind::LegacyGroup,
            stage: CodingExecutionStage::Coding,
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            rework_count: 0,
            max_auto_rework: 0,
            work_item_group_id: None,
            current_work_item_id: Some("work_item_0001".to_string()),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            target_snapshot: None,
            completed_at: None,
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            start_claim: None,
        }
    }

    /// 轻量 fixture：构造 repo 根 + issue 级 worktree 目录，无需真实 git 仓库
    /// ——`common_git_dir` 对非仓库回退 `repo_path/.git`，exclude 仍写入其
    /// `info/exclude`。
    fn fixture() -> (TempDir, ProductAppPaths, PathBuf) {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        let worktree = repo
            .join(".worktrees")
            .join("aria-issues")
            .join("issue_0001");
        fs::create_dir_all(&worktree).expect("create worktree dir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        (tmp, paths, repo)
    }

    fn worktree_path(repo: &Path) -> PathBuf {
        repo.join(".worktrees")
            .join("aria-issues")
            .join("issue_0001")
    }

    fn repo_exclude_path(repo: &Path) -> PathBuf {
        repo.join(".git").join("info").join("exclude")
    }

    fn attempt_record_path(paths: &ProductAppPaths, attempt: &CodingExecutionAttempt) -> PathBuf {
        paths
            .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join(&attempt.id)
            .join(EVIDENCE_TOKEN_RECORD_FILE)
    }

    fn git(repo: &Path, args: &[&str]) -> String {
        let output = StdCommand::new("git")
            .args(args)
            .current_dir(repo)
            .output()
            .expect("run git fixture command");
        assert!(
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).to_string()
    }

    #[test]
    fn issue_writes_64_hex_token_and_hashed_record() {
        let (_tmp, paths, repo) = fixture();
        let attempt = attempt_fixture(CodingAttemptStatus::Running);

        let token = issue_evidence_token(&paths, &repo, &attempt).expect("issue token");

        // 64 位纯 hex 令牌。
        assert_eq!(token.len(), 64);
        assert!(
            token.chars().all(|c| c.is_ascii_hexdigit()),
            "token must be hex: {token}"
        );

        // worktree 内写入明文令牌，权限 0600。
        let token_file = worktree_path(&repo)
            .join(ARIA_DIR_NAME)
            .join(EVIDENCE_TOKEN_FILE);
        assert_eq!(
            fs::read_to_string(&token_file).expect("read token file"),
            token
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&token_file)
                .expect("token metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "token file must be 0600, got {mode:o}");
        }

        // Aria 侧只存 SHA-256 哈希，不含明文。
        let record_path = attempt_record_path(&paths, &attempt);
        let record: EvidenceTokenRecord = read_json(&record_path).expect("read token record");
        assert_eq!(record.attempt_id, ATTEMPT_ID);
        assert_eq!(record.token_hash, compute_sha256(token.as_bytes()));
        assert!(!record.created_at.is_empty());

        let raw = fs::read_to_string(&record_path).expect("read record raw");
        assert!(
            !raw.contains(&token),
            "record must not store token plaintext"
        );
    }

    #[test]
    fn reissue_rotates_token_and_updates_record() {
        let (_tmp, paths, repo) = fixture();
        let attempt = attempt_fixture(CodingAttemptStatus::Running);

        let first = issue_evidence_token(&paths, &repo, &attempt).expect("first issue");
        let second = issue_evidence_token(&paths, &repo, &attempt).expect("second issue");

        assert_ne!(first, second, "second issue must rotate the token");

        let token_file = worktree_path(&repo)
            .join(ARIA_DIR_NAME)
            .join(EVIDENCE_TOKEN_FILE);
        assert_eq!(
            fs::read_to_string(&token_file).expect("read token file"),
            second,
            "token file must hold the latest token"
        );

        let record: EvidenceTokenRecord =
            read_json(&attempt_record_path(&paths, &attempt)).expect("read record");
        assert_eq!(record.token_hash, compute_sha256(second.as_bytes()));
        assert_ne!(
            record.token_hash,
            compute_sha256(first.as_bytes()),
            "record must be updated to the latest token hash"
        );
    }

    #[test]
    fn constant_time_hash_comparison_matches_only_equal_hashes() {
        assert!(constant_time_hash_eq("same-hash", "same-hash"));
        assert!(!constant_time_hash_eq("same-hash", "different"));
        assert!(!constant_time_hash_eq("same-hash", "same-has"));
        assert!(!constant_time_hash_eq(&"\0".repeat(256), ""));
    }

    #[test]
    fn validate_accepts_correct_token() {
        let (_tmp, paths, repo) = fixture();
        let attempt = attempt_fixture(CodingAttemptStatus::Running);

        let token = issue_evidence_token(&paths, &repo, &attempt).expect("issue token");

        assert!(validate_evidence_token(&paths, &attempt, &token).is_ok());
    }

    #[test]
    fn validate_rejects_wrong_token() {
        let (_tmp, paths, repo) = fixture();
        let attempt = attempt_fixture(CodingAttemptStatus::Running);

        let token = issue_evidence_token(&paths, &repo, &attempt).expect("issue token");
        let first_char = token.chars().next().expect("token is non-empty");
        let replacement = if first_char == 'a' { 'b' } else { 'a' };
        let wrong = format!("{replacement}{}", &token[1..]);

        let err = validate_evidence_token(&paths, &attempt, &wrong).expect_err("wrong token");
        assert_eq!(err, EvidenceError::Unauthorized);
    }

    #[test]
    fn validate_forbids_non_running_attempt() {
        let (_tmp, paths, repo) = fixture();
        let attempt = attempt_fixture(CodingAttemptStatus::Running);

        let token = issue_evidence_token(&paths, &repo, &attempt).expect("issue token");

        let mut completed = attempt.clone();
        completed.status = CodingAttemptStatus::Completed;

        let err =
            validate_evidence_token(&paths, &completed, &token).expect_err("non-running attempt");
        assert_eq!(err, EvidenceError::Forbidden);
    }

    #[test]
    fn exclude_append_is_idempotent() {
        let (_tmp, paths, repo) = fixture();
        let attempt = attempt_fixture(CodingAttemptStatus::Running);

        issue_evidence_token(&paths, &repo, &attempt).expect("first issue");
        issue_evidence_token(&paths, &repo, &attempt).expect("second issue");

        let exclude = fs::read_to_string(repo_exclude_path(&repo)).expect("read exclude");
        assert_eq!(
            exclude
                .lines()
                .filter(|line| line.trim() == ".aria/")
                .count(),
            1,
            "exclude must contain exactly one `.aria/` line, got: {exclude}"
        );
    }

    #[test]
    fn git_add_excludes_evidence_token() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        git(&repo, &["init"]);
        git(&repo, &["config", "user.email", "test@example.com"]);
        git(&repo, &["config", "user.name", "Test User"]);
        fs::write(repo.join("README.md"), "base\n").expect("write base");
        git(&repo, &["add", "README.md"]);
        git(&repo, &["commit", "-m", "base"]);

        let worktree = worktree_path(&repo);
        let worktree_str = worktree.to_string_lossy().to_string();
        git(
            &repo,
            &[
                "worktree",
                "add",
                &worktree_str,
                "-b",
                "aria/issues/issue_0001",
            ],
        );

        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let attempt = attempt_fixture(CodingAttemptStatus::Running);
        let token = issue_evidence_token(&paths, &repo, &attempt).expect("issue token");

        let token_file = worktree.join(ARIA_DIR_NAME).join(EVIDENCE_TOKEN_FILE);
        assert_eq!(
            fs::read_to_string(&token_file).expect("read token file"),
            token
        );

        // 公共 exclude（<repo>/.git/info/exclude）已写入 `.aria/` 一行。
        let exclude = fs::read_to_string(repo_exclude_path(&repo)).expect("read common exclude");
        assert!(
            exclude.lines().any(|line| line.trim() == ".aria/"),
            "common exclude must contain `.aria/`, got: {exclude}"
        );

        git(&worktree, &["add", "-A"]);
        let staged = git(&worktree, &["diff", "--cached", "--name-only"]);
        assert!(
            !staged.contains(EVIDENCE_TOKEN_FILE),
            "git add -A must not stage the injected token file, staged: {staged}"
        );
    }

    /// 平台自产本地产物目录及其现场产物（与 `local_artifacts` 单一事实源同源）。
    const LOCAL_ARTIFACT_DIRS: [(&str, &str); 3] = [
        (".codegraph", "index.db"),
        (".verify", "report.txt"),
        (".worktrees/aria-issues/issue_0001", "worktree.txt"),
    ];

    fn init_repo_with_commit(repo: &Path) {
        fs::create_dir_all(repo).expect("create repo");
        git(repo, &["init"]);
        git(repo, &["config", "user.email", "test@example.com"]);
        git(repo, &["config", "user.name", "Test User"]);
        fs::write(repo.join("README.md"), "base\n").expect("write base");
        git(repo, &["add", "README.md"]);
        git(repo, &["commit", "-m", "base"]);
    }

    /// ① 平台自产本地产物目录不进 `git status`；③ `.gitignore` 字节不变。
    #[test]
    fn local_artifact_dirs_stay_out_of_git_status_and_gitignore_is_untouched() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        init_repo_with_commit(&repo);
        let gitignore_path = repo.join(".gitignore");
        fs::write(&gitignore_path, "node_modules/\n").expect("write gitignore");
        git(&repo, &["add", ".gitignore"]);
        git(&repo, &["commit", "-m", "gitignore"]);
        let gitignore_before = fs::read(&gitignore_path).expect("read gitignore before");

        ensure_repo_exclude(&repo).expect("ensure exclude");

        for (dir, artifact) in LOCAL_ARTIFACT_DIRS {
            let path = repo.join(dir);
            fs::create_dir_all(&path).expect("create artifact dir");
            fs::write(path.join(artifact), b"x").expect("write artifact");
        }

        let status = git(&repo, &["status", "--porcelain"]);
        assert!(
            status.trim().is_empty(),
            "local artifact dirs must not show up in git status, got: {status}"
        );

        assert_eq!(
            fs::read(&gitignore_path).expect("read gitignore after"),
            gitignore_before,
            "platform must never touch the tracked .gitignore"
        );

        let exclude = fs::read_to_string(repo_exclude_path(&repo)).expect("read exclude");
        for line in local_artifacts::exclude_lines() {
            assert!(
                exclude.lines().any(|existing| existing.trim() == line),
                "exclude must contain `{line}`, got: {exclude}"
            );
        }
    }

    /// ② 幂等：重复调用不产生重复行。
    #[test]
    fn local_artifact_exclude_lines_are_appended_idempotently() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(repo.join(".git").join("info")).expect("create git info dir");

        ensure_repo_exclude(&repo).expect("first ensure");
        ensure_repo_exclude(&repo).expect("second ensure");
        ensure_repo_exclude(&repo).expect("third ensure");

        let exclude = fs::read_to_string(repo_exclude_path(&repo)).expect("read exclude");
        for line in local_artifacts::exclude_lines() {
            assert_eq!(
                exclude
                    .lines()
                    .filter(|existing| existing.trim() == line)
                    .count(),
                1,
                "exclude must contain exactly one `{line}` line, got: {exclude}"
            );
        }
    }

    /// ④ 用户自己写在 `info/exclude` 里的内容必须原样保留（平台只追加缺失行）。
    #[test]
    fn user_exclude_content_is_preserved_when_appending_local_artifact_lines() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(repo.join(".git").join("info")).expect("create git info dir");
        let exclude_path = repo_exclude_path(&repo);
        fs::write(&exclude_path, "# user notes\nmy-local-scratch/\n").expect("write user exclude");

        ensure_repo_exclude(&repo).expect("ensure exclude");

        let exclude = fs::read_to_string(&exclude_path).expect("read exclude");
        assert!(
            exclude.contains("# user notes"),
            "user comment must survive, got: {exclude}"
        );
        assert!(
            exclude.contains("my-local-scratch/"),
            "user ignore line must survive, got: {exclude}"
        );
        let user_offset = exclude
            .find("my-local-scratch/")
            .expect("user line present");
        let platform_offset = exclude.find(".aria/").expect("platform line present");
        assert!(
            user_offset < platform_offset,
            "platform lines must be appended after user content, got: {exclude}"
        );
    }

    /// 端到端：真实 worktree 内新建 `.codegraph/x` 与 `.verify/y` →
    /// worktree 与主 checkout 的 `git status --porcelain` 均为空。
    #[test]
    fn real_worktree_local_artifacts_are_invisible_to_git_status() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        init_repo_with_commit(&repo);

        let worktree = repo
            .join(".worktrees")
            .join("aria-issues")
            .join("issue_0001");
        let worktree_str = worktree.to_string_lossy().to_string();
        git(
            &repo,
            &[
                "worktree",
                "add",
                &worktree_str,
                "-b",
                "aria/issues/issue_0001",
            ],
        );

        ensure_repo_exclude(&repo).expect("ensure exclude");

        fs::create_dir_all(worktree.join(".codegraph")).expect("create codegraph dir");
        fs::write(worktree.join(".codegraph").join("index.db"), b"x").expect("write codegraph");
        fs::create_dir_all(worktree.join(".verify")).expect("create verify dir");
        fs::write(worktree.join(".verify").join("report.txt"), b"x").expect("write verify");

        let worktree_status = git(&worktree, &["status", "--porcelain"]);
        assert!(
            worktree_status.trim().is_empty(),
            "worktree git status must stay clean, got: {worktree_status}"
        );

        let root_status = git(&repo, &["status", "--porcelain"]);
        assert!(
            root_status.trim().is_empty(),
            "platform worktree namespace must stay out of the root git status, got: {root_status}"
        );
    }

    /// B1：repo_path 是 linked worktree 时，exclude 必须解析到公共 gitdir
    /// （主仓库 `<repo>/.git/info/exclude`）——linked worktree 的 `.git` 是
    /// gitdir 指针文件，直接 join 会写进 git 根本不读的路径。
    #[test]
    fn linked_worktree_exclude_lands_in_common_gitdir() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        init_repo_with_commit(&repo);
        let worktree = worktree_path(&repo);
        let worktree_str = worktree.to_string_lossy().to_string();
        git(
            &repo,
            &[
                "worktree",
                "add",
                &worktree_str,
                "-b",
                "aria/issues/issue_0001",
            ],
        );

        // sanity：fixture 确为 linked worktree（`.git` 是文件不是目录）。
        assert!(
            worktree.join(".git").is_file(),
            "fixture must be a linked worktree"
        );

        ensure_repo_exclude(&worktree).expect("ensure exclude from linked worktree");

        let exclude = fs::read_to_string(repo_exclude_path(&repo)).expect("read common exclude");
        for line in local_artifacts::exclude_lines() {
            assert!(
                exclude.lines().any(|existing| existing.trim() == line),
                "common exclude must contain `{line}`, got: {exclude}"
            );
        }

        // 公共 exclude 对 worktree 生效：worktree 内平台产物不进 git status。
        fs::create_dir_all(worktree.join(".codegraph")).expect("create codegraph dir");
        fs::write(worktree.join(".codegraph").join("index.db"), b"x").expect("write codegraph");
        let status = git(&worktree, &["status", "--porcelain"]);
        assert!(
            status.trim().is_empty(),
            "worktree git status must stay clean, got: {status}"
        );
    }

    /// P1-2：read→算缺失→写 的临界区由 `<gitdir>/aria-exclude.lock` 互斥——
    /// 锁被他人持有时 `ensure_repo_exclude` 必须阻塞等待，不得并发写入。
    #[test]
    fn concurrent_ensure_repo_exclude_is_serialized_by_flock() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");
        let lock_path = repo.join(".git").join("aria-exclude.lock");

        // 预持锁，模拟另一 attempt 正在临界区内。
        let held = ExclusiveFileLock::acquire_at(&lock_path).expect("pre-hold exclude lock");

        let writer_repo = repo.clone();
        let writer = std::thread::spawn(move || {
            ensure_repo_exclude(&writer_repo).expect("ensure exclude under lock");
        });

        std::thread::sleep(std::time::Duration::from_millis(300));
        assert!(
            !repo_exclude_path(&repo).exists(),
            "ensure_repo_exclude must block while the exclude lock is held"
        );

        drop(held);
        writer
            .join()
            .expect("writer thread completes after lock release");

        let exclude = fs::read_to_string(repo_exclude_path(&repo)).expect("read exclude");
        for line in local_artifacts::exclude_lines() {
            assert_eq!(
                exclude
                    .lines()
                    .filter(|existing| existing.trim() == line)
                    .count(),
                1,
                "exclude must contain exactly one `{line}` line, got: {exclude}"
            );
        }
        assert!(
            lock_path.exists(),
            "lock file must live at <gitdir>/aria-exclude.lock"
        );
    }

    /// P1-1：追加段带恰好一行说明注释且位于平台行之前；P1-2：temp+rename
    /// 原子写不在 `info/` 留下临时文件。
    #[test]
    fn exclude_append_carries_notice_and_leaves_no_temp_files() {
        let tmp = TempDir::new().expect("tempdir");
        let repo = tmp.path().join("repo");
        fs::create_dir_all(&repo).expect("create repo");

        ensure_repo_exclude(&repo).expect("first ensure");
        ensure_repo_exclude(&repo).expect("second ensure");

        let exclude = fs::read_to_string(repo_exclude_path(&repo)).expect("read exclude");
        let notice = local_artifacts::notice_text();
        assert_eq!(
            exclude.lines().filter(|line| line.trim() == notice).count(),
            1,
            "notice must appear exactly once, got: {exclude}"
        );
        let notice_offset = exclude.find(notice).expect("notice present");
        let first_platform_offset = exclude.find(".aria/").expect("platform line present");
        assert!(
            notice_offset < first_platform_offset,
            "notice must precede platform lines, got: {exclude}"
        );

        let info_dir = repo.join(".git").join("info");
        let leftovers: Vec<String> = fs::read_dir(&info_dir)
            .expect("read info dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .filter(|name| name != "exclude")
            .collect();
        assert!(
            leftovers.is_empty(),
            "atomic write must leave no temp files in info/, got: {leftovers:?}"
        );
    }
}
