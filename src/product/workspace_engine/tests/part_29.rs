fn linked_workspace_target(
    workspace_type: WorkspaceType,
    relation: crate::product::models::WorkspaceSessionRelation,
    entity_id: &str,
) -> LinkedWorkspaceAmendmentTarget {
    LinkedWorkspaceAmendmentTarget {
        entity_id: entity_id.to_string(),
        workspace_type,
        relation,
    }
}

fn linked_workspace_timeline(node_id: &str) -> Vec<TimelineNode> {
    vec![TimelineNode {
        node_id: node_id.to_string(),
        node_type: TimelineNodeType::HumanConfirm,
        agent: None,
        stage: WsWorkspaceStage::HumanConfirm,
        round: None,
        status: TimelineNodeStatus::Active,
        title: "等待确认".to_string(),
        summary: Some("等待人工确认修订".to_string()),
        started_at: "2026-07-20T00:00:00Z".to_string(),
        completed_at: None,
        duration_ms: None,
        artifact_ref: Some("artifact_0007/v7".to_string()),
        provider_config_snapshot: ProviderConfigSnapshot {
            author: ProviderName::ClaudeCode,
            reviewer: Some(ProviderName::Codex),
            review_rounds: 2,
        },
        retry: None,
    }]
}

fn linked_workspace_link(
    id: &str,
    parent_session_id: &str,
    child_session_id: &str,
    relation: crate::product::models::WorkspaceSessionRelation,
) -> crate::product::models::WorkspaceSessionLink {
    crate::product::models::WorkspaceSessionLink {
        id: id.to_string(),
        relation,
        parent_session_id: parent_session_id.to_string(),
        child_session_id: child_session_id.to_string(),
        trigger: crate::product::models::WorkspaceSessionLinkTrigger {
            attempt_id: "coding_attempt_0001".to_string(),
            unit_run_id: "coding_unit_run_0001".to_string(),
            review_id: Some("code_review_0001".to_string()),
            finding_id: "finding_0001".to_string(),
            repair_request_id: "plan_repair_request_0001".to_string(),
            amendment_id: "plan_amendment_0001".to_string(),
            fingerprint: "fingerprint_0001".to_string(),
            base_plan_revision_id: "plan_revision_0001".to_string(),
        },
        return_context: crate::product::models::WorkspaceReturnContext {
            original_attempt_id: "coding_attempt_0001".to_string(),
            original_unit_run_id: "coding_unit_run_0001".to_string(),
            timeline_anchor_id: "finding_0001".to_string(),
            original_route: format!("/workbench/workspace/{parent_session_id}"),
        },
        created_at: "2026-07-20T00:00:00Z".to_string(),
    }
}

#[test]
fn workspace_session_link_roundtrips_for_story_design_and_work_item_relations() {
    for (workspace_type, relation, entity_id) in [
        (
            WorkspaceType::Story,
            crate::product::models::WorkspaceSessionRelation::StoryAmendment,
            "story_spec_0001",
        ),
        (
            WorkspaceType::Design,
            crate::product::models::WorkspaceSessionRelation::DesignAmendment,
            "design_spec_0001",
        ),
        (
            WorkspaceType::WorkItem,
            crate::product::models::WorkspaceSessionRelation::PlanRepair,
            "work_item_0001",
        ),
    ] {
        let target = linked_workspace_target(workspace_type, relation.clone(), entity_id);
        assert_eq!(
            serde_json::from_value::<LinkedWorkspaceAmendmentTarget>(
                serde_json::to_value(&target).unwrap()
            )
            .unwrap(),
            target
        );
        let link = linked_workspace_link(
            &format!("workspace_session_link_{entity_id}"),
            "workspace_session_parent_0001",
            &format!("workspace_session_{entity_id}"),
            relation,
        );
        assert_eq!(
            serde_json::from_value::<crate::product::models::WorkspaceSessionLink>(
                serde_json::to_value(&link).unwrap()
            )
            .unwrap(),
            link
        );
    }
}

