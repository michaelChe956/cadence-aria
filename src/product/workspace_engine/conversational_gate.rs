use chrono::Utc;

use crate::product::coding_attempt_store::CodingAttemptStore;
use crate::product::coding_models::{CodingAttemptStatus, PlanAmendmentContext};
use crate::product::models::{
    HumanGateReservation, HumanGateTurn, HumanGateTurnStatus, SingleCandidatePhase,
    WorkspaceSessionStatus, WorkspaceType,
};
use crate::product::work_item_plan_compiler::grammar;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::web::workspace_ws_types::HumanConfirmDecision;

pub(crate) enum ScManualRevisionResult {
    Accepted { artifact_ref: String },
    ValidationRejected { diagnostics: Vec<String> },
}
pub(crate) const HUMAN_GATE_COMMAND_ID_MAX_BYTES: usize = 256;
pub(crate) const HUMAN_GATE_BUDGET_EXHAUSTED_CODE: &str = "HUMAN_GATE_BUDGET_EXHAUSTED";
/// Fixed upper bound for real provider starts belonging to one logical turn.
/// A turn is reserved as attempt 1 and may be resumed once as attempt 2.
pub(crate) const HUMAN_GATE_PROVIDER_MAX_ATTEMPTS: u32 = 2;

#[derive(Debug, Clone)]
pub(crate) struct HumanGateFeedbackInput {
    pub command_id: String,
    pub feedback: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HumanGateCommandOutcome {
    TurnOpened {
        turn: HumanGateTurn,
        remaining_budget: u32,
        prompt: String,
    },
    Busy {
        turn_id: String,
    },
    Replayed {
        turn: HumanGateTurn,
    },
    Rejected {
        code: String,
        reason: String,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum HumanGateCloseOutcome {
    Confirmed,
    Abandoned,
    Busy { turn_id: String },
}

fn rejected(code: &str, reason: impl Into<String>) -> HumanGateCommandOutcome {
    HumanGateCommandOutcome::Rejected {
        code: code.to_string(),
        reason: reason.into(),
    }
}

fn non_terminal(turn: &HumanGateTurn) -> bool {
    matches!(
        turn.status,
        HumanGateTurnStatus::Reserved | HumanGateTurnStatus::Running
    )
}

fn validate_command_id(command_id: &str) -> Result<(), String> {
    if command_id.trim().is_empty() {
        return Err("INVALID_COMMAND_ID: command_id must not be blank".to_string());
    }
    if command_id.len() > HUMAN_GATE_COMMAND_ID_MAX_BYTES {
        return Err(format!(
            "HUMAN_GATE_COMMAND_ID_TOO_LARGE: command_id exceeds {} bytes",
            HUMAN_GATE_COMMAND_ID_MAX_BYTES
        ));
    }
    Ok(())
}

fn validate_feedback(feedback: &str) -> Result<(), String> {
    super::prompts::validate_sc_manual_revision_feedback(feedback)
}

impl super::WorkspaceEngine {
    /// 校验当前 session 是否可在 amendment 上下文下重开原对话门（REQ-GCE-03）。
    /// 仅当：本 session 是单候选 WorkItemPlan、已过首次 approve（Confirmed +
    /// phase Completed）、门快照仍在（预算接续 manual_repairs_remaining）、存在
    /// 指向本 session 的 Open/Applying PlanAmendmentContext、且其 group attempt
    /// 处于 AwaitingPlanAmendment。任何一项不满足都保持既有 stage 拒绝路径。
    fn probe_amendment_gate_context(&self) -> Result<Option<PlanAmendmentContext>, String> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || self.session.stage == super::WorkspaceStage::HumanConfirm
        {
            return Ok(None);
        }
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let record = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        if record.status != WorkspaceSessionStatus::Confirmed
            || record.single_candidate_phase != Some(SingleCandidatePhase::Completed)
            || record.human_gate_snapshot.is_none()
        {
            return Ok(None);
        }
        let coding_store = CodingAttemptStore::new(store.app_paths());
        // 无 Open/Applying PlanAmendmentContext 时保持既有 stage 拒绝路径
        // (结构化 WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID),不上抛泛型 Err;否则
        // ws 层会把 7.2 之前结构化 Rejected 的同类输入映射成泛型 Error。
        let Some(context) = coding_store
            .find_open_plan_amendment_context_for_plan_session(
                &record.project_id,
                &record.issue_id,
                &record.entity_id,
                &record.id,
            )
            .map_err(|error| error.to_string())?
        else {
            return Ok(None);
        };
        let attempt = coding_store
            .get_attempt(
                &record.project_id,
                &record.issue_id,
                &context.group_attempt_id,
            )
            .map_err(|error| error.to_string())?;
        if attempt.status != CodingAttemptStatus::AwaitingPlanAmendment
            || attempt.work_item_group_id.as_deref() != Some(record.entity_id.as_str())
        {
            return Err(
                "group attempt is not awaiting plan amendment for this plan session".to_string(),
            );
        }
        Ok(Some(context))
    }

    pub(crate) fn build_sc_manual_revision_prompt_for_turn(
        &self,
        feedback: &str,
    ) -> Result<String, String> {
        let candidate_markdown = self
            .session
            .artifact
            .as_ref()
            .and_then(|artifact| artifact.markdown())
            // 批准链 compile 会把 current artifact 推成非 Markdown 投影；修订基线
            // 回落到最近一个 Markdown artifact version（批准时经 SC 流程
            // update_artifact(Markdown) 持久化的候选文本，语义上正是修订基线）。
            // 版本列表完全无 Markdown 时保持既有拒绝语义不变。
            .or_else(|| {
                self.artifact_versions
                    .iter()
                    .rev()
                    .find_map(|version| version.payload.markdown())
            })
            .ok_or_else(|| {
                "HUMAN_GATE_REVISION_CANDIDATE_MISSING: current candidate markdown is required"
                    .to_string()
            })?;
        let grammar_boundary =
            crate::product::work_item_split_engine::prompts::work_item_plan_markdown_grammar();
        super::prompts::build_sc_manual_revision_prompt(
            super::prompts::ScManualRevisionPromptInput {
                candidate_markdown,
                feedback,
                grammar_boundary: &grammar_boundary,
                language_rule: super::prompts::LANGUAGE_RULE_FILE_CONTENT,
            },
        )
    }
}

pub(crate) fn trim_provider_preamble(source: &str) -> &str {
    let document_heading = format!("{}\n", grammar::DOCUMENT_HEADING);
    source
        .find(&document_heading)
        .map(|offset| &source[offset..])
        .unwrap_or(source)
}

impl super::WorkspaceEngine {
    pub(crate) fn mark_human_gate_turn_running(&mut self, turn_id: &str) -> Result<(), String> {
        use crate::product::models::HumanGateTurnStatus;
        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let mut turn = store
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| error.to_string())?;
        if turn.status == HumanGateTurnStatus::Running {
            return Ok(());
        }
        if turn.status != HumanGateTurnStatus::Reserved {
            return Err(format!("human gate turn {turn_id} is not reservable"));
        }
        turn.status = HumanGateTurnStatus::Running;
        turn.updated_at = Utc::now().to_rfc3339();
        let saved = store
            .update_human_gate_turn(&expected, turn)
            .map_err(|error| error.to_string())?;
        self.session.provider_start_ledger = saved.provider_start_ledger;
        Ok(())
    }

    pub(crate) async fn fail_human_gate_turn(
        &mut self,
        turn_id: &str,
        failure_class: crate::product::models::HumanGateTurnFailureClass,
    ) -> Result<(), String> {
        use crate::product::models::HumanGateTurnStatus;
        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let mut turn = store
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| error.to_string())?;
        if matches!(
            turn.status,
            HumanGateTurnStatus::Completed | HumanGateTurnStatus::Failed
        ) {
            return Ok(());
        }
        turn.status = HumanGateTurnStatus::Failed;
        turn.failure_class = Some(failure_class);
        turn.updated_at = Utc::now().to_rfc3339();
        let saved = store
            .update_human_gate_turn(&expected, turn)
            .map_err(|error| error.to_string())?;
        self.session.provider_start_ledger = saved.provider_start_ledger;
        Ok(())
    }

