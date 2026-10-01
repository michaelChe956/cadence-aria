impl WorkspaceEngine {
    #[allow(clippy::too_many_arguments)]
    async fn initialize_advance_inner(
        &mut self,
        input: AdvanceInput,
        advance_store: AdvanceStore,
        coding_store: CodingAttemptStore,
        authoritative: AuthoritativeGroupPlanBinding,
        start_policy: crate::product::coding_models::CodingStartRunPolicy,
    ) -> Result<AdvanceOutcome, String> {
        let _initialization_guard = coding_store
            .acquire_group_initialization_arbitration(&input.project_id, &input.issue_id)
            .map_err(|error| format!("acquire advance initialization lock failed: {error}"))?;
        let record = advance_store
            .persist_advance_record_if_absent(&input, &authoritative.plan_revision_id)
            .map_err(|error| format!("persist advance record failed: {error}"))?;
        maybe_fail_advance_initialization(&input, AdvanceInitializationFailpoint::RecordPersisted)?;
        if record.plan_revision_id != authoritative.plan_revision_id {
            return Err(
                "advance record plan revision differs from authoritative plan revision".to_string(),
            );
        }

        // REQ-MTG-01/02（D1 显式两分支）：authoritative units 分出 ≥2 个 target 桶
        // 时进分流循环（per-target CreateGroupCodingAttemptInput）；单 target（含
        // 0-target focus 唯一）走下方现行路径零变化。
        let grouped = units_by_target(&authoritative);
        if grouped.by_target.len() >= 2 {
            return self
                .initialize_advance_split(
                    input,
                    advance_store,
                    coding_store,
                    authoritative,
                    grouped,
                    record,
                )
                .await;
        }

        let repository =
            Self::resolve_advance_repository(&advance_store.app_paths(), &input, &authoritative)?;
        let current_unit = authoritative
            .units
            .first()
            .ok_or_else(|| "authoritative group has no first unit".to_string())?;
        let existing_group_journal = match coding_store.get_group_initialization(
            &input.project_id,
            &input.issue_id,
            &input.plan_id,
        ) {
            Ok(journal) => {
                if journal.attempt.admission_kind != CodingAdmissionKind::ScAdvance
                    || journal.plan_binding.bound_plan_revision_id != authoritative.plan_revision_id
                {
                    return Err(
                        "existing group initialization is bound to another advance identity"
                            .to_string(),
                    );
                }
                Some(journal)
            }
            Err(crate::product::json_store::ProductStoreError::NotFound { .. }) => None,
            Err(error) => {
                return Err(format!(
                    "load existing group initialization failed: {error}"
                ));
            }
        };
        let provider_config = existing_group_journal
            .as_ref()
            .map(|journal| ProviderConfigSnapshot {
                author: journal.attempt.provider_config_snapshot.author.clone(),
                reviewer: journal.attempt.provider_config_snapshot.reviewer.clone(),
                review_rounds: journal.attempt.provider_config_snapshot.review_rounds,
                permission_modes: journal
                    .attempt
                    .provider_config_snapshot
                    .permission_modes
                    .clone(),
            })
            .unwrap_or_else(|| Self::advance_provider_config(&self.session, current_unit));
        let branch_name = existing_group_journal
            .as_ref()
            .map(|journal| journal.attempt.branch_name.clone())
            .unwrap_or_else(|| format!("aria/issues/{}", input.issue_id));
        let base_branch = match existing_group_journal.as_ref() {
            Some(journal) => journal.attempt.base_branch.clone(),
            // REQ-PIB-03（T3.1）：新 attempt 的 fork 基线读 issue.base_branch 经
            // 三面同源解析链；不可解析 fail-closed，不回退当前检出或 HEAD。
            None => Self::resolve_advance_base_branch(
                &advance_store.app_paths(),
                &repository.path,
                &input,
            )?,
        };
        let worktree_path = existing_group_journal
            .as_ref()
            .and_then(|journal| journal.attempt.worktree_path.clone())
            .unwrap_or_else(|| {
                repository
                    .path
                    .join(".worktrees")
                    .join("aria-issues")
                    .join(&input.issue_id)
            });
        let target_snapshot = existing_group_journal
            .as_ref()
            .and_then(|journal| journal.attempt.target_snapshot.clone())
            .or(Self::advance_target_snapshot(
                &advance_store.app_paths(),
                &input,
                &authoritative,
            )?);
        let group_input = CreateGroupCodingAttemptInput {
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            current_work_item_id: current_unit.logical_work_item_id.clone(),
            base_branch,
            branch_name,
            worktree_path: Some(worktree_path.clone()),
            provider_config_snapshot: provider_config,
            target_snapshot,
            max_auto_rework: 2,
            // P2 Task 2：首启策略随 advance 穿透（Manual=旧 handle_advance
            // 语义；AutoStartOnce 仅 Task 1 自动路径经
            // handle_advance_with_start_policy 传入）。
            start_run_policy: start_policy,
        };
        let mut group_journal = match existing_group_journal {
            Some(journal) => journal,
            None => coding_store
                .prepare_group_initialization_with_admission(
                    &group_input,
                    &authoritative.plan_revision_id,
                    &authoritative.units,
                    CodingAdmissionKind::ScAdvance,
                )
                .map_err(|error| format!("prepare group initialization failed: {error}"))?,
        };
        if group_journal.attempt.admission_kind != CodingAdmissionKind::ScAdvance {
            return Err("advance initialization journal is not an SC admission".to_string());
        }
        let mut outer = advance_store
            .load_or_prepare_advance_initialization(&record, &group_journal)
            .map_err(|error| format!("persist advance initialization failed: {error}"))?;
        maybe_fail_advance_initialization(&input, AdvanceInitializationFailpoint::JournalPrepared)?;
        if outer.phase.order_for_engine()
            >= AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
            && record.attempt_id.as_deref() != Some(outer.attempt_id.as_str())
        {
            return Err(
                "advance record attempt identity differs from initialization journal".to_string(),
            );
        }
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
        ) {
            let record_attempt = coding_store
                .ensure_group_initialization_attempt(
                    &group_journal,
                    &coding_store
                        .acquire_work_item_attempt_creation(
                            &input.project_id,
                            &input.issue_id,
                            &current_unit.logical_work_item_id,
                        )
                        .map_err(|error| {
                            format!("acquire attempt creation lock failed: {error}")
                        })?,
                )
                .map_err(|error| format!("persist group attempt failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::GroupAttemptPersisted,
            )?;
            group_journal = coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
                )
                .map_err(|error| format!("checkpoint group attempt persistence failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::AttemptPersisted,
            )?;
            if group_journal.attempt.id != record_attempt.id {
                return Err(
                    "group initialization attempt identity changed during replay".to_string(),
                );
            }
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
        {
            let mut updated = record.clone();
            updated.attempt_id = Some(group_journal.attempt.id.clone());
            updated.updated_at = chrono::Utc::now().to_rfc3339();
            advance_store
                .update_record(&updated)
                .map_err(|error| format!("bind attempt to advance record failed: {error}"))?;
            outer = advance_store
                .advance_initialization_phase(
                    &updated,
                    &outer,
                    AdvanceInitializationPhase::AttemptPersisted,
                )
                .map_err(|error| format!("checkpoint attempt persistence failed: {error}"))?;
        }
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
        ) {
            let lifecycle = self
                .lifecycle_store
                .as_ref()
                .ok_or("lifecycle store unavailable")?;
            // 单 target 逻辑仓（attempt 带 target_snapshot）：worktree 事实落
            // repo 维三元键 `shared-worktrees/{repository_id}.json`（与
            // initialize_advance_split 同构）。不得写 legacy
            // `issue-shared-worktree.json`——coding 引擎 §4.2.6 preflight 对带
            // target_snapshot 的 attempt 见 legacy 文件即 fail-closed
            // （legacy_shared_worktree_present）。纯物理单 target（无
            // snapshot）保持原 legacy 路径零变化。
            if let Some(snapshot) = group_journal.attempt.target_snapshot.as_ref() {
                lifecycle
                    .upsert_repo_shared_worktree(
                        crate::product::lifecycle_store::UpsertRepoSharedWorktreeInput {
                            project_id: input.project_id.clone(),
                            issue_id: input.issue_id.clone(),
                            repository_id: snapshot.logical_repository_id,
                            branch_name: group_journal.attempt.branch_name.clone(),
                            worktree_path: worktree_path.clone(),
                            base_branch: group_journal.attempt.base_branch.clone(),
                        },
                    )
                    .map_err(|error| format!("persist shared worktree failed: {error}"))?;
                // repo 维 lease 与 issue 维 worktree_lease_id 解耦：确定性派生自
                // journal id（重放同值幂等）；bind 语义要求
                // `repo_worktree_lease_` 前缀或 owner 已是 attempt id。
                let lease_id = format!("repo_worktree_lease_{}", group_journal.id);
                let lease = lifecycle
                    .try_acquire_repo_worktree_lock(
                        &input.project_id,
                        &input.issue_id,
                        snapshot.logical_repository_id,
                        &group_journal.lock_work_item_id,
                        &lease_id,
                    )
                    .map_err(|error| format!("acquire shared worktree failed: {error}"))?;
                if !lease.acquired
                    && lease.worktree.current_lock_owner_id.as_deref()
                        != Some(group_journal.attempt.id.as_str())
                {
                    return Err("shared worktree is owned by another attempt".to_string());
                }
                lifecycle
                    .bind_repo_worktree_lock_to_attempt(
                        &input.project_id,
                        &input.issue_id,
                        snapshot.logical_repository_id,
                        &group_journal.lock_work_item_id,
                        &group_journal.attempt.id,
                    )
                    .map_err(|error| format!("bind shared worktree failed: {error}"))?;
            } else {
                lifecycle
                    .upsert_issue_shared_worktree(
                        crate::product::lifecycle_store::UpsertIssueSharedWorktreeInput {
                            project_id: input.project_id.clone(),
                            issue_id: input.issue_id.clone(),
                            repository_id: repository.id.clone(),
                            branch_name: group_journal.attempt.branch_name.clone(),
                            worktree_path: worktree_path.clone(),
                            base_branch: group_journal.attempt.base_branch.clone(),
                        },
                    )
                    .map_err(|error| format!("persist shared worktree failed: {error}"))?;
                let lease = lifecycle
                    .try_acquire_issue_worktree_lock(
                        &input.project_id,
                        &input.issue_id,
                        &group_journal.lock_work_item_id,
                        &group_journal.worktree_lease_id,
                    )
                    .map_err(|error| format!("acquire shared worktree failed: {error}"))?;
                if !lease.acquired
                    && lease.worktree.current_lock_owner_id.as_deref()
                        != Some(group_journal.attempt.id.as_str())
                {
                    return Err("shared worktree is owned by another attempt".to_string());
                }
                lifecycle
                    .bind_issue_worktree_lock_to_attempt(
                        &input.project_id,
                        &input.issue_id,
                        &group_journal.lock_work_item_id,
                        &group_journal.attempt.id,
                    )
                    .map_err(|error| format!("bind shared worktree failed: {error}"))?;
            }
            group_journal = coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
                )
                .map_err(|error| format!("checkpoint group worktree binding failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::WorktreeBound,
            )?;
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::WorktreeBound.order_for_engine()
        {
            let current_record = advance_store
                .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
                .map_err(|error| format!("reload advance record failed: {error}"))?
                .ok_or("advance record disappeared")?;
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::WorktreeBound,
                )
                .map_err(|error| format!("checkpoint worktree binding failed: {error}"))?;
        }
        let current_record = advance_store
            .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
            .map_err(|error| format!("reload advance record failed: {error}"))?
            .ok_or("advance record disappeared")?;
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
        ) {
            coding_store
                .ensure_group_initialization_plan_binding(&group_journal)
                .map_err(|error| format!("persist group plan binding failed: {error}"))?;
            group_journal = coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
                )
                .map_err(|error| format!("checkpoint group plan binding failed: {error}"))?;
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::PlanBindingSaved.order_for_engine()
        {
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::PlanBindingSaved,
                )
                .map_err(|error| format!("checkpoint plan binding failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::PlanBindingSaved,
            )?;
        }
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
        ) {
            for index in 0..group_journal.units.len() {
                coding_store
                    .ensure_group_initialization_unit(&group_journal, index)
                    .map_err(|error| format!("persist group unit failed: {error}"))?;
            }
            group_journal = coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
                )
                .map_err(|error| format!("checkpoint group units materialization failed: {error}"))?;
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::UnitsMaterialized.order_for_engine()
        {
            outer = advance_store
                .advance_initialization_phase(
                    &current_record,
                    &outer,
                    AdvanceInitializationPhase::UnitsMaterialized,
                )
                .map_err(|error| format!("checkpoint units materialization failed: {error}"))?;
            maybe_fail_advance_initialization(
                &input,
                AdvanceInitializationFailpoint::UnitsMaterialized,
            )?;
        }
        let persisted_attempt = coding_store
            .get_attempt(
                &input.project_id,
                &input.issue_id,
                &group_journal.attempt.id,
            )
            .map_err(|error| format!("load initialized attempt failed: {error}"))?;
        coding_store
            .validate_group_attempt_integrity(&persisted_attempt)
            .map_err(|error| format!("validate initialized group failed: {error}"))?;
        let final_record = advance_store
            .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
            .map_err(|error| format!("reload final advance record failed: {error}"))?
            .ok_or("advance record disappeared")?;
        if !group_journal.phase.has_reached(
            crate::product::coding_attempt_store::CodingGroupInitializationPhase::Completed,
        ) {
            coding_store
                .advance_group_initialization_phase(
                    &group_journal,
                    crate::product::coding_attempt_store::CodingGroupInitializationPhase::Completed,
                )
                .map_err(|error| {
                    format!("checkpoint group initialization completion failed: {error}")
                })?;
        }
        advance_store
            .advance_initialization_phase(&final_record, &outer, AdvanceInitializationPhase::Ready)
            .map_err(|error| format!("checkpoint ready initialization failed: {error}"))?;
        let mut ready_record = final_record;
        ready_record.status = AdvanceStatus::Ready;
        ready_record.attempt_id = Some(persisted_attempt.id.clone());
        ready_record.workspace_entry = Some(Self::advance_workspace_entry(&persisted_attempt));
        ready_record.updated_at = chrono::Utc::now().to_rfc3339();
        advance_store
            .update_record(&ready_record)
            .map_err(|error| format!("persist ready advance record failed: {error}"))?;
        Ok(AdvanceOutcome::Completed {
            record: ready_record,
            attempt_id: persisted_attempt.id.clone(),
            workspace_entry: Self::advance_workspace_entry(&persisted_attempt),
            // 单 target 路径：集绑定为空（wire 不发送），单值 attempt_id 承载不变。
            target_attempts: Vec::new(),
        })
    }
}