#[test]
fn linked_workspace_timeline_and_artifact_binding_restore_for_all_artifact_types() {
    let (tmp, _) = setup();
    let app_paths = ProductAppPaths::new(tmp.path().join(".aria"));
    let lifecycle = LifecycleStore::new(app_paths.clone());
    let parent = lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "work_item_plan_0001".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
            "workspace_session_parent_0001".to_string(),
        )
        .unwrap();

    for (index, (workspace_type, relation, entity_id)) in [
        (
            WorkspaceType::Story,
            crate::product::models::WorkspaceSessionRelation::StoryAmendment,
            "story_spec_0001",
        ),
        (
            WorkspaceType::Design,
            crate::product::models::WorkspaceSessionRelation::DesignAmendment,
            "design_spec_0001",
        ),
        (
            WorkspaceType::WorkItem,
            crate::product::models::WorkspaceSessionRelation::PlanRepair,
            "work_item_0001",
        ),
    ]
    .into_iter()
    .enumerate()
    {
        let child_session_id = format!("workspace_session_linked_{:04}", index + 1);
        let child = lifecycle
            .create_workspace_session_with_id(
                CreateWorkspaceSessionInput {
                    project_id: parent.project_id.clone(),
                    issue_id: parent.issue_id.clone(),
                    entity_id: entity_id.to_string(),
                    workspace_type: workspace_type.clone(),
                    author_provider: ProviderName::ClaudeCode,
                    reviewer_provider: ProviderName::Codex,
                    review_rounds: 2,
                    superpowers_enabled: true,
                    openspec_enabled: true,
                },
                child_session_id.clone(),
            )
            .unwrap();
        let link = linked_workspace_link(
            &format!("workspace_session_link_shared_{:04}", index + 1),
            &parent.id,
            &child.id,
            relation,
        );
        lifecycle
            .put_session_link(&parent.project_id, &parent.issue_id, &link)
            .unwrap();
        lifecycle
            .append_artifact_version(
                &child.id,
                ArtifactVersion {
                    version: 7,
                    payload: ArtifactPayload::Markdown {
                        markdown: format!("# {entity_id} amendment"),
                        diff: None,
                    },
                    generated_by: ProviderName::ClaudeCode,
                    reviewed_by: Some(ProviderName::Codex),
                    review_verdict: None,
                    confirmed_by: None,
                    is_current: true,
                    created_at: "2026-07-20T00:00:00Z".to_string(),
                    source_node_id: format!("timeline_node_linked_{:04}", index + 1),
                },
            )
            .unwrap();
        let timeline_nodes = linked_workspace_timeline(&format!(
            "timeline_node_linked_{:04}",
            index + 1
        ));
        lifecycle
            .save_timeline_nodes(&child.id, &timeline_nodes)
            .unwrap();
        lifecycle
            .update_workspace_session_status(
                &child.id,
                crate::product::models::WorkspaceSessionStatus::WaitingForHuman,
            )
            .unwrap();

        let restarted = LifecycleStore::new(app_paths.clone());
        let restored = restore_linked_workspace_snapshot(
            &restarted,
            &parent.project_id,
            &parent.issue_id,
            &link,
        )
        .unwrap();

        assert_eq!(restored.workspace_type, workspace_type);
        assert_eq!(restored.artifact_version_id, Some(7));
        assert_eq!(restored.timeline_nodes, timeline_nodes);
        assert_eq!(
            restored.selected_timeline_node_id,
            Some(format!("timeline_node_linked_{:04}", index + 1))
        );
        assert_eq!(
            restored.human_confirm_state,
            crate::product::models::WorkspaceSessionStatus::WaitingForHuman
        );
    }
}

#[tokio::test]
async fn linked_workspace_amendment_creates_story_and_design_children_without_publishing() {
    for (workspace_type, relation, entity_id) in [
        (
            WorkspaceType::Story,
            crate::product::models::WorkspaceSessionRelation::StoryAmendment,
            "story_spec_0001",
        ),
        (
            WorkspaceType::Design,
            crate::product::models::WorkspaceSessionRelation::DesignAmendment,
            "design_spec_0001",
        ),
    ] {
        let (tmp, lifecycle, _revision_store, mut parent) = plan_repair_parent_engine();
        let repair_child = parent
            .start_plan_repair(plan_repair_fixture(
                "plan_repair_request_0001",
                "fingerprint_upgrade",
            ))
            .await
            .unwrap();
        let repair_child_id = repair_child.id.clone();
        let repair_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, repair_child);
        let target = linked_workspace_target(workspace_type.clone(), relation.clone(), entity_id);

        let first = repair_engine
            .start_linked_workspace_amendment(target.clone())
            .unwrap();
        let duplicate = repair_engine
            .start_linked_workspace_amendment(target)
            .unwrap();

        assert_eq!(duplicate, first);
        assert_eq!(first.workspace_type, workspace_type);
        assert_eq!(first.link.relation, relation);
        assert_eq!(first.link.parent_session_id, repair_child_id);
        assert_eq!(first.artifact_version_id, None);
        assert!(first.timeline_nodes.is_empty());
        assert_eq!(
            first.human_confirm_state,
            crate::product::models::WorkspaceSessionStatus::Open
        );
        assert!(
            lifecycle
                .list_artifact_versions(&first.link.child_session_id)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            lifecycle
                .list_workspace_sessions("project_0001", "issue_0001")
                .unwrap()
                .len(),
            3
        );
    }
}

