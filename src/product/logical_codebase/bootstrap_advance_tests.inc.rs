// ---------------------------------------------------------------------------
// A（REQ-REG-10「自动推进」）的 product 层用例：
// - 现场同构（后三步 not_started + prepare）→ advance 循环到 PlanningReady；
// - 重复 advance 幂等（零新 durable 写入、不重复 provider 准备）；
// - 并发 advance 不双推（LC 级排他锁）；
// - 某步 failed 时 advance 停止且绝不改写成 completed（不自动重试）。
//
// web/HTTP 面（真实 provider turn、跳过式回执、GET 只读）的用例在
// `src/web/handlers/aggregate_initialization/tests_parts/bootstrap_advance.inc.rs`。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod advance_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
    };
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateInitializationErrorRecord, AggregateInitializationOperation,
        AggregateInitializationOperationInput, AggregateInitializationOperationStatus,
        AggregateInitializationStepKind,
    };
    use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
    use crate::product::logical_codebase::policy::AggregatePolicyArtifact;
    use crate::product::logical_codebase::store::LogicalCodebaseStore;
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, MemberStatus,
        RepositoryCheckoutRecord, RepositoryType,
    };
    use crate::product::logical_codebase::{
        LogicalCodebaseManifest, LogicalRepositoryId, RepositoryCheckoutId,
    };
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    const PROJECT_ID: &str = "project_0001";
    const ADVANCE_OPERATION_ID: &str = "aggregate_initialization_advance_0001";
    const ADVANCE_INDEX_ID: &str = "aggregate_index_advance_0001";

    /// 现场同构 fixture：identity（活跃成员）与 manifest/checkout 已完成，
    /// rules/policy、member index、aggregate index active 全部
    /// `not_started + prepare`——即 owner 报告的真机现场。
    struct AdvanceFixture {
        temp: tempfile::TempDir,
        paths: ProductAppPaths,
        lc_id: String,
        aggregate_root: std::path::PathBuf,
        manifest: LogicalCodebaseManifest,
        policy: AggregatePolicyArtifact,
        rule_digest: String,
    }

    fn advance_fixture() -> AdvanceFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let aggregate_root = temp.path().join("aggregate-root");
        let lc_id = super::tests::create_project_and_lc(&paths, &aggregate_root);

        let repo = aggregate_root.join("repo");
        super::tests::init_git_repository_with_commit(&repo);
        let canonical = std::fs::canonicalize(&repo).unwrap();
        let source =
            crate::product::repository_store::resolve_repository_source(&canonical).unwrap();
        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);
        let mut manifest =
            LogicalCodebaseManifest::new(PROJECT_ID, aggregate_root.clone(), Vec::new());
        manifest.member_ids = vec![member_id];
        lc_store.save_manifest(PROJECT_ID, &manifest).unwrap();
        lc_store
            .save_member(
                PROJECT_ID,
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    alias: "repo".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: "2026-10-03T00:00:00Z".to_string(),
                    updated_at: "2026-10-03T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        lc_store
            .save_checkout(
                PROJECT_ID,
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: crate::product::id::repo_hash_for_path(
                        canonical.to_string_lossy().as_ref(),
                    ),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-10-03T00:00:00Z".to_string(),
                    created_at: "2026-10-03T00:00:00Z".to_string(),
                    updated_at: "2026-10-03T00:00:00Z".to_string(),
                },
            )
            .unwrap();

        // 根规则入口 + 真正文政策（与根配方发布同构，非自举桩）。
        std::fs::write(
            aggregate_root
                .join(crate::product::logical_codebase::root_recipe_receipt::ROOT_RULE_ENTRY_FILE),
            "# aggregate root rules\n",
        )
        .unwrap();
        let rule_digest = crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(
            &std::fs::canonicalize(&aggregate_root).unwrap(),
        )
        .unwrap()
        .expect("root rule digest");
        let policy = super::tests::real_root_policy_revision_one(
            PROJECT_ID,
            &manifest.logical_codebase_id.to_string(),
        );
        AdvanceFixture {
            temp,
            paths,
            lc_id,
            aggregate_root,
            manifest,
            policy,
            rule_digest,
        }
    }

    fn operations(fixture: &AdvanceFixture) -> AggregateInitializationOperationStore {
        AggregateInitializationOperationStore::for_lc(fixture.paths.clone(), fixture.lc_id.clone())
    }

    fn operation_input(fixture: &AdvanceFixture) -> AggregateInitializationOperationInput {
        AggregateInitializationOperationInput {
            idempotency_key: format!("advance-{ADVANCE_OPERATION_ID}"),
            manifest_revision: fixture.manifest.membership_revision,
            policy_digest: fixture.policy.digest.clone(),
            profile_evidence_digest: Some("sha256:profile".to_string()),
            provider_context_root: fixture.aggregate_root.clone(),
            provider: "claude_code".to_string(),
        }
    }

    /// 「跑根配方」的 durable 结果：政策发布 + 五步 Completed operation +
    /// 最终 receipt。等价于生产 provider turn 收口后的事实面。
    fn materialize_root_recipe(
        paths: &ProductAppPaths,
        lc_id: &str,
        aggregate_root: &std::path::Path,
        manifest: &LogicalCodebaseManifest,
        policy: &AggregatePolicyArtifact,
        rule_digest: &str,
    ) {
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            lc_id,
        )
        .save(PROJECT_ID, policy)
        .unwrap();
        let store = AggregateInitializationOperationStore::for_lc(paths.clone(), lc_id);
        let input = AggregateInitializationOperationInput {
            idempotency_key: format!("advance-{ADVANCE_OPERATION_ID}"),
            manifest_revision: manifest.membership_revision,
            policy_digest: policy.digest.clone(),
            profile_evidence_digest: Some("sha256:profile".to_string()),
            provider_context_root: aggregate_root.to_path_buf(),
            provider: "claude_code".to_string(),
        };
        store
            .create_idempotent(AggregateInitializationOperation::new(
                ADVANCE_OPERATION_ID.to_string(),
                PROJECT_ID.to_string(),
                input,
                "2026-10-03T00:01:00Z".to_string(),
            ))
            .unwrap();
        store
            .mark_running(
                PROJECT_ID,
                ADVANCE_OPERATION_ID,
                "2026-10-03T00:02:00Z".to_string(),
            )
            .unwrap();
        for step in AggregateInitializationStepKind::V1 {
            store
                .mark_step_running(
                    PROJECT_ID,
                    ADVANCE_OPERATION_ID,
                    step,
                    format!("advance:{ADVANCE_OPERATION_ID}:{}", step.as_str()),
                    "2026-10-03T00:03:00Z".to_string(),
                )
                .unwrap();
            store
                .checkpoint_step_output(
                    PROJECT_ID,
                    ADVANCE_OPERATION_ID,
                    step,
                    format!("aggregate-initializations/op/{}.json", step.as_str()),
                    "2026-10-03T00:04:00Z".to_string(),
                )
                .unwrap();
            store
                .mark_step_completed(
                    PROJECT_ID,
                    ADVANCE_OPERATION_ID,
                    step,
                    "2026-10-03T00:05:00Z".to_string(),
                )
                .unwrap();
        }
        store
            .finish_completed(
                PROJECT_ID,
                ADVANCE_OPERATION_ID,
                "2026-10-03T00:06:00Z".to_string(),
            )
            .unwrap();
        super::tests::finalize_root_receipt(
            paths,
            lc_id,
            ADVANCE_OPERATION_ID,
            aggregate_root,
            &policy.digest,
            rule_digest,
        );
    }

    fn mark_root_recipe_failed(fixture: &AdvanceFixture) {
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            fixture.paths.clone(),
            fixture.lc_id.clone(),
        )
        .save(PROJECT_ID, &fixture.policy)
        .unwrap();
        let store = operations(fixture);
        store
            .create_idempotent(AggregateInitializationOperation::new(
                ADVANCE_OPERATION_ID.to_string(),
                PROJECT_ID.to_string(),
                operation_input(fixture),
                "2026-10-03T00:01:00Z".to_string(),
            ))
            .unwrap();
        store
            .mark_running(
                PROJECT_ID,
                ADVANCE_OPERATION_ID,
                "2026-10-03T00:02:00Z".to_string(),
            )
            .unwrap();
        store
            .finish_failed(
                PROJECT_ID,
                ADVANCE_OPERATION_ID,
                None,
                AggregateInitializationErrorRecord {
                    stage: "pre_check".to_string(),
                    reason_code: "root_recipe_probe_failed".to_string(),
                    stderr_summary: Some("provider probe failed".to_string()),
                    retryable: true,
                    action: "retry_provider_turn".to_string(),
                    cause: None,
                    provider: None,
                },
                "2026-10-03T00:02:30Z".to_string(),
            )
            .unwrap();
    }

    /// 注入「跑根配方」与「重建活跃索引」两个真实派发面（provider/CLI 由
    /// 各自的 durable 结果面替代），并统计准备次数。
    fn service_with(
        fixture: &AdvanceFixture,
        prepare_calls: Arc<AtomicUsize>,
        prepare_delay: Option<Duration>,
    ) -> LogicalCodebaseBootstrapService {
        let paths = fixture.paths.clone();
        let lc_id = fixture.lc_id.clone();
        let aggregate_root = fixture.aggregate_root.clone();
        let manifest = fixture.manifest.clone();
        let policy = fixture.policy.clone();
        let rule_digest = fixture.rule_digest.clone();
        let index_store = AggregateIndexStore::for_lc(fixture.paths.clone(), fixture.lc_id.clone());
        LogicalCodebaseBootstrapService::new(paths.clone())
            .with_root_recipe_prepare(Arc::new(move |_request: &BootstrapPrepareRequest| {
                prepare_calls.fetch_add(1, Ordering::SeqCst);
                if let Some(delay) = prepare_delay {
                    std::thread::sleep(delay);
                }
                materialize_root_recipe(
                    &paths,
                    &lc_id,
                    &aggregate_root,
                    &manifest,
                    &policy,
                    &rule_digest,
                );
                BootstrapPrepareOutcome::Started
            }))
            .with_aggregate_index_rebuild(Arc::new(
                move |project_id: &str, command_id: &str, expected_revision: u64| {
                    let mut record = AggregateIndexRecord::building(
                        ADVANCE_INDEX_ID.to_string(),
                        project_id.to_string(),
                        expected_revision,
                        Vec::new(),
                        "2026-10-03T00:07:00Z".to_string(),
                    );
                    record.command_id = Some(command_id.to_string());
                    record.config_digest = "sha256:advance-index-config".to_string();
                    index_store
                        .create(project_id, record)
                        .expect("seed the building index generation");
                    index_store.mark_status(
                        project_id,
                        ADVANCE_INDEX_ID,
                        AggregateIndexStatus::Active,
                        None,
                    )
                },
            ))
    }

    fn advance_request(fixture: &AdvanceFixture) -> BootstrapAdvanceRequest {
        BootstrapAdvanceRequest {
            project_id: PROJECT_ID.to_string(),
            logical_codebase_id: fixture.lc_id.clone(),
        }
    }

    fn advanced_steps(result: &BootstrapAdvanceResult) -> Vec<LogicalCodebaseBootstrapStep> {
        result
            .advanced_steps
            .iter()
            .map(|attempt| attempt.step)
            .collect()
    }

    /// ①：现场五步中后三步均 `not_started + prepare` → advance 按依赖顺序
    /// 全部推进到 planning_ready。
    #[tokio::test]
    async fn advance_drives_the_reported_field_state_to_planning_ready() {
        let fixture = advance_fixture();
        let before = LogicalCodebaseBootstrapProjector::new(fixture.paths.clone())
            .project(PROJECT_ID, &fixture.lc_id)
            .unwrap();
        let blocked: Vec<_> = before
            .steps
            .iter()
            .filter(|step| step.status == LogicalCodebaseBootstrapStepStatus::NotStarted)
            .map(|step| step.step)
            .collect();
        assert_eq!(
            blocked,
            vec![
                LogicalCodebaseBootstrapStep::RulesPolicy,
                LogicalCodebaseBootstrapStep::MemberIndex,
                LogicalCodebaseBootstrapStep::AggregateIndexActive,
            ],
            "fixture must reproduce the reported field state"
        );
        for step in &before.steps {
            if blocked.contains(&step.step) {
                assert_eq!(
                    step.allowed_actions,
                    vec![BootstrapActionKind::Prepare],
                    "{} must offer prepare",
                    step.step.as_str()
                );
            }
        }
        assert!(!before.planning_ready);

        let prepare_calls = Arc::new(AtomicUsize::new(0));
        let service = service_with(&fixture, prepare_calls.clone(), None);
        let result = service
            .advance(advance_request(&fixture))
            .await
            .expect("advance must not fail on a healthy cold start");

        assert_eq!(result.stopped, BootstrapAdvanceStop::PlanningReady);
        assert!(result.projection.planning_ready, "{:?}", result.stopped);
        assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            advanced_steps(&result),
            vec![
                LogicalCodebaseBootstrapStep::RulesPolicy,
                LogicalCodebaseBootstrapStep::AggregateIndexActive,
            ],
            "member index rides on the same root recipe operation"
        );
        for step in &result.projection.steps {
            assert_eq!(
                step.status,
                LogicalCodebaseBootstrapStepStatus::Completed,
                "{} must be completed once planning is ready",
                step.step.as_str()
            );
        }
    }

    /// ②：重复 advance 幂等——不重复准备根配方、不改写任何 durable 事实。
    #[tokio::test]
    async fn repeated_advance_is_idempotent() {
        let fixture = advance_fixture();
        let prepare_calls = Arc::new(AtomicUsize::new(0));
        let service = service_with(&fixture, prepare_calls.clone(), None);
        let request = advance_request(&fixture);

        let first = service.advance(request.clone()).await.unwrap();
        assert!(first.projection.planning_ready);
        let inventory_before = super::tests::aria_inventory(fixture.temp.path());

        let second = service.advance(request).await.unwrap();
        assert_eq!(second.stopped, BootstrapAdvanceStop::PlanningReady);
        assert!(
            second.advanced_steps.is_empty(),
            "a replayed advance must not claim progress: {:?}",
            advanced_steps(&second)
        );
        assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
        assert_eq!(
            super::tests::aria_inventory(fixture.temp.path()),
            inventory_before,
            "a replayed advance must not write any durable fact"
        );
        assert_eq!(
            operations(&fixture).list(PROJECT_ID).unwrap().len(),
            1,
            "no second root recipe operation may be created"
        );
    }

    /// ③：两个并发 advance 不双推——LC 级排他锁把两侧串行化，准备只发生
    /// 一次，且只产出一个根配方 operation。
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn concurrent_advance_calls_do_not_double_push() {
        let fixture = advance_fixture();
        let prepare_calls = Arc::new(AtomicUsize::new(0));
        // 准备回调占锁 150ms：给并发 advance 真实的竞争窗口。
        let service = service_with(
            &fixture,
            prepare_calls.clone(),
            Some(Duration::from_millis(150)),
        );
        let request = advance_request(&fixture);
        let first_service = service.clone();
        let first_request = request.clone();
        let first = tokio::spawn(async move { first_service.advance(first_request).await });
        let second_service = service.clone();
        let second = tokio::spawn(async move { second_service.advance(request).await });
        let first = first.await.unwrap().expect("first advance must not fail");
        let second = second.await.unwrap().expect("second advance must not fail");

        assert_eq!(
            prepare_calls.load(Ordering::SeqCst),
            1,
            "the LC-scoped advance lock must serialize both callers"
        );
        assert_eq!(
            operations(&fixture).list(PROJECT_ID).unwrap().len(),
            1,
            "exactly one root recipe operation may exist"
        );
        assert!(
            first.advanced_steps.len() + second.advanced_steps.len() >= 1,
            "one advance must make progress"
        );
        assert!(
            first.projection.planning_ready || second.projection.planning_ready,
            "the serialized chain must still reach planning ready"
        );
    }

    /// ④：某步 Failed 时 advance 停止，不自动重试、不改写成 completed。
    #[tokio::test]
    async fn advance_stops_at_a_failed_step_without_retrying_it() {
        let fixture = advance_fixture();
        mark_root_recipe_failed(&fixture);
        let prepare_calls = Arc::new(AtomicUsize::new(0));
        let service = service_with(&fixture, prepare_calls.clone(), None);
        let result = service
            .advance(advance_request(&fixture))
            .await
            .expect("a failed step must be reported, not raised");

        assert_eq!(
            result.stopped,
            BootstrapAdvanceStop::Failed {
                step: LogicalCodebaseBootstrapStep::RulesPolicy,
                reason_code: "root_recipe_probe_failed".to_string(),
            }
        );
        assert!(result.advanced_steps.is_empty());
        assert_eq!(
            prepare_calls.load(Ordering::SeqCst),
            0,
            "a failed step must never be retried automatically"
        );
        let rules = result
            .projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::RulesPolicy)
            .unwrap();
        assert_eq!(rules.status, LogicalCodebaseBootstrapStepStatus::Failed);
        assert_eq!(
            operations(&fixture)
                .get(PROJECT_ID, ADVANCE_OPERATION_ID)
                .unwrap()
                .status,
            AggregateInitializationOperationStatus::Failed,
            "advance must not rewrite the failed operation"
        );
        assert_eq!(operations(&fixture).list(PROJECT_ID).unwrap().len(), 1);
    }

    /// 推进器不注入根配方派发器时，不假装推进（Prepare 无处可去即停）。
    #[tokio::test]
    async fn advance_without_a_prepare_dispatcher_does_not_fake_progress() {
        let fixture = advance_fixture();
        let service = LogicalCodebaseBootstrapService::new(fixture.paths.clone());
        let result = service.advance(advance_request(&fixture)).await.unwrap();
        assert_eq!(
            result.stopped,
            BootstrapAdvanceStop::NoActionableStep {
                step: LogicalCodebaseBootstrapStep::RulesPolicy,
                reason_code: "bootstrap_prepare_unavailable".to_string(),
            }
        );
        assert!(result.advanced_steps.is_empty());
        assert!(operations(&fixture).list(PROJECT_ID).unwrap().is_empty());
    }

    /// 稳定 command 身份：同 lc/step/revision 派生同 id，revision 变化改变
    /// id；非安全字符被折叠，仍保持 relative-id 安全。
    #[test]
    fn advance_command_identity_is_stable_and_path_safe() {
        let first = advance_command_id(
            "logical_codebase_abc",
            LogicalCodebaseBootstrapStep::RulesPolicy,
            Some(3),
        );
        assert_eq!(
            first,
            advance_command_id(
                "logical_codebase_abc",
                LogicalCodebaseBootstrapStep::RulesPolicy,
                Some(3),
            )
        );
        assert_eq!(
            first,
            "bootstrap-advance-logical_codebase_abc-rules_policy-rev3"
        );
        assert_ne!(
            first,
            advance_command_id(
                "logical_codebase_abc",
                LogicalCodebaseBootstrapStep::RulesPolicy,
                Some(4),
            )
        );
        let sanitized =
            advance_command_id("../lc/x", LogicalCodebaseBootstrapStep::MemberIndex, None);
        assert!(!sanitized.contains('/'));
        assert!(!sanitized.contains(".."));
        crate::product::json_store::validate_relative_id(&sanitized)
            .expect("derived command id must stay path safe");
    }

    /// ⑤（2026-11-04 真机缺陷）：prepare 被拒（provider 选择门/信任门同
    /// 路径）⇒ 停止解释成为 durable 事实——GET 同构投影透出 `advance_stop`
    /// 并合成恰好一条通知；绝不创建 operation；放行后推进到 PlanningReady
    /// 时清除记录。
    #[tokio::test]
    async fn advance_waiting_stop_record_is_durable_and_cleared_on_planning_ready() {
        let fixture = advance_fixture();
        let rejected = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let prepare_calls = Arc::new(AtomicUsize::new(0));
        let service = service_with_rejection(
            &fixture,
            prepare_calls.clone(),
            rejected.clone(),
            "aggregate_initialization_provider_needs_verification",
            "no provider carries verified capability evidence",
        );

        let first = service.advance(advance_request(&fixture)).await.unwrap();
        assert_eq!(
            first.stopped,
            BootstrapAdvanceStop::WaitingForHuman {
                step: LogicalCodebaseBootstrapStep::RulesPolicy,
                reason_code: "aggregate_initialization_provider_needs_verification".to_string(),
                detail: Some(
                    "no provider carries verified capability evidence".to_string()
                ),
            }
        );
        assert!(first.advanced_steps.is_empty());
        assert!(
            first.projection.advance_stop.is_some(),
            "the advance response projection must carry the stop record"
        );
        // 拒绝不改写 durable 事实：根配方 operation 仍不存在。
        assert!(operations(&fixture).list(PROJECT_ID).unwrap().is_empty());

        // GET 同构投影：步骤仍 not_started，但带上 stop 记录 + 恰好一条通知。
        let projection = LogicalCodebaseBootstrapProjector::new(fixture.paths.clone())
            .project(PROJECT_ID, &fixture.lc_id)
            .unwrap();
        let record = projection
            .advance_stop
            .as_ref()
            .expect("stop record must be projected");
        assert_eq!(record.step, LogicalCodebaseBootstrapStep::RulesPolicy);
        assert_eq!(record.reason, "waiting_for_human");
        assert_eq!(
            record.reason_code,
            "aggregate_initialization_provider_needs_verification"
        );
        assert_eq!(
            record.detail.as_deref(),
            Some("no provider carries verified capability evidence")
        );
        let rules = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::RulesPolicy)
            .unwrap();
        assert_eq!(rules.status, LogicalCodebaseBootstrapStepStatus::NotStarted);
        let stop_notices: Vec<_> = projection
            .notices
            .iter()
            .filter(|notice| notice.key.starts_with("bootstrap:advance-stop:"))
            .collect();
        assert_eq!(stop_notices.len(), 1, "{:?}", projection.notices);
        assert_eq!(
            stop_notices[0].reason_code,
            "aggregate_initialization_provider_needs_verification"
        );
        assert!(!stop_notices[0].summary.is_empty());
        assert_eq!(stop_notices[0].step, LogicalCodebaseBootstrapStep::RulesPolicy);

        // 放行后再次 advance → PlanningReady；停止解释被清除。
        rejected.store(false, Ordering::SeqCst);
        let second = service.advance(advance_request(&fixture)).await.unwrap();
        assert_eq!(second.stopped, BootstrapAdvanceStop::PlanningReady);
        assert!(second.projection.planning_ready);
        assert!(second.projection.advance_stop.is_none());
        assert!(second.projection.notices.iter().all(|notice| {
            !notice.key.starts_with("bootstrap:advance-stop:")
        }));
        assert_eq!(prepare_calls.load(Ordering::SeqCst), 2);
    }

    /// ⑥（2026-11-04）：rules_policy 的手动「准备」与自动推进同一条派发
    /// 链：被拒 ⇒ 落 stop 记录并返回 waiting_for_human（响应投影携带解
    /// 释，而不是无声等待）；放行 ⇒ 真正跑起根配方（Accepted）并清除旧
    /// 记录。
    #[tokio::test]
    async fn manual_prepare_on_rules_policy_routes_the_root_recipe_dispatcher() {
        let fixture = advance_fixture();
        let rejected = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let prepare_calls = Arc::new(AtomicUsize::new(0));
        let service = service_with_rejection(
            &fixture,
            prepare_calls.clone(),
            rejected.clone(),
            "aggregate_initialization_trust_waiting",
            "trust not ready for this provider",
        );
        let projection = LogicalCodebaseBootstrapProjector::new(fixture.paths.clone())
            .project(PROJECT_ID, &fixture.lc_id)
            .unwrap();
        let rules = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::RulesPolicy)
            .unwrap()
            .clone();
        assert_eq!(rules.status, LogicalCodebaseBootstrapStepStatus::NotStarted);
        assert_eq!(rules.allowed_actions, vec![BootstrapActionKind::Prepare]);

        let rejected_result = service
            .apply(BootstrapActionRequest {
                command_id: "manual-prepare-rules-policy-0001".to_string(),
                project_id: PROJECT_ID.to_string(),
                logical_codebase_id: fixture.lc_id.clone(),
                step: LogicalCodebaseBootstrapStep::RulesPolicy,
                action: BootstrapActionKind::Prepare,
                expected_revision: projection.membership_revision,
                expected_object_id: rules.object_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(rejected_result.outcome, BootstrapActionOutcome::WaitingForHuman);
        assert_eq!(prepare_calls.load(Ordering::SeqCst), 1);
        let stop = rejected_result
            .projection
            .advance_stop
            .as_ref()
            .expect("the rejection must be visible in the response projection");
        assert_eq!(stop.reason, "waiting_for_human");
        assert_eq!(stop.reason_code, "aggregate_initialization_trust_waiting");
        assert_eq!(
            stop.detail.as_deref(),
            Some("trust not ready for this provider")
        );

        rejected.store(false, Ordering::SeqCst);
        let started = service
            .apply(BootstrapActionRequest {
                command_id: "manual-prepare-rules-policy-0002".to_string(),
                project_id: PROJECT_ID.to_string(),
                logical_codebase_id: fixture.lc_id.clone(),
                step: LogicalCodebaseBootstrapStep::RulesPolicy,
                action: BootstrapActionKind::Prepare,
                expected_revision: projection.membership_revision,
                expected_object_id: rules.object_id.clone(),
            })
            .await
            .unwrap();
        assert_eq!(started.outcome, BootstrapActionOutcome::Accepted);
        assert_eq!(prepare_calls.load(Ordering::SeqCst), 2);
        let rules_after = started
            .projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::RulesPolicy)
            .unwrap();
        assert_eq!(
            rules_after.status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );
        assert!(started.projection.advance_stop.is_none());
        assert!(started
            .projection
            .notices
            .iter()
            .all(|notice| !notice.key.starts_with("bootstrap:advance-stop:")));
    }

    /// ⑤/⑥ 共用：先拒（携带稳定码 + detail）、放行后物化根配方的派发器。
    fn service_with_rejection(
        fixture: &AdvanceFixture,
        prepare_calls: Arc<AtomicUsize>,
        rejected: Arc<std::sync::atomic::AtomicBool>,
        reason_code: &'static str,
        detail: &'static str,
    ) -> LogicalCodebaseBootstrapService {
        let paths = fixture.paths.clone();
        let lc_id = fixture.lc_id.clone();
        let aggregate_root = fixture.aggregate_root.clone();
        let manifest = fixture.manifest.clone();
        let policy = fixture.policy.clone();
        let rule_digest = fixture.rule_digest.clone();
        let index_store = AggregateIndexStore::for_lc(fixture.paths.clone(), fixture.lc_id.clone());
        LogicalCodebaseBootstrapService::new(paths.clone())
            .with_root_recipe_prepare(Arc::new(
            move |_request: &BootstrapPrepareRequest| {
                prepare_calls.fetch_add(1, Ordering::SeqCst);
                if rejected.load(Ordering::SeqCst) {
                    return BootstrapPrepareOutcome::WaitingForHuman {
                        reason_code: reason_code.to_string(),
                        detail: Some(detail.to_string()),
                    };
                }
                materialize_root_recipe(
                    &paths,
                    &lc_id,
                    &aggregate_root,
                    &manifest,
                    &policy,
                    &rule_digest,
                );
                BootstrapPrepareOutcome::Started
            },
        ))
        // 与 service_with 相同的索引重建派发面：放行后的 advance 会继续
        // 推进到 aggregate_index_active。
        .with_aggregate_index_rebuild(Arc::new(
            move |project_id: &str, command_id: &str, expected_revision: u64| {
                let mut record = AggregateIndexRecord::building(
                    ADVANCE_INDEX_ID.to_string(),
                    project_id.to_string(),
                    expected_revision,
                    Vec::new(),
                    "2026-11-04T00:07:00Z".to_string(),
                );
                record.command_id = Some(command_id.to_string());
                record.config_digest = "sha256:advance-index-config".to_string();
                index_store
                    .create(project_id, record)
                    .expect("seed the building index generation");
                index_store.mark_status(
                    project_id,
                    ADVANCE_INDEX_ID,
                    AggregateIndexStatus::Active,
                    None,
                )
            },
        ))
    }
}
