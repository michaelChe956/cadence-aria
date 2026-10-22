use super::*;
use crate::cross_cutting::structured_output::StructuredOutputErrorCode;

const EXAMPLE_FINGERPRINT_IDS: [&str; 4] = ["DEC-001", "CMP-002", "API-002", "REQ-003"];

fn carries_boundary_example_payload(value: &serde_json::Value) -> bool {
    let serialized = value.to_string();
    EXAMPLE_FINGERPRINT_IDS
        .iter()
        .all(|id| serialized.contains(id))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ReviewCompletionError {
    Syntax(StructuredOutputError),
    Schema(ReviewStructuredOutputErrorCode),
    NotRequested,
    RepairPayloadChanged,
    VerificationScopeViolation(String),
}

impl ReviewCompletionError {
    pub(crate) fn code(&self) -> &'static str {
        match self {
            Self::Syntax(error) => error.code.as_str(),
            Self::Schema(error) => error.as_str(),
            Self::NotRequested => "structured_output_not_requested",
            Self::RepairPayloadChanged => "repair_payload_changed",
            Self::VerificationScopeViolation(_) => "verification_scope_violation",
        }
    }

    pub(crate) fn message(&self) -> String {
        match self {
            Self::Syntax(error) => error.message.clone(),
            Self::Schema(error) => error.message().to_string(),
            Self::NotRequested => "审核输入未请求结构化输出".to_string(),
            Self::RepairPayloadChanged => "结构化输出修复改变了审核业务内容".to_string(),
            Self::VerificationScopeViolation(message) => message.clone(),
        }
    }

    pub(crate) fn recoverable_value(&self) -> Option<&serde_json::Value> {
        match self {
            Self::Syntax(error) => error.recoverable_value.as_ref(),
            Self::Schema(_)
            | Self::NotRequested
            | Self::RepairPayloadChanged
            | Self::VerificationScopeViolation(_) => None,
        }
    }

    pub(crate) fn is_classification_fatal(&self) -> bool {
        matches!(
            self,
            Self::Schema(
                ReviewStructuredOutputErrorCode::UnknownFindingCategory
                    | ReviewStructuredOutputErrorCode::UnknownFindingClassHint
            )
        ) || matches!(self, Self::VerificationScopeViolation(_))
    }

    pub(crate) fn is_repairable(&self) -> bool {
        let Some(recoverable_value) = self.recoverable_value() else {
            return false;
        };
        if carries_boundary_example_payload(recoverable_value) {
            return false;
        }

        matches!(
            self,
            Self::Syntax(error)
                if matches!(
                    error.code,
                    StructuredOutputErrorCode::MissingEndTag
                        | StructuredOutputErrorCode::InvalidEndTag
                        | StructuredOutputErrorCode::MissingJsonNonce
                        | StructuredOutputErrorCode::JsonNonceMismatch
                )
        )
    }

    /// C2（REQ-HGC-03 场景 2/F-52 R6）：输出侧解析失败（envelope/JSON 语法
    /// 族）——同 invocation 一次静默重试的适用面；schema/协议错误是确定性
    /// 业务校验失败，重发不改变结果，不适用。
    pub(crate) fn is_output_parse_failure(&self) -> bool {
        matches!(self, Self::Syntax(_))
    }
}

pub(crate) fn repair_payload_is_compatible(
    first_error: &ReviewCompletionError,
    repaired: &ProviderCompletion,
) -> bool {
    let Some(expected) = first_error.recoverable_value() else {
        return false;
    };
    matches!(
        &repaired.structured_output,
        StructuredOutputState::Parsed(actual) if actual == expected
    )
}

pub(crate) fn success_diagnostic(
    first_error: &ReviewCompletionError,
) -> StructuredOutputDiagnostic {
    StructuredOutputDiagnostic {
        code: first_error.code().to_string(),
        message: first_error.message(),
        repair_attempted: true,
        repair_succeeded: true,
        raw_output_preview: None,
    }
}

pub(crate) fn structured_output_repair_event(
    status: ProviderExecutionEventStatus,
    error_code: &str,
) -> ProviderExecutionEvent {
    ProviderExecutionEvent {
        event_id: "structured_output_repair".to_string(),
        kind: ProviderExecutionEventKind::Output,
        status,
        title: "Structured output repair".to_string(),
        detail: Some(format!("repair reviewer structured output: {error_code}")),
        command: None,
        cwd: None,
        output: None,
        exit_code: None,
    }
}

