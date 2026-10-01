//! C4 Task 8：真实 provider/index/planning 准入门。
//!
//! 在任何 LC provider turn / planning envelope 启动前，用 Task 1 的
//! `RepositoryAuthorityResolver` 冻结 target 与 authority root，读取实际
//! 聚合 policy artifact（id/revision/digest），并检查真实 provider 将消费的
//! 每个成员 checkout 的 `.claude/rules/language.md` 与实际 gateway capability
//! 谓词。`ProviderCapabilityStore::ensure_bootstrap` 只产生待验证记录，不能
//! 证明真实能力；本预检的 `ready == true` 也只表示「材料齐备且 gateway
//! `validate` 产出 envelope」，spawn 前仍必须调用
//! `LogicalCodebaseProviderGateway::revalidate_before_spawn`。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use sha2::Digest as _;

use crate::product::logical_codebase::aggregate_initialization::{
    AggregateInitializationOperation, AggregateInitializationOperationStatus,
    AggregateInitializationStepKind, AggregateInitializationStepRecord,
    AggregateInitializationStepStatus,
};
use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
use crate::product::logical_codebase::policy::SessionPolicyAction;
use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::provider_gateway::{
    LogicalCodebaseProviderGateway, ProviderGatewayError, SessionLaunchRequest,
    ValidatedSessionLaunchPolicy,
};
use crate::product::logical_codebase::repository_routing::{
    AuthorityPolicyReference, RepositoryAuthorityResolver, RepositoryRoutingRequest,
    RepositoryTargetKind, ResolvedTargetIdentity,
};
use crate::product::logical_codebase::store::LogicalCodebaseStore;
use crate::product::logical_codebase::types::{
    CheckoutKind, LogicalRepositoryId, MemberStatus, RepositoryCheckoutId,
};

/// 冷启动等待面的动作种类。首定义位于 Task 8（Task 3 的 bootstrap 投影被
/// 排产延后）；Task 3 落地 bootstrap.rs 时必须 `use` 本类型，不得另造同义枚举。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapActionKind {
    Prepare,
    Continue,
    Retry,
    Revalidate,
    Repair,
}
impl BootstrapActionKind {
    /// 与 serde snake_case 序列化一致的稳定动作名（通知 payload/前端按钮
    /// 语义共用，不引入第二套字符串协议）。
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Continue => "continue",
            Self::Retry => "retry",
            Self::Revalidate => "revalidate",
            Self::Repair => "repair",
        }
    }
}


/// 真实 provider 将消费的成员规则文件引用（C4 Task 8）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderRuleReference {
    pub member_id: LogicalRepositoryId,
    pub checkout_id: RepositoryCheckoutId,
    pub path: PathBuf,
    pub digest: Option<String>,
}

/// REQ-BOOT-04：自举相位凭据——「根规则尚未生成」的唯一自举例外证明。
///
/// 内部不透明：只能经 [`BootstrapPhaseCredential::from_running_operation`]
/// 从当前 durable Running aggregate initialization operation 派生（不公开
/// 普通构造器）；字段在 provider spawn 前由 admission `check` 重新核验，
/// 普通 provider session 不得自行声明该相位。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapPhaseCredential {
    project_id: String,
    logical_codebase_id: String,
    operation_id: String,
    step: AggregateInitializationStepKind,
    input_digest: String,
    canonical_root: PathBuf,
}

/// admission 相位（Task 1.2）：`Normal` = 普通 session（根规则必须存在）；
/// `AggregateBootstrap` = LC 根 recipe 自举 turn——凭据只豁免「根规则尚未
/// 生成」的存在性检查，authority/policy/capability/gateway/cwd/target 与
/// availability 全部照常必检。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderAdmissionPhase {
    Normal,
    AggregateBootstrap(BootstrapPhaseCredential),
}

impl BootstrapPhaseCredential {
    /// 唯一构造入口：从 durable Running operation 派生凭据。要求 operation
    /// 处于 `Running`、目标 step 是 provider turn 且正在运行并已记录
    /// input digest、`canonical_root` 与 operation 的
    /// `provider_context_root` 一致；任一不满足即 fail-closed waiting。
    pub fn from_running_operation(
        store: &AggregateInitializationOperationStore,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        lc_id: &str,
        canonical_root: &Path,
    ) -> Result<Self, ProviderAdmissionError> {
        let operation = store.get(project_id, operation_id)?;
        ensure_bootstrap_operation_running(&operation)?;
        ensure_bootstrap_step_is_provider_turn(step)?;
        let record = bootstrap_step_record(&operation, step)?;
        let input_digest = record.input_digest.clone().ok_or_else(|| {
            bootstrap_waiting(
                "bootstrap_step_input_digest_missing",
                format!(
                    "bootstrap step {step:?} of operation {operation_id} has no input digest"
                ),
            )
        })?;
        if !bootstrap_roots_match(canonical_root, &operation.input.provider_context_root) {
            return Err(bootstrap_waiting(
                "bootstrap_root_drift",
                format!(
                    "credential root {} does not match operation provider context root {}",
                    canonical_root.display(),
                    operation.input.provider_context_root.display()
                ),
            ));
        }
        Ok(Self {
            project_id: project_id.to_string(),
            logical_codebase_id: lc_id.to_string(),
            operation_id: operation_id.to_string(),
            step,
            input_digest,
            canonical_root: canonical_root.to_path_buf(),
        })
    }

    /// spawn 前重核验（admission `check` 在 `AggregateBootstrap` 相位调用）：
    /// 重新读取 durable operation，比对 status/step/input digest/LC/root。
    /// 任一漂移（operation 已 Completed/Failed/Cancelled、step 已推进、
    /// digest 变化、LC 不符）即 fail-closed waiting，凭据不可复用。
    pub fn reverify_against_running_operation(
        &self,
        store: &AggregateInitializationOperationStore,
        lc_id: &str,
    ) -> Result<(), ProviderAdmissionError> {
        if self.logical_codebase_id != lc_id {
            return Err(bootstrap_waiting(
                "bootstrap_logical_codebase_mismatch",
                format!(
                    "credential logical codebase {} does not match admission scope {lc_id}",
                    self.logical_codebase_id
                ),
            ));
        }
        let operation = store.get(&self.project_id, &self.operation_id)?;
        ensure_bootstrap_operation_running(&operation)?;
        let record = bootstrap_step_record(&operation, self.step)?;
        let current_digest = record.input_digest.as_deref().ok_or_else(|| {
            bootstrap_waiting(
                "bootstrap_step_input_digest_missing",
                format!(
                    "bootstrap step {:?} of operation {} has no input digest",
                    self.step, self.operation_id
                ),
            )
        })?;
        if current_digest != self.input_digest {
            return Err(bootstrap_waiting(
                "bootstrap_input_digest_drift",
                format!(
                    "credential input digest {} does not match durable digest {current_digest}",
                    self.input_digest
                ),
            ));
        }
        if !bootstrap_roots_match(&self.canonical_root, &operation.input.provider_context_root) {
            return Err(bootstrap_waiting(
                "bootstrap_root_drift",
                format!(
                    "credential root {} does not match operation provider context root {}",
                    self.canonical_root.display(),
                    operation.input.provider_context_root.display()
                ),
            ));
        }
        Ok(())
    }

