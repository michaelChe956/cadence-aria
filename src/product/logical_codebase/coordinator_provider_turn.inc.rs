/// 默认托管配置 artifact 引用,经 gateway envelope 的 config_digest 复验。
const AGGREGATE_CONFIG_ARTIFACT_REF: &str = "sha256:aggregate-initialization-managed-config";

/// 单个 provider turn 输出摘要的累积上限（字节）：durable checkpoint 引用只
/// 保留有界摘要，完整输出仍归属 provider 会话侧（与单仓 `LimitedOutput`
/// 同一截断语义的聚合侧实现）。
const AGGREGATE_TURN_OUTPUT_LIMIT: usize = 4096;

/// Task 16 + Task 1.4:gateway-backed root recipe provider turn 驱动。
///
/// 三个 provider turn(`pre_check`/`rule_and_mcp_config`/`openspec_and_examples`)
/// 经 [`LogicalCodebaseProviderGateway::start_streaming`] 启动(feature gate):
/// 当注入此驱动作为 coordinator 的 `AggregateProviderTurnDriver` 时,每个 turn
/// 都会在共享的 [`GatewayRunAudit`] 累加一次 `stream_launches()` 记录,使「聚合
/// provider turn 唯一经 gateway 启动」成为可审计事实而非仅靠代码审查。
///
/// Task 1.4 起每个 turn 携带真实 root recipe 命令(四条无中断命令的固定映射,
/// 文本唯一来源 `RepositoryInitializationStepKind::command()`),并以单仓初始化
/// 同款的取消/超时/输出摘要语义消费会话事件直到 Completed;命令失败、取消或
/// 超时都转为可重试 provider turn 失败,由 coordinator 保留 durable 事实并让
/// 后续命令保持 Pending。
///
/// 聚合根是 canonical non-Git aggregate root(聚合初始化 envelope 配置);三个
/// turn 的 cwd 均为该根,配置来自托管配置 artifact。该驱动绝不依赖单仓持久化
/// 层、单仓注册协调器或单仓 git 终结点,故聚合模式不会进入成员仓 git 调用图。
/// 该隔离契约由 `aggregate_coordinator_isolation` 测试在编译期锁定。
pub struct GatewayBackedAggregateProviderTurnDriver {
    gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    provider: crate::product::logical_codebase::ProviderRef,
    /// C4 Task 8：Some(paths) 时，每个 provider turn 在 gateway validate 之前
    /// 先经 `LogicalCodebaseProviderAdmissionPreflight`（实际成员规则、policy
    /// digest/authority root 与 capability 谓词预检）；None 保持原行为。
    admission_paths: Option<crate::product::app_paths::ProductAppPaths>,
    /// Task 1.4：每条 root recipe 命令的超时预算。
    command_timeout: std::time::Duration,
}

