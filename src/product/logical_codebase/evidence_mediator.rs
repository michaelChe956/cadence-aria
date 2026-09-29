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
use std::sync::Arc;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::cross_cutting::bounded_command_runner::{
    BoundedCommandRunner, TokioBoundedCommandRunner,
};
use crate::cross_cutting::document_ops::compute_sha256;
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{AttemptTargetSnapshot, CodingExecutionAttempt};
use crate::product::json_store::{read_json, write_json};
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexRecord, AggregateIndexStore, CodeGraphCli,
};
use crate::product::logical_codebase::evidence_audit::{
    EvidenceAuditRecord, append_evidence_audit,
};
use crate::product::logical_codebase::evidence_budget::{
    BudgetOutcome, EVIDENCE_QUERY_RESULT_CHAR_LIMIT, EvidenceBudgetLedger,
};
use crate::product::logical_codebase::evidence_index::{
    EvidenceError, EvidenceHit, EvidenceIndexQuery,
};
use crate::product::logical_codebase::evidence_token::{
    EVIDENCE_TOKEN_RECORD_FILE, EvidenceTokenRecord, validate_evidence_token,
};
use crate::product::logical_codebase::store::{LogicalCodebaseManifest, LogicalCodebaseStore};
use crate::product::logical_codebase::{
    CheckoutAvailability, CheckoutKind, MemberStatus, RepositoryCheckoutRecord,
};

/// attempt 分区快照钉住记录文件名（设计 §4.3 命名）。
const EVIDENCE_INDEX_PIN_FILE: &str = "evidence-index-pin.json";

/// 单次结果被截断时附加的尾部标记。
const TRUNCATION_MARKER: &str = "（结果已截断，请缩小查询范围）";

/// 单行渲染文本的防御性字符上限（超出截断单行；非设计常量，仅防病理性符号串）。
const EVIDENCE_HIT_LINE_CHAR_LIMIT: usize = 4096;

/// 审计 `role` 字段的自报标记后缀（设计 §5.2）。
const ROLE_SELF_REPORTED_MARK: &str = "role_self_reported";

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