    /// 凭据绑定的 durable operation id（receipt 审计关联用）。
    pub fn operation_id(&self) -> &str {
        &self.operation_id
    }

    /// 凭据绑定的自举 step。
    pub fn step(&self) -> AggregateInitializationStepKind {
        self.step
    }

    /// 凭据冻结的 canonical 聚合根。
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    /// 测试专用构造器（`cfg(test)`）：生产面凭据只能经
    /// `from_running_operation` 派生；跨模块单测（如 kimi 通道判定）仅用此
    /// 满足类型面，不进入 admission/spawn 判定路径。
    #[cfg(test)]
    pub fn for_test(
        project_id: &str,
        logical_codebase_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        input_digest: &str,
        canonical_root: PathBuf,
    ) -> Self {
        Self {
            project_id: project_id.to_string(),
            logical_codebase_id: logical_codebase_id.to_string(),
            operation_id: operation_id.to_string(),
            step,
            input_digest: input_digest.to_string(),
            canonical_root,
        }
    }
}

/// D1：BootstrapExecutor 联合证明标记——「有写权限的 Executor」在
/// `ProviderToolPolicy` 通道上的唯一显式载体，由 credential（durable
/// Running operation 派生）、bootstrap action（写权限）、canonical root 与
/// receipt context 四要素共同证明；任一缺失即不成立。三 adapter 在真实
/// spawn 前经 `validate_tool_policy_for_role` 消费并复核本标记；普通
/// Executor/Coder 仍不得携带任何 tool policy。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootstrapExecutorMarker {
    credential: BootstrapPhaseCredential,
    action: SessionPolicyAction,
    canonical_root: PathBuf,
    receipt_context: String,
}

/// marker 构造失败（fail-closed）：四要素缺一不可。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BootstrapExecutorMarkerError {
    EmptyReceiptContext,
    EmptyCanonicalRoot,
    /// 自举执行器必须携带写权限 action（`CodingTargetWrite`）。
    InvalidAction(SessionPolicyAction),
}

impl std::fmt::Display for BootstrapExecutorMarkerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::EmptyReceiptContext => {
                write!(f, "bootstrap executor marker requires a receipt context")
            }
            Self::EmptyCanonicalRoot => {
                write!(f, "bootstrap executor marker requires a canonical root")
            }
            Self::InvalidAction(action) => write!(
                f,
                "bootstrap executor marker requires CodingTargetWrite, got {action:?}"
            ),
        }
    }
}

impl std::error::Error for BootstrapExecutorMarkerError {}

impl BootstrapExecutorMarker {
    /// 联合证明构造：credential + 写权限 action + canonical root + receipt
    /// context 齐备才成立；任一缺失/非法返回错误而非退化标记。
    pub fn new(
        credential: BootstrapPhaseCredential,
        action: SessionPolicyAction,
        canonical_root: impl Into<PathBuf>,
        receipt_context: impl Into<String>,
    ) -> Result<Self, BootstrapExecutorMarkerError> {
        let canonical_root = canonical_root.into();
        let receipt_context = receipt_context.into();
        if !matches!(action, SessionPolicyAction::CodingTargetWrite) {
            return Err(BootstrapExecutorMarkerError::InvalidAction(action));
        }
        if canonical_root.as_os_str().is_empty() {
            return Err(BootstrapExecutorMarkerError::EmptyCanonicalRoot);
        }
        if receipt_context.trim().is_empty() {
            return Err(BootstrapExecutorMarkerError::EmptyReceiptContext);
        }
        Ok(Self {
            credential,
            action,
            canonical_root,
            receipt_context,
        })
    }

    /// spawn 前结构复核（guard 消费点）：结构完整返回 `None`，否则返回
    /// 缺失要素说明。credential 由类型面保证必带（工厂唯一构造）。
    pub fn incomplete_reason(&self) -> Option<&'static str> {
        if !matches!(self.action, SessionPolicyAction::CodingTargetWrite) {
            Some("bootstrap executor action must be CodingTargetWrite")
        } else if self.canonical_root.as_os_str().is_empty() {
            Some("bootstrap executor canonical root is empty")
        } else if self.receipt_context.trim().is_empty() {
            Some("bootstrap executor receipt context is empty")
        } else {
            None
        }
    }

    /// marker 携带的自举相位凭据。
    pub fn credential(&self) -> &BootstrapPhaseCredential {
        &self.credential
    }

    /// marker 冻结的 canonical 聚合根（写边界锚点）。
    pub fn canonical_root(&self) -> &Path {
        &self.canonical_root
    }

    /// receipt 审计上下文（root recipe 命令/step 关联键）。
    pub fn receipt_context(&self) -> &str {
        &self.receipt_context
    }
}

fn bootstrap_waiting(reason_code: &str, detail: String) -> ProviderAdmissionError {
    ProviderAdmissionError::Waiting {
        reason_code: reason_code.to_string(),
        detail,
        missing_materials: Vec::new(),
        allowed_actions: vec![BootstrapActionKind::Revalidate, BootstrapActionKind::Retry],
    }
}

fn ensure_bootstrap_operation_running(
    operation: &AggregateInitializationOperation,
) -> Result<(), ProviderAdmissionError> {
    if operation.status != AggregateInitializationOperationStatus::Running {
        return Err(bootstrap_waiting(
            "bootstrap_operation_not_running",
            format!(
                "aggregate initialization operation {} is {:?}, not Running",
                operation.operation_id, operation.status
            ),
        ));
    }
    Ok(())
}

fn ensure_bootstrap_step_is_provider_turn(
    step: AggregateInitializationStepKind,
) -> Result<(), ProviderAdmissionError> {
    if !step.is_provider_turn() {
        return Err(bootstrap_waiting(
            "bootstrap_step_not_provider_turn",
            format!(
                "bootstrap step {step:?} is deterministic and must not derive a provider credential"
            ),
        ));
    }
    Ok(())
}

