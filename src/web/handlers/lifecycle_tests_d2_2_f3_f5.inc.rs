// 经 lifecycle_tests.inc.rs 的 tests mod `include!` 引入（large_file_guard
// 1200 行红线拆分，纯移动自 lifecycle_tests_d2_2.inc.rs 1371-1954 行，逐字不变）。
// ==== F3（fix-delete-automation-guard）：删除被自动化 enrollment 占用的实体 ====
//
// 真机缺陷：enabled enrollment 让 autopilot 每 tick ensure/reconcile/生成写该
// plan（scope 共享锁），DELETE 永远抢不到独占锁 → entity_busy 删不掉。修复：
// 取删除锁前先同源 CAS 终止绑定该实体的 enrollment，再继续删除；终止失败
// 明确报错且删除不执行；entity_busy details 补占用者信息。

/// 直写最小 enrollment JSON（durable 事实与真实 Enable 同构；删除路径只消费
/// enabled/terminated_at/plan_id/session_id/source 字段）。source 供 F5
/// 「source 引用该实体」的悬空失效判定播种。
fn f3_enable_automation(
    paths: &ProductAppPaths,
    plan_id: Option<&str>,
    source: crate::product::models::automation::EnrollmentSource,
) -> crate::product::models::automation::IssueAutomationEnrollment {
    use crate::product::models::automation::{
        EnrollmentOptions, IssueAutomationEnrollment, PlanApprovalPolicy,
    };
    let enrollment = IssueAutomationEnrollment {
        enrollment_id: "enr_f3_0001".to_string(),
        selection_key: "issue:0001".to_string(),
        project_id: PROJECT_ID.to_string(),
        issue_id: ISSUE_ID.to_string(),
        enabled: true,
        policy_revision: 3,
        source,
        options: EnrollmentOptions {
            author_provider: ProviderName::Fake,
            reviewer_provider: ProviderName::Fake,
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            plan_approval: PlanApprovalPolicy::Manual,
            plan_options: crate::product::models::lifecycle::IssueWorkItemPlanOptions {
                include_integration_tests: true,
                include_e2e_tests: false,
                force_frontend_backend_split: false,
                require_execution_plan_confirm: false,
            },
        },
        target: None,
        prepare_intent_id: "prep_f3_0001".to_string(),
        plan_id: plan_id.map(str::to_string),
        session_id: None,
        created_at: "2026-10-23T00:00:00Z".to_string(),
        updated_at: "2026-10-23T00:00:00Z".to_string(),
        binding_history: None,
        command_ledger: Vec::new(),
        terminated_at: None,
        attention_facts: Vec::new(),
        voided_at: None,
        void_reason: None,
    };
    crate::product::json_store::write_json(
        &paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("automation-enrollment.json"),
        &enrollment,
    )
    .expect("seed enrollment");
    enrollment
}

fn f3_empty_source() -> crate::product::models::automation::EnrollmentSource {
    crate::product::models::automation::EnrollmentSource {
        stories: Vec::new(),
        designs: Vec::new(),
    }
}

fn f3_read_enrollment(
    paths: &ProductAppPaths,
) -> crate::product::models::automation::IssueAutomationEnrollment {
    crate::product::issue_automation_store::IssueAutomationStore::new(paths.clone())
        .get(PROJECT_ID, ISSUE_ID)
        .expect("read enrollment")
        .expect("enrollment exists")
}

/// F3 ①：存在绑定该 Plan 且 enabled 的自动化 enrollment 时，删除先终止
/// enrollment（同源 terminate CAS、稳定命令 id 入账本）再删除成功——这是
/// 真机「永远删不掉」的根因面。红：现状删除 200 但 enrollment 仍 enabled，
/// autopilot 下一 tick 继续 ensure/重建。
#[tokio::test]
async fn cascade_f3_terminates_bound_automation_before_delete() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_A,
        Vec::new(),
        None,
        None,
        Vec::new(),
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );
    let seeded = f3_enable_automation(&paths, Some(D2_2_PLAN_A), f3_empty_source());

    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");

    let enrollment = f3_read_enrollment(&paths);
    assert!(
        enrollment.terminated_at.is_some(),
        "删除必须先终止绑定该实体的自动化 enrollment"
    );
    assert!(!enrollment.enabled);
    // 终止走与人工终止/autopilot 自愈同源的 terminate CAS：稳定命令 id 入账本。
    assert!(
        enrollment
            .command_ledger
            .iter()
            .any(|entry| entry.command_id
                == format!(
                    "entity-delete-terminate-{}-r{}",
                    seeded.enrollment_id, seeded.policy_revision
                )),
        "termination must go through the same-source CAS ledger: {:?}",
        enrollment.command_ledger
    );
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
            .is_err(),
        "plan 记录随删除消失"
    );
}

