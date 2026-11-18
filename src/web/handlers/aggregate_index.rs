//! HTTP projection and synchronous rebuild endpoints for aggregate indexes.

use super::support::{default_logical_codebase_id, product_app_paths, resolve_lc_authority};
use super::*;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::aggregate_index::codegraph_cli::codegraph_library_present;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexError, AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
};
use crate::product::logical_codebase::failure_copy::{
    SourceLessAlert, describe_envelope, member_source_less_alert,
};
use crate::product::logical_codebase::types::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord,
};
use crate::product::logical_codebase::{LogicalCodebaseStore, RepositoryCheckoutId};
use crate::web::error::ApiError;
use crate::web::state::WebAppState;
use crate::web::types::FailureCopyDto;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct AggregateIndexActiveResponse {
    pub state: &'static str,
    pub revision: Option<u64>,
    pub indexed_at: Option<String>,
    pub warning: Option<String>,
    /// 索引不可用时的中文成品（问题/原因/怎么做/机器码）。
    ///
    /// 只在「跨仓证据检索现在必然失败」时给出：durable 在册的世代指向的聚合根
    /// 上 codegraph 库目录不在（判据与 evidence 链同源：`codegraph_library_present`），
    /// 或根本没有在册世代。索引可用时缺省——库目录在册而查询无命中是合法空结果，
    /// 不进本分支（200 不变）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence_copy: Option<FailureCopyDto>,
    /// 最新一代聚合索引失败时的中文成品（问题/原因/怎么做/机器码）。
    ///
    /// 码来源＝持久化失败封套（`aggregate_index_failed:<code>: <message>` /
    /// `aggregate_index_degraded:<code>: <message>`）里的稳定内层码，
    /// 文案取自 `failure_copy` 唯一码表；无失败（或封套解析不出稳定码）时缺省。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_copy: Option<FailureCopyDto>,
    /// 结构化跳过告警（与成员行、证据 `not_searched` 同构）；无跳过事实时省略。
    ///
    /// **不承载成员名单**（R8）：前端只渲染 `message_zh`，点名在服务端成品里；
    /// 同一响应不得再出现 `skipped_members` 之类的第二个同义字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_less_alert: Option<SourceLessAlert>,
}

pub async fn get_active_aggregate_index(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    get_active_aggregate_index_for_lc(&state, &project_id, &logical_codebase_id)
}

/// v1.3 canonical endpoint: the active projection is resolved per logical
/// codebase.
pub async fn get_lc_active_aggregate_index(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    // C4 Task 2：canonical 路由先经唯一 authority resolver 冻结 LC 身份
    //（conflict fail-closed），再读纯投影。
    resolve_lc_authority(&paths, &project_id, &logical_codebase_id)?;
    get_active_aggregate_index_for_lc(&state, &project_id, &logical_codebase_id)
}

