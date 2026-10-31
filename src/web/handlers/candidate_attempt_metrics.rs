//! REQ-WSC-25 度量读数的只读 HTTP 出口（B）。
//!
//! `compute_candidate_attempt_metrics` / `list_issue_candidate_attempts` 已实现
//! 但此前没有任何 HTTP/CLI 调用方——owner 打不开通过率读数。本模块补一个
//! **只读** GET 端点：
//!
//! `GET /api/projects/{project_id}/issues/{issue_id}/candidate-attempt-metrics`
//!
//! - 返回该 issue 的尝试流水序列（跨会话，按时间升序）+ 读数（首次通过率、
//!   同 issue 连续失败次数 P50/P90、失败码 Top N、lowering 类终态失败计数）
//!   ——字段与 `candidate_attempts::CandidateAttemptMetrics` 既有结构同形，
//!   不另造形状；
//! - **零写操作**：不落盘、不改写 append-only 流水（读数靠流水活着）；
//! - 鉴权/来源沿用既有 `/api/projects/...` 路由的最外层中间件
//!   （Host/Origin 白名单已在 `build_web_router` 统一挂载，无需重复实现）。

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::candidate_attempts::{
    CandidateAttemptRecord, CandidateIssueAttemptSeries, compute_candidate_attempt_metrics,
};
use crate::web::error::ApiResult;
use crate::web::state::WebAppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::{Deserialize, Serialize};

use super::automation_enrollment::{ensure_issue_exists, validate_request_ids};
use super::support::{product_app_paths, product_store_api_error};

/// 失败码 Top N 的缺省条数与上界（读数可复制口径，jq 等价查法同 n=5）。
const DEFAULT_FAILURE_CODE_TOP_N: usize = 5;
const MAX_FAILURE_CODE_TOP_N: usize = 50;

/// 只读查询参数：`top_n` 缺省 5（失败码 Top N 条数）。
#[derive(Debug, Default, Deserialize)]
pub struct CandidateAttemptMetricsQuery {
    pub top_n: Option<usize>,
}

/// 失败码分布条目（与 `CandidateFailureCodeCount` 同形）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CandidateFailureCodeCountDto {
    pub code: String,
    pub count: usize,
}

/// 读数（与 `CandidateAttemptMetrics` 既有字段同形，一字段不增减）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct CandidateAttemptMetricsDto {
    pub issue_count: usize,
    pub first_pass_issue_count: usize,
    pub first_pass_rate: Option<f64>,
    pub consecutive_failure_p50: Option<f64>,
    pub consecutive_failure_p90: Option<f64>,
    pub failure_code_ranking: Vec<CandidateFailureCodeCountDto>,
    pub lowering_terminal_failure_count: usize,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct IssueCandidateAttemptMetricsResponse {
    pub project_id: String,
    pub issue_id: String,
    /// 该 issue 的尝试流水序列（跨会话、按落盘时间升序；store 单源读取）。
    pub attempts: Vec<CandidateAttemptRecord>,
    pub metrics: CandidateAttemptMetricsDto,
}

/// 只读读数出口：序列 + 读数同帧返回（owner 一次打开即可对账到逐次尝试）。
pub async fn get_issue_candidate_attempt_metrics(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Query(query): Query<CandidateAttemptMetricsQuery>,
) -> ApiResult<Json<IssueCandidateAttemptMetricsResponse>> {
    validate_request_ids(&project_id, &issue_id)?;
    ensure_issue_exists(&state, &project_id, &issue_id)?;
    let lifecycle = LifecycleStore::new(product_app_paths(&state));
    let attempts = lifecycle
        .list_issue_candidate_attempts(&project_id, &issue_id)
        .map_err(product_store_api_error)?;
    let series = vec![CandidateIssueAttemptSeries {
        issue_id: issue_id.clone(),
        records: attempts.clone(),
    }];
    let metrics = compute_candidate_attempt_metrics(&series);
    let top_n = query
        .top_n
        .unwrap_or(DEFAULT_FAILURE_CODE_TOP_N)
        .clamp(1, MAX_FAILURE_CODE_TOP_N);
    Ok(Json(IssueCandidateAttemptMetricsResponse {
        project_id,
        issue_id,
        attempts,
        metrics: CandidateAttemptMetricsDto {
            issue_count: metrics.issue_count,
            first_pass_issue_count: metrics.first_pass_issue_count,
            first_pass_rate: metrics.first_pass_rate,
            consecutive_failure_p50: metrics.consecutive_failure_p50,
            consecutive_failure_p90: metrics.consecutive_failure_p90,
            failure_code_ranking: metrics
                .failure_code_ranking
                .into_iter()
                .take(top_n)
                .map(|entry| CandidateFailureCodeCountDto {
                    code: entry.code,
                    count: entry.count,
                })
                .collect(),
            lowering_terminal_failure_count: metrics.lowering_terminal_failure_count,
        },
    }))
}
