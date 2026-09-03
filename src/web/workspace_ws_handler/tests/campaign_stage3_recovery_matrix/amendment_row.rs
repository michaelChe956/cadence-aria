// ============================================================================
// 修订行（深行）：amendment 四窗口崩溃/重连矩阵。
// 每个窗口 = 崩溃（真实 Error 模式或与生产 crash 落盘形态一致的 journal
// 前缀 seed）→ 重启（从 durable 记录重建引擎，零内存态）→ 同 command 恢复。
// 统一断言：同 attempt、原预算源（原 plan session 门快照）、binding 前缀
// 不变（直到恰一次应用）、resume target 恰一次。
// ============================================================================

/// 原预算源对账：amendment 链的预算只存在于原 plan session 的门快照。
fn matrix_plan_session_budget(fixture: &CampaignAmendmentFixture) -> u32 {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.plan_session_id)
        .expect("plan session durable")
        .human_gate_snapshot
        .as_ref()
        .expect("gate snapshot")
        .manual_repairs_remaining
}

/// provider 账本对账：以 durable ledger 长度为准（不以 WS 事件计数）。
fn matrix_provider_ledger_len(fixture: &CampaignAmendmentFixture) -> usize {
    fixture
        .lifecycle
        .get_workspace_session(&fixture.plan_session_id)
        .expect("plan session durable")
        .provider_start_ledger
        .len()
}

/// 修订行统一收口断言（四窗口恢复后必须全部成立）。
fn assert_amendment_row_final_invariants(
    fixture: &CampaignAmendmentFixture,
    context: &crate::product::coding_models::PlanAmendmentContext,
    manifest: &PlanAmendmentManifest,
    expected_budget: u32,
    expected_ledger_len: usize,
) -> crate::product::coding_models::CodingExecutionAttempt {
    let resumed = fixture.durable_attempt();
    assert_eq!(resumed.id, fixture.attempt.id, "恢复必须回到原 attempt（不新建）");
    assert_eq!(
        fixture
            .store
            .list_attempts_for_issue("project_0001", "issue_0001")
            .expect("attempts for issue")
            .len(),
        1,
        "全 issue 恒一个 group attempt"
    );
    let binding = fixture.store.get_plan_binding(&resumed).expect("binding");
    assert_eq!(
        binding.bound_plan_revision_id,
        manifest.new_plan_revision_id,
        "binding 恰一次切换到新 revision"
    );
    assert_eq!(
        binding.applied_amendment_ids,
        vec![manifest.id.clone()],
        "applied amendment 恰一次"
    );
    let applied_context = fixture.trigger_context();
    assert_eq!(applied_context.id, context.id);
    assert_eq!(applied_context.status, PlanAmendmentContextStatus::Applied);
    assert_eq!(
        applied_context.previous_plan_revision_id, context.previous_plan_revision_id,
        "previous revision 留在 context"
    );
    assert_eq!(
        applied_context.new_plan_revision_id.as_deref(),
        Some(manifest.new_plan_revision_id.as_str())
    );
    assert_eq!(
        applied_context.resume_target, manifest.resume_target,
        "resume target 恰好登记一次"
    );
    let journal = fixture
        .store
        .get_amendment_application_journal(&resumed, &manifest.id)
        .expect("application journal");
    assert_eq!(journal.phase, CodingAmendmentApplicationPhase::Completed);
    assert_eq!(journal.error, None, "恢复后 journal 错误清零");
    let delivery = fixture
        .store
        .get_plan_amendment_delivery(&resumed, &manifest.id)
        .expect("delivery marker");
    assert_eq!(delivery.status, CodingPlanAmendmentDeliveryStatus::Delivered);
    // resume 单元状态由真实 manifest resume_target 产生（8.4a 同款自适应断言）。
    let resume_unit = fixture
        .store
        .list_coding_units("project_0001", "issue_0001", &resumed.id)
        .expect("units")
        .into_iter()
        .find(|unit| unit.logical_work_item_id == manifest.resume_target.logical_work_item_id)
        .expect("resume target unit");
    match manifest.resume_target.mode {
        AmendmentResumeMode::Reexecute => {
            assert_eq!(resume_unit.status, CodingExecutionUnitStatus::Running);
            assert_eq!(resumed.status, CodingAttemptStatus::Running);
        }
        AmendmentResumeMode::Revalidate => {
            assert_eq!(resume_unit.status, CodingExecutionUnitStatus::NeedsRevalidation);
            assert_eq!(resumed.stage, crate::product::coding_models::CodingExecutionStage::CodeReview);
        }
        AmendmentResumeMode::AwaitHandoff => {
            assert_eq!(resume_unit.status, CodingExecutionUnitStatus::AwaitingAmendment);
            assert_eq!(
                resumed.status,
                CodingAttemptStatus::AwaitingPlanAmendment,
                "AwaitHandoff：恢复后回到等待 handoff 的暂停位"
            );
            assert_eq!(resumed.active_unit_id.as_deref(), Some(resume_unit.id.as_str()));
        }
    }
    // 原 plan session 的重开门关回 Confirmed；原预算源/ledger 终值对账。
    let closed = fixture
        .lifecycle
        .get_workspace_session(&fixture.plan_session_id)
        .expect("plan session after resume");
    assert_eq!(closed.status, WorkspaceSessionStatus::Confirmed);
    assert_eq!(
        closed
            .human_gate_snapshot
            .as_ref()
            .expect("snapshot")
            .manual_repairs_remaining,
        expected_budget,
        "原预算源终值（只按真实 turn 扣减）"
    );
    assert_eq!(
        closed.provider_start_ledger.len(),
        expected_ledger_len,
        "provider ledger 终值（以 ledger 对账）"
    );
    resumed
}

