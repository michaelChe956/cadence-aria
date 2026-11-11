//! 知识库 bootstrap / update 的执行层：provider turn、写入面审计、trust 门
//! 与 per-LC 单飞（LC 知识库模块实施计划 Task 6）。
//!
//! - turn 走 Normal 相位 + `SessionPolicyAction::CodingTargetWrite`，target =
//!   canonical 聚合根、cwd = 聚合根、可写根 = 聚合根；不引入
//!   `BootstrapPhaseCredential`（聚合初始化的凭据链保持冻结）。
//! - 单飞：同一 LC 的 bootstrap 与 update 互斥，任一 operation 在跑时拒绝新启。
//! - 写入面审计：turn 前后对聚合根做快照差分，允许面之外的新增/修改/删除
//!   一律使 operation 失败并记 `knowledge_base_write_scope_violation`。后快照
//!   在 provider 停止后无条件执行（成功/失败/取消都审计），越界作为附加事实
//!   并入失败/取消记录，不掩盖原始原因；取消时若无法确认 provider 已停止，
//!   审计按「不确定」fail-closed 留痕，不得静默当作干净取消。平台聚合索引
//!   与 turn 运行期互斥（`coordinator_audit.inc.rs`），索引产物
//!   路径不再豁免；仅 `**/.git/index` 与根层 `.aria/runtime/**` 不计入。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::diagnostic_redaction::sanitize_turn_output;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, ProviderSession, ProviderStatus,
    StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::locking::with_exact_exclusive_lock;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::aggregate_index::AggregateIndexStore;
use crate::product::logical_codebase::aggregate_initialization::{
    AggregateInitializationErrorRecord, AggregateProviderSelection,
};
use crate::product::logical_codebase::provider_admission_preflight::ProviderAdmissionPhase;
use crate::product::logical_codebase::provider_trust::{
    ProviderTrustPrecondition, ProviderTrustPreparationResult, ProviderTrustScenario,
    ProviderTrustWaiting, provider_wire_label,
};
use crate::product::logical_codebase::{
    LogicalCodebaseProviderAdmissionPreflight, LogicalCodebaseProviderGateway, PolicyTarget,
    ProviderRef, ProviderRefType, SessionLaunchRequest, SessionPolicyAction,
};
use crate::product::models::ProviderName;

use super::command::{bootstrap_command, update_command, user_input_command};
use super::operation::{
    KnowledgeBaseOperation, KnowledgeBaseOperationInput, KnowledgeBaseOperationKind,
    KnowledgeBaseOperationStatus, KnowledgeBaseStepKind, KnowledgeBaseStepRecord,
    KnowledgeBaseStepStatus, derive_operation_id,
};
use super::preflight::KnowledgeBasePreflightFailure;
use super::store::KnowledgeBaseOperationStore;

/// 与聚合初始化 root recipe 同款的托管配置 artifact 引用（经 gateway
/// envelope 的 config_digest 复验）。
const KNOWLEDGE_BASE_CONFIG_ARTIFACT_REF: &str = "sha256:aggregate-initialization-managed-config";
/// capability snapshot 引用，与自举 capability 记录一致。
const DEFAULT_CAPABILITY_SNAPSHOT_REF: &str = "cap_managed_snapshot";
/// 单个 provider turn 输出摘要的累积上限（字节）。
const KNOWLEDGE_BASE_TURN_OUTPUT_LIMIT: usize = 4096;
/// 单个 provider turn 的超时预算（与既有 root recipe 驱动一致）。
const DEFAULT_TURN_TIMEOUT_SECS: u64 = 1800;
/// Abort 后等待 provider 事件流关闭（确认停止）的默认宽限预算（秒）。
const DEFAULT_CANCEL_STOP_GRACE_SECS: u64 = 10;
/// 失败记录里最多列出的越界路径条数。
const VIOLATION_SUMMARY_LIMIT: usize = 20;

// 失败/取消记录映射与平台索引隔离见 `coordinator_audit.inc.rs`（1200 行
// 门禁：非测试代码按仓内 `*.inc.rs` include 惯例拆分）。
const REASON_CANCELLED: &str = "knowledge_base_cancelled";
/// 取消时无法确认 provider 已停止：审计结果不确定（下界），fail-closed 留痕。
const REASON_CANCEL_AUDIT_INDETERMINATE: &str = "knowledge_base_cancel_audit_indeterminate";

/// 时钟：返回 RFC3339 时间戳。
pub type Clock = dyn Fn() -> String + Send + Sync;

/// 知识库执行层错误。
#[derive(Debug, thiserror::Error)]
pub enum KnowledgeBaseError {
    #[error(transparent)]
    Store(#[from] ProductStoreError),
    /// 前置检查失败（单飞冲突复用 `OtherKnowledgeBaseRunning`）。
    #[error("knowledge_base_preflight: {0:?}")]
    Preflight(KnowledgeBasePreflightFailure),
    /// trust 门未就绪：可重试等待面，provider 零启动。
    #[error("aggregate_initialization_trust_waiting: {}", .0.message)]
    TrustWaiting(Box<ProviderTrustWaiting>),
    #[error("knowledge_base_invalid_input: {0}")]
    InvalidInput(String),
    #[error("knowledge_base_operation_not_running: {operation_id} ({status})")]
    NotRunning {
        operation_id: String,
        status: String,
    },
    #[error("knowledge_base_cancelled")]
    Cancelled,
    #[error("{reason_code}: {summary}")]
    TurnFailed {
        reason_code: String,
        summary: String,
        retryable: bool,
    },
    #[error("knowledge_base_write_scope_violation: {}", .paths.join(", "))]
    WriteScopeViolation { paths: Vec<String> },
    #[error("knowledge_base_io: {0}")]
    Io(String),
}

/// 单个 provider turn 的失败面。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KnowledgeBaseTurnError {
    Cancelled,
    /// 取消返回，但 runner 无法确认 provider 已停止（Abort 后事件流在宽限期内
    /// 未关闭）：后快照只能当作审计下界，协调器按「审计不确定」fail-closed
    /// 留痕。对调用方仍表现为 [`KnowledgeBaseError::Cancelled`]。
    CancelledStopUnconfirmed,
    Failed {
        reason_code: String,
        summary: String,
        retryable: bool,
    },
}