/// F3 ②：无 enrollment / enrollment 未绑定该实体时删除行为不变，且不误终止
/// 无关 enrollment（幂等重入语义与单仓路径不受影响）。
#[tokio::test]
async fn cascade_f3_delete_without_bound_automation_is_unchanged() {
    // A) 完全无 enrollment：原语义删除成功。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        d2_2_seed_plan(
            &lifecycle,
            D2_2_PLAN_A,
            Vec::new(),
            None,
            None,
            Vec::new(),
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "delete body: {body}");
        assert!(
            lifecycle
                .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
                .is_err()
        );
    }
    // B) enrollment enabled 但绑定其他 plan：删除照常成功，enrollment 原样。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        d2_2_seed_plan(
            &lifecycle,
            D2_2_PLAN_A,
            Vec::new(),
            None,
            None,
            Vec::new(),
            crate::product::models::IssueWorkItemPlanStatus::Draft,
        );
        let seeded = f3_enable_automation(&paths, Some("work_item_plan_other"), f3_empty_source());
        let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
        assert_eq!(status, StatusCode::OK, "delete body: {body}");
        let enrollment = f3_read_enrollment(&paths);
        assert!(enrollment.enabled, "未绑定该实体的 enrollment 不得被终止");
        assert!(enrollment.terminated_at.is_none());
        assert_eq!(enrollment.policy_revision, seeded.policy_revision);
        assert!(enrollment.command_ledger.is_empty());
    }
}

/// F3 ③：自动化终止失败 → 删除不执行、明确 `automation_termination_failed`
/// （绝不笼统 entity_busy），现场零破坏；恢复可写后重试收敛（终止→删除成功）。
#[tokio::test]
async fn cascade_f3_delete_fails_closed_when_termination_fails() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_A,
        Vec::new(),
        None,
        None,
        Vec::new(),
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );
    let seeded = f3_enable_automation(&paths, Some(D2_2_PLAN_A), f3_empty_source());
    // 终止 CAS 不可用：预占 enrollment 的伴生锁路径（目录形态 → 锁文件
    // open 失败）。issue 目录保持可写，精确只让 terminate_plan 落不了锁。
    let enrollment_lock = paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join(".automation-enrollment.json.lock");
    std::fs::create_dir_all(&enrollment_lock).expect("block enrollment lock");

    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "fail-closed body: {body}");
    assert_eq!(body["code"], "automation_termination_failed");
    assert_eq!(body["message"], "该实体的自动化未能停用，删除未执行");
    assert_eq!(
        body["details"]["automation_enrollment_id"],
        seeded.enrollment_id.clone(),
        "details 带上未能停用的 enrollment id"
    );

    // 解除注入后断言零破坏（plan/墓碑/enrollment 全部原样）。
    std::fs::remove_dir_all(&enrollment_lock).expect("unblock enrollment lock");
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
            .is_ok(),
        "终止失败时 plan 不得被删"
    );
    let enrollment = f3_read_enrollment(&paths);
    assert!(enrollment.enabled);
    assert!(enrollment.terminated_at.is_none());
    assert_eq!(enrollment.policy_revision, seeded.policy_revision);
    assert!(
        EntityDeletionStore::new(paths.clone())
            .get(&EntityScope::new(
                PROJECT_ID,
                ISSUE_ID,
                crate::product::entity_deletion_store::DeletionEntityKind::Plan,
                D2_2_PLAN_A
            ))
            .unwrap()
            .is_none(),
        "终止失败不立墓碑"
    );

    // 同一现场恢复后重试：先终止后删除，收敛成功。
    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::OK, "retry body: {body}");
    assert!(f3_read_enrollment(&paths).terminated_at.is_some());
}