fn get_active_aggregate_index_for_lc(
    state: &WebAppState,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<Response> {
    let paths = product_app_paths(state);
    validate_project_id(project_id)?;
    let response = read_active_projection(&paths, project_id, logical_codebase_id)?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn rebuild_aggregate_index(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    rebuild_aggregate_index_for_lc(state, project_id, logical_codebase_id).await
}

/// v1.3 canonical endpoint: the rebuild is resolved per logical codebase.
pub async fn rebuild_lc_aggregate_index(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    // C4 Task 2：canonical 路由先经唯一 authority resolver 冻结 LC 身份
    //（conflict fail-closed），再执行显式 rebuild 动作。
    resolve_lc_authority(&paths, &project_id, &logical_codebase_id)?;
    rebuild_aggregate_index_for_lc(state, project_id, logical_codebase_id).await
}

async fn rebuild_aggregate_index_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    validate_project_id(&project_id)?;
    let rebuild_key = format!("{project_id}/{logical_codebase_id}");
    let _lease = state
        .aggregate_index_rebuilds
        .try_register(&rebuild_key)
        .ok_or_else(|| {
            // §13/§14 第三出口：并发拒绝同码复用内部条目（同源 describe
            // 三段成品 + details.copy），不再直吐英文原文。
            aggregate_index_rejection("aggregate_index_rebuild_in_progress")
        })?;
    let dependencies = state
        .aggregate_initialization_dependencies()
        .for_lc(logical_codebase_id.clone());
    let operation = dependencies.index.clone();
    let project_id_for_worker = project_id.clone();
    let result = tokio::task::spawn_blocking(move || operation.rebuild(&project_id_for_worker))
        .await
        .map_err(|error| {
            // §13/§14 第三出口：worker join 异常外码走中性专用条目（脱敏
            // 原文进 details.detail），不再直吐英文原文。
            use crate::product::logical_codebase::failure_copy::{
                FailureInput, describe, render_detail,
            };
            let diagnostic = crate::cross_cutting::diagnostic_redaction::sanitize_diagnostic(
                &error.to_string(),
                &crate::cross_cutting::diagnostic_redaction::RedactionContext::default(),
            );
            let copy = describe(
                FailureInput::new("aggregate_index_unavailable")
                    .with_detail(&format!("aggregate index rebuild worker failed: {error}")),
            );
            ApiError::runtime(
                "aggregate_index_unavailable",
                render_detail(&copy, Some(&diagnostic)),
                serde_json::json!({ "detail": diagnostic, "copy": copy }),
            )
        })?;
    if let Err(error) = result {
        return Err(aggregate_index_api_error(error));
    }
    // Keep the lease until after the durable active projection is read. This
    // makes a same-codebase request observe either rebuilding or the new state,
    // never a transient gap between operation completion and response creation.
    let response = read_active_projection(&paths, &project_id, &logical_codebase_id)?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

/// 本次「被投影的那一份」记录：卡片与成员列表（片6）共用的唯一世代选择规则。
///
/// 输入按 `updated_at`/`aggregate_index_id` 降序排好；本函数只做选择：
/// Building 优先（用户正在行动）→ Degraded 优先（失败后的 LKG）→ Failed 取
/// last-known-good → 其余取 latest；`Superseded` 永不被选中（即使排最前）。
/// 片6 的成员列表投影必须复用本函数，不得自写「Active 优先」。
pub(crate) fn projected_record(records: &[AggregateIndexRecord]) -> Option<&AggregateIndexRecord> {
    let latest = records
        .iter()
        .find(|record| record.status != AggregateIndexStatus::Superseded)?;
    if latest.status == AggregateIndexStatus::Building {
        return Some(latest);
    }
    if let Some(degraded) = records
        .iter()
        .find(|record| record.status == AggregateIndexStatus::Degraded)
    {
        return Some(degraded);
    }
    if latest.status == AggregateIndexStatus::Failed {
        return records.iter().find(|record| {
            matches!(
                record.status,
                AggregateIndexStatus::Active
                    | AggregateIndexStatus::Stale
                    | AggregateIndexStatus::Degraded
            )
        });
    }
    Some(latest)
}

fn read_active_projection(
    paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<AggregateIndexActiveResponse> {
    let store = crate::product::logical_codebase::aggregate_index::AggregateIndexStore::for_lc(
        paths.clone(),
        logical_codebase_id,
    );
    let mut records = store
        .records(project_id)
        .map_err(aggregate_index_api_error)?;
    records.retain(|record| record.status != AggregateIndexStatus::Superseded);
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.aggregate_index_id.cmp(&left.aggregate_index_id))
    });
    let projected = projected_record(&records);

    let latest = records.first().cloned();
    // degraded LKG 优先于 active/stale 投影：只要存在一个 degraded generation，
    // 最近一次刷新已失败（成功 rebuild 会把 degraded 前代翻成 superseded）。
    let degraded_record = records
        .iter()
        .find(|candidate| candidate.status == AggregateIndexStatus::Degraded)
        .cloned();
    // 跨仓证据检索可用性（索引不可用 ⇒ 证据检索必然 503）：durable 在册的世代
    // 指向的聚合根上 codegraph 库目录不在 ⇒ 索引不可用；没有任何在册世代 ⇒ 同样
    // 不可用（mediator 无索引可钉住）。判据与 evidence 链的库目录守卫同源。
    let usable_generation = records.iter().find(|record| {
        matches!(
            record.status,
            AggregateIndexStatus::Active
                | AggregateIndexStatus::Stale
                | AggregateIndexStatus::Degraded
        )
    });
    let evidence_unavailable = match usable_generation {
        Some(record) => !codegraph_library_present(&record.codegraph_root),
        None => true,
    };
    let mut response = match latest {
        None => missing_response(None),
        Some(record) if record.status == AggregateIndexStatus::Building => {
            projection("rebuilding", &record, None)
        }
        _ if degraded_record.is_some() => {
            projection("degraded", &degraded_record.expect("checked above"), None)
        }
        Some(record) if record.status == AggregateIndexStatus::Failed => {
            let good = records
                .iter()
                .find(|candidate| {
                    matches!(
                        candidate.status,
                        AggregateIndexStatus::Active
                            | AggregateIndexStatus::Stale
                            | AggregateIndexStatus::Degraded
                    )
                })
                .cloned();
            match good {
                None => missing_response(record.warning),
                Some(good) => projection("degraded", &good, record.warning),
            }
        }
        Some(record) => projection(
            match record.status {
                AggregateIndexStatus::Active => "active",
                AggregateIndexStatus::Stale => "stale",
                AggregateIndexStatus::Degraded => "degraded",
                AggregateIndexStatus::Building => "rebuilding",
                AggregateIndexStatus::Superseded | AggregateIndexStatus::Failed => "missing",
            },
            &record,
            None,
        ),
    };
    // 「重建中」是用户正在行动，不判索引不可用（重建成功后会重建库目录）。
    // 失败面优先于证据面：failure_copy 已给出可操作的重建动作时不再补
    // evidence_copy，同屏只出现一个动作面（前端按同一优先级渲染）。
    if evidence_unavailable && response.state != "rebuilding" && response.failure_copy.is_none() {
        response.evidence_copy = Some(FailureCopyDto::from(
            &crate::product::logical_codebase::failure_copy::describe_code(
                "evidence_index_unavailable",
                None,
            ),
        ));
    }
    // 跳过告警只从「被投影的那一份」记录派生（REQ-IND-06：只读投影）；
    // 重建中不出提示（状态矩阵）；degraded 与失败三段并存，文案走历史措辞。
    let skipped = skipped_member_names(paths, project_id, logical_codebase_id, projected)?;
    if !skipped.is_empty() && response.state != "rebuilding" {
        let names: Vec<&str> = skipped.iter().map(String::as_str).collect();
        response.source_less_alert = Some(member_source_less_alert(
            &names,
            response.state == "degraded",
        ));
    }
    Ok(response)
}