impl From<KnowledgeBaseTurnError> for KnowledgeBaseError {
    fn from(error: KnowledgeBaseTurnError) -> Self {
        match error {
            KnowledgeBaseTurnError::Cancelled => Self::Cancelled,
            KnowledgeBaseTurnError::CancelledStopUnconfirmed => Self::Cancelled,
            KnowledgeBaseTurnError::Failed {
                reason_code,
                summary,
                retryable,
            } => Self::TurnFailed {
                reason_code,
                summary,
                retryable,
            },
        }
    }
}

/// 一次 provider turn 的启动请求。
pub struct KnowledgeBaseTurnRequest<'a> {
    pub project_id: &'a str,
    pub operation_id: &'a str,
    pub lc_id: &'a str,
    pub provider: &'a ProviderName,
    /// provider 前缀 + 固定命令名（update 追加变更包目录）。
    pub command: &'a str,
    pub canonical_root: &'a Path,
    pub cancellation: CancellationToken,
}

/// provider turn 执行抽象：生产实现经 gateway 启动，测试注入假 runner。
#[async_trait]
pub trait KnowledgeBaseTurnRunner: Send + Sync + 'static {
    /// 执行一个 turn，成功返回有界输出摘要。
    async fn run_turn(
        &self,
        request: KnowledgeBaseTurnRequest<'_>,
    ) -> Result<String, KnowledgeBaseTurnError>;
}

/// 知识库 operation 协调器。
#[derive(Clone)]
pub struct KnowledgeBaseCoordinator {
    paths: ProductAppPaths,
    lc_id: Option<String>,
    runner: Arc<dyn KnowledgeBaseTurnRunner>,
    trust: Arc<dyn ProviderTrustPrecondition>,
    clock: Arc<Clock>,
}

impl KnowledgeBaseCoordinator {
    pub fn new(
        paths: ProductAppPaths,
        runner: Arc<dyn KnowledgeBaseTurnRunner>,
        trust: Arc<dyn ProviderTrustPrecondition>,
        clock: Arc<Clock>,
    ) -> Self {
        Self {
            paths,
            lc_id: None,
            runner,
            trust,
            clock,
        }
    }

    /// 生产装配：turn 经 `LogicalCodebaseProviderGateway` 启动。
    pub fn with_gateway(
        paths: ProductAppPaths,
        gateway: Arc<LogicalCodebaseProviderGateway>,
        trust: Arc<dyn ProviderTrustPrecondition>,
        clock: Arc<Clock>,
    ) -> Self {
        let runner = Arc::new(GatewayKnowledgeBaseTurnRunner::new(gateway, paths.clone()));
        Self::new(paths, runner, trust, clock)
    }

    /// 绑定到一个 LC；之后的 store 读写与 trust 登记都限定在该 LC scope。
    pub fn for_lc(&self, lc_id: impl Into<String>) -> Self {
        Self {
            lc_id: Some(lc_id.into()),
            ..self.clone()
        }
    }

    fn lc_id(&self) -> Result<&str, KnowledgeBaseError> {
        self.lc_id.as_deref().ok_or_else(|| {
            KnowledgeBaseError::InvalidInput(
                "coordinator is not bound to a logical codebase".into(),
            )
        })
    }

    fn store(&self) -> Result<KnowledgeBaseOperationStore, KnowledgeBaseError> {
        Ok(KnowledgeBaseOperationStore::for_lc(
            self.paths.clone(),
            self.lc_id()?.to_string(),
        ))
    }

    fn now(&self) -> String {
        (self.clock)()
    }

    /// 单飞：同 LC 已有 running 的知识库 operation 时返回其 id（bootstrap
    /// 与 update 互斥）。
    pub fn other_running(&self, project_id: &str) -> Result<Option<String>, KnowledgeBaseError> {
        Ok(self
            .store()?
            .list_running(project_id)?
            .into_iter()
            .next()
            .map(|operation| operation.operation_id))
    }

    /// trust 硬前置门（沿用聚合初始化的 `Recipe` 场景）：HTTP 层在创建
    /// operation 之前调用；`execute` 在启动 provider 前再复验一次。
    pub async fn ensure_trust_ready(
        &self,
        project_id: &str,
        operation_id: &str,
        canonical_root: &Path,
        provider: &ProviderName,
    ) -> Result<(), KnowledgeBaseError> {
        let lc_id = self.lc_id()?.to_string();
        let canonical_root =
            std::fs::canonicalize(canonical_root).unwrap_or_else(|_| canonical_root.to_path_buf());
        let result = self
            .trust
            .clone()
            .ensure_ready_cancellable(
                project_id.to_string(),
                operation_id.to_string(),
                lc_id,
                canonical_root,
                vec![provider.clone()],
                ProviderTrustScenario::Recipe,
            )
            .await;
        match result {
            ProviderTrustPreparationResult::Ready { .. } => Ok(()),
            ProviderTrustPreparationResult::Waiting { waiting } => {
                Err(KnowledgeBaseError::TrustWaiting(Box::new(waiting)))
            }
        }
    }

