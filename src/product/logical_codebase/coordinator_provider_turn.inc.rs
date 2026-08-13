/// 默认托管配置 artifact 引用,经 gateway envelope 的 config_digest 复验。
const AGGREGATE_CONFIG_ARTIFACT_REF: &str = "sha256:aggregate-initialization-managed-config";

/// Task 16:gateway-backed provider turn 驱动。
///
/// 三个 provider turn(`pre_check`/`rule_and_mcp_config`/`openspec_and_examples`)
/// 经 [`LogicalCodebaseProviderGateway::start_streaming`] 启动(feature gate):
/// 当注入此驱动作为 coordinator 的 `AggregateProviderTurnDriver` 时,每个 turn
/// 都会在共享的 [`GatewayRunAudit`] 累加一次 `stream_launches()` 记录,使「聚合
/// provider turn 唯一经 gateway 启动」成为可审计事实而非仅靠代码审查。
///
/// 聚合根是 canonical non-Git aggregate root(聚合初始化 envelope 配置);三个
/// turn 的 cwd 均为该根,配置来自托管配置 artifact。该驱动绝不依赖单仓持久化
/// 层、单仓注册协调器或单仓 git 终结点,故聚合模式不会进入成员仓 git 调用图。
/// 该隔离契约由 `aggregate_coordinator_isolation` 测试在编译期锁定。
pub struct GatewayBackedAggregateProviderTurnDriver {
    gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    provider: crate::product::logical_codebase::ProviderRef,
}

impl GatewayBackedAggregateProviderTurnDriver {
    /// 用 Claude Code dialect 与给定 capability snapshot ref 构造驱动。聚合
    /// 初始化当前固定使用 Claude Code 作为唯一逻辑 provider(Codex 在
    /// `danger-full-access` 下被 gateway 路由级阻断)。
    pub fn claude_code(
        gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
        capability_snapshot_ref: impl Into<String>,
    ) -> Self {
        Self {
            gateway,
            provider: crate::product::logical_codebase::ProviderRef::claude_code(
                capability_snapshot_ref,
            ),
        }
    }

    /// 把单个 provider turn 组装成经 gateway 启动的 streaming 请求。read-only
    /// planning action(聚合根不写成员仓),target 锚定聚合根本身。
    fn launch_request(
        &self,
        project_id: &str,
        aggregate_root: &std::path::Path,
    ) -> crate::product::logical_codebase::SessionLaunchRequest {
        use crate::product::logical_codebase::{PolicyTarget, SessionPolicyAction};
        let target = PolicyTarget::aggregate_root(aggregate_root.to_path_buf());
        crate::product::logical_codebase::SessionLaunchRequest {
            project_id: project_id.to_string(),
            provider: self.provider.clone(),
            action: SessionPolicyAction::PlanningReadOnly,
            target,
            readable_roots: vec![aggregate_root.to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: AGGREGATE_CONFIG_ARTIFACT_REF.to_string(),
        }
    }

    fn streaming_input(
        &self,
        step: AggregateInitializationStepKind,
        aggregate_root: &std::path::Path,
    ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
        use crate::cross_cutting::streaming_provider::{
            ProviderPermissionMode, StreamingProviderInput,
        };
        use crate::protocol::contracts::{AdapterRole, ProviderType};
        StreamingProviderInput {
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Executor,
            prompt: format!("aggregate initialization turn: {}", step.as_str()),
            working_dir: aggregate_root.to_path_buf(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: std::collections::BTreeMap::new(),
            timeout_secs: 1,
        }
    }
}

#[async_trait]
impl AggregateProviderTurnDriver for GatewayBackedAggregateProviderTurnDriver {
    async fn run_turn(
        &self,
        project_id: &str,
        _operation_id: &str,
        step: AggregateInitializationStepKind,
        preflight: &AggregatePreflightSnapshot,
        cancellation: CancellationToken,
    ) -> Result<String, AggregateInitializationError> {
        use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
        let aggregate_root = std::path::PathBuf::from(&preflight.aggregate_root);
        let request = self.launch_request(project_id, &aggregate_root);
        let validated = self.gateway.validate(request).map_err(|error| {
            AggregateInitializationError::ProviderTurn {
                step,
                reason: format!("gateway validate failed: {error}"),
                retryable: true,
            }
        })?;
        let input = self.streaming_input(step, &aggregate_root);
        let launch = ValidatedStreamingProviderInput::new(input, validated);
        self.gateway
            .start_streaming(launch, cancellation)
            .await
            .map_err(|error| AggregateInitializationError::ProviderTurn {
                step,
                reason: format!("gateway start_streaming failed: {error}"),
                retryable: true,
            })?;
        Ok(format!("{} via gateway", step.as_str()))
    }
}

/// Task 16:聚合 asset 发布器。三个 provider turn 产出的聚合 artifact 只允许发布到
/// `.aria/aggregate/**`,禁止任何成员仓路径,使「聚合模式不进成员仓 git」成为可
/// 验证契约。发布的相对路径以正斜杠分隔;`published_paths()` 返回发布顺序供审计。
#[derive(Debug, Default)]
pub struct AggregateAssetPublisher {
    published: std::sync::Mutex<Vec<String>>,
}

impl AggregateAssetPublisher {
    pub fn new() -> Self {
        Self::default()
    }

    /// 发布一个聚合 asset 的相对路径。只允许 `.aria/aggregate/**`;其余路径
    /// (成员仓、父目录逃逸、绝对路径)一律 fail-closed。
    pub fn publish(
        &self,
        operation_id: &str,
        relative_path: &str,
    ) -> Result<(), AggregateInitializationError> {
        validate_relative_id(operation_id).map_err(|error| {
            AggregateInitializationError::state(
                operation_id,
                format!("invalid operation id: {error}"),
            )
        })?;
        if !Self::is_aggregate_asset_path(relative_path) {
            return Err(AggregateInitializationError::state(
                operation_id,
                format!("aggregate asset publisher rejects non-aggregate path: {relative_path}"),
            ));
        }
        self.published
            .lock()
            .expect("aggregate asset publisher mutex poisoned")
            .push(relative_path.to_string());
        Ok(())
    }

    /// 已发布的聚合 asset 相对路径,按发布顺序。
    pub fn published_paths(&self) -> Vec<String> {
        self.published
            .lock()
            .expect("aggregate asset publisher mutex poisoned")
            .clone()
    }

    /// 判定相对路径是否落在 `.aria/aggregate/**` 内。必须以 `.aria/aggregate/` 起
    /// 头,禁止空、禁止 `..` 段、禁止绝对路径前缀。
    fn is_aggregate_asset_path(relative_path: &str) -> bool {
        if relative_path.is_empty() {
            return false;
        }
        let normalized = std::path::Path::new(relative_path);
        if normalized.is_absolute() {
            return false;
        }
        if normalized.components().any(|component| {
            matches!(
                component,
                std::path::Component::ParentDir | std::path::Component::RootDir
            )
        }) {
            return false;
        }
        let mut components = normalized.components();
        matches!(components.next(), Some(std::path::Component::Normal(a)) if a == ".aria")
            && matches!(components.next(), Some(std::path::Component::Normal(b)) if b == "aggregate")
            && components.next().is_some()
    }
}
