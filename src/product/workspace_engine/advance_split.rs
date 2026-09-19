//! advance 分流初始化（REQ-MTG-01/02，k3 F3 必改点一）——`initialize_advance_split`
//! per-target 分流循环+worktree 嵌套共存检查+增殖审计写入（REQ-MTG-05）。
//! 自 `advance.rs` 拆出（1200 行守护，WP6 关闸）：纯移动零语义变化；单 target
//! 路径仍全程在 `advance.rs`（D1 显式两分支零变化）。

use super::advance::{AdvanceInitializationFailpoint, maybe_fail_advance_initialization};
use super::types::WorkspaceEngine;
use crate::product::advance_store::{
    AdvanceInitializationPhase, AdvanceInput, AdvanceOutcome, AdvanceRecord, AdvanceStatus,
    AdvanceStore, AdvanceTargetAttemptBinding,
};
use crate::product::coding_attempt_store::target_snapshot::build_attempt_target_snapshot;
use crate::product::coding_attempt_store::{
    AuthoritativeGroupPlanBinding, CodingAttemptStore, CreateGroupCodingAttemptInput,
    SplitAuditRecord, SplitAuditTrigger, SplitAuditTriggerKind, UnitsByTarget,
    topologically_order_unit_bindings,
};
use crate::product::coding_models::CodingAdmissionKind;
use crate::product::logical_codebase::{RepositoryRouting, resolve_issue_logical_codebase_id};
use crate::product::repository_store::RepositoryStore;
use crate::web::workspace_ws_types::ProviderConfigSnapshot;