/// 从持久化失败封套（`aggregate_index_failed:<code>: <message>` 或
/// `aggregate_index_degraded:<code>: <message>`）解析出三段中文成品。
///
/// 与冷启动投影、HTTP 错误出口共用产品层唯一封套入口
/// `failure_copy::describe_envelope`：同码同文案**同一点名**（成员名由
/// 共用入口统一抽出，此处不得自行拼接）；封套缺失（软告警不是失败
/// 封套）或解析不出稳定码时返回 `None`。
fn failure_copy_from_warning(warning: Option<&str>) -> Option<FailureCopyDto> {
    let (_inner_code, copy) = describe_envelope(warning?)?;
    Some(FailureCopyDto::from(&copy))
}

/// 从「被投影的那一份」记录派生被跳过的成员名单（REQ-IND-06：只读投影，
/// 不扫盘、不启 CodeGraph、不触发 provider）。
///
/// 匹配口径（M4/C7）：按（成员 ID，**当前**可用的 Main checkout ID）双键
/// 对上快照才贴标，不按显示名/别名/裸成员 ID；顺序按快照顺序
/// （= manifest 成员顺序）。成员存储读不到 ⇒ 降级为空集（不猜、不报错）。
pub(crate) fn skipped_member_names(
    paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    record: Option<&AggregateIndexRecord>,
) -> ApiResult<Vec<String>> {
    let Some(record) = record else {
        return Ok(Vec::new());
    };
    let skipped: Vec<&AggregateIndexMemberSnapshot> = record
        .member_snapshots
        .iter()
        .filter(|snapshot| snapshot.skip_reason.is_some())
        .collect();
    if skipped.is_empty() {
        return Ok(Vec::new());
    }
    let store = LogicalCodebaseStore::for_lc(paths.clone(), logical_codebase_id);
    let members = store
        .list_lc_members(project_id, logical_codebase_id)
        .unwrap_or_default();
    let mut names = Vec::with_capacity(skipped.len());
    for snapshot in skipped {
        let Some(member) = members
            .iter()
            .find(|member| member.logical_repository_id == snapshot.logical_repository_id)
        else {
            continue;
        };
        if is_current_main_checkout(&store, project_id, member, snapshot.checkout_id) {
            names.push(member.alias.clone());
        }
    }
    Ok(names)
}

