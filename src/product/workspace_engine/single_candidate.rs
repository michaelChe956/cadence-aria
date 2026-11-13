use sha2::{Digest, Sha256};

use super::lifecycle::NativeResumeCandidateError;
use super::*;
use crate::cross_cutting::provider_adapter::ProviderAdapterError;
use crate::product::models::{
    LcRecoveryBusinessContext, LcRecoveryRunContext, LcResumeRecoveryRecord, LcResumeRecoveryState,
    ProviderName,
};
use crate::product::work_item_plan_compiler::{
    PlanCandidateValidationContext, WorkItemPlanSourceContext, validate_plan_candidate_ir,
};
use crate::product::work_item_plan_policy::{
    ScTeachingAttemptLifecycle, ScTeachingAttemptRef, ScTeachingWindowStatus, WorkItemPlanFlowKind,
    next_sc_author_launch_ordinal,
};
use crate::product::work_item_plan_source_store::{
    PlanCandidateIrRecord, PlanCandidateMechanicalReportRecord, SourceRevisionRecord,
    WorkItemPlanSourceStore,
};
use crate::product::workspace_engine::draft_batch::member_checkout_prefixes;
use crate::web::workspace_ws_types::WorkItemGenerationModeDto;
use crate::web::workspace_ws_types::{ReviewGate, ReviewVerdict, ReviewVerdictType};

/// 作者阶段候选落盘的收尾形态（REQ-SG-02/REQ-WSC-18）：
///
/// - [`Self::Candidate`]：候选合法并通过结构门，已提升 current refs 并进入
///   既有评审/确认链（`item_count` 为编译出的候选 item 数）；
/// - [`Self::Rejected { failure, structure_failures }`]：候选被 SG3a.1 系统门拒收且
///   调用方传入 `allow_structure_rerun=true`（Task 6 起恒 true：重驱资格
///   由调用方消费同一耐久教学窗口的 claim 结果决定，不再有进程内独立
///   槽位）——durable 诊断 + 尝试流水已落盘；**sealed 的原失败身份**
///   （kind／stable_code／failure_id／detail 逐字不变）与**原 typed 结构失败集**
///   （多成员不丢、序不变）随本形态上交；修复指令由 runner 用既有
///   `author_structure_repair_prompt(&failure)` 生成（引擎侧不再提前造 prompt）；
///   不开人工门；
/// - [`Self::Routed`]：拒收且调用方判定不再重驱（`allow_structure_rerun=false`）
///   ——平台已在自动重写或人工门态，调用方**不得**再把它当硬失败。
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SingleCandidateAuthorCompletion {
    Candidate { item_count: usize },
    Rejected {
        /// 已 seal 的原失败身份（kind／stable_code／failure_id／detail 逐字不变）。
        failure: crate::product::workspace_engine::types::PlanningCompileFailure,
        /// 原 typed 结构失败集（原样透传，不重算、不从文本反解析）。
        structure_failures: crate::product::planning_failure::PlanningFailureSet,
    },
    Routed,
}

/// 仅由已编译 IR 的 item 数与服务端 provider profile 驱动的内部诊断输入。
///
/// 该输入不通过 WebSocket 反序列化；客户端不能提交候选数或 mode。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SingleCandidateGenerationDecisionInput {
    pub provider: ProviderName,
    pub candidate_item_count: usize,
}

/// 在完整 markdown 已编译为 IR 后，确定性记录内部 generation mode。
///
/// 这是诊断，不是 WS/OpenSpec 对外契约；相同 IR item 数与 provider profile 始终产生
/// 相同结果。Pi/Fake profile 保守记录 serial，其余 profile 在三项以内记录 batch。
pub(crate) fn select_internal_generation_mode(
    input: &SingleCandidateGenerationDecisionInput,
) -> WorkItemGenerationModeDto {
    if input.candidate_item_count <= 3
        && !matches!(input.provider, ProviderName::Pi | ProviderName::Fake)
    {
        WorkItemGenerationModeDto::Batch
    } else {
        WorkItemGenerationModeDto::Serial
    }
}

/// B2c（Task 5.5，设计 §5.3 SC 纪律①）：SC author native mismatch 的稳定
/// 等待判别码——与 gateway 冻结绑定家族 `RESUME_FINGERPRINT_UNAVAILABLE`
/// 同码（SC author 的 mismatch 即旧 native 在而冻结绑定不可用/已吊销）。
pub(crate) const SC_AUTHOR_NATIVE_MISMATCH_WAITING_REASON: &str =
    crate::product::logical_codebase::provider_gateway::RESUME_FINGERPRINT_UNAVAILABLE;

