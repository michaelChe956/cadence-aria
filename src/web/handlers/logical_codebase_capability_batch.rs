//! 票3「一键核验全部」HTTP 面（REQ-PCR-03/04/05，**机器级**路由）。
//!
//! provider 是平台公用能力 ⇒ 批量核验作业**不绑定项目 / 逻辑代码库**
//! （REQ-PCR-03）：作业只凭 `job_id` 定位与读回。
//!
//! - `POST /api/providers/capabilities/revalidate-all`：用户显式发起批量
//!   核验，**立即**返回 `{job_id, status:"running"}`（202 Accepted，
//!   MUST NOT 在响应内等待探针）。
//! - `GET /api/providers/capabilities/revalidate-all/{job_id}`：轮询作业状态
//!   + 逐家结果（durable，终态仍可读）。
//!
//! 单家语义（探针 / 幂等短路 / CLI 缺失快速失败）一律复用既有
//! `ProviderCapabilityRevalidateService`（本文件不改其一行）；本文件只做
//! 批量作业的 HTTP 装配。

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::{Value, json};

use super::support::{product_app_paths, product_store_api_error};
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::capability_revalidate_batch::{
    CapabilityRevalidateBatchJob, CapabilityRevalidateBatchService, revalidate_call_for,
};
use crate::web::error::{ApiError, ApiResult};
use crate::web::state::WebAppState;

/// 批量核验服务装配：逐家核验走 state 注入的单家服务（it_web fake seam /
/// 生产真实探针同源），缺省按请求 paths 构造真实服务。
fn batch_service(state: &WebAppState) -> CapabilityRevalidateBatchService {
    let service = CapabilityRevalidateBatchService::new(product_app_paths(state));
    match state.capability_revalidate_service.as_ref() {
        Some(injected) => {
            service.with_revalidate_call(revalidate_call_for(injected.as_ref().clone()))
        }
        None => service,
    }
}

/// 轮询响应体（`{job_id, status, results[]}`；逐项进度见 `results`）。
fn job_dto(job: &CapabilityRevalidateBatchJob) -> Value {
    json!({
        "job_id": job.job_id,
        "status": job.status,
        "created_at": job.created_at,
        "updated_at": job.updated_at,
        "finished_at": job.finished_at,
        "results": job.results,
    })
}

/// POST `/api/providers/capabilities/revalidate-all`：显式发起批量核验
/// （异步作业，立即返回 `job_id`；机器级，无项目 / 逻辑代码库前置）。
pub async fn post_lc_capability_revalidate_all(
    State(state): State<WebAppState>,
) -> ApiResult<Response> {
    let job = batch_service(&state)
        .start()
        .map_err(product_store_api_error)?;
    Ok((
        StatusCode::ACCEPTED,
        Json(json!({ "job_id": job.job_id, "status": job.status })),
    )
        .into_response())
}

