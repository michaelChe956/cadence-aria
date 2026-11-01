/// SG7a（plan v1.1 Task SG7a Step 1）：live CAS→镜像 sync→reload 与
/// production `load_durable_workspace_projection`（最新 durable record 纯
/// 投影）逐字段同值——真实 store CAS 链（reserve→claim→settle），不手工
/// 填 wire；纯投影零写（前后 durable 字节不变）；CAS 失败/错 scope 镜像
/// 保持。
#[cfg(test)]
mod sg_07a_tests {
    use super::*;
    use crate::product::json_store::{ProductStoreError, write_json};
    use crate::product::lifecycle_store::inputs::CreateWorkspaceSessionInput;
    use crate::product::models::{
        HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, ProviderName,
        StructuralGovernanceState, StructuralGrantPhase, StructuralRevisionGrant,
        UserBudgetReceipt, UserBudgetReceiptKind, WorkspaceSessionStatus,
    };
    use crate::product::work_item_plan_policy::{
        HumanGateSnapshot, HumanReason, ProviderStartLedgerEntry, WorkItemPlanFlowKind,
    };
    use crate::product::workspace_engine::contract_prerevision::{
        ProtectedBusinessFacts, StructuralRepairInstruction, StructuralRepairSolution,
    };
    use crate::web::workspace_ws_types::{
        StructuralGovernanceSnapshot, WsOutMessage, project_structural_governance_snapshot,
    };

    struct Sg07aStore {
        _tmp: tempfile::TempDir,
        paths: ProductAppPaths,
        lifecycle: LifecycleStore,
        birth: WorkspaceSessionRecord,
    }

    fn sg_07a_record_path(
        paths: &ProductAppPaths,
        record: &WorkspaceSessionRecord,
    ) -> std::path::PathBuf {
        paths
            .issue_lifecycle_root(&record.project_id, &record.issue_id)
            .join("workspace-sessions")
            .join(format!("{}.json", record.id))
    }