    /// 创建 Running 的 operation（幂等；同 id 重放返回既有记录）。
    ///
    /// 新建前先做单飞检查：同 LC 已有其它 running operation ⇒
    /// `Preflight(OtherKnowledgeBaseRunning)`。
    pub async fn begin(
        &self,
        operation_id: String,
        project_id: &str,
        lc_id: &str,
        kind: KnowledgeBaseOperationKind,
        input: KnowledgeBaseOperationInput,
        selection: AggregateProviderSelection,
    ) -> Result<KnowledgeBaseOperation, KnowledgeBaseError> {
        if self.lc_id()? != lc_id {
            return Err(KnowledgeBaseError::InvalidInput(
                "logical codebase differs from the bound coordinator".into(),
            ));
        }
        if operation_id != derive_operation_id(project_id, lc_id, &input.idempotency_key) {
            return Err(KnowledgeBaseError::InvalidInput(
                "operation_id does not match the derived id".into(),
            ));
        }
        // 先校验输入：非法输入不得落盘、不得占用单飞槽位。
        resolve_turn(kind, &input)?;

        let store = self.store()?;
        // 「幂等重放 → 单飞检查 → 创建」放进 LC 作用域的跨进程排他锁：
        // durable `list_running` 只是观察，不是占位，进程内互斥锁拦不住共享
        // 同一 store 的另一进程。
        let outcome = store.with_begin_lock(project_id, || {
            match store.get(project_id, &operation_id) {
                Ok(existing) => return Ok(Ok(existing)),
                Err(ProductStoreError::NotFound { .. }) => {}
                Err(error) => return Err(error),
            }
            if let Some(running) = store.list_running(project_id)?.into_iter().next() {
                return Ok(Err(running.operation_id));
            }
            let now = self.now();
            let operation = KnowledgeBaseOperation {
                operation_id,
                project_id: project_id.to_string(),
                logical_codebase_id: lc_id.to_string(),
                kind,
                input,
                status: KnowledgeBaseOperationStatus::Running,
                steps: vec![KnowledgeBaseStepRecord {
                    step_id: step_kind(kind),
                    status: KnowledgeBaseStepStatus::Running,
                    started_at: Some(now.clone()),
                    completed_at: None,
                    output_summary: None,
                }],
                provider_selection: Some(selection),
                error: None,
                created_at: now.clone(),
                started_at: Some(now),
                completed_at: None,
            };
            store.create(operation).map(Ok)
        })?;
        outcome.map_err(|running| {
            KnowledgeBaseError::Preflight(
                KnowledgeBasePreflightFailure::OtherKnowledgeBaseRunning {
                    operation_id: running,
                },
            )
        })
    }

    /// 执行一个已 `begin` 的 operation：trust 复验 → turn 前快照 → provider
    /// turn → turn 后快照差分审计 → 收口。任何失败都把 operation 推进到终态，
    /// 单飞槽位不会泄漏。
    pub async fn execute(
        &self,
        project_id: &str,
        operation_id: &str,
        canonical_root: &Path,
        cancellation: CancellationToken,
    ) -> Result<(), KnowledgeBaseError> {
        let store = self.store()?;
        let operation = store.get(project_id, operation_id)?;
        if operation.status != KnowledgeBaseOperationStatus::Running {
            return Err(KnowledgeBaseError::NotRunning {
                operation_id: operation_id.to_string(),
                status: format!("{:?}", operation.status),
            });
        }
        let step = step_kind(operation.kind);
        let provider = operation.input.provider.as_str();
        match self.drive(&operation, canonical_root, cancellation).await {
            // 成功：步骤、输出摘要与 operation 在同一次落盘里原子收口。
            Ok(summary) => store
                .complete_step(
                    project_id,
                    operation_id,
                    step,
                    Some(sanitize_turn_output(&summary)),
                    self.now(),
                )
                .map_err(|error| self.settle_store_conflict(project_id, operation_id, error)),
            Err(DriveFailure {
                error: KnowledgeBaseError::Cancelled,
                violations,
                audit_indeterminate,
            }) => Err(self.settle_cancelled(
                project_id,
                operation_id,
                provider,
                operation.kind,
                &violations,
                audit_indeterminate,
            )),
            Err(DriveFailure {
                error, violations, ..
            }) => {
                let record = error_record(&error, &violations, provider, operation.kind);
                match store.finish_failed(project_id, operation_id, Some(step), record, self.now())
                {
                    Ok(()) => Err(error),
                    // 「取消 API 已落盘 + runner 返回普通失败」的交叉情形：已算出
                    // 的审计事实不得丢弃，改为附加到 Cancelled 记录上。
                    Err(store_error) => Err(self.settle_failed_after_cancel_won(
                        &operation,
                        &error,
                        &violations,
                        store_error,
                    )),
                }
            }
        }
    }

    /// 取消一个 running operation（置 Cancelled；provider 中止由调用方经
    /// run registry 的取消令牌驱动）。
    pub fn cancel(&self, project_id: &str, operation_id: &str) -> Result<(), KnowledgeBaseError> {
        Ok(self.store()?.cancel(project_id, operation_id, self.now())?)
    }

    /// 取消收口：operation 已被 cancel API 置为终态（`IdentityMismatch`）时同样
    /// 按已取消处理；取消前 provider 已越界写时，把越界事实附加到取消记录上
    /// （状态仍为 `Cancelled`，不改写成失败）；取消时无法确认 provider 已停止
    /// ⇒ 审计按「不确定」fail-closed 留痕，不得静默当作干净取消。返回值恒为
    /// 错误（`Cancelled` 或真实的 store 故障）。
    fn settle_cancelled(
        &self,
        project_id: &str,
        operation_id: &str,
        provider: &str,
        kind: KnowledgeBaseOperationKind,
        violations: &[String],
        audit_indeterminate: bool,
    ) -> KnowledgeBaseError {
        let store = match self.store() {
            Ok(store) => store,
            Err(error) => return error,
        };
        let result = if violations.is_empty() && !audit_indeterminate {
            store.cancel(project_id, operation_id, self.now())
        } else {
            let record = cancelled_audit_record(provider, kind, violations, audit_indeterminate);
            store.cancel_with_error(project_id, operation_id, record, self.now())
        };
        match result {
            Ok(()) | Err(ProductStoreError::IdentityMismatch { .. }) => {
                KnowledgeBaseError::Cancelled
            }
            Err(error) => error.into(),
        }
    }

