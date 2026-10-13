// cascade_d1_1（fix-cascade-deletion Task D1.1：真实 manifest、审计子依赖与根最后）。
// 独立文件经 include! 引入 tests.rs（与 tool_policy_audit.rs 同惯例，large_file_guard）。

// DurableToolPolicyEvent/ToolPolicyAuditSink 已由 tests/tool_policy_audit.rs 的
// 模块级 use 引入（同一 tests 模块内 use 全模块可见），此处只补 ProviderStartAudit。
use crate::cross_cutting::tool_policy_audit::ProviderStartAudit;
use crate::product::advance_store::{
    AdvanceInitializationJournal, AdvanceInitializationPhase, AdvanceRecord, AdvanceStatus,
    AdvanceStore,
};
use crate::product::entity_deletion_store::{EntityDeletionManifest, EntityReference};
use crate::product::models::automation::{OperationState, RetryInitializationRecord};

fn d1_1_provider_start_event(session_id: &str) -> DurableToolPolicyEvent {
    DurableToolPolicyEvent::ProviderStart(ProviderStartAudit {
        provider: "codex".to_string(),
        role: "orchestrator".to_string(),
        workspace_session_id: session_id.to_string(),
        provider_session_id: "thread-d11".to_string(),
        tool_policy_canonical_digest: "sha256:d11-digest".to_string(),
        argv: vec!["--mode".to_string(), "rpc".to_string()],
        sandbox: None,
        approval_policy: None,
        provider_version: "provider 1.2.3".to_string(),
        adapter_dialect: "codex-app-server-rpc".to_string(),
        lc_projection: None,
    })
}

/// Plan 全闭包种子（真实 writer 落档）：摘要缺项 WI（source 有、work_item_ids
/// 无）、VP/profile/run 配置 refs、plan/WI 双 session、review、两个全局 session
/// 审计分区、target repository、advance record/journal/retry（D1.2c 真实接口）。
struct D1_1PlanClosure {
    run_id: String,
    profile_id: String,
    vp_id: String,
    plan_session_id: String,
    wi_session_id: String,
    review_id: String,
    target_repository_id: String,
    advance_id: String,
    retry_command_id: String,
}