/// GET `/api/providers/capabilities/revalidate-all/{job_id}`：轮询作业状态与
/// 逐家结果（durable，终态仍可读；只凭 `job_id`）。
pub async fn get_lc_capability_revalidate_all_job(
    State(state): State<WebAppState>,
    Path(job_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let service = CapabilityRevalidateBatchService::new(paths);
    match service.get(&job_id) {
        Ok(job) => Ok(Json(job_dto(&job)).into_response()),
        // 未知 / 过期 job_id：明确的 404 稳定码（不返回空作业冒充进度）。
        // 注：`src/web/error.rs` 的状态映射表归属片 1（跨块串行文件），本片
        // 不新增臂 ⇒ 这里直接给出 404 + 同一 `{code,message,details}` 信封。
        Err(ProductStoreError::NotFound { .. }) => Ok((
            StatusCode::NOT_FOUND,
            Json(ApiError::runtime(
                "capability_revalidate_batch_job_not_found",
                "capability revalidate batch job not found",
                json!({ "job_id": job_id }),
            )),
        )
            .into_response()),
        Err(error) => Err(product_store_api_error(error)),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use axum::extract::{Path, State};
    use axum::http::StatusCode;
    use serde_json::Value;
    use tempfile::TempDir;

    use super::*;
    use crate::cross_cutting::provider_boundary::ProviderBoundaryError;
    use crate::product::logical_codebase::provider_capability_revalidate::{
        ProbeInvocation, ProviderCapabilityRevalidateService,
    };
    use crate::web::events::EventHub;
    use crate::web::runtime::WebRuntime;

    struct Fixture {
        state: WebAppState,
        _root: TempDir,
    }

    /// 机器级夹具（REQ-PCR-03）：批量核验面**不要求**任何项目 / 逻辑代码库
    /// 在场 ⇒ 夹具一个都不建。
    fn fixture(injected: Option<ProviderCapabilityRevalidateService>) -> Fixture {
        let root = tempfile::tempdir().expect("temp root");
        let mut state = WebAppState::with_events(
            root.path().to_path_buf(),
            WebRuntime::new_fake(root.path().to_path_buf()),
            EventHub::new(),
        );
        if let Some(service) = injected {
            state = state.with_capability_revalidate_service(Arc::new(service));
        }
        Fixture { state, _root: root }
    }

    async fn body_json(response: Response) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .expect("body");
        serde_json::from_slice(&bytes).expect("json body")
    }

    async fn post_batch(fixture: &Fixture) -> (StatusCode, Value) {
        let response = post_lc_capability_revalidate_all(State(fixture.state.clone()))
            .await
            .expect("post batch");
        let status = response.status();
        (status, body_json(response).await)
    }

    async fn get_batch(fixture: &Fixture, job_id: &str) -> (StatusCode, Value) {
        let response = get_lc_capability_revalidate_all_job(
            State(fixture.state.clone()),
            Path(job_id.to_string()),
        )
        .await
        .expect("get batch");
        let status = response.status();
        (status, body_json(response).await)
    }

    /// 版本探测恒失败的注入服务（四家全部 skipped + cli_not_installed）。
    fn cli_missing_service(root: &std::path::Path) -> ProviderCapabilityRevalidateService {
        ProviderCapabilityRevalidateService::new(crate::product::app_paths::ProductAppPaths::new(
            root.join(".aria"),
        ))
        .with_version_source(Arc::new(|program: &str| {
            Err(ProviderBoundaryError::Unsupported(format!(
                "boundary_unavailable: cannot execute `{program} --version`: \
                 No such file or directory (os error 2)"
            )))
        }))
        .with_probe_runner(Arc::new(|_invocation: ProbeInvocation| {
            Box::pin(async {
                panic!("版本探测已失败，不得再跑 action 探针");
            })
        }))
    }

    async fn wait_terminal(fixture: &Fixture, job_id: &str) -> Value {
        for _ in 0..400 {
            let (status, body) = get_batch(fixture, job_id).await;
            assert_eq!(status, StatusCode::OK);
            if body["status"] == "completed" {
                return body;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("作业未在 4s 内到终态");
    }

    #[tokio::test]
    async fn post_returns_job_id_immediately() {
        let gate = Arc::new(tokio::sync::Semaphore::new(0));
        let gate_for_runner = gate.clone();
        let root_dir = tempfile::tempdir().expect("probe root");
        let service = ProviderCapabilityRevalidateService::new(
            crate::product::app_paths::ProductAppPaths::new(root_dir.path().join(".aria")),
        )
        .with_version_source(Arc::new(|_program: &str| Ok("1.0.0".to_string())))
        .with_probe_runner(Arc::new(move |_invocation: ProbeInvocation| {
            let gate = gate_for_runner.clone();
            Box::pin(async move {
                let _permit = gate.acquire().await.expect("gate never closed");
                Err(ProviderBoundaryError::ProbeFailed(
                    "probe blocked by test gate".to_string(),
                ))
            })
        }));
        let fixture = fixture(Some(service));

        let started = Instant::now();
        let (status, body) = post_batch(&fixture).await;
        assert_eq!(status, StatusCode::ACCEPTED, "异步作业创建");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "POST 不得等待探针（探针被闸门挡住，等它就会挂住），实际 {:?}",
            started.elapsed()
        );
        assert_eq!(body["status"], "running");
        let job_id = body["job_id"].as_str().expect("job_id").to_string();
        assert!(job_id.starts_with("capability_batch_"));

        // 探针仍被闸门挡住 ⇒ 逐项仍在核验中。
        let (status, observed) = get_batch(&fixture, &job_id).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(observed["status"], "running");
        assert_eq!(
            observed["results"].as_array().expect("results").len(),
            4,
            "四家逐项进度"
        );
        for item in observed["results"].as_array().expect("results") {
            assert_eq!(item["status"], "running");
        }

        gate.add_permits(64);
        wait_terminal(&fixture, &job_id).await;
    }

    #[tokio::test]
    async fn get_returns_per_provider_results() {
        let root_dir = tempfile::tempdir().expect("probe root");
        let service = cli_missing_service(root_dir.path());
        let fixture = fixture(Some(service));

        let (status, body) = post_batch(&fixture).await;
        assert_eq!(status, StatusCode::ACCEPTED);
        let job_id = body["job_id"].as_str().expect("job_id").to_string();

        let job = wait_terminal(&fixture, &job_id).await;
        assert_eq!(job["status"], "completed");
        let results = job["results"].as_array().expect("results");
        assert_eq!(results.len(), 4);
        let providers: Vec<&str> = results
            .iter()
            .map(|item| item["provider_type"].as_str().expect("provider_type"))
            .collect();
        assert_eq!(
            providers,
            vec!["claude_code", "codex", "pi", "kimi_code"],
            "四家逐项结果（顺序固定）"
        );
        for item in results {
            assert_eq!(item["status"], "skipped", "CLI 缺失不冒充失败/已确认");
            assert_eq!(item["reason"], "cli_not_installed");
        }

        // 未知 job_id：明确 404 稳定码。
        let (status, error) = get_batch(&fixture, "capability_batch_missing").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(error["code"], "capability_revalidate_batch_job_not_found");
    }

    /// 机器级（REQ-PCR-03）：批量作业**不绑定**项目 / 逻辑代码库 ⇒ 响应体
    /// 不带归属字段，且一个项目都没有也能发起与读回。
    #[tokio::test]
    async fn batch_job_carries_no_project_or_lc_ownership() {
        let root_dir = tempfile::tempdir().expect("probe root");
        let service = cli_missing_service(root_dir.path());
        let fixture = fixture(Some(service));

        let (status, body) = post_batch(&fixture).await;
        assert_eq!(status, StatusCode::ACCEPTED, "body: {body}");
        let job_id = body["job_id"].as_str().expect("job_id").to_string();

        let (status, job) = get_batch(&fixture, &job_id).await;
        assert_eq!(status, StatusCode::OK, "body: {job}");
        assert_eq!(job["job_id"], json!(job_id), "body: {job}");
        assert!(job.get("project_id").is_none(), "作业不得绑定项目: {job}");
        assert!(
            job.get("logical_codebase_id").is_none(),
            "作业不得绑定逻辑代码库: {job}"
        );
        assert_eq!(
            job["results"].as_array().expect("results").len(),
            4,
            "四家逐项结果: {job}"
        );
    }
}
