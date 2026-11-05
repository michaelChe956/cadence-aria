//! 票3「一键核验全部」：LC provider capability 批量核验作业。
//!
//! 职责（REQ-PCR-03/04/05/06）：
//! - `start` 只由用户显式点击发起（MUST NOT 因聚合初始化失败自动触发），
//!   立即返回 `job_id` + `running`，MUST NOT 在响应内等待探针；
//! - 四家（claude_code / codex / pi / kimi_code）**并发**各跑一次既有单家
//!   `ProviderCapabilityRevalidateService::revalidate`（单家语义零改动：仍按
//!   「全行已确认 → already_confirmed 秒回」「CLI 缺失在版本探测阶段失败」）；
//! - 逐项状态 `running` → `confirmed` / `failed` / `skipped`（带稳定原因），
//!   终态仍未完成项标 `timeout`，绝不谎报已确认；
//! - 单家失败不阻断其它家，整批不因一家失败而失败；
//! - 作业状态 durable（落 LC 子树 `capability-revalidate-batch/`），刷新后
//!   可凭 `job_id` 继续轮询，终态仍可读。
//!
//! 与块2 安装作业（`src/product/provider_installation/**`）的边界：本模块
//! 自建作业存储与编排，块2 文件零改动；两者只在契约层同构（创建 → 轮询 →
//! 终态）。安装 / 更新入口一律由块2 前端组件承担（H3）。

use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::Duration;

use futures_util::future::join_all;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::{ProductStoreError, read_json, validate_relative_id, write_json};
use crate::product::logical_codebase::provider_capability_revalidate::{
    ProviderCapabilityRevalidateService, RevalidateError, RevalidateOutcome, revalidate_action_text,
};
use crate::product::models::ProviderName;

#[cfg(test)]
#[path = "capability_revalidate_batch_tests.inc.rs"]
mod tests;

/// 作业记录持久化 schema 版本（冻结 DTO/持久化面）。
pub const BATCH_JOB_SCHEMA_VERSION: u32 = 1;

/// 批量核验覆盖的四家真实 provider（Fake 无探针通道，不参与）。
pub const BATCH_PROVIDER_TYPES: [ProviderName; 4] = [
    ProviderName::ClaudeCode,
    ProviderName::Codex,
    ProviderName::Pi,
    ProviderName::KimiCode,
];

/// 作业默认总预算（真机探针分钟级；四家并发，不逐家累加）。
pub const BATCH_JOB_DEADLINE: Duration = Duration::from_secs(1800);

/// 稳定原因码：整行已在本版本确认，秒回不重跑。
pub const BATCH_REASON_ALREADY_CONFIRMED: &str = "already_confirmed";
/// 稳定原因码：本次真实探针完成并原子导入。
pub const BATCH_REASON_REVALIDATED: &str = "revalidated";
/// 稳定原因码：CLI 不可用（未安装 / 无法执行 / 版本探测失败）⇒ 引导【安装】。
pub const BATCH_REASON_CLI_NOT_INSTALLED: &str = "cli_not_installed";
/// 稳定原因码：provider 无真实探针通道（不冒充探测）。
pub const BATCH_REASON_PROBE_UNSUPPORTED: &str = "probe_unsupported";
/// 稳定原因码：真实探针失败（如实上报，不伪造 Confirmed）。
pub const BATCH_REASON_PROBE_FAILED: &str = "probe_failed";
/// 稳定原因码：底层 durable 存储错误。
pub const BATCH_REASON_STORE_ERROR: &str = "store_error";
/// 稳定原因码：作业终态时该项仍未完成（总预算耗尽）。
pub const BATCH_REASON_DEADLINE_EXCEEDED: &str = "job_deadline_exceeded";

/// 作业总体状态：`running` → `completed`（终态，仍可读）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchJobStatus {
    Running,
    Completed,
}

impl BatchJobStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Completed => "completed",
        }
    }
}

/// 逐项状态：`running` → `confirmed` / `failed` / `skipped`，终态未完成 →
/// `timeout`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchItemStatus {
    Running,
    Confirmed,
    Failed,
    Skipped,
    Timeout,
}

impl BatchItemStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Running => "running",
            Self::Confirmed => "confirmed",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Timeout => "timeout",
        }
    }

    /// 终态判定（`running` 之外皆终态）。
    pub fn is_terminal(self) -> bool {
        !matches!(self, Self::Running)
    }
}