/// F3 ④：终止自动化后删除独占仍争用（在途 run/写者持 scope 共享锁）→
/// entity_busy 保持稳定码，details 补占用者信息（scope kind/id＋刚停用的
/// enrollment id）便于判断该等还是该先停自动化；占用期间删除零破坏，释放
/// 后重删成功。
#[tokio::test]
async fn cascade_f3_busy_error_reports_automation_occupant() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    d2_2_seed_plan(
        &lifecycle,
        D2_2_PLAN_A,
        Vec::new(),
        None,
        None,
        Vec::new(),
        crate::product::models::IssueWorkItemPlanStatus::Draft,
    );
    let seeded = f3_enable_automation(&paths, Some(D2_2_PLAN_A), f3_empty_source());
    // 在途运行者：持 scope 共享 run lease（autopilot 生成链的持有形态）。
    let scope = EntityScope::new(
        PROJECT_ID,
        ISSUE_ID,
        crate::product::entity_deletion_store::DeletionEntityKind::Plan,
        D2_2_PLAN_A,
    );
    let run_lease = EntityOperationStore::new(paths.clone())
        .acquire_run(&scope)
        .expect("hold run lease");

    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::CONFLICT, "busy body: {body}");
    assert_eq!(body["code"], "entity_busy");
    assert_eq!(body["details"]["entity_kind"], "plan");
    assert_eq!(body["details"]["entity_id"], D2_2_PLAN_A);
    assert_eq!(
        body["details"]["automation_enrollment_id"],
        seeded.enrollment_id.clone(),
        "占用者信息：识别到绑定该实体的自动化 enrollment"
    );
    // 先终止语义同样成立；占用期间删除零破坏。
    assert!(f3_read_enrollment(&paths).terminated_at.is_some());
    assert!(
        lifecycle
            .get_issue_work_item_plan(PROJECT_ID, ISSUE_ID, D2_2_PLAN_A)
            .is_ok()
    );
    assert!(
        EntityDeletionStore::new(paths.clone())
            .get(&scope)
            .unwrap()
            .is_none()
    );

    drop(run_lease);
    let (status, body) = d2_2_delete(&app, "work-item-plans", D2_2_PLAN_A, None).await;
    assert_eq!(status, StatusCode::OK, "release body: {body}");
}

// ==== F5（悬空源根修）：删除 Story/Design 时 source 引用该实体的 enrollment 失效 ====
//
// 真机缺陷：级联删除不覆盖 automation_enrollment——enabled enrollment 的
// source 引用被删实体后autopilot 继续带着悬空授权跑；terminated 记录则留下
// 悬空 source 让人工重新开启必然 422。语义（裁定）：删 Story/Design 时把
// 引用它的 enrollment 一并失效（同源 terminate CAS、幂等、失败只 warn 不
// 阻断删除）；已 terminated/disabled 的记录本身已失效，不动。

/// F5 红测①：enabled enrollment 的 source.stories 引用被删 story → 删除
/// 成功且 enrollment 被同源 terminate CAS 失效（幂等重删不重复终止）。
#[tokio::test]
async fn cascade_f5_terminates_enrollment_referencing_deleted_story() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "F5 story".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    lifecycle
        .append_version(crate::product::lifecycle_store::AppendSpecVersionInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            entity_id: story.id.clone(),
            markdown: "# f5 story".to_string(),
            provider_run_refs: Vec::new(),
            review_refs: Vec::new(),
            confirmed_by: None,
        })
        .unwrap();
    let seeded = f3_enable_automation(
        &paths,
        None,
        crate::product::models::automation::EnrollmentSource {
            stories: vec![crate::product::models::automation::SourceRevisionRef {
                id: story.id.clone(),
                version: 1,
            }],
            designs: Vec::new(),
        },
    );

    let (status, body) = d2_2_delete(&app, "story-specs", &story.id, None).await;
    assert_eq!(status, StatusCode::OK, "delete story: {body}");

    // 不得留悬空引用：enrollment 已失效（terminated＋enabled=false）。
    let enrollment = f3_read_enrollment(&paths);
    assert!(
        enrollment.terminated_at.is_some(),
        "删除引用的 story 必须一并失效 enrollment"
    );
    assert!(!enrollment.enabled);
    assert!(
        enrollment.command_ledger.iter().any(|entry| entry.command_id
            == format!(
                "entity-delete-terminate-{}-r{}",
                seeded.enrollment_id, seeded.policy_revision
            )),
        "失效走同源 terminate CAS 账本: {:?}",
        enrollment.command_ledger
    );

    // 幂等：重复删除已完成的 story 不再重复终止（账本不新增）。
    let ledger_len = enrollment.command_ledger.len();
    let (status, body) = d2_2_delete(&app, "story-specs", &story.id, None).await;
    assert_eq!(status, StatusCode::OK, "idempotent redelete: {body}");
    assert_eq!(f3_read_enrollment(&paths).command_ledger.len(), ledger_len);
}