impl WorkspaceEngine {
    /// SingleCandidate 的 Verification 已有 immutable scope；任何 parser/schema
    /// 失败都不能退化成可人工继续的 reviewer fallback，必须由路由层作为
    /// ProtocolViolation durable failed 收口。
    pub(crate) fn review_parse_error_requires_verification_protocol_fatal(&self) -> bool {
        self.session.workspace_type == WorkspaceType::WorkItemPlan
            && self.session.flow_kind
                == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
            && matches!(
                self.session.review_invocation_scope,
                Some(
                    crate::product::work_item_plan_policy::ReviewInvocationScope::Verification { .. }
                )
            )
    }

    /// C2（REQ-HGC-03 场景 2/F-52 R6）：同 invocation 恰一次静默重试的共享
    /// 收尾（直连与 gateway 两路 drive 镜像复用；会话由调用方以同一 input
    /// 启动后传入）——驱动重试会话后：解析成功则正常消费并带
    /// attempted/succeeded diagnostic；provider 失败或二次解析失败则保原始
    /// diagnostic 进人工（attempted=true，routing 侧对该降级轮零计数）。
    /// Aborted 仅静默返回（调用臂后无后续语句，语义与整体 return 等价）。
    pub(crate) async fn complete_silent_output_retry(
        &mut self,
        first_completion: &ProviderCompletion,
        first_error: &ReviewCompletionError,
        retry_session: Result<
            ProviderSession,
            crate::cross_cutting::provider_adapter::ProviderAdapterError,
        >,
        command_rx: &mut mpsc::Receiver<ProviderCommand>,
        reviewer: &ProviderName,
    ) {
        let retry_completion = match self
            .drive_reviewer_provider_session_once(retry_session, command_rx, reviewer)
            .await
        {
            ReviewProviderRunResult::Completed(completion) => completion,
            ReviewProviderRunResult::Aborted => return,
            ReviewProviderRunResult::Failed(_) => {
                let verdict = fallback_review_verdict(first_completion, first_error, true);
                self.complete_review(first_completion.clone(), verdict)
                    .await;
                return;
            }
        };
        match self.parse_review_completion_for_active_node(&retry_completion) {
            Ok(mut verdict) => {
                verdict.structured_output_diagnostic = Some(success_diagnostic(first_error));
                let normalized = ProviderCompletion {
                    full_output: format!(
                        "{}\n{}",
                        first_completion.full_output, retry_completion.full_output
                    ),
                    readable_output: first_completion.readable_output.clone(),
                    structured_output: retry_completion.structured_output,
                    provider_session_id: retry_completion.provider_session_id,
                };
                self.complete_review(normalized, verdict).await;
            }
            Err(second_error) => {
                let normalized = ProviderCompletion {
                    full_output: format!(
                        "{}\n{}",
                        first_completion.full_output, retry_completion.full_output
                    ),
                    readable_output: first_completion.readable_output.clone(),
                    structured_output: retry_completion.structured_output,
                    provider_session_id: retry_completion.provider_session_id,
                };
                let verdict = fallback_review_verdict(&normalized, &second_error, true);
                self.complete_review(normalized, verdict).await;
            }
        }
    }

    fn verification_scope_error_if_needed(
        &self,
        error: ReviewCompletionError,
    ) -> ReviewCompletionError {
        if self.review_parse_error_requires_verification_protocol_fatal()
            && !error.is_classification_fatal()
        {
            ReviewCompletionError::VerificationScopeViolation(format!(
                "verification review output rejected: {}",
                error.message()
            ))
        } else {
            error
        }
    }