    fn sg_07a_store(project_id: &str) -> Sg07aStore {
        let tmp = tempfile::TempDir::new().unwrap();
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let lifecycle = LifecycleStore::new(paths.clone());
        let mut record = lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: project_id.to_string(),
                issue_id: "issue_sg07a_proj".to_string(),
                entity_id: "plan_sg07a_proj".to_string(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: None,
                review_rounds: 0,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: None,
            })
            .expect("durable session record");
        record.flow_kind = WorkItemPlanFlowKind::SingleCandidate;
        record.status = WorkspaceSessionStatus::WaitingForHuman;
        record.human_gate_snapshot = Some(HumanGateSnapshot {
            findings: Vec::new(),
            repeated_fingerprints: Vec::new(),
            attempts_used: 0,
            manual_repairs_remaining: 3,
            trigger: HumanReason::NativeHumanRequired,
            resumable: false,
            accepted_feedback_turns: Some(0),
            candidate_recovery: None,
        });
        record.work_item_plan_source_revision_ref = Some("sg07a-source-ref".to_string());
        record.plan_candidate_ir_ref = Some("sg07a-ir-ref".to_string());
        record.mechanical_report_ref = Some("sg07a-report-ref".to_string());
        write_json(&sg_07a_record_path(&paths, &record), &record).expect("persist birth");
        Sg07aStore {
            _tmp: tmp,
            paths,
            lifecycle,
            birth: record,
        }
    }

    fn sg_07a_turn(session_id: &str, turn_id: &str, command_id: &str) -> HumanGateTurn {
        HumanGateTurn {
            turn_id: turn_id.to_string(),
            session_id: session_id.to_string(),
            command_id: command_id.to_string(),
            feedback_text: "修复互依赖环".to_string(),
            status: HumanGateTurnStatus::Reserved,
            attempt_no: 1,
            budget_reserved: 1,
            source_hash: "a".repeat(64),
            result_artifact_ref: None,
            failure_class: None,
            created_at: "2026-10-14T00:00:00Z".to_string(),
            updated_at: "2026-10-14T00:00:00Z".to_string(),
        }
    }

    /// 真实 store CAS：接受反馈 → 物化聚合状态 + 活动 hold（SG4a 权威链）。
    fn sg_07a_reserve(
        store: &Sg07aStore,
        turn_id: &str,
        command_id: &str,
    ) -> WorkspaceSessionRecord {
        let turn = sg_07a_turn(&store.birth.id, turn_id, command_id);
        let reservation = HumanGateReservation {
            command_id: command_id.to_string(),
            turn_id: turn_id.to_string(),
            provider_start_idempotency_key: format!("sg07a:start:{turn_id}"),
            reserved_at: "2026-10-14T00:00:00Z".to_string(),
        };
        store
            .lifecycle
            .compare_and_reserve_human_gate_turn(&store.birth, turn, reservation)
            .expect("reserve human gate turn")
            .0
    }

    fn sg_07a_receipt(
        kind: UserBudgetReceiptKind,
        turn_id: &str,
        command_id: &str,
    ) -> UserBudgetReceipt {
        UserBudgetReceipt {
            kind,
            turn_id: turn_id.to_string(),
            command_id: command_id.to_string(),
            attempt_reason: Some(crate::product::models::StructuralAttemptReason::HumanRevision),
            source_revision_ref: Some("sg07a-source-ref".to_string()),
            plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
            structure_report_ref: Some("sg07a-report-ref".to_string()),
            context_digest: Some("sha256:sg07a-context".to_string()),
            reason: Some("sg07a five-state fixture".to_string()),
            settled_at: "2026-10-14T00:00:00Z".to_string(),
        }
    }

    /// SG5.3/SG6a.1 同款 read-modify-write 注入：保留既有 CAS 事实
    ///（hold/ledger/预算），只补无生产 producer 的耐久形状（producer 归
    /// SG6a.3）。
    fn sg_07a_mutate<F>(store: &Sg07aStore, mutate: F) -> WorkspaceSessionRecord
    where
        F: FnOnce(&mut WorkspaceSessionRecord),
    {
        let mut record = store
            .lifecycle
            .get_workspace_session(&store.birth.id)
            .expect("latest record");
        mutate(&mut record);
        write_json(&sg_07a_record_path(&store.paths, &record), &record)
            .expect("persist fixture mutation");
        record
    }

    fn sg_07a_instruction() -> StructuralRepairInstruction {
        StructuralRepairInstruction {
            repair_id: "repair_sg07a_0001".to_string(),
            failure_id: "failure_sg07a_0001".to_string(),
            rule_version: "structure_rules/v1".to_string(),
            source_hash: "sha256:sg07a-failing-source".to_string(),
            context_digest: "sha256:sg07a-context".to_string(),
            affected_work_item_ids: vec!["work_item_0001".to_string()],
            affected_spans: Vec::new(),
            conflict_edges: Vec::new(),
            allowed_change_closure: vec!["heading".to_string()],
            forbidden_operations: vec!["delete_required_inputs".to_string()],
            protected_facts: ProtectedBusinessFacts {
                business_request_digest: "d".repeat(64),
                confirmed_design_digest: None,
                protected_field_refs: Vec::new(),
                protected_contract_refs: Vec::new(),
                authorized_member_ids: Vec::new(),
            },
            solution: StructuralRepairSolution::Candidates {
                candidates: Vec::new(),
            },
            required_action: "修复非法 heading".to_string(),
            expected_predicates: Vec::new(),
            previous_valid_refs: None,
            instruction_digest: "e".repeat(64),
        }
    }

    fn sg_07a_snapshot_of(frame: &WsOutMessage) -> StructuralGovernanceSnapshot {
        match frame {
            WsOutMessage::SessionState {
                structural_governance: Some(snapshot),
                ..
            } => snapshot.clone(),
            WsOutMessage::SessionState {
                structural_governance: None,
                ..
            } => panic!("structural governance snapshot missing"),
            other => panic!("expected SessionState, got {other:?}"),
        }
    }

    /// 每态同值链：manager production `load_durable_workspace_projection`
    ///（最新 record 纯投影，出生 record 仅身份对照）＝ 引擎纯构造
    /// build_session_state ＝ live sync helper 吸收后的镜像 ＝
    /// reload_session_from_record 重建——逐字段相等＋快照事实与 durable
    /// 单源对账。
    fn sg_07a_assert_chain(
        store: &Sg07aStore,
        latest: &WorkspaceSessionRecord,
    ) -> StructuralGovernanceSnapshot {
        // ① manager production 纯投影。
        let identity = WorkspaceSessionIdentity::from_bound_record(&store.birth);
        let projection = load_durable_workspace_projection(&store.paths, &identity)
            .expect("pure projection of latest record");
        let projected = sg_07a_snapshot_of(&projection.session_state);

        // ② 引擎纯构造（ProjectionOnly）build_session_state。
        let engine = {
            let lifecycle = LifecycleStore::new(store.paths.clone());
            let (event_tx, _event_rx) = mpsc::channel(1);
            let checkpoint_store = Arc::new(CheckpointStore::new(
                store
                    .paths
                    .issue_lifecycle_root(&latest.project_id, &latest.issue_id),
            ));
            WorkspaceEngine::new_persistent_for_projection(
                checkpoint_store,
                lifecycle,
                event_tx,
                WorkspaceSession::from_record(latest.clone()),
            )
            .expect("projection engine")
        };
        let built = sg_07a_snapshot_of(&engine.build_session_state());
        assert_eq!(projected, built, "manager 纯投影与引擎构造必须同值");

        // ③ live CAS→sync helper：旧镜像吸收同 scope 最新 record。
        let mut synced = WorkspaceSession::from_record(store.birth.clone());
        synced
            .sync_structural_governance_from_record(latest)
            .expect("same-scope sync");
        assert_eq!(
            synced.structural_governance_state, latest.structural_governance_state,
            "sync 后镜像必须等于最新 record 聚合状态"
        );
        assert_eq!(synced.human_gate_snapshot, latest.human_gate_snapshot);
        assert_eq!(synced.provider_start_ledger, latest.provider_start_ledger);
        assert_eq!(
            synced.work_item_plan_source_revision_ref,
            latest.work_item_plan_source_revision_ref
        );
        assert_eq!(synced.plan_candidate_ir_ref, latest.plan_candidate_ir_ref);
        assert_eq!(synced.mechanical_report_ref, latest.mechanical_report_ref);

        // ④ reload_session_from_record 重建后 build 同值。
        let mut live_engine = {
            let lifecycle = LifecycleStore::new(store.paths.clone());
            let (event_tx, _event_rx) = mpsc::channel(1);
            let checkpoint_store = Arc::new(CheckpointStore::new(
                store
                    .paths
                    .issue_lifecycle_root(&store.birth.project_id, &store.birth.issue_id),
            ));
            WorkspaceEngine::new_persistent_for_projection(
                checkpoint_store,
                lifecycle,
                event_tx,
                WorkspaceSession::from_record(store.birth.clone()),
            )
            .expect("reload engine")
        };
        live_engine.reload_session_from_record(latest.clone());
        let reloaded = sg_07a_snapshot_of(&live_engine.build_session_state());
        assert_eq!(projected, reloaded, "reload 重建必须与纯投影同值");

        // ⑤ 快照事实与 durable 单源对账。
        let state = latest.structural_governance_state.as_ref().unwrap();
        assert_eq!(projected.user_receipts, state.user_receipts);
        assert_eq!(projected.user_holds, state.user_holds);
        assert_eq!(
            projected.available_user_budget,
            state.available_user_budget()
        );
        assert_eq!(
            projected.accepted_feedback_turns,
            latest
                .human_gate_snapshot
                .as_ref()
                .and_then(|gate| gate.accepted_feedback_turns)
        );
        assert_eq!(
            projected.actual_provider_starts,
            latest
                .provider_start_ledger
                .iter()
                .filter(|entry| entry.is_actual_launch_observation())
                .count() as u32
        );
        projected
    }

    fn sg_07a_durable_bytes(paths: &ProductAppPaths) -> Vec<(String, Vec<u8>)> {
        fn walk(dir: &std::path::Path, prefix: String, out: &mut Vec<(String, Vec<u8>)>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            let mut entries: Vec<_> = entries.flatten().collect();
            entries.sort_by_key(|entry| entry.file_name());
            for entry in entries {
                let path = entry.path();
                let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
                if path.is_dir() {
                    walk(&path, name, out);
                } else {
                    out.push((name, std::fs::read(&path).expect("read durable file")));
                }
            }
        }
        let mut snapshot = Vec::new();
        walk(&paths.projects_root(), String::new(), &mut snapshot);
        snapshot
    }

    #[test]
    fn sg_07a_live_cas_and_reload_match_latest_durable_projection() {
        let store = sg_07a_store("project_sg07a_live_cas");

        // 1) live CAS：reserve（SG4a 权威链）→ 最新 record 纯投影同值。
        let turn = sg_07a_turn(&store.birth.id, "turn_sg07a_live", "cmd_sg07a_live");
        let reservation = HumanGateReservation {
            command_id: "cmd_sg07a_live".to_string(),
            turn_id: "turn_sg07a_live".to_string(),
            provider_start_idempotency_key: "sg07a:start:live".to_string(),
            reserved_at: "2026-10-14T00:00:00Z".to_string(),
        };
        let reserved = store
            .lifecycle
            .compare_and_reserve_human_gate_turn(&store.birth, turn, reservation)
            .expect("reserve human gate turn")
            .0;
        sg_07a_assert_chain(&store, &reserved);

        // 2) live CAS：注入 grant（producer 归 SG6a.3，fixture 注入沿
        //    SG5.3/SG6a.1 同款）后 claim_structural_attempt 真实推进
        //    attempt/phase/prompt 绑定。
        let mut granted = store
            .lifecycle
            .get_workspace_session(&store.birth.id)
            .expect("latest before grant injection");
        granted
            .structural_governance_state
            .get_or_insert_with(StructuralGovernanceState::default)
            .current_grant = Some(StructuralRevisionGrant {
            grant_id: "structural_grant_turn_sg07a_live".to_string(),
            authorization_hash: "sha256:sg07a-authorization".to_string(),
            turn_id: Some("turn_sg07a_live".to_string()),
            command_id: Some("cmd_sg07a_live".to_string()),
            candidate_source_revision_ref: Some("sg07a-source-ref".to_string()),
            plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
            mechanical_report_ref: Some("sg07a-report-ref".to_string()),
            phase: Some(StructuralGrantPhase::AuthorRunning),
            attempt_no: 1,
            prompt_ref: None,
            prompt_digest: None,
            instruction_digest: None,
            failing_source_hash: None,
            review_identity: None,
            granted_at: "2026-10-14T00:00:00Z".to_string(),
        });
        write_json(&sg_07a_record_path(&store.paths, &granted), &granted)
            .expect("persist grant injection");
        let mut running_turn = sg_07a_turn(&store.birth.id, "turn_sg07a_live", "cmd_sg07a_live");
        running_turn.status = HumanGateTurnStatus::Running;
        let granted = store
            .lifecycle
            .update_human_gate_turn(&granted, running_turn)
            .expect("mark turn running");
        sg_07a_assert_chain(&store, &granted);

        let claim = store
            .lifecycle
            .claim_structural_attempt(
                &granted,
                "turn_sg07a_live",
                &sg_07a_instruction(),
                "sg07a-frozen-prompt-ref",
                "sha256:sg07a-prompt",
            )
            .expect("claim structural attempt");
        assert!(
            matches!(
                claim,
                crate::product::models::StructuralClaimOutcome::Claimed { attempt_no: 2, .. }
            ),
            "claim 必须推进到 attempt2：{claim:?}"
        );
        let claimed = store
            .lifecycle
            .get_workspace_session(&store.birth.id)
            .expect("latest after claim");
        sg_07a_assert_chain(&store, &claimed);

        // 态②：注入 grant phase Reviewing（turn 仍 Running）→ settle
        // delivered_charge（真实 CAS）——reload（from_record 镜像）、sync
        // 与 manager 纯投影逐字段同值；纯投影前后 durable 字节不变。
        let reviewing = sg_07a_mutate(&store, |record| {
            let state = record
                .structural_governance_state
                .as_mut()
                .expect("state after claim");
            state
                .current_grant
                .as_mut()
                .expect("grant after claim")
                .phase = Some(StructuralGrantPhase::Reviewing);
        });
        // P3 修复（审查）：零写断言收缩到纯读段——基线在 settle 写完成
        // 后、assert_chain（manager 纯投影+引擎构造+sync+reload 全链）
        // 前，窗口比较完整 (name, bytes) 向量相等。
        let settled = store
            .lifecycle
            .settle_structural_user_budget(
                &reviewing,
                "turn_sg07a_live",
                UserBudgetReceipt {
                    kind: UserBudgetReceiptKind::DeliveredCharge,
                    turn_id: "turn_sg07a_live".to_string(),
                    command_id: "cmd_sg07a_live".to_string(),
                    attempt_reason: Some(
                        crate::product::models::StructuralAttemptReason::HumanRevision,
                    ),
                    source_revision_ref: Some("sg07a-source-ref".to_string()),
                    plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
                    structure_report_ref: Some("sg07a-report-ref".to_string()),
                    context_digest: Some("sha256:sg07a-context".to_string()),
                    reason: Some("sg07a live-cas fixture".to_string()),
                    settled_at: "2026-10-14T00:00:00Z".to_string(),
                },
            )
            .expect("settle delivered charge");
        let bytes_baseline = sg_07a_durable_bytes(&store.paths);
        let delivered_snapshot = sg_07a_assert_chain(&store, &settled);
        assert_eq!(
            delivered_snapshot.available_user_budget, 2,
            "3 − 1 delivered"
        );
        assert_eq!(delivered_snapshot.active_hold_count, 0);
        assert!(
            matches!(
                delivered_snapshot
                    .current_grant
                    .as_ref()
                    .map(|grant| grant.phase),
                Some(Some(StructuralGrantPhase::Reviewing))
            ),
            "grant phase 沿注入的 Reviewing 进度"
        );
        let bytes_after = sg_07a_durable_bytes(&store.paths);
        assert_eq!(
            bytes_baseline, bytes_after,
            "纯读窗口（assert_chain 全链）durable 字节必须零写"
        );

        // 态③：结构耗尽 released_structural——预算全额恢复（不实扣）。
        let store3 = sg_07a_store("project_sg07a_release");
        let reserved3 = sg_07a_reserve(&store3, "turn_sg07a_rel", "cmd_sg07a_rel");
        let released = store3
            .lifecycle
            .settle_structural_user_budget(
                &reserved3,
                "turn_sg07a_rel",
                sg_07a_receipt(
                    UserBudgetReceiptKind::ReleasedStructural,
                    "turn_sg07a_rel",
                    "cmd_sg07a_rel",
                ),
            )
            .expect("settle released structural");
        let released_snapshot = sg_07a_assert_chain(&store3, &released);
        assert_eq!(
            released_snapshot.available_user_budget, 3,
            "released 不实扣"
        );
        assert_eq!(released_snapshot.active_hold_count, 0);
        assert_eq!(
            released_snapshot.user_receipts.len(),
            1,
            "released receipt 历史保留"
        );

        // 态④：provider/取消非结构失败扣次——available 2、delivered 0。
        let store4 = sg_07a_store("project_sg07a_failure");
        let reserved4 = sg_07a_reserve(&store4, "turn_sg07a_fail", "cmd_sg07a_fail");
        let failed = store4
            .lifecycle
            .settle_structural_user_budget(
                &reserved4,
                "turn_sg07a_fail",
                sg_07a_receipt(
                    UserBudgetReceiptKind::NonstructuralFailureCharge,
                    "turn_sg07a_fail",
                    "cmd_sg07a_fail",
                ),
            )
            .expect("settle nonstructural failure charge");
        let failed_snapshot = sg_07a_assert_chain(&store4, &failed);
        assert_eq!(
            failed_snapshot.available_user_budget, 2,
            "3 − 1 failure charge"
        );
        assert_eq!(failed_snapshot.active_hold_count, 0);
        assert!(
            failed_snapshot
                .user_receipts
                .iter()
                .all(|receipt| receipt.kind == UserBudgetReceiptKind::NonstructuralFailureCharge)
        );

        // 态⑤：grant 终态（Resolved）＋无活动授权、同 scope checkpoint
        // 历史/已解决事实仍在、actual start 证据在 ledger。
        let store5 = sg_07a_store("project_sg07a_terminal");
        let reserved5 = sg_07a_reserve(&store5, "turn_sg07a_term", "cmd_sg07a_term");
        store5
            .lifecycle
            .settle_structural_user_budget(
                &reserved5,
                "turn_sg07a_term",
                sg_07a_receipt(
                    UserBudgetReceiptKind::ReleasedStructural,
                    "turn_sg07a_term",
                    "cmd_sg07a_term",
                ),
            )
            .expect("settle before terminal");
        let terminal = sg_07a_mutate(&store5, |record| {
            let state = record
                .structural_governance_state
                .get_or_insert_with(StructuralGovernanceState::default);
            state.current_grant = Some(StructuralRevisionGrant {
                grant_id: "structural_grant_turn_sg07a_term".to_string(),
                authorization_hash: "sha256:sg07a-authorization".to_string(),
                turn_id: Some("turn_sg07a_term".to_string()),
                command_id: Some("cmd_sg07a_term".to_string()),
                candidate_source_revision_ref: Some("sg07a-source-ref".to_string()),
                plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
                mechanical_report_ref: Some("sg07a-report-ref".to_string()),
                phase: Some(StructuralGrantPhase::Resolved),
                attempt_no: 2,
                prompt_ref: Some("sg07a-frozen-prompt-ref".to_string()),
                prompt_digest: Some("sha256:sg07a-prompt".to_string()),
                instruction_digest: Some("sha256:sg07a-instruction".to_string()),
                failing_source_hash: Some("sha256:sg07a-failing-source".to_string()),
                review_identity: None,
                granted_at: "2026-10-14T00:00:00Z".to_string(),
            });
            state.structure_checkpoints = vec![crate::product::models::StructuralCheckpoint {
                checkpoint_id: "sg07a-terminal-checkpoint".to_string(),
                source_revision_ref: Some("sg07a-source-ref".to_string()),
                plan_candidate_ir_ref: Some("sg07a-ir-ref".to_string()),
                structure_report_ref: Some("sg07a-report-ref".to_string()),
                artifact_version: Some(2),
                source_hash: Some("sha256:sg07a-failing-source".to_string()),
                context_digest: Some("sha256:sg07a-context".to_string()),
                authorization_hash: Some("sha256:sg07a-authorization".to_string()),
                receipt_ref: None,
                review_scope: Some("sg07a-review-scope".to_string()),
                status: crate::product::models::StructuralCheckpointStatus::Superseded,
                recorded_at: "2026-10-14T00:00:00Z".to_string(),
            }];
            state.resolved_structure_facts = vec![crate::product::models::ResolvedStructureFact {
                predicate: "dependency_cycle_resolved".to_string(),
                evidence_refs: vec!["sg07a-report-ref".to_string()],
                source_hash: None,
                resolved_at: "2026-10-14T00:00:00Z".to_string(),
            }];
            state.current_structure_report_ref = Some("sg07a-report-ref".to_string());
            state.current_context_digest = Some("sha256:sg07a-context".to_string());
            record.provider_start_ledger.push(ProviderStartLedgerEntry {
                provider_start_idempotency_key: "sg07a:start:terminal-actual".to_string(),
                started: true,
                provider: Some("fake".to_string()),
                started_at: Some("2026-10-14T00:00:00Z".to_string()),
            });
        });
        let terminal_snapshot = sg_07a_assert_chain(&store5, &terminal);
        let terminal_grant = terminal_snapshot
            .current_grant
            .expect("terminal grant kept");
        assert_eq!(terminal_grant.phase, Some(StructuralGrantPhase::Resolved));
        assert_eq!(terminal_grant.attempt_no, 2);
        assert_eq!(
            terminal_snapshot.structure_checkpoints.len(),
            1,
            "checkpoint 历史仍在"
        );
        assert_eq!(terminal_snapshot.resolved_structure_facts.len(), 1);
        assert_eq!(terminal_snapshot.actual_provider_starts, 1);
        assert_eq!(terminal_snapshot.available_user_budget, 3);

        // 4) 负例：CAS 失败（stale expected）不推进镜像——runtime 镜像
        //    保持上次成功 CAS 事实。
        let mut runtime = WorkspaceSession::from_record(reserved.clone());
        runtime
            .sync_structural_governance_from_record(&claimed)
            .expect("sync to claimed state");
        assert_eq!(
            runtime.structural_governance_state, claimed.structural_governance_state,
            "同步后镜像必须等于最新成功 CAS 事实"
        );
        let stale_settle = store.lifecycle.settle_structural_user_budget(
            &claimed,
            "turn_sg07a_live",
            UserBudgetReceipt {
                kind: UserBudgetReceiptKind::ReleasedStructural,
                turn_id: "turn_sg07a_live".to_string(),
                command_id: "cmd_sg07a_live".to_string(),
                attempt_reason: None,
                source_revision_ref: None,
                plan_candidate_ir_ref: None,
                structure_report_ref: None,
                context_digest: None,
                reason: None,
                settled_at: "2026-10-14T00:00:00Z".to_string(),
            },
        );
        assert!(
            matches!(stale_settle, Err(ProductStoreError::Conflict { .. })),
            "stale expected 的 CAS 必须失败：{stale_settle:?}"
        );
        assert_eq!(
            runtime.structural_governance_state, claimed.structural_governance_state,
            "CAS 失败后镜像保持不变"
        );

        // 5) 负例：错 scope 同步返回 IdentityMismatch 且镜像保持。
        let mut wrong_scope_record = settled.clone();
        wrong_scope_record.entity_id = "entity_other_scope".to_string();
        let sync_result = runtime.sync_structural_governance_from_record(&wrong_scope_record);
        assert!(
            matches!(sync_result, Err(ProductStoreError::IdentityMismatch { .. })),
            "错 scope 同步必须失败：{sync_result:?}"
        );
        assert_eq!(
            runtime.structural_governance_state, claimed.structural_governance_state,
            "错 scope 拒绝后镜像保持不变"
        );

        // 6) 纯函数守卫：投影不重算 accepted/actual（单源透传）。
        let state = settled.structural_governance_state.as_ref().unwrap();
        let projected = project_structural_governance_snapshot(
            state,
            settled.human_gate_snapshot.as_ref(),
            &settled.provider_start_ledger,
            settled.work_item_plan_source_revision_ref.as_deref(),
            settled.plan_candidate_ir_ref.as_deref(),
            settled.mechanical_report_ref.as_deref(),
        );
        assert_eq!(
            projected.accepted_feedback_turns,
            settled
                .human_gate_snapshot
                .as_ref()
                .and_then(|gate| gate.accepted_feedback_turns)
        );
        assert_eq!(
            projected.actual_provider_starts,
            settled
                .provider_start_ledger
                .iter()
                .filter(|entry| entry.is_actual_launch_observation())
                .count() as u32
        );
    }
}