/// 审计 `role` 字段标签：`coder(role_self_reported)` / `reviewer(role_self_reported)`。
///
/// T7 被拒查询审计补记（T6 Minor-1）与 T6 成功路径共用同一标签来源，避免
/// `role_self_reported` 标记在调用点各自拼接导致漂移。
pub fn evidence_role_label(role: EvidenceRole) -> String {
    format!("{}({})", role.as_str(), ROLE_SELF_REPORTED_MARK)
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
pub fn handle_evidence_query(
    paths: &ProductAppPaths,
    input: &EvidenceQueryInput,
) -> Result<EvidenceQueryResponse, EvidenceError> {
    handle_evidence_query_with_runner(paths, input, Arc::new(TokioBoundedCommandRunner))
}

/// 可注入 runner 的编排实现（测试经 fake runner 注入 CodeGraphCli）。
fn handle_evidence_query_with_runner(
    paths: &ProductAppPaths,
    input: &EvidenceQueryInput,
    runner: Arc<dyn BoundedCommandRunner>,
) -> Result<EvidenceQueryResponse, EvidenceError> {
    // ① 令牌校验：反查 attempt 归属（Unauthorized/401）+ Running 校验（Forbidden/403）。
    let attempt = resolve_attempt_by_token(paths, &input.token)?;
    validate_evidence_token(paths, &attempt, &input.token)?;

    // ② target_snapshot 锚定：None → evidence_not_available（404）；推导目标成员目录名。
    let snapshot = attempt
        .target_snapshot
        .as_ref()
        .ok_or(EvidenceError::NotAvailable)?;
    // v1.3：按 issue 所属代码库把 manifest/member/index 全部解析到 lc_id 子树。
    let lc_id = attempt_logical_codebase_id(paths, &attempt)?;
    let logical = match lc_id.as_deref() {
        Some(lc_id) => LogicalCodebaseStore::for_lc(paths.clone(), lc_id),
        None => LogicalCodebaseStore::new(paths.clone()),
    };
    let target_member_dir = resolve_target_member_dir(&logical, &attempt, snapshot)?;

    // ③ ACL：manifest 成员集合 + 成员目录名（排除本仓/非成员）。
    let manifest = logical
        .load_manifest(&attempt.project_id)
        .map_err(map_store_error)?
        .ok_or_else(|| EvidenceError::QueryFailed {
            code: "evidence_acl_manifest_missing",
            message: format!(
                "project {} has no logical-codebase manifest",
                attempt.project_id
            ),
        })?;
    let member_dir_names = member_dir_names(&logical, &manifest)?;
    if !manifest
        .member_ids
        .contains(&snapshot.logical_repository_id)
    {
        return Err(EvidenceError::QueryFailed {
            code: "evidence_acl_target_not_member",
            message: format!(
                "target member {} is not in the manifest member set",
                snapshot.logical_repository_id.0
            ),
        });
    }
    let index_query = EvidenceIndexQuery::new(
        CodeGraphCli::new(runner, "codegraph".to_string()),
        manifest.provider_context_root.clone(),
        member_dir_names,
        target_member_dir,
    );

    // ④ 快照钉住：首查写 pin，后续读钉住 record；stale 两方向判定。
    let pinned = load_or_pin_index(paths, &attempt, &manifest, lc_id.as_deref())?;
    let index_stale = is_index_stale(snapshot, &pinned);

    // ⑤ 查询 + 渲染 + 单次 12k 截断 + 累计配额（Exhausted→evidence_budget_exhausted/429）。
    let hits = index_query.query(&input.query)?;
    let (text, truncated) = render_and_truncate(&hits);
    let result_chars = text.chars().count();
    let ledger = EvidenceBudgetLedger::new(paths.clone());
    let budget_remaining = match ledger.consume(&attempt, &input.query, result_chars)? {
        BudgetOutcome::Accepted { remaining } => remaining,
        BudgetOutcome::Exhausted => return Err(EvidenceError::BudgetExhausted),
    };

    // ⑥ 审计 append（role 拼 role_self_reported 标记）。
    append_evidence_audit(
        paths,
        &attempt,
        &EvidenceAuditRecord {
            attempt_id: attempt.id.clone(),
            role: evidence_role_label(input.role),
            query: input.query.clone(),
            hit_count: hits.len(),
            result_chars,
            snapshot_refs: hits.iter().map(|hit| hit.file_path.clone()).collect(),
            budget_remaining,
            timestamp: Utc::now().to_rfc3339(),
        },
    )?;

    Ok(EvidenceQueryResponse {
        text,
        truncated,
        index_stale,
        budget_remaining,
    })
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
                let Some(attempt_id) = attempt_dir.file_name().and_then(|name| name.to_str())
                else {
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

// ---- C2 Task 10（REQ-ENV-C2-POLICY，#17／BYPASS-17）：受限政策读取与重新授权 ----
//
// 页面读取／返修 run 读取／重新授权三个应用服务共用同一条 fail-closed 链：
// attempt 冻结 envelope digest（target_snapshot.policy_digest）为唯一权威，
// resolver（resolve_for_issue）必须解析出同 digest 的 AuthorityPolicyReference，
// 正文经 read_policy_text_for_reference 复核 canonical SHA-256。resolver 不可
// 用或 digest 不一致时读取与重新授权均 fail-closed 并落"政策核验"等待事实
//（Task 12 投影 kind `policy_verification`），MUST NOT 回落成员仓路径／项目级
// 历史布局／绝对路径猜测，MUST NOT 把政策绝对路径或正文写入 context note。

/// 政策核验等待事实文件（attempt 分区，单文件最新事实）。
const POLICY_VERIFICATION_WAITING_FILE: &str = "policy-verification.json";

/// 重新授权有效时长（小时）。运行态约束由 `validate_evidence_token` 的
/// Running 校验承担，墙钟过期是第二道防线（解析失败按已过期 fail-closed）。
pub const POLICY_REAUTHORIZATION_TTL_HOURS: i64 = 24;

/// C2 Task 10：受限政策读取/重新授权应用层错误（fail-closed 口径）。
/// `ResolverUnavailable`／`DigestMismatch` 返回前已落政策核验等待事实。
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyAccessError {
    /// resolver 无法唯一解析（无 LC 归属、policy artifact 缺失等）。
    #[error("policy_resolver_unavailable:{detail}")]
    ResolverUnavailable { detail: String },
    /// resolver 当前 digest 与 attempt 冻结 envelope digest 不一致。
    #[error("policy_digest_mismatch:expected:{expected}:actual:{actual}")]
    DigestMismatch { expected: String, actual: String },
    /// 令牌无效或已被替换（Unauthorized 口径）。
    #[error("policy_unauthorized")]
    Unauthorized,
    /// 错 role／过期／claims 缺失或 digest 不匹配——提示重新授权。
    #[error("policy_forbidden:{reason}")]
    Forbidden { reason: String },
    /// 同 command 异 payload（请刷新）。
    #[error("policy_command_conflict:{detail}")]
    CommandConflict { detail: String },
    /// expected 版本与 durable 版本不符（请刷新）。
    #[error("policy_version_conflict:expected:{expected}:actual:{actual}")]
    VersionConflict { expected: u64, actual: u64 },
    /// 材料缺失（无 target snapshot 等）或状态不在等待面。
    #[error("policy_not_available:{reason}")]
    NotAvailable { reason: String },
    /// 低层 reader fail-closed（错误详情字符串化，保 Clone/Eq）。
    #[error("policy_read_failed:{detail}")]
    ReadFailed { detail: String },
    /// 服务端 IO 失败。
    #[error("policy_io:{message}")]
    Io { message: String },
}

impl From<EvidenceError> for PolicyAccessError {
    fn from(error: EvidenceError) -> Self {
        match error {
            EvidenceError::Unauthorized => Self::Unauthorized,
            EvidenceError::Forbidden => Self::Forbidden {
                reason: "attempt_not_running".to_string(),
            },
            EvidenceError::NotAvailable => Self::NotAvailable {
                reason: "evidence_not_available".to_string(),
            },
            EvidenceError::Io { message } => Self::Io { message },
            other => Self::Io {
                message: other.to_string(),
            },
        }
    }
}

impl From<crate::product::logical_codebase::repository_routing::PolicyReadError>
    for PolicyAccessError
{
    fn from(error: crate::product::logical_codebase::repository_routing::PolicyReadError) -> Self {
        Self::ReadFailed {
            detail: error.to_string(),
        }
    }
}

/// C2 Task 10：政策核验等待事实（fail-closed 落账；Task 12 投影
/// kind `policy_verification`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PolicyVerificationWaitingRecord {
    pub attempt_id: String,
    pub reason_code: String,
    pub detail: String,
    pub policy_digest: Option<String>,
    pub created_at: String,
}

fn policy_verification_waiting_path(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> PathBuf {
    paths
        .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join(POLICY_VERIFICATION_WAITING_FILE)
}

/// 读取政策核验等待事实（只读；无事实返回 `None`）。
pub fn load_policy_verification_waiting_fact(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<Option<PolicyVerificationWaitingRecord>, EvidenceError> {
    let path = policy_verification_waiting_path(paths, attempt);
    if !path.exists() {
        return Ok(None);
    }
    read_json(&path).map_err(|error| EvidenceError::Io {
        message: format!("read policy verification waiting fact {}: {error}", path.display()),
    })
}

fn land_policy_verification_waiting_fact(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    reason_code: &str,
    detail: String,
    policy_digest: Option<String>,
) -> Result<(), EvidenceError> {
    let record = PolicyVerificationWaitingRecord {
        attempt_id: attempt.id.clone(),
        reason_code: reason_code.to_string(),
        detail,
        policy_digest,
        created_at: Utc::now().to_rfc3339(),
    };
    write_json(&policy_verification_waiting_path(paths, attempt), &record).map_err(|error| {
        EvidenceError::Io {
            message: format!("write policy verification waiting fact: {error}"),
        }
    })
}

fn clear_policy_verification_waiting_fact(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) {
    let path = policy_verification_waiting_path(paths, attempt);
    if path.exists() && let Err(error) = std::fs::remove_file(&path) {
        tracing::warn!(
            %error,
            attempt_id = %attempt.id,
            "remove policy verification waiting fact failed; stale fact stays readable"
        );
    }
}

fn policy_access_io(message: String) -> PolicyAccessError {
    PolicyAccessError::Io { message }
}

fn load_policy_attempt(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<CodingExecutionAttempt, PolicyAccessError> {
    CodingAttemptStore::new(paths.clone())
        .get_attempt(project_id, issue_id, attempt_id)
        .map_err(|error| policy_access_io(format!("load attempt {attempt_id}: {error}")))
}

fn frozen_policy_digest(attempt: &CodingExecutionAttempt) -> Result<&str, PolicyAccessError> {
    attempt
        .target_snapshot
        .as_ref()
        .map(|snapshot| snapshot.policy_digest.as_str())
        .ok_or_else(|| PolicyAccessError::NotAvailable {
            reason: "target_snapshot_missing".to_string(),
        })
}

fn resolve_frozen_reference(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<crate::product::logical_codebase::repository_routing::AuthorityPolicyReference, PolicyAccessError> {
    let resolution = crate::product::logical_codebase::repository_routing::RepositoryAuthorityResolver::new(paths.clone())
        .resolve_for_issue(&attempt.project_id, &attempt.issue_id)
        .map_err(|error| policy_access_io(format!("resolve authority: {error}")))?
        .ok_or_else(|| PolicyAccessError::ResolverUnavailable {
            detail: "no_lc_authority_resolution".to_string(),
        })?;
    resolution.policy.ok_or_else(|| PolicyAccessError::ResolverUnavailable {
        detail: "policy_reference_missing".to_string(),
    })
}

/// resolver 冻结一致性读取：reference digest 必须与 attempt 冻结 envelope
/// digest 一致，正文经低层 reader 复核。
fn read_policy_text_checked(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<crate::product::logical_codebase::repository_routing::PolicyTextResult, PolicyAccessError> {
    let frozen = frozen_policy_digest(attempt)?.to_string();
    let reference = resolve_frozen_reference(paths, attempt)?;
    if reference.policy_digest != frozen {
        return Err(PolicyAccessError::DigestMismatch {
            expected: frozen,
            actual: reference.policy_digest,
        });
    }
    crate::product::logical_codebase::repository_routing::read_policy_text_for_reference(
        paths, &reference,
    )
    .map_err(|error| PolicyAccessError::ReadFailed { detail: error.to_string() })
}

/// fail-closed 统一收口：ResolverUnavailable／DigestMismatch 落政策核验等待
/// 事实后原样返回；成功路径清除等待事实。
fn settle_policy_read(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<crate::product::logical_codebase::repository_routing::PolicyTextResult, PolicyAccessError> {
    match read_policy_text_checked(paths, attempt) {
        Ok(result) => {
            clear_policy_verification_waiting_fact(paths, attempt);
            Ok(result)
        }
        Err(error @ (PolicyAccessError::ResolverUnavailable { .. }
        | PolicyAccessError::DigestMismatch { .. })) => {
            let reason_code = match &error {
                PolicyAccessError::ResolverUnavailable { .. } => "policy_resolver_unavailable",
                _ => "policy_digest_mismatch",
            };
            let frozen = attempt
                .target_snapshot
                .as_ref()
                .map(|snapshot| snapshot.policy_digest.clone());
            let detail = error.to_string();
            land_policy_verification_waiting_fact(
                paths,
                attempt,
                reason_code,
                detail,
                frozen,
            )
            .map_err(|io_error| PolicyAccessError::Io {
                message: io_error.to_string(),
            })?;
            Err(error)
        }
        Err(error) => Err(error),
    }
}

/// C2 Task 10：页面受限政策读取（blocked／rework 等待面；REST 接线 Task 12
/// 统一）。返回正文＋三元引用（policy_id/revision/digest），不暴露宿主绝对
/// 路径；resolver fail-closed 已落政策核验等待事实。
pub fn read_policy_text_for_attempt(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    attempt_id: &str,
) -> Result<crate::product::logical_codebase::repository_routing::PolicyTextResult, PolicyAccessError> {
    let attempt = load_policy_attempt(paths, project_id, issue_id, attempt_id)?;
    if !matches!(
        attempt.status,
        crate::product::coding_models::CodingAttemptStatus::Blocked
            | crate::product::coding_models::CodingAttemptStatus::WaitingForHuman
    ) {
        return Err(PolicyAccessError::NotAvailable {
            reason: format!("attempt_status_not_waiting_surface: {:?}", attempt.status),
        });
    }
    settle_policy_read(paths, &attempt)
}

/// C2 Task 10：返修 run 运行中受限政策读取输入（HTTP body `{token, role}`）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PolicyTextQueryInput {
    pub token: String,
    pub role: EvidenceRole,
}

/// C2 Task 10：返修 run 运行中受限政策读取。校验链：令牌反查 attempt
/// （Unauthorized）→ Running＋哈希（Forbidden/Unauthorized）→ claims 存在
/// 且 role／attempt／digest／过期全部匹配（否则 Forbidden 提示重新授权，
/// 旧记录无 claims 不误放行）→ resolver 冻结一致性（fail-closed 落等待
/// 事实）。拒绝维度复用 mediator 既有拒绝链语义。
pub fn handle_policy_text_query(
    paths: &ProductAppPaths,
    input: &PolicyTextQueryInput,
) -> Result<crate::product::logical_codebase::repository_routing::PolicyTextResult, PolicyAccessError> {
    let attempt = resolve_attempt_by_token(paths, &input.token).map_err(PolicyAccessError::from)?;
    validate_evidence_token(paths, &attempt, &input.token).map_err(PolicyAccessError::from)?;

    let claims = crate::product::logical_codebase::evidence_token::load_evidence_token_claims(
        paths,
        &attempt,
    )
    .map_err(PolicyAccessError::from)?
    .ok_or_else(|| PolicyAccessError::Forbidden {
        reason: "policy_reauthorization_required_missing_claims".to_string(),
    })?;
    if claims.attempt_id != attempt.id {
        return Err(PolicyAccessError::Forbidden {
            reason: "policy_claims_attempt_mismatch".to_string(),
        });
    }
    if claims.role != input.role.as_str() {
        return Err(PolicyAccessError::Forbidden {
            reason: "policy_claims_role_mismatch".to_string(),
        });
    }
    if policy_authorization_expired(&claims.expires_at) {
        return Err(PolicyAccessError::Forbidden {
            reason: "policy_authorization_expired".to_string(),
        });
    }
    let frozen = frozen_policy_digest(&attempt)?;
    if claims.policy_digest != frozen {
        return Err(PolicyAccessError::Forbidden {
            reason: "policy_claims_digest_mismatch_reauthorization_required".to_string(),
        });
    }
    settle_policy_read(paths, &attempt)
}

/// 墙钟过期判定：RFC3339 解析失败按已过期 fail-closed。
fn policy_authorization_expired(expires_at: &str) -> bool {
    chrono::DateTime::parse_from_rfc3339(expires_at)
        .map(|expiry| chrono::Utc::now() >= expiry)
        .unwrap_or(true)
}

/// C2 Task 10：重新授权请求（用户确认后；REST/页面接线 Task 12 统一）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct PolicyReauthorizationRequest {
    pub command_id: String,
    pub attempt_id: String,
    pub role: String,
    pub policy_digest: String,
    pub expected_version: u64,
}

/// C2 Task 10：重新授权结果——`state` 复用 C1 `OperationState`（Accepted 签发
/// 成功／Replayed 同 command 同 payload 重放／Rejected 版本·身份·role 不符
///／NeedsHuman resolver fail-closed 或签发失败），`expires_at` 仅 Accepted
/// 携带。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PolicyReauthorizationResult {
    pub command_id: String,
    pub state: crate::product::models::automation::OperationState,
    pub attempt_id: String,
    pub reason: Option<String>,
    pub expires_at: Option<String>,
}

/// C2 Task 10：重新授权应用服务。校验链：命令账本幂等（同 command 同
/// payload Replayed 且不旋转令牌，异 payload 冲突）→ expected 版本 → 等待面
/// 状态 → role ∈ {coder, reviewer} → resolver 冻结一致性（fail-closed 落
/// 核验等待事实）→ 签发绑定 attempt＋role＋policy digest 的授权（旋转旧
/// 令牌，旧授权立即失效），仅下一次返修 run 运行态有效。
pub fn handle_policy_reauthorization(
    paths: &ProductAppPaths,
    project_id: &str,
    issue_id: &str,
    request: &PolicyReauthorizationRequest,
) -> Result<PolicyReauthorizationResult, PolicyAccessError> {
    use crate::product::coding_attempt_store::CodingAttemptCommandRecord;
    use crate::product::models::automation::OperationState;

    let attempt = load_policy_attempt(paths, project_id, issue_id, &request.attempt_id)?;
    let store = CodingAttemptStore::new(paths.clone());
    let payload_digest = format!(
        "policy_reauthorization|{}|{}|{}|{}|{}|{}",
        attempt.project_id,
        attempt.issue_id,
        attempt.id,
        request.role,
        request.policy_digest,
        request.expected_version
    );
    let result = |state: OperationState, reason: Option<String>, expires_at: Option<String>| {
        PolicyReauthorizationResult {
            command_id: request.command_id.clone(),
            state,
            attempt_id: attempt.id.clone(),
            reason,
            expires_at,
        }
    };
    let ledger_record = |state: OperationState| CodingAttemptCommandRecord {
        command_id: request.command_id.clone(),
        payload_digest: payload_digest.clone(),
        state,
        recorded_at: Utc::now().to_rfc3339(),
    };
    let append_ledger =
        |state: OperationState| -> Result<(), PolicyAccessError> {
            store
                .append_attempt_command_result(
                    &attempt.project_id,
                    &attempt.issue_id,
                    &attempt.id,
                    &ledger_record(state),
                )
                .map(|_| ())
                .map_err(|error| policy_access_io(format!("append command ledger: {error}")))
        };

    // 命令账本幂等：同 command 同 payload 重放首次 durable 结果（Accepted →
    // Replayed，不旋转已签发令牌）；异 payload fail-closed。
    if let Some(existing) = store
        .find_attempt_command_result(
            &attempt.project_id,
            &attempt.issue_id,
            &attempt.id,
            &request.command_id,
        )
        .map_err(|error| policy_access_io(format!("read command ledger: {error}")))?
    {
        if existing.payload_digest != payload_digest {
            return Err(PolicyAccessError::CommandConflict {
                detail: "同 command 异 payload，请刷新后重试".to_string(),
            });
        }
        if existing.state == OperationState::Accepted {
            return Ok(result(OperationState::Replayed, None, None));
        }
    }

    // 版本校验：错版本 Rejected（请刷新），不改变 attempt、不签发。
    if attempt.version != request.expected_version {
        append_ledger(OperationState::Rejected)?;
        return Ok(result(
            OperationState::Rejected,
            Some(format!(
                "expected version {} but durable version is {}",
                request.expected_version, attempt.version
            )),
            None,
        ));
    }
    // 等待面状态校验：重新授权只在 blocked／rework 等待面提供。
    if !matches!(
        attempt.status,
        crate::product::coding_models::CodingAttemptStatus::Blocked
            | crate::product::coding_models::CodingAttemptStatus::WaitingForHuman
    ) {
        append_ledger(OperationState::Rejected)?;
        return Ok(result(
            OperationState::Rejected,
            Some(format!(
                "attempt_status_not_waiting_surface: {:?}",
                attempt.status
            )),
            None,
        ));
    }
    // role 校验：仅 coder／reviewer 可被授权。
    if request.role != "coder" && request.role != "reviewer" {
        append_ledger(OperationState::Rejected)?;
        return Ok(result(
            OperationState::Rejected,
            Some(format!("invalid_role: {}", request.role)),
            None,
        ));
    }

    // resolver 冻结一致性：不可解析／digest 不一致（含请求 digest 与冻结
    // envelope 不符）→ NeedsHuman fail-closed，落核验等待事实，不签发。
    let frozen = match frozen_policy_digest(&attempt) {
        Ok(frozen) => frozen.to_string(),
        Err(error) => {
            append_ledger(OperationState::NeedsHuman)?;
            return Ok(result(OperationState::NeedsHuman, Some(error.to_string()), None));
        }
    };
    match resolve_frozen_reference(paths, &attempt) {
        Ok(reference) if reference.policy_digest == frozen && request.policy_digest == frozen => {}
        Ok(reference) => {
            let mismatch = if reference.policy_digest != frozen {
                PolicyAccessError::DigestMismatch {
                    expected: frozen.clone(),
                    actual: reference.policy_digest,
                }
            } else {
                PolicyAccessError::DigestMismatch {
                    expected: frozen.clone(),
                    actual: request.policy_digest.clone(),
                }
            };
            land_policy_verification_waiting_fact(
                paths,
                &attempt,
                "policy_digest_mismatch",
                mismatch.to_string(),
                Some(frozen),
            )
            .map_err(|error| policy_access_io(error.to_string()))?;
            append_ledger(OperationState::NeedsHuman)?;
            return Ok(result(OperationState::NeedsHuman, Some(mismatch.to_string()), None));
        }
        Err(error @ PolicyAccessError::ResolverUnavailable { .. }) => {
            land_policy_verification_waiting_fact(
                paths,
                &attempt,
                "policy_resolver_unavailable",
                error.to_string(),
                Some(frozen),
            )
            .map_err(|io_error| policy_access_io(io_error.to_string()))?;
            append_ledger(OperationState::NeedsHuman)?;
            return Ok(result(OperationState::NeedsHuman, Some(error.to_string()), None));
        }
        Err(error) => {
            append_ledger(OperationState::NeedsHuman)?;
            return Ok(result(OperationState::NeedsHuman, Some(error.to_string()), None));
        }
    }

    // 签发：绑定 attempt＋role＋policy digest，仅下一次返修 run 运行态有效。
    if attempt.worktree_path.is_none() {
        append_ledger(OperationState::NeedsHuman)?;
        return Ok(result(
            OperationState::NeedsHuman,
            Some("attempt_worktree_unknown".to_string()),
            None,
        ));
    }
    let expires_at = (Utc::now() + chrono::Duration::hours(POLICY_REAUTHORIZATION_TTL_HOURS))
        .to_rfc3339();
    if let Err(error) = crate::product::logical_codebase::evidence_token::issue_policy_reauthorization(
        paths,
        &attempt,
        &request.role,
        &frozen,
        expires_at.clone(),
    ) {
        append_ledger(OperationState::NeedsHuman)?;
        return Ok(result(
            OperationState::NeedsHuman,
            Some(format!("issue policy reauthorization failed: {error}")),
            None,
        ));
    }

    clear_policy_verification_waiting_fact(paths, &attempt);
    append_ledger(OperationState::Accepted)?;
    Ok(result(OperationState::Accepted, None, Some(expires_at)))
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
    lc_id: Option<&str>,
) -> Result<AggregateIndexRecord, EvidenceError> {
    let pin_path = attempt_pin_path(paths, attempt);
    if pin_path.exists() {
        let pin: EvidenceIndexPinRecord =
            read_json(&pin_path).map_err(|error| EvidenceError::Io {
                message: format!("read evidence index pin {}: {error}", pin_path.display()),
            })?;
        return load_pinned_index(paths, attempt, &pin.pinned_aggregate_index_id, lc_id);
    }

    let pinned_id =
        manifest
            .active_aggregate_index_id
            .clone()
            .ok_or_else(|| EvidenceError::QueryFailed {
                code: "evidence_index_unavailable",
                message: format!(
                    "project {} has no active aggregate index to pin",
                    attempt.project_id
                ),
            })?;
    let record = load_pinned_index(paths, attempt, &pinned_id, lc_id)?;
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
    lc_id: Option<&str>,
) -> Result<AggregateIndexRecord, EvidenceError> {
    let store = match lc_id {
        Some(lc_id) => AggregateIndexStore::for_lc(paths.clone(), lc_id),
        None => AggregateIndexStore::new(paths.clone()),
    };
    store
        .get(&attempt.project_id, pinned_id)?
        .ok_or_else(|| EvidenceError::QueryFailed {
            code: "evidence_index_unavailable",
            message: format!("pinned aggregate index {pinned_id} was not found"),
        })
}

/// 从 attempt 反查 issue 唯一归属的 lc_id（v1.3）；issue 不存在/单仓返回 None。
fn attempt_logical_codebase_id(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
) -> Result<Option<String>, EvidenceError> {
    crate::product::logical_codebase::resolve_issue_logical_codebase_id(
        paths,
        &attempt.project_id,
        &attempt.issue_id,
    )
    .map_err(map_store_error)
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

/// 目标成员目录名推导：读 target_snapshot 字段 + LogicalCodebaseStore member/checkout
/// 记录，本仓目录名取目标成员 checkout canonical_path 最后一段。
fn resolve_target_member_dir(
    logical: &LogicalCodebaseStore,
    attempt: &CodingExecutionAttempt,
    snapshot: &AttemptTargetSnapshot,
) -> Result<String, EvidenceError> {
    let member = logical
        .load_member(&attempt.project_id, snapshot.logical_repository_id)
        .map_err(map_store_error)?
        .ok_or_else(|| EvidenceError::QueryFailed {
            code: "evidence_acl_target_member_missing",
            message: format!(
                "target member {} has no authority record",
                snapshot.logical_repository_id.0
            ),
        })?;
    if member.status != MemberStatus::Active {
        return Err(EvidenceError::QueryFailed {
            code: "evidence_acl_target_member_inactive",
            message: format!(
                "target member {} is not active",
                snapshot.logical_repository_id.0
            ),
        });
    }
    let checkout = logical
        .load_checkout(&attempt.project_id, snapshot.checkout_id)
        .map_err(map_store_error)?
        .ok_or_else(|| EvidenceError::QueryFailed {
            code: "evidence_acl_target_checkout_missing",
            message: format!("target checkout {} has no record", snapshot.checkout_id.0),
        })?;
    if checkout.logical_repository_id != snapshot.logical_repository_id {
        return Err(EvidenceError::QueryFailed {
            code: "evidence_acl_target_checkout_mismatch",
            message: format!(
                "target checkout {} belongs to member {} rather than {}",
                snapshot.checkout_id.0,
                checkout.logical_repository_id.0,
                snapshot.logical_repository_id.0
            ),
        });
    }
    checkout_dir_name(&checkout)
}

/// 成员目录名：按 manifest.member_ids 顺序，对每个成员取其唯一 Main 且 Available 的
/// checkout canonical_path 最后一段（与 aggregate_index::operation 的
/// `included_main_checkouts` 判定一致，但按任务简报取 canonical_path 最后一段）。
fn member_dir_names(
    logical: &LogicalCodebaseStore,
    manifest: &LogicalCodebaseManifest,
) -> Result<Vec<String>, EvidenceError> {
    let members = logical
        .list_members(&manifest.project_id)
        .map_err(map_store_error)?;
    let checkouts = logical
        .list_checkouts(&manifest.project_id)
        .map_err(map_store_error)?;
    let members_by_id: BTreeMap<_, _> = members
        .iter()
        .map(|member| (member.logical_repository_id, member))
        .collect();

    let mut names = Vec::with_capacity(manifest.member_ids.len());
    for member_id in &manifest.member_ids {
        let member = members_by_id.get(member_id).copied().ok_or_else(|| {
            acl_error(format!(
                "manifest member {} has no authority record",
                member_id.0
            ))
        })?;
        if member.status != MemberStatus::Active {
            return Err(acl_error(format!(
                "manifest member {} is not active",
                member_id.0
            )));
        }
        let main_checkouts: Vec<_> = checkouts
            .iter()
            .filter(|checkout| {
                checkout.logical_repository_id == *member_id && checkout.kind == CheckoutKind::Main
            })
            .collect();
        let [checkout] = main_checkouts.as_slice() else {
            return Err(acl_error(format!(
                "manifest member {} must have exactly one main checkout, found {}",
                member_id.0,
                main_checkouts.len()
            )));
        };
        if !member.checkout_ids.contains(&checkout.checkout_id)
            || checkout.availability != CheckoutAvailability::Available
        {
            return Err(acl_error(format!(
                "main checkout {} is not an available checkout of member {}",
                checkout.checkout_id.0, member_id.0
            )));
        }
        names.push(checkout_dir_name(checkout)?);
    }
    Ok(names)
}

/// checkout canonical_path 最后一段作为成员目录名。
fn checkout_dir_name(checkout: &RepositoryCheckoutRecord) -> Result<String, EvidenceError> {
    checkout
        .canonical_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .map(str::to_string)
        .ok_or_else(|| EvidenceError::QueryFailed {
            code: "evidence_acl_checkout_dir",
            message: format!(
                "checkout {} canonical path has no usable directory name: {}",
                checkout.checkout_id.0,
                checkout.canonical_path.display()
            ),
        })
}

fn acl_error(reason: String) -> EvidenceError {
    EvidenceError::QueryFailed {
        code: "evidence_acl_member_invalid",
        message: reason,
    }
}

/// EvidenceHit → 文本行 `file:line symbol`（每行用 symbol 内容，行超单行上限截断单行）。
fn render_hits(hits: &[EvidenceHit]) -> String {
    let mut out = String::new();
    for hit in hits {
        let line = format!("{}:{} {}", hit.file_path, hit.start_line, hit.symbol);
        let line = truncate_chars(&line, EVIDENCE_HIT_LINE_CHAR_LIMIT);
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// 渲染后按 `EVIDENCE_QUERY_RESULT_CHAR_LIMIT` 截断，`truncated` 并附尾部标记。
fn render_and_truncate(hits: &[EvidenceHit]) -> (String, bool) {
    let rendered = render_hits(hits);
    if rendered.chars().count() <= EVIDENCE_QUERY_RESULT_CHAR_LIMIT {
        return (rendered, false);
    }
    let mut truncated = truncate_chars(&rendered, EVIDENCE_QUERY_RESULT_CHAR_LIMIT);
    truncated.push_str(TRUNCATION_MARKER);
    (truncated, true)
}

/// 按字符数截断（char 边界安全）。
fn truncate_chars(value: &str, limit: usize) -> String {
    value.chars().take(limit).collect()
}

// 测试模块按仓库惯例拆入 `.inc.rs`,保持主文件低于 large_file_guard 的 1200 行上限。
include!("evidence_mediator_tests.inc.rs");