    pub(crate) fn parse_review_completion_for_active_node(
        &self,
        completion: &ProviderCompletion,
    ) -> Result<ReviewVerdict, ReviewCompletionError> {
        let StructuredOutputState::Parsed(value) = &completion.structured_output else {
            let error = match &completion.structured_output {
                StructuredOutputState::Failed(error) => {
                    ReviewCompletionError::Syntax(error.clone())
                }
                StructuredOutputState::NotRequested => ReviewCompletionError::NotRequested,
                StructuredOutputState::Parsed(_) => unreachable!(),
            };
            return Err(self.verification_scope_error_if_needed(error));
        };

        let parsed = if self.session.workspace_type == WorkspaceType::WorkItemPlan {
            let scope = match self.active_node_type() {
                Some(TimelineNodeType::WorkItemPlanOutlineReview) => {
                    WorkItemPlanReviewScope::Outline
                }
                Some(TimelineNodeType::WorkItemDraftReview) => WorkItemPlanReviewScope::Item,
                Some(TimelineNodeType::WorkItemBatchReview) => WorkItemPlanReviewScope::Batch,
                _ => WorkItemPlanReviewScope::Outline,
            };
            parse_work_item_plan_review_value(
                value,
                &completion.readable_output,
                &self.current_work_item_plan_outline_ids(),
                scope,
            )
            .map_err(ReviewCompletionError::Schema)
        } else {
            parse_review_value(value, &completion.readable_output)
                .map_err(ReviewCompletionError::Schema)
        };

        parsed
            .map_err(|error| self.verification_scope_error_if_needed(error))
            .and_then(|verdict| self.reject_incomplete_logical_codebase_findings(verdict))
    }

    /// REQ-PLN-16（设计 A 条目 11）：多仓形态下 finding 的 `evidence` 与
    /// `required_action` 缺任一即整轮判 `malformed_findings`——该 finding 不进
    /// 入 `findings`，调用侧经 `fallback_review_verdict` 收敛为 `needs_human`
    /// 且诊断码稳定为 `malformed_findings`。单仓形态（无 gateway）逐字不变。
    ///
    /// 该门置于 verification-scope 升级之后：契约要求该码稳定可见，不得被
    /// 升级成 `verification_scope_violation`（`malformed_findings` 本就不在
    /// `is_classification_fatal` 内）。
    fn reject_incomplete_logical_codebase_findings(
        &self,
        verdict: ReviewVerdict,
    ) -> Result<ReviewVerdict, ReviewCompletionError> {
        if !self.is_logical_codebase_issue() {
            return Ok(verdict);
        }
        let incomplete = verdict.findings.iter().any(|finding| {
            finding.evidence.trim().is_empty() || finding.required_action.trim().is_empty()
        });
        if incomplete {
            return Err(ReviewCompletionError::Schema(
                ReviewStructuredOutputErrorCode::MalformedFindings,
            ));
        }
        Ok(verdict)
    }
}