/// 窗口 1 —— context：计划缺陷暂停（context Open）后断开重连，
/// 原 plan session 上同 command（typed feedback）恰一次受理，
/// group attempt/binding/context 零切换；重发 Replay 零增量。
#[tokio::test]
async fn campaign_stage3_recovery_matrix_amendment_context_window_reconnect_resumes_same_command()
{
    let fixture = campaign_amendment_fixture().await;
    let paused = fixture.durable_attempt();
    assert_eq!(paused.status, CodingAttemptStatus::AwaitingPlanAmendment);
    let context = fixture.trigger_context();
    assert_eq!(context.status, PlanAmendmentContextStatus::Open);
    assert_eq!(context.plan_session_id, fixture.plan_session_id);
    let binding_before = fixture.store.get_plan_binding(&paused).expect("binding");
    assert_eq!(
        binding_before.bound_plan_revision_id, context.previous_plan_revision_id,
        "binding 前缀 = context 登记的 previous revision"
    );
    assert_eq!(matrix_plan_session_budget(&fixture), 2, "原预算源初始值");
    assert_eq!(matrix_provider_ledger_len(&fixture), 0);
    let sessions_before = fixture
        .lifecycle
        .list_workspace_sessions("project_0001", "issue_0001")
        .expect("sessions")
        .len();

    // WS 断开 + 进程/存储重开：丢弃内存态，仅从 durable 记录重建引擎。
    {
        let mut recovered = fixture.plan_session_engine("matrix-context-reconnect");
        let actions = recovered
            .recover_human_gate_turns(false)
            .expect("recover turns");
        assert!(
            actions.is_empty(),
            "context 窗口无 in-flight turn，恢复零动作: {actions:?}"
        );
        // 重开后原 session/context 仍是权威。
        assert_eq!(
            fixture.durable_attempt().status,
            CodingAttemptStatus::AwaitingPlanAmendment
        );
        assert_eq!(fixture.trigger_context().id, context.id);
    }

    // 同 command：重开后的 worker 上 typed feedback 受理，turn 恰一次。
    let command_id = "cmd-campaign-matrix-amend-context";
    let turn_id = open_amendment_turn_and_run_fake_revision(&fixture, command_id).await;
    let turns = fixture
        .lifecycle
        .list_human_gate_turns(&fixture.plan_session_id)
        .expect("durable turns");
    assert_eq!(turns.len(), 1, "恰好一个 amendment turn");
    assert_eq!(turns[0].turn_id, turn_id);
    assert_eq!(turns[0].status, HumanGateTurnStatus::Completed);
    assert_eq!(turns[0].command_id, command_id);
    assert_eq!(
        matrix_plan_session_budget(&fixture),
        1,
        "预算只从原 plan session 扣一次"
    );
    assert_eq!(matrix_provider_ledger_len(&fixture), 1);
    // group attempt 侧无预算账：attempt 逐字段不变、不新增 session、无 open gate。
    assert_eq!(
        fixture.durable_attempt(),
        paused,
        "context 窗口 group attempt 逐字段不变"
    );
    assert!(
        fixture
            .store
            .list_open_blocked_gates("project_0001", "issue_0001", &paused.id)
            .expect("open gates")
            .is_empty(),
        "group attempt 不开人工门"
    );
    assert_eq!(
        fixture
            .lifecycle
            .list_workspace_sessions("project_0001", "issue_0001")
            .expect("sessions after feedback")
            .len(),
        sessions_before,
        "不产生第二个人工门 session"
    );
    // binding 前缀不变：context 窗口不切换 revision。
    assert_eq!(
        fixture.store.get_plan_binding(&paused).expect("binding after"),
        binding_before
    );
    assert_eq!(
        fixture.trigger_context().status,
        PlanAmendmentContextStatus::Open
    );

    // 重连后再同 command 重发：Replay 同一 turn，预算/ledger 零增量。
    let mut replay_engine = fixture.plan_session_engine("matrix-context-replay");
    let replayed = replay_engine
        .handle_human_gate_feedback(HumanGateFeedbackInput {
            command_id: command_id.to_string(),
            feedback: "同 command 重发（context 窗口重连后）".to_string(),
        })
        .await
        .expect("replay");
    assert!(matches!(
        &replayed,
        HumanGateCommandOutcome::Replayed { turn } if turn.turn_id == turn_id
    ));
    assert_eq!(matrix_plan_session_budget(&fixture), 1);
    assert_eq!(matrix_provider_ledger_len(&fixture), 1);
    assert_eq!(fixture.durable_attempt(), paused);
}