    /// 终态写回遇到 `IdentityMismatch`：若记录已被并发取消，按 `Cancelled` 处理。
    fn settle_store_conflict(
        &self,
        project_id: &str,
        operation_id: &str,
        error: ProductStoreError,
    ) -> KnowledgeBaseError {
        if matches!(error, ProductStoreError::IdentityMismatch { .. })
            && let Ok(store) = self.store()
            && let Ok(current) = store.get(project_id, operation_id)
            && current.status == KnowledgeBaseOperationStatus::Cancelled
        {
            return KnowledgeBaseError::Cancelled;
        }
        error.into()
    }

    /// 失败收口撞上「取消 API 已先落盘」（`IdentityMismatch`）：若记录确已被
    /// 并发取消，已算出的越界审计事实不得丢弃——把审计记录附加到 Cancelled
    /// 记录上（状态保持 `Cancelled`）；无越界事实或非取消态冲突维持既有
    /// 收口语义（`settle_store_conflict`）。
    fn settle_failed_after_cancel_won(
        &self,
        operation: &KnowledgeBaseOperation,
        error: &KnowledgeBaseError,
        violations: &[String],
        store_error: ProductStoreError,
    ) -> KnowledgeBaseError {
        let (project_id, operation_id, provider, kind) = (
            operation.project_id.as_str(),
            operation.operation_id.as_str(),
            operation.input.provider.as_str(),
            operation.kind,
        );
        if violations.is_empty()
            || !matches!(store_error, ProductStoreError::IdentityMismatch { .. })
        {
            return self.settle_store_conflict(project_id, operation_id, store_error);
        }
        let Ok(store) = self.store() else {
            return store_error.into();
        };
        match store.get(project_id, operation_id) {
            Ok(current) if current.status == KnowledgeBaseOperationStatus::Cancelled => {
                match store.cancel_with_error(
                    project_id,
                    operation_id,
                    error_record(error, violations, provider, kind),
                    self.now(),
                ) {
                    // 附加成功，或他人已附加过（事实已在盘上）。
                    Ok(()) | Err(ProductStoreError::IdentityMismatch { .. }) => {
                        KnowledgeBaseError::Cancelled
                    }
                    Err(attach_error) => attach_error.into(),
                }
            }
            _ => self.settle_store_conflict(project_id, operation_id, store_error),
        }
    }

    /// 独占持有聚合索引锁直到返回的 guard drop（裁决 (b)：平台索引写入与
    /// 知识库 turn 互斥）。等锁在 blocking 线程上进行，不占 async executor。
    async fn acquire_index_isolation(
        &self,
        project_id: &str,
    ) -> Result<AggregateIndexIsolation, KnowledgeBaseError> {
        let lc_id = self.lc_id()?.to_string();
        let lock_path = aggregate_index_lock_path(&self.paths, &lc_id, project_id)
            .map_err(KnowledgeBaseError::Io)?;
        tokio::task::spawn_blocking(move || AggregateIndexIsolation::acquire(lock_path))
            .await
            .map_err(|error| {
                KnowledgeBaseError::Io(format!("index isolation task failed: {error}"))
            })?
            .map_err(KnowledgeBaseError::Io)
    }

    /// 驱动一次 turn。前后快照期间独占聚合索引锁（平台索引写入与 turn 互斥，
    /// 索引产物路径无需豁免）；provider 停止后（成功/失败/取消）无条件做后
    /// 快照审计：成功 turn 的越界以 `WriteScopeViolation` 失败，失败/取消
    /// turn 的越界作为 `violations` 附加，原始失败原因保持不变。取消但无法
    /// 确认 provider 已停止 ⇒ 审计标记为不确定（下界），由取消收口按
    /// fail-closed 留痕。后快照本身失败时，成功 turn fail-closed，失败/取消
    /// turn 保留原始原因。
    // 错误变体承载失败明细（主错误 + 越界路径 + 审计可信度），体积偏大；
    // 与仓内既有错误类型一致的处理方式，按值上抛不做装箱。
    #[allow(clippy::result_large_err)]
    async fn drive(
        &self,
        operation: &KnowledgeBaseOperation,
        canonical_root: &Path,
        cancellation: CancellationToken,
    ) -> Result<String, DriveFailure> {
        let (provider, command) = resolve_turn(operation.kind, &operation.input)?;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(KnowledgeBaseError::Cancelled.into()),
            ready = self.ensure_trust_ready(
                &operation.project_id,
                &operation.operation_id,
                canonical_root,
                &provider,
            ) => ready?,
        }
        let isolation = self.acquire_index_isolation(&operation.project_id).await?;
        let before = snapshot_blocking(canonical_root).await?;
        let lc_id = self.lc_id()?;
        let turn = self
            .runner
            .run_turn(KnowledgeBaseTurnRequest {
                project_id: &operation.project_id,
                operation_id: &operation.operation_id,
                lc_id,
                provider: &provider,
                command: &command,
                canonical_root,
                cancellation,
            })
            .await;
        // runner 返回取消 ≠ provider 已停止：未确认停止时后快照只是审计下界。
        let stop_unconfirmed =
            matches!(turn, Err(KnowledgeBaseTurnError::CancelledStopUnconfirmed));
        let audit = snapshot_blocking(canonical_root)
            .await
            .map(|after| write_scope_violations(&before, &after, operation.kind));
        drop(isolation);
        match (turn, audit) {
            (Ok(summary), Ok(violations)) if violations.is_empty() => Ok(summary),
            (Ok(_), Ok(paths)) => Err(KnowledgeBaseError::WriteScopeViolation { paths }.into()),
            (Ok(_), Err(error)) => Err(error.into()),
            (Err(turn_error), Ok(violations)) => Err(DriveFailure {
                error: turn_error.into(),
                violations,
                audit_indeterminate: stop_unconfirmed,
            }),
            (Err(turn_error), Err(_)) => {
                let cancelled = matches!(
                    turn_error,
                    KnowledgeBaseTurnError::Cancelled
                        | KnowledgeBaseTurnError::CancelledStopUnconfirmed
                );
                Err(DriveFailure {
                    error: KnowledgeBaseError::from(turn_error),
                    violations: Vec::new(),
                    // 取消 + 后快照失败 ⇒ 审计不可用，同样按不确定处理。
                    audit_indeterminate: stop_unconfirmed || cancelled,
                })
            }
        }
    }
}