impl GatewayBackedAggregateProviderTurnDriver {
    pub(crate) const DEFAULT_COMMAND_TIMEOUT_SECS: u64 = 1800;

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
            admission_paths: None,
            command_timeout: std::time::Duration::from_secs(Self::DEFAULT_COMMAND_TIMEOUT_SECS),
        }
    }

    /// C4 Task 8：带 admission 预检的构造——provider turn 前预检实际成员
    /// `.claude/rules/language.md`、policy artifact 与 gateway capability，
    /// 缺失/漂移时在 spawn 前返回 waiting 类错误，provider 保持零启动。
    pub fn claude_code_with_admission(
        gateway: Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
        capability_snapshot_ref: impl Into<String>,
        paths: crate::product::app_paths::ProductAppPaths,
    ) -> Self {
        Self {
            admission_paths: Some(paths),
            ..Self::claude_code(gateway, capability_snapshot_ref)
        }
    }

    /// Task 1.4：注入每条命令的超时预算（测试用短超时；生产保持
    /// [`DEFAULT_COMMAND_TIMEOUT_SECS`]）。
    pub fn with_command_timeout(mut self, command_timeout: std::time::Duration) -> Self {
        self.command_timeout = command_timeout;
        self
    }

    /// Task 1.4（REQ-BOOT-03）：五步 root recipe 的三个 provider turn 到四条
    /// 无中断命令的固定映射。命令文本唯一来源是
    /// `RepositoryInitializationStepKind::command()`（Task 1.1 隔离锁显式豁免
    /// 该消费）：`PreCheck`=命令 1，`RuleAndMcpConfig`=命令 2+3（同一 Claude
    /// turn 内顺序执行），`OpenspecAndExamples`=命令 4。确定性 step 不映射任何
    /// 命令（空表=fail-closed，调用方拒绝该 turn）。
    fn recipe_commands(step: AggregateInitializationStepKind) -> Vec<&'static str> {
        use crate::product::repository_store::RepositoryInitializationStepKind as RepoStep;
        match step {
            AggregateInitializationStepKind::PreCheck => [RepoStep::PreCheck]
                .into_iter()
                .filter_map(|kind| kind.command())
                .collect(),
            AggregateInitializationStepKind::RuleAndMcpConfig => {
                [RepoStep::RuleConfig, RepoStep::McpConfiguration]
                    .into_iter()
                    .filter_map(|kind| kind.command())
                    .collect()
            }
            AggregateInitializationStepKind::OpenspecAndExamples => {
                [RepoStep::ProjectRulesExamples]
                    .into_iter()
                    .filter_map(|kind| kind.command())
                    .collect()
            }
            _ => Vec::new(),
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
            // Task 2.5：独立 cwd 字段；recipe 命令的 cwd 固定聚合根（=target）。
            working_directory: aggregate_root.to_path_buf(),
            readable_roots: vec![aggregate_root.to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: AGGREGATE_CONFIG_ARTIFACT_REF.to_string(),
        }
    }

    /// Task 3.5 carry ①（BOOT-04/D1，Task 1.8 §五.1）：把 admission 凭据升格
    /// 为 spawn 输入上的 `BootstrapExecutorMarker`——「有写权限的 Executor」
    /// 在 `ProviderToolPolicy` 通道上的唯一显式载体。receipt context 冻结
    /// operation 与固定命令索引的关联键（与 receipt auditor 的
    /// `root_recipe_command_index` 同源）；四要素缺一即 fail-closed，绝不
    /// 退化为无 marker 的普通 Executor。
    fn bootstrap_executor_tool_policy(
        bootstrap: crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        aggregate_root: &std::path::Path,
    ) -> Result<
        crate::cross_cutting::streaming_provider::ProviderToolPolicy,
        AggregateInitializationError,
    > {
        use crate::cross_cutting::streaming_provider::{ProviderToolPolicy, ToolPolicyIntent};
        use crate::product::logical_codebase::policy::SessionPolicyAction;
        use crate::product::logical_codebase::provider_admission_preflight::BootstrapExecutorMarker;

        let commands: Vec<usize> =
            crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index()
                .into_iter()
                .filter(|(command_step, _, _)| *command_step == step)
                .map(|(_, index, _)| index)
                .collect();
        let receipt_context = format!(
            "root-recipe:{operation_id}:commands-{}",
            commands
                .iter()
                .map(usize::to_string)
                .collect::<Vec<_>>()
                .join("+")
        );
        let marker = BootstrapExecutorMarker::new(
            bootstrap,
            SessionPolicyAction::CodingTargetWrite,
            aggregate_root.to_path_buf(),
            receipt_context,
        )
        .map_err(|error| AggregateInitializationError::ProviderTurn {
            step,
            reason: format!("bootstrap executor marker rejected: {error}"),
            retryable: false,
        })?;
        Ok(ProviderToolPolicy {
            intent: ToolPolicyIntent::BootstrapExecutorMarker(marker),
        })
    }

    pub(crate) fn streaming_input(
        &self,
        step: AggregateInitializationStepKind,
        aggregate_root: &std::path::Path,
        tool_policy: Option<crate::cross_cutting::streaming_provider::ProviderToolPolicy>,
    ) -> crate::cross_cutting::streaming_provider::StreamingProviderInput {
        use crate::cross_cutting::streaming_provider::{
            ProviderPermissionMode, StreamingProviderInput,
        };
        use crate::protocol::contracts::{AdapterRole, ProviderType};
        StreamingProviderInput {
            working_directory: None,
            baseline_tree: None,
            // Task 3.5 carry ①：LC 根 recipe 自举 turn 携带
            // BootstrapExecutorMarker（Executor 的唯一自举通道）；无凭据的
            // 调用面保持 `None`（普通 Executor 不得携带任何 tool policy）。
            tool_policy,
            audit_sink: None,
            provider_type: ProviderType::ClaudeCode,
            role: AdapterRole::Executor,
            // Task 1.4：真实 root recipe 命令（固定顺序，来源
            // `RepositoryInitializationStepKind::command()`），替换占位串。
            prompt: Self::recipe_commands(step).join("\n"),
            working_dir: aggregate_root.to_path_buf(),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: ProviderPermissionMode::Auto,
            structured_output_contract: None,
            env_vars: std::collections::BTreeMap::new(),
            // Task 1.4：真实命令超时，替换 1s 占位。
            timeout_secs: self.command_timeout.as_secs().max(1),
        }
    }

    /// Task 1.4：以单仓初始化命令同款语义消费会话——取消/超时共用命令预算，
    /// 事件流累积有界输出摘要；Completed 返回摘要，Failed/ProtocolError/
    /// 超时/取消/流提前关闭转为可重试失败（交互请求除外：fail-closed 不可
    /// 自动重试）。
    async fn consume_turn(
        &self,
        mut session: crate::cross_cutting::streaming_provider::ProviderSession,
        step: AggregateInitializationStepKind,
        remaining: std::time::Duration,
        cancellation: CancellationToken,
    ) -> Result<String, AggregateInitializationError> {
        use crate::cross_cutting::streaming_provider::ProviderEvent::Completed;
        use crate::cross_cutting::streaming_provider::{ProviderEvent, ProviderStatus};

        let mut output = BoundedOutput::new();
        let timeout = tokio::time::sleep(remaining);
        tokio::pin!(timeout);
        loop {
            let event = tokio::select! {
                _ = cancellation.cancelled() => {
                    best_effort_abort(&session);
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: "aggregate recipe command cancelled".to_string(),
                        retryable: true,
                    });
                }
                _ = &mut timeout => {
                    best_effort_abort(&session);
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: "aggregate recipe command timed out".to_string(),
                        retryable: true,
                    });
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
                Some(Completed(completion)) => {
                    if output.is_empty() {
                        output.push(&completion.full_output);
                    }
                    return Ok(output.summary());
                }
                Some(ProviderEvent::Failed { message }) => {
                    output.push(&message);
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: format!("provider reported failure: {}", output.summary()),
                        retryable: true,
                    });
                }
                Some(ProviderEvent::ProtocolError { code, message, .. }) => {
                    output.push(&format!("{code}: {message}"));
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: format!("provider protocol error: {}", output.summary()),
                        retryable: true,
                    });
                }
                Some(ProviderEvent::PermissionTimeout { permission_id }) => {
                    output.push(&format!("permission request {permission_id} timed out"));
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: "provider permission timeout".to_string(),
                        retryable: true,
                    });
                }
                Some(ProviderEvent::StatusChanged(ProviderStatus::Failed)) => {
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: "provider status failed".to_string(),
                        retryable: true,
                    });
                }
                Some(ProviderEvent::StatusChanged(ProviderStatus::Aborted)) => {
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: "provider status aborted".to_string(),
                        retryable: true,
                    });
                }
                Some(ProviderEvent::PermissionRequest(_))
                | Some(ProviderEvent::ChoiceRequest(_)) => {
                    best_effort_abort(&session);
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: format!("provider interaction required: {}", output.summary()),
                        retryable: false,
                    });
                }
                Some(ProviderEvent::StatusChanged(_))
                | Some(ProviderEvent::ToolCall(_))
                | Some(ProviderEvent::UsageReport(_))
                | Some(ProviderEvent::ToolPolicyDecision(_))
                | Some(ProviderEvent::ToolPolicyWarning(_))
                | Some(ProviderEvent::ToolPolicyTerminated(_)) => {}
                None => {
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: "provider event stream closed before completion".to_string(),
                        retryable: true,
                    });
                }
            }
        }
    }
}

