//! 知识库 bootstrap / update 的执行层：provider turn、写入面审计、trust 门
//! 与 per-LC 单飞（LC 知识库模块实施计划 Task 6）。
//!
//! - turn 走 Normal 相位 + `SessionPolicyAction::CodingTargetWrite`，target =
//!   canonical 聚合根、cwd = 聚合根、可写根 = 聚合根；不引入
//!   `BootstrapPhaseCredential`（聚合初始化的凭据链保持冻结）。
//! - 单飞：同一 LC 的 bootstrap 与 update 互斥，任一 operation 在跑时拒绝新启。
//! - 写入面审计：turn 前后对聚合根做快照差分，允许面之外的新增/修改/删除
//!   一律使 operation 失败并记 `knowledge_base_write_scope_violation`。
//! - trust 前置门沿用聚合初始化口径（`ProviderTrustScenario::Recipe`）。

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::cross_cutting::diagnostic_redaction::sanitize_turn_output;
use crate::cross_cutting::streaming_provider::{
    ProviderCommand, ProviderEvent, ProviderPermissionMode, ProviderSession, ProviderStatus,
    StreamingProviderInput,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
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

use super::command::{bootstrap_command, update_command};
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
/// 失败记录里最多列出的越界路径条数。
const VIOLATION_SUMMARY_LIMIT: usize = 20;

const REASON_WRITE_SCOPE_VIOLATION: &str = "knowledge_base_write_scope_violation";
const REASON_TURN_FAILED: &str = "knowledge_base_provider_turn_failed";
const REASON_INVALID_INPUT: &str = "knowledge_base_invalid_input";
const REASON_INTERNAL: &str = "knowledge_base_internal_error";
const REASON_TRUST_WAITING: &str = "aggregate_initialization_trust_waiting";
const FAILURE_STAGE: &str = "knowledge_base";

/// 时钟：返回 RFC3339 时间戳。
pub type Clock = dyn Fn() -> String + Send + Sync;

/// 进程内 begin 临界区：「查重 → 单飞检查 → 创建」不可被同进程并发打断。
static BEGIN_LOCK: Mutex<()> = Mutex::new(());

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
        let _guard = BEGIN_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match store.get(project_id, &operation_id) {
            Ok(existing) => return Ok(existing),
            Err(ProductStoreError::NotFound { .. }) => {}
            Err(error) => return Err(error.into()),
        }
        if let Some(running) = self.other_running(project_id)? {
            return Err(KnowledgeBaseError::Preflight(
                KnowledgeBasePreflightFailure::OtherKnowledgeBaseRunning {
                    operation_id: running,
                },
            ));
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
        Ok(store.create(operation)?)
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
        match self.drive(&operation, canonical_root, cancellation).await {
            Ok(()) => store
                .finish_completed(project_id, operation_id, self.now())
                .map_err(|error| self.settle_store_conflict(project_id, operation_id, error)),
            Err(KnowledgeBaseError::Cancelled) => {
                Err(self.settle_cancelled(project_id, operation_id))
            }
            Err(error) => {
                let record = error_record(&error, &operation.input.provider);
                match store.finish_failed(
                    project_id,
                    operation_id,
                    Some(step_kind(operation.kind)),
                    record,
                    self.now(),
                ) {
                    Ok(()) => Err(error),
                    Err(store_error) => {
                        Err(self.settle_store_conflict(project_id, operation_id, store_error))
                    }
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
    /// 按已取消处理。返回值恒为错误（`Cancelled` 或真实的 store 故障）。
    fn settle_cancelled(&self, project_id: &str, operation_id: &str) -> KnowledgeBaseError {
        let store = match self.store() {
            Ok(store) => store,
            Err(error) => return error,
        };
        match store.cancel(project_id, operation_id, self.now()) {
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

    async fn drive(
        &self,
        operation: &KnowledgeBaseOperation,
        canonical_root: &Path,
        cancellation: CancellationToken,
    ) -> Result<(), KnowledgeBaseError> {
        let (provider, command) = resolve_turn(operation.kind, &operation.input)?;
        tokio::select! {
            biased;
            _ = cancellation.cancelled() => return Err(KnowledgeBaseError::Cancelled),
            ready = self.ensure_trust_ready(
                &operation.project_id,
                &operation.operation_id,
                canonical_root,
                &provider,
            ) => ready?,
        }
        let before = snapshot_blocking(canonical_root).await?;
        let lc_id = self.lc_id()?;
        self.runner
            .run_turn(KnowledgeBaseTurnRequest {
                project_id: &operation.project_id,
                operation_id: &operation.operation_id,
                lc_id,
                provider: &provider,
                command: &command,
                canonical_root,
                cancellation,
            })
            .await?;
        let after = snapshot_blocking(canonical_root).await?;
        let violations = write_scope_violations(&before, &after);
        if violations.is_empty() {
            Ok(())
        } else {
            Err(KnowledgeBaseError::WriteScopeViolation { paths: violations })
        }
    }
}

fn step_kind(kind: KnowledgeBaseOperationKind) -> KnowledgeBaseStepKind {
    match kind {
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

/// 把执行层错误映射为持久化失败记录。
fn error_record(error: &KnowledgeBaseError, provider: &str) -> AggregateInitializationErrorRecord {
    let record = |reason_code: &str,
                  summary: Option<String>,
                  retryable: bool,
                  action: &str,
                  cause: Option<String>| AggregateInitializationErrorRecord {
        stage: FAILURE_STAGE.to_string(),
        reason_code: reason_code.to_string(),
        stderr_summary: summary,
        retryable,
        action: action.to_string(),
        cause,
        provider: Some(provider.to_string()),
    };
    match error {
        KnowledgeBaseError::TrustWaiting(waiting) => AggregateInitializationErrorRecord {
            stage: FAILURE_STAGE.to_string(),
            reason_code: REASON_TRUST_WAITING.to_string(),
            stderr_summary: Some(waiting.message.clone()),
            retryable: true,
            action: waiting.retry_action.clone(),
            cause: Some(waiting.reason_code.clone()),
            provider: Some(provider_wire_label(&waiting.provider).to_string()),
        },
        KnowledgeBaseError::WriteScopeViolation { paths } => {
            let mut listed = paths
                .iter()
                .take(VIOLATION_SUMMARY_LIMIT)
                .cloned()
                .collect::<Vec<_>>()
                .join(", ");
            if paths.len() > VIOLATION_SUMMARY_LIMIT {
                listed.push_str(&format!(" 等共 {} 处", paths.len()));
            }
            record(
                REASON_WRITE_SCOPE_VIOLATION,
                Some(listed),
                false,
                "知识库只允许写 cadence/knowledge-base、cadence/project-rules 与根 CLAUDE.md/AGENTS.md；检查越界改动后重新发起",
                None,
            )
        }
        KnowledgeBaseError::TurnFailed {
            reason_code,
            summary,
            retryable,
        } => record(
            REASON_TURN_FAILED,
            Some(summary.clone()),
            *retryable,
            "查看 provider 输出摘要，处理后重新发起",
            Some(reason_code.clone()),
        ),
        KnowledgeBaseError::InvalidInput(message) => record(
            REASON_INVALID_INPUT,
            Some(message.clone()),
            false,
            "修正输入后重新发起",
            None,
        ),
        other => record(
            REASON_INTERNAL,
            Some(other.to_string()),
            false,
            "检查服务端日志后重新发起",
            None,
        ),
    }
}

// ───────────────────────── 写入面审计 ─────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
struct FileStamp {
    len: u64,
    modified: Option<SystemTime>,
}

type TreeSnapshot = BTreeMap<String, FileStamp>;

/// 递归文件清单（路径 → 大小 + mtime）。`.git` 元数据被跳过（只读 git 命令
/// 也会刷新其内容）；符号链接按链接自身记录，不跟随。
fn snapshot_aggregate_root(root: &Path) -> Result<TreeSnapshot, KnowledgeBaseError> {
    fn walk(root: &Path, dir: &Path, out: &mut TreeSnapshot) -> Result<(), KnowledgeBaseError> {
        let io = |path: &Path, error: std::io::Error| {
            KnowledgeBaseError::Io(format!("snapshot {}: {error}", path.display()))
        };
        for entry in std::fs::read_dir(dir).map_err(|error| io(dir, error))? {
            let entry = entry.map_err(|error| io(dir, error))?;
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            let metadata = std::fs::symlink_metadata(&path).map_err(|error| io(&path, error))?;
            if metadata.is_dir() {
                walk(root, &path, out)?;
                continue;
            }
            let relative = path
                .strip_prefix(root)
                .map_err(|error| KnowledgeBaseError::Io(error.to_string()))?
                .components()
                .map(|component| component.as_os_str().to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join("/");
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

/// 允许面：`cadence/knowledge-base/**`、`cadence/project-rules/**`、根
/// `CLAUDE.md`、根 `AGENTS.md`。
fn is_allowed_write(relative: &str) -> bool {
    relative == "CLAUDE.md"
        || relative == "AGENTS.md"
        || relative.starts_with("cadence/knowledge-base/")
        || relative.starts_with("cadence/project-rules/")
}

/// turn 前后快照差分：新增、修改、删除且不在允许面内的路径（升序去重）。
fn write_scope_violations(before: &TreeSnapshot, after: &TreeSnapshot) -> Vec<String> {
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
        .filter(|path| !is_allowed_write(path))
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
}

impl GatewayKnowledgeBaseTurnRunner {
    pub fn new(gateway: Arc<LogicalCodebaseProviderGateway>, paths: ProductAppPaths) -> Self {
        Self {
            gateway,
            paths,
            capability_snapshot_ref: DEFAULT_CAPABILITY_SNAPSHOT_REF.to_string(),
            command_timeout: Duration::from_secs(DEFAULT_TURN_TIMEOUT_SECS),
        }
    }

    /// 注入 turn 超时预算（测试用短超时）。
    pub fn with_command_timeout(mut self, command_timeout: Duration) -> Self {
        self.command_timeout = command_timeout;
        self
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
            prompt: command.to_string(),
            working_dir: aggregate_root.to_path_buf(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: BTreeMap::new(),
            timeout_secs: timeout.as_secs().max(1),
        }
    }

    /// 以单仓/聚合初始化同款语义消费会话：取消/超时共用预算，输出累积为有界摘要。
    async fn consume_turn(
        mut session: ProviderSession,
        remaining: Duration,
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
                    return Err(KnowledgeBaseTurnError::Cancelled);
                }
                _ = &mut timeout => {
                    best_effort_abort(&session);
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
                Some(ProviderEvent::PermissionRequest(_))
                | Some(ProviderEvent::ChoiceRequest(_)) => {
                    best_effort_abort(&session);
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
            _ = cancellation.cancelled() => return Err(KnowledgeBaseTurnError::Cancelled),
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
        Self::consume_turn(session, remaining, cancellation).await
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
}
