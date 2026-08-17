// C-4 跨仓只读证据中介：attempt 级会话令牌（Task 3）。
//
// 契约 REQ-COD-05「受控接口」；设计 §4.1。attempt 启动/恢复时幂等生成/重写
// 32 字节随机令牌（解决 issue 级 worktree 跨 attempt 复用下旧令牌残留）：
// 原文只写入 worktree `.aria/evidence-token`（0600），Aria 侧 attempt 分区只存
// SHA-256 哈希（`evidence-token.json`：attempt_id/token_hash/created_at）。
// 同时向仓库公共 exclude（`<repo>/.git/info/exclude`，git 2.54 下 linked worktree
// 的 `info/exclude` 解析到 common dir，而非 `<repo>/.git/worktrees/<name>/info/exclude`）
// 幂等追加 `.aria/`，确保注入文件不进 `git add -A`、不进 commit（令牌明文外泄向量闭合）。
// 全仓排除 `.aria/` 语义正确：`.aria/` 本就是 Aria 在整仓（主 checkout 与所有
// worktree）的注入/运行目录。
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
use crate::product::coding_models::{CodingAttemptStatus, CodingExecutionAttempt};
use crate::product::coding_workspace_engine::worktree_path_for_attempt;
use crate::product::json_store::{read_json, write_json};
use crate::product::logical_codebase::evidence_index::EvidenceError;

/// worktree 注入目录名：`.aria/`（含尾斜杠，exclude 以目录语义忽略整个注入目录）。
const ARIA_DIR_NAME: &str = ".aria";
/// 仓库公共 exclude 中追加的目录忽略行。
const ARIA_EXCLUDE_LINE: &str = ".aria/";
/// 令牌文件相对 worktree 根的路径：`.aria/evidence-token`。
const EVIDENCE_TOKEN_FILE: &str = "evidence-token";
/// attempt 分区令牌哈希记录文件名。
const EVIDENCE_TOKEN_RECORD_FILE: &str = "evidence-token.json";
/// 令牌原始字节数（32 字节 → 64 位 hex）。
const TOKEN_BYTES: usize = 32;

/// Aria 侧 attempt 分区的令牌哈希记录（存 SHA-256，非明文）。
///
/// 字段名与设计 §4.1 一致（snake_case，serde 显式声明）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvidenceTokenRecord {
    pub attempt_id: String,
    pub token_hash: String,
    pub created_at: String,
}

/// 生成/重写 attempt 级会话令牌：写 worktree `.aria/evidence-token`（0600）、
/// 向仓库公共 exclude（`<repo>/.git/info/exclude`）幂等追加 `.aria/`、写 attempt 分区哈希记录，返回
/// 64 位 hex 令牌原文。同 attempt 二次调用生成新令牌并覆盖旧值（旧令牌立即失效）。
///
/// `paths` 提供 Aria 侧 attempt 分区根（`issue_lifecycle_root/coding-attempts/{id}`）；
/// `repo_path` 为物理仓库根，worktree 路径经 `worktree_path_for_attempt` 推导；
/// exclude 写入仓库公共 `<repo>/.git/info/exclude`（见 `ensure_repo_exclude`）。
pub fn issue_evidence_token(
    paths: &ProductAppPaths,
    repo_path: &Path,
    attempt: &CodingExecutionAttempt,
) -> Result<String, EvidenceError> {
    let token = generate_token();
    let worktree = worktree_path_for_attempt(repo_path, attempt);

    write_token_file(&worktree, &token)?;
    ensure_repo_exclude(repo_path)?;

    let record = EvidenceTokenRecord {
        attempt_id: attempt.id.clone(),
        token_hash: compute_sha256(token.as_bytes()),
        created_at: Utc::now().to_rfc3339(),
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

    if record.token_hash == compute_sha256(token.as_bytes()) {
        Ok(())
    } else {
        Err(EvidenceError::Unauthorized)
    }
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

/// 向仓库公共 exclude 幂等追加 `.aria/` 一行。
///
/// git 2.54 下 linked worktree 的 `info/exclude` 解析到 common dir（`--git-path
/// info/exclude` 返回 `<repo>/.git/info/exclude`），故写入物理仓库根的公共
/// `<repo>/.git/info/exclude`（`info/` 目录不存在时自动创建）。
fn ensure_repo_exclude(repo_path: &Path) -> Result<(), EvidenceError> {
    let exclude_path = repo_path.join(".git").join("info").join("exclude");
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

    if content.lines().any(|line| line.trim() == ARIA_EXCLUDE_LINE) {
        return Ok(());
    }

    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(ARIA_EXCLUDE_LINE);
    content.push('\n');

    std::fs::write(&exclude_path, content.as_bytes()).map_err(|error| EvidenceError::Io {
        message: format!("write {}: {error}", exclude_path.display()),
    })
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
        }
    }

    /// 轻量 fixture：构造 repo 根 + issue 级 worktree 目录（`repo_path/.git`
    /// 作为公共 gitdir，exclude 写入其 `info/exclude`），无需真实 git 仓库。
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
                .filter(|line| line.trim() == ARIA_EXCLUDE_LINE)
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
            exclude.lines().any(|line| line.trim() == ARIA_EXCLUDE_LINE),
            "common exclude must contain `.aria/`, got: {exclude}"
        );

        git(&worktree, &["add", "-A"]);
        let staged = git(&worktree, &["diff", "--cached", "--name-only"]);
        assert!(
            !staged.contains(EVIDENCE_TOKEN_FILE),
            "git add -A must not stage the injected token file, staged: {staged}"
        );
    }
}