fn step_kind(kind: KnowledgeBaseOperationKind) -> KnowledgeBaseStepKind {
    match kind {
        KnowledgeBaseOperationKind::UserInput => KnowledgeBaseStepKind::PrepareUserInput,
        KnowledgeBaseOperationKind::Bootstrap => KnowledgeBaseStepKind::Bootstrap,
        KnowledgeBaseOperationKind::Update => KnowledgeBaseStepKind::ApplyUpdate,
    }
}

/// 由 operation 输入解析 provider 与命令文本；任何缺失 fail-closed。
fn resolve_turn(
    kind: KnowledgeBaseOperationKind,
    input: &KnowledgeBaseOperationInput,
) -> Result<(ProviderName, String), KnowledgeBaseError> {
    let provider: ProviderName =
        serde_json::from_value(serde_json::Value::String(input.provider.clone())).map_err(
            |_| KnowledgeBaseError::InvalidInput(format!("unknown provider: {}", input.provider)),
        )?;
    let command = match kind {
        KnowledgeBaseOperationKind::UserInput => user_input_command(&provider),
        KnowledgeBaseOperationKind::Bootstrap => bootstrap_command(&provider),
        KnowledgeBaseOperationKind::Update => {
            let package = input.package_path.as_deref().ok_or_else(|| {
                KnowledgeBaseError::InvalidInput("update requires package_path".into())
            })?;
            update_command(&provider, package)
        }
    }
    .ok_or_else(|| {
        KnowledgeBaseError::InvalidInput(format!(
            "provider {} has no knowledge-base skill command",
            input.provider
        ))
    })?;
    Ok((provider, command))
}

/// `drive` 的失败面：主错误 + 失败/取消 turn 期间发现的附加越界路径 +
/// 审计可信度（取消时无法确认 provider 已停止 ⇒ 结果只能当下界）。
struct DriveFailure {
    error: KnowledgeBaseError,
    violations: Vec<String>,
    audit_indeterminate: bool,
}

impl From<KnowledgeBaseError> for DriveFailure {
    fn from(error: KnowledgeBaseError) -> Self {
        Self {
            error,
            violations: Vec::new(),
            audit_indeterminate: false,
        }
    }
}

include!("coordinator_audit.inc.rs");

/// 取消收口的附加记录：越界事实（如有）与审计可信度。取消时无法确认
/// provider 已停止 ⇒ 后快照只能作为下界，按 fail-closed 记为「审计不确定」，
/// 不得静默当作干净取消。
fn cancelled_audit_record(
    provider: &str,
    kind: KnowledgeBaseOperationKind,
    violations: &[String],
    audit_indeterminate: bool,
) -> AggregateInitializationErrorRecord {
    let mut record = error_record(&KnowledgeBaseError::Cancelled, violations, provider, kind);
    if audit_indeterminate {
        let note = "取消时无法确认 provider 已停止，写入面审计为下界，可能漏报";
        record.stderr_summary = Some(match record.stderr_summary.take() {
            Some(summary) => format!("{summary}；{note}"),
            None => note.to_string(),
        });
        if record.reason_code == REASON_CANCELLED {
            record.reason_code = REASON_CANCEL_AUDIT_INDETERMINATE.to_string();
        }
        record.retryable = false;
        record.action =
            "取消时无法确认 provider 已停止；人工检查聚合根工作树后重新发起".to_string();
    }
    record
}

// ───────────────────────── 写入面审计 ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

type TreeSnapshot = BTreeMap<String, FileStamp>;

/// 平台运行时子树（聚合根层 `.aria/runtime/**`）：issue 运行绑定与任务
/// 运行态（`IssueRuntimeBindingRecord.task_root` 直接落在该子树），由平台
/// 任意时刻写入且不受跨进程锁保护，来源无法归因 ⇒ 不计入审计。`.aria` 下
/// 其余内容与嵌套 `.aria` 照常审计：provider 的改动仍判越界。聚合索引产物
/// （`codegraph.json`、`.codegraph/**`）不在此列——turn 与平台索引写入已由
/// `.aggregate-index.lock` 运行期互斥，无需按路径豁免。
fn is_platform_runtime(relative: &str) -> bool {
    relative == ".aria/runtime" || relative.starts_with(".aria/runtime/")
}

/// 只读 git 命令（如 `git status`）会刷新 `index`：仅 `**/.git/index` 豁免，
/// `.git` 下其余内容（config / hooks / refs / HEAD 等）保留审计。
fn is_git_index(relative: &str) -> bool {
    relative == ".git/index" || relative.ends_with("/.git/index")
}

