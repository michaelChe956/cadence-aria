use crate::product::models::{ProviderName, WorkspaceType};
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ChoiceAnswer, ChoiceOption, ChoiceQuestion,
    ProviderConfigSnapshot, RepositoryProfileDto, ReviewGate, ReviewVerdict, ReviewVerdictType,
    TimelineNode, TimelineNodeStatus, TimelineNodeType, ValidatorFindingDto,
    VerificationCommandDto, VerificationManualCheckDto, VerificationPlanDto, WorkItemCandidateDto,
    WorkItemCandidateMetaDto, WorkItemDependencyEdgeDto, WorkItemGenerationModeDto,
    WorkItemPlanCandidateDto, WorkItemPlanDto, WorkItemPlanReviewAction,
    WorkItemPlanReviewComplete, WorkItemPlanReviewGate, WorkItemPlanReviewScope,
    WorkItemPlanReviewVerdict, WorkItemSplitOptionsDto, WorkspaceStage, WsExecutionEvent,
    WsExecutionEventKind, WsExecutionEventStatus, WsInMessage, WsOutMessage, WsPermissionRiskLevel,
    WsProviderStatus,
};

#[test]
fn permission_messages_use_snake_case_type_tags() {
    let out = WsOutMessage::PermissionRequest {
        id: "perm_001".to_string(),
        tool_name: "bash".to_string(),
        description: "Run cargo test".to_string(),
        risk_level: WsPermissionRiskLevel::Medium,
    };
    let value = serde_json::to_value(out).unwrap();
    assert_eq!(value["type"], "permission_request");
    assert_eq!(value["risk_level"], "medium");

    let status = WsOutMessage::ProviderStatus {
        status: WsProviderStatus::WaitingApproval,
    };
    let value = serde_json::to_value(status).unwrap();
    assert_eq!(value["type"], "provider_status");
    assert_eq!(value["status"], "waiting_approval");

    let input: WsInMessage = serde_json::from_value(serde_json::json!({
        "type": "permission_response",
        "id": "perm_001",
        "approved": true,
        "reason": null
    }))
    .unwrap();

    assert!(matches!(
        input,
        WsInMessage::PermissionResponse { approved: true, .. }
    ));
}

#[test]
fn permission_message_values_are_constrained() {
    let invalid_risk: Result<WsOutMessage, _> = serde_json::from_value(serde_json::json!({
        "type": "permission_request",
        "id": "perm_001",
        "tool_name": "bash",
        "description": "Run cargo test",
        "risk_level": "critical"
    }));
    assert!(invalid_risk.is_err());

    let invalid_status: Result<WsOutMessage, _> = serde_json::from_value(serde_json::json!({
        "type": "provider_status",
        "status": "ready"
    }));
    assert!(invalid_status.is_err());
}

#[test]
fn execution_event_messages_use_snake_case_type_tags() {
    let out = WsOutMessage::ExecutionEvent {
        event: WsExecutionEvent {
            event_id: "command_cmd_001".to_string(),
            node_id: Some("node_generation_001".to_string()),
            agent: Some(ProviderName::ClaudeCode),
            kind: WsExecutionEventKind::Command,
            status: WsExecutionEventStatus::Completed,
            title: "Command completed".to_string(),
            detail: Some("exit code 0".to_string()),
            command: Some("pwd".to_string()),
            cwd: Some("/tmp/repo".to_string()),
            output: Some("/tmp/repo\n".to_string()),
            exit_code: Some(0),
        },
    };

    let value = serde_json::to_value(out).unwrap();
    assert_eq!(value["type"], "execution_event");
    assert_eq!(value["event"]["kind"], "command");
    assert_eq!(value["event"]["status"], "completed");
    assert_eq!(value["event"]["node_id"], "node_generation_001");
    assert_eq!(value["event"]["agent"], "claude_code");
    assert_eq!(value["event"]["command"], "pwd");
    assert_eq!(value["event"]["cwd"], "/tmp/repo");
}

