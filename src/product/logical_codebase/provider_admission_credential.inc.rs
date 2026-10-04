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
    /// 显式新会话启动(Task 9a:resume 指纹漂移 supersede 后的唯一稳定
    /// 等待动作;由现有 `WsInMessage::StartGeneration` /
    /// `WorkspaceEngine::start_generation` 接受)。
    StartGeneration,
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
            Self::StartGeneration => "start_generation",
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
                format!("bootstrap step {step:?} of operation {operation_id} has no input digest"),
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

    /// marker 携带的写权限 action（guard/审计消费点）。
    pub fn action(&self) -> crate::product::logical_codebase::policy::SessionPolicyAction {
        self.action
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