/// 递归文件清单（路径 → 大小 + mtime）。根层 `.aria/runtime/**` 子树与
/// `**/.git/index` 被跳过；符号链接按链接自身记录，不跟随。
fn snapshot_aggregate_root(root: &Path) -> Result<TreeSnapshot, KnowledgeBaseError> {
    fn walk(root: &Path, dir: &Path, out: &mut TreeSnapshot) -> Result<(), KnowledgeBaseError> {
        let io = |path: &Path, error: std::io::Error| {
            KnowledgeBaseError::Io(format!("snapshot {}: {error}", path.display()))
        };
        for entry in std::fs::read_dir(dir).map_err(|error| io(dir, error))? {
            let entry = entry.map_err(|error| io(dir, error))?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| KnowledgeBaseError::Io(error.to_string()))?
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
            if is_platform_runtime(&relative) {
                continue;
            }
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| io(&path, error))?;
            if metadata.is_dir() {
                walk(root, &path, out)?;
                continue;
            }
            if is_git_index(&relative) {
                continue;
            }
            out.insert(
                relative,
                FileStamp {
                    len: metadata.len(),
                    modified: metadata.modified().ok(),
                },
            );
        }
        Ok(())
    }
    let mut snapshot = TreeSnapshot::new();
    walk(root, root, &mut snapshot)?;
    Ok(snapshot)
}

async fn snapshot_blocking(root: &Path) -> Result<TreeSnapshot, KnowledgeBaseError> {
    let root: PathBuf = root.to_path_buf();
    tokio::task::spawn_blocking(move || snapshot_aggregate_root(&root))
        .await
        .map_err(|error| KnowledgeBaseError::Io(format!("snapshot task failed: {error}")))?
}

/// 允许面按 operation kind 区分：
/// - Bootstrap/Update：`cadence/knowledge-base/**`、`cadence/project-rules/**`、
///   根 `CLAUDE.md`、根 `AGENTS.md`；
/// - UserInput：仅 `cadence/knowledge-base/user-input/**`（起草 skill 契约
///   「只写这个目录」，其余路径——含 knowledge-base 其它内容与根入口
///   文件——一律越界）。
fn is_allowed_write(kind: KnowledgeBaseOperationKind, relative: &str) -> bool {
    match kind {
        KnowledgeBaseOperationKind::UserInput => {
            relative.starts_with("cadence/knowledge-base/user-input/")
        }
        KnowledgeBaseOperationKind::Bootstrap | KnowledgeBaseOperationKind::Update => {
            relative == "CLAUDE.md"
                || relative == "AGENTS.md"
                || relative.starts_with("cadence/knowledge-base/")
                || relative.starts_with("cadence/project-rules/")
        }
    }
}

/// turn 前后快照差分：新增、修改、删除且不在该 kind 允许面内的路径
/// （升序去重）。
fn write_scope_violations(
    before: &TreeSnapshot,
    after: &TreeSnapshot,
    kind: KnowledgeBaseOperationKind,
) -> Vec<String> {
    let mut changed = BTreeSet::new();
    for (path, stamp) in before {
        if after.get(path) != Some(stamp) {
            changed.insert(path.clone());
        }
    }
    for path in after.keys() {
        if !before.contains_key(path) {
            changed.insert(path.clone());
        }
    }
    changed
        .into_iter()
        .filter(|path| !is_allowed_write(kind, path))
        .collect()
}

// ───────────────────────── gateway 驱动 ─────────────────────────

/// 生产 runner：turn 经 [`LogicalCodebaseProviderGateway::start_streaming`]
/// 启动，Normal 相位 + `CodingTargetWrite`，不携带 bootstrap 凭据。
pub struct GatewayKnowledgeBaseTurnRunner {
    gateway: Arc<LogicalCodebaseProviderGateway>,
    paths: ProductAppPaths,
    capability_snapshot_ref: String,
    command_timeout: Duration,
    /// Abort 后等待 provider 事件流关闭（确认停止）的宽限预算。
    cancel_stop_grace: Duration,
}

impl GatewayKnowledgeBaseTurnRunner {
    pub fn new(gateway: Arc<LogicalCodebaseProviderGateway>, paths: ProductAppPaths) -> Self {
        Self {
            gateway,
            paths,
            capability_snapshot_ref: DEFAULT_CAPABILITY_SNAPSHOT_REF.to_string(),
            command_timeout: Duration::from_secs(DEFAULT_TURN_TIMEOUT_SECS),
            cancel_stop_grace: Duration::from_secs(DEFAULT_CANCEL_STOP_GRACE_SECS),
        }
    }

    /// 注入 turn 超时预算（测试用短超时）。
    pub fn with_command_timeout(mut self, command_timeout: Duration) -> Self {
        self.command_timeout = command_timeout;
        self
    }

    /// 注入取消/超时后等待 provider 停止的宽限预算（测试用短宽限）。
    pub fn with_cancel_stop_grace(mut self, cancel_stop_grace: Duration) -> Self {
        self.cancel_stop_grace = cancel_stop_grace;
        self
    }

    /// Abort 后等待 provider 会话真正结束：事件流关闭（`recv` 返回 `None`）
    /// 即管道断开、provider 停止输出，后快照才落在可信边界上。宽限期满
    /// 仍未关闭 ⇒ 返回 `false`（停止未确认）。
    async fn await_provider_stop(session: &mut ProviderSession, grace: Duration) -> bool {
        let deadline = tokio::time::Instant::now() + grace;
        loop {
            let wait =
                tokio::time::sleep(deadline.saturating_duration_since(tokio::time::Instant::now()));
            tokio::pin!(wait);
            tokio::select! {
                _ = &mut wait => return false,
                event = session.events.recv() => {
                    if event.is_none() {
                        return true;
                    }
                    // 收尾期间的事件不进摘要：turn 结论已定为取消/超时/交互。
                }
            }
        }
    }

    fn launch_request(
        provider: &ProviderRef,
        project_id: &str,
        aggregate_root: &Path,
    ) -> SessionLaunchRequest {
        SessionLaunchRequest {
            project_id: project_id.to_string(),
            provider: provider.clone(),
            action: SessionPolicyAction::CodingTargetWrite,
            target: PolicyTarget::aggregate_root(aggregate_root.to_path_buf()),
            working_directory: aggregate_root.to_path_buf(),
            readable_roots: vec![aggregate_root.to_path_buf()],
            writable_roots: vec![aggregate_root.to_path_buf()],
            config_artifact_ref: KNOWLEDGE_BASE_CONFIG_ARTIFACT_REF.to_string(),
        }
    }