fn d1_1_plan_closure(store: &LifecycleStore, plan_id: &str) -> D1_1PlanClosure {
    // split provider run（真实 legacy 落档 writer；该路径无归属字段，归属由
    // Plan/VP/profile 配置 refs 表达——D1.3b 关切②按 plan §0.5 refs join 处置，
    // 无 refs 的 issue 级 run 不进任何实体 manifest、不被实体删除误伤）。
    let run_id = store
        .save_work_item_split_provider_run(
            PROJECT_ID,
            ISSUE_ID,
            &ProviderName::Codex,
            "d1_1 split prompt",
            &serde_json::json!({"plan": plan_id}),
        )
        .unwrap();

    let profile = store
        .create_repository_profile(CreateRepositoryProfileInput {
            id: Some("repository_profile_0001".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            provider_run_ref: Some(run_id.clone()),
            languages: Vec::new(),
            frameworks: Vec::new(),
            package_managers: Vec::new(),
            test_frameworks: Vec::new(),
            build_systems: Vec::new(),
            verification_capabilities: Vec::new(),
            detected_layers: Vec::new(),
            split_recommendation: "single".to_string(),
            confidence: crate::product::models::RepositoryProfileConfidence::Low,
            uncertainties: Vec::new(),
        })
        .unwrap();

    // 摘要缺项：work_item_ids 只列 work_item_0001；VP 不进 verification_plan_ids。
    store
        .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
            id: Some(plan_id.to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: d1_3a_plan_options(),
            status: crate::product::models::IssueWorkItemPlanStatus::Draft,
            work_item_ids: vec!["work_item_0001".to_string()],
            repository_profile_ref: Some(profile.id.clone()),
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: Some(run_id.clone()),
            validator_findings: Vec::new(),
        })
        .unwrap();

    let listed = d1_3a_wi(store, "work_item_0001", plan_id);
    let unlisted = d1_3a_wi(store, "work_item_0002", plan_id);

    // target repository（park-1 compile 派生字段）。
    let target_repository_id = LogicalRepositoryId(Uuid::new_v4()).0.to_string();
    let mut compiled = listed.clone();
    compiled.target_repository_id = Some(LogicalRepositoryId(
        Uuid::parse_str(&target_repository_id).unwrap(),
    ));
    store.upsert_compiled_work_item_record(compiled).unwrap();

    let vp = store
        .create_verification_plan(CreateVerificationPlanInput {
            id: Some("verification_plan_0001".to_string()),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: listed.id.clone(),
            repository_profile_ref: Some(profile.id.clone()),
            provider_run_ref: Some(run_id.clone()),
            scope: crate::product::models::VerificationScope::Unit,
            commands: Vec::new(),
            manual_checks: Vec::new(),
            required_gates: Vec::new(),
            risk_notes: Vec::new(),
            confidence: crate::product::models::RepositoryProfileConfidence::Low,
            fallback_policy: crate::product::models::VerificationFallbackPolicy::ManualGate,
        })
        .unwrap();

    let plan_session = create_session(store, plan_id, WorkspaceType::WorkItemPlan);
    let wi_session = create_session(store, &unlisted.id, WorkspaceType::WorkItem);

    let review = store
        .append_provider_review_round(AppendProviderReviewRoundInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            session_id: plan_session.id.clone(),
            round_index: 1,
            author_provider: ProviderName::Codex,
            reviewer_provider: None,
            review_result: "approved".to_string(),
            revision_result: "none".to_string(),
        })
        .unwrap();

    // 两个全局 session 审计分区（§0.5「split-run/tool-policy audit」D1.1 行）：
    // 真实 writer 落档，按 owned session 分区清理。
    store
        .append(
            &plan_session.id,
            1,
            d1_1_provider_start_event(&plan_session.id),
        )
        .unwrap();
    store
        .begin_work_item_split_provider_run(
            PROJECT_ID,
            ISSUE_ID,
            &ProviderName::Codex,
            &wi_session.id,
        )
        .unwrap();

    // advance record/journal/retry（D1.2c 真实接口；retry 落档同 D1.2c fixture
    // 按 §0.5 固定路径——retry_path 私有不可跨模块）。
    let advance = AdvanceStore::new(store.app_paths());
    let record = AdvanceRecord {
        id: "advance_command_0001".to_string(),
        command_id: "command_0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        plan_id: plan_id.to_string(),
        plan_revision_id: "revision_0001".to_string(),
        attempt_id: Some("attempt_0001".to_string()),
        target_attempts: Vec::new(),
        status: AdvanceStatus::Ready,
        workspace_entry: Some("/workspaces/attempt_0001".to_string()),
        error: None,
        created_at: "2026-10-13T00:00:00Z".to_string(),
        updated_at: "2026-10-13T00:00:01Z".to_string(),
    };
    advance.put_record(&record).unwrap();
    advance
        .save_advance_initialization(
            &record,
            &AdvanceInitializationJournal {
                advance_id: record.id.clone(),
                plan_id: plan_id.to_string(),
                attempt_id: "attempt_0001".to_string(),
                target_attempt_ids: Vec::new(),
                phase: AdvanceInitializationPhase::JournalPrepared,
                error: None,
                created_at: "2026-10-13T00:00:00Z".to_string(),
                updated_at: "2026-10-13T00:00:01Z".to_string(),
            },
        )
        .unwrap();
    write_json(
        &store
            .app_paths()
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("advance-retries")
            .join("retry_cmd_0001.json"),
        &RetryInitializationRecord {
            retry_id: "advance_retry_retry_cmd_0001".to_string(),
            command_id: "retry_cmd_0001".to_string(),
            advance_id: record.id.clone(),
            attempt_id: "attempt_0001".to_string(),
            state: OperationState::Accepted,
            checkpoint: AdvanceInitializationPhase::JournalPrepared,
            created_at: "2026-10-13T00:00:00Z".to_string(),
            updated_at: "2026-10-13T00:00:01Z".to_string(),
        },
    )
    .unwrap();

    D1_1PlanClosure {
        run_id,
        profile_id: profile.id,
        vp_id: vp.id,
        plan_session_id: plan_session.id,
        wi_session_id: wi_session.id,
        review_id: review.id,
        target_repository_id,
        advance_id: record.id,
        retry_command_id: "retry_cmd_0001".to_string(),
    }
}