#[async_trait]
impl AggregateProviderTurnDriver for GatewayBackedAggregateProviderTurnDriver {
    async fn run_turn(
        &self,
        request: AggregateProviderTurnRequest<'_>,
    ) -> Result<String, AggregateInitializationError> {
        use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;

        let AggregateProviderTurnRequest {
            project_id,
            operation_id,
            step,
            preflight,
            lc_id: _lc_id,
            bootstrap,
            cancellation,
        } = request;

        // 确定性 step 不得进入 provider turn：固定映射为空即 fail-closed。
        let commands = Self::recipe_commands(step);
        if commands.is_empty() {
            return Err(AggregateInitializationError::ProviderTurn {
                step,
                reason: "deterministic aggregate steps must not run a provider turn".to_string(),
                retryable: false,
            });
        }
        let aggregate_root = std::path::PathBuf::from(&preflight.aggregate_root);
        // Task 3.5 carry ①：spawn 输入携带 BootstrapExecutorMarker（credential
        // 副本升格；原凭据仍供下方 admission 相位重核验消费）。
        let tool_policy = Self::bootstrap_executor_tool_policy(
            bootstrap.clone(),
            operation_id,
            step,
            &aggregate_root,
        )?;
        let request = self.launch_request(project_id, &aggregate_root);
        // C4 Task 8 / Task 1.4：真实材料 admission 预检先于 gateway validate——
        // 成员规则缺失、policy 漂移或 capability 不满足时，在此 fail-closed，
        // provider 保持零启动，而不是把缺材料暴露成运行时 Failed。Task 1.4 起
        // root recipe turn 一律以 AggregateBootstrap 相位预检：凭据先对 durable
        // Running operation 重核验，仅豁免「根规则尚未生成」存在性检查，
        // authority/policy/capability/gateway/cwd/target 照常必检。
        if let (Some(paths), Some(lc_id)) = (&self.admission_paths, _lc_id) {
            let admission =
                crate::product::logical_codebase::LogicalCodebaseProviderAdmissionPreflight::new(
                    paths.clone(),
                    lc_id,
                    self.gateway.clone(),
                );
            let phase = crate::product::logical_codebase::provider_admission_preflight::ProviderAdmissionPhase::AggregateBootstrap(bootstrap);
            if let Err(error) = admission.check(&request, &phase) {
                return Err(AggregateInitializationError::ProviderTurn {
                    step,
                    reason: format!("provider admission preflight denied: {error:?}"),
                    retryable: true,
                });
            }
        }
        let input = self.streaming_input(step, &aggregate_root, Some(tool_policy));
        // Task 1b 段③:admission paths 在场(生产主路径)时,provider turn 经
        // gateway `prepare_streaming_launch` 组装 prepared launch——prepare 前
        // 绑定 run-bound audit sink(BootstrapExecutor marker 校验/角色策略
        // guard 在 prepare 内统一施加);paths 缺席(legacy 别名解析失败的
        // 降级组装/测试构造)保持原 validate+new 路径(无审计旧路径,
        // 与 production_dependencies 的降级语义一致)。
        let launch = match self.admission_paths.as_ref() {
            Some(paths) => {
                let lifecycle = crate::product::lifecycle_store::LifecycleStore::new(paths.clone());
                let workspace_session_id = format!("aggregate-{project_id}-{operation_id}");
                let role_run_seq = lifecycle
                    .next_tool_policy_role_run_seq(&workspace_session_id)
                    .map_err(|error| AggregateInitializationError::ProviderTurn {
                        step,
                        reason: format!("tool_policy_role_run_seq_alloc_failed: {error}"),
                        retryable: true,
                    })?;
                let context =
                    crate::product::logical_codebase::provider_gateway::ProviderLaunchAuditContext {
                        workspace_session_id,
                        role_run_seq,
                        audit_sink: std::sync::Arc::new(lifecycle),
                    };
                self.gateway
                    .prepare_streaming_launch(input, request, context)
                    .map_err(|error| AggregateInitializationError::ProviderTurn {
                        step,
                        reason: format!("gateway prepare_streaming_launch failed: {error}"),
                        retryable: true,
                    })?
            }
            None => {
                let validated = self.gateway.validate(request).map_err(|error| {
                    AggregateInitializationError::ProviderTurn {
                        step,
                        reason: format!("gateway validate failed: {error}"),
                        retryable: true,
                    }
                })?;
                ValidatedStreamingProviderInput::new(input, validated)
            }
        };
        // Task 1.4：复用单仓初始化命令的取消/超时/摘要语义——启动与事件
        // 消费共享同一命令超时预算。
        let command_timeout = self.command_timeout;
        let started = std::time::Instant::now();
        let start = self.gateway.start_streaming(launch, cancellation.clone());
        tokio::pin!(start);
        let timeout = tokio::time::sleep(command_timeout);
        tokio::pin!(timeout);
        let session = tokio::select! {
            _ = cancellation.cancelled() => {
                return Err(AggregateInitializationError::ProviderTurn {
                    step,
                    reason: "aggregate recipe command cancelled".to_string(),
                    retryable: true,
                });
            }
            _ = &mut timeout => {
                return Err(AggregateInitializationError::ProviderTurn {
                    step,
                    reason: "aggregate recipe command timed out before session start".to_string(),
                    retryable: true,
                });
            }
            result = &mut start => result.map_err(|error| {
                AggregateInitializationError::ProviderTurn {
                    step,
                    reason: format!("gateway start_streaming failed: {error}"),
                    retryable: true,
                }
            })?,
        };
        let remaining = command_timeout.saturating_sub(started.elapsed());
        self.consume_turn(session, step, remaining, cancellation)
            .await
    }
}