#[tokio::test]
async fn linked_workspace_amendment_fails_closed_for_noncanonical_target_or_parent() {
    let (tmp, lifecycle, _revision_store, mut parent) = plan_repair_parent_engine();
    let repair_child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_upgrade_invalid",
        ))
        .await
        .unwrap();
    let repair_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, repair_child);

    for target in [
        linked_workspace_target(
            WorkspaceType::Story,
            crate::product::models::WorkspaceSessionRelation::DesignAmendment,
            "story_spec_0001",
        ),
        linked_workspace_target(
            WorkspaceType::Story,
            crate::product::models::WorkspaceSessionRelation::StoryAmendment,
            "story_spec_unknown",
        ),
        linked_workspace_target(
            WorkspaceType::WorkItem,
            crate::product::models::WorkspaceSessionRelation::PlanRepair,
            "work_item_0001",
        ),
    ] {
        assert!(
            repair_engine
                .start_linked_workspace_amendment(target)
                .is_err()
        );
    }
    assert_eq!(
        lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .unwrap()
            .len(),
        2
    );

    assert!(
        parent
            .start_linked_workspace_amendment(linked_workspace_target(
                WorkspaceType::Story,
                crate::product::models::WorkspaceSessionRelation::StoryAmendment,
                "story_spec_0001",
            ))
            .is_err()
    );
}

#[tokio::test]
async fn linked_workspace_amendment_rejects_persisted_child_with_wrong_target_entity() {
    let (tmp, lifecycle, _revision_store, mut parent) = plan_repair_parent_engine();
    let repair_child = parent
        .start_plan_repair(plan_repair_fixture(
            "plan_repair_request_0001",
            "fingerprint_upgrade_target_mismatch",
        ))
        .await
        .unwrap();
    let repair_link = lifecycle.get_session_link(&repair_child.id).unwrap();
    let amendment_id = repair_link.trigger.amendment_id.clone();
    let identity_hash = crate::cross_cutting::document_ops::compute_sha256(
        format!(
            "{}\nstory_amendment\nstory_spec_0001\n{}",
            repair_child.id, amendment_id
        )
        .as_bytes(),
    );
    let child_session_id = format!("workspace_session_story_amendment_{identity_hash}");
    lifecycle
        .create_workspace_session_with_id(
            CreateWorkspaceSessionInput {
                project_id: repair_child.project_id.clone(),
                issue_id: repair_child.issue_id.clone(),
                entity_id: "story_spec_wrong".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: ProviderName::Codex,
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
            },
            child_session_id.clone(),
        )
        .unwrap();
    let mut corrupt_link = repair_link;
    corrupt_link.id = format!("workspace_session_link_story_amendment_{identity_hash}");
    corrupt_link.relation = crate::product::models::WorkspaceSessionRelation::StoryAmendment;
    corrupt_link.parent_session_id = repair_child.id.clone();
    corrupt_link.child_session_id = child_session_id;
    corrupt_link.return_context.original_route =
        format!("/workbench/workspace/{}", repair_child.id);
    lifecycle
        .put_session_link(
            &repair_child.project_id,
            &repair_child.issue_id,
            &corrupt_link,
        )
        .unwrap();
    let repair_engine = plan_repair_restarted_child_engine(&tmp, &lifecycle, repair_child);

    assert!(
        repair_engine
            .start_linked_workspace_amendment(linked_workspace_target(
                WorkspaceType::Story,
                crate::product::models::WorkspaceSessionRelation::StoryAmendment,
                "story_spec_0001",
            ))
            .is_err()
    );
}