#[test]
fn workspace_stage_supports_review_decision_and_revision() {
    let decision = serde_json::to_value(WorkspaceStage::ReviewDecision).unwrap();
    let revision = serde_json::to_value(WorkspaceStage::Revision).unwrap();

    assert_eq!(decision, "review_decision");
    assert_eq!(revision, "revision");
}

#[test]
fn timeline_messages_include_node_identity() {
    let node = TimelineNode {
        node_id: "node_review_001".to_string(),
        node_type: TimelineNodeType::ReviewerRun,
        agent: Some(ProviderName::Codex),
        stage: WorkspaceStage::CrossReview,
        round: Some(1),
        status: TimelineNodeStatus::Active,
        title: "Review Round 1".to_string(),
        summary: None,
        started_at: "2026-05-19T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: Some("version_0001".to_string()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 2,
        },
        retry: None,
    };

    let created =
        serde_json::to_value(WsOutMessage::TimelineNodeCreated { node: node.clone() }).unwrap();
    assert_eq!(created["type"], "timeline_node_created");
    assert_eq!(created["node"]["node_type"], "reviewer_run");
    assert_eq!(created["node"]["status"], "active");
    assert_eq!(created["node"]["agent"], "codex");

    let chunk = serde_json::to_value(WsOutMessage::StreamChunk {
        role: "assistant".to_string(),
        content: "reviewing".to_string(),
        node_id: Some("node_review_001".to_string()),
    })
    .unwrap();
    assert_eq!(chunk["type"], "stream_chunk");
    assert_eq!(chunk["node_id"], "node_review_001");

    let complete = serde_json::to_value(WsOutMessage::MessageComplete {
        message_id: "msg_002".to_string(),
        checkpoint_id: "checkpoint_001".to_string(),
        node_id: Some("node_review_001".to_string()),
    })
    .unwrap();
    assert_eq!(complete["type"], "message_complete");
    assert_eq!(complete["node_id"], "node_review_001");
}

#[test]
fn review_messages_and_session_state_serialize_as_contract() {
    let verdict = ReviewVerdict {
        verdict: ReviewVerdictType::Revise,
        comments: "需要补充验收标准".to_string(),
        summary: "补充验收标准后返修".to_string(),
        findings: vec![crate::web::workspace_ws_types::ReviewFinding {
            severity: crate::web::workspace_ws_types::ReviewFindingSeverity::MustFix,
            message: "缺少验收标准".to_string(),
            evidence: "Artifact 未列出验收标准".to_string(),
            impact: "无法进入下一阶段".to_string(),
            required_action: "补充验收标准".to_string(),
        }],
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: None,
    };

    let review_complete = serde_json::to_value(WsOutMessage::ReviewComplete {
        node_id: "node_review_001".to_string(),
        round: 1,
        verdict: verdict.verdict.clone(),
        comments: verdict.comments.clone(),
        summary: verdict.summary.clone(),
        findings: verdict.findings.clone(),
        review_gate: verdict.review_gate.clone(),
        work_item_plan_review: None,
    })
    .unwrap();
    assert_eq!(review_complete["type"], "review_complete");
    assert_eq!(review_complete["verdict"], "revise");
    assert_eq!(review_complete["review_gate"], "user_triage_required");
    assert_eq!(review_complete["findings"][0]["severity"], "must_fix");
    assert!(review_complete.get("work_item_plan_review").is_none());

    let input: WsInMessage = serde_json::from_value(serde_json::json!({
        "type": "review_decision_response",
        "decision": "continue_with_context",
        "extra_context": "请补充边界条件"
    }))
    .unwrap();
    assert!(matches!(
        input,
        WsInMessage::ReviewDecisionResponse {
            decision,
            extra_context: Some(_),
        } if decision == "continue_with_context"
    ));

    let state = serde_json::to_value(WsOutMessage::SessionState {
        session_id: "workspace_session_0001".to_string(),
        workspace_type: WorkspaceType::Story,
        stage: "review_decision".to_string(),
        superpowers_enabled: true,
        openspec_enabled: true,
        messages: Vec::new(),
        checkpoints: Vec::new(),
        artifact: Some(ArtifactPayload::Markdown {
            markdown: "# Story".to_string(),
            diff: None,
        }),
        providers: crate::web::workspace_ws_types::WsProviderConfig {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
        },
        timeline_nodes: Vec::new(),
        active_node_id: Some("node_review_decision_001".to_string()),
        artifact_versions: Vec::new(),
        artifact_version_summaries: Vec::new(),
        timeline_node_details: std::collections::HashMap::new(),
        timeline_node_summaries: std::collections::HashMap::new(),
        active_run_id: None,
    })
    .unwrap();
    assert_eq!(state["type"], "session_state");
    assert_eq!(state["active_node_id"], "node_review_decision_001");
    assert_eq!(state["superpowers_enabled"], true);
    assert_eq!(state["openspec_enabled"], true);
    assert_eq!(state["timeline_nodes"].as_array().unwrap().len(), 0);
    assert_eq!(state["artifact_versions"].as_array().unwrap().len(), 0);
}