    pub(crate) async fn run_sc_manual_revision_turn(
        &mut self,
        turn_id: &str,
        provider_output: String,
    ) -> Result<ScManualRevisionResult, String> {
        use crate::product::models::{HumanGateTurnFailureClass, HumanGateTurnStatus};
        use crate::product::work_item_plan_compiler::{
            PlanCandidateValidationContext, WorkItemPlanSourceContext, compile_work_item_plan,
            validate_plan_candidate_ir,
        };
        use crate::product::work_item_plan_source_store::{
            PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
            WorkItemPlanSourceStore,
        };
        use sha2::{Digest, Sha256};

        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let mut expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        let turn = lifecycle
            .get_human_gate_turn(&self.session.session_id, turn_id)
            .map_err(|error| error.to_string())?;
        if turn.status != HumanGateTurnStatus::Running
            && turn.status != HumanGateTurnStatus::Reserved
        {
            return Err(format!("human gate turn {turn_id} is not active"));
        }
        let source = trim_provider_preamble(&provider_output).to_string();
        let plan = lifecycle
            .get_issue_work_item_plan(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
            )
            .map_err(|error| format!("load plan for human gate revision failed: {error}"))?;
        let repository_id = self.work_item_plan_repository_id(&lifecycle, &plan)?;
        let repository_profile = plan
            .repository_profile_ref
            .as_deref()
            .map(|profile_id| {
                lifecycle.get_repository_profile(
                    &self.session.project_id,
                    &self.session.issue_id,
                    profile_id,
                )
            })
            .transpose()
            .map_err(|error| {
                format!("load repository profile for human gate revision failed: {error}")
            })?;

        let ir = match compile_work_item_plan(
            &source,
            &WorkItemPlanSourceContext {
                target_repository_id: repository_id,
            },
        ) {
            Ok(ir) => ir,
            Err(diagnostics) => {
                let messages = diagnostics
                    .iter()
                    .map(|diagnostic| {
                        format!(
                            "{}:{}:{}",
                            diagnostic.code, diagnostic.line, diagnostic.message
                        )
                    })
                    .collect();
                let mut failed = turn.clone();
                failed.status = HumanGateTurnStatus::Failed;
                failed.failure_class = Some(HumanGateTurnFailureClass::ValidationReject);
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                expected = lifecycle
                    .update_human_gate_turn(&expected, failed)
                    .map_err(|error| error.to_string())?;
                self.session.provider_start_ledger = expected.provider_start_ledger;
                return Ok(ScManualRevisionResult::ValidationRejected {
                    diagnostics: messages,
                });
            }
        };
        let validation_now = chrono::Utc::now().to_rfc3339();
        let report = match validate_plan_candidate_ir(
            &ir,
            &PlanCandidateValidationContext {
                project_id: &self.session.project_id,
                issue_id: &self.session.issue_id,
                plan_id: &self.session.entity_id,
                source_story_spec_ids: &plan.source_story_spec_ids,
                source_design_spec_ids: &plan.source_design_spec_ids,
                repository_profile: repository_profile.as_ref(),
                now: &validation_now,
            },
        ) {
            Ok(report) => report,
            Err(diagnostics) => {
                let messages = diagnostics
                    .iter()
                    .map(|diagnostic| {
                        format!(
                            "{}:{}:{}",
                            diagnostic.code, diagnostic.line, diagnostic.message
                        )
                    })
                    .collect();
                let mut failed = turn.clone();
                failed.status = HumanGateTurnStatus::Failed;
                failed.failure_class = Some(HumanGateTurnFailureClass::ValidationReject);
                failed.updated_at = chrono::Utc::now().to_rfc3339();
                expected = lifecycle
                    .update_human_gate_turn(&expected, failed)
                    .map_err(|error| error.to_string())?;
                self.session.provider_start_ledger = expected.provider_start_ledger;
                return Ok(ScManualRevisionResult::ValidationRejected {
                    diagnostics: messages,
                });
            }
        };

        let source_hash = hex::encode(Sha256::digest(source.as_bytes()));
        let source_id = format!("source-{}", &source_hash[..16]);
        let mut source_record = SourceRevisionRecord {
            id: source_id.clone(),
            source: source.clone(),
            source_revision_hash: source_hash.clone(),
            content_hash: String::new(),
        };
        source_record.content_hash = source_record
            .content_hash()
            .map_err(|error| error.code().to_string())?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source_ref = source_store
            .put_source_revision(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
                &source_record,
            )
            .map_err(|error| error.code().to_string())?;
        let ir_id = format!("ir-{}", &source_hash[..16]);
        let mut ir_record = PlanCandidateIrRecord {
            id: ir_id.clone(),
            source_revision_id: source_id.clone(),
            ir,
            content_hash: String::new(),
        };
        ir_record.content_hash = ir_record
            .content_hash()
            .map_err(|error| error.code().to_string())?;
        let ir_ref = source_store
            .put_plan_candidate_ir(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
                &ir_record,
            )
            .map_err(|error| error.code().to_string())?;
        let report_id = format!("report-{}", &source_hash[..16]);
        let mut report_record = PlanCandidateMechanicalReportRecord {
            id: report_id,
            source_revision_id: source_id,
            ir_id,
            report,
            content_hash: String::new(),
        };
        report_record.content_hash = report_record
            .content_hash()
            .map_err(|error| error.code().to_string())?;
        let report_ref = source_store
            .put_mechanical_report(
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
                &report_record,
            )
            .map_err(|error| error.code().to_string())?;
        let mut completed = turn;
        completed.status = HumanGateTurnStatus::Completed;
        completed.failure_class = None;
        let artifact_versions = {
            let mut versions = self.artifact_versions.clone();
            for version in &mut versions {
                version.is_current = false;
            }
            let version = versions.len() as u32 + 1;
            versions.push(crate::web::workspace_ws_types::ArtifactVersion {
                version,
                payload: crate::web::workspace_ws_types::ArtifactPayload::Markdown {
                    markdown: source.clone(),
                    diff: None,
                },
                generated_by: self.session.author_provider.clone(),
                reviewed_by: None,
                review_verdict: None,
                confirmed_by: None,
                is_current: true,
                created_at: chrono::Utc::now().to_rfc3339(),
                source_node_id: self
                    .active_node_id
                    .clone()
                    .unwrap_or_else(|| "timeline_node_unknown".to_string()),
            });
            versions
        };
        let artifact_id = format!(
            "artifact_version_{:03}",
            artifact_versions
                .last()
                .map(|version| version.version)
                .unwrap_or(0)
        );
        let saved = lifecycle
            .complete_human_gate_revision(
                &expected,
                completed,
                &source_ref,
                &ir_ref,
                &report_ref,
                &artifact_versions,
                &artifact_id,
            )
            .map_err(|error| error.to_string())?;
        self.artifact_versions = artifact_versions;
        self.session.artifact = Some(crate::web::workspace_ws_types::ArtifactPayload::Markdown {
            markdown: source,
            diff: None,
        });
        if let Some(node_id) = self.active_node_id.clone() {
            let _ = self
                .persist_artifact_ref(
                    &node_id,
                    crate::product::models::ArtifactRef {
                        artifact_id: artifact_id.clone(),
                        version: self
                            .artifact_versions
                            .last()
                            .map(|version| version.version)
                            .unwrap_or(0),
                    },
                )
                .await;
        }
        let _ = self
            .event_tx
            .send(super::EngineEvent::ArtifactUpdate {
                version: self
                    .artifact_versions
                    .last()
                    .map(|version| version.version)
                    .unwrap_or(0),
                payload: self.session.artifact.clone().expect("artifact set above"),
            })
            .await;
        self.session = super::WorkspaceSession::from_record(saved);
        if let Some(current_version) = self
            .artifact_versions
            .iter()
            .find(|version| version.is_current)
        {
            self.session.artifact = Some(current_version.payload.clone());
        }
        // 人工修订与初始 author 同构：候选落盘后必须重走 Evaluate policy route。
        // 缺失这一步时 session 停留 Evaluate，confirm 的 `compare_and_save_human_gate_close`
        // 前置(WaitingForHuman+Approval)永久冲突，门死锁。无 reviewer 走本地
        // synthetic Pass 路由进 Approval；有 reviewer 重启评审，不让 close 绕过 Approval。
        if self.session.review_rounds == 0 || self.session.reviewer_provider.is_none() {
            self.route_single_candidate_evaluate_without_reviewer()
                .await;
        } else {
            self.start_review().await;
            if self.session.stage == super::WorkspaceStage::CrossReview {
                self.request_provider_run(super::ProviderRunKind::ReviewOnly)
                    .await;
            }
        }
        Ok(ScManualRevisionResult::Accepted {
            artifact_ref: artifact_id,
        })
    }

