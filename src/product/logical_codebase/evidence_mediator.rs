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
