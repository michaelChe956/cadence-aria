// cascade_d1_4（fix-cascade-deletion Task D1.4：单调 ever_confirmed 与旧未知兼容）。
// 独立文件经 include! 引入 tests.rs（与 cascade_d1_1.rs 同惯例，large_file_guard）。
// 覆盖 Step 1 全项：新 Draft=Some(false) 免确认；current Confirmed/ever=true/
// 真实发布（Plan 发布日志、Story confirmed_by 版本）需确认；None 旧 Draft 未知
// 需确认且再 serialize 不补字段；确认→改回 Draft→restore 旧 snapshot 仍 true；
// typed 字段不可通过候选替换变 false。

use crate::product::models::{
    DeletionConfirmation, DeletionConfirmationReason, IssueWorkItemPlan, IssueWorkItemPlanStatus,
    RepositoryProfile, RepositoryProfileConfidence,
};
use crate::product::work_item_split_engine::WorkItemSplitProviderOutput;

fn d1_4_required_previously() -> DeletionConfirmation {
    DeletionConfirmation::Required {
        reason: DeletionConfirmationReason::PreviouslyConfirmed,
    }
}

fn d1_4_required_unknown() -> DeletionConfirmation {
    DeletionConfirmation::Required {
        reason: DeletionConfirmationReason::LegacyHistoryUnknown,
    }
}

/// 最小候选输出（空 WI/VP），供 replace_issue_work_item_plan_candidate 走真实
/// update 路径；profile 走真实 create_repository_profile 落档。
fn d1_4_candidate_output(plan_id: &str) -> WorkItemSplitProviderOutput {
    let now = "2026-10-14T00:00:00Z".to_string();
    WorkItemSplitProviderOutput {
        repository_profile: RepositoryProfile {
            id: "repo_profile_d14".to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            logical_repository_id: None,
            membership_revision: 0,
            provider_run_ref: None,
            languages: Vec::new(),
            frameworks: Vec::new(),
            package_managers: Vec::new(),
            test_frameworks: Vec::new(),
            build_systems: Vec::new(),
            verification_capabilities: Vec::new(),
            detected_layers: Vec::new(),
            split_recommendation: "mono".to_string(),
            confidence: RepositoryProfileConfidence::High,
            uncertainties: Vec::new(),
            created_at: now.clone(),
            updated_at: now,
        },
        plan: IssueWorkItemPlan {
            id: plan_id.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            source_story_spec_ids: Vec::new(),
            source_design_spec_ids: Vec::new(),
            options: d1_3a_plan_options(),
            status: IssueWorkItemPlanStatus::Draft,
            work_item_ids: Vec::new(),
            repository_profile_ref: Some("repo_profile_d14".to_string()),
            verification_plan_ids: Vec::new(),
            dependency_graph: Vec::new(),
            created_from_provider_run: None,
            validator_findings: Vec::new(),
            review_summary: None,
            created_at: "2026-10-14T00:00:00Z".to_string(),
            updated_at: "2026-10-14T00:00:00Z".to_string(),
            ever_confirmed: None,
        },
        work_items: Vec::new(),
        verification_plans: Vec::new(),
    }
}

/// 旧 bytes（无 ever_confirmed 字段）的 Story 根，直接落盘模拟 legacy 记录。
fn d1_4_seed_legacy_story(store: &LifecycleStore, story_id: &str) {
    write_json(
        &store
            .story_specs_root(PROJECT_ID, ISSUE_ID)
            .join(format!("{story_id}.json")),
        &serde_json::json!({
            "id": story_id,
            "project_id": PROJECT_ID,
            "issue_id": ISSUE_ID,
            "repository_id": REPOSITORY_ID,
            "title": "legacy story",
            "current_version": null,
            "confirmation_status": "draft",
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
        }),
    )
    .unwrap();
}

