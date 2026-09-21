#[test]
fn session_state_serde_roundtrip_preserves_work_item_plan_candidate() {
    let candidate = WorkItemPlanCandidateDto {
        plan: WorkItemPlanDto {
            id: "plan_001".to_string(),
            status: "draft".to_string(),
            options: WorkItemSplitOptionsDto {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: true,
                require_execution_plan_confirm: false,
            },
            dependency_graph: vec![WorkItemDependencyEdgeDto {
                from_work_item_id: "wi_001".to_string(),
                to_work_item_id: "wi_002".to_string(),
            }],
        },
        work_items: vec![
            WorkItemCandidateDto {
                id: "wi_001".to_string(),
                kind: "backend".to_string(),
                title: "后端 API".to_string(),
                depends_on: vec![],
                exclusive_write_scopes: vec!["src/api".to_string()],
                verification_plan_ref: Some("vp_001".to_string()),
                meta: WorkItemCandidateMetaDto {
                    reverted: true,
                    revert_feedback: Some("拆得太粗".to_string()),
                },
            },
            WorkItemCandidateDto {
                id: "wi_002".to_string(),
                kind: "frontend".to_string(),
                title: "前端组件".to_string(),
                depends_on: vec!["wi_001".to_string()],
                exclusive_write_scopes: vec!["web/src".to_string()],
                verification_plan_ref: None,
                meta: WorkItemCandidateMetaDto {
                    reverted: false,
                    revert_feedback: None,
                },
            },
        ],
        verification_plans: vec![VerificationPlanDto {
            plan_ref: "vp_001".to_string(),
            scope: "unit".to_string(),
            commands: vec![VerificationCommandDto {
                label: "cargo test".to_string(),
                command: "cargo test".to_string(),
                cwd: "".to_string(),
                purpose: "unit tests".to_string(),
                required: true,
                timeout_seconds: 120,
                safety: "approved".to_string(),
            }],
            manual_checks: vec![VerificationManualCheckDto {
                label: "手工验证".to_string(),
                instructions: "运行并观察".to_string(),
                required: false,
            }],
            required_gates: vec![],
            risk_notes: vec![],
            confidence: "high".to_string(),
            fallback_policy: "manual_gate".to_string(),
        }],
        repository_profile: None,
        validator_findings: vec![ValidatorFindingDto {
            severity: "warning".to_string(),
            code: "SCOPE_OVERLAP".to_string(),
            message: "范围可能重叠".to_string(),
            work_item_ids: vec!["wi_001".to_string()],
        }],
    };

    let state = WsOutMessage::SessionState {
        session_id: "workspace_session_001".to_string(),
        connection_id: None,
        workspace_type: WorkspaceType::WorkItemPlan,
        stage: "author_confirm".to_string(),
        superpowers_enabled: true,
        openspec_enabled: true,
        messages: vec![WsMessageDto {
            id: "msg_001".to_string(),
            role: "system".to_string(),
            content: "候选 work item plan 生成器".to_string(),
            checkpoint_id: None,
            created_at: "2026-06-17T00:00:00Z".to_string(),
        }],
        checkpoints: vec![WsCheckpointDto {
            id: "ckpt_001".to_string(),
            message_index: 1,
            stage: "author_confirm".to_string(),
            created_at: "2026-06-17T00:00:00Z".to_string(),
        }],
        artifact: Some(ArtifactPayload::WorkItemPlanCandidate {
            candidate: Box::new(candidate.clone()),
        }),
        providers: WsProviderConfig {
            author: cadence_aria::product::models::ProviderName::Fake,
            reviewer: Some(cadence_aria::product::models::ProviderName::Codex),
        },
        timeline_nodes: vec![TimelineNode {
            node_id: "node_001".to_string(),
            node_type: TimelineNodeType::AuthorConfirm,
            agent: None,
            stage: WorkspaceStage::AuthorConfirm,
            round: None,
            status: TimelineNodeStatus::Paused,
            title: "Author 结果确认".to_string(),
            summary: None,
            started_at: "2026-06-17T00:00:00Z".to_string(),
            completed_at: None,
            duration_ms: None,
            artifact_ref: Some("artifact_current".to_string()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: cadence_aria::product::models::ProviderName::Fake,
                reviewer: Some(cadence_aria::product::models::ProviderName::Codex),
                review_rounds: 1,
                permission_modes: cadence_aria::product::models::WorkspaceRolePermissionModes::default(),
            },
            retry: None,
        }],
        active_node_id: Some("node_001".to_string()),
        artifact_versions: vec![],
        artifact_version_summaries: vec![],
        timeline_node_details: HashMap::new(),
        timeline_node_summaries: HashMap::new(),
        active_run_id: None,
        human_presentation_revisions: vec![],
        reviewer_enabled_at_start: None,
        recoverable_interrupted_run: None,
        plan_repair: None,
        session_status: cadence_aria::product::models::WorkspaceSessionStatus::Running,
        flow_kind: cadence_aria::product::work_item_plan_policy::WorkItemPlanFlowKind::Legacy,
        run_policy: cadence_aria::product::work_item_plan_policy::RunPolicy::Interactive,
        run_history: cadence_aria::product::work_item_plan_policy::RunHistory::default(),
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
        pending_choice_requests: Vec::new(),
    };

    let value = serde_json::to_value(&state).expect("serialize SessionState");
    let roundtrip: WsOutMessage = serde_json::from_value(value).expect("deserialize SessionState");

    match roundtrip {
        WsOutMessage::SessionState {
            artifact: Some(ArtifactPayload::WorkItemPlanCandidate { candidate: rt }),
            ..
        } => {
            assert_eq!(rt.work_items.len(), 2);
            let wi_001 = rt.work_items.iter().find(|w| w.id == "wi_001").unwrap();
            assert!(wi_001.meta.reverted);
            assert_eq!(wi_001.meta.revert_feedback, Some("拆得太粗".to_string()));
            assert_eq!(rt.verification_plans.len(), 1);
            assert_eq!(rt.validator_findings.len(), 1);
        }
        other => panic!("expected SessionState with WorkItemPlanCandidate, got {other:?}"),
    }
}


// 退役留档（T5/REQ-RET-02）：`reconnect_preserves_revert_marks_from_current_artifact_version` 直接驱动已删除的 legacy 决策面
// （原 #[ignore]：legacy full-candidate revert 恢复已被 WP2 outline 生成取代；本测含 revert_work_item 退役 wire 字面量），
// 随消息族退役——T1 矩阵 legacy 回归全绿证据在案
// （wp1-gate-retest/evidence-matrix.md §2），见 wp5-attribution-table.md。

// 退役留档（T5/REQ-RET-02）：draft 输出构造夹具（valid_draft_output/valid_frontend_draft_output/
// valid_integration_draft_output）为 staged 恢复矩阵测试专用，随消息族一并退役。
