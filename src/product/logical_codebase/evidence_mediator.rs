// C-4 跨仓只读证据中介：六步编排核心（EvidenceQueryMediator，Task 6）。
//
// 契约 REQ-COD-05 全语义；设计 §3.1/§4.2/§4.3。按固定顺序编排——①身份校验
// （令牌哈希反查 attempt 归属 + Running 校验）②target_snapshot 锚定
// （None→evidence_not_available/404 + 目标成员目录名推导）③ACL（manifest 成员集合
// + 成员目录名，排除本仓/非成员）④快照钉住（首查写 evidence-index-pin.json，
// 后续读钉住 record + stale 判定）⑤查询+渲染+单次 12k 截断+累计配额
// （Exhausted→evidence_budget_exhausted/429）⑥审计 append（role 拼
// role_self_reported 标记）。
//
// 本模块不调用真实 Provider：`handle_evidence_query` 以 `TokioBoundedCommandRunner`
// 构造真实 `CodeGraphCli`；测试经 `handle_evidence_query_with_runner` 注入 fake runner。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::cross_cutting::document_ops::compute_sha256;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::{read_json, write_json};
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexRecord, AggregateIndexStore,
};
use crate::product::logical_codebase::evidence_index::EvidenceError;
use crate::product::logical_codebase::evidence_token::{
    EvidenceTokenRecord, EVIDENCE_TOKEN_RECORD_FILE,
};
use crate::product::logical_codebase::store::{LogicalCodebaseManifest, LogicalCodebaseStore};

/// attempt 分区快照钉住记录文件名（设计 §4.3 命名）。
const EVIDENCE_INDEX_PIN_FILE: &str = "evidence-index-pin.json";

/// 证据查询可用角色（Coder/Reviewer 共用，审计区分角色；serde snake_case）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceRole {
    Coder,
    Reviewer,
}

impl EvidenceRole {
    fn as_str(self) -> &'static str {
        match self {
            Self::Coder => "coder",
            Self::Reviewer => "reviewer",
        }
    }
}

/// 受控证据查询输入（HTTP body `{token, role, query}`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvidenceQueryInput {
    pub token: String,
    pub role: EvidenceRole,
    pub query: String,
}

/// 受控证据查询响应（HTTP body `{text, truncated, index_stale, budget_remaining}`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvidenceQueryResponse {
    pub text: String,
    pub truncated: bool,
    pub index_stale: bool,
    pub budget_remaining: usize,
}

/// 六步编排入口（T7 使用）：以真实 `TokioBoundedCommandRunner` 构造 CodeGraphCli。
#[allow(unused_variables)]
pub fn handle_evidence_query(
    paths: &ProductAppPaths,
    input: &EvidenceQueryInput,
) -> Result<EvidenceQueryResponse, EvidenceError> {
    todo!()
}

/// attempt 分区快照钉住记录（`pinned_aggregate_index_id`/`pinned_at`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvidenceIndexPinRecord {
    pub pinned_aggregate_index_id: String,
    pub pinned_at: String,
}

/// 令牌→attempt 反查：扫描各 project 下各 issue 的 attempts 分区，读
/// `evidence-token.json` 比对 `token_hash`，命中后 load attempt；无命中→Unauthorized。
///
/// 设计 §4.1 未指定反查机制，T3 亦未提供；本函数按 O(attempts) 目录扫描实现
/// （规模小可接受），后续可优化为 `token_hash -> attempt` 索引。
pub fn resolve_attempt_by_token(
    paths: &ProductAppPaths,
    token: &str,
) -> Result<CodingExecutionAttempt, EvidenceError> {
    let target_hash = compute_sha256(token.as_bytes());
    let projects_root = paths.projects_root();
    for project_path in child_directories(&projects_root)? {
        let Some(project_id) = project_path.file_name().and_then(|name| name.to_str()) else {
            continue;
        };
        let issues_root = project_path.join("issues");
        for issue_path in child_directories(&issues_root)? {
            let Some(issue_id) = issue_path.file_name().and_then(|name| name.to_str()) else {
                continue;
            };
            let attempts_root = issue_path.join("coding-attempts");
            for attempt_dir in child_directories(&attempts_root)? {
                let token_path = attempt_dir.join(EVIDENCE_TOKEN_RECORD_FILE);
                if !token_path.exists() {
                    continue;
                }
                let record: EvidenceTokenRecord =
                    read_json(&token_path).map_err(|error| EvidenceError::Io {
                        message: format!(
                            "read evidence token record {}: {error}",
                            token_path.display()
                        ),
                    })?;
                if record.token_hash != target_hash {
                    continue;
                }
                let Some(attempt_id) = attempt_dir.file_name().and_then(|name| name.to_str()) else {
                    return Err(EvidenceError::Io {
                        message: format!(
                            "attempt dir name is not UTF-8: {}",
                            attempt_dir.display()
                        ),
                    });
                };
                let store = CodingAttemptStore::new(paths.clone());
                return store
                    .get_attempt(project_id, issue_id, attempt_id)
                    .map_err(|error| EvidenceError::Io {
                        message: format!("load attempt {attempt_id}: {error}"),
                    });
            }
        }
    }
    Err(EvidenceError::Unauthorized)
}