/// 逐家核验结果（逐项进度载体）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchItemResult {
    /// provider 稳定名（`claude_code` / `codex` / `pi` / `kimi_code`，与
    /// capability DTO 同口径）。
    pub provider_type: ProviderName,
    pub status: BatchItemStatus,
    /// 稳定原因码（`confirmed`：`already_confirmed`/`revalidated`；
    /// `skipped`：`cli_not_installed`/`probe_unsupported`；`failed`：
    /// `probe_failed`/`store_error`；`timeout`：`job_deadline_exceeded`）。
    pub reason: Option<String>,
    /// 人类可读细节（失败原始错误 / 导入的 action 列表）；不参与判定。
    pub detail: Option<String>,
    /// 探针判定的 CLI 版本（成功路径才有）。
    pub version: Option<String>,
    pub started_at: Option<String>,
    pub finished_at: Option<String>,
}

impl BatchItemResult {
    fn running(provider_type: ProviderName, started_at: String) -> Self {
        Self {
            provider_type,
            status: BatchItemStatus::Running,
            reason: None,
            detail: None,
            version: None,
            started_at: Some(started_at),
            finished_at: None,
        }
    }

    fn finished(
        provider_type: ProviderName,
        status: BatchItemStatus,
        reason: Option<&str>,
        detail: Option<String>,
        version: Option<String>,
        started_at: Option<String>,
        finished_at: String,
    ) -> Self {
        Self {
            provider_type,
            status,
            reason: reason.map(str::to_string),
            detail,
            version,
            started_at,
            finished_at: Some(finished_at),
        }
    }

    pub fn is_terminal(&self) -> bool {
        self.status.is_terminal()
    }
}

/// 批量核验作业（durable 记录；刷新后按 `job_id` 继续轮询）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CapabilityRevalidateBatchJob {
    pub schema_version: u32,
    pub job_id: String,
    pub project_id: String,
    pub logical_codebase_id: String,
    pub status: BatchJobStatus,
    pub created_at: String,
    pub updated_at: String,
    pub finished_at: Option<String>,
    /// 逐家逐项结果（顺序固定为 [`BATCH_PROVIDER_TYPES`]）。
    pub results: Vec<BatchItemResult>,
}

impl CapabilityRevalidateBatchJob {
    /// 终态判定（全部逐项终态 ⇒ 作业 `completed`）。
    pub fn is_terminal(&self) -> bool {
        self.status == BatchJobStatus::Completed
    }

    pub fn item(&self, provider_type: ProviderName) -> Option<&BatchItemResult> {
        self.results
            .iter()
            .find(|item| item.provider_type == provider_type)
    }
}

/// 逐家核验调用 seam（生产 = 既有单家 `revalidate`；测试注入可控实现）。
pub type BatchRevalidateFuture =
    Pin<Box<dyn Future<Output = Result<RevalidateOutcome, RevalidateError>> + Send>>;
pub type BatchRevalidateCall =
    Arc<dyn Fn(String, String, ProviderName) -> BatchRevalidateFuture + Send + Sync>;

/// 把既有单家服务包装成批量核验调用（只做转发，不改单家语义）。
pub fn revalidate_call_for(service: ProviderCapabilityRevalidateService) -> BatchRevalidateCall {
    Arc::new(
        move |_project_id: String, _logical_codebase_id: String, provider: ProviderName| {
            let service = service.clone();
            // 机器级重验证（REQ-PCR-07）：单家核验不再按项目/逻辑代码库定位，
            // seam 的两个作用域参数仅为兼容既有批量作业签名（Task 5 收敛）。
            Box::pin(async move { service.revalidate(provider).await })
        },
    )
}