pub(crate) fn fallback_review_verdict(
    completion: &ProviderCompletion,
    error: &ReviewCompletionError,
    repair_attempted: bool,
) -> ReviewVerdict {
    ReviewVerdict {
        verdict: ReviewVerdictType::NeedsHuman,
        comments: completion.readable_output.clone(),
        summary: "需要人工确认".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
        structured_output_diagnostic: Some(StructuredOutputDiagnostic {
            code: error.code().to_string(),
            message: error.message(),
            repair_attempted,
            repair_succeeded: false,
            raw_output_preview: Some(preview(&completion.full_output)),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn syntax_error(
        code: StructuredOutputErrorCode,
        recoverable_value: serde_json::Value,
    ) -> ReviewCompletionError {
        ReviewCompletionError::Syntax(StructuredOutputError {
            code,
            message: "structured output failure".to_string(),
            expected_nonce: Some("expected-nonce".to_string()),
            observed_nonce: Some("observed-nonce".to_string()),
            recoverable_value: Some(recoverable_value),
        })
    }

    fn example_fingerprint_payload() -> serde_json::Value {
        json!({
            "findings": ["DEC-001", "CMP-002", "API-002", "REQ-003"]
        })
    }

    #[test]
    fn classification_errors_are_fatal_and_not_repairable() {
        assert!(
            ReviewCompletionError::Schema(ReviewStructuredOutputErrorCode::UnknownFindingCategory)
                .is_classification_fatal()
        );
        assert!(
            ReviewCompletionError::Schema(ReviewStructuredOutputErrorCode::UnknownFindingClassHint)
                .is_classification_fatal()
        );
        assert!(
            !ReviewCompletionError::Schema(ReviewStructuredOutputErrorCode::InvalidFindingField)
                .is_classification_fatal()
        );
    }

    #[test]
    fn json_nonce_mismatch_with_example_fingerprint_is_not_repairable() {
        let error = syntax_error(
            StructuredOutputErrorCode::JsonNonceMismatch,
            example_fingerprint_payload(),
        );

        assert!(!error.is_repairable());
    }

    #[test]
    fn missing_json_nonce_with_example_fingerprint_is_not_repairable() {
        let error = syntax_error(
            StructuredOutputErrorCode::MissingJsonNonce,
            example_fingerprint_payload(),
        );

        assert!(!error.is_repairable());
    }

    #[test]
    fn envelope_only_repairs_without_fingerprint_remain_repairable() {
        let error = syntax_error(
            StructuredOutputErrorCode::JsonNonceMismatch,
            json!({"verdict": "revise", "findings": []}),
        );

        assert!(error.is_repairable());
    }

    #[test]
    fn nonce_mismatch_arm_is_gone() {
        let error = syntax_error(
            StructuredOutputErrorCode::NonceMismatch,
            json!({"verdict": "revise", "findings": []}),
        );

        assert!(!error.is_repairable());
    }
}

/// REQ-PLN-16（设计 A 条目 11）：多仓（`is_logical_codebase_issue()` 为真）
/// 评审的每条 finding 都必须带非空 `evidence` 与非空 `required_action`；缺
/// 任一即整轮判 `malformed_findings`（→ `needs_human`），该 finding 不进入
/// `findings`。单仓同输入沿用 `parsers.rs` 现状（空串照收），行为逐字不变。
#[cfg(test)]
mod lc_finding_completeness_tests {
    use super::*;
    use crate::cross_cutting::provider_availability_gate::{
        ProviderAvailabilityGate, ProviderHealthSource,
    };
    use crate::cross_cutting::provider_health::{ProviderHealthEntry, ProviderHealthSnapshot};
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::checkpoint_store::CheckpointStore;
    use crate::product::logical_codebase::{
        AggregatePolicyArtifactStore, LogicalCodebaseProviderGateway, PolicyTarget,
        PolicyTargetResolver, ProviderCapability, ProviderCapabilitySource, ProviderGatewayError,
        ProviderRef, SessionLaunchRequest, SessionPolicyAction,
    };
    use crate::web::workspace_ws_types::ReviewVerdictType;
    use serde_json::json;
    use std::sync::Arc;
    use tokio::sync::mpsc;

    /// 占位 capability source：形态门禁只读 `Option::is_some()`，恒拒即可。
    struct UnusedCapabilitySource;

    impl ProviderCapabilitySource for UnusedCapabilitySource {
        fn require_supported(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "lc finding fixture capability source is never queried".to_string(),
            ))
        }

        fn require_resume_supported(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "lc finding fixture capability source is never queried".to_string(),
            ))
        }

        fn require_write_boundary(
            &self,
            _provider: &ProviderRef,
            _action: SessionPolicyAction,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "lc finding fixture capability source is never queried".to_string(),
            ))
        }

        fn require_root_recipe_supported(
            &self,
            _provider: &ProviderRef,
            _credential: &crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        ) -> Result<ProviderCapability, ProviderGatewayError> {
            Err(ProviderGatewayError::UnsupportedCapability(
                "lc finding fixture capability source is never queried".to_string(),
            ))
        }
    }

    /// 占位 target resolver：透传请求 target（不会被调用）。
    struct UnusedTargetResolver;

    impl PolicyTargetResolver for UnusedTargetResolver {
        fn resolve_and_revalidate(
            &self,
            request: &SessionLaunchRequest,
        ) -> Result<PolicyTarget, ProviderGatewayError> {
            Ok(request.target.clone())
        }
    }

    /// 占位同步 adapter：gateway 构造参数，不会被调用。
    struct UnusedSyncAdapter;

    impl crate::cross_cutting::provider_adapter::ProviderAdapter for UnusedSyncAdapter {
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

    fn always_available_gate() -> Arc<ProviderAvailabilityGate> {
        struct AlwaysHealthy(Arc<ProviderHealthSnapshot>);

        impl ProviderHealthSource for AlwaysHealthy {
            fn snapshot(&self) -> Arc<ProviderHealthSnapshot> {
                self.0.clone()
            }

            fn degraded(&self) -> bool {
                false
            }
        }

        let checked_at = chrono::Utc::now();
        let snapshot = Arc::new(ProviderHealthSnapshot {
            schema_version: 1,
            generation: 1,
            checked_at,
            providers: [ProviderName::ClaudeCode]
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

    /// 构造仅满足形态判据的最小 gateway（门禁只读 `Option::is_some()`，
    /// 不触 bootstrap / capability / availability 链）。
    fn stub_logical_gateway() -> Arc<LogicalCodebaseProviderGateway> {
        let root = tempfile::tempdir().expect("gateway fixture root under /tmp");
        let authority_root = root.path().join("authority");
        std::fs::create_dir_all(&authority_root).expect("create authority root");
        Arc::new(LogicalCodebaseProviderGateway::new(
            AggregatePolicyArtifactStore::new(ProductAppPaths::new(root.path().join(".aria"))),
            Arc::new(UnusedCapabilitySource),
            Arc::new(UnusedTargetResolver),
            Arc::new(ProviderRegistry::new()),
            Arc::new(UnusedSyncAdapter),
            always_available_gate(),
            authority_root,
        ))
    }

    fn session(workspace_type: WorkspaceType) -> WorkspaceSession {
        WorkspaceSession {
            session_id: "sess_lc_finding_completeness".to_string(),
            project_id: "project_0001".to_string(),
            issue_id: "issue_0001".to_string(),
            entity_id: "story_spec_0001".to_string(),
            workspace_type,
            stage: WorkspaceStage::Revision,
            messages: Vec::new(),
            artifact: None,
            author_provider: ProviderName::ClaudeCode,
            reviewer_provider: Some(ProviderName::Codex),
            review_rounds: 1,
            permission_modes: crate::product::models::WorkspaceRolePermissionModes::default(),
            provisional_reviewer_provider: None,
            reviewer_enabled_at_start: None,
            superpowers_enabled: true,
            openspec_enabled: true,
            session_status: crate::product::models::WorkspaceSessionStatus::Open,
            flow_kind: crate::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
            run_policy: crate::product::work_item_plan_policy::RunPolicy::Interactive,
            run_history: Default::default(),
            review_invocation_scope: None,
            human_gate_snapshot: None,
            repair_reservation: None,
            policy_diagnostics: Vec::new(),
            provider_start_ledger: Vec::new(),
            single_candidate_phase: None,
            work_item_plan_source_revision_ref: None,
            plan_candidate_ir_ref: None,
            mechanical_report_ref: None,
            publication_provenance_ref: None,
            approval_attempt_id: None,
            approved_at: None,
            compile_reservation: None,
            provider_conversations: Vec::new(),
            logical_launch_fingerprints: Vec::new(),
            superseded_native_sessions: Vec::new(),
            lc_resume_recovery: None,
            structural_governance_state: None,
            review_identity_id: String::new(),
            repository_path: None,
        }
    }

    /// `logical == true` 走多仓形态（注入 gateway）；`false` 是单仓基线。
    fn engine(workspace_type: WorkspaceType, logical: bool) -> WorkspaceEngine {
        let (event_tx, _event_rx) = mpsc::channel(8);
        let store = Arc::new(CheckpointStore::new(
            tempfile::tempdir().unwrap().path().to_path_buf(),
        ));
        let engine = WorkspaceEngine::new(store, event_tx, session(workspace_type));
        if logical {
            engine.with_logical_provider_gateway(stub_logical_gateway())
        } else {
            engine
        }
    }

    fn completion(payload: serde_json::Value) -> ProviderCompletion {
        ProviderCompletion {
            full_output: payload.to_string(),
            readable_output: "审核意见".to_string(),
            structured_output: StructuredOutputState::Parsed(payload),
            provider_session_id: None,
        }
    }

    fn generic_review_completion(findings: serde_json::Value) -> ProviderCompletion {
        completion(json!({
            "verdict": "revise",
            "summary": "需要返修",
            "findings": findings,
        }))
    }

    #[test]
    fn lc_review_rejects_finding_without_evidence() {
        let cases = [
            (
                "evidence 缺字段",
                json!({
                    "severity": "must_fix",
                    "message": "缺口未附证据",
                    "required_action": "补充证据",
                }),
            ),
            (
                "evidence 全空白",
                json!({
                    "severity": "must_fix",
                    "message": "缺口未附证据",
                    "evidence": "   ",
                    "required_action": "补充证据",
                }),
            ),
            (
                "required_action 缺字段",
                json!({
                    "severity": "must_fix",
                    "message": "缺口未附证据",
                    "evidence": "work_item=WI-002",
                }),
            ),
        ];

        for (label, finding) in cases {
            let engine = engine(WorkspaceType::Story, true);
            let completion = generic_review_completion(json!([finding]));
            let error = engine
                .parse_review_completion_for_active_node(&completion)
                .expect_err("多仓缺证据 finding 必须整轮拒收");

            assert!(
                matches!(
                    &error,
                    ReviewCompletionError::Schema(
                        ReviewStructuredOutputErrorCode::MalformedFindings
                    )
                ),
                "{label}: 期望 Schema(MalformedFindings)，实得 {error:?}"
            );
            assert_eq!(error.code(), "malformed_findings", "{label}");
            assert!(
                !error.is_classification_fatal(),
                "{label}: malformed_findings 不得进入分类致命臂"
            );
            assert!(
                !error.is_repairable(),
                "{label}: malformed_findings 不得进入一次 repair"
            );

            let fallback = fallback_review_verdict(&completion, &error, false);
            assert_eq!(
                fallback.verdict,
                ReviewVerdictType::NeedsHuman,
                "{label}: 缺证据轮必须判 needs_human"
            );
            assert!(
                fallback.findings.is_empty(),
                "{label}: 缺证据 finding 不得进入 findings"
            );
            assert_eq!(
                fallback
                    .structured_output_diagnostic
                    .as_ref()
                    .map(|diagnostic| diagnostic.code.as_str()),
                Some("malformed_findings"),
                "{label}: diagnostic 码必须是 malformed_findings"
            );
        }
    }

    #[test]
    fn lc_review_accepts_finding_with_evidence_and_required_action() {
        let engine = engine(WorkspaceType::Story, true);
        let completion = generic_review_completion(json!([{
            "severity": "must_fix",
            "message": "缺口未附证据",
            "evidence": "work_item=WI-002",
            "required_action": "补充证据",
        }]));

        let verdict = engine
            .parse_review_completion_for_active_node(&completion)
            .expect("证据齐备的多仓 finding 必须照收");
        assert_eq!(verdict.findings.len(), 1);
        assert_eq!(verdict.findings[0].evidence, "work_item=WI-002");
        assert_eq!(verdict.findings[0].required_action, "补充证据");
    }

    #[test]
    fn lc_work_item_plan_review_rejects_finding_without_evidence() {
        let engine = engine(WorkspaceType::WorkItemPlan, true);
        let completion = completion(json!({
            "review_scope": "outline",
            "verdict": "revise",
            "summary": "需要返修",
            "generation_round_id": "gen_001",
            "findings": [{
                "severity": "must_fix",
                "message": "缺口未附证据",
                "required_action": "补充证据",
            }],
        }));

        assert!(
            matches!(
                engine.parse_review_completion_for_active_node(&completion),
                Err(ReviewCompletionError::Schema(
                    ReviewStructuredOutputErrorCode::MalformedFindings
                ))
            ),
            "多仓 Work Item Plan 评审同样必须拒收缺证据 finding"
        );
    }

    #[test]
    fn single_repo_still_accepts_finding_without_evidence() {
        let engine = engine(WorkspaceType::Story, false);
        let completion = generic_review_completion(json!([{
            "severity": "must_fix",
            "message": "缺口未附证据",
            "required_action": "补充证据",
        }]));

        let verdict = engine
            .parse_review_completion_for_active_node(&completion)
            .expect("单仓同输入必须与改造前一致：照收");
        assert_eq!(verdict.findings.len(), 1);
        assert_eq!(verdict.findings[0].evidence, "");
        assert_eq!(verdict.findings[0].required_action, "补充证据");
    }
}