#[test]
fn cascade_d1_1_real_owner_manifest() {
    let (_tmp, store) = setup();
    let closed = d1_1_plan_closure(&store, "issue_work_item_plan_0001");

    // 同 issue 他 Plan 并存：自己的 WI/session 与无归属 legacy run，不进 A 的 manifest。
    d1_3a_plan(&store, "issue_work_item_plan_0002", Vec::new());
    d1_3a_wi(&store, "work_item_0003", "issue_work_item_plan_0002");
    let other_session = create_session(
        &store,
        "issue_work_item_plan_0002",
        WorkspaceType::WorkItemPlan,
    );
    let unattributed_run = store
        .save_work_item_split_provider_run(
            PROJECT_ID,
            ISSUE_ID,
            &ProviderName::Codex,
            "other split prompt",
            &serde_json::json!({}),
        )
        .unwrap();

    // —— Plan A：真实归属 manifest（不依赖健康摘要）——
    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001");
    let manifest = store.collect_entity_deletion_manifest(&scope).unwrap();
    assert_eq!(
        manifest.work_item_ids,
        vec!["work_item_0001".to_string(), "work_item_0002".to_string()],
        "摘要缺项 WI 必须凭真实 source 归属补齐"
    );
    assert!(manifest.session_ids.contains(&closed.plan_session_id));
    assert!(manifest.session_ids.contains(&closed.wi_session_id));
    assert!(!manifest.session_ids.contains(&other_session.id));
    assert_eq!(manifest.review_ids, vec![closed.review_id.clone()]);
    assert_eq!(manifest.provider_run_ids, vec![closed.run_id.clone()]);
    assert!(
        !manifest.provider_run_ids.contains(&unattributed_run),
        "无归属字段的 issue 级 run 不得被 refs 之外的实体认领"
    );
    assert_eq!(manifest.verification_plan_ids, vec![closed.vp_id.clone()]);
    assert_eq!(
        manifest.repository_profile_ids,
        vec![closed.profile_id.clone()]
    );
    assert_eq!(
        manifest.target_repository_ids,
        vec![closed.target_repository_id.clone()]
    );
    assert_eq!(manifest.advance_ids, vec![closed.advance_id.clone()]);
    assert_eq!(
        manifest.advance_retry_command_ids,
        vec![closed.retry_command_id.clone()]
    );

    // —— 同名他 Plan 隔离：B 只含自己的真实归属 ——
    let scope_b = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0002");
    let manifest_b = store.collect_entity_deletion_manifest(&scope_b).unwrap();
    assert_eq!(manifest_b.work_item_ids, vec!["work_item_0003".to_string()]);
    assert_eq!(manifest_b.session_ids, vec![other_session.id.clone()]);
    assert!(manifest_b.provider_run_ids.is_empty());
    assert!(manifest_b.review_ids.is_empty());

    // —— Story：版本 provider_run_refs + session + review（REQ-DEL-01）——
    let story = d1_3a_story(&store);
    let story_session = create_session(&store, &story.id, WorkspaceType::Story);
    let story_review = store
        .append_provider_review_round(AppendProviderReviewRoundInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            session_id: story_session.id.clone(),
            round_index: 1,
            author_provider: ProviderName::Codex,
            reviewer_provider: None,
            review_result: "approved".to_string(),
            revision_result: "none".to_string(),
        })
        .unwrap();
    store
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: story.id.clone(),
            markdown: "# v1".to_string(),
            provider_run_refs: vec![closed.run_id.clone()],
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .unwrap();
    let story_manifest = store
        .collect_entity_deletion_manifest(&d1_3a_scope(DeletionEntityKind::Story, &story.id))
        .unwrap();
    assert_eq!(story_manifest.session_ids, vec![story_session.id.clone()]);
    assert_eq!(story_manifest.review_ids, vec![story_review.id.clone()]);
    assert_eq!(story_manifest.provider_run_ids, vec![closed.run_id.clone()]);
    assert!(story_manifest.work_item_ids.is_empty());
    assert!(story_manifest.advance_ids.is_empty());

    // —— root 与 pending 均缺（未索引 owned-index）仍凭存活记录/身份索引列残留 ——
    std::fs::remove_file(
        store
            .app_paths()
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("issue-work-item-plans")
            .join("issue_work_item_plan_0001.json"),
    )
    .unwrap();
    let residue = store.list_entity_owned_residue(&scope).unwrap();
    assert_eq!(
        residue, manifest,
        "根已删时残留反查仍按真实归属列全量 owned 内容"
    );

    // —— JSON 错误不当空：损坏 WI 记录必须让收集 fail-closed ——
    std::fs::write(
        store
            .app_paths()
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("work-items")
            .join("work_item_0002.json"),
        "{ not json",
    )
    .unwrap();
    assert!(store.collect_entity_deletion_manifest(&scope).is_err());
    assert!(store.list_entity_owned_residue(&scope).is_err());
}