/// 快照的 checkout 是否仍是该成员「当前可用的 Main 检出」（双键匹配）；
/// 检出记录读不到或已不可用 ⇒ 不算（不得给换过检出的成员贴旧标）。
pub(crate) fn is_current_main_checkout(
    store: &LogicalCodebaseStore,
    project_id: &str,
    member: &CodebaseMemberRecord,
    checkout_id: RepositoryCheckoutId,
) -> bool {
    member.checkout_ids.contains(&checkout_id)
        && store
            .load_checkout(project_id, checkout_id)
            .ok()
            .flatten()
            .is_some_and(|checkout| {
                checkout.kind == CheckoutKind::Main
                    && checkout.availability == CheckoutAvailability::Available
                    && checkout.logical_repository_id == member.logical_repository_id
            })
}

fn projection(
    state: &'static str,
    record: &AggregateIndexRecord,
    warning: Option<String>,
) -> AggregateIndexActiveResponse {
    let warning = warning.or_else(|| record.warning.clone());
    let failure_copy = failure_copy_from_warning(warning.as_deref());
    AggregateIndexActiveResponse {
        state,
        revision: Some(record.membership_revision),
        indexed_at: Some(record.updated_at.clone()),
        warning,
        evidence_copy: None,
        failure_copy,
        source_less_alert: None,
    }
}

fn missing_response(warning: Option<String>) -> AggregateIndexActiveResponse {
    AggregateIndexActiveResponse {
        state: "missing",
        revision: None,
        indexed_at: None,
        failure_copy: failure_copy_from_warning(warning.as_deref()),
        warning,
        evidence_copy: None,
        source_less_alert: None,
    }
}

fn validate_project_id(project_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })?;
    Ok(())
}

/// 聚合索引同步出口（第三出口）：内层稳定 code + message → **同一**文案来源。
///
/// 外层 code 与 HTTP 状态保持既有契约（`aggregate_index_unavailable`，
/// `details.reason_code` 仍是内层码）；`message` 换成共享 `render_detail`
/// 的三段成品（问题/原因/怎么做/机器码），原始错误脱敏后另存
/// `details.detail`，不冒充中文原因。
fn aggregate_index_api_error(error: AggregateIndexError) -> ApiError {
    use crate::product::logical_codebase::failure_copy::{
        FailureInput, describe, render_detail, subject_from_message,
    };
    let (code, message) = match &error {
        AggregateIndexError::Failed { code, message }
        | AggregateIndexError::Degraded { code, message } => (*code, message.as_str()),
    };
    // 已知句式能点名的对象（如覆盖失败里的成员名清单）要进原因段，
    // 原始英文串仍整句留在脱敏诊断里；subject 抬到函数级作用域供 input 借用。
    let subject = subject_from_message(code, message);
    let mut input = FailureInput::new(code).with_detail(message);
    if let Some(subject) = subject.as_deref() {
        input = input.with_subject(subject);
    }
    let copy = describe(input);
    let diagnostic = crate::cross_cutting::diagnostic_redaction::sanitize_diagnostic(
        message,
        &crate::cross_cutting::diagnostic_redaction::RedactionContext::default(),
    );
    let rendered = render_detail(&copy, None);
    ApiError::runtime(
        "aggregate_index_unavailable",
        rendered,
        serde_json::json!({
            "reason_code": code,
            "detail": diagnostic,
            "copy": copy,
        }),
    )
}