/// 列出 `root` 下全部子目录（不存在视为空；非 UTF-8 名称跳过）。
fn child_directories(root: &Path) -> Result<Vec<PathBuf>, EvidenceError> {
    let entries = match std::fs::read_dir(root) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => {
            return Err(EvidenceError::Io {
                message: format!("read {}: {error}", root.display()),
            });
        }
    };
    let mut dirs = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| EvidenceError::Io {
            message: format!("read {} entry: {error}", root.display()),
        })?;
        let file_type = entry.file_type().map_err(|error| EvidenceError::Io {
            message: format!("stat {}: {error}", entry.path().display()),
        })?;
        if file_type.is_dir() {
            dirs.push(entry.path());
        }
    }
    dirs.sort();
    Ok(dirs)
}

/// attempt 分区快照钉住记录路径。
fn attempt_pin_path(paths: &ProductAppPaths, attempt: &CodingExecutionAttempt) -> PathBuf {
    paths
        .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join(EVIDENCE_INDEX_PIN_FILE)
}

/// 快照钉住：首查用 `manifest.active_aggregate_index_id` 写 pin，后续读钉住值
/// 并按 id load 该 record（superseded 可读）。
fn load_or_pin_index(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    manifest: &LogicalCodebaseManifest,
) -> Result<AggregateIndexRecord, EvidenceError> {
    let pin_path = attempt_pin_path(paths, attempt);
    if pin_path.exists() {
        let pin: EvidenceIndexPinRecord =
            read_json(&pin_path).map_err(|error| EvidenceError::Io {
                message: format!("read evidence index pin {}: {error}", pin_path.display()),
            })?;
        return load_pinned_index(paths, attempt, &pin.pinned_aggregate_index_id);
    }

    let pinned_id = manifest.active_aggregate_index_id.clone().ok_or_else(|| {
        EvidenceError::QueryFailed {
            code: "evidence_index_unavailable",
            message: format!(
                "project {} has no active aggregate index to pin",
                attempt.project_id
            ),
        }
    })?;
    let record = load_pinned_index(paths, attempt, &pinned_id)?;
    let pin = EvidenceIndexPinRecord {
        pinned_aggregate_index_id: pinned_id,
        pinned_at: Utc::now().to_rfc3339(),
    };
    write_json(&pin_path, &pin).map_err(|error| EvidenceError::Io {
        message: format!("write evidence index pin {}: {error}", pin_path.display()),
    })?;
    Ok(record)
}

fn load_pinned_index(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    pinned_id: &str,
) -> Result<AggregateIndexRecord, EvidenceError> {
    let store = AggregateIndexStore::new(paths.clone());
    store
        .get(&attempt.project_id, pinned_id)?
        .ok_or_else(|| EvidenceError::QueryFailed {
            code: "evidence_index_unavailable",
            message: format!("pinned aggregate index {pinned_id} was not found"),
        })
}