impl WorkspaceEngine {
    /// REQ-MTG-01/02（WP2 分流创建，k3 F3 必改点一）：多 target（`units_by_target`
    /// 桶数 ≥2）显式分流循环。单 target（含 0-target focus 唯一）不进入本函数
    /// ——`initialize_advance_inner` 现行路径零变化（D1 显式两分支）。
    ///
    /// 每 target：独立 `CreateGroupCodingAttemptInput`（OQ2 命名
    /// branch=`aria/issues/{issue_id}/{logical_id}`、
    /// worktree=`.worktrees/aria-issues/{issue_id}/{logical_id}`、base_branch=各仓
    /// 当前分支、per-target 冻结快照 `build_attempt_target_snapshot`）→
    /// `prepare_group_initialization_with_admission_for_target(Some(t))` per-target
    /// journal 子路径 → `ensure_group_initialization_attempt` → repo 维三元键
    /// worktree 三件套（含 T2S3 嵌套共存检查）→ units 分组物化。外层 advance
    /// journal 用集绑定形态（`load_or_prepare_advance_initialization_for_attempts`：
    /// `attempt_id`=全局拓扑序首个+`target_attempt_ids` 全集，集合一致性比对）。
    /// 本函数只建组——无自动跨 attempt 编排（REQ-MTG-03，StartCoding 唯一入口）。
    pub(super) async fn initialize_advance_split(
        &mut self,
        input: AdvanceInput,
        advance_store: AdvanceStore,
        coding_store: CodingAttemptStore,
        authoritative: AuthoritativeGroupPlanBinding,
        grouped: UnitsByTarget,
        record: AdvanceRecord,
    ) -> Result<AdvanceOutcome, String> {
        // D2.1 对偶（A2 不静默归属）：≥2 target 桶下无归属 unit 不入任何桶、也
        // 不回退 focus——显式 fail-closed。
        if !grouped.unattributed.is_empty() {
            return Err(
                "advance split requires every authoritative unit to carry a target repository"
                    .to_string(),
            );
        }
        let paths = advance_store.app_paths();
        let lc_id = resolve_issue_logical_codebase_id(&paths, &input.project_id, &input.issue_id)
            .map_err(|error| format!("resolve advance target codebase failed: {error}"))?;
        let RepositoryRouting::Logical { manifest, .. } =
            RepositoryRouting::load_for_issue(&paths, &input.project_id, &input.issue_id)
                .map_err(|error| format!("load advance repository routing failed: {error}"))?
        else {
            // Legacy 路由无逻辑仓可分流（快照/仓解析均按 logical id 寻址）。
            return Err("advance split requires logical repository routing".to_string());
        };

        // 既有 per-target journal（幂等重放的权威输入）：按 attempt 冻结快照的
        // target 归桶；准入身份与 plan revision 绑定校验与单 target 路径同语义。
        let mut existing_by_target = std::collections::BTreeMap::new();
        for journal in coding_store
            .list_group_initialization_journals_for_plan(
                &input.project_id,
                &input.issue_id,
                &input.plan_id,
            )
            .map_err(|error| format!("load existing split journals failed: {error}"))?
        {
            if journal.attempt.admission_kind != CodingAdmissionKind::ScAdvance
                || journal.plan_binding.bound_plan_revision_id != authoritative.plan_revision_id
            {
                return Err(
                    "existing group initialization is bound to another advance identity"
                        .to_string(),
                );
            }
            let target = journal
                .attempt
                .target_snapshot
                .as_ref()
                .map(|snapshot| snapshot.logical_repository_id)
                .ok_or_else(|| {
                    "existing split group initialization has no target snapshot".to_string()
                })?;
            existing_by_target.insert(target, journal);
        }
        if existing_by_target
            .keys()
            .any(|target| !grouped.by_target.contains_key(target))
        {
            return Err(
                "existing split group initialization covers a target absent from the plan"
                    .to_string(),
            );
        }

        // 全局拓扑序：每 target 以其 units 的最小全局序定位——record.attempt_id 与
        // `target_attempts` 顺序由全局拓扑序首个 unit 所属 target 承载。
        let global_order = topologically_order_unit_bindings(&authoritative.units)
            .map_err(|error| format!("order split units failed: {error}"))?;
        let global_position: std::collections::BTreeMap<&str, usize> = global_order
            .iter()
            .enumerate()
            .map(|(index, unit)| (unit.logical_work_item_id.as_str(), index))
            .collect();

        struct SplitTarget {
            target: crate::product::logical_codebase::LogicalRepositoryId,
            journal: crate::product::coding_attempt_store::CodingGroupInitializationJournal,
            order_index: usize,
        }
        let mut targets: Vec<SplitTarget> = Vec::with_capacity(grouped.by_target.len());
        for (target, units) in grouped.by_target {
            let ordered_units = topologically_order_unit_bindings(&units)
                .map_err(|error| format!("order split units failed: {error}"))?;
            if !manifest.member_ids.contains(&target) {
                return Err("advance logical repository target is not selected".to_string());
            }
            let repository = RepositoryStore::new(paths.clone())
                .resolve_logical_repository_for_issue_codebase(
                    &input.project_id,
                    lc_id.as_deref(),
                    target,
                )
                .map(|(_, _, repository)| repository)
                .map_err(|error| format!("resolve advance logical repository failed: {error}"))?;
            let existing = existing_by_target.get(&target);
            let provider_config = existing
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
                .unwrap_or_else(|| Self::advance_provider_config(&self.session, &ordered_units[0]));
            // OQ2 命名：branch/worktree 以 target UUID 限定；重放以 journal 冻结值
            // 为权威（快照 captured_at 不可重捕获）。
            let branch_name = existing
                .map(|journal| journal.attempt.branch_name.clone())
                .unwrap_or_else(|| format!("aria/issues/{}/{}", input.issue_id, target.0));
            let base_branch = existing
                .map(|journal| journal.attempt.base_branch.clone())
                .unwrap_or_else(|| {
                    Self::current_git_branch(&repository.path).unwrap_or_else(|| "HEAD".to_string())
                });
            let worktree_path = existing
                .and_then(|journal| journal.attempt.worktree_path.clone())
                .unwrap_or_else(|| {
                    repository
                        .path
                        .join(".worktrees")
                        .join("aria-issues")
                        .join(&input.issue_id)
                        .join(target.0.to_string())
                });
            let target_snapshot = existing
                .and_then(|journal| journal.attempt.target_snapshot.clone())
                .or(Some(
                    build_attempt_target_snapshot(
                        &paths,
                        &input.project_id,
                        target,
                        lc_id.as_deref(),
                    )
                    .map_err(|error| format!("capture advance target snapshot failed: {error}"))?,
                ));
            let group_input = CreateGroupCodingAttemptInput {
                project_id: input.project_id.clone(),
                issue_id: input.issue_id.clone(),
                plan_id: input.plan_id.clone(),
                current_work_item_id: ordered_units[0].logical_work_item_id.clone(),
                base_branch,
                branch_name,
                worktree_path: Some(worktree_path),
                provider_config_snapshot: provider_config,
                target_snapshot,
                max_auto_rework: 2,
            };
            let journal = match existing {
                Some(journal) => journal.clone(),
                None => coding_store
                    .prepare_group_initialization_with_admission_for_target(
                        &group_input,
                        &authoritative.plan_revision_id,
                        &ordered_units,
                        CodingAdmissionKind::ScAdvance,
                        Some(target),
                    )
                    .map_err(|error| format!("prepare group initialization failed: {error}"))?,
            };
            // REQ-MTG-05（WP5 增殖审计）：分流创建/恢复/重放每过此点都以幂等
            // 语义落审计（store 侧首写定档——身份一致命中不重写；中断后重放
            // 补齐缺口不漂移，R7「创建/恢复/重放入口」全覆盖）。审计只记
            // 事实，不承载调度。
            record_split_audit(
                &coding_store,
                &input,
                &authoritative,
                &target,
                &journal.attempt.id,
            )?;
            let order_index = journal
                .units
                .iter()
                .map(|unit| {
                    global_position
                        .get(unit.logical_work_item_id.as_str())
                        .copied()
                        .unwrap_or(usize::MAX)
                })
                .min()
                .unwrap_or(usize::MAX);
            targets.push(SplitTarget {
                target,
                journal,
                order_index,
            });
        }
        targets.sort_by_key(|split| split.order_index);
        let first_attempt_id = targets
            .first()
            .ok_or("advance split has no target attempts")?
            .journal
            .attempt
            .id
            .clone();
        let target_attempts: Vec<AdvanceTargetAttemptBinding> = targets
            .iter()
            .map(|split| AdvanceTargetAttemptBinding {
                target_repository_id: split.target.0.to_string(),
                attempt_id: split.journal.attempt.id.clone(),
            })
            .collect();
        let target_attempt_ids: Vec<String> = targets
            .iter()
            .map(|split| split.journal.attempt.id.clone())
            .collect();

        // 外层 advance journal 集绑定（OQ1）：store 侧做集合一致性比对（:508-515
        // 断言的集合化升级）；重放时 record 集绑定与 journal 全集互证。
        let mut outer = advance_store
            .load_or_prepare_advance_initialization_for_attempts(
                &record,
                &first_attempt_id,
                &target_attempt_ids,
            )
            .map_err(|error| format!("persist advance initialization failed: {error}"))?;
        maybe_fail_advance_initialization(&input, AdvanceInitializationFailpoint::JournalPrepared)?;
        if outer.phase.order_for_engine()
            >= AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
        {
            let current_record = advance_store
                .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
                .map_err(|error| format!("reload advance record failed: {error}"))?
                .ok_or("advance record disappeared")?;
            let mut persisted_attempts: Vec<String> = current_record
                .target_attempts
                .iter()
                .map(|binding| binding.attempt_id.clone())
                .collect();
            let mut expected_attempts: Vec<String> = target_attempt_ids.clone();
            persisted_attempts.sort_unstable();
            expected_attempts.sort_unstable();
            if current_record.attempt_id.as_deref() != Some(outer.attempt_id.as_str())
                || persisted_attempts != expected_attempts
            {
                return Err(
                    "advance record attempt set differs from initialization journal".to_string(),
                );
            }
        }

        // per-target attempt 持久化：各自 creation lock 与 failpoint 前缀，与单
        // target 路径同套（中断恢复由 journal.phase.has_reached 前缀判定续走）。
        for split in &mut targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
            ) {
                let record_attempt = coding_store
                    .ensure_group_initialization_attempt(
                        &split.journal,
                        &coding_store
                            .acquire_work_item_attempt_creation(
                                &input.project_id,
                                &input.issue_id,
                                &split.journal.lock_work_item_id,
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
                split.journal = coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::AttemptPersisted,
                    )
                    .map_err(|error| {
                        format!("checkpoint group attempt persistence failed: {error}")
                    })?;
                maybe_fail_advance_initialization(
                    &input,
                    AdvanceInitializationFailpoint::AttemptPersisted,
                )?;
                if split.journal.attempt.id != record_attempt.id {
                    return Err(
                        "group initialization attempt identity changed during replay"
                            .to_string(),
                    );
                }
            }
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::AttemptPersisted.order_for_engine()
        {
            let mut updated = record.clone();
            updated.attempt_id = Some(first_attempt_id.clone());
            updated.target_attempts = target_attempts.clone();
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

        // per-target worktree 绑定：repo 维三元键三件套 + T2S3 嵌套共存检查。
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or("lifecycle store unavailable")?;
        for split in &mut targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
            ) {
                let worktree_path = split
                    .journal
                    .attempt
                    .worktree_path
                    .clone()
                    .ok_or("split group attempt has no worktree path")?;
                let parent = worktree_path
                    .parent()
                    .ok_or("split worktree path has no parent")?
                    .to_path_buf();
                Self::ensure_split_worktree_parent_free(
                    lifecycle,
                    &input.project_id,
                    &input.issue_id,
                    &parent,
                    split.target,
                )?;
                lifecycle
                    .upsert_repo_shared_worktree(
                        crate::product::lifecycle_store::UpsertRepoSharedWorktreeInput {
                            project_id: input.project_id.clone(),
                            issue_id: input.issue_id.clone(),
                            repository_id: split.target,
                            branch_name: split.journal.attempt.branch_name.clone(),
                            worktree_path,
                            base_branch: split.journal.attempt.base_branch.clone(),
                        },
                    )
                    .map_err(|error| format!("persist shared worktree failed: {error}"))?;
                // repo 维 lease 与 journal.worktree_lease_id（issue 维前缀）解耦：
                // 确定性派生自 journal id（重放同值幂等）；bind 语义要求
                // `repo_worktree_lease_` 前缀或 owner 已是 attempt id。
                let lease_id = format!("repo_worktree_lease_{}", split.journal.id);
                let lease = lifecycle
                    .try_acquire_repo_worktree_lock(
                        &input.project_id,
                        &input.issue_id,
                        split.target,
                        &split.journal.lock_work_item_id,
                        &lease_id,
                    )
                    .map_err(|error| format!("acquire shared worktree failed: {error}"))?;
                if !lease.acquired
                    && lease.worktree.current_lock_owner_id.as_deref()
                        != Some(split.journal.attempt.id.as_str())
                {
                    return Err("shared worktree is owned by another attempt".to_string());
                }
                lifecycle
                    .bind_repo_worktree_lock_to_attempt(
                        &input.project_id,
                        &input.issue_id,
                        split.target,
                        &split.journal.lock_work_item_id,
                        &split.journal.attempt.id,
                    )
                    .map_err(|error| format!("bind shared worktree failed: {error}"))?;
                split.journal = coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::WorktreeBound,
                    )
                    .map_err(|error| {
                        format!("checkpoint group worktree binding failed: {error}")
                    })?;
            }
        }
        maybe_fail_advance_initialization(&input, AdvanceInitializationFailpoint::WorktreeBound)?;
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

        // per-target plan binding 与 units 物化；外层相位在全部 per-target journal
        // 达到对应相位后推进。
        for split in &mut targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
            ) {
                coding_store
                    .ensure_group_initialization_plan_binding(&split.journal)
                    .map_err(|error| format!("persist group plan binding failed: {error}"))?;
                split.journal = coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::PlanBindingSaved,
                    )
                    .map_err(|error| {
                        format!("checkpoint group plan binding failed: {error}")
                    })?;
            }
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::PlanBindingSaved.order_for_engine()
        {
            let current_record = advance_store
                .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
                .map_err(|error| format!("reload advance record failed: {error}"))?
                .ok_or("advance record disappeared")?;
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
        for split in &mut targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
            ) {
                for index in 0..split.journal.units.len() {
                    coding_store
                        .ensure_group_initialization_unit(&split.journal, index)
                        .map_err(|error| format!("persist group unit failed: {error}"))?;
                }
                split.journal = coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::UnitsMaterialized,
                    )
                    .map_err(|error| {
                        format!("checkpoint group units materialization failed: {error}")
                    })?;
            }
        }
        if outer.phase.order_for_engine()
            < AdvanceInitializationPhase::UnitsMaterialized.order_for_engine()
        {
            let current_record = advance_store
                .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
                .map_err(|error| format!("reload advance record failed: {error}"))?
                .ok_or("advance record disappeared")?;
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

        let mut persisted_attempts = Vec::with_capacity(targets.len());
        for split in &targets {
            let persisted = coding_store
                .get_attempt(
                    &input.project_id,
                    &input.issue_id,
                    &split.journal.attempt.id,
                )
                .map_err(|error| format!("load initialized attempt failed: {error}"))?;
            coding_store
                .validate_group_attempt_integrity(&persisted)
                .map_err(|error| format!("validate initialized group failed: {error}"))?;
            persisted_attempts.push(persisted);
        }
        for split in &targets {
            if !split.journal.phase.has_reached(
                crate::product::coding_attempt_store::CodingGroupInitializationPhase::Completed,
            ) {
                coding_store
                    .advance_group_initialization_phase(
                        &split.journal,
                        crate::product::coding_attempt_store::CodingGroupInitializationPhase::Completed,
                    )
                    .map_err(|error| {
                        format!("checkpoint group initialization completion failed: {error}")
                    })?;
            }
        }
        let final_record = advance_store
            .get_advance_for_plan(&input.project_id, &input.issue_id, &record.plan_id)
            .map_err(|error| format!("reload final advance record failed: {error}"))?
            .ok_or("advance record disappeared")?;
        advance_store
            .advance_initialization_phase(&final_record, &outer, AdvanceInitializationPhase::Ready)
            .map_err(|error| format!("checkpoint ready initialization failed: {error}"))?;
        let workspace_entry = Self::advance_workspace_entry(&persisted_attempts[0]);
        let mut ready_record = final_record;
        ready_record.status = AdvanceStatus::Ready;
        ready_record.attempt_id = Some(first_attempt_id.clone());
        ready_record.target_attempts = target_attempts.clone();
        ready_record.workspace_entry = Some(workspace_entry.clone());
        ready_record.updated_at = chrono::Utc::now().to_rfc3339();
        advance_store
            .update_record(&ready_record)
            .map_err(|error| format!("persist ready advance record failed: {error}"))?;
        Ok(AdvanceOutcome::Completed {
            record: ready_record,
            attempt_id: first_attempt_id,
            workspace_entry,
            target_attempts,
        })
    }

    /// T2S3（OQ2 披露，worktree 嵌套共存检查）：per-target worktree 父路径
    /// `.worktrees/aria-issues/{issue_id}` 若已被 issue 级 shared worktree 或其他
    /// repo 维 worktree 记录占用（比对 worktree_path）→ fail-closed。本 target
    /// 既有记录由随后的 repo 维 upsert 覆写归一（同三元键权威自愈）。
    // 供 initialize_advance_split 调用（同模块）。
    fn ensure_split_worktree_parent_free(
        lifecycle: &crate::product::lifecycle_store::LifecycleStore,
        project_id: &str,
        issue_id: &str,
        parent: &std::path::Path,
        target: crate::product::logical_codebase::LogicalRepositoryId,
    ) -> Result<(), String> {
        if let Some(issue_worktree) = lifecycle
            .get_issue_shared_worktree(project_id, issue_id)
            .map_err(|error| format!("load issue shared worktree failed: {error}"))?
            && issue_worktree.worktree_path == parent
        {
            return Err(format!(
                "split worktree parent path is already registered as the issue shared worktree: {}",
                parent.display()
            ));
        }
        for repository in lifecycle
            .list_repo_shared_worktrees(project_id, issue_id)
            .map_err(|error| format!("list repo shared worktrees failed: {error}"))?
        {
            if repository == target {
                continue;
            }
            if let Some(record) = lifecycle
                .get_repo_shared_worktree(project_id, issue_id, repository)
                .map_err(|error| format!("load repo shared worktree failed: {error}"))?
                && record.worktree_path == parent
            {
                return Err(format!(
                    "split worktree parent path is already registered as another repository worktree: {}",
                    parent.display()
                ));
            }
        }
        Ok(())
    }
}