/// 旧 bytes（无 ever_confirmed 字段）的 Plan 根，直接落盘模拟 legacy 记录。
fn d1_4_seed_legacy_plan(store: &LifecycleStore, plan_id: &str) {
    write_json(
        &store
            .issue_work_item_plans_root(PROJECT_ID, ISSUE_ID)
            .join(format!("{plan_id}.json")),
        &serde_json::json!({
            "id": plan_id,
            "project_id": PROJECT_ID,
            "issue_id": ISSUE_ID,
            "source_story_spec_ids": [],
            "source_design_spec_ids": [],
            "options": {
                "include_integration_tests": false,
                "include_e2e_tests": false,
                "force_frontend_backend_split": false,
                "require_execution_plan_confirm": false,
            },
            "status": "draft",
            "work_item_ids": [],
            "repository_profile_ref": null,
            "verification_plan_ids": [],
            "dependency_graph": [],
            "created_from_provider_run": null,
            "validator_findings": [],
            "review_summary": null,
            "created_at": "2026-01-01T00:00:00Z",
            "updated_at": "2026-01-01T00:00:00Z",
        }),
    )
    .unwrap();
}

#[test]
fn cascade_d1_4_confirmation_history_classification() {
    let (tmp, store) = setup();

    // ──① 新建三类：root 首次持久 Some(false)，classify NotRequired。──
    let story = d1_3a_story(&store);
    assert_eq!(story.ever_confirmed, Some(false));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Story,
                &story.id
            ))
            .unwrap(),
        DeletionConfirmation::NotRequired
    );

    let design = store
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "design".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    assert_eq!(design.ever_confirmed, Some(false));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Design,
                &design.id
            ))
            .unwrap(),
        DeletionConfirmation::NotRequired
    );

    let plan_id = "issue_work_item_plan_d14";
    let plan = d1_3a_plan(&store, plan_id, vec![story.id.clone()]);
    assert_eq!(plan.ever_confirmed, Some(false));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(DeletionEntityKind::Plan, plan_id))
            .unwrap(),
        DeletionConfirmation::NotRequired
    );

    // ──② current Confirmed → Required(PreviouslyConfirmed)，原 write 置 true。──
    store
        .update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &story.id,
            LifecycleConfirmationStatus::Confirmed,
        )
        .unwrap();
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Story,
                &story.id
            ))
            .unwrap(),
        d1_4_required_previously()
    );

    store
        .confirm_issue_work_item_plan(PROJECT_ID, ISSUE_ID, plan_id)
        .unwrap();
    let confirmed_plan = store
        .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, plan_id)
        .unwrap();
    assert_eq!(confirmed_plan.ever_confirmed, Some(true));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(DeletionEntityKind::Plan, plan_id))
            .unwrap(),
        d1_4_required_previously()
    );

    // ──③ 回退（Confirmed→Draft/ChangeRequested）单调合并 true，仍需确认。──
    store
        .update_spec_confirmation_status(
            PROJECT_ID,
            ISSUE_ID,
            &story.id,
            LifecycleConfirmationStatus::Draft,
        )
        .unwrap();
    let reverted_story = store
        .list_story_specs(PROJECT_ID, ISSUE_ID)
        .unwrap()
        .into_iter()
        .find(|record| record.id == story.id)
        .unwrap();
    assert_eq!(reverted_story.ever_confirmed, Some(true));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Story,
                &story.id
            ))
            .unwrap(),
        d1_4_required_previously()
    );

    store
        .request_issue_work_item_plan_change(PROJECT_ID, ISSUE_ID, plan_id, None)
        .unwrap();
    let change_requested = store
        .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, plan_id)
        .unwrap();
    assert_eq!(
        change_requested.status,
        IssueWorkItemPlanStatus::ChangeRequested
    );
    assert_eq!(change_requested.ever_confirmed, Some(true));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(DeletionEntityKind::Plan, plan_id))
            .unwrap(),
        d1_4_required_previously()
    );

    // ──④ 确认→（回 Draft）→restore 旧 snapshot（legacy None/Draft）仍 true。──
    let mut legacy_snapshot = confirmed_plan.clone();
    legacy_snapshot.status = IssueWorkItemPlanStatus::Draft;
    legacy_snapshot.ever_confirmed = None;
    legacy_snapshot.updated_at = "2026-01-02T00:00:00Z".to_string();
    store
        .restore_issue_work_item_plan_snapshot(PROJECT_ID, ISSUE_ID, plan_id, &legacy_snapshot)
        .unwrap();
    let restored = store
        .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, plan_id)
        .unwrap();
    assert_eq!(restored.status, IssueWorkItemPlanStatus::Draft);
    assert_eq!(restored.ever_confirmed, Some(true));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(DeletionEntityKind::Plan, plan_id))
            .unwrap(),
        d1_4_required_previously()
    );

    // ──⑤ typed 字段不可通过候选替换变 false（Draft 态真实 replace 路径）。──
    store
        .replace_issue_work_item_plan_candidate(
            PROJECT_ID,
            ISSUE_ID,
            plan_id,
            &d1_4_candidate_output(plan_id),
            Vec::new(),
        )
        .unwrap();
    let replaced = store
        .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, plan_id)
        .unwrap();
    assert_eq!(replaced.ever_confirmed, Some(true));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(DeletionEntityKind::Plan, plan_id))
            .unwrap(),
        d1_4_required_previously()
    );

    // ──⑥ None 旧 Draft 未知 → Required(LegacyHistoryUnknown)。──
    let legacy_story_id = "story_spec_legacy";
    d1_4_seed_legacy_story(&store, legacy_story_id);
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Story,
                legacy_story_id
            ))
            .unwrap(),
        d1_4_required_unknown()
    );

    let legacy_plan_id = "issue_work_item_plan_legacy";
    d1_4_seed_legacy_plan(&store, legacy_plan_id);
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Plan,
                legacy_plan_id
            ))
            .unwrap(),
        d1_4_required_unknown()
    );

    // ──⑦ 真实发布需确认：legacy Plan 根 bytes 不动，发布日志在档即
    //     PreviouslyConfirmed；Story 的 confirmed_by 版本同理（versions 证据）。──
    let publication_root = store
        .app_paths()
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join("work-item-revision-publications")
        .join(legacy_plan_id);
    std::fs::create_dir_all(&publication_root).unwrap();
    std::fs::write(publication_root.join("compile_0001.json"), "{}").unwrap();
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Plan,
                legacy_plan_id
            ))
            .unwrap(),
        d1_4_required_previously()
    );

    let legacy_version_root = store.versions_root(PROJECT_ID, ISSUE_ID, legacy_story_id);
    std::fs::create_dir_all(&legacy_version_root).unwrap();
    write_json(
        &legacy_version_root.join("version_0001.json"),
        &serde_json::json!({
            "id": "version_0001",
            "project_id": PROJECT_ID,
            "issue_id": ISSUE_ID,
            "entity_id": legacy_story_id,
            "version": 1,
            "markdown": "# legacy confirmed version",
            "provider_run_refs": [],
            "review_refs": [],
            "confirmed_by": "user",
            "created_at": "2026-01-01T00:00:01Z",
        }),
    )
    .unwrap();
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Story,
                legacy_story_id
            ))
            .unwrap(),
        d1_4_required_previously()
    );

    // ──⑧ None 旧 bytes 再 serialize 不补字段（update 路径重写根，键仍缺）。──
    store
        .append_version(AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: legacy_story_id.to_string(),
            markdown: "# legacy v2".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .unwrap();
    let story_bytes = std::fs::read_to_string(
        store
            .story_specs_root(PROJECT_ID, ISSUE_ID)
            .join(format!("{legacy_story_id}.json")),
    )
    .unwrap();
    assert!(!story_bytes.contains("ever_confirmed"));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Story,
                legacy_story_id
            ))
            .unwrap(),
        d1_4_required_previously()
    );

    store
        .update_issue_work_item_plan(
            PROJECT_ID,
            ISSUE_ID,
            legacy_plan_id,
            IssueWorkItemPlanUpdate {
                work_item_ids: Vec::new(),
                verification_plan_ids: Vec::new(),
                repository_profile_ref: None,
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            },
        )
        .unwrap();
    let plan_bytes = std::fs::read_to_string(
        store
            .issue_work_item_plans_root(PROJECT_ID, ISSUE_ID)
            .join(format!("{legacy_plan_id}.json")),
    )
    .unwrap();
    assert!(!plan_bytes.contains("ever_confirmed"));
    assert_eq!(
        store
            .classify_entity_deletion_confirmation(&d1_3a_scope(
                DeletionEntityKind::Plan,
                legacy_plan_id
            ))
            .unwrap(),
        d1_4_required_previously()
    );

    drop(tmp);
}