#[test]
fn work_item_plan_review_complete_roundtrips() {
    let review = WorkItemPlanReviewComplete {
        verdict: WorkItemPlanReviewVerdict::PlanReopenRequired,
        review_scope: WorkItemPlanReviewScope::Item,
        target_outline_id: Some("outline_backend_api".to_string()),
        generation_round_id: "round_0001".to_string(),
        draft_id: Some("draft_0002".to_string()),
        batch_id: None,
        review_action: WorkItemPlanReviewAction::ReviseOutline,
        gates: vec![WorkItemPlanReviewGate::RequiresPlanReopen],
        affects_items: Vec::new(),
        warnings: Vec::new(),
    };
    let value = serde_json::to_value(WsOutMessage::ReviewComplete {
        node_id: "node_review_001".to_string(),
        round: 1,
        verdict: ReviewVerdictType::NeedsHuman,
        comments: "当前 item 依赖 outline 缺口，需回到 outline".to_string(),
        summary: "需要重开 Outline".to_string(),
        findings: Vec::new(),
        review_gate: ReviewGate::UserTriageRequired,
        work_item_plan_review: Some(review),
    })
    .unwrap();

    assert_eq!(value["type"], "review_complete");
    assert_eq!(
        value["work_item_plan_review"]["verdict"],
        "plan_reopen_required"
    );
    assert_eq!(value["work_item_plan_review"]["review_scope"], "item");
    assert_eq!(
        value["work_item_plan_review"]["target_outline_id"],
        "outline_backend_api"
    );
    assert_eq!(
        value["work_item_plan_review"]["review_action"],
        "revise_outline"
    );
    assert_eq!(
        value["work_item_plan_review"]["gates"][0],
        "requires_plan_reopen"
    );

    let parsed: WsOutMessage = serde_json::from_value(value).unwrap();
    match parsed {
        WsOutMessage::ReviewComplete {
            work_item_plan_review: Some(parsed_review),
            ..
        } => {
            assert_eq!(
                parsed_review.verdict,
                WorkItemPlanReviewVerdict::PlanReopenRequired
            );
            assert_eq!(
                parsed_review.gates,
                vec![WorkItemPlanReviewGate::RequiresPlanReopen]
            );
        }
        other => panic!("expected WorkItemPlan review extension, got {other:?}"),
    }

    let legacy: WsOutMessage = serde_json::from_value(serde_json::json!({
        "type": "review_complete",
        "node_id": "node_review_001",
        "round": 1,
        "verdict": "pass",
        "comments": "",
        "summary": "审核通过",
        "findings": [],
        "review_gate": "user_confirm_allowed"
    }))
    .unwrap();
    assert!(matches!(
        legacy,
        WsOutMessage::ReviewComplete {
            work_item_plan_review: None,
            ..
        }
    ));
}