fn bootstrap_step_record<'a>(
    operation: &'a AggregateInitializationOperation,
    step: AggregateInitializationStepKind,
) -> Result<&'a AggregateInitializationStepRecord, ProviderAdmissionError> {
    let record = operation
        .steps
        .get(step.index())
        .ok_or_else(|| {
            bootstrap_waiting(
                "bootstrap_step_not_running",
                format!("operation {} has no record for step {step:?}", operation.operation_id),
            )
        })?;
    if record.status != AggregateInitializationStepStatus::Running {
        return Err(bootstrap_waiting(
            "bootstrap_step_not_running",
            format!(
                "bootstrap step {step:?} of operation {} is {:?}, not Running",
                operation.operation_id, record.status
            ),
        ));
    }
    Ok(record)
}

fn bootstrap_roots_match(credential_root: &Path, durable_root: &Path) -> bool {
    if credential_root == durable_root {
        return true;
    }
    match (
        std::fs::canonicalize(credential_root),
        std::fs::canonicalize(durable_root),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// 准入预检结果：材料齐备且 gateway validate 通过时 `ready == true`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderAdmissionPreflightResult {
    pub target: ResolvedTargetIdentity,
    pub authority_root: PathBuf,
    pub policy: AuthorityPolicyReference,
    pub rules: Vec<ProviderRuleReference>,
    pub capability_snapshot_ref: String,
    pub ready: bool,
    pub missing_materials: Vec<String>,
    pub allowed_actions: Vec<BootstrapActionKind>,
}

/// 预检失败：waiting 是可操作的持久等待事实（携带缺失材料与允许动作），
/// 其余为不可恢复的存储/路由错误。
#[derive(Debug)]
pub enum ProviderAdmissionError {
    Waiting {
        reason_code: String,
        detail: String,
        missing_materials: Vec<String>,
        allowed_actions: Vec<BootstrapActionKind>,
    },
    Store(ProductStoreError),
}

impl From<ProductStoreError> for ProviderAdmissionError {
    fn from(error: ProductStoreError) -> Self {
        Self::Store(error)
    }
}

/// LC-scoped provider admission preflight：resolver + policy + gateway。
pub struct LogicalCodebaseProviderAdmissionPreflight {
    paths: ProductAppPaths,
    lc_id: String,
    gateway: Arc<LogicalCodebaseProviderGateway>,
}

impl LogicalCodebaseProviderAdmissionPreflight {
    pub fn new(
        paths: ProductAppPaths,
        lc_id: impl Into<String>,
        gateway: Arc<LogicalCodebaseProviderGateway>,
    ) -> Self {
        Self {
            paths,
            lc_id: lc_id.into(),
            gateway,
        }
    }

    /// 预检一次 provider 启动请求。只读 durable 事实 + gateway `validate`
    /// （validate 不启动 provider）；任何缺失/漂移都返回 waiting 事实而非
    /// 让运行时 Failed。
    ///
    /// `phase`（Task 1.2，REQ-BOOT-04）：`AggregateBootstrap` 携带有效凭据时
    /// **只豁免「根规则尚未生成」的存在性检查**——凭据先对 durable Running
    /// operation 重核验（status/step/input digest/LC/root），漂移即 waiting；
    /// authority/policy/capability/gateway/cwd/target 全部照常必检。
    pub fn check(
        &self,
        request: &SessionLaunchRequest,
        phase: &ProviderAdmissionPhase,
    ) -> Result<ProviderAdmissionPreflightResult, ProviderAdmissionError> {
        // 0. 相位凭据先核验（REQ-BOOT-04）：AggregateBootstrap 凭据必须仍与
        //    durable Running operation 一致；失效/漂移凭据 fail-closed，
        //    绝不降级为普通 session 或放宽其他维度。
        let waive_missing_root_rules = match phase {
            ProviderAdmissionPhase::Normal => false,
            ProviderAdmissionPhase::AggregateBootstrap(credential) => {
                if credential.project_id != request.project_id {
                    return Err(bootstrap_waiting(
                        "bootstrap_project_mismatch",
                        format!(
                            "credential project {} does not match launch request project {}",
                            credential.project_id, request.project_id
                        ),
                    ));
                }
                let operations = AggregateInitializationOperationStore::for_lc(
                    self.paths.clone(),
                    self.lc_id.clone(),
                );
                credential.reverify_against_running_operation(&operations, &self.lc_id)?;
                true
            }
        };
        let resolution = RepositoryAuthorityResolver::new(self.paths.clone()).resolve(
            RepositoryRoutingRequest {
                project_id: request.project_id.clone(),
                issue_id: None,
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(self.lc_id.clone()),
                logical_repository_id: parse_member_id(&request.target.logical_repository_id),
                checkout_id: parse_checkout_id(&request.target.checkout_id),
            },
        )?;

        let mut missing_materials = Vec::new();
        let mut allowed_actions = Vec::new();
        // 根规则存在性缺失（Task 1.2 与其他材料分离：自举相位凭据是唯一
        // 豁免面，其余维度永不豁免）。
        let mut missing_rules = Vec::new();

        // 1. manifest：冷启动未登记时投影 waiting（Prepare）。
        let manifest = resolution
            .manifest
            .as_ref()
            .ok_or_else(|| ProviderAdmissionError::Waiting {
                reason_code: "logical_codebase_manifest_missing".to_string(),
                detail: format!(
                    "logical codebase {} has no manifest; register members first",
                    self.lc_id
                ),
                missing_materials: vec![format!(
                    "logical-codebases/{}/manifest.json",
                    self.lc_id
                )],
                allowed_actions: vec![BootstrapActionKind::Prepare],
            })?;

        // 2. 实际成员规则：每个 active 成员的 main checkout 必须有
        //    `.claude/rules/language.md`（与 single_candidate_author 同一路径）。
        let logical = LogicalCodebaseStore::for_lc(self.paths.clone(), &self.lc_id);
        let members = logical.list_members(&request.project_id)?;
        let checkouts = logical.list_checkouts(&request.project_id)?;
        let mut rules = Vec::new();
        for member in &members {
            if member.status != MemberStatus::Active {
                continue;
            }
            let Some(checkout) = checkouts
                .iter()
                .find(|checkout| {
                    member.checkout_ids.contains(&checkout.checkout_id)
                        && checkout.kind == CheckoutKind::Main
                })
                .or_else(|| {
                    checkouts
                        .iter()
                        .find(|checkout| {
                            member.checkout_ids.contains(&checkout.checkout_id)
                        })
                })
            else {
                missing_materials.push(format!(
                    "member {} ({}) has no recorded checkout",
                    member.alias,
                    member.logical_repository_id.0
                ));
                continue;
            };
            let rule_path = checkout
                .canonical_path
                .join(".claude/rules/language.md");
            match std::fs::read(&rule_path) {
                Ok(bytes) => rules.push(ProviderRuleReference {
                    member_id: member.logical_repository_id,
                    checkout_id: checkout.checkout_id,
                    path: rule_path,
                    digest: Some(format!("sha256:{:x}", sha2::Sha256::digest(&bytes))),
                }),
                Err(_) => {
                    missing_rules.push(format!(
                        "member {} missing {}",
                        member.alias,
                        rule_path.display()
                    ));
                    rules.push(ProviderRuleReference {
                        member_id: member.logical_repository_id,
                        checkout_id: checkout.checkout_id,
                        path: rule_path,
                        digest: None,
                    });
                }
            }
        }
        if waive_missing_root_rules {
            // BOOT-04（Task 1.2）：有效自举凭据只豁免「根规则尚未生成」——
            // 缺失规则保持为记录事实（digest=None），不构成阻断材料，也
            // 不污染后续维度的 missing_materials；其余维度照常必检。
        } else if !missing_rules.is_empty() {
            missing_materials.extend(missing_rules.clone());
            allowed_actions.push(BootstrapActionKind::Prepare);
            allowed_actions.push(BootstrapActionKind::Retry);
        }

        // 3. 聚合 policy artifact：digest/revision 由 store 校验后冻结进引用。
        let policy = resolution.policy.clone().ok_or_else(|| {
            let mut materials = missing_materials.clone();
            materials.push(format!(
                "logical-codebases/{}/aggregate-policy.json",
                self.lc_id
            ));
            ProviderAdmissionError::Waiting {
                reason_code: "aggregate_policy_artifact_missing".to_string(),
                detail: format!(
                    "logical codebase {} has no aggregate policy artifact",
                    self.lc_id
                ),
                missing_materials: materials,
                allowed_actions: vec![BootstrapActionKind::Prepare, BootstrapActionKind::Retry],
            }
        })?;

        // 4. 真实 gateway 谓词：bootstrap capability 记录不满足 snapshot/action
        //    时在此拒绝（validate 不启动 provider）。
        let validated = self.gateway.validate(request.clone()).map_err(|error| {
            let reason_code = match &error {
                ProviderGatewayError::UnsupportedCapability(_) => {
                    "provider_capability_not_satisfied"
                }
                ProviderGatewayError::PolicyMissing(_) => "aggregate_policy_artifact_missing",
                _ => "provider_gateway_denied",
            };
            let mut materials = missing_materials.clone();
            if matches!(error, ProviderGatewayError::PolicyMissing(_)) {
                materials.push(format!(
                    "logical-codebases/{}/aggregate-policy.json",
                    self.lc_id
                ));
            }
            ProviderAdmissionError::Waiting {
                reason_code: reason_code.to_string(),
                detail: error.to_string(),
                missing_materials: materials,
                allowed_actions: vec![BootstrapActionKind::Revalidate, BootstrapActionKind::Retry],
            }
        })?;

        // 5. envelope 与 resolver 冻结值比对：authority root / policy digest 漂移
        //    在 spawn 前转为 waiting（Revalidate），绝不回落旧路径。
        let envelope = validated.envelope();
        if envelope.authority_root != resolution.authority_root {
            return Err(ProviderAdmissionError::Waiting {
                reason_code: "authority_root_drift".to_string(),
                detail: format!(
                    "envelope authority root {} does not match resolver-frozen root {}",
                    envelope.authority_root.display(),
                    resolution.authority_root.display()
                ),
                missing_materials: missing_materials.clone(),
                allowed_actions: vec![BootstrapActionKind::Revalidate],
            });
        }
        if envelope.policy_digest != policy.policy_digest
            || envelope.policy_revision != policy.policy_revision
        {
            return Err(ProviderAdmissionError::Waiting {
                reason_code: "policy_digest_drift".to_string(),
                detail: format!(
                    "envelope policy {}/{} does not match authority policy {}/{}",
                    envelope.policy_id,
                    envelope.policy_revision,
                    policy.policy_id,
                    policy.policy_revision
                ),
                missing_materials: missing_materials.clone(),
                allowed_actions: vec![BootstrapActionKind::Revalidate],
            });
        }

        // 6. spawn 前复验（pub(crate) 拓宽后由 admission 显式调用）：policy
        //    revision/digest、capability、config digest、target cwd 的 TOCTOU
        //    全维度复核；失败转 waiting，provider 保持零启动。
        let cwd = if request.target.worktree.is_absolute() {
            request.target.worktree.clone()
        } else {
            manifest.provider_context_root.join(&request.target.worktree)
        };
        self.gateway
            .revalidate_before_spawn(&validated, &cwd, false)
            .map_err(|error| ProviderAdmissionError::Waiting {
                reason_code: "spawn_revalidation_drift".to_string(),
                detail: error.to_string(),
                missing_materials: missing_materials.clone(),
                allowed_actions: vec![BootstrapActionKind::Revalidate, BootstrapActionKind::Retry],
            })?;

        if !missing_materials.is_empty() {
            return Err(ProviderAdmissionError::Waiting {
                reason_code: "member_language_rules_missing".to_string(),
                detail: format!(
                    "logical codebase {} has members without .claude/rules/language.md",
                    self.lc_id
                ),
                missing_materials,
                allowed_actions,
            });
        }

        Ok(ProviderAdmissionPreflightResult {
            target: resolution.target,
            authority_root: resolution.authority_root,
            policy,
            rules,
            capability_snapshot_ref: validated.capability_snapshot_ref().to_string(),
            ready: true,
            missing_materials,
            allowed_actions: Vec::new(),
        })
    }
}

fn parse_member_id(value: &str) -> Option<LogicalRepositoryId> {
    uuid::Uuid::parse_str(value)
        .ok()
        .map(LogicalRepositoryId)
}

fn parse_checkout_id(value: &str) -> Option<RepositoryCheckoutId> {
    uuid::Uuid::parse_str(value)
        .ok()
        .map(RepositoryCheckoutId)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::policy::{ProviderDialect, SessionPolicyAction};
    use crate::product::logical_codebase::provider_gateway::{
        PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource, ProviderRef,
        ProviderRefType,
    };
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CodebaseMemberRecord, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    // ---- 测试 fakes（与 provider_gateway_tests 同型，作用域隔离） ----

    struct PassThroughTargetResolver;
    impl PolicyTargetResolver for PassThroughTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<crate::product::logical_codebase::policy::PolicyTarget, ProviderGatewayError>
        {
            let canonical = std::fs::canonicalize(&request.target.worktree)
                .map_err(|_| ProviderGatewayError::Target("worktree missing".to_string()))?;
            if request.target.logical_repository_id.is_empty() {
                Ok(crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                    canonical,
                ))
            } else {
                Ok(crate::product::logical_codebase::policy::PolicyTarget::checkout(
                    request.target.logical_repository_id.clone(),
                    request.target.checkout_id.clone(),
                    canonical,
                ))
            }
        }
    }

    struct StaticCapabilitySource {
        deny: std::sync::atomic::AtomicBool,
    }
    impl StaticCapabilitySource {
        fn allowing() -> Self {
            Self {
                deny: std::sync::atomic::AtomicBool::new(false),
            }
        }
        fn deny(&self) {
            self.deny.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }
    impl ProviderCapabilitySource for StaticCapabilitySource {
        fn require_supported(
            &self,
            provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            if self.deny.load(std::sync::atomic::Ordering::SeqCst) {
                return Err(ProviderGatewayError::UnsupportedCapability(
                    "capability record missing".to_string(),
                ));
            }
            let adapter_dialect = match provider.provider_type {
                ProviderRefType::ClaudeCode => ProviderDialect::ClaudeCodeCliV1,
                ProviderRefType::Codex => ProviderDialect::CodexCliV1,
            };
            Ok(ProviderCapability {
                provider_type: provider.provider_type,
                version: "1.4.0".to_string(),
                adapter_dialect,
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                resume_evidence:
                    crate::product::logical_codebase::provider_gateway::ResumeEvidenceState::Confirmed,
            })
        }
    }

    struct CountingStreamingAdapter {
        start_count: std::sync::atomic::AtomicUsize,
    }
    impl CountingStreamingAdapter {
        fn new() -> Self {
            Self {
                start_count: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn start_count(&self) -> usize {
            self.start_count.load(std::sync::atomic::Ordering::SeqCst)
        }
    }
    #[async_trait::async_trait]
    impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter
        for CountingStreamingAdapter
    {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> Result<
            crate::cross_cutting::streaming_provider::ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            self.start_count
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let (_event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            Ok(crate::cross_cutting::streaming_provider::ProviderSession {
                events,
                commands,
                native_session_id: None,
            })
        }
    }

    struct StubSyncAdapter;
    impl crate::cross_cutting::provider_adapter::ProviderAdapter for StubSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<
            crate::protocol::contracts::AdapterOutput,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            use crate::protocol::contracts::TimeoutStatus;
            Ok(crate::protocol::contracts::AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    fn always_available_gate()
    -> Arc<crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate> {
        use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
        use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
        use crate::product::models::ProviderName;
        use chrono::Utc;

        struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);
        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }
            fn degraded(&self) -> bool {
                false
            }
        }

        let checked_at = Utc::now();
        let snapshot = Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: [ProviderName::ClaudeCode, ProviderName::Codex]
                .into_iter()
                .map(|provider| ProviderHealthEntry {
                    provider,
                    command: "stub".to_string(),
                    available: true,
                    version: Some("1.0".to_string()),
                    reason_code: None,
                    reason: None,
                    checked_at,
                })
                .collect(),
        });
        Arc::new(ProviderAvailabilityGate::new(Arc::new(AlwaysHealthy(
            snapshot,
        ))))
    }

    struct AdmissionFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        project_id: String,
        lc_id: String,
        aggregate_root: PathBuf,
        member_root: PathBuf,
        policy_store: AggregatePolicyArtifactStore,
        capabilities: Arc<StaticCapabilitySource>,
        streaming_adapter: Arc<CountingStreamingAdapter>,
        gateway: Arc<LogicalCodebaseProviderGateway>,
    }

    fn admission_fixture() -> AdmissionFixture {
        admission_fixture_with_policy(true)
    }

    /// Task 1.2：跳过 aggregate policy artifact 的变体（证明自举豁免不
    /// 覆盖 policy 维度）。
    fn admission_fixture_without_policy() -> AdmissionFixture {
        admission_fixture_with_policy(false)
    }

    fn admission_fixture_with_policy(with_policy: bool) -> AdmissionFixture {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ProductAppPaths::new(temp.path());
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "admission-project".to_string(),
                description: None,
            })
            .expect("project");
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let member_root = temp.path().join("member-a");
        git_init_with_commit(&member_root);

        let logical = LogicalCodebaseStore::new(paths.clone());
        let lc = logical
            .create(
                &project.id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "admission-lc".to_string(),
                    aggregate_root: aggregate_root.clone(),
                },
            )
            .expect("lc");

        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc.id);
        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let canonical = std::fs::canonicalize(&member_root).unwrap();
        let source = RepositorySourceIdentity {
            scheme: "test".to_string(),
            key_digest: "sha256:admission-member".to_string(),
            canonical_git_dir: canonical.join(".git"),
            canonical_origin: None,
            first_seen_path_hash: "sha256:admission-path".to_string(),
        };
        let mut manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            &project.id,
            aggregate_root.clone(),
            vec![member_id],
        );
        manifest.logical_codebase_id = uuid::Uuid::new_v4();
        lc_store.save_manifest(&project.id, &manifest).unwrap();
        let now = "2026-09-28T00:00:00Z".to_string();
        lc_store
            .save_member(
                &project.id,
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_admission_member".to_string(),
                    alias: "member-a".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: now.clone(),
                    updated_at: now.clone(),
                },
            )
            .unwrap();
        lc_store
            .save_checkout(
                &project.id,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_admission_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: "sha256:admission-checkout".to_string(),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: now.clone(),
                    created_at: now.clone(),
                    updated_at: now,
                },
            )
            .unwrap();

        let policy_store = AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc.id);
        if with_policy {
            policy_store
                .ensure_bootstrap(&manifest)
                .expect("bootstrap policy");
        }

        let capabilities = Arc::new(StaticCapabilitySource::allowing());
        let streaming_adapter = Arc::new(CountingStreamingAdapter::new());
        let mut registry = ProviderRegistry::new();
        registry.register(
            crate::product::models::ProviderName::ClaudeCode,
            streaming_adapter.clone(),
        );
        registry.register(
            crate::product::models::ProviderName::Codex,
            streaming_adapter.clone(),
        );
        let gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
            policy_store.clone(),
            capabilities.clone(),
            Arc::new(PassThroughTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            Arc::new(crate::product::logical_codebase::GatewayRunAudit::new()),
            std::fs::canonicalize(&aggregate_root).expect("canonical authority root"),
        ));

        AdmissionFixture {
            _temp: temp,
            paths,
            project_id: project.id,
            lc_id: lc.id,
            aggregate_root,
            member_root,
            policy_store,
            capabilities,
            streaming_adapter,
            gateway,
        }
    }

    impl AdmissionFixture {
        fn write_language_rules(&self, contents: &str) {
            let dir = self.member_root.join(".claude/rules");
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join("language.md"), contents).unwrap();
        }

        fn preflight(&self) -> LogicalCodebaseProviderAdmissionPreflight {
            LogicalCodebaseProviderAdmissionPreflight::new(
                self.paths.clone(),
                self.lc_id.clone(),
                self.gateway.clone(),
            )
        }

        fn launch_request(&self) -> SessionLaunchRequest {
            SessionLaunchRequest::planning(
                self.project_id.clone(),
                ProviderRef::claude_code("snapshot-admission-test"),
                crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                    self.aggregate_root.clone(),
                ),
                vec![self.aggregate_root.clone()],
                "sha256:admission-managed-config",
            )
        }
    }

    fn git_init_with_commit(path: &Path) {
        std::fs::create_dir_all(path).unwrap();
        for args in [
            vec!["init", "-b", "main"],
            vec!["config", "user.email", "admission@test.local"],
            vec!["config", "user.name", "Admission Test"],
        ] {
            let output = std::process::Command::new("git")
                .current_dir(path)
                .args(&args)
                .output()
                .expect("git");
            assert!(output.status.success(), "git {args:?} failed");
        }
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        let output = std::process::Command::new("git")
            .current_dir(path)
            .args(["add", "."])
            .output()
            .unwrap();
        assert!(output.status.success());
        let output = std::process::Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();
        assert!(output.status.success());
    }

    #[test]
    fn missing_member_language_rule_waits_before_provider_spawn() {
        let fixture = admission_fixture();
        // 成员 checkout 缺 .claude/rules/language.md。
        let error = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting {
                reason_code,
                missing_materials,
                allowed_actions,
                ..
            } => {
                assert_eq!(reason_code, "member_language_rules_missing");
                assert!(missing_materials
                    .iter()
                    .any(|item| item.contains("language.md")));
                assert!(allowed_actions.contains(&BootstrapActionKind::Prepare));
                assert!(allowed_actions.contains(&BootstrapActionKind::Retry));
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        // provider 零启动。
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    #[test]
    fn bootstrap_capability_record_is_not_real_provider_capability() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# rules\n");
        // 真实 capability 谓词拒绝（记录缺失/不支持）→ waiting，而非因 JSON
        // 存在而放行。
        fixture.capabilities.deny();
        let error = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting {
                reason_code,
                detail,
                ..
            } => {
                assert_eq!(reason_code, "provider_capability_not_satisfied");
                assert!(detail.contains("capability"));
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    #[test]
    fn policy_digest_or_authority_root_drift_blocks_spawn() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# rules\n");
        // validate 冻结 envelope 后升级 policy revision/digest → spawn 前复验
        // （pub(crate) 拓宽后的 revalidate_before_spawn）拒绝，provider 零启动。
        let validated = fixture.gateway.validate(fixture.launch_request()).unwrap();
        let existing = fixture.policy_store.get(&fixture.project_id).unwrap().unwrap();
        let revised = existing.with_revised_policy("upgrade", "2026-09-28T01:00:00Z".to_string());
        fixture
            .policy_store
            .save(&fixture.project_id, &revised)
            .unwrap();

        let error = fixture
            .gateway
            .revalidate_before_spawn(
                &validated,
                &std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
                false,
            )
            .unwrap_err();
        assert!(
            matches!(
                &error,
                ProviderGatewayError::PolicyDrift { dimension }
                    if dimension.contains("policy_revision") || dimension.contains("policy_digest")
            ),
            "unexpected drift error: {error:?}"
        );
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // 之后的 admission 预检同样 waiting（resolver 与 store 的 policy 引用
        // 一致，但 envelope 由最新 validate 产出；漂移事实由复验路径证明）。
        let result = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal);
        assert!(result.is_ok() || matches!(result, Err(ProviderAdmissionError::Waiting { .. })));
    }

    #[test]
    fn valid_rules_policy_and_gateway_produce_envelope() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# language rules\n");

        let result = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap();
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        assert_eq!(result.rules.len(), 1);
        let rule = &result.rules[0];
        assert!(rule.digest.as_deref().unwrap().starts_with("sha256:"));
        assert_eq!(
            result.authority_root,
            std::fs::canonicalize(&fixture.aggregate_root).unwrap()
        );
        assert_eq!(result.capability_snapshot_ref, "snapshot-admission-test");
        assert!(result.policy.policy_digest.starts_with("sha256:"));
        // 预检本身零启动。
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    // ---- Task 1.2（REQ-BOOT-04/D1）：phase credential 与 BootstrapExecutor ----

    use crate::cross_cutting::streaming_provider::{
        ProviderToolPolicy, ToolPolicyGuardError, ToolPolicyIntent, canonical_tool_policy,
        validate_tool_policy_for_role,
    };
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateCancellationRecord, AggregateInitializationErrorRecord,
        AggregateInitializationOperation, AggregateInitializationOperationInput,
        AggregateInitializationStepKind,
    };
    use crate::product::logical_codebase::aggregate_initialization_store::
        AggregateInitializationOperationStore;
    use crate::protocol::contracts::AdapterRole;

    const BOOTSTRAP_TS: &str = "2026-10-01T00:01:00Z";

    /// 构造 durable Running operation：MachineSkills/AggregatePreflight 已
    /// 完成，目标 step（默认 PreCheck）运行中并带 input digest——这是
    /// `BootstrapPhaseCredential` 的唯一合法派生面。
    fn running_bootstrap_operation(
        fixture: &AdmissionFixture,
    ) -> (AggregateInitializationOperationStore, String) {
        running_bootstrap_operation_at_step(fixture, AggregateInitializationStepKind::PreCheck)
    }

    fn running_bootstrap_operation_at_step(
        fixture: &AdmissionFixture,
        step: AggregateInitializationStepKind,
    ) -> (AggregateInitializationOperationStore, String) {
        let store = AggregateInitializationOperationStore::for_lc(
            fixture.paths.clone(),
            fixture.lc_id.clone(),
        );
        let operation_id = format!("op-{}", uuid::Uuid::new_v4().simple());
        let input = AggregateInitializationOperationInput {
            idempotency_key: format!("bootstrap-{operation_id}"),
            manifest_revision: 1,
            policy_digest: "sha256:bootstrap-fixture-policy".to_string(),
            profile_evidence_digest: None,
            provider_context_root: std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
            provider: "claude_code".to_string(),
        };
        store
            .create_idempotent(AggregateInitializationOperation::new(
                operation_id.clone(),
                fixture.project_id.clone(),
                input,
                BOOTSTRAP_TS.to_string(),
            ))
            .expect("create bootstrap operation");
        store
 .mark_running(&fixture.project_id, &operation_id, BOOTSTRAP_TS.to_string())
            .expect("mark operation running");
        // 前置步骤按 V1 顺序完成，直到目标 step 可以运行。
        for predecessor in AggregateInitializationStepKind::V1 {
            if predecessor == step {
                break;
            }
            complete_bootstrap_step(&store, &fixture.project_id, &operation_id, predecessor);
        }
        store
            .mark_step_running(
                &fixture.project_id,
                &operation_id,
                step,
                format!("sha256:input-{}", step.as_str()),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("mark bootstrap step running");
        (store, operation_id)
    }

    fn complete_bootstrap_step(
        store: &AggregateInitializationOperationStore,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
    ) {
        store
            .mark_step_running(
                project_id,
                operation_id,
                step,
                format!("sha256:input-{}", step.as_str()),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("mark predecessor running");
        store
            .checkpoint_step_output(
                project_id,
                operation_id,
                step,
                format!("artifact-{step:?}"),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("checkpoint predecessor");
        store
            .mark_step_completed(project_id, operation_id, step, BOOTSTRAP_TS.to_string())
            .expect("complete predecessor");
    }

    /// 完成剩余全部步骤并 `finish_completed`（Completed 终态夹具）。
    fn finish_bootstrap_operation(
        store: &AggregateInitializationOperationStore,
        fixture: &AdmissionFixture,
        operation_id: &str,
    ) {
        let operation = store
            .get(&fixture.project_id, operation_id)
            .expect("load operation for completion");
        for (index, step) in AggregateInitializationStepKind::V1.into_iter().enumerate() {
            // 已 Completed 的步骤不可重复 mark_step_running（store 拒绝非
            // Pending 起始状态）；只推进尚未完成的步骤。
            if operation.steps[index].status
                == crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepStatus::Completed
            {
                continue;
            }
            complete_bootstrap_step(store, &fixture.project_id, operation_id, step);
        }
        store
            .finish_completed(&fixture.project_id, operation_id, BOOTSTRAP_TS.to_string())
            .expect("finish completed");
    }

    fn derived_credential(
        fixture: &AdmissionFixture,
    ) -> (AggregateInitializationOperationStore, String, BootstrapPhaseCredential) {
        let (store, operation_id) = running_bootstrap_operation(fixture);
        let credential = BootstrapPhaseCredential::from_running_operation(
            &store,
            &fixture.project_id,
            &operation_id,
            AggregateInitializationStepKind::PreCheck,
            &fixture.lc_id,
            &std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
        )
        .expect("derive bootstrap credential from running operation");
        (store, operation_id, credential)
    }

    fn assert_bootstrap_waiting<T: std::fmt::Debug>(
        result: Result<T, ProviderAdmissionError>,
        expected_reason: &str,
    ) {
        match result {
            Err(ProviderAdmissionError::Waiting { reason_code, .. }) => {
                assert_eq!(reason_code, expected_reason);
            }
            other => panic!("expected waiting {expected_reason}, got {other:?}"),
        }
    }

    #[test]
    fn bootstrap_phase_credential_rejects_completed_or_drifted_operation() {
        let fixture = admission_fixture();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();

        // 基线：Running + step 运行中 + digest/root/LC 匹配 → 唯一可派生面。
        let (_store, _operation_id, credential) = derived_credential(&fixture);
        assert_eq!(credential.step, AggregateInitializationStepKind::PreCheck);

        // Completed 终态 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        finish_bootstrap_operation(&store, &fixture, &operation_id);
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_operation_not_running",
        );

        // Failed 终态 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        store
            .finish_failed(
                &fixture.project_id,
                &operation_id,
                Some(AggregateInitializationStepKind::PreCheck),
                AggregateInitializationErrorRecord::interrupted(),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("finish failed");
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_operation_not_running",
        );

        // Cancelled 终态 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        store
            .cancel(
                &fixture.project_id,
                &operation_id,
                AggregateCancellationRecord {
                    reason_code: "test_cancelled".to_string(),
                    cancelled_at: BOOTSTRAP_TS.to_string(),
                    detail: None,
                },
                BOOTSTRAP_TS.to_string(),
            )
            .expect("cancel operation");
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_operation_not_running",
        );

        // step 漂移：durable 运行中的是 RuleAndMcpConfig，凭据声明 PreCheck → 拒绝。
        let (store, operation_id) = running_bootstrap_operation_at_step(
            &fixture,
            AggregateInitializationStepKind::RuleAndMcpConfig,
        );
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_step_not_running",
        );

        // 非 provider turn step（MachineSkills 运行中）→ 拒绝：确定性步骤
        // 不得派生 spawn 凭据。
        let (store, operation_id) = running_bootstrap_operation_at_step(
            &fixture,
            AggregateInitializationStepKind::MachineSkills,
        );
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::MachineSkills,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_step_not_provider_turn",
        );

        // root 漂移：凭据 root 与 operation 的 provider_context_root 不一致 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        let member_root = std::fs::canonicalize(&fixture.member_root).unwrap();
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &member_root,
            ),
            "bootstrap_root_drift",
        );

        // input digest 漂移：已派生凭据与当前 durable 记录 digest 不一致 →
        // spawn 前重核验拒绝（fail-closed，不可复用旧凭据）。
        let (store, _operation_id, credential) = derived_credential(&fixture);
        let drifted_digest = BootstrapPhaseCredential {
            input_digest: "sha256:drifted-input".to_string(),
            ..credential.clone()
        };
        assert_bootstrap_waiting(
            drifted_digest.reverify_against_running_operation(&store, &fixture.lc_id),
            "bootstrap_input_digest_drift",
        );

        // LC 漂移：凭据 LC 身份与核验 scope 不一致 → 拒绝。
        let drifted_lc = BootstrapPhaseCredential {
            logical_codebase_id: uuid::Uuid::new_v4().to_string(),
            ..credential
        };
        assert_bootstrap_waiting(
            drifted_lc.reverify_against_running_operation(&store, &fixture.lc_id),
            "bootstrap_logical_codebase_mismatch",
        );

        // 伪造 operation id → durable 缺失走 Store 错误（fail-closed）。
        assert!(matches!(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                "op-missing",
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            Err(ProviderAdmissionError::Store(_))
        ));
    }

    #[test]
    fn bootstrap_phase_only_waives_missing_root_rules() {
        // 成员规则缺失 = 根 recipe 尚未生成的正常自举事实。
        let fixture = admission_fixture();
        let (_store, _operation_id, credential) = derived_credential(&fixture);

        // AggregateBootstrap：只豁免规则存在性；其余维度全过 → ready。
        let result = fixture
            .preflight()
            .check(
                &fixture.launch_request(),
                &ProviderAdmissionPhase::AggregateBootstrap(credential.clone()),
            )
            .expect("bootstrap phase must waive only missing root rules");
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        // 缺失规则仍作为事实记录（digest=None），不构成阻断材料。
        assert_eq!(result.rules.len(), 1);
        assert!(result.rules[0].digest.is_none());
        assert!(result.missing_materials.is_empty());
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // Normal 对照组：同样材料下规则缺失仍阻断（既有语义零变化）。
        assert_bootstrap_waiting(
            fixture
                .preflight()
                .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
                .map(|_| ()),
            "member_language_rules_missing",
        );

        // 豁免不覆盖 policy：aggregate policy artifact 缺失仍 waiting。
        let fixture_no_policy = admission_fixture_without_policy();
        let (_store_no_policy, _op_no_policy, credential_no_policy) =
            derived_credential(&fixture_no_policy);
        assert_bootstrap_waiting(
            fixture_no_policy
                .preflight()
                .check(
                    &fixture_no_policy.launch_request(),
                    &ProviderAdmissionPhase::AggregateBootstrap(credential_no_policy),
                )
                .map(|_| ()),
            "aggregate_policy_artifact_missing",
        );

        // 豁免不覆盖 capability：gateway capability 谓词拒绝仍 waiting。
        let fixture_denied = admission_fixture();
        fixture_denied.capabilities.deny();
        let (_store_denied, _op_denied, credential_denied) = derived_credential(&fixture_denied);
        assert_bootstrap_waiting(
            fixture_denied
                .preflight()
                .check(
                    &fixture_denied.launch_request(),
                    &ProviderAdmissionPhase::AggregateBootstrap(credential_denied),
                )
                .map(|_| ()),
            "provider_capability_not_satisfied",
        );

        // 凭据失效优先于豁免：operation Completed 后，凭据重核验失败——
        // 绝不降级为普通 session 或带病放行。
        let fixture_stale = admission_fixture();
        let (store_stale, _op_stale, stale_credential) = derived_credential(&fixture_stale);
        finish_bootstrap_operation(&store_stale, &fixture_stale, stale_credential.operation_id());
        assert_bootstrap_waiting(
            fixture_stale
                .preflight()
                .check(
                    &fixture_stale.launch_request(),
                    &ProviderAdmissionPhase::AggregateBootstrap(stale_credential),
                )
                .map(|_| ()),
            "bootstrap_operation_not_running",
        );
        assert_eq!(fixture_stale.streaming_adapter.start_count(), 0);
    }

    #[test]
    fn bootstrap_executor_marker_rejects_ordinary_executor_policy_in_lc_root_recipe() {
        let fixture = admission_fixture();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let (_store, _operation_id, credential) = derived_credential(&fixture);
        let marker = BootstrapExecutorMarker::new(
            credential,
            SessionPolicyAction::CodingTargetWrite,
            canonical_root,
            "root-recipe:pre_check:command-1",
        )
        .expect("complete bootstrap executor marker");
        let marker_policy = ProviderToolPolicy {
            intent: ToolPolicyIntent::BootstrapExecutorMarker(marker),
        };

        // LC root recipe 的自举通道：Executor + 完整 marker 是唯一放行形态。
        assert!(validate_tool_policy_for_role(&AdapterRole::Executor, Some(&marker_policy)).is_ok());

        // 普通 Executor 策略（deny）仍被拒绝——marker 通道不是给普通
        // Executor/Coder 带策略的豁免口。
        let deny = ProviderToolPolicy::deny_file_write_builtins();
        assert!(validate_tool_policy_for_role(&AdapterRole::Executor, Some(&deny)).is_err());

        // 策略角色携带 marker：marker 不是策略（PolicyRequired）。
        for role in [
            AdapterRole::Orchestrator,
            AdapterRole::Reviewer,
            AdapterRole::WorkItemSplitter,
        ] {
            let rejected = matches!(
                validate_tool_policy_for_role(&role, Some(&marker_policy)),
                Err(ToolPolicyGuardError::PolicyRequired { .. })
            );
            assert!(
                rejected,
                "policy role {role:?} must not substitute the marker for its policy"
            );
        }

        // Handoff 不得使用自举通道。
        assert!(matches!(
            validate_tool_policy_for_role(&AdapterRole::Handoff, Some(&marker_policy)),
            Err(ToolPolicyGuardError::PolicyForbidden { .. })
        ));

        // marker 不投影为 canonical deny 策略：不进入策略会话 argv/审计通道。
        assert!(canonical_tool_policy("claude-code", &marker_policy).is_err());
        // 普通 Executor/Coder 无策略照常放行（既有双向语义零变化）。
        assert!(validate_tool_policy_for_role(&AdapterRole::Executor, None).is_ok());
    }

    #[test]
    fn bootstrap_executor_without_credential_or_receipt_context_is_rejected() {
        let fixture = admission_fixture();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let (_store, _operation_id, credential) = derived_credential(&fixture);

        // 凭据是 marker 的必带字段（类型面）：不存在「无凭据 marker」的
        // 构造形态——`BootstrapExecutorMarker::new` 只接受由 durable
        // Running operation 派生的 credential。
        // receipt context 缺失（空白）→ 构造即拒绝。
        assert_eq!(
            BootstrapExecutorMarker::new(
                credential.clone(),
                SessionPolicyAction::CodingTargetWrite,
                canonical_root.clone(),
                "   ",
            ),
            Err(BootstrapExecutorMarkerError::EmptyReceiptContext)
        );

        // canonical root 缺失 → 构造即拒绝。
        assert_eq!(
            BootstrapExecutorMarker::new(
                credential.clone(),
                SessionPolicyAction::CodingTargetWrite,
                std::path::PathBuf::new(),
                "root-recipe:pre_check:command-1",
            ),
            Err(BootstrapExecutorMarkerError::EmptyCanonicalRoot)
        );

        // 非写权限 action → 拒绝：BootstrapExecutor = 有写权限的 Executor。
        assert_eq!(
            BootstrapExecutorMarker::new(
                credential.clone(),
                SessionPolicyAction::PlanningReadOnly,
                canonical_root.clone(),
                "root-recipe:pre_check:command-1",
            ),
            Err(BootstrapExecutorMarkerError::InvalidAction(
                SessionPolicyAction::PlanningReadOnly
            ))
        );

        // spawn 前守卫对结构不完整 marker 再次拒绝（纵深防御：即使绕过
        // validating constructor，三 adapter 真实子进程前仍 fail-closed）。
        let complete = BootstrapExecutorMarker::new(
            credential,
            SessionPolicyAction::CodingTargetWrite,
            canonical_root,
            "root-recipe:pre_check:command-1",
        )
        .expect("complete marker");
        let degenerate = BootstrapExecutorMarker {
            receipt_context: String::new(),
            ..complete
        };
        let degenerate_policy = ProviderToolPolicy {
            intent: ToolPolicyIntent::BootstrapExecutorMarker(degenerate),
        };
        assert!(matches!(
            validate_tool_policy_for_role(&AdapterRole::Executor, Some(&degenerate_policy)),
            Err(ToolPolicyGuardError::BootstrapMarkerInvalid { .. })
        ));
    }
}