/// F5 红测②：source.designs 引用被删 design → 同样失效；不引用被删实体的
/// enrollment 不受影响。
#[tokio::test]
async fn cascade_f5_terminates_enrollment_referencing_deleted_design() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "F5 story b".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    let design = lifecycle
        .create_design_spec(CreateDesignSpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            story_spec_ids: vec![story.id.clone()],
            title: "F5 design b".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    let source = crate::product::models::automation::EnrollmentSource {
        stories: Vec::new(),
        designs: vec![crate::product::models::automation::SourceRevisionRef {
            id: design.id.clone(),
            version: 1,
        }],
    };
    f3_enable_automation(&paths, None, source);

    let (status, body) = d2_2_delete(&app, "design-specs", &design.id, None).await;
    assert_eq!(status, StatusCode::OK, "delete design: {body}");
    let enrollment = f3_read_enrollment(&paths);
    assert!(
        enrollment.terminated_at.is_some(),
        "删除引用的 design 必须一并失效 enrollment"
    );
    assert!(!enrollment.enabled);

 // 反例：enrollment 引用别的实体 → 删除照常、enrollment 原样保持 enabled。
    {
        let root = TempDir::new().expect("root");
        let app = build_test_router(root.path());
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        let lifecycle = d2_2_seed_project_issue(&paths);
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                title: "F5 story c".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "F5 design c".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        let other = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "F5 design other".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        let seeded = f3_enable_automation(
            &paths,
            None,
            crate::product::models::automation::EnrollmentSource {
                stories: Vec::new(),
                designs: vec![crate::product::models::automation::SourceRevisionRef {
                    id: design.id.clone(),
                    version: 1,
                }],
            },
        );
        let (status, body) = d2_2_delete(&app, "design-specs", &other.id, None).await;
        assert_eq!(status, StatusCode::OK, "unrelated delete: {body}");
        let enrollment = f3_read_enrollment(&paths);
        assert!(
            enrollment.enabled && enrollment.terminated_at.is_none(),
            "不引用被删实体的 enrollment 不得被失效"
        );
        assert_eq!(enrollment.policy_revision, seeded.policy_revision);
    }
}

/// F5 红测③：失效失败只 warn——终止 CAS 不可用时删除照常成功（不得阻断
/// 级联删除本身）。
#[tokio::test]
async fn cascade_f5_invalidation_failure_does_not_block_deletion() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "F5 story warn".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    f3_enable_automation(
        &paths,
        None,
        crate::product::models::automation::EnrollmentSource {
            stories: vec![crate::product::models::automation::SourceRevisionRef {
                id: story.id.clone(),
                version: 1,
            }],
            designs: Vec::new(),
        },
    );
    // 阻断 enrollment 伴生锁：terminate CAS 落不了锁（失效必失败）。
    let enrollment_lock = paths
        .issue_root(PROJECT_ID, ISSUE_ID)
        .join(".automation-enrollment.json.lock");
    std::fs::create_dir_all(&enrollment_lock).expect("block enrollment lock");

    let (status, body) = d2_2_delete(&app, "story-specs", &story.id, None).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "失效失败只 warn，不得阻断级联删除: {body}"
    );
    // 删除完成（story 根已清）；失效动作失败留待后续收敛（warn 只留日志）。
    assert!(
        !paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("story-specs")
            .join(format!("{}.json", story.id))
            .exists(),
        "story 根必须删除"
    );
}

/// F5 红测④：已 terminated 的 enrollment（source 引用被删实体）本身已失效
/// ——删除照常成功且不重复写 enrollment（幂等、零改写）。
#[tokio::test]
async fn cascade_f5_already_terminated_referencing_enrollment_left_untouched() {
    let root = TempDir::new().expect("root");
    let app = build_test_router(root.path());
    let paths = ProductAppPaths::new(root.path().join(".aria"));
    let lifecycle = d2_2_seed_project_issue(&paths);
    let story = lifecycle
        .create_story_spec(CreateStorySpecInput {
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            repository_id: REPOSITORY_ID.to_string(),
            title: "F5 story done".to_string(),
            aggregate_codebase: None,
        })
        .unwrap();
    let mut seeded = f3_enable_automation(
        &paths,
        None,
        crate::product::models::automation::EnrollmentSource {
            stories: vec![crate::product::models::automation::SourceRevisionRef {
                id: story.id.clone(),
                version: 1,
            }],
            designs: Vec::new(),
        },
    );
    // 预置终止态（真机现场形态：先终止自动化、再删实体）。
    seeded.enabled = false;
    seeded.terminated_at = Some("2026-10-27T08:26:00Z".to_string());
    crate::product::json_store::write_json(
        &paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("automation-enrollment.json"),
        &seeded,
    )
    .unwrap();

    let (status, body) = d2_2_delete(&app, "story-specs", &story.id, None).await;
    assert_eq!(status, StatusCode::OK, "delete body: {body}");
    let enrollment = f3_read_enrollment(&paths);
    assert_eq!(
        enrollment.policy_revision, seeded.policy_revision,
        "已失效记录零改写"
    );
    assert!(enrollment.command_ledger.is_empty(), "不重复终止");
    assert_eq!(
        enrollment.terminated_at.as_deref(),
        Some("2026-10-27T08:26:00Z")
    );
}
