//! provider 启动路径选择（Task 11）与 gateway 错误映射。

use super::*;

/// Task 11→Task 7:按是否携带 validated input + 是否注入 gateway 选择 provider
/// 启动路径。
///
/// - validated + gateway 两者均在:经 `LogicalCodebaseProviderGateway::
///   start_streaming` 启动,政策校验、canonical 复验、resume fail-closed
///   都在 gateway 内完成并留 audit。Task 2.6（REQ-ENV-10）起 LC Coder/retry
///   的 validated input 携带独立 `working_directory`（=envelope 冻结的
///   canonical root，调用侧 `resolve_coder_root_launch_policy` + envelope
///   重绑产出），gateway 在 `revalidate_before_spawn` 以 effective cwd
///   消费该字段并做 canonical/authority 复验——本分流不重建该校验，
///   fail-closed 语义全部留在 gateway。
/// - validated 在场而 gateway 缺失(Task 7 分流收口):LC validated 输入必须
///   经 gateway 启动——返回稳定错误 `lc_validated_launch_requires_gateway`,
///   🔴 禁止回退裸 `provider.start`。
/// - validated 缺失(传统/非逻辑 issue):直接 adapter start,保留既有
///   行为（单仓 cwd 仍取 `working_dir`，`working_directory` 为 None 的回填
///   语义不变）。
///
/// B1-1(波 2):三条分流透传**同一**受信资源归属载体 `ownership`(engine
/// 三处真实 scope 铸造)——validated+gateway 臂绑定进 validated input 由
/// adapter `start_validated` 消费;非 LC direct 臂经 trait 默认方法
/// `provider.start_owned` 执行登记生命周期;缺 gateway 拒绝臂不变
/// (fail-closed,不因归属在场/缺失改变)。归属在场时不静默降回裸
/// spawn;`None` = 无归属请求(legacy/测试面),保持既有裸 start 语义。
///
/// 返回 boxed future 以便外层 `tokio::select!` 统一内联。gateway 错误映射为
/// `ProviderAdapterError`,使其与直接 adapter 错误在 stream 层等价处理。
pub(super) fn launch_provider_session<'a>(
    provider: &'a dyn StreamingProviderAdapter,
    input: StreamingProviderInput,
    cancel: CancellationToken,
    validated: Option<crate::cross_cutting::session_launch::ValidatedStreamingProviderInput>,
    gateway: Option<
        &std::sync::Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
    >,
    launch_ownership: Option<crate::cross_cutting::provider_resources::TrustedLaunchOwnership>,
) -> std::pin::Pin<
    Box<
        dyn Future<
                Output = Result<
                    crate::cross_cutting::streaming_provider::ProviderSession,
                    ProviderAdapterError,
                >,
            > + Send
            + 'a,
    >,
> {
    match (validated, gateway) {
        (Some(validated), Some(gateway)) => {
            let gateway = gateway.clone();
            Box::pin(async move {
                // B1-1:归属绑定进 validated input(crate 私有 setter),
                // adapter `start_validated` 的 spawn 点消费同一生命周期。
                let validated = match launch_ownership {
                    Some(ownership) => validated.with_launch_ownership(ownership),
                    None => validated,
                };
                // T6 fix-loop r2：Coding 共享启动口（Coder/CodeReviewer/
                // InternalReviewer）接产品层共享分类桥——trust 维度拒绝
                // 提升 provider_trust_missing 分类+同源指导；恢复动作用
                // Coding 场景（重新发起该 Coding 任务），不借用 generation
                // 的「显式重新生成」。provider 取 validated envelope 冻结
                // dialect 的同源映射（与 gateway 自身 trust 复验同口径）。
                let trust_provider =
                    crate::product::logical_codebase::provider_gateway::provider_name_for_dialect(
                        validated.clone().into_parts().1.envelope().provider_dialect,
                    );
                gateway
                    .start_streaming(validated, cancel)
                    .await
                    .map_err(|error| {
                        let mut mapped = provider_adapter_error_from_gateway(&error);
                        mapped.details = gateway.classify_launch_trust_failure_details(
                            &error,
                            &trust_provider,
                            &mapped.details,
                            crate::product::logical_codebase::provider_trust::ProviderTrustScenario::Coding,
                        );
                        mapped
                    })
            })
        }
        // Task 7 分流收口:LC validated 输入缺 gateway 是装配错误,返回稳定
        // 错误码 fail-closed,不回退裸 `provider.start`(B1-1:归属在场
        // 亦不改变——缺 gateway 即拒绝,不降级)。
        (Some(_), None) => Box::pin(async move {
            Err(ProviderAdapterError::provider_unavailable(
                "lc_validated_launch_requires_gateway: validated LC launches must start through LogicalCodebaseProviderGateway; raw provider start fallback is forbidden",
            ))
        }),
        // 非 gateway 路径(传统/非逻辑 issue):直接 adapter start。逻辑代码库 feature
        // 在入口处由 `validated_input` 的存在与否分流,使旧 API 行为不被本工作包扩大。
        // B1-1:归属在场时经 trait 默认方法 `start_owned` 执行登记生命周期
        // (预留→spawn→回填→尾部收口注销);`None` 保持裸 start。
        (None, _) => {
            Box::pin(async move { provider.start_owned(input, launch_ownership, cancel).await })
        }
    }
}