impl WorkspaceEngine {
    pub(crate) fn persist_single_candidate_terminal_phase(
        &mut self,
        phase: crate::product::models::SingleCandidatePhase,
    ) {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return;
        }
        let Some(lifecycle) = self.lifecycle_store.as_ref() else {
            self.session.single_candidate_phase = Some(phase);
            return;
        };
        let Ok(expected) = lifecycle.get_workspace_session(&self.session.session_id) else {
            return;
        };
        let status = match phase {
            crate::product::models::SingleCandidatePhase::Completed => {
                crate::product::models::WorkspaceSessionStatus::Confirmed
            }
            crate::product::models::SingleCandidatePhase::Failed => {
                crate::product::models::WorkspaceSessionStatus::Failed
            }
            _ => return,
        };
        if let Ok(saved) =
            lifecycle.compare_and_save_single_candidate_phase(&expected, phase, status)
        {
            self.session.single_candidate_phase = saved.single_candidate_phase;
            self.session.session_status = saved.status;
        }
    }

    /// C-1：LC admission waiting 投影——非终态相位回落 Prepare + Open（可操作
    /// 等待面，与 `rearm_failed_single_candidate_for_start_generation` 的重开
    /// 形态同族：用户补齐成员材料后可直接再次发起生成）。waiting 不是失败：
    /// 终态（Completed/Failed）保持自身不可重放；CAS 失败静默（迟到 run 让位
    /// 持有者，不产生虚假投影）。
    pub(crate) fn persist_single_candidate_waiting_phase(&mut self) {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return;
        }
        let Some(lifecycle) = self.lifecycle_store.as_ref() else {
            self.session.single_candidate_phase =
                Some(crate::product::models::SingleCandidatePhase::Prepare);
            return;
        };
        let Ok(expected) = lifecycle.get_workspace_session(&self.session.session_id) else {
            return;
        };
        if matches!(
            expected.single_candidate_phase,
            Some(
                crate::product::models::SingleCandidatePhase::Completed
                    | crate::product::models::SingleCandidatePhase::Failed
            )
        ) {
            return;
        }
        if let Ok(saved) = lifecycle.compare_and_save_single_candidate_phase(
            &expected,
            crate::product::models::SingleCandidatePhase::Prepare,
            crate::product::models::WorkspaceSessionStatus::Open,
        ) {
            self.session.single_candidate_phase = saved.single_candidate_phase;
            self.session.session_status = saved.status;
        }
    }

    /// 先以 durable ledger 保留 provider start，再允许 SingleCandidate author 启动。
    pub(crate) fn reserve_single_candidate_author_start(&mut self) -> Result<bool, String> {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return Err(
                "single candidate provider reservation requires SingleCandidate flow".to_string(),
            );
        }
        let lifecycle = self
            .lifecycle_store
            .as_ref()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| format!("load session for provider reservation failed: {error}"))?;
        // Task 5（序 3，tasks 1.2 迁移面）：删除 `phase==Generate ? len-1 :
        // len` 位置启发式——author 启动键不再由「全 ledger 长度/末位条目」
        // 隐式派生。优先级：
        // - A 接力：`repair_reservation` 是 A 池显式载体，预领键按**原键**
        //   精确消费（Reserved 推进 ProviderStarted 放行；已启动同键幂等
        //   让位，k3 单飞语义不变）；
        // - Generate 相位的原调用重入：按当前业务键（最后一个非教学前缀键）
        //   精确匹配幂等（迟到 run 让位，不把 C 教学占位误当业务键）；
        // - 其余（新业务启动）：键序号 = next_sc_author_launch_ordinal（前缀
        //   最大数字后缀 + 1，序号单调、与人门键及 C 教学条目计数解耦）。
        let key = if let Some(reservation) = expected.repair_reservation.as_ref() {
            reservation.provider_start_idempotency_key.clone()
        } else if expected.single_candidate_phase
            == Some(crate::product::models::SingleCandidatePhase::Generate)
            && let Some(current) = current_sc_author_business_key(&expected)
        {
            current
        } else {
            format!(
                "single_candidate_author:{}:{}",
                expected.id,
                next_sc_author_launch_ordinal(&expected)
            )
        };
        let (saved, should_start) = lifecycle
            .reserve_single_candidate_provider_start(&expected, &key)
            .map_err(|error| format!("persist provider reservation failed: {error}"))?;
        self.reload_session_from_record(saved);
        Ok(should_start)
    }

    /// Task 4（REQ-WSC-28C，tasks 2.3 接线面）：教学重驱真实启动的同步
    /// 回填。调用契约：web 层 SC 生成 run 驱动入口（`drive_single_candidate_
    /// reredrive`）在 `start_work_item_plan_author` 返回——spawn 身份已由
    /// adapter/gateway 内 `confirm_started_resource` 链确认、`ProviderSession`
    /// 已拿到——之后、进入事件驱动循环之前的窗口内调用。
    ///
    /// - `None`（非教学 run）原样透传（零 durable 副作用）；
    /// - 启动 `Err`（如治理层 full fingerprint 漂移拒启）：无会话可回填，
    ///   确定未启动——attempt 保持 Claimed（design D2），原样上浮；
    /// - 回填成功 ⇒ attempt → ActualStarted、占位条目**原位**补全
    ///   （provider = 会话 author_provider 名，started_at = 当前 RFC3339 UTC）；
    /// - 回填失败 ⇒ 先持久化 `mark_sc_teaching_attempt_unknown`
    ///   （LaunchUnknown，fail-closed 证据保留），再把 Ok 换成带稳定原因码
    ///   的 Err 交既有失败收口停止该 run（不自动重播）。
    pub(crate) fn backfill_sc_teaching_launch(
        &mut self,
        sc_teaching_attempt: Option<&ScTeachingAttemptRef>,
        session: Result<ProviderSession, ProviderAdapterError>,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let Some(attempt) = sc_teaching_attempt else {
            return session;
        };
        let session = session?; // 启动失败：无会话可回填（占位保持 Claimed）。
        match self.observe_sc_teaching_provider_launch(attempt) {
            Ok(()) => Ok(session),
            Err(message) => Err(ProviderAdapterError::provider_unavailable(format!(
                "sc_teaching_launch_backfill_failed: {message}"
            ))),
        }
    }

    /// `backfill_sc_teaching_launch` 的耐久内核：锁内 CAS observe 原位补全
    /// 占位条目；失败路径先尽力迁移 LaunchUnknown（迁移自身失败只留告警——
    /// 会话尾迁移与本错误消息共同保留证据）。
    pub(crate) fn observe_sc_teaching_provider_launch(
        &mut self,
        attempt: &ScTeachingAttemptRef,
    ) -> Result<(), String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                format!(
                    "load session for sc teaching launch backfill failed ({}): {error:?}",
                    self.session.session_id
                )
            })?;
        // provider 文本取 serde snake_case 投影（与 F-19 ledger 登记同口径）。
        let provider_text = serde_json::to_value(&self.session.author_provider)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_else(|| "unknown".to_string());
        let started_at = chrono::Utc::now().to_rfc3339();
        match lifecycle.observe_sc_teaching_provider_start(
            &expected,
            attempt.generation_seq,
            attempt.attempt_no,
            &provider_text,
            started_at,
        ) {
            Ok(saved) => {
                self.reload_session_from_record(saved);
                Ok(())
            }
            Err(error) => {
                tracing::error!(
                    session_id = %self.session.session_id,
                    attempt_key = %attempt.attempt_key,
                    error = ?error,
                    "sc teaching provider start backfill failed; marking attempt LaunchUnknown"
                );
                if let Ok(fresh) = lifecycle.get_workspace_session(&self.session.session_id) {
                    match lifecycle.mark_sc_teaching_attempt_unknown(
                        &fresh,
                        attempt.generation_seq,
                        attempt.attempt_no,
                    ) {
                        Ok(saved) => self.reload_session_from_record(saved),
                        Err(mark_error) => tracing::warn!(
                            session_id = %self.session.session_id,
                            attempt_key = %attempt.attempt_key,
                            error = ?mark_error,
                            "sc teaching attempt unknown migration failed (evidence stays in error path)"
                        ),
                    }
                }
                Err(format!(
                    "observe sc teaching provider start failed ({}): {error:?}",
                    attempt.attempt_key
                ))
            }
        }
    }

    /// Task 4（tasks 2.3）：会话尾收口——SC 生成 run 任一出口收尾时，把
    /// Active 窗口内仍在 `Claimed`（已 claim 未回填）的教学 attempt 持久化
    /// 为 `LaunchUnknown`（幂等；无窗口/已回填/已迁移均直通零写盘）。
    /// provider 资源注销由 adapter 的 `release_after_session_tail` 自行收口
    /// （cross_cutting 不引 store），本验证点在调用侧（web SC run 臂的统一
    /// 出口）。迁移失败只留告警——收口不复活 run，durable 原样保留由恢复
    /// 侧按 Unknown 等待语义处理。
    pub(crate) fn migrate_claimed_sc_teaching_attempts_to_unknown(&mut self) {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return;
        }
        let Some(lifecycle) = self.lifecycle_store.clone() else {
            return;
        };
        loop {
            let Ok(expected) = lifecycle.get_workspace_session(&self.session.session_id) else {
                return;
            };
            let Some((generation_seq, attempt_no)) = expected
                .sc_generation_teaching_windows
                .iter()
                .filter(|window| matches!(window.status, ScTeachingWindowStatus::Active))
                .find_map(|window| {
                    window
                        .attempts
                        .iter()
                        .find(|attempt| {
                            matches!(attempt.lifecycle, ScTeachingAttemptLifecycle::Claimed)
                        })
                        .map(|attempt| (window.generation_seq, attempt.attempt_no))
                })
            else {
                return;
            };
            match lifecycle.mark_sc_teaching_attempt_unknown(&expected, generation_seq, attempt_no)
            {
                Ok(saved) => self.reload_session_from_record(saved),
                Err(error) => {
                    tracing::warn!(
                        session_id = %self.session.session_id,
                        generation_seq,
                        attempt_no,
                        error = ?error,
                        "session-tail sc teaching unknown migration failed; durable evidence left as-is"
                    );
                    return;
                }
            }
        }
    }

    /// B2c（Task 5.5，设计 §5.3 SC 纪律①，plan 接口字典 recovery entry
    /// Ordinary 臂）：SC author 常规入口的 native mismatch precheck——
    /// **先于新业务 reserve** 判定 full resume 可用性。出生（无旧
    /// native）/健康 resume 返回 `Ok(None)` 照常走 reserve 路径（零变化）；
    /// `FrozenBindingUnavailable`（旧 native 在而冻结绑定不可用/已吊销）
    /// 返回 `Ok(Some(reason))`——caller 转 typed
    /// `NativeSupersededWaiting` 等待（不 generic Failed、不 rearm）；
    /// 其余读取错误原样上浮（fail-closed，与既有
    /// `build_work_item_plan_streaming_input` 内同调用的传播语义一致）。
    pub(crate) fn precheck_single_candidate_author_resume(
        &self,
    ) -> Result<Option<&'static str>, String> {
        if self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate {
            return Ok(None);
        }
        let provider = self.session.author_provider.clone();
        match self.provider_resume_session_candidate(ProviderConversationRole::Author, &provider) {
            Ok(_) => Ok(None),
            Err(NativeResumeCandidateError::FrozenBindingUnavailable) => {
                Ok(Some(SC_AUTHOR_NATIVE_MISMATCH_WAITING_REASON))
            }
            Err(error) => Err(format!(
                "single candidate author resume precheck failed: {error}"
            )),
        }
    }

    /// B2c（Task 5.5，设计 §5.3 SC 纪律②，plan 接口字典）：SC author
    /// native mismatch 的 durable 等待收口——手写 ScAuthor tagged record
    /// 后 CAS Waiting，再清被取代旧会话引用并幂等收口等待面
    /// （`reconcile_lc_resume_waiting`）。**不可复用
    /// `persist_lc_superseded_wait`**：其内部
    /// `provider_resume_session_candidate` 在 mismatch 场景自身必败，
    /// 被取代 native id 在此直接取 conversation 引用（内存优先、
    /// durable 兜底）。SC phase/provider_start_ledger 不动——等待保留
    /// 原 key/phase（不 reserve 第二次、不 rearm；显式 fresh 的物理
    /// run/node/native 是新 native 证据，不是新 SC logical reservation）。
    pub(crate) async fn persist_single_candidate_author_superseded_wait(
        &mut self,
        reason: &str,
    ) -> Result<LcResumeRecoveryRecord, String> {
        let store = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let expected = store
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                format!(
                    "load session for sc author superseded wait failed ({}): {error}",
                    self.session.session_id
                )
            })?;
        let role = ProviderConversationRole::Author;
        let provider = self.session.author_provider.clone();
        let conversation = self
            .session
            .provider_conversations
            .iter()
            .find(|conversation| conversation.role == role && conversation.provider == provider)
            .or_else(|| {
                expected.provider_conversations.iter().find(|conversation| {
                    conversation.role == role && conversation.provider == provider
                })
            });
        let superseded_native_session_id = conversation.and_then(|conversation| {
            let native_id = conversation.provider_session_id.trim();
            (!native_id.is_empty()).then(|| native_id.to_string())
        });
        let failed_provider_node_id = self
            .active_node_id
            .clone()
            .or_else(|| conversation.and_then(|conversation| conversation.last_node_id.clone()))
            .unwrap_or_default();
        // Task 5（序 3，tasks 1.2 迁移面）：等待收口的 reservation 锚改读显式
        // 载体——A 接力取 `repair_reservation` 原键（不再取「ledger 最后一个
        // 前缀键」：C 教学占位会被误当业务键）；无预领的原调用取当前业务键
        //（最后一个非教学前缀键）；两者皆无 ⇒ None（无 A 接力锚，fresh 校验
        // 退回 phase/source 对照）。等待保留原 key；fresh 重入不 reserve 第二次。
        let provider_start_reservation_key = expected
            .repair_reservation
            .as_ref()
            .map(|reservation| reservation.provider_start_idempotency_key.clone())
            .or_else(|| current_sc_author_business_key(&expected));
        // 候选 source hash：上一版 candidate source 的 durable content hash
        // （对账证据；装载失败不阻塞等待收口——fresh 返修的真实装载点自会
        // fail-closed）。
        let candidate_source_hash = expected
            .work_item_plan_source_revision_ref
            .as_deref()
            .and_then(|source_ref| {
                let scope = crate::product::work_item_plan_source_store::SourceStoreScope {
                    project_id: expected.project_id.clone(),
                    issue_id: expected.issue_id.clone(),
                    plan_id: expected.entity_id.clone(),
                };
                WorkItemPlanSourceStore::new(store.app_paths())
                    .get_source_revision(&scope, source_ref)
                    .ok()
                    .map(|revision| revision.content_hash)
            });
        let run_context = LcRecoveryRunContext::ScAuthor {
            business: LcRecoveryBusinessContext {
                entity_id: self.session.entity_id.clone(),
                plan_id: None,
                workspace_type: self.session.workspace_type.clone(),
                business_repository_physical_id: None,
                business_repository_logical_id: None,
                business_checkout_id: None,
                business_repository_path: self
                    .session
                    .repository_path
                    .clone()
                    .map(|path| path.to_string_lossy().to_string()),
                source_story_ids: Vec::new(),
                source_design_ids: Vec::new(),
                plan_options_ref: None,
                artifact_refs: Vec::new(),
                review_refs: Vec::new(),
                author_provider: provider.clone(),
                effective_reviewer_provider: self.session.reviewer_provider.clone(),
                original_node_id: (!failed_provider_node_id.is_empty())
                    .then(|| failed_provider_node_id.clone()),
                original_role_run_seq: None,
            },
            flow: WorkItemPlanFlowKind::SingleCandidate,
            single_candidate_phase: expected
                .single_candidate_phase
                .clone()
                .unwrap_or(crate::product::models::SingleCandidatePhase::Prepare),
            provider_start_reservation_key,
            source_revision_ref: expected.work_item_plan_source_revision_ref.clone(),
            ir_ref: expected.plan_candidate_ir_ref.clone(),
            report_ref: expected.mechanical_report_ref.clone(),
            candidate_source_hash,
            business_material_identity: self.session.repository_path.as_ref().map(|path| {
                path.join(".claude/rules/language.md")
                    .to_string_lossy()
                    .to_string()
            }),
        };
        let now = chrono::Utc::now().to_rfc3339();
        let record = LcResumeRecoveryRecord {
            schema_version: 1,
            recovery_id: format!("lc_resume_recovery_{}", uuid::Uuid::new_v4().simple()),
            failed_provider_node_id,
            reason_code: reason.to_string(),
            role: role.clone(),
            provider: provider.clone(),
            superseded_native_session_id: superseded_native_session_id.clone(),
            previous_fingerprint_digest: self
                .logical_launch_fingerprints
                .get(&self.session.author_provider)
                .map(|fingerprint| fingerprint.digest.clone()),
            previous_session_status: expected.status.clone(),
            previous_stage: self.session.stage.as_str().to_string(),
            run_context,
            state: LcResumeRecoveryState::Waiting,
            operation_id: None,
            fresh_node_id: None,
            fresh_native_session_id: None,
            fresh_role_run_seq: None,
            fresh_operations: Vec::new(),
            detected_at: now.clone(),
            updated_at: now,
        };
        let saved = store
            .compare_and_save_lc_resume_state(&expected, record)
            .map_err(|error| {
                format!(
                    "persist sc author superseded wait failed ({}): {error}",
                    self.session.session_id
                )
            })?;
        // 内存镜像同步（A2a 合同：durable 以 CAS 锁内 stored 为准）。
        self.session.session_status = saved.status.clone();
        self.session.lc_resume_recovery = saved.lc_resume_recovery.clone();
        // 被取代旧会话引用清理（显式 fresh 不再携带旧 resume id）+ 等待面
        // 幂等收口；SC phase/ledger 不动。
        if let Some(native_id) = superseded_native_session_id.as_deref() {
            self.clear_superseded_provider_session(role, &provider, native_id)
                .await;
        }
        self.reconcile_lc_resume_waiting().await;
        saved.lc_resume_recovery.ok_or_else(|| {
            "persist sc author superseded wait returned record without recovery".to_string()
        })
    }

    /// F5-A：SC author 修订轮判定。最近一次 review verdict 要求返修（Revise 或
    /// RequiresRevision）时，本轮 SC author 重跑是修订轮，必须把 reviewer findings
    /// 回灌进 prompt；首轮（无 verdict）与 Pass verdict 返回 None，prompt 逐字节不变。
    pub(crate) fn single_candidate_pending_revision_verdict(&self) -> Option<ReviewVerdict> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        {
            return None;
        }
        self.latest_review_verdict.clone().filter(|verdict| {
            verdict.verdict == ReviewVerdictType::Revise
                || verdict.review_gate == ReviewGate::RequiresRevision
        })
    }

    /// SC 修订接力顶杀在途 run 根修（oracle 候选6）：`CrossReview` 阶段对
    /// SingleCandidate 流是二义的——`phase=Evaluate` 表示候选已生成、待驱评审
    /// （在途 run 的 followups 自续 reviewer）；`phase=Generate` 表示评审已裁决
    /// revise/机械契约缺口、policy 路由已把 author 重跑委托给
    /// `ProviderRunRequested{WorkItemPlanSingleCandidateAuthor}` 接力（唯一接续
    /// 路径）。委托态下再按 `stage==CrossReview` 驱 reviewer 只会误燃第二轮
    /// （候选已裁决，reviewer 无意义），接力 spawn 的 supersede 随即取消在途
    /// token 杀死该误燃握手（现场：`claude policy session: handshake failed:
    /// cancelled`）。在途 run 的 followups 据此让位接力。
    pub(crate) fn sc_author_rerun_delegated(&self) -> bool {
        self.session.flow_kind == WorkItemPlanFlowKind::SingleCandidate
            && self.session.single_candidate_phase
                == Some(crate::product::models::SingleCandidatePhase::Generate)
    }

    /// F2（REQ-WIGA-05，workspace_session_0018 现场）：SC 委托返修接力的 spawn
    /// 失败/被拒回执落 durable——仅当会话处于委托态（SingleCandidate 且
    /// phase=Generate，即 followups 已让位、接力是唯一接续路径）且无引擎在途 run
    /// 时回落人工门：`enter_human_confirm` 落 WaitingForHuman（durable）、建新门节点
    /// （摘要携带失败原因）、广播 HumanGateOpened——会话回到可操作状态（人工可再
    /// 反馈修订或放弃），不再静默滞留 running。其余任何形态零副作用：非委托态的
    /// 迟到/无关失败不改写会话状态。
    pub(crate) async fn recover_delegated_author_rerun_failure(&mut self, message: &str) {
        // durable 状态必须仍在途族（Running）——已经回落到 WaitingForHuman 的会话
        // （例如本守卫运行期已收口）不再重复开门；phase 保持 Generate 不构成重入。
        if !self.sc_author_rerun_delegated()
            || self.active_run_id.is_some()
            || self.session.session_status
                != crate::product::models::WorkspaceSessionStatus::Running
        {
            return;
        }
        let summary = format!("SC 返修接力启动失败，已回落人工门：{message}");
        let _ = self
            .event_tx
            .send(EngineEvent::Error {
                message: summary.clone(),
            })
            .await;
        self.enter_human_confirm(Some(summary)).await;
    }

    /// 将 SingleCandidate provider 的 markdown 原文通过 compiler 与 typed source store
    /// 落盘，并在 durable Evaluate 后启动阶段 1 的 reviewer 路由。
    ///
    /// `allow_structure_rerun`：结构门拒收时是否返回可重驱形态——Task 6 起
    /// runner 恒传 true（重驱资格由其消费同一耐久教学窗口的 claim 结果
    /// 决定，四类失败共用，不再有进程内独立槽位）；true 时拒收返回
    /// `Rejected`（不开人工门），false 时保持既有落门收口。
    pub(crate) async fn complete_single_candidate_work_item_plan_author(
        &mut self,
        source: String,
        target_repository_id: String,
        member_targets: Option<crate::product::work_item_plan_compiler::PlanMemberTargets>,
        allow_structure_rerun: bool,
    ) -> Result<SingleCandidateAuthorCompletion, String> {
        if self.session.workspace_type != WorkspaceType::WorkItemPlan
            || self.session.flow_kind != WorkItemPlanFlowKind::SingleCandidate
        {
            return Err(
                "single candidate author requires a SingleCandidate WorkItemPlan session"
                    .to_string(),
            );
        }
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or_else(|| "lifecycle_store unavailable".to_string())?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();
        let plan = lifecycle
            .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
            .map_err(|error| format!("load plan for single candidate source failed: {error}"))?;
        let repository_profile = plan
            .repository_profile_ref
            .as_deref()
            .map(|profile_id| {
                lifecycle
                    .get_repository_profile(&project_id, &issue_id, profile_id)
                    .map_err(|error| {
                        format!(
                            "load repository profile for single candidate source failed: {error}"
                        )
                    })
            })
            .transpose()?;

        // O1（裁决二 §2.2/§2.4）：author 落盘前不再本地改写 source——
        // converge 单次编译并产出机器指令；IR/source 保持 author 原始
        // 产物，Error 级契约缺口由既有机械 verdict 通道（F5-A）交原授权
        // author 返修，指令随完成面呈现给 owner（不应用、不丢弃）。
        let contract_autorepair::ContractConvergeOutcome {
            ir,
            instructions: contract_autorepair_instructions,
            audit,
        } = contract_autorepair::converge_work_item_plan_source(
            &source,
            WorkItemPlanSourceContext {
                target_repository_id,
                member_targets: member_targets.clone(),
            },
        )
        .map_err(|diagnostics| {
            format_compiler_diagnostics("compile markdown source", &diagnostics)
        })?;
        tracing::debug!(
            session_id = %self.session.session_id,
            equivalence_canonicalizations = audit.equivalence_canonicalizations.len(),
            contract_autorepair_instructions = contract_autorepair_instructions.len(),
            "single-candidate author completion converged (O1: instructions only, no source mutation)"
        );
        let source_hash = hex::encode(Sha256::digest(source.as_bytes()));
        let source_id = format!("source-{}", &source_hash[..16]);
        let mut source_revision = SourceRevisionRecord {
            id: source_id.clone(),
            source: source.clone(),
            source_revision_hash: source_hash,
            content_hash: String::new(),
        };
        source_revision.content_hash = source_revision
            .content_hash()
            .map_err(|error| format!("hash source revision failed: {error:?}"))?;
        let source_store = WorkItemPlanSourceStore::new(lifecycle.app_paths());
        let source_ref = source_store
            .put_source_revision(&project_id, &issue_id, &plan_id, &source_revision)
            .map_err(|error| {
                format!(
                    "persist source revision failed [{}]: {error:?}",
                    error.code()
                )
            })?;

        let ir_id = format!("ir-{}", &source_revision.source_revision_hash[..16]);
        let mut ir_record = PlanCandidateIrRecord {
            id: ir_id.clone(),
            source_revision_id: source_id,
            ir,
            content_hash: String::new(),
        };
        ir_record.content_hash = ir_record
            .content_hash()
            .map_err(|error| format!("hash plan candidate IR failed: {error:?}"))?;
        let ir_ref = source_store
            .put_plan_candidate_ir(&project_id, &issue_id, &plan_id, &ir_record)
            .map_err(|error| {
                format!(
                    "persist plan candidate IR failed [{}]: {error:?}",
                    error.code()
                )
            })?;

        // SG3a.1 系统门（REQ-SG-02/03、REQ-PLN-09，plan §0.6.2）：先于
        // current refs 提升与 reviewer。R1/parser/target/scope 坏候选保留
        // typed evidence 但零提升、零评审、零确认；合法候选才冻结 Some
        // 结构报告并随同 source/IR/report refs 落盘（None 是旧记录参考
        // 形态，不冒新合法授权）。live 事实一次装载，只读不 selection
        // 写回；装载失败是 operational 停点，同样不提升。
        let structure_context = self
            .build_single_candidate_structure_context(&ir_record.ir, &source_ref, &ir_ref)
            .map_err(|error| format!("single-candidate structure context unavailable: {error}"))?;
        let assessment = plan_projection::assess_single_candidate_structure(
            source.clone(),
            &ir_record.ir,
            &structure_context,
        );
        let frozen_structure_report = match &assessment {
            crate::product::workspace_engine::types::StructuralAssessment::Accepted(prepared) => {
                Some(
                    plan_projection::freeze_persisted_structure_report(
                        &prepared.report,
                        &source_revision.id,
                        &ir_id,
                    )
                    .map_err(|error| format!("freeze structure report failed: {error}"))?,
                )
            }
            // SG3a.1 拒收**不再硬失败**（现场 2026-10-31：owner 只看到「生成
            // 失败」）：与 Final Compile 同一条 typed 失败路——durable 诊断 +
            // 尝试流水 + 自动重写/人工门（REQ-WSC-18/21/25）。候选仍零提升、
            // 零评审、零确认（current refs 语义零变化）。恰一次教学重驱槽位
            // 空闲时先交还修复指令重驱作者（`Rejected`），不落门。
            crate::product::workspace_engine::types::StructuralAssessment::Rejected(
                failures,
                _,
            ) => {
                let failure =
                    crate::product::workspace_engine::single_candidate_structure_failure::
                        author_structure_gate_failure(
                            &ir_record.ir,
                            &structure_context,
                            failures,
                        );
                return Ok(self
                    .route_author_structure_gate_failure(
                        &lifecycle,
                        failure,
                        failures.clone(),
                        &source_ref,
                        &ir_ref,
                        allow_structure_rerun,
                    )
                    .await);
            }
            _ => {
                return Err(
                    plan_projection::structural_assessment_stop_reason(&assessment)
                        .expect_err("non-accepted assessment must produce a typed stop"),
                );
            }
        };

        let expected = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| {
                format!("reload workspace session before generated refs CAS failed: {error}")
            })?;
        let generated = lifecycle
            .compare_and_save_single_candidate_generation(&expected, &source_ref, &ir_ref)
            .map_err(|error| format!("persist single candidate generated refs failed: {error}"))?;

        // F-56→REQ-PIB-03：plan 基线=issue 基线分支树（按分支名取树，不依赖
        // 共享 worktree）；不可解析 → Err 直通 fail-closed（废弃跳过——软限制
        // 残余风险的唯一硬兜底）。
        let baseline_tree = plan_preflight::plan_baseline_tree(&lifecycle, &project_id, &issue_id)?;
        // C1 Task 5（REQ-C1-PLAN-01）：existing 意图的授权解析面——issue
        // 下 durable 既有 work item id 集（读失败 fail-closed 不猜）。
        let existing_work_item_ids = lifecycle
            .list_work_items(&project_id, &issue_id)
            .map_err(|error| format!("list existing work items failed: {error}"))?
            .into_iter()
            .map(|record| record.id)
            .collect::<Vec<_>>();
        // 批 2 R1（REQ-WIGA-01）：enrolled 会话传真实 binding+成员映射——
        // 装载失败走显式错误（不以 None 吞错）；Manual/非 enrolled 保持
        // None（intent target 核对不触发，零回归）。
        let enrollment_binding = self
            .current_enrollment_binding_for_compile(&lifecycle, &plan_id)
            .map_err(|error| {
                format!("load enrollment binding for candidate validation failed: {error}")
            })?;
        let validation_member_targets = match &enrollment_binding {
            Some(_) => member_targets.as_ref(),
            None => None,
        };
        // REQ-WSC-10（写范围×被授权仓归属）：enrolled 装载成员 checkout 前缀表（失败显式错误不吞错；单仓保持 None 零回归）。
        let validation_member_checkout_prefixes = member_checkout_prefixes::for_validation(
            &lifecycle,
            &plan,
            enrollment_binding.as_ref(),
        )
        .map_err(|error| {
            format!("load member checkout prefixes for candidate validation failed: {error}")
        })?;
        let validation_now = chrono::Utc::now().to_rfc3339();
        let report = validate_plan_candidate_ir(
            &ir_record.ir,
            &PlanCandidateValidationContext {
                project_id: &project_id,
                issue_id: &issue_id,
                plan_id: &plan_id,
                source_story_spec_ids: &plan.source_story_spec_ids,
                source_design_spec_ids: &plan.source_design_spec_ids,
                repository_profile: repository_profile.as_ref(),
                // F-51：三生产路径之一（author 权威落盘）——存储 options 显式
                // 提供，候选校验不从 IR 反推。
                plan_options: &plan.options,
                baseline_tree: baseline_tree.as_ref(),
                existing_work_item_ids: &existing_work_item_ids,
                enrollment_target: enrollment_binding.as_ref().map(|binding| &binding.target),
                member_targets: validation_member_targets,
                member_checkout_prefixes: validation_member_checkout_prefixes.as_ref(),
                now: &validation_now,
            },
        )
        .map_err(|diagnostics| {
            format_compiler_diagnostics("validate plan candidate IR", &diagnostics)
        })?;
        // compile 同源机械校验（build/validate_dependency_contract_graph）。
        // 存在 Error 级缺口时在本轮 Evaluate 即产生机械返修 verdict（经
        // complete_review 既有 ingestion 驱动 F5-A 修订轮回灌），不再等到
        // Approval compile 才失败；无 Error 缺口时零变化（不产生 verdict）。
        // Warning 级 findings 以 Suggestion+Advisory 搭车同一 verdict。
        let contract_prerevision_verdict =
            contract_prerevision::single_candidate_contract_prerevision_verdict(&ir_record.ir);
        // report_id 全量内容寻址（对齐 provenance 全 sha256 惯例）：
        // id 由 (source_revision_id, ir_id, findings/kind,
        // compiler_version) 全量记录体派生，不再只按 source hash 寻址
        // ——同 source 上 validator 语义演进（如 F2b typed kind）产生新
        // id 追加写入，同内容重放走整记录相等分支幂等 Ok，杜绝同 id
        // 不同 body 撞 put_immutable 的确定性 IdentityMismatch。
        let mut report_record = PlanCandidateMechanicalReportRecord {
            id: String::new(),
            source_revision_id: source_revision.id.clone(),
            ir_id: ir_id.clone(),
            report,
            // SG3a.1：系统门通过的合法候选同 refs 冻结 Some 结构投影
            // （同 record/content_hash 绑定，context 同代由 reviewer
            // builder 复验）；拒收候选在提升前已 typed 停点，不落 record。
            structure_report: frozen_structure_report,
            content_hash: String::new(),
        };
        report_record.id = report_record
            .content_addressed_id()
            .map_err(|error| format!("derive mechanical report id failed: {error:?}"))?;
        // C1 preflight 前移（F-51/F-56）：存储 options×items 三族缺口与 AC
        // 路径×基线树缺口同样在本轮 Evaluate 产生机械返修 verdict（preflight
        // 族不硬失败 author 轮次），与 canonical 契约缺口共用一次 complete_review
        // ingestion。
        let preflight_verdict = plan_preflight::preflight_review_verdict(&report_record.report);
        let mechanical_revision_verdict = match (contract_prerevision_verdict, preflight_verdict) {
            (Some(contract), Some(preflight)) => {
                Some(plan_preflight::merge_revision_verdicts(contract, preflight))
            }
            (contract, preflight) => contract.or(preflight),
        };
        report_record.content_hash = report_record
            .content_hash()
            .map_err(|error| format!("hash mechanical report failed: {error:?}"))?;
        let report_ref = source_store
            .put_mechanical_report(&project_id, &issue_id, &plan_id, &report_record)
            .map_err(|error| {
                format!(
                    "persist mechanical report failed [{}]: {error:?}",
                    error.code()
                )
            })?;
        let saved = lifecycle
            .compare_and_save_single_candidate_evaluation(&generated, &report_ref)
            .map_err(|error| {
                format!("persist single candidate Evaluate report ref failed: {error}")
            })?;
        self.session.work_item_plan_source_revision_ref = saved.work_item_plan_source_revision_ref;
        self.session.plan_candidate_ir_ref = saved.plan_candidate_ir_ref;
        self.session.mechanical_report_ref = saved.mechanical_report_ref;
        self.session.publication_provenance_ref = saved.publication_provenance_ref;
        self.session.single_candidate_phase = saved.single_candidate_phase;
        self.session.session_status = saved.status;

        self.update_artifact(ArtifactPayload::Markdown {
            markdown: source,
            diff: None,
        })
        .await?;
        let completion_message = if contract_autorepair_instructions.is_empty() {
            "SingleCandidate markdown source 已编译并持久化，等待 Evaluate".to_string()
        } else {
            // Task 3 评审 Minor-3（Task 4 补正）：指令段与 handler 侧同一截断口径。
            completion_message_with_instruction_actions(
                contract_autorepair_instructions.len(),
                &contract_autorepair::cap_autorepair_instruction_lines(
                    contract_autorepair_instructions
                        .iter()
                        .map(|instruction| instruction.required_action.clone())
                        .collect(),
                ),
            )
        };
        self.complete_active_node(Some(completion_message)).await;
        if let Some(verdict) = mechanical_revision_verdict {
            self.route_single_candidate_contract_prerevision(verdict)
                .await;
        } else if self.session.review_rounds == 0 || self.session.reviewer_provider.is_none() {
            self.route_single_candidate_evaluate_without_reviewer()
                .await;
        } else {
            self.start_review().await;
            if self.session.stage == WorkspaceStage::CrossReview {
                self.request_provider_run(ProviderRunKind::ReviewOnly).await;
            }
        }
        Ok(SingleCandidateAuthorCompletion::Candidate {
            item_count: ir_record.ir.items.len(),
        })
    }

    /// 机械 canonical 契约缺口（Error 级）的返修路由：与 reviewer verdict 完全
    /// 同一条既有链路——建同一 ReviewerRun 评审轮节点（start_review）、按
    /// durable 候选 refs 重建 invocation scope（与 drive_review_session 同源），
    /// 再经 complete_review 既有 ingestion 完成：
    /// - `latest_review_verdict` 注入 → F5-A 修订轮判定命中，下一轮 SC author
    ///   重跑回灌机械 findings；
    /// - verdict 持久化到 ReviewerRun 节点 → 跨轮指纹对比数据源；
    /// - policy 路由（ContractGap+Repairable）→ TriggerAggregateRepair 重驱
    ///   SC author（repairs_used 与 reviewer 返修轮同池）；连续 2 轮同指纹
    ///   由既有 RepeatedFingerprint 人工门兜底，不新增终止逻辑。
    ///
    /// Fake reviewer 快速路径（start_review 直接进 HumanConfirm）不叠加
    /// synthetic verdict，保持其既有语义零变化。
    pub(crate) async fn route_single_candidate_contract_prerevision(
        &mut self,
        verdict: ReviewVerdict,
    ) {
        self.start_review().await;
        if self.session.stage != WorkspaceStage::CrossReview {
            return;
        }
        if let Err(message) = self.ensure_review_invocation_scope().await {
            let _ = self.event_tx.send(EngineEvent::Error { message }).await;
            self.finish_failed_run().await;
            return;
        }
        // preflight/合并 verdict 的 readable 用 preflight 前缀（timeline 可区分
        // 来源）；纯 canonical 契约缺口保持既有文本零变化。
        let readable = if verdict.findings.iter().any(|finding| {
            finding
                .evidence
                .starts_with("plan preflight mechanical finding")
        }) {
            plan_preflight::preflight_readable_output(&verdict)
        } else {
            contract_prerevision::contract_prerevision_readable_output(&verdict)
        };
        self.complete_review(ProviderCompletion::plain(readable, None), verdict)
            .await;
    }

    /// P08a（计划 §2.4）：失败链身份锚——从持久 lifecycle 的**最新 durable**
    /// 会话记录读原业务键，口径与 `current_sc_author_business_key` 完全同源
    /// （不另立规则，也不看进程内镜像）。缺失即显式
    /// `sc_failure_chain_identity_missing`：不空串兜底、不回退去猜「ledger
    /// 最后键」。本方法只读——不分配授权、不追加 ledger、不建日志/handle；
    /// 返回的业务键由 P08 首 consumer 用于构造失败链作用域。
    pub(crate) fn sc_author_business_key_for_failure_chain(&self) -> Result<String, String> {
        let lifecycle = self
            .persistent_lifecycle()
            .map_err(|error| format!("sc_failure_chain_identity_unavailable: {error:?}"))?;
        let record = lifecycle
            .get_workspace_session(&self.session.session_id)
            .map_err(|error| format!("sc_failure_chain_identity_unavailable: {error}"))?;
        current_sc_author_business_key(&record).ok_or_else(|| {
            "sc_failure_chain_identity_missing: durable session has no original single candidate author business key"
                .to_string()
        })
    }
}