/// 有界输出累积：超限截断（UTF-8 字符边界安全），语义对齐单仓初始化的
/// `LimitedOutput`——durable 面只落有界摘要。
struct BoundedOutput {
    buffer: String,
}

impl BoundedOutput {
    fn new() -> Self {
        Self {
            buffer: String::new(),
        }
    }

    fn push(&mut self, value: &str) {
        if self.buffer.len() >= AGGREGATE_TURN_OUTPUT_LIMIT {
            return;
        }
        if self.buffer.len() + value.len() <= AGGREGATE_TURN_OUTPUT_LIMIT {
            self.buffer.push_str(value);
            return;
        }
        let mut end = AGGREGATE_TURN_OUTPUT_LIMIT - self.buffer.len();
        while end > 0 && !value.is_char_boundary(end) {
            end -= 1;
        }
        self.buffer.push_str(&value[..end]);
    }

    fn is_empty(&self) -> bool {
        self.buffer.is_empty()
    }

    fn summary(&self) -> String {
        self.buffer.clone()
    }
}

/// 复用单仓初始化的 best-effort 中止：取消/超时/交互请求时向 provider 会话
/// 发送 Abort；失败不阻塞失败路径。
fn best_effort_abort(session: &crate::cross_cutting::streaming_provider::ProviderSession) {
    let _ = session
        .commands
        .try_send(crate::cross_cutting::streaming_provider::ProviderCommand::Abort);
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

// Task 1c-coordinator:credential phase 与 run-bound audit 时序的内嵌测试。
// 独立 `mod`(不并入 coordinator_tests.inc.rs 的共享 `mod tests`)以保持
// 1c 切片文件边界——本文件是 1c-coordinator 的独占交付面。
#[cfg(test)]
mod provider_turn_lcg_tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};

    use tokio_util::sync::CancellationToken;

    use super::AggregatePreflightSnapshot;
    use super::{
        AggregateProviderTurnDriver, AggregateProviderTurnRequest,
        GatewayBackedAggregateProviderTurnDriver,
    };
    use crate::cross_cutting::provider_adapter::ProviderAdapterError;
    use crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate;
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::cross_cutting::session_launch::ValidatedStreamingProviderInput;
    use crate::cross_cutting::streaming_provider::{ProviderSession, StreamingProviderAdapter};
    use crate::cross_cutting::tool_policy_audit::{DurableToolPolicyEvent, ProviderStartAudit};
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepStatus;
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateInitializationOperation, AggregateInitializationOperationInput,
        AggregateInitializationStepKind,
    };
    use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
    use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential;
    use crate::product::logical_codebase::provider_gateway::{
        GatewayRunAudit, PolicyTargetResolver,
    };
    use crate::product::logical_codebase::{
        LogicalCodebaseStore, ProviderCapabilityStore, StoreBackedProviderCapabilitySource,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    const FIXTURE_TS: &str = "2026-10-04T00:00:00Z";

    /// start_validated 启动时刻的真实观测快照(T1B-P3-1 同款:由 adapter
    /// 在真实分发路径上记录,而非测试线程手工计数)。
    #[derive(Debug, Clone)]
    struct ValidatedStartObservation {
        /// provider_start 审计行落盘后立即取得的共享计数器标记。
        seq_mark: usize,
        /// 启动请求冻结相位是否为 RootRecipe(pub(crate)
        /// `ValidatedSessionLaunchPolicy::is_root_recipe_phase`)。
        root_recipe_phase: bool,
    }

    /// LC validated 启动观测 adapter:镜像真实 LC adapter(claude_code_provider
    /// 的 lc validated start)在 `start_validated` 首步把 `provider_start`
    /// 写入 run-bound audit sink 的行为——该 append 即「audit 首行」的真实
    /// 落盘时刻;随后取共享计数器标记并快照冻结相位,最后立即完成会话。
    struct RootRecipeObservingAdapter {
        seq: Arc<AtomicUsize>,
        observations: Arc<Mutex<Vec<ValidatedStartObservation>>>,
    }

    #[async_trait::async_trait]
    impl StreamingProviderAdapter for RootRecipeObservingAdapter {
        async fn start_validated(
            &self,
            launch: ValidatedStreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (input, policy) = launch.into_parts();
            let sink = input
                .audit_sink
                .as_ref()
                .expect("prepared launch must bind a run-bound audit sink");
            sink.append_bound(DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
                provider: "claude".to_string(),
                role: "executor".to_string(),
                workspace_session_id: input.workspace_session_id.clone().unwrap_or_default(),
                provider_session_id: "lcg-t01-fake-native-session".to_string(),
                ..ProviderStartAudit::default()
            }))
            .expect("provider_start audit append");
            let seq_mark = self.seq.fetch_add(1, Ordering::SeqCst);
            self.observations
                .lock()
                .expect("observation probe mutex")
                .push(ValidatedStartObservation {
                    seq_mark,
                    root_recipe_phase: policy.is_root_recipe_phase(),
                });
            let (event_tx, events) = tokio::sync::mpsc::channel(1);
            let (commands, _command_rx) = tokio::sync::mpsc::channel(1);
            let _ = event_tx.try_send(
                crate::cross_cutting::streaming_provider::ProviderEvent::Completed(
                    crate::cross_cutting::streaming_provider::ProviderCompletion::plain(
                        "aggregate turn complete",
                        None,
                    ),
                ),
            );
            Ok(ProviderSession {
                events,
                commands,
                native_session_id: Some("lcg-t01-fake-native-session".to_string()),
            })
        }
    }

    /// 测试用 target resolver:与 coordinator_tests/provider_admission_preflight_tests
    /// 的 PassThroughTargetResolver 同型(aggregate root target 原样通过,
    /// spawn 前 canonical 复验由 gateway 内部完成)。
    struct PassThroughTargetResolver;

    impl PolicyTargetResolver for PassThroughTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &crate::product::logical_codebase::SessionLaunchRequest,
        ) -> Result<
            crate::product::logical_codebase::policy::PolicyTarget,
            crate::product::logical_codebase::ProviderGatewayError,
        > {
            Ok(request.target.clone())
        }
    }

    /// 测试用同步 adapter 桩(coordinator_tests 的 StubSyncAdapter 同型)。
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
                stdout: String::new(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: TimeoutStatus::NotTimedOut,
            })
        }
    }

    /// 恒可用 availability gate(coordinator_tests 同型)。
    fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
        use crate::cross_cutting::provider_availability_gate::ProviderHealthSource;
        use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
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
            providers: [crate::product::models::ProviderName::ClaudeCode]
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

    /// Task 1c fixture(蓝图:LC=create+for_lc manifest+material prep 自动物化
    /// policy/capability;Running op=create→mark_running→start_step(PreCheck);
    /// 凭据=from_running_operation):真实 for_lc 存储链 + 生产装配驱动
    /// (`claude_code_with_admission`,snapshot ref 与自举记录一致的
    /// `cap_managed_snapshot`),capability source 为携带 root-recipe 凭据
    /// durable Running 重核验通道的 `StoreBackedProviderCapabilitySource::for_lc`。
    struct LcProductionTurnFixture {
        _temp: tempfile::TempDir,
        paths: ProductAppPaths,
        project_id: String,
        lc_id: String,
        operation_id: String,
        operations: AggregateInitializationOperationStore,
        /// 生产臂在 prepare 前绑定的 run-bound workspace 会话 id(与驱动内
        /// 派生同式:`aggregate-{project}-{operation}`)。
        workspace_session_id: String,
        driver: GatewayBackedAggregateProviderTurnDriver,
    }

    fn lc_production_turn_fixture(
        seq: Arc<AtomicUsize>,
        observations: Arc<Mutex<Vec<ValidatedStartObservation>>>,
    ) -> LcProductionTurnFixture {
        let temp = tempfile::tempdir().expect("tempdir");
        let paths = ProductAppPaths::new(temp.path().join(".aria"));
        let project = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "lcg-t01-production-turn".to_string(),
                description: None,
            })
            .expect("project");
        let aggregate_root = temp.path().join("aggregate-root");
        std::fs::create_dir_all(&aggregate_root).unwrap();
        let canonical_root = std::fs::canonicalize(&aggregate_root).unwrap();

        // LC=create + for_lc manifest(无成员:成员规则门无检查面;根规则
        // 存在性由 AggregateBootstrap 凭据豁免)。
        let lc = LogicalCodebaseStore::new(paths.clone())
            .create(
                &project.id,
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "lcg-t01-lc".to_string(),
                    aggregate_root: aggregate_root.clone(),
                },
            )
            .expect("logical codebase");
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), lc.id.clone());
        let mut manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            &project.id,
            aggregate_root.clone(),
            Vec::new(),
        );
        manifest.logical_codebase_id = uuid::Uuid::new_v4();
        lc_store.save_manifest(&project.id, &manifest).unwrap();

        // material prep:policy/capability 自举材料自动物化(for_lc 作用域)。
        AggregatePolicyArtifactStore::for_lc(paths.clone(), lc.id.clone())
            .ensure_bootstrap(&manifest)
            .expect("bootstrap policy");
        ProviderCapabilityStore::for_lc(paths.clone(), lc.id.clone())
            .ensure_bootstrap(&project.id)
            .expect("bootstrap capability");

        let streaming_adapter = Arc::new(RootRecipeObservingAdapter { seq, observations });
        let mut registry = ProviderRegistry::new();
        registry.register(
            crate::product::models::ProviderName::ClaudeCode,
            streaming_adapter,
        );
        let gateway = Arc::new(
            crate::product::logical_codebase::LogicalCodebaseProviderGateway::with_audit(
                AggregatePolicyArtifactStore::for_lc(paths.clone(), lc.id.clone()),
                Arc::new(StoreBackedProviderCapabilitySource::for_lc(
                    paths.clone(),
                    project.id.clone(),
                    lc.id.clone(),
                )),
                Arc::new(PassThroughTargetResolver),
                Arc::new(registry),
                Arc::new(StubSyncAdapter),
                always_available_gate(),
                Arc::new(GatewayRunAudit::new()),
                canonical_root.clone(),
            ),
        );

        // Running op:create→mark_running→(前置确定性步骤完成)→
        // start_step(PreCheck, digest)——凭据的唯一合法派生面。
        let operations =
            AggregateInitializationOperationStore::for_lc(paths.clone(), lc.id.clone());
        let operation_id = "aggregate_initialization_lcg0001".to_string();
        let input = AggregateInitializationOperationInput {
            idempotency_key: format!("lcg-t01-{operation_id}"),
            manifest_revision: manifest.membership_revision,
            policy_digest: "sha256:lcg-t01-fixture-policy".to_string(),
            profile_evidence_digest: None,
            provider_context_root: canonical_root.clone(),
            provider: "claude_code".to_string(),
        };
        operations
            .create_idempotent(AggregateInitializationOperation::new(
                operation_id.clone(),
                project.id.clone(),
                input,
                FIXTURE_TS.to_string(),
            ))
            .expect("create operation");
        operations
            .mark_running(&project.id, &operation_id, FIXTURE_TS.to_string())
            .expect("mark operation running");
        for predecessor in [
            AggregateInitializationStepKind::MachineSkills,
            AggregateInitializationStepKind::AggregatePreflight,
        ] {
            operations
                .mark_step_running(
                    &project.id,
                    &operation_id,
                    predecessor,
                    format!("sha256:lcg-t01-input-{predecessor:?}"),
                    FIXTURE_TS.to_string(),
                )
                .expect("mark predecessor running");
            operations
                .checkpoint_step_output(
                    &project.id,
                    &operation_id,
                    predecessor,
                    format!("artifact-{predecessor:?}"),
                    FIXTURE_TS.to_string(),
                )
                .expect("checkpoint predecessor");
            operations
                .mark_step_completed(
                    &project.id,
                    &operation_id,
                    predecessor,
                    FIXTURE_TS.to_string(),
                )
                .expect("complete predecessor");
        }
        operations
            .mark_step_running(
                &project.id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                "sha256:lcg-t01-precheck-input".to_string(),
                FIXTURE_TS.to_string(),
            )
            .expect("mark pre_check running");

        // 生产装配驱动:admission paths 在场=生产主路径(credential phase
        // 准备即本切片交付面);snapshot ref 与自举 capability 记录一致
        // (`cap_managed_snapshot`)。
        let driver = GatewayBackedAggregateProviderTurnDriver::claude_code_with_admission(
            gateway,
            "cap_managed_snapshot",
            paths.clone(),
        );
        let workspace_session_id = format!("aggregate-{}-{}", project.id, operation_id);

        LcProductionTurnFixture {
            _temp: temp,
            paths,
            project_id: project.id,
            lc_id: lc.id,
            operation_id,
            operations,
            workspace_session_id,
            driver,
        }
    }

    /// Task 1c(Step 1 冻结测试):生产臂(admission paths 在场)root recipe
    /// turn 的 credential phase 准备与 run-bound audit 真实时序——
    /// ① adapter 经 `start_validated` 真实观测冻结相位为 RootRecipe
    /// (当前 1b 过渡期经 `prepare_streaming_launch` 走 Normal 相位,先红);
    /// ② `provider_start` 恰为该 run 审计分区文件首行(durable 落盘);
    /// ③ audit 首行(adapter 落盘时刻标记)先于 completed 记录落盘
    /// (run_turn 成功后按 coordinator 生命周期收口 step 再取下一标记);
    /// ④ completed 记录 durable(step Completed)。
    #[tokio::test]
    async fn lcg_t01_split_start_audit_precedes_completed_record() {
        let seq = Arc::new(AtomicUsize::new(0));
        let observations: Arc<Mutex<Vec<ValidatedStartObservation>>> = Arc::default();
        let fixture = lc_production_turn_fixture(seq.clone(), observations.clone());

        let bootstrap = BootstrapPhaseCredential::from_running_operation(
            &fixture.operations,
            &fixture.project_id,
            &fixture.operation_id,
            AggregateInitializationStepKind::PreCheck,
            &fixture.lc_id,
            &std::path::PathBuf::from(
                std::fs::canonicalize(fixture._temp.path().join("aggregate-root")).unwrap(),
            ),
        )
        .expect("derive bootstrap credential from running operation");
        let snapshot = AggregatePreflightSnapshot {
            aggregate_root: std::fs::canonicalize(fixture._temp.path().join("aggregate-root"))
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            index_excludes_assets: true,
            members: Vec::new(),
            manifest_revision: 1,
            manifest_digest: "sha256:lcg-t01-manifest".to_string(),
        };
        let summary = fixture
            .driver
            .run_turn(AggregateProviderTurnRequest {
                project_id: &fixture.project_id,
                operation_id: &fixture.operation_id,
                step: AggregateInitializationStepKind::PreCheck,
                preflight: &snapshot,
                lc_id: Some(&fixture.lc_id),
                bootstrap,
                cancellation: CancellationToken::new(),
            })
            .await
            .expect("production turn must complete");
        assert_eq!(summary, "aggregate turn complete");

        // ① credential phase:adapter 在 start_validated 分发路径上真实观测
        // 冻结相位(快照缺失=未走 prepared 分流,同样失败)。
        let observation = observations
            .lock()
            .expect("observation probe mutex")
            .first()
            .cloned()
            .expect("adapter must have started via start_validated dispatch");
        assert!(
            observation.root_recipe_phase,
            "生产臂必须以 RootRecipe 凭据相位启动(root recipe turn 不得误走 \
             Normal 相位的 prepare_streaming_launch)"
        );

        // ② audit 首行 durable:provider_start 恰为该 run 审计分区首行
        // (fresh workspace 分区 ⇒ 首个 role_run_seq 为 0)。
        let lifecycle = LifecycleStore::new(fixture.paths.clone());
        let lines = lifecycle
            .read_tool_policy_lines(&fixture.workspace_session_id, 0)
            .expect("read run-bound audit lines");
        assert_eq!(
            lines.first().map(|line| line.event_type()),
            Some("provider_start"),
            "provider_start 必须是该 run 审计文件的首行"
        );

        // ③ 真实时序:audit 首行(adapter 落盘时刻标记)先于 completed
        //    记录落盘(收口 step 后取下一标记)。
        let provider_start_seq = observation.seq_mark;
        fixture
            .operations
            .checkpoint_step_output(
                &fixture.project_id,
                &fixture.operation_id,
                AggregateInitializationStepKind::PreCheck,
                "artifact-precheck-lcg-t01".to_string(),
                FIXTURE_TS.to_string(),
            )
            .expect("checkpoint pre_check output");
        fixture
            .operations
            .mark_step_completed(
                &fixture.project_id,
                &fixture.operation_id,
                AggregateInitializationStepKind::PreCheck,
                FIXTURE_TS.to_string(),
            )
            .expect("complete pre_check step");
        let split_completed_seq = seq.fetch_add(1, Ordering::SeqCst);
        assert!(
            provider_start_seq < split_completed_seq,
            "provider 启动审计标记({provider_start_seq})必须先于 split completed \
             标记({split_completed_seq})"
        );

        // ④ completed 记录 durable:operation 记录中 PreCheck 已 Completed。
        let operation = fixture
            .operations
            .get(&fixture.project_id, &fixture.operation_id)
            .expect("read back operation record");
        let pre_check = operation
            .steps
            .iter()
            .find(|step| step.step_id == AggregateInitializationStepKind::PreCheck)
            .expect("pre_check step record");
        assert_eq!(
            pre_check.status,
            AggregateInitializationStepStatus::Completed
        );
    }
}