/// 把 gateway 错误映射为 `ProviderAdapterError`,使 provider stream 层把「政策门/
/// 复验拒绝」与「adapter 运行失败」统一为 transport 错误。fail-closed 维度保留在
/// `details` 中供上游诊断。
fn provider_adapter_error_from_gateway(
    error: &crate::product::logical_codebase::ProviderGatewayError,
) -> ProviderAdapterError {
    use crate::protocol::contracts::TimeoutStatus;
    use crate::protocol::provider_errors::ProviderErrorCode;
    // 真实 adapter 的启动/运行失败(Adapter(inner))**保留 inner 的结构化
    // 字段**(code/stdout/stderr/exit_code/timeout/duration):transport 分类
    // (provider_retry::classify_transport_message 的 message 拼接含
    // stdout/stderr)依赖这些字段携带的关键词("text file busy"/"connection"
    // 等);此前拍平成 ProviderUnavailable+Display 会丢掉全部证据,使 LC
    // 路径一切 stderr 关键词型 transport 失败被误判 non_retryable。其余
    // gateway 自身的 fail-closed 门(政策/复验/registry/可用性)保持既有
    // 映射不变。
    match error {
        crate::product::logical_codebase::ProviderGatewayError::Adapter(inner) => inner.clone(),
        other => ProviderAdapterError {
            code: ProviderErrorCode::ProviderUnavailable,
            details: other.to_string(),
            stdout: String::new(),
            stderr: String::new(),
            exit_code: None,
            timeout_status: TimeoutStatus::NotTimedOut,
            duration_ms: 0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::streaming_provider::ProviderSession;
    use crate::product::app_paths::ProductAppPaths;

    /// T6 fix-loop r2（U8 位点）：Coding 共享启动口 trust 拒绝分类。
    /// capability double：任意 provider×action 恒 Confirmed（真实 dialect）。
    struct CodingTrustCapabilitySource;

    impl crate::product::logical_codebase::ProviderCapabilitySource for CodingTrustCapabilitySource {
        fn require_supported(
            &self,
            provider: &crate::product::logical_codebase::ProviderRef,
            action: crate::product::logical_codebase::SessionPolicyAction,
        ) -> Result<
            crate::product::logical_codebase::ProviderCapability,
            crate::product::logical_codebase::ProviderGatewayError,
        > {
            use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
            use crate::product::logical_codebase::policy::ProviderWireDialect;
            use crate::product::logical_codebase::provider_capability_store::ProviderActionCapability;
            let (adapter_dialect, wire_dialect) = match provider.provider_type {
                crate::product::logical_codebase::ProviderRefType::ClaudeCode => (
                    crate::product::logical_codebase::policy::ProviderDialect::ClaudeCodeCliV1,
                    ProviderWireDialect::ClaudeCodeStreamJson,
                ),
                crate::product::logical_codebase::ProviderRefType::Codex => (
                    crate::product::logical_codebase::policy::ProviderDialect::CodexCliV1,
                    ProviderWireDialect::CodexAppServerRpc,
                ),
                crate::product::logical_codebase::ProviderRefType::Pi => (
                    crate::product::logical_codebase::policy::ProviderDialect::PiRpcV1,
                    ProviderWireDialect::PiRpc,
                ),
                crate::product::logical_codebase::ProviderRefType::KimiCode => (
                    crate::product::logical_codebase::policy::ProviderDialect::KimiAcpV1,
                    ProviderWireDialect::KimiAcp,
                ),
            };
            Ok(crate::product::logical_codebase::ProviderCapability {
                provider_type: provider.provider_type,
                version: "1.4.0".to_string(),
                adapter_dialect,
                wire_dialect,
                capability_snapshot_ref: provider.capability_snapshot_ref.clone(),
                action_capability: ProviderActionCapability {
                    action,
                    launch: ProviderCapabilityEvidence::Confirmed,
                    resume: ProviderCapabilityEvidence::Confirmed,
                    write_boundary: ProviderCapabilityEvidence::Confirmed,
                    projection_digest: format!("projection-digest-{action:?}"),
                    evidence_ref: format!("probe://{action:?}"),
                },
                trust: ProviderCapabilityEvidence::Confirmed,
            })
        }

        fn require_resume_supported(
            &self,
            provider: &crate::product::logical_codebase::ProviderRef,
            action: crate::product::logical_codebase::SessionPolicyAction,
        ) -> Result<
            crate::product::logical_codebase::ProviderCapability,
            crate::product::logical_codebase::ProviderGatewayError,
        > {
            self.require_supported(provider, action)
        }

        fn require_write_boundary(
            &self,
            provider: &crate::product::logical_codebase::ProviderRef,
            action: crate::product::logical_codebase::SessionPolicyAction,
        ) -> Result<
            crate::product::logical_codebase::ProviderCapability,
            crate::product::logical_codebase::ProviderGatewayError,
        > {
            self.require_supported(provider, action)
        }

        fn require_root_recipe_supported(
            &self,
            provider: &crate::product::logical_codebase::ProviderRef,
            _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        ) -> Result<
            crate::product::logical_codebase::ProviderCapability,
            crate::product::logical_codebase::ProviderGatewayError,
        > {
            self.require_supported(
                provider,
                crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
            )
        }
    }

    struct CodingTrustTargetResolver;

    impl crate::product::logical_codebase::PolicyTargetResolver for CodingTrustTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &crate::product::logical_codebase::SessionLaunchRequest,
        ) -> Result<
            crate::product::logical_codebase::PolicyTarget,
            crate::product::logical_codebase::ProviderGatewayError,
        > {
            Ok(request.target.clone())
        }
    }

    struct CodingTrustSyncAdapter;

    impl crate::cross_cutting::provider_adapter::ProviderAdapter for CodingTrustSyncAdapter {
        fn run(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
        ) -> Result<
            crate::protocol::contracts::AdapterOutput,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        > {
            Ok(crate::protocol::contracts::AdapterOutput {
                exit_code: Some(0),
                stdout: "ok".to_string(),
                stderr: String::new(),
                structured_output: None,
                files_modified: Vec::new(),
                duration_ms: 0,
                timeout_status: crate::protocol::contracts::TimeoutStatus::NotTimedOut,
            })
        }
    }

    struct CodingTrustAlwaysHealthy;

    impl crate::cross_cutting::provider_availability_gate::ProviderHealthSource
        for CodingTrustAlwaysHealthy
    {
        fn snapshot(
            &self,
        ) -> std::sync::Arc<crate::cross_cutting::provider_health::ProviderHealthSnapshot> {
            let checked_at = chrono::Utc::now();
            std::sync::Arc::new(
                crate::cross_cutting::provider_health::ProviderHealthSnapshot {
                    schema_version: 1,
                    generation: 1,
                    checked_at,
                    providers: [
                        crate::product::models::ProviderName::ClaudeCode,
                        crate::product::models::ProviderName::Codex,
                        crate::product::models::ProviderName::KimiCode,
                    ]
                    .into_iter()
                    .map(
                        |provider| crate::cross_cutting::provider_health::ProviderHealthEntry {
                            provider,
                            command: "stub".to_string(),
                            available: true,
                            version: Some("1.0".to_string()),
                            reason_code: None,
                            reason: None,
                            checked_at,
                        },
                    )
                    .collect(),
                },
            )
        }

        fn degraded(&self) -> bool {
            false
        }
    }

    /// 零启动观测 provider：被启动即 panic（trust 拒绝必须先于 adapter）。
    struct NeverStartAdapter;

    #[async_trait::async_trait]
    impl crate::cross_cutting::streaming_provider::StreamingProviderAdapter for NeverStartAdapter {
        async fn start(
            &self,
            _input: crate::cross_cutting::streaming_provider::StreamingProviderInput,
            _cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            panic!("coding trust rejection must precede adapter start");
        }

        async fn start_validated(
            &self,
            validated: crate::cross_cutting::session_launch::ValidatedStreamingProviderInput,
            cancel: CancellationToken,
        ) -> Result<ProviderSession, ProviderAdapterError> {
            let (input, _launch) = validated.into_parts();
            self.start(input, cancel).await
        }

        async fn run_streaming(
            &self,
            _input: &crate::protocol::contracts::AdapterInput,
            _cancel: CancellationToken,
        ) -> Result<
            mpsc::Receiver<crate::cross_cutting::streaming_provider::StreamChunk>,
            ProviderAdapterError,
        > {
            unreachable!("coding launch tests use start paths")
        }
    }

    fn coding_trust_gateway(
        paths: &ProductAppPaths,
        trust_home: &std::path::Path,
    ) -> std::sync::Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway> {
        let manifest = crate::product::logical_codebase::store::LogicalCodebaseManifest::new(
            "project_0001",
            paths.root().to_path_buf(),
            vec![],
        );
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .expect("bootstrap aggregate policy");
        let lc_id = "logical_codebase_0001".to_string();
        let gateway =
            crate::product::logical_codebase::LogicalCodebaseProviderGateway::with_audit(
                crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::new(
                    paths.clone(),
                ),
                std::sync::Arc::new(CodingTrustCapabilitySource),
                std::sync::Arc::new(CodingTrustTargetResolver),
                std::sync::Arc::new(
                    crate::cross_cutting::provider_registry::ProviderRegistry::new(),
                ),
                std::sync::Arc::new(CodingTrustSyncAdapter),
                std::sync::Arc::new(
                    crate::cross_cutting::provider_availability_gate::ProviderAvailabilityGate::new(
                        std::sync::Arc::new(CodingTrustAlwaysHealthy),
                    ),
                ),
                std::sync::Arc::new(
                    crate::product::logical_codebase::provider_gateway::GatewayRunAudit::new(),
                ),
                std::fs::canonicalize(&manifest.provider_context_root).expect("authority root"),
            )
            .with_readonly_lc_facts(
                crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                    paths.clone(),
                    &lc_id,
                ),
                std::sync::Arc::new(
                    crate::product::logical_codebase::provider_trust::ReadonlyProviderTrustSource::new(
                        paths.clone(),
                        Some(lc_id.clone()),
                        vec![
                            std::sync::Arc::new(
                                crate::product::logical_codebase::provider_trust_adapters::CodexTrustAdapter::for_home(trust_home),
                            ),
                            std::sync::Arc::new(
                                crate::product::logical_codebase::provider_trust_adapters::KimiTrustAdapter::for_home(trust_home),
                            ),
                        ],
                    ),
                ),
                Some(lc_id),
            );
        std::sync::Arc::new(gateway)
    }

    fn coding_validated_input(
        gateway: &std::sync::Arc<crate::product::logical_codebase::LogicalCodebaseProviderGateway>,
        working_dir: std::path::PathBuf,
    ) -> crate::cross_cutting::session_launch::ValidatedStreamingProviderInput {
        let request = crate::product::logical_codebase::SessionLaunchRequest {
            project_id: "project_0001".to_string(),
            provider: crate::product::logical_codebase::ProviderRef::from_provider_name(
                &crate::product::models::ProviderName::KimiCode,
                "cap_managed_snapshot",
            )
            .expect("kimi provider ref"),
            action: crate::product::logical_codebase::SessionPolicyAction::PlanningReadOnly,
            target: crate::product::logical_codebase::PolicyTarget::aggregate_root(working_dir),
            working_directory: gateway.authority_root().to_path_buf(),
            readable_roots: vec![gateway.authority_root().to_path_buf()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:managed-config-artifact".to_string(),
        };
        let validated = gateway.validate(request).expect("coding launch validate");
        let input = crate::cross_cutting::streaming_provider::StreamingProviderInput {
            provider_type: crate::protocol::contracts::ProviderType::KimiCode,
            role: crate::protocol::contracts::AdapterRole::Orchestrator,
            prompt: "coding task prompt".to_string(),
            working_dir: validated.envelope().target.worktree.clone(),
            working_directory: Some(validated.envelope().working_directory.clone()),
            workspace_session_id: None,
            resume_provider_session_id: None,
            permission_mode: crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            tool_policy: None,
            audit_sink: None,
            structured_output_contract: None,
            env_vars: Default::default(),
            timeout_secs: 30,
            baseline_tree: None,
        };
        crate::cross_cutting::session_launch::ValidatedStreamingProviderInput::new(input, validated)
    }

    /// U8 主用例：Kimi 条目缺失 → Coding 共享启动口以
    /// `provider_trust_missing` 分类＋同源指导＋**Coding 场景**
    /// retry_action（重新发起该 Coding 任务，不借用「显式重新生成」）
    /// 拒收；provider 零启动。
    #[tokio::test]
    async fn coding_launch_trust_missing_classifies_with_coding_retry_action() {
        let root = tempfile::tempdir().expect("temporary product root");
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let trust_home = tempfile::tempdir().expect("isolated trust home");
        let gateway = coding_trust_gateway(&paths, trust_home.path());
        let target_dir = tempfile::tempdir().expect("coding target anchor");
        let validated = coding_validated_input(&gateway, target_dir.path().to_path_buf());

        let result = launch_provider_session(
            &NeverStartAdapter,
            crate::cross_cutting::streaming_provider::StreamingProviderInput {
                provider_type: crate::protocol::contracts::ProviderType::KimiCode,
                role: crate::protocol::contracts::AdapterRole::Orchestrator,
                prompt: "coding task prompt".to_string(),
                working_dir: target_dir.path().to_path_buf(),
                working_directory: None,
                workspace_session_id: None,
                resume_provider_session_id: None,
                permission_mode:
                    crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
                tool_policy: None,
                audit_sink: None,
                structured_output_contract: None,
                env_vars: Default::default(),
                timeout_secs: 30,
                baseline_tree: None,
            },
            CancellationToken::new(),
            Some(validated),
            Some(&gateway),
            // B1-1:无归属请求(测试面)——分流透传 None。
            None,
        )
        .await;
        let error = match result {
            Ok(_) => panic!("coding launch must reject on missing trust"),
            Err(error) => error,
        };

        assert!(
            error.details.contains("provider_trust_missing"),
            "Coding 启动臂 trust 拒绝必须提升 provider_trust_missing 分类，got: {}",
            error.details
        );
        assert!(
            error
                .details
                .contains("provider trust not established for KimiCode"),
            "原拒绝原文必须保留，got: {}",
            error.details
        );
        assert!(
            error.details.contains("人工登记") && error.details.contains("重新发起该 Coding 任务"),
            "Coding 臂必须携带 Coding 场景 retry_action，got: {}",
            error.details
        );
        assert!(
            !error.details.contains("显式重新生成"),
            "Coding 臂不得借用 generation 的「显式重新生成」，got: {}",
            error.details
        );
        assert!(
            error.details.contains(
                &trust_home
                    .path()
                    .join(".kimi-code")
                    .join("workspace-trust")
                    .display()
                    .to_string()
            ),
            "Coding 臂指导必须同源（真实工件路径），got: {}",
            error.details
        );
    }
}
