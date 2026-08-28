use super::*;

#[cfg(test)]
#[allow(clippy::enum_variant_names)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum SingleCandidateCompileCheckpoint {
    ApprovalPersisted,
    ReservationPersisted,
    ProvenancePersisted,
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct SingleCandidateCompileFailpointKey {
    session_id: String,
    project_id: String,
    issue_id: String,
    plan_id: String,
    checkpoint: SingleCandidateCompileCheckpoint,
}

#[cfg(test)]
pub(crate) struct SingleCandidateCompileFailpointGuard {
    key: SingleCandidateCompileFailpointKey,
    registration_id: u64,
}

#[cfg(test)]
static SINGLE_CANDIDATE_COMPILE_FAILPOINTS: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<SingleCandidateCompileFailpointKey, u64>>,
> = std::sync::OnceLock::new();

#[cfg(test)]
static NEXT_SINGLE_CANDIDATE_COMPILE_FAILPOINT_ID: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(1);

impl WorkspaceEngine {
    #[cfg(test)]
    pub(crate) fn register_single_candidate_compile_failpoint(
        &self,
        checkpoint: SingleCandidateCompileCheckpoint,
    ) -> SingleCandidateCompileFailpointGuard {
        let key = SingleCandidateCompileFailpointKey {
            session_id: self.session.session_id.clone(),
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
            checkpoint,
        };
        let registration_id = NEXT_SINGLE_CANDIDATE_COMPILE_FAILPOINT_ID
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let previous = single_candidate_compile_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(key.clone(), registration_id);
        assert!(
            previous.is_none(),
            "single candidate compile failpoint already registered"
        );
        SingleCandidateCompileFailpointGuard {
            key,
            registration_id,
        }
    }

    #[cfg(test)]
    fn maybe_fail_single_candidate_compile(
        &self,
        checkpoint: SingleCandidateCompileCheckpoint,
    ) -> Result<(), String> {
        let key = SingleCandidateCompileFailpointKey {
            session_id: self.session.session_id.clone(),
            project_id: self.session.project_id.clone(),
            issue_id: self.session.issue_id.clone(),
            plan_id: self.session.entity_id.clone(),
            checkpoint,
        };
        if single_candidate_compile_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(&key)
            .is_some()
        {
            return Err(format!("single_candidate_compile_failpoint:{checkpoint:?}"));
        }
        Ok(())
    }