/// 批量核验作业存储目录（LC 子树；durable）。
pub fn batch_jobs_dir(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> Result<PathBuf, ProductStoreError> {
    validate_relative_id(project_id)?;
    validate_relative_id(logical_codebase_id)?;
    Ok(
        super::store::lc_scope_root(paths, project_id, &Some(logical_codebase_id.to_string()))?
            .join("capability-revalidate-batch"),
    )
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

/// 批量核验编排服务。
///
/// 作业只由 [`CapabilityRevalidateBatchService::start`]（用户显式点击）创建；
/// 构造本服务本身不产生任何作业、不跑任何探针。
#[derive(Clone)]
pub struct CapabilityRevalidateBatchService {
    paths: ProductAppPaths,
    revalidate: BatchRevalidateCall,
    job_deadline: Duration,
    write_lock: Arc<Mutex<()>>,
}

impl CapabilityRevalidateBatchService {
    pub fn new(paths: ProductAppPaths) -> Self {
        let revalidate =
            revalidate_call_for(ProviderCapabilityRevalidateService::new(paths.clone()));
        Self {
            paths,
            revalidate,
            job_deadline: BATCH_JOB_DEADLINE,
            write_lock: Arc::new(Mutex::new(())),
        }
    }

    /// 测试 seam：替换逐家核验调用。
    pub fn with_revalidate_call(mut self, call: BatchRevalidateCall) -> Self {
        self.revalidate = call;
        self
    }

    /// 用既有单家服务承载逐家核验（handler 装配 / it_web 注入同源）。
    pub fn with_revalidate_service(self, service: ProviderCapabilityRevalidateService) -> Self {
        self.with_revalidate_call(revalidate_call_for(service))
    }

    /// 测试 seam：作业总预算（生产 [`BATCH_JOB_DEADLINE`]）。
    pub fn with_job_deadline(mut self, deadline: Duration) -> Self {
        self.job_deadline = deadline;
        self
    }

    /// 显式发起批量核验：立即返回 `running` 作业（零探针等待），四家并发在
    /// 后台推进。
    ///
    /// # Panics
    /// 需在 tokio runtime 内调用（后台作业以 `tokio::spawn` 推进）。
    pub fn start(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<CapabilityRevalidateBatchJob, ProductStoreError> {
        validate_relative_id(project_id)?;
        validate_relative_id(logical_codebase_id)?;
        let now = timestamp();
        let job = CapabilityRevalidateBatchJob {
            schema_version: BATCH_JOB_SCHEMA_VERSION,
            job_id: format!("capability_batch_{}", uuid::Uuid::new_v4().simple()),
            project_id: project_id.to_string(),
            logical_codebase_id: logical_codebase_id.to_string(),
            status: BatchJobStatus::Running,
            created_at: now.clone(),
            updated_at: now.clone(),
            finished_at: None,
            results: BATCH_PROVIDER_TYPES
                .iter()
                .map(|provider| BatchItemResult::running(provider.clone(), now.clone()))
                .collect(),
        };
        self.write_job(&job)?;
        let service = self.clone();
        let spawned = job.clone();
        tokio::spawn(async move { service.run(spawned).await });
        Ok(job)
    }

    /// 按 `job_id` 读取 durable 作业状态（终态仍可读）。
    pub fn get(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        job_id: &str,
    ) -> Result<CapabilityRevalidateBatchJob, ProductStoreError> {
        self.read_job(project_id, logical_codebase_id, job_id)?
            .ok_or_else(|| ProductStoreError::NotFound {
                kind: "capability_revalidate_batch_job",
                id: job_id.to_string(),
            })
    }

    /// 四家全并发推进；总预算耗尽则把未完成项如实标超时。
    async fn run(&self, job: CapabilityRevalidateBatchJob) {
        let project_id = job.project_id.clone();
        let logical_codebase_id = job.logical_codebase_id.clone();
        let job_id = job.job_id.clone();
        let futures = BATCH_PROVIDER_TYPES.iter().cloned().map(|provider| {
            let service = self.clone();
            let project_id = project_id.clone();
            let logical_codebase_id = logical_codebase_id.clone();
            let job_id = job_id.clone();
            async move {
                service
                    .run_provider(&job_id, &project_id, &logical_codebase_id, provider)
                    .await;
            }
        });
        // join_all 同时轮询四家（真并发，不串行 await）；超时后丢弃未完成
        // 的探针 future（显式取消），并把它们标成 timeout。
        let joined = tokio::time::timeout(self.job_deadline, join_all(futures)).await;
        if joined.is_err() {
            self.mark_unfinished_timeout(&project_id, &logical_codebase_id, &job_id)
                .await;
        }
    }

    async fn run_provider(
        &self,
        job_id: &str,
        project_id: &str,
        logical_codebase_id: &str,
        provider: ProviderName,
    ) {
        let outcome = (self.revalidate)(
            project_id.to_string(),
            logical_codebase_id.to_string(),
            provider.clone(),
        )
        .await;
        let item = classify_outcome(provider, outcome, timestamp());
        self.apply_item(project_id, logical_codebase_id, job_id, item)
            .await;
    }

    /// 单家结果落盘（逐项进度可见；全部终态 ⇒ 作业 completed）。
    async fn apply_item(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        job_id: &str,
        item: BatchItemResult,
    ) {
        let _guard = self.write_lock.lock().await;
        let Ok(Some(mut job)) = self.read_job(project_id, logical_codebase_id, job_id) else {
            return;
        };
        for slot in job
            .results
            .iter_mut()
            .filter(|slot| slot.provider_type == item.provider_type)
        {
            *slot = item.clone();
        }
        job.updated_at = timestamp();
        if job.results.iter().all(BatchItemResult::is_terminal) {
            job.status = BatchJobStatus::Completed;
            job.finished_at.get_or_insert(job.updated_at.clone());
        }
        if let Err(error) = self.write_job(&job) {
            tracing::warn!(%error, job_id, "capability revalidate batch item persist failed");
        }
    }

    /// 终态收口：未完成项标 timeout（不谎报已确认，不覆盖已得结果）。
    async fn mark_unfinished_timeout(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        job_id: &str,
    ) {
        let _guard = self.write_lock.lock().await;
        let Ok(Some(mut job)) = self.read_job(project_id, logical_codebase_id, job_id) else {
            return;
        };
        let now = timestamp();
        for item in job.results.iter_mut().filter(|item| !item.is_terminal()) {
            let provider_type = item.provider_type.clone();
            let started_at = item.started_at.clone();
            *item = BatchItemResult::finished(
                provider_type,
                BatchItemStatus::Timeout,
                Some(BATCH_REASON_DEADLINE_EXCEEDED),
                Some("job deadline exceeded before this provider finished".to_string()),
                None,
                started_at,
                now.clone(),
            );
        }
        job.status = BatchJobStatus::Completed;
        job.updated_at = now.clone();
        job.finished_at.get_or_insert(now);
        if let Err(error) = self.write_job(&job) {
            tracing::warn!(%error, job_id, "capability revalidate batch timeout persist failed");
        }
    }

    fn job_path(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        job_id: &str,
    ) -> Result<PathBuf, ProductStoreError> {
        validate_relative_id(job_id)?;
        Ok(
            batch_jobs_dir(&self.paths, project_id, logical_codebase_id)?
                .join(format!("{job_id}.json")),
        )
    }

    fn write_job(&self, job: &CapabilityRevalidateBatchJob) -> Result<(), ProductStoreError> {
        write_json(
            &self.job_path(&job.project_id, &job.logical_codebase_id, &job.job_id)?,
            job,
        )
    }

    /// 读取归属该 project/LC 的作业；不存在或归属不符 → `None`。
    fn read_job(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        job_id: &str,
    ) -> Result<Option<CapabilityRevalidateBatchJob>, ProductStoreError> {
        let path = self.job_path(project_id, logical_codebase_id, job_id)?;
        if !path.exists() {
            return Ok(None);
        }
        let job: CapabilityRevalidateBatchJob = read_json(&path)?;
        if job.project_id != project_id || job.logical_codebase_id != logical_codebase_id {
            return Ok(None);
        }
        Ok(Some(job))
    }
}

/// 单家结果 → 逐项状态（稳定原因码；失败如实，不伪造 Confirmed）。
fn classify_outcome(
    provider: ProviderName,
    outcome: Result<RevalidateOutcome, RevalidateError>,
    finished_at: String,
) -> BatchItemResult {
    let started_at = None;
    match outcome {
        Ok(RevalidateOutcome::AlreadyConfirmed { version }) => BatchItemResult::finished(
            provider,
            BatchItemStatus::Confirmed,
            Some(BATCH_REASON_ALREADY_CONFIRMED),
            None,
            Some(version),
            started_at,
            finished_at,
        ),
        Ok(RevalidateOutcome::Revalidated {
            version,
            imported_actions,
        }) => {
            let detail = imported_actions
                .iter()
                .map(|action| revalidate_action_text(*action))
                .collect::<Vec<_>>()
                .join(",");
            BatchItemResult::finished(
                provider,
                BatchItemStatus::Confirmed,
                Some(BATCH_REASON_REVALIDATED),
                Some(detail),
                Some(version),
                started_at,
                finished_at,
            )
        }
        // 无真实探针通道：不冒充探测，跳过（四家在生产不会走到这里）。
        Err(RevalidateError::Unsupported { provider: name }) => BatchItemResult::finished(
            provider,
            BatchItemStatus::Skipped,
            Some(BATCH_REASON_PROBE_UNSUPPORTED),
            Some(format!("provider {name} has no real probe channel")),
            None,
            started_at,
            finished_at,
        ),
        // 版本探测阶段失败 = CLI 不可用（未安装 / 无法执行）⇒ 跳过并引导安装；
        // 不挂起、不整体失败。
        Err(RevalidateError::ProbeFailed {
            action: None,
            detail,
        }) => BatchItemResult::finished(
            provider,
            BatchItemStatus::Skipped,
            Some(BATCH_REASON_CLI_NOT_INSTALLED),
            Some(detail),
            None,
            started_at,
            finished_at,
        ),
        Err(RevalidateError::ProbeFailed {
            action: Some(action),
            detail,
        }) => BatchItemResult::finished(
            provider,
            BatchItemStatus::Failed,
            Some(BATCH_REASON_PROBE_FAILED),
            Some(format!("{}: {detail}", revalidate_action_text(action))),
            None,
            started_at,
            finished_at,
        ),
        Err(RevalidateError::Store(error)) => BatchItemResult::finished(
            provider,
            BatchItemStatus::Failed,
            Some(BATCH_REASON_STORE_ERROR),
            Some(error.to_string()),
            None,
            started_at,
            finished_at,
        ),
    }
}