/// 窗口 2 —— new revision：出版 journal 七 checkpoint 逐一崩溃+重启+同
/// command（同一 confirm 入口重发）恢复；崩溃窗口内 lock 不释放、coding
/// binding 前缀不变、request 不 Published；恢复后全链收口回原 attempt。
#[tokio::test]
async fn campaign_stage3_recovery_matrix_amendment_publication_checkpoints_resume_same_command()
{
    let cases = [
        (
            PlanAmendmentPublicationCheckpoint::JournalPreparing,
            PlanAmendmentPublicationPhase::Preparing,
            false,
            false,
        ),
        (
            PlanAmendmentPublicationCheckpoint::FirstArtifactsWritten,
            PlanAmendmentPublicationPhase::Preparing,
            false,
            false,
        ),
        (
            PlanAmendmentPublicationCheckpoint::JournalPrepared,
            PlanAmendmentPublicationPhase::Prepared,
            false,
            false,
        ),
        (
            PlanAmendmentPublicationCheckpoint::FirstActiveWorkItemRevisionPublished,
            PlanAmendmentPublicationPhase::Prepared,
            false,
            true,
        ),
        (
            PlanAmendmentPublicationCheckpoint::JournalWorkItemsPublished,
            PlanAmendmentPublicationPhase::WorkItemsPublished,
            false,
            true,
        ),
        (
            PlanAmendmentPublicationCheckpoint::ActivePlanRevisionPublished,
            PlanAmendmentPublicationPhase::WorkItemsPublished,
            true,
            true,
        ),
        (
            PlanAmendmentPublicationCheckpoint::JournalPlanPublished,
            PlanAmendmentPublicationPhase::PlanPublished,
            true,
            true,
        ),
    ];
    for (checkpoint, failed_phase, plan_revision_switched, work_item_revision_switched) in cases {
        let label = format!("{checkpoint:?}");
        let fixture = campaign_amendment_fixture().await;
        let paused = fixture.durable_attempt();
        let context = fixture.trigger_context();
        // 出版前半程（真实 prepare/persist/attestation/awaiting），停在确认前。
        let (prepared, _attestation) = stage_amendment_candidate(&fixture).await;
        let manifest_id = prepared.manifest.id.clone();
        let previous_plan_revision_id = context.previous_plan_revision_id.clone();
        let next_plan_revision_id = prepared.manifest.new_plan_revision_id.clone();
        let previous_wi_revision_id = fixture
            .revision_store
            .get_plan_revision(
                "project_0001",
                "issue_0001",
                &fixture.plan.id,
                &previous_plan_revision_id,
            )
            .expect("pre-publish plan revision")
            .work_item_bindings
            .get("wi_a")
            .expect("wi_a binding")
            .clone();
        let next_wi_revision_id = prepared
            .manifest
            .revised_work_items
            .get("wi_a")
            .expect("revised wi_a")
            .next_revision_id
            .clone();

        // 崩溃注入：出版 failpoint（Error 模式，一次性）。
        let _failpoint = register_plan_amendment_publication_failpoint(
            &fixture.revision_store,
            &fixture.plan,
            &prepared.publication_ids.journal_id,
            checkpoint,
        );
        let crash = fixture
            .child_engine()
            .confirm_and_publish_plan_amendment(&manifest_id, "workspace_user")
            .await;
        let crash_error = format!("{crash:?}");
        assert!(
            crash.is_err() && crash_error.contains("amendment_publication_failpoint"),
            "{label}: 出版必须在 checkpoint 崩溃: {crash_error}"
        );

        // 崩溃后 durable 证据（磁盘重开读取）。
        let failed_lineage = fixture
            .revision_store
            .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
            .expect("failed lineage");
        assert_eq!(
            failed_lineage.active_revision_id.as_deref(),
            Some(if plan_revision_switched {
                next_plan_revision_id.as_str()
            } else {
                previous_plan_revision_id.as_str()
            }),
            "{label}: checkpoint 后 active plan revision 形态"
        );
        let failed_wi = fixture
            .revision_store
            .get_logical_work_item(&failed_lineage, "wi_a")
            .expect("failed logical wi");
        assert_eq!(
            failed_wi.active_revision_id.as_deref(),
            Some(if work_item_revision_switched {
                next_wi_revision_id.as_str()
            } else {
                previous_wi_revision_id.as_str()
            }),
            "{label}: checkpoint 后 wi_a active revision 形态"
        );
        assert_eq!(
            failed_lineage.active_amendment_id.as_deref(),
            Some(manifest_id.as_str()),
            "{label}: 崩溃不释放 amendment lock"
        );
        let failed_journal = fixture
            .revision_store
            .get_plan_amendment_publication_journal(
                &failed_lineage,
                &prepared.publication_ids.journal_id,
            )
            .expect("failed journal");
        assert_eq!(failed_journal.phase, failed_phase, "{label}: journal phase");
        assert!(failed_journal.error.is_some(), "{label}: journal 错误留痕");
        assert!(failed_journal.recovery.is_some(), "{label}: recovery 提示留痕");
        assert_eq!(
            fixture
                .revision_store
                .get_repair_request(&failed_lineage, &prepared.manifest.repair_request_id)
                .expect("failed request")
                .status,
            PlanRepairRequestStatus::AwaitingConfirmation,
            "{label}: request 未 Published"
        );
        // campaign 不变量：出版崩溃窗口内 coding 侧零切换。
        assert_eq!(fixture.durable_attempt(), paused, "{label}: 同 attempt 不变");
        assert_eq!(
            fixture
                .store
                .get_plan_binding(&paused)
                .expect("binding")
                .bound_plan_revision_id,
            previous_plan_revision_id,
            "{label}: binding 前缀不变（coding 侧未应用）"
        );
        assert_eq!(
            fixture.trigger_context().status,
            PlanAmendmentContextStatus::Open
        );
        assert_eq!(matrix_plan_session_budget(&fixture), 2, "{label}: 原预算源不动");
        assert_eq!(
            matrix_provider_ledger_len(&fixture),
            0,
            "{label}: 出版链不启动 provider"
        );

        // 重启 + 同 command：从 durable child 记录重建引擎，同一 confirm 入口重发。
        let manifest = confirm_real_amendment_publication(&fixture, &manifest_id).await;
        assert_eq!(manifest.id, manifest_id);
        let published_lineage = fixture
            .revision_store
            .get_plan_lineage("project_0001", "issue_0001", &fixture.plan.id)
            .expect("published lineage");
        assert_eq!(
            published_lineage.active_revision_id.as_deref(),
            Some(next_plan_revision_id.as_str()),
            "{label}: 恢复后新 revision 激活"
        );
        assert_eq!(
            published_lineage.active_amendment_id.as_deref(),
            Some(manifest_id.as_str())
        );
        let published_journal = fixture
            .revision_store
            .get_plan_amendment_publication_journal(
                &published_lineage,
                &prepared.publication_ids.journal_id,
            )
            .expect("published journal");
        assert_eq!(
            published_journal.phase,
            PlanAmendmentPublicationPhase::PlanPublished
        );
        assert_eq!(published_journal.error, None);
        assert_eq!(published_journal.recovery, None);
        assert_eq!(
            fixture
                .revision_store
                .get_repair_request(&published_lineage, &prepared.manifest.repair_request_id)
                .expect("published request")
                .status,
            PlanRepairRequestStatus::Published
        );

        // 全链收口：application 回原 attempt（本窗口未开 turn，预算/ledger 保持
        // 初始值——出版/应用链不需要 provider 预算）。
        let resumed = fixture
            .coding_engine()
            .resume_group_after_plan_amendment(&paused, &context, &manifest)
            .await
            .unwrap_or_else(|error| panic!("{label}: 应用必须回原 attempt: {error}"));
        assert_eq!(resumed.id, paused.id);
        assert_amendment_row_final_invariants(&fixture, &context, &manifest, 2, 0);
    }
}