/// stale 判定：attempt 冻结的 target revision / membership_revision 与钉住 record
/// 的目标成员 revision / record membership_revision 比对，任一不一致 → stale。
fn is_index_stale(
    snapshot: &crate::product::coding_models::AttemptTargetSnapshot,
    record: &AggregateIndexRecord,
) -> bool {
    if snapshot.membership_revision != record.membership_revision {
        return true;
    }
    let Some(target_revision) = snapshot.revision.as_deref() else {
        return false;
    };
    match record
        .member_snapshots
        .iter()
        .find(|member| member.logical_repository_id == snapshot.logical_repository_id)
    {
        Some(member) => member.revision != target_revision,
        None => true,
    }
}

fn map_store_error(error: crate::product::json_store::ProductStoreError) -> EvidenceError {
    EvidenceError::Io {
        message: error.to_string(),
    }
}

// 占位：BTreeMap/LogicalCodebaseStore 供后续编排与 ACL 使用，phase 1 仅用 map_store_error。
#[allow(dead_code)]
fn _unused_placeholder(
    _logical: &LogicalCodebaseStore,
    _map: &BTreeMap<String, String>,
    _store: &CodingAttemptStore,
) {
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use tempfile::TempDir;
    use uuid::Uuid;

    use super::*;
    use crate::product::coding_models::{
        AttemptTargetSnapshot, CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    };
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexStatus,
    };
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, LogicalRepositoryId,
        MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity,
        RepositoryType,
    };
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const ATTEMPT_ID: &str = "coding_attempt_0001";

    struct Fixture {
        _tmp: TempDir,
        paths: ProductAppPaths,
        repo: PathBuf,
        attempt: CodingExecutionAttempt,
        token: String,
        manifest: LogicalCodebaseManifest,
        aggregate_index_id: String,
        api_id: LogicalRepositoryId,
        web_id: LogicalRepositoryId,
        api_checkout_id: RepositoryCheckoutId,
        web_checkout_id: RepositoryCheckoutId,
    }

    fn setup() -> Fixture {
        let tmp = TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(&repo).expect("create repo dir");

        let api_id = LogicalRepositoryId(Uuid::new_v4());
        let web_id = LogicalRepositoryId(Uuid::new_v4());
        let api_checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let web_checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        let aggregate_index_id = format!("aggregate_index_{}", Uuid::new_v4().simple());

        let logical = LogicalCodebaseStore::new(paths.clone());
        let manifest = LogicalCodebaseManifest {
            schema_version: 1,
            project_id: PROJECT_ID.to_string(),
            logical_codebase_id: Uuid::new_v4(),
            provider_context_root: repo.clone(),
            layout: crate::product::logical_codebase::LogicalCodebaseLayout::CommonNonGitParent,
            membership_revision: 1,
            member_ids: vec![api_id, web_id],
            active_aggregate_index_id: Some(aggregate_index_id.clone()),
            context_policy_digest: String::new(),
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
        };
        logical.save_manifest(PROJECT_ID, &manifest).expect("save manifest");

        logical
            .save_member(
                PROJECT_ID,
                &member_record(api_id, "api", api_checkout_id, "repo_api"),
            )
            .expect("save api member");
        logical
            .save_member(
                PROJECT_ID,
                &member_record(web_id, "web", web_checkout_id, "repo_web"),
            )
            .expect("save web member");
        logical
            .save_checkout(
                PROJECT_ID,
                &checkout_record(api_id, api_checkout_id, repo.join("api")),
            )
            .expect("save api checkout");
        logical
            .save_checkout(
                PROJECT_ID,
                &checkout_record(web_id, web_checkout_id, repo.join("web")),
            )
            .expect("save web checkout");

        let index_store = AggregateIndexStore::new(paths.clone());
        index_store
            .create(
                PROJECT_ID,
                index_record(
                    &aggregate_index_id,
                    api_id,
                    web_id,
                    api_checkout_id,
                    web_checkout_id,
                    "rev-api",
                    "rev-web",
                    &repo,
                ),
            )
            .expect("create aggregate index record");

        let snapshot = target_snapshot(api_id, api_checkout_id, &repo.join("api"), "rev-api", 1);
        let attempt = attempt_fixture(CodingAttemptStatus::Running, Some(snapshot));
        write_json(&attempt_record_path(&paths), &attempt).expect("write attempt record");

        let token =
            crate::product::logical_codebase::evidence_token::issue_evidence_token(
                &paths,
                &repo,
                &attempt,
            )
            .expect("issue evidence token");

        Fixture {
            _tmp: tmp,
            paths,
            repo,
            attempt,
            token,
            manifest,
            aggregate_index_id,
            api_id,
            web_id,
            api_checkout_id,
            web_checkout_id,
        }
    }

    fn attempt_fixture(
        status: CodingAttemptStatus,
        target_snapshot: Option<AttemptTargetSnapshot>,
    ) -> CodingExecutionAttempt {
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
            target_snapshot,
            completed_at: None,
        }
    }

    fn target_snapshot(
        logical_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        canonical_path: &Path,
        revision: &str,
        membership_revision: u64,
    ) -> AttemptTargetSnapshot {
        AttemptTargetSnapshot {
            logical_repository_id: logical_id,
            checkout_id,
            physical_repository_id: format!("repo_{}", logical_id.0),
            canonical_path: canonical_path.to_path_buf(),
            git_dir_identity: "git-dir-id".to_string(),
            revision: Some(revision.to_string()),
            policy_digest: "policy-digest".to_string(),
            membership_revision,
            captured_at: "2026-08-14T00:00:00Z".to_string(),
            capture_source: "test".to_string(),
        }
    }

    fn member_record(
        id: LogicalRepositoryId,
        alias: &str,
        checkout_id: RepositoryCheckoutId,
        physical_id: &str,
    ) -> CodebaseMemberRecord {
        CodebaseMemberRecord {
            logical_repository_id: id,
            physical_repository_id: physical_id.to_string(),
            alias: alias.to_string(),
            role: "backend".to_string(),
            ordinal: 0,
            source_identity: RepositorySourceIdentity {
                scheme: "git_dir_only_v1".to_string(),
                key_digest: format!("sha256:{alias}"),
                canonical_git_dir: PathBuf::from(format!("/workspace/{alias}/.git")),
                canonical_origin: None,
                first_seen_path_hash: format!("hash:{alias}"),
            },
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: vec![checkout_id],
            status: MemberStatus::Active,
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
        }
    }

    fn checkout_record(
        logical_id: LogicalRepositoryId,
        checkout_id: RepositoryCheckoutId,
        canonical_path: PathBuf,
    ) -> RepositoryCheckoutRecord {
        RepositoryCheckoutRecord {
            checkout_id,
            logical_repository_id: logical_id,
            physical_repository_id: format!("repo_{}", logical_id.0),
            kind: CheckoutKind::Main,
            canonical_path,
            checkout_path_hash: "checkout-hash".to_string(),
            git_dir_identity: "git-dir-id".to_string(),
            revision: None,
            availability: CheckoutAvailability::Available,
            observed_at: "2026-08-14T00:00:00Z".to_string(),
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
        }
    }

    fn index_record(
        id: &str,
        api_id: LogicalRepositoryId,
        web_id: LogicalRepositoryId,
        api_checkout_id: RepositoryCheckoutId,
        web_checkout_id: RepositoryCheckoutId,
        api_revision: &str,
        web_revision: &str,
        codegraph_root: &Path,
    ) -> AggregateIndexRecord {
        AggregateIndexRecord {
            aggregate_index_id: id.to_string(),
            project_id: PROJECT_ID.to_string(),
            membership_revision: 1,
            status: AggregateIndexStatus::Active,
            member_snapshots: vec![
                AggregateIndexMemberSnapshot::indexed(
                    api_id,
                    api_checkout_id,
                    api_revision.to_string(),
                    false,
                    "2026-08-14T00:00:00Z".to_string(),
                ),
                AggregateIndexMemberSnapshot::indexed(
                    web_id,
                    web_checkout_id,
                    web_revision.to_string(),
                    false,
                    "2026-08-14T00:00:00Z".to_string(),
                ),
            ],
            codegraph_version: "1.5.0".to_string(),
            codegraph_root: codegraph_root.to_path_buf(),
            config_digest: String::new(),
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
            supersedes_aggregate_index_id: None,
            warning: None,
        }
    }

    fn attempt_record_path(paths: &ProductAppPaths) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(format!("{ATTEMPT_ID}.json"))
    }

    fn pin_file_path(paths: &ProductAppPaths) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(ATTEMPT_ID)
            .join(EVIDENCE_INDEX_PIN_FILE)
    }

    #[test]
    fn resolve_attempt_by_token_returns_matching_attempt() {
        let fx = setup();

        let attempt = resolve_attempt_by_token(&fx.paths, &fx.token).expect("resolve attempt");
        assert_eq!(attempt.id, ATTEMPT_ID);
        assert_eq!(attempt.project_id, PROJECT_ID);
        assert_eq!(attempt.issue_id, ISSUE_ID);
    }

    #[test]
    fn resolve_attempt_by_token_rejects_unknown_token() {
        let fx = setup();

        let unknown = format!("unknown-{}-token", Uuid::new_v4());
        let err = resolve_attempt_by_token(&fx.paths, &unknown).expect_err("unknown token");
        assert_eq!(err, EvidenceError::Unauthorized);
    }

    #[test]
    fn pin_is_written_on_first_load_and_reused_on_second() {
        let mut fx = setup();

        // 首次：按 manifest.active_aggregate_index_id 写 pin 并返回该 record。
        let first = load_or_pin_index(&fx.paths, &fx.attempt, &fx.manifest)
            .expect("first pin load");
        assert_eq!(first.aggregate_index_id, fx.aggregate_index_id);

        let pin: EvidenceIndexPinRecord =
            read_json(&pin_file_path(&fx.paths)).expect("read pin file");
        assert_eq!(pin.pinned_aggregate_index_id, fx.aggregate_index_id);
        assert!(!pin.pinned_at.is_empty());

        // 中途 active 指针被 supersede：写入第二代索引并把 manifest.active 指向它。
        let second_id = format!("aggregate_index_{}", Uuid::new_v4().simple());
        let index_store = AggregateIndexStore::new(fx.paths.clone());
        index_store
            .create(
                PROJECT_ID,
                index_record(
                    &second_id,
                    fx.api_id,
                    fx.web_id,
                    fx.api_checkout_id,
                    fx.web_checkout_id,
                    "rev-api-2",
                    "rev-web-2",
                    &fx.repo,
                ),
            )
            .expect("create second index");
        fx.manifest.active_aggregate_index_id = Some(second_id.clone());
        LogicalCodebaseStore::new(fx.paths.clone())
            .save_manifest(PROJECT_ID, &fx.manifest)
            .expect("save manifest with new active id");

        // 二次查询读钉住值（第一代 record），不重写 pin。
        let second = load_or_pin_index(&fx.paths, &fx.attempt, &fx.manifest)
            .expect("second pin load");
        assert_eq!(second.aggregate_index_id, fx.aggregate_index_id);

        let pin: EvidenceIndexPinRecord =
            read_json(&pin_file_path(&fx.paths)).expect("re-read pin file");
        assert_eq!(pin.pinned_aggregate_index_id, fx.aggregate_index_id);
    }

    #[test]
    fn stale_detects_membership_revision_and_revision_mismatch() {
        let fx = setup();
        let snapshot =
            target_snapshot(fx.api_id, fx.api_checkout_id, &fx.repo.join("api"), "rev-api", 1);

        let matching = index_record(
            &fx.aggregate_index_id,
            fx.api_id,
            fx.web_id,
            fx.api_checkout_id,
            fx.web_checkout_id,
            "rev-api",
            "rev-web",
            &fx.repo,
        );
        assert!(!is_index_stale(&snapshot, &matching));

        // 方向一：membership_revision 不一致 → stale。
        let mut membership_advanced = matching.clone();
        membership_advanced.membership_revision = 2;
        assert!(is_index_stale(&snapshot, &membership_advanced));

        // 方向二：目标成员 revision 不一致 → stale。
        let mut revision_drifted = matching.clone();
        revision_drifted.member_snapshots[0].revision = "rev-api-other".to_string();
        assert!(is_index_stale(&snapshot, &revision_drifted));
    }
}