    fn streaming_input(
        provider: &ProviderRef,
        aggregate_root: &Path,
        command: &str,
        timeout: Duration,
    ) -> StreamingProviderInput {
        use crate::protocol::contracts::{AdapterRole, ProviderType};
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            tool_policy: None,
            audit_sink: None,
            provider_type: match provider.provider_type {
                ProviderRefType::ClaudeCode => ProviderType::ClaudeCode,
                ProviderRefType::Codex => ProviderType::Codex,
                ProviderRefType::Pi => ProviderType::Pi,
                ProviderRefType::KimiCode => ProviderType::KimiCode,
            },
            role: AdapterRole::Executor,
            prompt: Self::unattended_prompt(command),
            working_dir: aggregate_root.to_path_buf(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: timeout.as_secs().max(1),
        }
    }

    /// 知识库操作的 prompt = 命令 + **无人值守指令**。
    ///
    /// 知识库 operation 是后台长任务，没有交互通道：provider 一旦发起提问或
    /// 授权请求，本次 turn 直接判失败（`provider_interaction_required`）。真机
    /// 上 Pi 在 bootstrap 里发问即失败，用户只看到一句无信息量的兜底。
    ///
    /// 指令只约束**交互行为**（不要提问、不要等回答），不改变 skill 的业务
    /// 规则：需要用户授权的破坏性分支仍按 skill 规定停止并记录，不得擅自清理。
    fn unattended_prompt(command: &str) -> String {
        format!(
            "{command}\n\n\
             （无人值守执行：本次操作没有交互通道。不要向用户提问，也不要请求授权\
             或等待回答；遇到需要用户决策的分支，按 skill 规定的非破坏性默认路径继续\
             （无法安全继续时停止并在产物中写明），不要停在等待回答上。）"
        )
    }