    pub(crate) async fn handle_human_gate_feedback(
        &mut self,
        input: HumanGateFeedbackInput,
    ) -> Result<HumanGateCommandOutcome, String> {
        // A replay is intentionally checked before stage and budget checks so
        // reconnects can safely resend a command after the session advanced.
        if let Err(error) = validate_command_id(&input.command_id) {
            let (code, reason) = error
                .split_once(':')
                .map_or(("INVALID_COMMAND_ID", error.as_str()), |(code, reason)| {
                    (code, reason.trim())
                });
            return Ok(rejected(code, reason));
        }
        let store = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        if let Some(turn) = store
            .get_human_gate_turn_by_command_id(&self.session.session_id, &input.command_id)
            .map_err(|error| error.to_string())?
        {
            return Ok(HumanGateCommandOutcome::Replayed { turn });
        }

        // REQ-GCE-03 场景二：attempt 处于 AwaitingPlanAmendment 期间，原 SC plan
        // session 以 amendment 上下文重开同一人工门接受 typed feedback。stage 不在
        // HumanConfirm 时先探测并校验 session/plan lineage 与 PlanAmendmentContext，
        // 命中才放行；否则保持既有 stage 拒绝语义（零副作用）。
        let amendment_gate = self.probe_amendment_gate_context()?;
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || (self.session.stage != super::WorkspaceStage::HumanConfirm
                && amendment_gate.is_none())
        {
            return Ok(rejected(
                "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
                "human gate feedback is only available for a single-candidate work-item plan in human_confirm",
            ));
        }
        if let Err(error) = validate_feedback(&input.feedback) {
            let (code, reason) = error.split_once(':').map_or(
                ("INVALID_HUMAN_GATE_FEEDBACK", error.as_str()),
                |(code, reason)| (code, reason.trim()),
            );
            return Ok(rejected(code, reason));
        }

        let turns = store
            .list_human_gate_turns(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        if let Some(turn) = turns.iter().find(|turn| non_terminal(turn)) {
            return Ok(HumanGateCommandOutcome::Busy {
                turn_id: turn.turn_id.clone(),
            });
        }

        let remaining_budget = self
            .session
            .human_gate_snapshot
            .as_ref()
            .map(|snapshot| snapshot.manual_repairs_remaining)
            .ok_or_else(|| "human gate snapshot is missing".to_string())?;
        if remaining_budget == 0 {
            return Ok(rejected(
                HUMAN_GATE_BUDGET_EXHAUSTED_CODE,
                "manual repair budget is exhausted",
            ));
        }

        // 构造完整 SC revision prompt 必须发生在 HumanGateTurn CAS 之前。这样候选或
        // 固定契约超出独立预算时，反馈请求只返回 bounded error，不消耗预算/ledger。
        let prompt = match self.build_sc_manual_revision_prompt_for_turn(&input.feedback) {
            Ok(prompt) => prompt,
            Err(error) => {
                let (code, reason) = error.split_once(':').map_or(
                    ("HUMAN_GATE_REVISION_PROMPT_TOO_LARGE", error.as_str()),
                    |(code, reason)| (code, reason.trim()),
                );
                return Ok(rejected(code, reason));
            }
        };

        let now = Utc::now().to_rfc3339();
        let source_hash = self
            .session
            .artifact
            .as_ref()
            .map(|artifact| {
                use sha2::{Digest, Sha256};
                hex::encode(Sha256::digest(artifact.markdown_or_empty().as_bytes()))
            })
            .unwrap_or_default();
        let turn = HumanGateTurn {
            turn_id: format!("human_gate_turn_{}", uuid::Uuid::new_v4()),
            session_id: self.session.session_id.clone(),
            command_id: input.command_id.clone(),
            feedback_text: input.feedback,
            status: HumanGateTurnStatus::Reserved,
            attempt_no: 1,
            budget_reserved: 1,
            source_hash,
            result_artifact_ref: None,
            failure_class: None,
            created_at: now.clone(),
            updated_at: now.clone(),
        };
        let reservation = HumanGateReservation {
            command_id: turn.command_id.clone(),
            turn_id: turn.turn_id.clone(),
            provider_start_idempotency_key: format!("human_gate:{}:attempt:1", turn.turn_id),
            reserved_at: now,
        };
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        // Re-check the durable status immediately before the CAS.  A websocket
        // may have retained a fresh-looking in-memory stage while another
        // worker already advanced the session; reserving a turn in that stale
        // window would reopen a closed gate.
        let expected =
            if expected.status == WorkspaceSessionStatus::Confirmed && amendment_gate.is_some() {
                // amendment 上下文重开：CAS Confirmed→WaitingForHuman，门快照与预算
                // 原样保留（D11 单一预算源），不新建第二个门实例。
                match store.compare_and_reopen_amendment_gate(&expected) {
                    Ok(saved) => {
                        self.session.session_status = saved.status.clone();
                        self.session.human_gate_snapshot = saved.human_gate_snapshot.clone();
                        saved
                    }
                    Err(_) => {
                        return Ok(rejected(
                            "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
                            "amendment gate reopen lost the durable race",
                        ));
                    }
                }
            } else {
                expected
            };
        if expected.status != WorkspaceSessionStatus::WaitingForHuman {
            return Ok(rejected(
                "WORK_ITEM_PLAN_HUMAN_GATE_STAGE_INVALID",
                "human gate feedback requires a waiting_for_human session",
            ));
        }
        let (saved, saved_turn) =
            match store.compare_and_reserve_human_gate_turn(&expected, turn, reservation) {
                Ok(result) => result,
                Err(error) => {
                    // Another websocket worker may have won the CAS. Reconcile
                    // from disk and report the durable single-flight owner.
                    if let Some(existing) = store
                        .get_human_gate_turn_by_command_id(
                            &self.session.session_id,
                            &input.command_id,
                        )
                        .map_err(|read_error| read_error.to_string())?
                    {
                        return Ok(HumanGateCommandOutcome::Replayed { turn: existing });
                    }
                    if let Some(existing) = store
                        .list_human_gate_turns(&self.session.session_id)
                        .map_err(|read_error| read_error.to_string())?
                        .into_iter()
                        .find(non_terminal)
                    {
                        return Ok(HumanGateCommandOutcome::Busy {
                            turn_id: existing.turn_id,
                        });
                    }
                    return Err(error.to_string());
                }
            };
        self.session.human_gate_snapshot = saved.human_gate_snapshot;
        self.session.provider_start_ledger = saved.provider_start_ledger;
        Ok(HumanGateCommandOutcome::TurnOpened {
            turn: saved_turn,
            remaining_budget: remaining_budget - 1,
            prompt,
        })
    }

    pub(crate) async fn handle_human_gate_termination(
        &mut self,
        decision: HumanConfirmDecision,
    ) -> Result<HumanGateCloseOutcome, String> {
        self.close_human_gate(decision).await
    }

    /// Atomically closes the single-candidate human gate. Approval enters the
    /// existing deterministic compile path and only becomes Confirmed after
    /// that path durably publishes the plan; termination is terminal without
    /// creating a compile transaction.
    pub(crate) async fn close_human_gate(
        &mut self,
        decision: HumanConfirmDecision,
    ) -> Result<HumanGateCloseOutcome, String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
            || self.session.stage != super::WorkspaceStage::HumanConfirm
        {
            return Err("human gate close is only available for a single-candidate work-item plan in human_confirm".to_string());
        }

        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| error.to_string())?;
        if expected.status != WorkspaceSessionStatus::WaitingForHuman {
            return Err("human gate close requires a waiting_for_human session".to_string());
        }
        if let Some(turn) = lifecycle
            .list_human_gate_turns(&self.session.session_id)
            .map_err(|error| error.to_string())?
            .into_iter()
            .find(non_terminal)
        {
            return Ok(HumanGateCloseOutcome::Busy {
                turn_id: turn.turn_id,
            });
        }

