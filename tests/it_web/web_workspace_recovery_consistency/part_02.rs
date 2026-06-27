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
            },
            retry: None,
        }],
        active_node_id: Some("node_001".to_string()),
        artifact_versions: vec![],
        artifact_version_summaries: vec![],
        timeline_node_details: HashMap::new(),
        timeline_node_summaries: HashMap::new(),
        active_run_id: None,
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

fn valid_draft_output(outline_id: &str) -> Value {
    json!({
        "draft": {
            "outline_id": outline_id,
            "title": "实现后端登录会话 API",
            "kind": "backend",
            "goal": "提供登录会话过期检测与刷新相关 API。",
            "implementation_context": "实现 product service 与 web handler，返回稳定 DTO。",
            "exclusive_write_scopes": ["src/product/session.rs", "src/web/session_handlers.rs"],
            "forbidden_write_scopes": ["web/**"],
            "depends_on_outline_ids": [],
            "required_handoff_from_outline_ids": [],
            "handoff_summary": "输出 SessionStatusDto 与错误语义。",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_backend_session",
                        "label": "cargo test session",
                        "command": "cargo test --locked --lib session",
                        "cwd": "",
                        "purpose": "验证后端 session 逻辑",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_backend_session"]
            }
        }
    })
}

fn valid_frontend_draft_output() -> Value {
    json!({
        "draft": {
            "outline_id": "outline_frontend_expiry",
            "title": "实现前端会话过期提示",
            "kind": "frontend",
            "goal": "在前端展示会话过期提示并触发重新登录入口。",
            "implementation_context": "消费后端会话状态 DTO，展示稳定 UI 状态。",
            "exclusive_write_scopes": ["web/src/session/expiry.ts"],
            "forbidden_write_scopes": ["src/product/**"],
            "depends_on_outline_ids": ["outline_backend_session"],
            "required_handoff_from_outline_ids": ["outline_backend_session"],
            "handoff_summary": "输出前端会话过期提示组件。",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_frontend_session",
                        "label": "pnpm web test",
                        "command": "pnpm -C web test",
                        "cwd": "",
                        "purpose": "验证前端 session UI",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_frontend_session"]
            }
        }
    })
}

fn valid_integration_draft_output() -> Value {
    json!({
        "draft": {
            "outline_id": "outline_integration_session",
            "title": "集成测试：会话过期端到端",
            "kind": "integration",
            "goal": "覆盖会话过期到前端提示的贯通路径。",
            "implementation_context": "覆盖后端会话 DTO 到前端提示的集成路径。",
            "exclusive_write_scopes": ["tests/session/expiry.rs"],
            "forbidden_write_scopes": [],
            "depends_on_outline_ids": ["outline_frontend_expiry"],
            "required_handoff_from_outline_ids": ["outline_frontend_expiry"],
            "handoff_summary": "输出端到端验证覆盖。",
            "verification_plan": {
                "commands": [
                    {
                        "id": "cmd_integration_session",
                        "label": "cargo test session integration",
                        "command": "cargo test --locked --test it_web session",
                        "cwd": "",
                        "purpose": "验证会话过期贯通路径",
                        "required": true,
                        "timeout_seconds": 120,
                        "safety": "approved",
                        "source": "provider"
                    }
                ],
                "manual_checks": [],
                "required_gates": ["cmd_integration_session"]
            }
        }
    })
}

#[tokio::test]
#[ignore = "legacy full-candidate revert recovery is superseded by WP2 outline generation; WP3+ will replace this coverage"]
async fn reconnect_preserves_revert_marks_from_current_artifact_version() {
    let _guard = WS_TEST_LOCK.lock().await;
    let (app, _repo) = app_with_confirmed_story_and_design(valid_split_output()).await;

    let (_status, prepare_resp) = request_json(
        app.clone(),
        Method::POST,
        "/api/projects/project_0001/issues/issue_0001/work-item-plans:prepare",
        json!({
            "title": "重连恢复 revert 标记测试",
            "story_spec_ids": ["story_spec_0001"],
            "design_spec_ids": ["design_spec_0001"],
            "author_provider": "fake",
            "reviewer_provider": null,
            "review_rounds": 1,
            "superpowers_enabled": false,
            "openspec_enabled": true,
            "include_integration_tests": true,
            "include_e2e_tests": false,
            "force_frontend_backend_split": true,
            "require_execution_plan_confirm": false
        }),
    )
    .await;
    let session_id = prepare_resp["workspace_session"]["workspace_session_id"]
        .as_str()
        .unwrap()
        .to_string();

    let mut ws = connect_ws(app.clone(), &session_id).await;
    ws.send(Message::Text(
        json!({
            "type": "start_generation",
            "provider_config": { "author": "fake", "reviewer": null, "review_rounds": 0 },
            "reviewer_enabled": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send start_generation");

    let messages = recv_ws_until(&mut ws, Duration::from_secs(15), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
            && msgs
                .iter()
                .any(|m| m["type"] == "stage_change" && m["stage"] == "author_confirm")
    })
    .await;
    let first_work_item_id = messages
        .iter()
        .find(|m| m["type"] == "artifact_update")
        .unwrap()["candidate"]["work_items"][0]["id"]
        .as_str()
        .unwrap()
        .to_string();

    ws.send(Message::Text(
        json!({
            "type": "revert_work_item",
            "work_item_id": first_work_item_id,
            "feedback": "拆得太粗",
            "clear": false
        })
        .to_string()
        .into(),
    ))
    .await
    .expect("send revert_work_item");

    let _messages = recv_ws_until(&mut ws, Duration::from_secs(5), |msgs| {
        msgs.iter().any(|m| m["type"] == "artifact_update")
    })
    .await;
    ws.close(None).await.ok();

    // 重连：服务端应发送 SessionState，其中当前 artifact version 保留 revert 标记
    let mut ws2 = connect_ws(app, &session_id).await;
    let state_messages = recv_ws_until(&mut ws2, Duration::from_secs(5), |msgs| {
        msgs.iter().any(|m| m["type"] == "session_state")
    })
    .await;
    let session_state = state_messages
        .iter()
        .find(|m| m["type"] == "session_state")
        .expect("session_state after reconnect");
    let reverted_item = session_state["artifact"]["candidate"]["work_items"]
        .as_array()
        .unwrap()
        .iter()
        .find(|w| w["id"] == first_work_item_id)
        .expect("work item still in recovered candidate");
    assert_eq!(reverted_item["meta"]["reverted"], true);
    assert_eq!(reverted_item["meta"]["revert_feedback"], "拆得太粗");

    ws2.close(None).await.ok();
}