/// 窗口 3 —— binding：application journal 中途崩溃（journal Started 后/
/// binding 写入前；binding 写入后/unit runs 前）。seed 通道与生产 crash 落盘
/// 形态一致（同 `test_controls/plan_repair/recovery.rs` 手段）；重启后同
/// command（typed 生产入口）从 journal 前缀恢复，binding 恰一次写入。
#[tokio::test]
async fn campaign_stage3_recovery_matrix_amendment_binding_window_resumes_from_journal_prefix() {
    for (label, seed_through_binding) in
        [("started_before_binding", false), ("after_binding_written", true)]
    {
        let fixture = campaign_amendment_fixture().await;
        let paused = fixture.durable_attempt();
        let context = fixture.trigger_context();
        // amendment turn（预算源真实扣减一次，ledger 一项）。
        open_amendment_turn_and_run_fake_revision(
            &fixture,
            &format!("cmd-campaign-matrix-amend-binding-{label}"),
        )
        .await;
        assert_eq!(matrix_plan_session_budget(&fixture), 1);
        assert_eq!(matrix_provider_ledger_len(&fixture), 1);
        let manifest = publish_real_amendment(&fixture).await;
        // 崩溃后状态（journal 前缀，生产 apply 链在对应点中断的落盘形态）。
        assert_eq!(
            fixture
                .store
                .load_or_prepare_amendment_application(&paused, &manifest)
                .expect("journal Started")
                .phase,
            CodingAmendmentApplicationPhase::Started
        );
        fixture
            .store
            .update_attempt_status(
                "project_0001",
                "issue_0001",
                &paused.id,
                CodingAttemptStatus::ApplyingPlanAmendment,
            )
            .expect("applying status");
        if seed_through_binding {
            let applying = fixture.durable_attempt();
            fixture
                .store
                .update_plan_binding_from_manifest(&applying, &manifest)
                .expect("binding written before crash");
            fixture
                .store
                .advance_amendment_application_journal(
                    &applying,
                    &manifest.id,
                    CodingAmendmentApplicationPhase::PlanBindingWritten,
                    None,
                    "2026-06-30T00:00:20Z".to_string(),
                )
                .expect("journal PlanBindingWritten");
        }
        let crashed_attempt = fixture.durable_attempt();
        assert_eq!(
            crashed_attempt.status,
            CodingAttemptStatus::ApplyingPlanAmendment,
            "{label}: crash 落盘形态"
        );
        assert_eq!(crashed_attempt.id, paused.id);
        let crashed_binding = fixture
            .store
            .get_plan_binding(&crashed_attempt)
            .expect("crashed binding");
        if seed_through_binding {
            assert_eq!(
                crashed_binding.bound_plan_revision_id,
                manifest.new_plan_revision_id,
                "{label}: binding 已写（前缀校验锚点）"
            );
        } else {
            assert_eq!(
                crashed_binding.bound_plan_revision_id,
                context.previous_plan_revision_id,
                "{label}: binding 前缀不变"
            );
        }

        // 重启 + 同 command：typed 生产入口从 journal 前缀恢复。
        let resumed = fixture
            .coding_engine()
            .resume_group_after_plan_amendment(&crashed_attempt, &context, &manifest)
            .await
            .unwrap_or_else(|error| panic!("{label}: 必须从 journal 前缀恢复: {error}"));
        assert_eq!(resumed.id, paused.id, "{label}: 同 attempt");
        assert_amendment_row_final_invariants(&fixture, &context, &manifest, 1, 1);
    }
}