/// Task 5（序 3，tasks 1.2 迁移面）：当前业务链最后一个**非教学** SC author
/// 启动键（前缀匹配且 `provider_attempt_key == None` 的最大数字后缀条目）
/// ——原调用重入与等待收口共用的幂等锚。不取「ledger 最后一个前缀键」
/// （C 教学占位会被误当业务键），也不取「全 ledger 长度」（人门键与教学
/// 条目会把序号顶偏）。
pub(crate) fn current_sc_author_business_key(
    record: &crate::product::models::WorkspaceSessionRecord,
) -> Option<String> {
    let prefix = format!("single_candidate_author:{}:", record.id);
    record
        .provider_start_ledger
        .iter()
        .filter(|entry| {
            entry.provider_start_idempotency_key.starts_with(&prefix)
                && entry.provider_attempt_key.is_none()
        })
        .filter_map(|entry| {
            entry
                .provider_start_idempotency_key
                .strip_prefix(&prefix)
                .and_then(|suffix| suffix.parse::<usize>().ok())
        })
        .max()
        .map(|ordinal| format!("{prefix}{ordinal}"))
}

fn format_compiler_diagnostics(
    operation: &str,
    diagnostics: &[crate::product::work_item_plan_compiler::CompilerDiagnostic],
) -> String {
    let detail = diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "{}:{}:{}",
                diagnostic.code, diagnostic.line, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("; ");
    format!("{operation} failed: {detail}")
}