#[test]
fn context_note_roundtrip() {
    let msg = WsInMessage::ContextNote {
        content: "需要支持空查询参数兜底".to_string(),
    };

    let json = serde_json::to_value(&msg).unwrap();

    assert_eq!(json["type"], "context_note");
    assert_eq!(json["content"], "需要支持空查询参数兜底");
    let back: WsInMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn start_generation_roundtrip() {
    let snapshot = ProviderConfigSnapshot {
        author: ProviderName::ClaudeCode,
        reviewer: Some(ProviderName::Codex),
        review_rounds: 1,
    };
    let msg = WsInMessage::StartGeneration {
        provider_config: snapshot,
        reviewer_enabled: true,
    };

    let json = serde_json::to_value(&msg).unwrap();

    assert_eq!(json["type"], "start_generation");
    assert_eq!(json["reviewer_enabled"], true);
    let back: WsInMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn work_item_plan_mode_messages_roundtrip() {
    assert_eq!(
        serde_json::to_value(TimelineNodeType::WorkItemPlanOutlineReview).unwrap(),
        "work_item_plan_outline_review"
    );
    assert_eq!(
        serde_json::to_value(TimelineNodeType::WorkItemGenerationMode).unwrap(),
        "work_item_generation_mode"
    );

    let select = WsInMessage::SelectWorkItemGenerationMode {
        mode: WorkItemGenerationModeDto::Serial,
    };
    let json = serde_json::to_value(&select).unwrap();
    assert_eq!(json["type"], "select_work_item_generation_mode");
    assert_eq!(json["mode"], "serial");
    let back: WsInMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, select);

    let batch: WsInMessage = serde_json::from_value(serde_json::json!({
        "type": "select_work_item_generation_mode",
        "mode": "batch"
    }))
    .unwrap();
    assert_eq!(
        batch,
        WsInMessage::SelectWorkItemGenerationMode {
            mode: WorkItemGenerationModeDto::Batch
        }
    );

    let revise = WsInMessage::RequestOutlineRevision {
        feedback: Some("拆分粒度再细一点".to_string()),
    };
    let json = serde_json::to_value(&revise).unwrap();
    assert_eq!(json["type"], "request_outline_revision");
    assert_eq!(json["feedback"], "拆分粒度再细一点");
    let back: WsInMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, revise);
}

#[test]
fn protocol_error_outbound_roundtrip() {
    let msg = WsOutMessage::ProtocolError {
        code: "INVALID_MESSAGE_FOR_STAGE".to_string(),
        message: "context_note not allowed in Running".to_string(),
        context: Some(serde_json::json!({"stage": "Running"})),
    };

    let json = serde_json::to_value(&msg).unwrap();

    assert_eq!(json["type"], "protocol_error");
    let back: WsOutMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn provider_locked_roundtrip() {
    let msg = WsOutMessage::ProviderLocked {
        snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 1,
        },
        locked_at: "2026-05-20T14:35:00Z".to_string(),
    };

    let json = serde_json::to_value(&msg).unwrap();

    assert_eq!(json["type"], "provider_locked");
    let back: WsOutMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, msg);
}

#[test]
fn choice_request_and_response_roundtrip() {
    let out = WsOutMessage::ChoiceRequest {
        id: "choice_001".to_string(),
        prompt: "请选择下一步".to_string(),
        options: vec![
            ChoiceOption {
                id: "continue".to_string(),
                label: "继续".to_string(),
                description: Some("继续当前方案".to_string()),
            },
            ChoiceOption {
                id: "stop".to_string(),
                label: "停止".to_string(),
                description: None,
            },
        ],
        allow_multiple: false,
        allow_free_text: true,
        questions: vec![ChoiceQuestion {
            id: "scope".to_string(),
            prompt: "请选择下一步".to_string(),
            options: vec![ChoiceOption {
                id: "continue".to_string(),
                label: "继续".to_string(),
                description: Some("继续当前方案".to_string()),
            }],
            allow_multiple: false,
            allow_free_text: true,
        }],
        source: "ask_user_question".to_string(),
    };

    let json = serde_json::to_value(&out).unwrap();

    assert_eq!(json["type"], "choice_request");
    assert_eq!(json["source"], "ask_user_question");
    assert_eq!(json["options"][0]["id"], "continue");
    assert_eq!(json["questions"][0]["id"], "scope");
    let back: WsOutMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, out);

    let input = WsInMessage::ChoiceResponse {
        id: "choice_001".to_string(),
        selected_option_ids: vec!["continue".to_string()],
        free_text: Some("补充说明".to_string()),
        answers: vec![ChoiceAnswer {
            question_id: "scope".to_string(),
            selected_option_ids: vec!["continue".to_string()],
            free_text: Some("补充说明".to_string()),
        }],
    };
    let json = serde_json::to_value(&input).unwrap();
    assert_eq!(json["type"], "choice_response");
    assert_eq!(json["selected_option_ids"][0], "continue");
    assert_eq!(json["answers"][0]["question_id"], "scope");
    let back: WsInMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, input);
}