    /// 以单仓/聚合初始化同款语义消费会话：取消/超时共用预算，输出累积为有界
    /// 摘要。Abort 后在 `stop_grace` 内等待事件流关闭，确认 provider 停止再
    /// 返回——取消路径的返回值区分「已确认停止」与「未确认」。
    async fn consume_turn(
        mut session: ProviderSession,
        remaining: Duration,
        stop_grace: Duration,
        cancellation: CancellationToken,
    ) -> Result<String, KnowledgeBaseTurnError> {
        let mut output = BoundedOutput::default();
        let timeout = tokio::time::sleep(remaining);
        tokio::pin!(timeout);
        loop {
            let event = tokio::select! {
                biased;
                _ = cancellation.cancelled() => {
                    best_effort_abort(&session);
                    // provider 真正停止（事件流关闭）后才返回取消；宽限期内
                    // 未关闭 ⇒ 停止未确认，交给协调器按「审计不确定」处理。
                    if Self::await_provider_stop(&mut session, stop_grace).await {
                        return Err(KnowledgeBaseTurnError::Cancelled);
                    }
                    return Err(KnowledgeBaseTurnError::CancelledStopUnconfirmed);
                }
                _ = &mut timeout => {
                    best_effort_abort(&session);
                    let _ = Self::await_provider_stop(&mut session, stop_grace).await;
                    return Err(turn_failed("provider_turn_timed_out", "knowledge base turn timed out", &output, true));
                }
                event = session.events.recv() => event,
            };
            match event {
                Some(ProviderEvent::TextDelta { content }) => output.push(&content),
                Some(ProviderEvent::Execution(execution)) => {
                    if let Some(event_output) = execution.output {
                        output.push(&event_output);
                    }
                }
                Some(ProviderEvent::ToolResult(result)) => output.push(&result.output),
                Some(ProviderEvent::Completed(completion)) => {
                    if output.is_empty() {
                        output.push(&completion.full_output);
                    }
                    return Ok(output.0);
                }
                Some(ProviderEvent::Failed { message }) => {
                    output.push(&message);
                    return Err(turn_failed(
                        "provider_reported_failure",
                        "provider reported failure",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::ProtocolError { code, message, .. }) => {
                    output.push(&format!("{code}: {message}"));
                    return Err(turn_failed(
                        "provider_protocol_error",
                        "provider protocol error",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::PermissionTimeout { .. }) => {
                    return Err(turn_failed(
                        "provider_permission_timeout",
                        "provider permission timeout",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::StatusChanged(ProviderStatus::Failed)) => {
                    return Err(turn_failed(
                        "provider_status_failed",
                        "provider status failed",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::StatusChanged(ProviderStatus::Aborted)) => {
                    return Err(turn_failed(
                        "provider_status_aborted",
                        "provider status aborted",
                        &output,
                        true,
                    ));
                }
                Some(ProviderEvent::PermissionRequest(request)) => {
                    // 交互请求（授权/提问）在知识库操作里没有应答通道 ⇒ 失败。
                    // 但**必须把 provider 问了什么写进诊断**：否则用户只看到
                    // 「provider interaction required」这句无信息量的兜底，
                    // 不知道该改哪里（真机：Pi 在 bootstrap 里发问，失败面看不出
                    // 原因）。诊断文本由事件数据类型自己渲染（见
                    // `cross_cutting::streaming_provider`）。
                    output.push(&crate::product::logical_codebase::interaction_diagnostic::permission_interaction_text(&request));
                    best_effort_abort(&session);
                    let _ = Self::await_provider_stop(&mut session, stop_grace).await;
                    return Err(turn_failed(
                        "provider_interaction_required",
                        "provider interaction required",
                        &output,
                        false,
                    ));
                }
                Some(ProviderEvent::ChoiceRequest(request)) => {
                    // 同上：把 provider 的提问原文写进诊断（用户要看到问题）。
                    output.push(&crate::product::logical_codebase::interaction_diagnostic::choice_interaction_text(&request));
                    best_effort_abort(&session);
                    let _ = Self::await_provider_stop(&mut session, stop_grace).await;
                    return Err(turn_failed(
                        "provider_interaction_required",
                        "provider interaction required",
                        &output,
                        false,
                    ));
                }
                Some(ProviderEvent::StatusChanged(_))
                | Some(ProviderEvent::ToolCall(_))
                | Some(ProviderEvent::UsageReport(_))
                | Some(ProviderEvent::ToolPolicyDecision(_))
                | Some(ProviderEvent::ToolPolicyWarning(_))
                | Some(ProviderEvent::ToolPolicyTerminated(_))
                | Some(ProviderEvent::TerminalActivity(_)) => {}
                None => {
                    return Err(turn_failed(
                        "provider_stream_closed",
                        "provider event stream closed before completion",
                        &output,
                        true,
                    ));
                }
            }
        }
    }
}

#[async_trait]
impl KnowledgeBaseTurnRunner for GatewayKnowledgeBaseTurnRunner {
    async fn run_turn(
        &self,
        request: KnowledgeBaseTurnRequest<'_>,
    ) -> Result<String, KnowledgeBaseTurnError> {
        let KnowledgeBaseTurnRequest {
            project_id,
            operation_id,
            lc_id,
            provider,
            command,
            canonical_root,
            cancellation,
        } = request;
        let launch_failed = |code: &str, detail: String| KnowledgeBaseTurnError::Failed {
            reason_code: code.to_string(),
            summary: sanitize_turn_output(&detail),
            retryable: true,
        };
        let provider_ref = ProviderRef::from_provider_name(provider, &self.capability_snapshot_ref)
            .map_err(|error| launch_failed("provider_launch_unsupported", error.to_string()))?;
        let launch_request = Self::launch_request(&provider_ref, project_id, canonical_root);

        // 真实材料 admission 预检先于 gateway validate：缺材料/漂移在 spawn 前
        // fail-closed，provider 保持零启动。
        let admission = LogicalCodebaseProviderAdmissionPreflight::new(
            self.paths.clone(),
            lc_id,
            self.gateway.clone(),
        );
        if let Err(error) = admission.check(&launch_request, &ProviderAdmissionPhase::Normal) {
            return Err(launch_failed(
                "provider_admission_denied",
                format!("provider admission preflight denied: {error:?}"),
            ));
        }

        let input =
            Self::streaming_input(&provider_ref, canonical_root, command, self.command_timeout);
        let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(self.paths.clone());
        let workspace_session_id = format!("knowledge-base-{project_id}-{operation_id}");
        let context = lifecycle
            .allocate_launch_audit_context(&workspace_session_id)
            .map_err(|error| {
                launch_failed("tool_policy_role_run_seq_alloc_failed", error.to_string())
            })?;
        let launch = self
            .gateway
            .prepare_streaming_launch(input, launch_request, context)
            .map_err(|error| {
                launch_failed(
                    "gateway_prepare_failed",
                    format!("gateway prepare_streaming_launch failed: {error}"),
                )
            })?;

        // 启动与事件消费共享同一超时预算。
        let started = std::time::Instant::now();
        let start = self.gateway.start_streaming(launch, cancellation.clone());
        tokio::pin!(start);
        let timeout = tokio::time::sleep(self.command_timeout);
        tokio::pin!(timeout);
        let session = tokio::select! {
            biased;
            // start future 被 drop，无法确认 provider 子进程是否已 spawn：
            // 停止未确认，按审计不确定收口。
            _ = cancellation.cancelled() => {
                return Err(KnowledgeBaseTurnError::CancelledStopUnconfirmed)
            }
            _ = &mut timeout => {
                return Err(launch_failed(
                    "provider_turn_timed_out",
                    "knowledge base turn timed out before session start".to_string(),
                ));
            }
            result = &mut start => result.map_err(|error| {
                launch_failed("provider_start_failed", format!("gateway start_streaming failed: {error}"))
            })?,
        };
        let remaining = self.command_timeout.saturating_sub(started.elapsed());
        Self::consume_turn(session, remaining, self.cancel_stop_grace, cancellation).await
    }
}

/// 有界输出累积：超限截断（UTF-8 字符边界安全）。
#[derive(Default)]
struct BoundedOutput(String);

impl BoundedOutput {
    fn push(&mut self, value: &str) {
        let used = self.0.len();
        if used >= KNOWLEDGE_BASE_TURN_OUTPUT_LIMIT {
            return;
        }
        let room = KNOWLEDGE_BASE_TURN_OUTPUT_LIMIT - used;
        if value.len() <= room {
            self.0.push_str(value);
            return;
        }
        let mut end = room;
        while end > 0 && !value.is_char_boundary(end) {
            end -= 1;
        }
        self.0.push_str(&value[..end]);
    }

    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

fn turn_failed(
    reason_code: &str,
    reason: &str,
    output: &BoundedOutput,
    retryable: bool,
) -> KnowledgeBaseTurnError {
    let detail = if output.is_empty() {
        reason.to_string()
    } else {
        format!("{reason}: {}", output.0)
    };
    KnowledgeBaseTurnError::Failed {
        reason_code: reason_code.to_string(),
        summary: sanitize_turn_output(&detail),
        retryable,
    }
}

/// best-effort 中止：取消/超时/交互请求时向会话发送 Abort；失败不阻塞失败路径。
fn best_effort_abort(session: &ProviderSession) {
    let _ = session.commands.try_send(ProviderCommand::Abort);
}

#[cfg(test)]
mod tests {
    include!("coordinator_tests.inc.rs");
    include!("coordinator_draft_tests.inc.rs");
    include!("coordinator_fix_tests.inc.rs");
}