#[test]
fn cascade_d1_1_audit_dependency_order() {
    let (_tmp, store) = setup();
    let closed = d1_1_plan_closure(&store, "issue_work_item_plan_0001");

    // 旁观者：他 Plan B（引用 Story 与 A 的 run）+ 未被引用 legacy run。
    let story = d1_3a_story(&store);
    let plan_b = d1_3a_plan(&store, "issue_work_item_plan_0002", vec![story.id.clone()]);
    d1_3a_wi(&store, "work_item_0003", "issue_work_item_plan_0002");
    let bystander_session = create_session(
        &store,
        "issue_work_item_plan_0002",
        WorkspaceType::WorkItemPlan,
    );
    let bystander_run = store
        .save_work_item_split_provider_run(
            PROJECT_ID,
            ISSUE_ID,
            &ProviderName::Codex,
            "bystander split prompt",
            &serde_json::json!({}),
        )
        .unwrap();
    // B 引用 A 的 run → 共享 run 外部引用事实（协调层 entity_referenced 拒删素材）。
    store
        .update_issue_work_item_plan(
            PROJECT_ID,
            ISSUE_ID,
            &plan_b.id,
            IssueWorkItemPlanUpdate {
                work_item_ids: vec!["work_item_0003".to_string()],
                verification_plan_ids: Vec::new(),
                repository_profile_ref: None,
                dependency_graph: Vec::new(),
                created_from_provider_run: Some(closed.run_id.clone()),
                validator_findings: Vec::new(),
            },
        )
        .unwrap();

    let lifecycle = store.app_paths().issue_lifecycle_root(PROJECT_ID, ISSUE_ID);
    let issue_root = store.app_paths().issue_root(PROJECT_ID, ISSUE_ID);
    let scope = d1_3a_scope(DeletionEntityKind::Plan, "issue_work_item_plan_0001");
    let story_scope = d1_3a_scope(DeletionEntityKind::Story, &story.id);

    // —— 共享 run 外部引用与他 Plan/WI 对 Story 的引用都能被反查 ——
    let incoming = store.list_incoming_entity_references(&scope).unwrap();
    assert!(incoming.contains(&EntityReference {
        kind: "issue_work_item_plan".to_string(),
        id: "issue_work_item_plan_0002".to_string(),
    }));
    let story_incoming = store.list_incoming_entity_references(&story_scope).unwrap();
    assert!(story_incoming.iter().any(|reference| {
        reference.kind == "issue_work_item_plan" && reference.id == plan_b.id
    }));

    // —— 旧 lineage refs 仍阻挡（REQ-DEL-03：不得仅凭当前摘要放行）——
    write_json(
        &issue_root
            .join("work-item-revisions")
            .join("plan_lineage_holder")
            .join("lineage.json"),
        &serde_json::json!({
            "id": "plan_lineage_holder",
            "project_id": PROJECT_ID,
            "issue_id": ISSUE_ID,
            "story_spec_refs": [story.id],
            "design_spec_refs": [],
            "active_revision_id": null,
            "active_amendment_id": null,
            "created_at": "2026-10-13T00:00:00Z",
            "updated_at": "2026-10-13T00:00:00Z"
        }),
    )
    .unwrap();
    let story_incoming = store.list_incoming_entity_references(&story_scope).unwrap();
    assert!(story_incoming.iter().any(|reference| {
        reference.kind == "plan_lineage" && reference.id == "plan_lineage_holder"
    }));

    // 解除共享 run 引用后 A 的 incoming 为空（可进入删除编排）。
    store
        .update_issue_work_item_plan(
            PROJECT_ID,
            ISSUE_ID,
            &plan_b.id,
            IssueWorkItemPlanUpdate {
                work_item_ids: vec!["work_item_0003".to_string()],
                verification_plan_ids: Vec::new(),
                repository_profile_ref: None,
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            },
        )
        .unwrap();
    assert!(
        store
            .list_incoming_entity_references(&scope)
            .unwrap()
            .is_empty()
    );

    // 无误伤基线字节。
    let plan_b_bytes = std::fs::read(
        lifecycle
            .join("issue-work-item-plans")
            .join("issue_work_item_plan_0002.json"),
    )
    .unwrap();
    let story_bytes = std::fs::read(
        lifecycle
            .join("story-specs")
            .join(format!("{}.json", story.id)),
    )
    .unwrap();
    let bystander_session_bytes = std::fs::read(
        lifecycle
            .join("workspace-sessions")
            .join(format!("{}.json", bystander_session.id)),
    )
    .unwrap();

    // —— 删除编排（store 原语）：manifest→pending→owned-index→精准 purge→根最后 ——
    let operations = EntityOperationStore::new(store.app_paths());
    let deletions = EntityDeletionStore::new(store.app_paths());
    let lease = operations.try_acquire_delete(&scope).unwrap();
    deletions.begin(&lease).unwrap();
    let manifest = store.collect_entity_deletion_manifest(&scope).unwrap();
    deletions.save_pending_manifest(&lease, &manifest).unwrap();
    deletions
        .index_owned_artifact_ids(&lease, &manifest)
        .unwrap();
    store
        .purge_entity_owned_children(&lease, &manifest)
        .unwrap();

    // 审计与子依赖清干净：review/run/两全局分区/timeline/session/WI/VP/profile/advance。
    assert!(
        !lifecycle
            .join("provider-review-rounds")
            .join(format!("{}.json", closed.review_id))
            .exists()
    );
    assert!(
        !lifecycle
            .join("provider-runs")
            .join(&closed.run_id)
            .exists()
    );
    assert!(
        !store
            .app_paths()
            .root()
            .join("tool-policy-run-audit")
            .join(&closed.plan_session_id)
            .exists()
    );
    assert!(
        !store
            .app_paths()
            .root()
            .join("work-item-split-runs")
            .join(&closed.wi_session_id)
            .exists()
    );
    for session_id in [&closed.plan_session_id, &closed.wi_session_id] {
        assert!(
            !lifecycle
                .join("workspace-timelines")
                .join(session_id)
                .exists()
        );
        assert!(
            !lifecycle
                .join("workspace-sessions")
                .join(session_id)
                .exists()
        );
        assert!(
            !lifecycle
                .join("workspace-sessions")
                .join(format!("{session_id}.json"))
                .exists()
        );
        assert!(
            !lifecycle
                .join("workspace-sessions")
                .join(format!(".{session_id}.json.lock"))
                .exists()
        );
    }
    for work_item_id in ["work_item_0001", "work_item_0002"] {
        assert!(
            !lifecycle
                .join("work-items")
                .join(format!("{work_item_id}.json"))
                .exists()
        );
    }
    assert!(
        !lifecycle
            .join("verification-plans")
            .join(format!("{}.json", closed.vp_id))
            .exists()
    );
    assert!(
        !lifecycle
            .join("repository-profiles")
            .join(format!("{}.json", closed.profile_id))
            .exists()
    );
    assert!(
        AdvanceStore::new(store.app_paths())
            .list_plan_artifacts(PROJECT_ID, ISSUE_ID, "issue_work_item_plan_0001")
            .unwrap()
            .advance_ids
            .is_empty()
    );

    // 旁观者零误伤：他 Plan/WI/session、Story、无归属 run 字节不变。
    assert_eq!(
        std::fs::read(
            lifecycle
                .join("issue-work-item-plans")
                .join("issue_work_item_plan_0002.json")
        )
        .unwrap(),
        plan_b_bytes
    );
    assert_eq!(
        std::fs::read(
            lifecycle
                .join("story-specs")
                .join(format!("{}.json", story.id))
        )
        .unwrap(),
        story_bytes
    );
    assert_eq!(
        std::fs::read(
            lifecycle
                .join("workspace-sessions")
                .join(format!("{}.json", bystander_session.id))
        )
        .unwrap(),
        bystander_session_bytes
    );
    assert!(
        lifecycle
            .join("provider-runs")
            .join(&bystander_run)
            .exists()
    );
    assert!(
        lifecycle
            .join("work-items")
            .join("work_item_0003.json")
            .exists()
    );

    // owned-index 在位而内容全清：残留反查为空（§0.5 完成清理判据）。
    assert_eq!(
        store.list_entity_owned_residue(&scope).unwrap(),
        EntityDeletionManifest::empty(scope.clone())
    );

    // 重入幂等：purge 可重复；根缺不 404；根最后删。
    store
        .purge_entity_owned_children(&lease, &manifest)
        .unwrap();
    store.delete_entity_root(&lease).unwrap();
    assert!(
        !lifecycle
            .join("issue-work-item-plans")
            .join("issue_work_item_plan_0001.json")
            .exists()
    );
    store.delete_entity_root(&lease).unwrap();
    deletions.remove_pending_manifest(&lease).unwrap();
    d1_3a_complete_purge(&store, lease);

    // 身份 owner 索引（D1.0 纯 ID 事实）在完成后仍可反查,不当业务内容复活。
    assert!(
        !operations
            .list_workspace_session_ids_by_owner(&scope)
            .unwrap()
            .is_empty()
    );

    // —— 先删后置（B 引用 Story）再删上游：Story 闭包含 versions 与主记录 ——
    let scope_b = d1_3a_scope(DeletionEntityKind::Plan, &plan_b.id);
    let lease_b = operations.try_acquire_delete(&scope_b).unwrap();
    deletions.begin(&lease_b).unwrap();
    let manifest_b = store.collect_entity_deletion_manifest(&scope_b).unwrap();
    assert_eq!(manifest_b.work_item_ids, vec!["work_item_0003".to_string()]);
    store
        .purge_entity_owned_children(&lease_b, &manifest_b)
        .unwrap();
    store.delete_entity_root(&lease_b).unwrap();
    deletions.remove_pending_manifest(&lease_b).unwrap();
    d1_3a_complete_purge(&store, lease_b);

    // 发布 lineage 属 D1.2b revision purge 面（work-item-revisions/{plan}/ 整树）：
    // 此处按其 purge 完成后的磁盘状态推演——先删后置全链收口后上游可删。
    std::fs::remove_dir_all(
        issue_root
            .join("work-item-revisions")
            .join("plan_lineage_holder"),
    )
    .unwrap();

    assert!(
        store
            .list_incoming_entity_references(&story_scope)
            .unwrap()
            .is_empty()
    );

    let story_manifest = store
        .collect_entity_deletion_manifest(&story_scope)
        .unwrap();
    let lease_story = operations.try_acquire_delete(&story_scope).unwrap();
    deletions.begin(&lease_story).unwrap();
    deletions
        .save_pending_manifest(&lease_story, &story_manifest)
        .unwrap();
    store
        .purge_entity_owned_children(&lease_story, &story_manifest)
        .unwrap();
    // versions 子树随 children purge 清除（§0.5 Story versions 行）。
    assert!(!lifecycle.join("versions").join(&story.id).exists());
    assert!(
        !lifecycle
            .join("provider-review-rounds")
            .join("review_round_0002.json")
            .exists()
    );
    store.delete_entity_root(&lease_story).unwrap();
    assert!(
        !lifecycle
            .join("story-specs")
            .join(format!("{}.json", story.id))
            .exists()
    );
    deletions.remove_pending_manifest(&lease_story).unwrap();
    d1_3a_complete_purge(&store, lease_story);

    // 共享 run 已无任何归属者,随 Story 闭包清除;无归属 bystander run 保留。
    assert!(
        !lifecycle
            .join("provider-runs")
            .join(&closed.run_id)
            .exists()
    );
    assert!(
        lifecycle
            .join("provider-runs")
            .join(&bystander_run)
            .exists()
    );
}