/// REQ-MTG-05（WP5 增殖审计）：per-target 分流审计写入（T2 占位的真实现，
/// R10 交接——签名按计划 Interfaces 定案结构从 T2 编译期接口演化为逐 target
/// 调用：定案记录需 bound_plan_revision_id/dependency_graph_revision_id/trigger
/// 三类 T2 占位签名未承载的字段）。落点
/// `issue_lifecycle_root/{project}/{issue}/split-audit/{attempt_id}.json`，
/// 一 target-attempt 一条；`trigger.command_id` 从 `AdvanceInput` 透传。
fn record_split_audit(
    coding_store: &CodingAttemptStore,
    input: &AdvanceInput,
    authoritative: &AuthoritativeGroupPlanBinding,
    target: &crate::product::logical_codebase::LogicalRepositoryId,
    attempt_id: &str,
) -> Result<(), String> {
    coding_store
        .record_split_audit(&SplitAuditRecord {
            id: attempt_id.to_string(),
            project_id: input.project_id.clone(),
            issue_id: input.issue_id.clone(),
            plan_id: input.plan_id.clone(),
            target_repository_id: target.0.to_string(),
            attempt_id: attempt_id.to_string(),
            bound_plan_revision_id: authoritative.plan_revision_id.clone(),
            dependency_graph_revision_id: authoritative.dependency_graph_revision_id.clone(),
            trigger: SplitAuditTrigger {
                kind: SplitAuditTriggerKind::Advance,
                command_id: Some(input.command_id.clone()),
            },
            created_at: chrono::Utc::now().to_rfc3339(),
        })
        .map_err(|error| format!("record split audit failed: {error}"))
}