    pub(super) async fn run_single_candidate_initial_plan_compile(
        &mut self,
    ) -> Result<InitialPlanCompileOutcome, String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let failure = |engine: &mut WorkspaceEngine, code: &str, message: String| {
            engine.record_single_candidate_compile_failure(&lifecycle, code, &message);
            message
        };
        let record = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| format!("load single candidate session failed: {error}"))?;
        if record.project_id != self.session.project_id
            || record.issue_id != self.session.issue_id
            || record.entity_id != self.session.entity_id
            || record.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || record.single_candidate_phase
                != Some(crate::product::models::SingleCandidatePhase::Approval)
        {
            return Err(failure(
                self,
                "single_candidate_invalid_session",
                "single candidate compile session is not in Approval for this scope".to_string(),
            ));
        }
        let scope = SourceStoreScope {
            project_id: record.project_id.clone(),
            issue_id: record.issue_id.clone(),
            plan_id: record.entity_id.clone(),
        };
        let source_ref = record
            .work_item_plan_source_revision_ref
            .clone()
            .ok_or_else(|| {
                failure(
                    self,
                    "SOURCE_STORE_MALFORMED_REF",
                    "single candidate source revision ref is missing".to_string(),
                )
            })?;
        let ir_ref = record.plan_candidate_ir_ref.clone().ok_or_else(|| {
            failure(
                self,
                "SOURCE_STORE_MALFORMED_REF",
                "single candidate IR ref is missing".to_string(),
            )
        })?;
        let report_ref = record.mechanical_report_ref.clone().ok_or_else(|| {
            failure(
                self,
                "SOURCE_STORE_MALFORMED_REF",
                "single candidate mechanical report ref is missing".to_string(),
            )
        })?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source = source_store
            .get_source_revision(&scope, &source_ref)
            .map_err(|error| {
                failure(
                    self,
                    error.code(),
                    format!("single candidate source reload failed: {error:?}"),
                )
            })?;
        let ir = source_store
            .get_plan_candidate_ir(&scope, &ir_ref)
            .map_err(|error| {
                failure(
                    self,
                    error.code(),
                    format!("single candidate IR reload failed: {error:?}"),
                )
            })?;
        let report = source_store
            .get_mechanical_report(&scope, &report_ref)
            .map_err(|error| {
                failure(
                    self,
                    error.code(),
                    format!("single candidate mechanical report reload failed: {error:?}"),
                )
            })?;
        verify_publish_freshness(&source.source, &ir.ir, &report.report).map_err(|error| {
            failure(
                self,
                "single_candidate_freshness_failed",
                format!("single candidate publish freshness failed: {error:?}"),
            )
        })?;

        let approval_attempt_id = single_candidate_approval_attempt_id(
            &record.id,
            &record.entity_id,
            &source_ref,
            &ir_ref,
            &report_ref,
        );
        let (approved_at, approved) = match (
            record.approval_attempt_id.as_deref(),
            record.approved_at.as_deref(),
        ) {
            (None, None) => {
                let approved_at = chrono::Utc::now().to_rfc3339();
                let approved = lifecycle
                    .compare_and_save_single_candidate_approval(
                        &record,
                        &approval_attempt_id,
                        &approved_at,
                    )
                    .map_err(|error| {
                        failure(
                            self,
                            "persistence_failure",
                            format!("save single candidate Approval failed: {error}"),
                        )
                    })?;
                (approved_at, approved)
            }
            (Some(existing_id), Some(existing_at))
                if existing_id == approval_attempt_id
                    && record.approval_attempt_id.as_deref()
                        == Some(approval_attempt_id.as_str()) =>
            {
                (existing_at.to_string(), record)
            }
            _ => {
                return Err(failure(
                    self,
                    "single_candidate_approval_conflict",
                    "single candidate Approval tuple conflicts with current durable refs"
                        .to_string(),
                ));
            }
        };
        #[cfg(test)]
        self.maybe_fail_single_candidate_compile(
            SingleCandidateCompileCheckpoint::ApprovalPersisted,
        )?;
        let reservation = crate::product::models::SingleCandidateCompileReservation {
            compile_id: single_candidate_compile_id(
                &approved.id,
                &approved.entity_id,
                &approval_attempt_id,
                &approved_at,
            ),
            now: approved_at.to_string(),
            publication_provenance_ref: String::new(),
        };
        let mut reservation = reservation;
        reservation.publication_provenance_ref = format!(
            "project/{}/issue/{}/plan/{}/publication_provenance/{}",
            approved.project_id, approved.issue_id, approved.entity_id, reservation.compile_id
        );
        let reserved = lifecycle
            .put_compile_reservation_cas(
                &approved.project_id,
                &approved.issue_id,
                &approved.entity_id,
                &approved.id,
                &approved,
                &reservation,
            )
            .map_err(|error| match error {
                CompileReservationError::Conflict => failure(
                    self,
                    "SINGLE_CANDIDATE_COMPILE_RESERVATION_CONFLICT",
                    error.to_string(),
                ),
                CompileReservationError::InvalidSession(message) => failure(
                    self,
                    "SINGLE_CANDIDATE_COMPILE_RESERVATION_INVALID_SESSION",
                    message,
                ),
                CompileReservationError::PersistenceFailure(error) => failure(
                    self,
                    "persistence_failure",
                    format!("save compile reservation failed: {error}"),
                ),
            })?;
        #[cfg(test)]
        self.maybe_fail_single_candidate_compile(
            SingleCandidateCompileCheckpoint::ReservationPersisted,
        )?;
        self.session.approval_attempt_id = reserved.approval_attempt_id.clone();
        self.session.approved_at = reserved.approved_at.clone();
        self.session.compile_reservation = reserved.compile_reservation.clone();
        self.session.publication_provenance_ref = reserved.publication_provenance_ref.clone();
        let reservation = reserved
            .compile_reservation
            .as_ref()
            .ok_or_else(|| "compile reservation disappeared after CAS".to_string())?;
        let previous_plan = lifecycle
            .get_issue_work_item_plan(&scope.project_id, &scope.issue_id, &scope.plan_id)
            .map_err(|error| format!("load plan for single candidate compile failed: {error}"))?;
        let logical_targets = self
            .logical_work_item_plan_repository_targets(&lifecycle, &previous_plan)
            .map_err(|error| failure(self, "single_candidate_target_failed", error))?;
        let repository_id = if logical_targets.is_none() {
            ir.ir
                .items
                .first()
                .map(|item| item.target_repository_id.clone())
                .ok_or_else(|| "single candidate IR contains no items".to_string())?
        } else {
            String::new()
        };
        let change_order = draft_batch::compile_support::load_change_order_from_confirmed_design(
            &lifecycle,
            &previous_plan,
        )
        .map_err(|error| failure(self, "single_candidate_change_order_failed", error))?;
        let context = ir_adapter::IrCompileAdapterContext {
            project_id: scope.project_id.clone(),
            issue_id: scope.issue_id.clone(),
            plan_id: scope.plan_id.clone(),
            previous_plan: previous_plan.clone(),
            source_revision_id: source.id.clone(),
            source_revision_ref: source_ref.clone(),
            plan_candidate_ir_ref: ir_ref.clone(),
            mechanical_report_ref: report_ref.clone(),
            publication_provenance_ref: reservation.publication_provenance_ref.clone(),
            logical_targets,
            repository_id,
            change_order,
            compile_id: reservation.compile_id.clone(),
            now: reservation.now.clone(),
        };
        let logical_ids = ir
            .ir
            .items
            .iter()
            .map(|item| item.contract.identity.logical_work_item_id.clone())
            .collect::<Vec<_>>();
        let allocated_ids =
            crate::product::work_item_revision_store::allocate_initial_plan_publication_ids(
                &scope.project_id,
                &scope.issue_id,
                &scope.plan_id,
                &reservation.compile_id,
                &logical_ids,
            )
            .map_err(|error| {
                failure(
                    self,
                    "single_candidate_publication_ids_failed",
                    error.to_string(),
                )
            })?;
        let mut provenance = PlanCandidatePublicationProvenance {
            id: reservation.compile_id.clone(),
            plan_id: scope.plan_id.clone(),
            plan_revision_id: allocated_ids.plan_revision_id,
            source_revision_ref: source_ref,
            plan_candidate_ir_ref: ir_ref,
            mechanical_report_ref: report_ref,
            source_revision_hash: source.source_revision_hash,
            compiler_version: ir.ir.compiler_version.clone(),
            published_at: reservation.now.clone(),
            content_hash: String::new(),
        };
        provenance.content_hash = provenance.content_hash().map_err(|error| {
            failure(
                self,
                error.code(),
                format!("hash provenance failed: {error:?}"),
            )
        })?;
        let provenance_ref = source_store
            .put_publication_provenance(
                &scope.project_id,
                &scope.issue_id,
                &scope.plan_id,
                &provenance,
            )
            .map_err(|error| {
                failure(
                    self,
                    error.code(),
                    format!("persist provenance failed: {error:?}"),
                )
            })?;
        if provenance_ref != reservation.publication_provenance_ref {
            return Err(failure(
                self,
                "single_candidate_provenance_ref_mismatch",
                "provenance canonical ref does not match reservation".to_string(),
            ));
        }
        let provenance = source_store
            .get_publication_provenance(&scope, &provenance_ref)
            .map_err(|error| {
                failure(
                    self,
                    error.code(),
                    format!("reload provenance failed: {error:?}"),
                )
            })?;
        #[cfg(test)]
        self.maybe_fail_single_candidate_compile(
            SingleCandidateCompileCheckpoint::ProvenancePersisted,
        )?;
        let durable_context = ir_adapter::durable_compile_context_from_ir(&context, &provenance)?;
        let input =
            ir_adapter::initial_plan_compile_input_from_ir(&context, &ir.ir, &report.report)?;
        let prepared = prepare_initial_plan_compile(input, durable_context)?;
        let stores = CompileStores {
            plan_store: self.work_item_plan_store()?,
            revision_store: self.revision_store(),
        };
        let (outcome, mut tx) = execute_initial_plan_compile(&stores, prepared)?;
        self.finalize_initial_plan_compile(&lifecycle, &stores.plan_store, &mut tx, &outcome)
            .await?;
        Ok(outcome)
    }

    pub(super) fn resume_single_candidate_compile_transaction(
        &mut self,
        store: &WorkItemPlanStore,
        tx: &mut WorkItemPlanCompileTransaction,
    ) -> Result<InitialPlanCompileOutcome, String> {
        let result = self.resume_single_candidate_compile_transaction_inner(store, tx);
        if let Err(error) = &result {
            let lifecycle = self.lifecycle_store.clone();
            if let Some(lifecycle) = lifecycle {
                self.record_single_candidate_compile_failure(
                    &lifecycle,
                    "single_candidate_recovery_failed",
                    error,
                );
            }
        }
        result
    }

    fn resume_single_candidate_compile_transaction_inner(
        &mut self,
        store: &WorkItemPlanStore,
        tx: &mut WorkItemPlanCompileTransaction,
    ) -> Result<InitialPlanCompileOutcome, String> {
        self.validate_single_candidate_transaction_refs_inner(tx)?;
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let scope = SourceStoreScope {
            project_id: tx.project_id.clone(),
            issue_id: tx.issue_id.clone(),
            plan_id: tx.plan_id.clone(),
        };
        let source_ref = tx
            .source_revision_ref
            .clone()
            .ok_or_else(|| "single candidate transaction source ref is missing".to_string())?;
        let ir_ref = tx
            .plan_candidate_ir_ref
            .clone()
            .ok_or_else(|| "single candidate transaction IR ref is missing".to_string())?;
        let report_ref = tx
            .mechanical_report_ref
            .clone()
            .ok_or_else(|| "single candidate transaction report ref is missing".to_string())?;
        let provenance_ref = tx
            .publication_provenance_ref
            .clone()
            .ok_or_else(|| "single candidate transaction provenance ref is missing".to_string())?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source = source_store
            .get_source_revision(&scope, &source_ref)
            .map_err(|error| format!("{}: transaction source reload failed", error.code()))?;
        let ir = source_store
            .get_plan_candidate_ir(&scope, &ir_ref)
            .map_err(|error| format!("{}: transaction IR reload failed", error.code()))?;
        let report = source_store
            .get_mechanical_report(&scope, &report_ref)
            .map_err(|error| format!("{}: transaction report reload failed", error.code()))?;
        verify_publish_freshness(&source.source, &ir.ir, &report.report)
            .map_err(|error| format!("single candidate freshness failed: {error:?}"))?;
        let provenance = source_store
            .get_publication_provenance(&scope, &provenance_ref)
            .map_err(|error| format!("{}: transaction provenance reload failed", error.code()))?;
        if tx.publication_provenance_content_hash.as_deref()
            != Some(provenance.content_hash.as_str())
            || provenance.plan_revision_id
                != crate::product::work_item_revision_store::allocate_initial_plan_publication_ids(
                    &scope.project_id,
                    &scope.issue_id,
                    &scope.plan_id,
                    &tx.compile_id,
                    &ir.ir
                        .items
                        .iter()
                        .map(|item| item.contract.identity.logical_work_item_id.clone())
                        .collect::<Vec<_>>(),
                )
                .map_err(|error| error.to_string())?
                .plan_revision_id
        {
            return Err("single candidate provenance identity mismatch".to_string());
        }
        let previous_plan = tx.previous_plan_snapshot.clone();
        let logical_targets =
            self.logical_work_item_plan_repository_targets(&lifecycle, &previous_plan)?;
        let repository_id = if logical_targets.is_none() {
            ir.ir
                .items
                .first()
                .map(|item| item.target_repository_id.clone())
                .ok_or_else(|| "single candidate IR contains no items".to_string())?
        } else {
            String::new()
        };
        let change_order = draft_batch::compile_support::load_change_order_from_confirmed_design(
            &lifecycle,
            &previous_plan,
        )?;
        let context = ir_adapter::IrCompileAdapterContext {
            project_id: scope.project_id,
            issue_id: scope.issue_id,
            plan_id: scope.plan_id,
            previous_plan,
            source_revision_id: source.id,
            source_revision_ref: source_ref,
            plan_candidate_ir_ref: ir_ref,
            mechanical_report_ref: report_ref,
            publication_provenance_ref: provenance_ref,
            logical_targets,
            repository_id,
            change_order,
            compile_id: tx.compile_id.clone(),
            now: tx.created_at.clone(),
        };
        let input =
            ir_adapter::initial_plan_compile_input_from_ir(&context, &ir.ir, &report.report)?;
        let durable_context = ir_adapter::durable_compile_context_from_ir(&context, &provenance)?;
        let prepared = prepare_initial_plan_compile(input, durable_context)?;
        let publication_input = prepared
            .publication_input
            .ok_or_else(|| "single candidate recovery publication input is missing".to_string())?;
        let journal = prepare_initial_plan_publication(publication_input)
            .map_err(|error| error.to_string())?;
        let outcome = publish_initial_plan_revision(&self.revision_store(), &journal)
            .map_err(|error| error.to_string())?;
        tx.outline_to_work_item_id = prepared
            .work_items
            .iter()
            .map(|work_item| {
                let outline_id = work_item.source_outline_id.clone().ok_or_else(|| {
                    format!(
                        "recovery work item `{}` has no source outline id",
                        work_item.id
                    )
                })?;
                Ok((outline_id, work_item.id.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        tx.outline_to_verification_plan_id = prepared
            .verification_plans
            .iter()
            .map(|verification_plan| {
                let outline_id = prepared
                    .work_items
                    .iter()
                    .find(|work_item| work_item.id == verification_plan.work_item_id)
                    .and_then(|work_item| work_item.source_outline_id.clone())
                    .ok_or_else(|| {
                        format!(
                            "recovery verification plan `{}` has no matching outline",
                            verification_plan.id
                        )
                    })?;
                Ok((outline_id, verification_plan.id.clone()))
            })
            .collect::<Result<BTreeMap<_, _>, String>>()?;
        tx.step_cursor = "publication_resumed".to_string();
        tx.updated_at = tx.created_at.clone();
        store
            .put_compile_transaction(tx)
            .map_err(|error| format!("save resumed publication cursor failed: {error}"))?;
        Ok(outcome)
    }

    pub(crate) fn validate_single_candidate_transaction_refs(
        &mut self,
        tx: &WorkItemPlanCompileTransaction,
    ) -> Result<(), String> {
        let result = self.validate_single_candidate_transaction_refs_inner(tx);
        if let Err(error) = &result {
            let lifecycle = self.lifecycle_store.clone();
            if let Some(lifecycle) = lifecycle {
                self.record_single_candidate_compile_failure(
                    &lifecycle,
                    "single_candidate_recovery_failed",
                    error,
                );
            }
        }
        result
    }

    fn validate_single_candidate_transaction_refs_inner(
        &self,
        tx: &WorkItemPlanCompileTransaction,
    ) -> Result<(), String> {
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let scope = SourceStoreScope {
            project_id: tx.project_id.clone(),
            issue_id: tx.issue_id.clone(),
            plan_id: tx.plan_id.clone(),
        };
        let source_ref = tx
            .source_revision_ref
            .as_deref()
            .ok_or_else(|| "single candidate transaction source ref is missing".to_string())?;
        let ir_ref = tx
            .plan_candidate_ir_ref
            .as_deref()
            .ok_or_else(|| "single candidate transaction IR ref is missing".to_string())?;
        let report_ref = tx
            .mechanical_report_ref
            .as_deref()
            .ok_or_else(|| "single candidate transaction report ref is missing".to_string())?;
        let provenance_ref = tx
            .publication_provenance_ref
            .as_deref()
            .ok_or_else(|| "single candidate transaction provenance ref is missing".to_string())?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        source_store
            .get_source_revision(&scope, source_ref)
            .map_err(|error| format!("{}: transaction source ref invalid", error.code()))?;
        source_store
            .get_plan_candidate_ir(&scope, ir_ref)
            .map_err(|error| format!("{}: transaction IR ref invalid", error.code()))?;
        source_store
            .get_mechanical_report(&scope, report_ref)
            .map_err(|error| format!("{}: transaction report ref invalid", error.code()))?;
        let provenance = source_store
            .get_publication_provenance(&scope, provenance_ref)
            .map_err(|error| format!("{}: transaction provenance ref invalid", error.code()))?;
        if tx.publication_provenance_content_hash.as_deref()
            != Some(provenance.content_hash.as_str())
        {
            return Err(
                "SOURCE_STORE_CONTENT_HASH_MISMATCH: transaction provenance hash differs"
                    .to_string(),
            );
        }
        Ok(())
    }

    fn record_single_candidate_compile_failure(
        &mut self,
        lifecycle: &LifecycleStore,
        code: &str,
        message: &str,
    ) {
        let Ok(record) = lifecycle.get_workspace_session(&self.session.session_id) else {
            return;
        };
        let mut diagnostics = record.policy_diagnostics.clone();
        diagnostics.push(PolicyDiagnostic {
            code: code.to_string(),
            message: message.to_string(),
            field: None,
        });
        if let Ok(saved) = lifecycle.compare_and_save_policy_route(
            &record,
            PolicyRoutePersist {
                status: WorkspaceSessionStatus::Failed,
                single_candidate_phase: Some(crate::product::models::SingleCandidatePhase::Failed),
                run_history: record.run_history.clone(),
                scope: record.review_invocation_scope.clone(),
                gate: record.human_gate_snapshot.clone(),
                diagnostics,
                repair_reservation: record.repair_reservation.clone(),
                provider_start_ledger: record.provider_start_ledger.clone(),
            },
        ) {
            self.session.session_status = saved.status;
            self.session.single_candidate_phase = saved.single_candidate_phase;
            self.session.policy_diagnostics = saved.policy_diagnostics;
        }
    }
}

#[cfg(test)]
fn single_candidate_compile_failpoints()
-> &'static std::sync::Mutex<std::collections::HashMap<SingleCandidateCompileFailpointKey, u64>> {
    SINGLE_CANDIDATE_COMPILE_FAILPOINTS
        .get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

#[cfg(test)]
impl Drop for SingleCandidateCompileFailpointGuard {
    fn drop(&mut self) {
        let mut failpoints = single_candidate_compile_failpoints()
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if failpoints.get(&self.key) == Some(&self.registration_id) {
            failpoints.remove(&self.key);
        }
    }
}