/// 窗口 4 —— application journal/delivery：journal Completed 后 delivery 收口
/// 前崩溃，两种 Error 模式（socket 写失败 / delivery-mark failpoint：socket
/// 写成功但 durable mark 前中断）。重启后生产恢复入口重投同一 event_id
/// 恰一次，attempt 回原 id、binding/resume target/budget 终值不变。
#[tokio::test]
async fn campaign_stage3_recovery_matrix_amendment_application_delivery_recovers_same_event() {
    for (label, socket_write_succeeds, expected_first_error) in [
        (
            "socket_write_failed",
            false,
            "plan_amendment_socket_write_failed",
        ),
        ("delivery_mark_crash", true, "delivery_mark_failpoint"),
    ] {
        let fixture = campaign_amendment_fixture().await;
        let paused = fixture.durable_attempt();
        let context = fixture.trigger_context();
        let _turn_id = open_amendment_turn_and_run_fake_revision(
            &fixture,
            &format!("cmd-campaign-matrix-amend-delivery-{label}"),
        )
        .await;
        let manifest = publish_real_amendment(&fixture).await;
        let failpoint = if socket_write_succeeds {
            Some(register_plan_amendment_delivery_mark_failpoint(
                &fixture.store,
                &paused,
                &manifest.id,
            ))
        } else {
            None
        };

        // 首轮：Error 模式中断在 delivery 收口前。
        let observed_first: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let engine = matrix_coding_engine_with_socket_outcome(
            &fixture,
            socket_write_succeeds,
            observed_first.clone(),
        );
        let error = engine
            .resume_group_after_plan_amendment(&paused, &context, &manifest)
            .await
            .expect_err(&format!("{label}: delivery 窗口必须以 Error 模式中断"));
        assert!(
            error.to_string().contains(expected_first_error),
            "{label}: 意外错误: {error}"
        );
        let failed = fixture.durable_attempt();
        assert_eq!(failed.id, paused.id, "{label}: 同 attempt");
        assert_eq!(
            failed.status,
            CodingAttemptStatus::AmendmentApplyFailed,
            "{label}: attempt 不可运行，等待恢复"
        );
        let delivery = fixture
            .store
            .get_plan_amendment_delivery(&failed, &manifest.id)
            .expect("delivery marker");
        assert_eq!(
            delivery.status,
            CodingPlanAmendmentDeliveryStatus::Pending,
            "{label}: marker 悬挂待恢复"
        );
        let journal = fixture
            .store
            .get_amendment_application_journal(&failed, &manifest.id)
            .expect("application journal");
        assert_eq!(
            journal.phase,
            CodingAmendmentApplicationPhase::Completed,
            "{label}: journal 已完成，只差 delivery 收口"
        );
        // binding 已在 delivery 之前切换（前缀校验锚点）；context 回 Open 可恢复。
        assert_eq!(
            fixture
                .store
                .get_plan_binding(&failed)
                .expect("binding")
                .bound_plan_revision_id,
            manifest.new_plan_revision_id
        );
        assert_eq!(
            fixture.trigger_context().status,
            PlanAmendmentContextStatus::Open,
            "{label}: 可重试失败回到 Open"
        );
        let first_event_ids = observed_first.lock().expect("observed").clone();
        assert_eq!(
            first_event_ids.len(),
            1,
            "{label}: 首轮恰一个 delivery 事件（事件身份，非计数对账）"
        );
        drop(failpoint);

        // 重启（全新 coding 引擎 + 成功 socket 写）：生产恢复入口，同 event 补投。
        let observed_recovery: Arc<StdMutex<Vec<String>>> = Arc::new(StdMutex::new(Vec::new()));
        let recovered_engine = matrix_coding_engine_with_socket_outcome(
            &fixture,
            true,
            observed_recovery.clone(),
        );
        let resumed = recovered_engine
            .recover_plan_amendment(&failed)
            .await
            .unwrap_or_else(|error| panic!("{label}: 同 event 恢复必须完成: {error}"));
        assert_eq!(resumed.id, paused.id, "{label}: 恢复回原 attempt");
        let recovered_event_ids = observed_recovery.lock().expect("observed").clone();
        assert_eq!(
            recovered_event_ids, first_event_ids,
            "{label}: 恢复必须重投同一 event_id（恰一次）"
        );
        assert_amendment_row_final_invariants(&fixture, &context, &manifest, 1, 1);
    }
}

/// 带投递观察者的 coding 引擎：记录 `PlanAmendmentUpdated` 的 event_id
/// （身份对账用），并按 mode 回执 socket 写成功/失败（Error 模式）。
fn matrix_coding_engine_with_socket_outcome(
    fixture: &CampaignAmendmentFixture,
    socket_write_succeeds: bool,
    observed_event_ids: Arc<StdMutex<Vec<String>>>,
) -> CodingWorkspaceEngine {
    let (event_tx, mut socket_event_rx) = mpsc::channel(64);
    tokio::spawn(async move {
        while let Some(event) = socket_event_rx.recv().await {
            if let CodingWsOutMessage::PlanAmendmentUpdated { event_id, .. } = &event {
                observed_event_ids
                    .lock()
                    .expect("observed event ids")
                    .push(event_id.clone());
            }
            if socket_write_succeeds {
                confirm_plan_amendment_socket_write(&event);
            } else {
                fail_plan_amendment_socket_write(&event);
            }
        }
    });
    CodingWorkspaceEngine::new(
        fixture.store.clone(),
        GitWorkspaceService::new(),
        event_tx,
    )
}