#[test]
fn hello_ping_roundtrip() {
    let hello = WsInMessage::Hello {
        session_id: "sess-1".to_string(),
        last_seen_node_id: Some("node-1".to_string()),
    };

    let json = serde_json::to_value(&hello).unwrap();

    assert_eq!(json["type"], "hello");
    let back: WsInMessage = serde_json::from_value(json).unwrap();
    assert_eq!(back, hello);

    let ping = WsInMessage::Ping;
    let json = serde_json::to_value(&ping).unwrap();
    assert_eq!(json["type"], "ping");
}

#[test]
fn timeline_node_type_rename_keeps_legacy_deserialization_aliases() {
    let author = TimelineNodeType::AuthorRun;
    let json = serde_json::to_value(&author).unwrap();
    assert_eq!(json, "author_run");
    let legacy: TimelineNodeType = serde_json::from_value(serde_json::json!("generation"))
        .expect("legacy generation value should deserialize");
    assert_eq!(legacy, TimelineNodeType::AuthorRun);

    let reviewer = TimelineNodeType::ReviewerRun;
    let json = serde_json::to_value(&reviewer).unwrap();
    assert_eq!(json, "reviewer_run");
    let legacy: TimelineNodeType = serde_json::from_value(serde_json::json!("review"))
        .expect("legacy review value should deserialize");
    assert_eq!(legacy, TimelineNodeType::ReviewerRun);
}

#[test]
fn work_item_plan_candidate_dto_roundtrips_through_serde() {
    let dto = WorkItemPlanCandidateDto {
        plan: WorkItemPlanDto {
            id: "issue_work_item_plan_0001".to_string(),
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
        work_items: vec![WorkItemCandidateDto {
            id: "wi_001".to_string(),
            kind: "backend".to_string(),
            title: "实现爬楼梯问题".to_string(),
            depends_on: vec!["wi_000".to_string()],
            exclusive_write_scopes: vec!["src/product/stairs.rs".to_string()],
            verification_plan_ref: Some("vp_001".to_string()),
            meta: WorkItemCandidateMetaDto {
                reverted: true,
                revert_feedback: Some("需要细化边界条件".to_string()),
            },
        }],
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
                label: "人工检查".to_string(),
                instructions: "检查输出".to_string(),
                required: false,
            }],
            required_gates: vec![],
            risk_notes: vec![],
            confidence: "high".to_string(),
            fallback_policy: "manual_gate".to_string(),
        }],
        repository_profile: Some(RepositoryProfileDto {
            profile_id: "rp_001".to_string(),
            repository_id: "repo_001".to_string(),
            languages: vec!["rust".to_string()],
            frameworks: vec![],
            package_managers: vec!["cargo".to_string()],
            test_frameworks: vec![],
            build_systems: vec!["cargo".to_string()],
            detected_layers: vec!["backend".to_string()],
            split_recommendation: "backend_only".to_string(),
            confidence: "high".to_string(),
        }),
        validator_findings: vec![ValidatorFindingDto {
            severity: "warning".to_string(),
            code: "W001".to_string(),
            message: "注意边界条件".to_string(),
            work_item_ids: vec!["wi_001".to_string()],
        }],
    };

    let json = serde_json::to_value(&dto).unwrap();
    let back: WorkItemPlanCandidateDto = serde_json::from_value(json.clone()).unwrap();
    assert_eq!(back, dto);

    // 显式断言 plan 文档约定的字段路径
    assert_eq!(json["plan"]["id"], "issue_work_item_plan_0001");
    assert_eq!(json["plan"]["status"], "draft");
    assert_eq!(json["work_items"][0]["id"], "wi_001");
    assert_eq!(json["work_items"][0]["kind"], "backend");
    assert_eq!(json["work_items"][0]["verification_plan_ref"], "vp_001");
    assert_eq!(json["work_items"][0]["meta"]["reverted"], true);
    assert_eq!(
        json["work_items"][0]["meta"]["revert_feedback"],
        "需要细化边界条件"
    );
    assert!(json["verification_plans"][0]["plan_ref"] == "vp_001");
    assert!(json["repository_profile"]["profile_id"] == "rp_001");
    assert!(json["validator_findings"][0]["code"] == "W001");
}