        match decision {
            HumanConfirmDecision::Confirm => {
                let saved = lifecycle
                    .compare_and_save_human_gate_close(
                        &expected,
                        WorkspaceSessionStatus::Running,
                    )
                    .map_err(|error| error.to_string())?;
                self.session.session_status = saved.status;
                self.session.human_gate_snapshot = saved.human_gate_snapshot;
                self.enter_policy_valid_work_item_plan_compile().await;

                let durable = lifecycle
                    .get_workspace_session(&self.session.session_id)
                    .map_err(|error| error.to_string())?;
                self.session.session_status = durable.status.clone();
                self.session.single_candidate_phase = durable.single_candidate_phase.clone();
                self.session.human_gate_snapshot = durable.human_gate_snapshot.clone();
                if durable.status != WorkspaceSessionStatus::Confirmed
                    || durable.single_candidate_phase
                        != Some(crate::product::models::SingleCandidatePhase::Completed)
                {
                    return Err(
                        "single-candidate approval compile failed; human gate remains open"
                            .to_string(),
                    );
                }
                let _ = self
                    .event_tx
                    .send(super::EngineEvent::HumanGateClosed {
                        decision: "confirm".to_string(),
                        stage: self.session.stage.as_str().to_string(),
                    })
                    .await;
                Ok(HumanGateCloseOutcome::Confirmed)
            }
            HumanConfirmDecision::Terminate => {
                let saved = lifecycle
                    .compare_and_save_human_gate_close(
                        &expected,
                        WorkspaceSessionStatus::Terminated,
                    )
                    .map_err(|error| error.to_string())?;
                self.session.session_status = saved.status;
                self.session.human_gate_snapshot = saved.human_gate_snapshot;
                let terminal_stage = super::WorkspaceStage::Completed;
                self.session.stage = terminal_stage.clone();
                let _ = self
                    .event_tx
                    .send(super::EngineEvent::HumanGateClosed {
                        decision: "terminate".to_string(),
                        stage: terminal_stage.as_str().to_string(),
                    })
                    .await;
                self.complete_active_node(Some("已终止".to_string())).await;
                self.transition_stage(terminal_stage).await;
                let _ = self
                    .create_timeline_node(super::TimelineNodeDraft {
                        node_type: super::TimelineNodeType::Completed,
                        agent: None,
                        stage: super::WorkspaceStage::Completed,
                        round: None,
                        title: "流程终止".to_string(),
                        summary: Some("已终止".to_string()),
                        status: super::TimelineNodeStatus::Completed,
                    })
                    .await;
                Ok(HumanGateCloseOutcome::Abandoned)
            }
            HumanConfirmDecision::RequestChange => Err(
                "single-candidate human gate does not support request-change; submit feedback through HumanGateFeedback"
                    .to_string(),
            ),
        }
    }
}