/// O1：completion message 的指令段拼装——`total` 为真实指令条数，
/// `capped_actions` 为已经 `cap_autorepair_instruction_lines` 截断的动作行
/// （条数上限 + 计数摘要，与 web handler 侧同一口径，owner 可见消息有界）。
fn completion_message_with_instruction_actions(total: usize, capped_actions: &[String]) -> String {
    format!(
        "SingleCandidate markdown source 已编译并持久化（O1：{total} 条契约缺口机器指令交原授权 author，平台不改写 source——{}），等待 Evaluate",
        capped_actions.join("；")
    )
}

#[cfg(test)]
mod completion_message_tests {
    use super::completion_message_with_instruction_actions;
    use crate::product::workspace_engine::contract_autorepair::{
        MAX_AUTOREPAIR_INSTRUCTION_LINES, cap_autorepair_instruction_lines,
    };

    /// Task 3 评审 Minor-3（Task 4 补正）：completion message 的指令段与
    /// web handler 侧同一截断口径——超上限走计数摘要，消息长度有界，
    /// 不再逐条无界透传全部 `required_action`。
    #[test]
    fn completion_message_instruction_segment_is_capped_like_handler_side() {
        let actions: Vec<String> = (0..50)
            .map(|index| format!("动作-{index}：{}", "禁则与证据说明".repeat(40)))
            .collect();
        let uncapped_len = actions.join("；").len();
        let capped = cap_autorepair_instruction_lines(actions);
        let message = completion_message_with_instruction_actions(50, &capped);

        assert_eq!(capped.len(), MAX_AUTOREPAIR_INSTRUCTION_LINES + 1);
        assert!(
            message.contains("O1：50 条契约缺口机器指令"),
            "真实总数必须保留: {message}"
        );
        assert!(message.contains("动作-3："), "上限内的指令必须在场");
        assert!(!message.contains("动作-4："), "超上限的指令不得逐条透传");
        assert!(
            message.contains("共 50 条契约修复指令"),
            "必须以计数摘要收口"
        );
        assert!(
            message.len() * 5 < uncapped_len,
            "消息长度必须显著小于无界全文（{} vs {}）",
            message.len(),
            uncapped_len
        );
    }
}
