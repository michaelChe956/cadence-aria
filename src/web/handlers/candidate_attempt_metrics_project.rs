//! REQ-WSC-25 **项目级**通过率读数的只读 HTTP 出口。
//!
//! 按单个 issue 算出来的 `first_pass_rate` 只能是 0.0 或 1.0——那不是「率」
//! （`issue_count` 恒为 1）。真正的通过率必须跨 issue 聚合：
//! `compute_candidate_attempt_metrics` 本来就是吃**多个 issue 的序列**设计的。
//!
//! `GET /api/projects/{project_id}/candidate-attempt-metrics`（可带 `?top_n=`）
//!
//! - 把该项目下**所有 issue** 的尝试序列喂进**同一个**既有 metrics 函数
//!   （不另造一套指标口径）；
//! - 同时返回 issue 维度明细（`per_issue`），让 owner 能定位到是哪个 issue
//!   在拖后腿；
//! - **零写操作**：不落盘、不改写 append-only 流水（读数靠流水活着）。
//!
//! 口径说明：项目下只有一个 issue 时，项目级读数与按 issue 读数**同值**
//! （`issue_count` = 1 → 率只能是 0.0/1.0）——这是口径的必然结果，不是造假。

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::candidate_attempts::{
    CandidateAttemptOutcome, compute_candidate_attempt_metrics,
};
use crate::web::error::{ApiError, ApiResult};
use crate::web::state::WebAppState;
use axum::Json;
use axum::extract::{Path, Query, State};
use serde::Serialize;

use super::candidate_attempt_metrics::{
    CandidateAttemptMetricsDto, CandidateAttemptMetricsQuery, DEFAULT_FAILURE_CODE_TOP_N,
    MAX_FAILURE_CODE_TOP_N,
};
use super::support::{product_app_paths, product_store_api_error};

/// 项目级读数中的 issue 明细（一条 = 一个 issue 的尝试序列读数）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectIssueAttemptMetricsDto {
    /// issue 标识。
    pub issue_id: String,
    /// 该 issue 的尝试总数。
    pub attempts: usize,
    /// 首次尝试是否即通过（首次通过率的分项）。
    pub first_pass: bool,
    /// 首次通过之前的连续失败次数（连续失败分布的分项）。
    pub consecutive_failures: usize,
}

/// 项目级读数响应（`metrics` 与按 issue 出口**同形同口径**）。
#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct ProjectCandidateAttemptMetricsResponse {
    pub project_id: String,
    /// 项目级读数（复用既有 `CandidateAttemptMetrics` 口径）。
    pub metrics: CandidateAttemptMetricsDto,
    /// issue 维度明细（定位拖后腿的 issue）。
    pub per_issue: Vec<ProjectIssueAttemptMetricsDto>,
}

/// 项目级只读读数出口：跨 issue 聚合的读数 + issue 明细。
pub async fn get_project_candidate_attempt_metrics(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Query(query): Query<CandidateAttemptMetricsQuery>,
) -> ApiResult<Json<ProjectCandidateAttemptMetricsResponse>> {
    crate::product::json_store::validate_relative_id(&project_id)
        .map_err(|_| ApiError::validation("invalid_project_id", "project id must be relative"))?;
    let lifecycle = LifecycleStore::new(product_app_paths(&state));
    let series = lifecycle
        .list_project_candidate_attempt_series(&project_id)
        .map_err(product_store_api_error)?;
    let metrics = compute_candidate_attempt_metrics(&series);
    let per_issue = series
        .iter()
        .map(|issue| {
            let mut records = issue.records.clone();
            records.sort_by(|left, right| {
                (left.recorded_at.as_str(), left.attempt_seq)
                    .cmp(&(right.recorded_at.as_str(), right.attempt_seq))
            });
            ProjectIssueAttemptMetricsDto {
                issue_id: issue.issue_id.clone(),
                attempts: records.len(),
                first_pass: records
                    .first()
                    .is_some_and(|record| record.outcome == CandidateAttemptOutcome::Passed),
                consecutive_failures: records
                    .iter()
                    .take_while(|record| record.outcome == CandidateAttemptOutcome::Failed)
                    .count(),
            }
        })
        .collect();
    let top_n = query
        .top_n
        .unwrap_or(DEFAULT_FAILURE_CODE_TOP_N)
        .clamp(1, MAX_FAILURE_CODE_TOP_N);
    Ok(Json(ProjectCandidateAttemptMetricsResponse {
        project_id,
        metrics: CandidateAttemptMetricsDto::from_metrics(metrics, top_n),
        per_issue,
    }))
}
