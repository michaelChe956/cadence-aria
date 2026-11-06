impl WorkspaceEngine {
    /// Task 2.3：LC revision 的 root launch + resume 决策（`drive_revision_session`
    /// 的 LC 分支消费）。launch 与 Task 2.1 author 首轮同源（cwd = authority
    /// manifest `provider_context_root`；target = E `native_launch_anchor`
    /// 聚合根锚，REQ-PLN-08：scope 回写不迁移；`PlanningReadOnly` + author
    /// provider ref），envelope/resume 面一致。
    ///
    /// resume 决策（REQ-ENV-04 cwd 维度）：input 携带旧 native session 且引擎
    /// 记有其发行 launch 的 cwd-inclusive 指纹时，经 `gateway.resume_or_start`
    /// 全维度比对——指纹一致（cwd 未漂移）→ `resume_allowed=true`，native
    /// session identity 原样续接；漂移 → supersede 审计 + StartNew
    ///（`resume_allowed=false`，丢 resume id 走 full prompt 新 thread）。指纹
    /// 记忆缺失（跨连接重建 engine）→ fail-closed 新会话：无法证明旧 thread 与
    /// 当前 launch 全维度一致，绝不静默续接。
    pub(crate) fn resolve_revision_root_launch(
        &self,
    ) -> Option<Result<RevisionLaunchDecision, String>> {
        self.resolve_revision_root_launch_with_freshness(false)
    }

    /// B1（终审 P1）：`force_fresh=true` —— 显式 fresh run（FreshRunning
    /// author 族 recovery spawn 的本 run）的 launch 决策：不进
    /// `resume_or_start` 的 supersede 决策（本 run 即授权的 StartNew，旧
    /// native 已 tombstone 不回捞），直接 `gateway.validate` 冻结全新
    /// launch；`resume_allowed=false` 供调用方按 B1a 合同强制 resume=None
    /// 的 full-fresh prompt。
    fn resolve_revision_root_launch_fresh(&self) -> Option<Result<RevisionLaunchDecision, String>> {
        self.resolve_revision_root_launch_with_freshness(true)
    }

    fn resolve_revision_root_launch_with_freshness(
        &self,
        force_fresh: bool,
    ) -> Option<Result<RevisionLaunchDecision, String>> {
        let gateway = self.logical_provider_gateway()?;
        let Some(lifecycle) = self.lifecycle_store.as_ref() else {
            return Some(Err(
                "logical revision launch requires a persistent lifecycle store".to_string(),
            ));
        };
        let record = match lifecycle.get_workspace_session(&self.session.session_id) {
            Ok(record) => record,
            // B1a（Task 4.1）：record 读取错误不再 `.ok()` 吞错——底层
            // 类别（product_store_io/json/identity）随消息保留传播。
            Err(error) => {
                return Some(Err(format!(
                    "logical revision launch cannot load workspace session record {}: {error}",
                    self.session.session_id
                )));
            }
        };
        let provider = self.session.author_provider.clone();
        let request = match self.build_root_launch_request(
            &record,
            &provider,
            SessionPolicyAction::PlanningReadOnly,
        )? {
            Ok(request) => request,
            Err(message) => return Some(Err(message)),
        };
        // B1a（Task 4.1，BP2）：candidateErr 即 `Some(Err(..))`（typed 类别
        // 保留），不落 `(None, _)` validate/fresh 臂；成功候选保留 native/seq
        // （seq 与 durable fullmap/audit 联合绑定已在候选内校验）。
        // B1（终审 P1）：fresh 直接 validate 全新 launch——不查旧 candidate
        //（tombstone 候选的 FrozenBindingUnavailable 不阻断授权 fresh）、
        // 不进 resume_or_start 的 supersede 决策。
        let decision = if force_fresh {
            match gateway.validate(request) {
                Ok(launch) => RevisionLaunchDecision {
                    launch,
                    resume_allowed: false,
                    superseded: None,
                },
                Err(error) => {
                    return Some(Err(format!(
                        "logical revision gateway validation failed: {error}"
                    )));
                }
            }
        } else {
            let resume_session_id = match self
                .provider_resume_session_candidate(ProviderConversationRole::Author, &provider)
            {
                Ok(candidate) => candidate
                    .map(|candidate| candidate.native_id)
                    .filter(|id| !id.trim().is_empty()),
                Err(error) => {
                    return Some(Err(format!(
                        "logical revision resume candidate unavailable: {error}"
                    )));
                }
            };
            match (
                resume_session_id,
                self.logical_launch_fingerprints.get(&provider),
            ) {
                (Some(previous_session_id), Some(previous_fingerprint)) => {
                    match gateway.resume_or_start(ResumeSessionLaunchRequest {
                        launch: request,
                        previous_fingerprint: previous_fingerprint.clone(),
                        previous_session_id,
                    }) {
                        Ok(GatewaySessionDisposition::Resume(launch)) => RevisionLaunchDecision {
                            launch,
                            resume_allowed: true,
                            superseded: None,
                        },
                        // Task 9c:StartNew 携带 superseded session id 与稳定等待
                        // 投影——同一 revision driver 不得据此继续 start_streaming。
                        Ok(GatewaySessionDisposition::StartNew {
                            validated,
                            superseded_session_id,
                            waiting,
                        }) => RevisionLaunchDecision {
                            launch: validated,
                            resume_allowed: false,
                            superseded: Some(SupersededRevisionWait {
                                superseded_session_id,
                                waiting,
                            }),
                        },
                        Err(error) => {
                            return Some(Err(format!(
                                "logical revision gateway validation failed: {error}"
                            )));
                        }
                    }
                }
                // 指纹记忆缺失（跨连接重建）：fail-closed 新会话。
                (Some(_), None) => match gateway.validate(request) {
                    Ok(launch) => RevisionLaunchDecision {
                        launch,
                        resume_allowed: false,
                        superseded: None,
                    },
                    Err(error) => {
                        return Some(Err(format!(
                            "logical revision gateway validation failed: {error}"
                        )));
                    }
                },
                (None, _) => match gateway.validate(request) {
                    Ok(launch) => RevisionLaunchDecision {
                        launch,
                        resume_allowed: true,
                        superseded: None,
                    },
                    Err(error) => {
                        return Some(Err(format!(
                            "logical revision gateway validation failed: {error}"
                        )));
                    }
                },
            }
        };
        Some(Ok(decision))
    }

    /// A2b（Task 2.6）：LC 等待 record 的业务上下文快照——engine 已知身份
    /// 全量携带；未持久化的身份（业务 repository 物理/logical id、source
    /// story/design ids、role_run_seq）按 A1a 模型纪律留 None 不倒算
    /// （covered-run fresh handler 由 B1b/B2 按 run kind 补全重建事实）。
    fn lc_recovery_business_context(&self) -> LcRecoveryBusinessContext {
        LcRecoveryBusinessContext {
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
            author_provider: self.session.author_provider.clone(),
            effective_reviewer_provider: self.session.reviewer_provider.clone(),
            original_node_id: self.active_node_id.clone(),
            original_role_run_seq: None,
        }
    }

    /// B1（终审 P1）：durable 侧的 author 族显式 fresh run 判据——
    /// `FreshRunning` 且 run_context tag 属 author 族（Author/
    /// AuthorChoiceFollowup/AuthorFeedback/ReviewerRevision）。以 durable
    /// record 为准（dispatch 推进的是 durable 状态；engine 内存镜像可能滞留
    /// Waiting）；读取失败按无 fresh 处理（fail-safe 等待方向）并 warn。
    pub(crate) fn lc_fresh_author_family_recovery(
        &self,
    ) -> Option<crate::product::models::LcResumeRecoveryRecord> {
        let store = self.lifecycle_store.as_ref()?;
        let record = match store.get_workspace_session(&self.session.session_id) {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(
                    %error,
                    session_id = %self.session.session_id,
                    "load workspace session for author family fresh recovery failed; treating as not-fresh"
                );
                return None;
            }
        };
        record.lc_resume_recovery.filter(|recovery| {
            recovery.state == crate::product::models::LcResumeRecoveryState::FreshRunning
                && matches!(
                    recovery.run_context,
                    crate::product::models::LcRecoveryRunContext::Author { .. }
                        | crate::product::models::LcRecoveryRunContext::AuthorChoiceFollowup { .. }
                        | crate::product::models::LcRecoveryRunContext::AuthorFeedback { .. }
                        | crate::product::models::LcRecoveryRunContext::ReviewerRevision { .. }
                )
        })
    }

    /// B1（终审 P1，§4.3/§5.3 author 族行）：fresh run 驱动结束后的 recovery
    /// 对账——fresh 节点 Completed → Resolved（新 native/节点回填 +
    /// operation Completed）；其余（Failed/悬置）→ 回 Waiting（operation
    /// Failed，结果保留：同 ID 只 Replay、下一次显式点击用新 ID）。
    /// 整记录 CAS single-flight；败给并发写方让位、store 写失败 warn（等待
    /// 方向 fail-safe）；engine 内存镜像同步。
    pub(crate) async fn reconcile_lc_fresh_author_family_run(
        &mut self,
        fresh_node_id: Option<&str>,
    ) {
        use crate::product::models::{
            LcFreshOperationState, LcRecoveryRunContext, LcResumeRecoveryState,
        };
        let Some(store) = self.lifecycle_store.clone() else {
            return;
        };
        let Ok(expected) = store.get_workspace_session(&self.session.session_id) else {
            // 读取失败不误判终态（等待方向 fail-safe）。
            return;
        };
        let Some(mut recovery) = expected.lc_resume_recovery.clone().filter(|recovery| {
            recovery.state == LcResumeRecoveryState::FreshRunning
                && matches!(
                    recovery.run_context,
                    LcRecoveryRunContext::Author { .. }
                        | LcRecoveryRunContext::AuthorChoiceFollowup { .. }
                        | LcRecoveryRunContext::AuthorFeedback { .. }
                        | LcRecoveryRunContext::ReviewerRevision { .. }
                )
        }) else {
            return;
        };
        let author_provider = self.session.author_provider.clone();
        let conversation = self
            .session
            .provider_conversations
            .iter()
            .find(|conversation| {
                conversation.role == ProviderConversationRole::Author
                    && conversation.provider == author_provider
            })
            .cloned();
        let fresh_node_id = fresh_node_id
            .map(str::to_string)
            .or_else(|| recovery.fresh_node_id.clone())
            .or_else(|| {
                conversation
                    .as_ref()
                    .and_then(|conversation| conversation.last_node_id.clone())
            });
        let native_session_id = conversation
            .map(|conversation| conversation.provider_session_id)
            .filter(|id| !id.trim().is_empty());
        let node_completed = fresh_node_id
            .as_deref()
            .and_then(|node_id| {
                self.timeline_nodes
                    .iter()
                    .find(|node| node.node_id == node_id)
            })
            .is_some_and(|node| node.status == TimelineNodeStatus::Completed);
        let now = chrono::Utc::now().to_rfc3339();
        if node_completed {
            recovery.state = LcResumeRecoveryState::Resolved;
            if recovery.fresh_native_session_id.is_none() {
                recovery.fresh_native_session_id = native_session_id;
            }
            if recovery.fresh_node_id.is_none() {
                recovery.fresh_node_id = fresh_node_id.clone();
            }
            if let Some(operation_id) = recovery.operation_id.as_deref() {
                for operation in recovery.fresh_operations.iter_mut() {
                    if operation.operation_id == operation_id {
                        operation.state = LcFreshOperationState::Completed;
                        if operation.fresh_node_id.is_none() {
                            operation.fresh_node_id = fresh_node_id.clone();
                        }
                        operation.updated_at = now.clone();
                    }
                }
            }
        } else {
            recovery.state = LcResumeRecoveryState::Waiting;
            let operation_id = recovery.operation_id.take();
            if let Some(operation_id) = operation_id.as_deref() {
                for operation in recovery.fresh_operations.iter_mut() {
                    if operation.operation_id == operation_id {
                        operation.state = LcFreshOperationState::Failed;
                        operation.reason_code = Some(
                            "lc_resume_fresh_run_failed: fresh author run did not complete"
                                .to_string(),
                        );
                        operation.updated_at = now.clone();
                    }
                }
            }
        }
        recovery.updated_at = now.clone();
        match store.compare_and_save_lc_resume_state(&expected, recovery) {
            Ok(saved) => {
                self.session.lc_resume_recovery = saved.lc_resume_recovery.clone();
            }
            Err(crate::product::json_store::ProductStoreError::Conflict { .. }) => {
                tracing::debug!(
                    session_id = %self.session.session_id,
                    "lc fresh author run reconcile CAS lost to a concurrent writer; conceding"
                );
            }
            Err(error) => {
                tracing::warn!(
                    %error,
                    session_id = %self.session.session_id,
                    "reconcile lc fresh author run outcome failed (fail-safe: durable state unchanged)"
                );
            }
        }
    }
}