#[test]
fn revert_work_item_message_deserializes() {
    let input: WsInMessage = serde_json::from_value(serde_json::json!({
        "type": "revert_work_item",
        "work_item_id": "wi_001",
        "feedback": "需要回退",
        "clear": false
    }))
    .unwrap();

    assert!(matches!(
        input,
        WsInMessage::RevertWorkItem {
            work_item_id,
            feedback,
            clear,
        } if work_item_id == "wi_001" && feedback.as_deref() == Some("需要回退") && !clear
    ));
}

#[test]
fn artifact_payload_markdown_variant_serializes_to_flat_json() {
    let payload = ArtifactPayload::Markdown {
        markdown: "# Plan\n".to_string(),
        diff: Some("@@ -1 +1 @@\n-old\n+new".to_string()),
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert_eq!(json["markdown"], "# Plan\n");
    assert_eq!(json["diff"], "@@ -1 +1 @@\n-old\n+new");

    let payload_without_diff = ArtifactPayload::Markdown {
        markdown: "# Plan\n".to_string(),
        diff: None,
    };
    let json_without_diff = serde_json::to_value(&payload_without_diff).unwrap();
    assert_eq!(
        json_without_diff,
        serde_json::json!({"markdown": "# Plan\n"})
    );
}

#[test]
fn artifact_payload_candidate_variant_serializes_to_flat_json() {
    let payload = ArtifactPayload::WorkItemPlanCandidate {
        candidate: Box::new(WorkItemPlanCandidateDto {
            plan: WorkItemPlanDto {
                id: "issue_work_item_plan_0001".to_string(),
                status: "draft".to_string(),
                options: WorkItemSplitOptionsDto {
                    include_integration_tests: false,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                dependency_graph: vec![],
            },
            work_items: vec![WorkItemCandidateDto {
                id: "wi_001".to_string(),
                kind: "backend".to_string(),
                title: "实现爬楼梯问题".to_string(),
                depends_on: vec![],
                exclusive_write_scopes: vec!["src/product/stairs.rs".to_string()],
                verification_plan_ref: None,
                meta: WorkItemCandidateMetaDto {
                    reverted: false,
                    revert_feedback: None,
                },
            }],
            verification_plans: vec![],
            repository_profile: None,
            validator_findings: vec![],
        }),
    };
    let json = serde_json::to_value(&payload).unwrap();
    assert!(json.get("candidate").is_some());
    assert_eq!(json["candidate"]["plan"]["id"], "issue_work_item_plan_0001");
    assert_eq!(json["candidate"]["plan"]["status"], "draft");
    assert_eq!(json["candidate"]["work_items"][0]["id"], "wi_001");
    assert_eq!(
        json["candidate"]["work_items"][0]["meta"]["reverted"],
        false
    );
    assert!(!json.as_object().unwrap().contains_key("markdown"));
}

#[test]
fn artifact_update_carries_candidate_payload_as_expected_json() {
    let candidate = WorkItemPlanCandidateDto {
        plan: WorkItemPlanDto {
            id: "issue_work_item_plan_0001".to_string(),
            status: "draft".to_string(),
            options: WorkItemSplitOptionsDto {
                include_integration_tests: false,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
            dependency_graph: vec![],
        },
        work_items: vec![WorkItemCandidateDto {
            id: "wi_001".to_string(),
            kind: "backend".to_string(),
            title: "实现爬楼梯问题".to_string(),
            depends_on: vec![],
            exclusive_write_scopes: vec!["src/product/stairs.rs".to_string()],
            verification_plan_ref: None,
            meta: WorkItemCandidateMetaDto {
                reverted: false,
                revert_feedback: None,
            },
        }],
        verification_plans: vec![],
        repository_profile: None,
        validator_findings: vec![],
    };
    let out = WsOutMessage::ArtifactUpdate {
        version: 7,
        payload: ArtifactPayload::WorkItemPlanCandidate {
            candidate: Box::new(candidate.clone()),
        },
    };
    let json = serde_json::to_value(out).unwrap();
    assert_eq!(json["type"], "artifact_update");
    assert_eq!(json["version"], 7);
    assert_eq!(json["candidate"]["plan"]["id"], "issue_work_item_plan_0001");
    assert_eq!(json["candidate"]["work_items"][0]["id"], "wi_001");
    let parsed_candidate: WorkItemPlanCandidateDto =
        serde_json::from_value(json["candidate"].clone()).unwrap();
    assert_eq!(parsed_candidate.plan.id, "issue_work_item_plan_0001");
    assert_eq!(parsed_candidate.work_items[0].id, "wi_001");
}

#[test]
fn artifact_update_with_markdown_payload_serializes_flat() {
    let out = WsOutMessage::ArtifactUpdate {
        version: 3,
        payload: ArtifactPayload::Markdown {
            markdown: "# Markdown payload\n".to_string(),
            diff: Some("@@ -1 +1 @@\n-old\n+new".to_string()),
        },
    };
    let json = serde_json::to_value(out).unwrap();
    assert_eq!(json["type"], "artifact_update");
    assert_eq!(json["version"], 3);
    assert_eq!(json["markdown"], "# Markdown payload\n");
    assert_eq!(json["diff"], "@@ -1 +1 @@\n-old\n+new");
    assert!(!json.as_object().unwrap().contains_key("candidate"));
}

#[test]
fn session_state_artifact_accepts_markdown_payload() {
    let state = WsOutMessage::SessionState {
        session_id: "workspace_session_0001".to_string(),
        workspace_type: WorkspaceType::Story,
        stage: "author_confirm".to_string(),
        superpowers_enabled: true,
        openspec_enabled: true,
        messages: Vec::new(),
        checkpoints: Vec::new(),
        artifact: Some(ArtifactPayload::Markdown {
            markdown: "# Story".to_string(),
            diff: None,
        }),
        providers: crate::web::workspace_ws_types::WsProviderConfig {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
        },
        timeline_nodes: Vec::new(),
        active_node_id: None,
        artifact_versions: Vec::new(),
        artifact_version_summaries: Vec::new(),
        timeline_node_details: std::collections::HashMap::new(),
        timeline_node_summaries: std::collections::HashMap::new(),
        active_run_id: None,
    };
    let json = serde_json::to_value(state).unwrap();
    assert_eq!(json["artifact"]["markdown"], "# Story");
    assert!(json["artifact"]["diff"].is_null());
}

#[test]
fn artifact_version_roundtrips_with_markdown_payload() {
    let version = ArtifactVersion {
        version: 1,
        payload: ArtifactPayload::Markdown {
            markdown: "# Artifact version\n".to_string(),
            diff: Some("diff".to_string()),
        },
        generated_by: ProviderName::ClaudeCode,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-06-01T00:00:00Z".to_string(),
        source_node_id: "node_001".to_string(),
    };
    let json = serde_json::to_value(&version).unwrap();
    assert_eq!(json["markdown"], "# Artifact version\n");
    assert_eq!(json["diff"], "diff");
    assert!(!json.as_object().unwrap().contains_key("payload"));

    let back: ArtifactVersion = serde_json::from_value(json).unwrap();
    assert_eq!(back, version);
}
