use super::*;
use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::models::automation::EnrollmentBindingIdentity;
use crate::product::models::{
    ChildBindingIdentity, WorkItemContextBudget, WorkItemExecutionPlanStatus,
    WorkItemRuntimeBinding, WorkItemStatus, match_compile_child,
};
use crate::web::workspace_context::ensure_workspace_context_message;
use crate::web::workspace_ws_types::{
    WorkItemHistoryEntryDto, WorkItemHistoryEntryKind, WorkItemRevisionHistoryDto,
};

impl WorkspaceEngine {
    /// C1 Task 8（REQ-C1-CHILD-01）：从 durable enrollment 事实派生当前
    /// compile 的期望 child binding 身份来源。仅当 enrollment enabled 且
    /// binding history current 精确指向本 plan/session 时返回 Some；
    /// enrollment 缺失/关闭/指向他处一律 None（manual 链复用语义），不猜
    /// 「最新 plan/session」。
    pub(crate) fn current_enrollment_binding_for_compile(
        &self,
        lifecycle: &LifecycleStore,
        plan_id: &str,
    ) -> Result<Option<EnrollmentBindingIdentity>, ProductStoreError> {
        let store = IssueAutomationStore::new(lifecycle.app_paths());
        let Some(enrollment) = store.get(&self.session.project_id, &self.session.issue_id)? else {
            return Ok(None);
        };
        if !enrollment.enabled {
            return Ok(None);
        }
        Ok(enrollment
            .binding_history
            .filter(|history| {
                history.current.plan_id == plan_id
                    && history.current.session_id == self.session.session_id
            })
            .map(|history| history.current))
    }
    pub(super) fn resume_initial_plan_compile_transaction(
        &mut self,
        store: &WorkItemPlanStore,
        tx: &mut WorkItemPlanCompileTransaction,
    ) -> Result<InitialPlanCompileOutcome, String> {
        let index = store
            .load_active_index(&tx.project_id, &tx.issue_id, &tx.plan_id)
            .map_err(|error| format!("load work item plan active index failed: {error}"))?
            .ok_or_else(|| "work item plan active index missing".to_string())?;
        if index.current_generation_round_id != tx.generation_round_id {
            return Err("compile recovery generation round changed".to_string());
        }
        let outline_candidate = self.latest_work_item_plan_outline_candidate()?;
        if outline_candidate.outline.id != tx.outline_version_ref {
            return Err("compile recovery outline version changed".to_string());
        }
        let outline_order = work_item_plan_outline_topological_order(&outline_candidate.outline)?;
        let draft_records =
            self.accepted_active_draft_records_for_compile(store, &index, &outline_order)?;
        let active_draft_ids = draft_records
            .iter()
            .map(|record| record.draft_id.clone())
            .collect::<Vec<_>>();
        if active_draft_ids != tx.active_draft_ids {
            return Err("compile recovery active draft bindings changed".to_string());
        }
        let accepted_drafts = draft_records
            .iter()
            .map(work_item_draft_revision_from_record)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| error.to_string())?;
        let outcome = self
            .compile_initial_plan_revision(&accepted_drafts)
            .map_err(|error| error.to_string())?;
        let compiled_by_logical_id = outcome
            .work_items
            .iter()
            .map(|item| (item.work_item_revision.logical_work_item_id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        tx.outline_to_work_item_id = draft_records
            .iter()
            .map(|draft| {
                (
                    draft.outline_id.clone(),
                    draft.candidate.logical_work_item_id.clone(),
                )
            })
            .collect();
        tx.outline_to_verification_plan_id = draft_records
            .iter()
            .map(|draft| {
                let compiled = compiled_by_logical_id
                    .get(draft.candidate.logical_work_item_id.as_str())
                    .ok_or_else(|| {
                        format!(
                            "compiled outcome missing `{}` during recovery",
                            draft.candidate.logical_work_item_id
                        )
                    })?;
                Ok((
                    draft.outline_id.clone(),
                    compiled.verification_plan_revision.id.clone(),
                ))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        // D2.1b 探针：legacy resume 恢复重放位点（resume 还在跑同样 busy）。
        #[cfg(test)]
        probe_work_item_plan_compile_admission(
            &tx.project_id,
            &tx.issue_id,
            &tx.plan_id,
            "legacy_publication_resumed",
        );
        tx.step_cursor = "publication_resumed".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save resumed publication cursor failed: {error}"))?;
        Ok(outcome)
    }

    /// park-1（8-19 验收欠账）：finalize 持久化 per-target
    /// LifecycleWorkItemRecord（work-items/{id}.json）。
    ///
    /// 记录 id 取 logical work item id（entity 语义）：coding 会话创建走
    /// `workspace_repository` 的 WorkItem 臂 `list_work_items →
    /// find(work_item.id == session.entity_id)`，plan.work_item_ids 与
    /// child session entity_id 均为 logical 域。归属从 compiled
    /// work_item_revision（intent target）+ 与 Final Compile 同源的成员
    /// target 表派生；Legacy 单仓落 story 仓。upsert 幂等：重放不重复建、
    /// 不清执行态。
    fn persist_compiled_work_item_records(
        &self,
        lifecycle: &LifecycleStore,
        tx: &mut WorkItemPlanCompileTransaction,
        outcome: &InitialPlanCompileOutcome,
    ) -> Result<(), String> {
        let ordered = &outcome
            .plan_projection_bundle
            .coder_group_context
            .ordered_logical_work_item_ids;
        let compiled_by_logical_id = outcome
            .work_items
            .iter()
            .map(|item| (item.work_item_revision.logical_work_item_id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        let member_targets = match self
            .logical_work_item_plan_repository_targets(lifecycle, &tx.previous_plan_snapshot)
            .map_err(|error| {
                format!("resolve plan repository targets for work item records failed: {error}")
            })? {
            None => None,
            Some(_) => {
                let session_record = lifecycle
                    .get_workspace_session(&self.session.session_id)
                    .map_err(|error| {
                        format!("load plan session record for work item records failed: {error}")
                    })?;
                let session_repository =
                    crate::product::workspace_repository::resolve_workspace_repository_context(
                        &lifecycle.app_paths(),
                        lifecycle,
                        &session_record,
                    )
                    .map_err(|error| {
                        format!(
                            "resolve plan session repository for work item records failed: {error}"
                        )
                    })?
                    .business_repository
                    .map_err(|error| {
                        format!(
                            "resolve plan session business repository for work item records failed: {error}"
                        )
                    })?;
                crate::product::workspace_engine::draft_batch::compile_support::plan_compile_member_targets(
                    lifecycle,
                    &tx.previous_plan_snapshot,
                    &session_repository,
                )
                .map_err(|error| {
                    format!("resolve plan member targets for work item records failed: {error}")
                })?
            }
        };
        let legacy_repository_id = if member_targets.is_none() {
            self.work_item_plan_repository_id(lifecycle, &tx.previous_plan_snapshot)
                .map_err(|error| {
                    format!("resolve legacy repository for work item records failed: {error}")
                })?
        } else {
            String::new()
        };
        // SC 纪律：finalize 不得读 legacy compile store（panic_on_legacy_
        // compile_reads 只挂读方法），source_draft_id 不在此派生——消费面
        // （group draft context）自有 tx 兜底解析。
        let mut records = Vec::with_capacity(ordered.len());
        for (index, logical_id) in ordered.iter().enumerate() {
            let compiled = compiled_by_logical_id.get(logical_id.as_str()).ok_or_else(|| {
                format!(
                    "compiled outcome missing logical work item `{logical_id}` during work item record derivation"
                )
            })?;
            let contract = &compiled.work_item_revision.canonical_contract;
            let intent_target = contract
                .intent_contract
                .as_ref()
                .map(|intent| &intent.target);
            let (repository_id, target_repository_id) = work_item_record_attribution(
                member_targets.as_ref(),
                &legacy_repository_id,
                intent_target,
                logical_id,
            )?;
            // outline 反查双域兼容：legacy tx 值域为 logical id；SC tx 键域
            //（outline id == logical id）命中时取键本身，绝不取值（值为物理
            // work_item id，P2 复审缺陷）。
            let source_outline_id = tx
                .outline_to_work_item_id
                .iter()
                .find(|(_, value)| value.as_str() == logical_id.as_str())
                .map(|(outline_id, _)| outline_id.clone())
                .or_else(|| {
                    tx.outline_to_work_item_id
                        .contains_key(logical_id)
                        .then(|| logical_id.clone())
                });
            let source_draft_id = None;
            let depends_on = outcome
                .dependency_graph_revision
                .edges
                .iter()
                .filter(|edge| edge.to == *logical_id)
                .map(|edge| edge.from.clone())
                .collect::<Vec<_>>();
            records.push(LifecycleWorkItemRecord {
                id: logical_id.clone(),
                project_id: tx.project_id.clone(),
                issue_id: tx.issue_id.clone(),
                repository_id,
                target_repository_id,
                story_spec_ids: tx.previous_plan_snapshot.source_story_spec_ids.clone(),
                design_spec_ids: tx.previous_plan_snapshot.source_design_spec_ids.clone(),
                title: contract.identity.title.clone(),
                plan_status: WorkItemPlanStatus::Confirmed,
                execution_status: WorkItemStatus::Pending,
                worktree_path: None,
                work_item_set_id: Some(tx.plan_id.clone()),
                source_work_item_plan_id: Some(tx.plan_id.clone()),
                source_outline_id,
                source_draft_id,
                planned_implementation_context: None,
                kind: crate::product::work_item_split_engine::types::parse_work_item_kind(
                    &contract.identity.kind,
                ),
                sequence_hint: Some((index + 1) as u32),
                depends_on,
                exclusive_write_scopes: contract.write_policy.exclusive_scopes.clone(),
                forbidden_write_scopes: contract.write_policy.forbidden_scopes.clone(),
                context_budget: WorkItemContextBudget::default(),
                verification_plan_ref: Some(compiled.verification_plan_revision.id.clone()),
                require_execution_plan_confirm: tx
                    .previous_plan_snapshot
                    .options
                    .require_execution_plan_confirm,
                execution_plan_status: WorkItemExecutionPlanStatus::NotStarted,
                completion_commit: None,
                completion_diff_summary_ref: None,
                created_at: tx.created_at.clone(),
                updated_at: tx.created_at.clone(),
            });
        }
        for record in records {
            lifecycle
                .upsert_compiled_work_item_record(record)
                .map_err(|error| format!("persist compiled work item record failed: {error}"))?;
        }
        tx.created_work_item_ids = ordered.clone();
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn persist_compiled_work_item_records_for_test(
        &self,
        lifecycle: &LifecycleStore,
        tx: &mut WorkItemPlanCompileTransaction,
        outcome: &InitialPlanCompileOutcome,
    ) -> Result<(), String> {
        self.persist_compiled_work_item_records(lifecycle, tx, outcome)
    }

    pub(super) async fn finalize_initial_plan_compile(
        &mut self,
        lifecycle: &LifecycleStore,
        store: &WorkItemPlanStore,
        tx: &mut WorkItemPlanCompileTransaction,
        outcome: &InitialPlanCompileOutcome,
    ) -> Result<(), String> {
        // 单 plan 不变量（REQ-WPS-01/REQ-WPS-02 Scenario「重新编译确认不产生第二
        // 个 plan」与「原地重编译确认不得跳过旧代运行资源清理」）：编译确认前
        // 先清旧代运行资源（attempts/worktree/reviewer/锁/workspace lifecycle）
        // 并删除同 issue 的其它存活 plan；失败 → 不确认新代（fail-closed）。
        //
        // plan-slot 互斥覆盖「取代删除 + 编译确认」全过程：并发的 prepare 不得
        // 在本编译确认期间建出第二个 plan。
        let plan_slot = crate::product::plan_generation_supersede::acquire_issue_plan_slot(
            &lifecycle.app_paths(),
            &tx.project_id,
            &tx.issue_id,
        )
        .await
        .map_err(|error| {
            format!("acquire issue plan slot before compile confirm failed: {error:?}")
        })?;
        // F2/R2：旧代 **advance engine checkpoint** 的清理根。`ProductAppPaths`
        // 根是 `<workspace_root>/.aria`（`WebAppState` 的唯一构造口径；与
        // `logical_codebase::evidence_injection` 的注入根同源），故父目录即
        // workspace root。此前此处传 `None`，使 `purge_plan_runtime_resources`
        // 的 checkpoint 清理分支成为死代码——新代仍可能从旧代同 session 的
        // checkpoint 恢复。
        let app_paths = lifecycle.app_paths();
        // 只接受绝对父目录：相对根（如 `ProductAppPaths::new("aria")`）的
        // `parent()` 是空串，拼出的相对 checkpoint 根会落到进程 CWD——宁可不
        // 清（fail-safe）也不误删。
        let workspace_root = app_paths
            .root()
            .parent()
            .filter(|parent| parent.is_absolute())
            .map(std::path::Path::to_path_buf);
        crate::product::plan_generation_supersede::ensure_single_plan_generation(
            &app_paths,
            &plan_slot,
            &tx.project_id,
            &tx.issue_id,
            std::slice::from_ref(&tx.plan_id),
            crate::product::entity_deletion_cascade::PlanSupersedeReason::RecompileConfirm,
            workspace_root.as_deref(),
        )
        .await
        .map_err(|error| {
            format!("plan generation supersede before compile confirm failed: {error:?}")
        })?;
        let work_item_ids = outcome
            .plan_projection_bundle
            .coder_group_context
            .ordered_logical_work_item_ids
            .clone();
        let compiled_by_logical_id = outcome
            .work_items
            .iter()
            .map(|item| (item.work_item_revision.logical_work_item_id.as_str(), item))
            .collect::<BTreeMap<_, _>>();
        let verification_plan_ids = work_item_ids
            .iter()
            .map(|logical_id| {
                compiled_by_logical_id
                    .get(logical_id.as_str())
                    .map(|item| item.verification_plan_revision.id.clone())
                    .ok_or_else(|| {
                        format!(
                            "compiled outcome missing logical work item `{logical_id}` during finalization"
                        )
                    })
            })
            .collect::<Result<Vec<_>, _>>()?;

        let plan_update = IssueWorkItemPlanUpdate {
            work_item_ids: work_item_ids.clone(),
            verification_plan_ids: verification_plan_ids.clone(),
            repository_profile_ref: tx.previous_plan_snapshot.repository_profile_ref.clone(),
            dependency_graph: outcome
                .dependency_graph_revision
                .edges
                .iter()
                .map(|edge| IssueWorkItemDependencyEdge {
                    from_work_item_id: edge.from.clone(),
                    to_work_item_id: edge.to.clone(),
                })
                .collect(),
            created_from_provider_run: tx.previous_plan_snapshot.created_from_provider_run.clone(),
            validator_findings: tx.validator_findings.clone(),
        };
        tx.step_cursor = "plan_summary_prepared".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save plan summary cursor failed: {error}"))?;
        #[cfg(test)]
        self.maybe_fail_work_item_plan_compile_finalizer(
            tx,
            WorkItemPlanCompileFinalizerCheckpoint::PlanSummaryPrepared,
        )?;

        // park-1：per-target work item 记录必须在 child session 建立前
        // 落盘——coding 会话创建的 WorkItem 臂 find(entity_id) 依赖它们。
        self.persist_compiled_work_item_records(lifecycle, tx, outcome)?;
        tx.step_cursor = "work_item_records_persisted".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save work item records cursor failed: {error}"))?;
        #[cfg(test)]
        self.maybe_fail_work_item_plan_compile_finalizer(
            tx,
            WorkItemPlanCompileFinalizerCheckpoint::WorkItemRecordsPersisted,
        )?;

        // C1 Task 8（REQ-C1-CHILD-01）：读取 durable enrollment 当前代
        // binding——仅当 enrollment enabled 且 current binding 精确指向本
        // plan/session 时，child 按 binding_version/enrollment_id/target
        // 精确绑定；否则（manual/非 enrolled）expected 为 None，保持既有
        // entity_id 复用语义。
        let enrollment_binding = self
            .current_enrollment_binding_for_compile(lifecycle, &tx.plan_id)
            .map_err(|error| {
                format!("load enrollment binding for compile children failed: {error}")
            })?;
        let mut sessions = lifecycle
            .list_workspace_sessions(&tx.project_id, &tx.issue_id)
            .map_err(|error| format!("list child work item workspaces failed: {error}"))?;
        tx.child_session_ids.clear();
        for (index, logical_id) in work_item_ids.iter().enumerate() {
            let compiled = compiled_by_logical_id
                .get(logical_id.as_str())
                .ok_or_else(|| {
                    format!(
                        "compiled outcome missing logical work item `{logical_id}` during child workspace preparation"
                    )
                })?;
            let binding = WorkItemRuntimeBinding {
                plan_id: tx.plan_id.clone(),
                plan_revision_id: outcome.plan_revision.id.clone(),
                logical_work_item_id: logical_id.clone(),
                work_item_revision_id: compiled.work_item_revision.id.clone(),
                projection_bundle_id: compiled.projection_bundle.id.clone(),
                verification_plan_revision_id: compiled.verification_plan_revision.id.clone(),
                canonical_contract_hash: compiled
                    .work_item_revision
                    .canonical_contract_hash
                    .clone(),
                projection_compiler_version: compiled.projection_bundle.compiler_version.clone(),
                human_projection_hash: compiled.projection_bundle.human_projection_hash.clone(),
                coder_projection_hash: compiled.projection_bundle.coder_projection_hash.clone(),
                reviewer_projection_hash: compiled
                    .projection_bundle
                    .reviewer_projection_hash
                    .clone(),
            };
            let expected_child_binding =
                enrollment_binding
                    .as_ref()
                    .map(|binding| ChildBindingIdentity {
                        plan_id: tx.plan_id.clone(),
                        plan_revision_id: outcome.plan_revision.id.clone(),
                        logical_work_item_id: logical_id.clone(),
                        work_item_revision_id: compiled.work_item_revision.id.clone(),
                        binding_version: binding.binding_version,
                        enrollment_id: binding.enrollment_id.clone(),
                        target: binding.target.clone(),
                    });
            let matched = sessions
                .iter()
                .filter(|session| {
                    session.workspace_type == WorkspaceType::WorkItem
                        && session.entity_id == *logical_id
                        && match_compile_child(session, expected_child_binding.as_ref())
                })
                .collect::<Vec<_>>();
            let session_id = match matched.as_slice() {
                [existing] => existing.id.clone(),
                [] => {
                    let input = CreateWorkspaceSessionInput {
                        project_id: tx.project_id.clone(),
                        issue_id: tx.issue_id.clone(),
                        entity_id: logical_id.clone(),
                        workspace_type: WorkspaceType::WorkItem,
                        author_provider: self.session.author_provider.clone(),
                        // C2 Task 5（REQ-CRO-05）：三值透传——不回填 Codex。
                        reviewer_provider: self.session.reviewer_provider.clone(),
                        review_rounds: self.session.review_rounds,
                        superpowers_enabled: self.session.superpowers_enabled,
                        openspec_enabled: self.session.openspec_enabled,
                        work_item_plan_options: None,
                    };
                    let created = match expected_child_binding {
                        Some(identity) => lifecycle.create_workspace_child_session(
                            CreateWorkItemChildSessionInput {
                                session: input,
                                child_binding: identity,
                            },
                        ),
                        None => lifecycle.create_workspace_session(input),
                    }
                    .map_err(|error| format!("create child work item workspace failed: {error}"))?;
                    let id = created.id.clone();
                    sessions.push(created);
                    id
                }
                _ => {
                    return Err(format!(
                        "expected exactly one child work item workspace for `{logical_id}`, found {}",
                        matched.len()
                    ));
                }
            };
            tx.child_session_ids.push(session_id.clone());
            tx.step_cursor = format!("child_session_{:03}_ensured", index + 1);
            tx.updated_at = tx.created_at.clone();
            store
                .put_compile_transaction(tx)
                .map_err(|error| format!("save child session cursor failed: {error}"))?;
            #[cfg(test)]
            if index == 0 {
                self.maybe_fail_work_item_plan_compile_finalizer(
                    tx,
                    WorkItemPlanCompileFinalizerCheckpoint::FirstChildSessionEnsured,
                )?;
            }
            let bound_session = lifecycle
                .ensure_work_item_runtime_binding(&session_id, &binding)
                .map_err(|error| format!("ensure child runtime binding failed: {error}"))?;
            tx.step_cursor = format!("child_session_{:03}_binding_ensured", index + 1);
            tx.updated_at = tx.created_at.clone();
            store
                .put_compile_transaction(tx)
                .map_err(|error| format!("save child binding cursor failed: {error}"))?;
            #[cfg(test)]
            if index == 0 {
                self.maybe_fail_work_item_plan_compile_finalizer(
                    tx,
                    WorkItemPlanCompileFinalizerCheckpoint::FirstChildBindingEnsured,
                )?;
            }
            ensure_workspace_context_message(&lifecycle.app_paths(), lifecycle, bound_session)
                .await
                .map_err(|error| format!("ensure child workspace context failed: {error}"))?;
            tx.step_cursor = format!("child_session_{:03}_context_prepared", index + 1);
            tx.updated_at = tx.created_at.clone();
            store
                .put_compile_transaction(tx)
                .map_err(|error| format!("save child context cursor failed: {error}"))?;
            #[cfg(test)]
            if index == 0 {
                self.maybe_fail_work_item_plan_compile_finalizer(
                    tx,
                    WorkItemPlanCompileFinalizerCheckpoint::FirstChildContextPrepared,
                )?;
            }
        }
        tx.step_cursor = "child_workspaces_prepared".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save child sessions cursor failed: {error}"))?;

        lifecycle
            .commit_issue_work_item_plan(&tx.project_id, &tx.issue_id, &tx.plan_id, plan_update)
            .map_err(|error| format!("commit issue work item plan failed: {error}"))?;
        tx.step_cursor = "plan_confirmed".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save plan confirmed cursor failed: {error}"))?;

        let compile_report = WorkItemPlanCompileReportPayload {
            compile_id: tx.compile_id.clone(),
            generation_round_id: tx.generation_round_id.clone(),
            status: WorkItemPlanCompileStatus::Committed,
            plan_commit_state: WorkItemPlanCommitState::Committed,
            work_item_ids,
            verification_plan_ids,
            child_session_ids: tx.child_session_ids.clone(),
            validator_findings: work_item_split_findings_to_dto(&tx.validator_findings),
        };
        let compile_report_update = self
            .persist_compile_report(compile_report, &tx.created_at)
            .await?;
        self.persist_initial_projection_artifacts(lifecycle, outcome, compile_report_update)
            .await?;
        tx.step_cursor = "compile_report_persisted".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save compile report cursor failed: {error}"))?;
        #[cfg(test)]
        self.maybe_fail_work_item_plan_compile_finalizer(
            tx,
            WorkItemPlanCompileFinalizerCheckpoint::CompileReportPersisted,
        )?;

        // D2.1b 探针：final 提交位点（commit 段终点，lease 仍持）。
        #[cfg(test)]
        probe_work_item_plan_compile_admission(
            &tx.project_id,
            &tx.issue_id,
            &tx.plan_id,
            "committed",
        );
        tx.status = WorkItemPlanCompileStatus::Committed;
        tx.plan_commit_state = WorkItemPlanCommitState::Committed;
        tx.committed_at = Some(tx.created_at.clone());
        tx.failure_reason = None;
        tx.step_cursor = "committed".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save compile committed marker failed: {error}"))
    }

    async fn persist_compile_report(
        &mut self,
        compile_report: WorkItemPlanCompileReportPayload,
        created_at: &str,
    ) -> Result<ArtifactUpdateEvent, String> {
        let payload = ArtifactPayload::WorkItemPlanCompileReport {
            compile_report: Box::new(compile_report.clone()),
        };
        let matching = self
            .artifact_versions
            .iter()
            .enumerate()
            .filter(|(_, version)| {
                matches!(
                    &version.payload,
                    ArtifactPayload::WorkItemPlanCompileReport { compile_report: existing }
                        if existing.compile_id == compile_report.compile_id
                )
            })
            .map(|(index, _)| index)
            .collect::<Vec<_>>();
        if matching.len() > 1 {
            return Err(format!(
                "duplicate compile reports for `{}`",
                compile_report.compile_id
            ));
        }
        if let Some(index) = matching.first().copied() {
            if self.artifact_versions[index].payload != payload {
                return Err(format!(
                    "compile report identity mismatch for `{}`",
                    compile_report.compile_id
                ));
            }
            self.artifact_versions[index].created_at = created_at.to_string();
            self.session.artifact = Some(payload.clone());
            Ok(ArtifactUpdateEvent {
                version: self.artifact_versions[index].version,
                payload,
            })
        } else {
            let update = self
                .append_artifact_version_without_event(payload, created_at)
                .await;
            let version = self.artifact_versions.last_mut().ok_or_else(|| {
                "compile report artifact version missing after update".to_string()
            })?;
            version.created_at = created_at.to_string();
            Ok(update)
        }
    }

    async fn persist_initial_projection_artifacts(
        &mut self,
        lifecycle: &LifecycleStore,
        outcome: &InitialPlanCompileOutcome,
        compile_report_update: ArtifactUpdateEvent,
    ) -> Result<(), String> {
        let mut updates = vec![compile_report_update];
        for item in &outcome.work_items {
            updates.push(
                self.ensure_initial_projection_artifact(
                    ArtifactPayload::WorkItemProjection {
                        projection: Box::new(item.projection_bundle.clone()),
                    },
                    &outcome.plan_revision.created_at,
                )
                .await?,
            );
        }
        updates.push(
            self.ensure_initial_projection_artifact(
                ArtifactPayload::ProjectionValidation {
                    report: Box::new(outcome.projection_validation.clone()),
                },
                &outcome.plan_revision.created_at,
            )
            .await?,
        );
        updates.push(
            self.ensure_initial_projection_artifact(
                ArtifactPayload::WorkItemRevisionHistory {
                    history: Box::new(self.initial_revision_history(outcome)),
                },
                &outcome.plan_revision.created_at,
            )
            .await?,
        );
        let plan_payload = ArtifactPayload::WorkItemPlanProjection {
            projection: Box::new(outcome.plan_projection_bundle.clone()),
        };
        let plan_update = if let Some(index) = self
            .artifact_versions
            .iter()
            .position(|version| version.payload == plan_payload)
        {
            for version in &mut self.artifact_versions {
                version.is_current = false;
            }
            self.artifact_versions[index].is_current = true;
            self.session.artifact = Some(plan_payload.clone());
            ArtifactUpdateEvent {
                version: self.artifact_versions[index].version,
                payload: plan_payload,
            }
        } else {
            self.ensure_projection_identity_is_immutable(&plan_payload)?;
            self.append_artifact_version_without_event(
                plan_payload,
                &outcome.plan_revision.created_at,
            )
            .await
        };
        updates.push(plan_update);
        lifecycle
            .save_artifact_versions(&self.session.session_id, &self.artifact_versions)
            .map_err(|error| format!("persist initial projection artifacts failed: {error}"))?;
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactBatchUpdate { updates })
            .await;
        Ok(())
    }

    async fn ensure_initial_projection_artifact(
        &mut self,
        payload: ArtifactPayload,
        created_at: &str,
    ) -> Result<ArtifactUpdateEvent, String> {
        if let Some(version) = self
            .artifact_versions
            .iter()
            .find(|version| version.payload == payload)
        {
            return Ok(ArtifactUpdateEvent {
                version: version.version,
                payload,
            });
        }
        self.ensure_projection_identity_is_immutable(&payload)?;
        Ok(self
            .append_artifact_version_without_event(payload, created_at)
            .await)
    }

    async fn append_artifact_version_without_event(
        &mut self,
        payload: ArtifactPayload,
        created_at: &str,
    ) -> ArtifactUpdateEvent {
        self.session.artifact = Some(payload.clone());
        for version in &mut self.artifact_versions {
            version.is_current = false;
        }
        let version = self.artifact_versions.len() as u32 + 1;
        let source_node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "timeline_node_unknown".to_string());
        self.artifact_versions.push(ArtifactVersion {
            version,
            payload: payload.clone(),
            generated_by: self.session.author_provider.clone(),
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: created_at.to_string(),
            source_node_id: source_node_id.clone(),
        });
        let _ = self
            .persist_artifact_ref(
                &source_node_id,
                ArtifactRef {
                    artifact_id: format!("artifact_version_{version:03}"),
                    version,
                },
            )
            .await;
        ArtifactUpdateEvent { version, payload }
    }

    fn ensure_projection_identity_is_immutable(
        &self,
        payload: &ArtifactPayload,
    ) -> Result<(), String> {
        for version in &self.artifact_versions {
            let identity_conflict = match (&version.payload, payload) {
                (
                    ArtifactPayload::WorkItemPlanProjection {
                        projection: existing,
                    },
                    ArtifactPayload::WorkItemPlanProjection { projection: next },
                ) => existing.id == next.id && existing != next,
                (
                    ArtifactPayload::WorkItemProjection {
                        projection: existing,
                    },
                    ArtifactPayload::WorkItemProjection { projection: next },
                ) => existing.id == next.id && existing != next,
                _ => false,
            };
            if identity_conflict {
                return Err("projection artifact identity mismatch".to_string());
            }
        }
        Ok(())
    }

    fn initial_revision_history(
        &self,
        outcome: &InitialPlanCompileOutcome,
    ) -> WorkItemRevisionHistoryDto {
        let mut entries = Vec::new();
        for item in &outcome.work_items {
            entries.push(WorkItemHistoryEntryDto {
                kind: WorkItemHistoryEntryKind::DraftRevision,
                id: item.draft_revision.id.clone(),
                logical_work_item_id: item.draft_revision.logical_work_item_id.clone(),
                related_revision_id: Some(item.work_item_revision.id.clone()),
                summary: format!("Draft revision {}", item.draft_revision.revision_no),
                created_at: item.draft_revision.created_at.clone(),
            });
            entries.push(WorkItemHistoryEntryDto {
                kind: WorkItemHistoryEntryKind::WorkItemRevision,
                id: item.work_item_revision.id.clone(),
                logical_work_item_id: item.work_item_revision.logical_work_item_id.clone(),
                related_revision_id: Some(item.work_item_revision.source_draft_revision_id.clone()),
                summary: format!(
                    "Compiled WorkItem revision from {}",
                    item.work_item_revision.source_draft_revision_id
                ),
                created_at: item.work_item_revision.created_at.clone(),
            });
        }
        for node in self.timeline_nodes.iter().filter(|node| {
            matches!(
                node.node_type,
                TimelineNodeType::WorkItemPlanOutlineReview
                    | TimelineNodeType::WorkItemDraftReview
                    | TimelineNodeType::WorkItemBatchReview
            )
        }) {
            let logical_work_item_id = self.artifact_versions.iter().find_map(|version| {
                if version.source_node_id != node.node_id {
                    return None;
                }
                match &version.payload {
                    ArtifactPayload::WorkItemDraftCandidate { draft_candidate } => Some(
                        draft_candidate
                            .draft_record
                            .candidate
                            .logical_work_item_id
                            .clone(),
                    ),
                    _ => None,
                }
            });
            let Some(logical_work_item_id) = logical_work_item_id else {
                continue;
            };
            let related_revision_id = outcome
                .work_items
                .iter()
                .find(|item| item.work_item_revision.logical_work_item_id == logical_work_item_id)
                .map(|item| item.work_item_revision.id.clone());
            entries.push(WorkItemHistoryEntryDto {
                kind: WorkItemHistoryEntryKind::PlanReview,
                id: node.node_id.clone(),
                logical_work_item_id,
                related_revision_id,
                summary: node.summary.clone().unwrap_or_else(|| node.title.clone()),
                created_at: node.started_at.clone(),
            });
        }
        entries.sort_by(|left, right| {
            left.created_at
                .cmp(&right.created_at)
                .then_with(|| left.id.cmp(&right.id))
        });
        WorkItemRevisionHistoryDto { entries }
    }
}
/// park-1：单个 compiled work item 的仓库归属解析（纯函数，供 finalize 与
/// 单测共用）。与 Final Compile 的成员归属语义同源（lower.rs
/// `resolve_item_target_repository_id` / ir_adapter `logical_target_for_ir_item`）：
/// - Legacy（`member_targets=None`）：story 仓，无逻辑 target；
/// - LC：intent target 按声明落成员物理仓（logical_codebase 声明直取，
///   single_repository 声明反查唯一逻辑成员）；未声明 intent 落确定性
///   回落仓并反查逻辑成员。目标失效/非成员/回落缺失 →
///   `work_item_target_missing` 语义错 fail-closed，不猜默认。
pub(crate) fn work_item_record_attribution(
    member_targets: Option<&crate::product::work_item_plan_compiler::PlanMemberTargets>,
    legacy_repository_id: &str,
    intent_target: Option<&crate::product::logical_codebase::EnrollmentTarget>,
    logical_work_item_id: &str,
) -> Result<
    (
        String,
        Option<crate::product::logical_codebase::LogicalRepositoryId>,
    ),
    String,
> {
    use crate::product::logical_codebase::{EnrollmentTarget, LogicalRepositoryId};
    use crate::product::workspace_engine::member_attribution::{
        MemberReverse, reverse_member_logical,
    };

    let Some(targets) = member_targets else {
        // 批 2 R1（REQ-WIGA-01）：无成员表时集合 intent 同样拒绝——集合
        // target 不能作为单 WI 归属（不落 story 仓兜底，语义错 fail-closed）。
        if let Some(EnrollmentTarget::LogicalCodebaseMembers { .. }) = intent_target {
            return Err(format!(
                "work_item_target_missing: `{logical_work_item_id}` 声明了集合 target\
                 （logical_codebase_members）；单个 work item 的归属必须是恰一成员"
            ));
        }
        return Ok((legacy_repository_id.to_string(), None));
    };
    // REQ-WG-02：物理仓 → 逻辑成员的唯一反查来源（候选结构门共用同一
    // 函数）；错误文案与三态判定逐字保持既有语义。
    let reverse_logical = |physical: &str| -> Result<LogicalRepositoryId, String> {
        match reverse_member_logical(physical, &targets.physical_by_logical) {
            MemberReverse::Unique(logical) => Ok(logical),
            MemberReverse::Absent => Err(format!(
                "work_item_target_missing: physical repository `{physical}` for `{logical_work_item_id}` is not an effective logical member"
            )),
            MemberReverse::Ambiguous => Err(format!(
                "work_item_target_missing: physical repository `{physical}` for `{logical_work_item_id}` maps to multiple logical repositories"
            )),
        }
    };
    match intent_target {
        Some(EnrollmentTarget::LogicalCodebase {
            logical_repository_id,
            ..
        }) => {
            let physical = targets.physical_by_logical.get(logical_repository_id).ok_or_else(
                || {
                    format!(
                        "work_item_target_missing: declared logical member `{logical_repository_id:?}` for `{logical_work_item_id}` is not effective"
                    )
                },
            )?;
            Ok((physical.clone(), Some(*logical_repository_id)))
        }
        // 批 2 R1（REQ-WIGA-01）：集合 intent 拒绝（含成员表在场——集合
        // target 不能作为单 WI 归属）。
        Some(EnrollmentTarget::LogicalCodebaseMembers { .. }) => Err(format!(
            "work_item_target_missing: `{logical_work_item_id}` 声明了集合 target\
             （logical_codebase_members）；单个 work item 的归属必须是恰一成员"
        )),
        Some(EnrollmentTarget::SingleRepository { repository_id }) => {
            Ok((repository_id.clone(), Some(reverse_logical(repository_id)?)))
        }
        None => {
            let fallback = targets.fallback_physical_id.as_deref().ok_or_else(|| {
                format!(
                    "work_item_target_missing: `{logical_work_item_id}` has no intent target and no deterministic fallback repository"
                )
            })?;
            Ok((fallback.to_string(), Some(reverse_logical(fallback)?)))
        }
    }
}