/// §13/§14 第三出口：无内层错误的外层同步拒绝（并发 rebuild 等）——
/// 同码复用共享文案源（describe 三段成品 + details.copy），不直吐英文。
fn aggregate_index_rejection(code: &'static str) -> ApiError {
    use crate::product::logical_codebase::failure_copy::{FailureInput, describe, render_detail};
    let copy = describe(FailureInput::new(code));
    ApiError::runtime(
        code,
        render_detail(&copy, None),
        serde_json::json!({ "copy": copy }),
    )
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_index::AggregateIndexStore;

    fn record(
        aggregate_index_id: &str,
        status: AggregateIndexStatus,
        updated_at: &str,
    ) -> AggregateIndexRecord {
        let mut record = AggregateIndexRecord::building(
            aggregate_index_id.to_string(),
            "project_0001".to_string(),
            3,
            Vec::new(),
            updated_at.to_string(),
        );
        record.status = status;
        record
    }

    #[test]
    fn read_active_projection_reports_missing_when_no_generation_exists() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "missing");
        assert_eq!(response.revision, None);
    }

    #[test]
    fn read_active_projection_maps_first_build_failure_to_missing_with_warning() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_failed_gen",
                    AggregateIndexStatus::Failed,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_failed_gen",
                AggregateIndexStatus::Failed,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        // 无 LKG：首建失败投影 missing，但保留可操作 warning。
        assert_eq!(response.state, "missing");
        assert!(
            response
                .warning
                .as_deref()
                .unwrap_or_default()
                .contains("codegraph_init_failed")
        );
    }

    #[test]
    fn read_active_projection_maps_rebuild_failure_to_degraded_last_known_good() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_lkg",
                    AggregateIndexStatus::Active,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        // rebuild 失败现场：LKG 被 degrade，新 generation 以 Stale + warning 落盘。
        store
            .mark_status(
                "project_0001",
                "aggregate_index_lkg",
                AggregateIndexStatus::Degraded,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_refresh_gen",
                    AggregateIndexStatus::Stale,
                    "2026-09-28T02:00:00Z",
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_refresh_gen",
                AggregateIndexStatus::Stale,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "degraded");
        assert_eq!(response.revision, Some(3));
        assert!(
            response
                .warning
                .as_deref()
                .unwrap_or_default()
                .contains("codegraph_init_failed")
        );
    }

    /// 索引在册但磁盘 codegraph 库目录不在 ⇒ 跨仓证据检索必然 503
    /// （`evidence_index_unavailable`）⇒ 投影必须带中文成品 + 入口指引。
    #[test]
    fn read_active_projection_reports_evidence_unavailable_when_library_dir_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        let mut generation = record(
            "aggregate_index_gen",
            AggregateIndexStatus::Active,
            "2026-09-28T01:00:00Z",
        );
        // 在册世代指向的聚合根上没有 `.codegraph` 库目录。
        generation.codegraph_root = temp.path().join("aggregate-root");
        store.create("project_0001", generation).unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "active");
        let copy = response
            .evidence_copy
            .expect("库目录缺失 ⇒ 跨仓证据检索不可用，投影必须给出中文成品");
        assert_eq!(copy.machine_line, "evidence_index_unavailable");
        assert!(copy.action_zh.contains("重建索引"));
        assert!(copy.action_zh.contains("逻辑代码库管理面板"));
    }

    /// 硬边界①：库目录在册（哪怕查询零命中）⇒ 不是索引不可用，投影不带成品。
    #[test]
    fn read_active_projection_has_no_evidence_copy_when_library_dir_exists() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        let mut generation = record(
            "aggregate_index_gen",
            AggregateIndexStatus::Active,
            "2026-09-28T01:00:00Z",
        );
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(aggregate_root.join(".codegraph")).unwrap();
        generation.codegraph_root = aggregate_root;
        store.create("project_0001", generation).unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "active");
        assert!(
            response.evidence_copy.is_none(),
            "库目录在册 ⇒ 查询无命中仍是合法空结果，不进索引不可用分支"
        );
    }

    /// 失败封套（成员覆盖不足码）⇒ 投影出三段中文成品：原因段点名具体成员、
    /// 机器码是稳定内层码；原始英文串留在 warning 里当诊断，不进成品。
    #[test]
    fn read_active_projection_renders_failure_copy_for_coverage_failure() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_failed_gen",
                    AggregateIndexStatus::Failed,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        let warning = concat!(
            "aggregate_index_failed:aggregate_index_member_coverage_failed: ",
            "no indexable code file under included members ",
            "[gray-config, msha-sdk]; ",
            "each included member needs at least 1 indexable code file"
        )
        .to_string();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_failed_gen",
                AggregateIndexStatus::Failed,
                Some(warning),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "missing");
        let copy = response
            .failure_copy
            .expect("失败封套在册 ⇒ 投影必须带三段中文成品");
        assert_eq!(copy.machine_line, "aggregate_index_member_coverage_failed");
        assert!(
            copy.title_zh.contains("没有可索引的代码文件"),
            "标题不对：{}",
            copy.title_zh
        );
        assert!(
            copy.reason_zh.contains("gray-config"),
            "原因段：{}",
            copy.reason_zh
        );
        assert!(
            copy.reason_zh.contains("msha-sdk"),
            "原因段：{}",
            copy.reason_zh
        );
        assert!(!copy.action_zh.trim().is_empty());
    }

    /// 硬边界：Active 世代的 warning 是健康闸软告警（不是失败封套）
    /// ⇒ 不算失败，failure_copy 必须缺省。
    #[test]
    fn read_active_projection_has_no_failure_copy_for_soft_health_warning() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        let mut generation = record(
            "aggregate_index_gen",
            AggregateIndexStatus::Active,
            "2026-09-28T01:00:00Z",
        );
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(aggregate_root.join(".codegraph")).unwrap();
        generation.codegraph_root = aggregate_root;
        generation.warning =
            Some("codegraph 索引健康闸（不阻断激活）：跳过部分成员的索引".to_string());
        store.create("project_0001", generation).unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "active");
        assert!(response.failure_copy.is_none());
    }

    /// 重建失败（第三出口）：覆盖不足码的 details.copy 原因段点名具体成员，
    /// 机器码是稳定内层码。
    #[test]
    fn aggregate_index_api_error_names_members_for_coverage_failure() {
        let message = concat!(
            "no indexable code file under included members ",
            "[gray-config, msha-sdk]; ",
            "each included member needs at least 1 indexable code file"
        )
        .to_string();
        let error = AggregateIndexError::Failed {
            code: "aggregate_index_member_coverage_failed",
            message,
        };
        let api_error = aggregate_index_api_error(error);
        let copy = &api_error.details["copy"];
        let reason_zh = copy["reason_zh"].as_str().unwrap_or_default();
        assert!(reason_zh.contains("gray-config"), "原因段：{reason_zh}");
        assert!(reason_zh.contains("msha-sdk"), "原因段：{reason_zh}");
        assert_eq!(
            copy["machine_line"].as_str(),
            Some("aggregate_index_member_coverage_failed")
        );
    }

    /// 没有任何在册世代 ⇒ 同样不可用（mediator 无索引可钉住）。
    #[test]
    fn read_active_projection_reports_evidence_unavailable_without_any_generation() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "missing");
        assert!(response.evidence_copy.is_some());
    }

    #[test]
    fn read_active_projection_reports_rebuilding_while_generation_is_building() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_building_gen",
                    AggregateIndexStatus::Building,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "rebuilding");
    }

    /// （修1·a）Rebuild/Sync 失败且无 LKG：新世代以 Stale + 失败封套落盘
    /// ⇒ 投影仍是 `stale` 且带三段中文成品；主检出不可用码必须命中
    /// 具体文案（点名成员主检出与修复动作），不能落到通用重试兜底。
    #[test]
    fn read_active_projection_keeps_stale_state_and_failure_copy_without_lkg() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_refresh_gen",
                    AggregateIndexStatus::Stale,
                    "2026-09-28T02:00:00Z",
                ),
            )
            .unwrap();
        // 生产形态（snapshot.rs）：message 只是该成员主检出的 canonical path。
        store
            .mark_status(
                "project_0001",
                "aggregate_index_refresh_gen",
                AggregateIndexStatus::Stale,
                Some(
                    "aggregate_index_failed:aggregate_index_checkout_unavailable: \
                     /workspace/members/gray-config"
                        .to_string(),
                ),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "stale");
        let copy = response
            .failure_copy
            .expect("失败封套在册 ⇒ Stale 世代也必须带三段中文成品");
        assert_eq!(copy.machine_line, "aggregate_index_checkout_unavailable");
        assert!(
            copy.reason_zh.contains("主检出"),
            "原因段必须点名成员主检出不可用：{}",
            copy.reason_zh
        );
        assert!(
            copy.action_zh.contains("重新 clone") || copy.action_zh.contains("重新登记"),
            "怎么做必须给出修复检出的具体动作：{}",
            copy.action_zh
        );
    }

    /// （修1·b）Degraded LKG 的 warning 封套前缀是 `aggregate_index_degraded:`
    /// ⇒ 两种封套前缀都解析，机器码仍是封套里的稳定内层码。
    #[test]
    fn read_active_projection_renders_degraded_envelope_with_inner_machine_code() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_lkg",
                    AggregateIndexStatus::Active,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_lkg",
                AggregateIndexStatus::Degraded,
                Some("aggregate_index_degraded:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "degraded");
        let copy = response
            .failure_copy
            .expect("degraded 封套同样要出三段中文成品");
        assert_eq!(copy.machine_line, "codegraph_init_failed");
    }

    /// （修1·c）码表外 code 的封套 ⇒ 中性兜底：机器码仍是封套里的内层码，
    /// 不吞码、不拿英文原文当中文主标题。
    #[test]
    fn read_active_projection_neutral_fallback_keeps_inner_machine_code() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_failed_gen",
                    AggregateIndexStatus::Failed,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_failed_gen",
                AggregateIndexStatus::Failed,
                Some(
                    "aggregate_index_failed:aggregate_index_member_checkout_retired: /gone"
                        .to_string(),
                ),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "missing");
        let copy = response
            .failure_copy
            .expect("封套可解析 ⇒ 必须出中性三段成品，而不是吞掉失败面");
        assert_eq!(copy.machine_line, "aggregate_index_member_checkout_retired");
        assert!(
            copy.reason_zh.contains("无法确定具体原因"),
            "码表外码必须走中性兜底：{}",
            copy.reason_zh
        );
    }

    /// （修2·d）首建失败（无 LKG）时失败面优先：`failure_copy` 存在 ⇒
    /// 不再补 `evidence_copy`，同屏只有一个动作面（前端按同一优先级渲染）。
    #[test]
    fn read_active_projection_suppresses_evidence_copy_when_failure_copy_present() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_failed_gen",
                    AggregateIndexStatus::Failed,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_failed_gen",
                AggregateIndexStatus::Failed,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "missing");
        assert!(
            response.failure_copy.is_some(),
            "失败封套在册 ⇒ 必须带三段中文成品"
        );
        assert!(
            response.evidence_copy.is_none(),
            "failure_copy 优先：证据面不得与失败面同屏"
        );
    }

    // ---- C4 Task 2：旧 project 级布局不得战胜显式 LC 解析 ----

    #[test]
    fn old_project_layout_never_wins_over_explicit_lc_resolution() {
        use crate::product::id::repo_hash_for_path;
        use crate::product::issue_store::CreateProductIssueInput;
        use crate::product::logical_codebase::issue_selection::IssueCodebaseSelectionStore;
        use crate::product::logical_codebase::policy::PolicyTarget;
        use crate::product::logical_codebase::provider_gateway::{
            ProviderRef, SessionLaunchRequest,
        };
        use crate::product::logical_codebase::types::{
            CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, MemberStatus,
            RepositoryCheckoutRecord, RepositoryType,
        };
        use crate::product::logical_codebase::{
            IssueCodebaseSelection, LogicalCodebaseCreateInput, LogicalCodebaseManifest,
            LogicalCodebaseStore, LogicalRepositoryId, PlanningContextSetResolver,
            PolicyTargetResolver, ProductionPolicyTargetResolver, RepositoryCheckoutId,
        };
        use crate::product::project_store::{CreateProjectInput, ProjectStore};

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let project_id = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "legacy-layout-project".to_string(),
                description: None,
            })
            .unwrap()
            .id;

        // 真实成员仓（git dir 身份用于 source identity）。
        let repo = temp.path().join("workspace").join("repo-shared");
        std::fs::create_dir_all(&repo).unwrap();
        for arguments in [
            vec!["init", "-b", "main"],
            vec!["config", "user.email", "legacy@test.local"],
            vec!["config", "user.name", "Legacy Test"],
        ] {
            let output = std::process::Command::new("git")
                .current_dir(&repo)
                .args(&arguments)
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        std::fs::write(repo.join("README.md"), "# shared\n").unwrap();
        let output = std::process::Command::new("git")
            .current_dir(&repo)
            .args(["add", "."])
            .output()
            .unwrap();
        assert!(output.status.success());
        let output = std::process::Command::new("git")
            .current_dir(&repo)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let canonical = std::fs::canonicalize(&repo).unwrap();
        let source =
            crate::product::repository_store::resolve_repository_source(&canonical).unwrap();

        // 旧 project 级布局：project-level manifest + 同 source 成员，并迁移出别名 LC。
        let legacy_store = LogicalCodebaseStore::new(paths.clone());
        let legacy_member = LogicalRepositoryId(uuid::Uuid::new_v4());
        let mut legacy_manifest =
            LogicalCodebaseManifest::new(&project_id, temp.path().join("workspace"), Vec::new());
        legacy_manifest.member_ids = vec![legacy_member];
        legacy_store
            .save_manifest(&project_id, &legacy_manifest)
            .unwrap();
        legacy_store
            .save_member(
                &project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: legacy_member,
                    physical_repository_id: "repository_legacy_member".to_string(),
                    alias: "repo-shared".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: Vec::new(),
                    status: MemberStatus::Active,
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        legacy_store.migrate_legacy(&project_id).unwrap();

        // 显式新 LC：不同 root，但成员 source 与旧布局重叠。
        let explicit_root = temp.path().join("explicit-root");
        std::fs::create_dir_all(&explicit_root).unwrap();
        let lc2 = LogicalCodebaseStore::new(paths.clone())
            .create(
                &project_id,
                LogicalCodebaseCreateInput {
                    name: "explicit".to_string(),
                    aggregate_root: explicit_root.clone(),
                },
            )
            .unwrap()
            .id;
        let lc2_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc2);
        let member2 = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout2 = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let mut manifest2 =
            LogicalCodebaseManifest::new(&project_id, explicit_root.clone(), Vec::new());
        manifest2.member_ids = vec![member2];
        lc2_store.save_manifest(&project_id, &manifest2).unwrap();
        lc2_store
            .save_member(
                &project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: member2,
                    physical_repository_id: "repository_explicit_member".to_string(),
                    alias: "repo-shared".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout2],
                    status: MemberStatus::Active,
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        lc2_store
            .save_checkout(
                &project_id,
                &RepositoryCheckoutRecord {
                    checkout_id: checkout2,
                    logical_repository_id: member2,
                    physical_repository_id: "repository_explicit_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: repo_hash_for_path(canonical.to_string_lossy().as_ref()),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-09-29T00:00:00Z".to_string(),
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();

        // issue 显式归属 lc2。
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: lc2.clone(),
                title: "explicit issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        IssueCodebaseSelectionStore::for_lc(paths.clone(), &lc2)
            .save(
                &IssueCodebaseSelection::all_members(&project_id, &issue.id, None)
                    .for_logical_codebase(&lc2),
            )
            .unwrap();

        let expected_conflict = "repository_routing_legacy_conflict";

        // 入口 1：规划/工作区上下文（PlanningContextSetResolver）。
        let planning =
            PlanningContextSetResolver::new(paths.clone()).resolve(&project_id, &issue.id);
        match planning {
            Err(crate::product::json_store::ProductStoreError::Conflict { kind, .. }) => {
                assert_eq!(kind, expected_conflict);
            }
            other => panic!("planning must fail closed with legacy conflict, got {other:?}"),
        }

        // 入口 2：aggregate index canonical GET 的身份校验。
        let index_error = super::super::support::resolve_lc_authority(&paths, &project_id, &lc2)
            .expect_err("aggregate index GET identity check must fail closed");
        assert_eq!(index_error.code, expected_conflict);

        // 入口 3：policy target resolver（provider spawn 前复验）。
        let request = SessionLaunchRequest::planning(
            project_id.clone(),
            ProviderRef::claude_code("cap_snapshot"),
            PolicyTarget::checkout(
                member2.0.to_string(),
                checkout2.0.to_string(),
                canonical.clone(),
            ),
            vec![canonical.clone()],
            "sha256:managed-config-artifact",
        );
        let policy_error = ProductionPolicyTargetResolver::for_lc(paths.clone(), &lc2)
            .resolve_and_revalidate(&request)
            .expect_err("policy target resolver must fail closed");
        match policy_error {
            crate::product::logical_codebase::ProviderGatewayError::Target(reason) => {
                assert!(
                    reason.contains(expected_conflict),
                    "policy resolver error must carry the stable conflict code: {reason}"
                );
            }
            other => panic!("expected Target error, got {other:?}"),
        }
    }
}
// 片5 测试外挪（large_file_guard 1200 行上限）：结构化跳过告警 + 世代取法
// 的用例经 include! 挂载，与本文件同一模块作用域（纯外挪，无行为变化）。
#[cfg(test)]
mod source_less_tests {
    include!("aggregate_index_source_less_tests.inc.rs");
}
