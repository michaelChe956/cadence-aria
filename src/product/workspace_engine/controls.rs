use super::*;

impl WorkspaceEngine {
    pub async fn handle_rollback(&mut self, checkpoint_id: &str) -> Result<(), String> {
        let target = self
            .checkpoint_store
            .rollback_to(&self.session.session_id, checkpoint_id)
            .map_err(|e| format!("rollback failed: {e}"))?;

        let keep_count = target.message_index as usize;
        self.session.messages.truncate(keep_count);

        if let Some(stage) = WorkspaceStage::from_stage_name(&target.stage)
            && self.session.stage != stage
        {
            self.transition_stage(stage).await;
        }

        self.session.artifact = target.artifact_snapshot.clone();
        if let Some(store) = &self.lifecycle_store {
            let _ = store.truncate_workspace_session_messages(
                &self.session.session_id,
                keep_count,
                workspace_status_for_stage(&self.session.stage),
            );
        }

        Ok(())
    }

    pub async fn handle_confirm(&mut self) -> Result<WorkspaceConfirmOutcome, String> {
        match self.session.stage {
            WorkspaceStage::HumanConfirm => {
                self.validate_confirm_aggregate_spec_gate()?;
                // T4.1（REQ-REG-15 方案 A/D1）：合法 HumanConfirm 门在
                // aggregate spec 校验后、定稿/创建子会话/直入 compile 之前
                // 做确认 trust 按需准备（mutex 内锁定的 session 事实取有效
                // provider；SC Approval 直入 compile 臂同样先准备）。Running
                // 臂不登记。未 Ready 返回 typed TrustWaiting，零定稿副作用。
                if let Err(waiting) = self
                    .ensure_confirmation_trust_ready(
                        self.session.author_provider.clone(),
                        self.session.reviewer_provider.clone(),
                    )
                    .await
                {
                    return Ok(WorkspaceConfirmOutcome::TrustWaiting { waiting });
                }
                match self.session.workspace_type {
                    WorkspaceType::WorkItemPlan
                        if self.session.flow_kind
                            == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
                            && self.session.single_candidate_phase
                                == Some(crate::product::models::SingleCandidatePhase::Approval) =>
                    {
                        // REQ-WSC-19：确认＝必然重跑 Final Compile。当前候选已
                        // 确定编译不过时 fail-closed（与 close_human_gate 的
                        // Approve 臂同一判据、同一文案），零 compile 副作用。
                        self.ensure_current_candidate_compile_not_failed()?;
                        self.enter_policy_valid_work_item_plan_compile_trust_ready()
                            .await;
                        return Ok(WorkspaceConfirmOutcome::None);
                    }
                    WorkspaceType::WorkItemPlan => {
                        let (_plan, new_sessions) =
                            self.confirm_work_item_plan_and_complete().await?;
                        return Ok(WorkspaceConfirmOutcome::WorkItemPlan {
                            child_sessions: new_sessions,
                        });
                    }
                    _ => {
                        // 与 AuthorDecision::AcceptFinalize 共用定稿实现
                        //（mark_latest_artifact_confirmed + 状态 Confirmed + Completed 节点）。
                        self.finalize_current_artifact("已确认通过").await?;
                    }
                }
            }
            WorkspaceStage::Running => {
                self.transition_stage(WorkspaceStage::CrossReview).await;
            }
            _ => {}
        }
        Ok(WorkspaceConfirmOutcome::None)
    }

    /// WorkItemPlan 确认的引擎推进段（**纯移动**自 `handle_confirm` 的 WorkItemPlan 臂，
    /// 行为不变）：收口当前门节点 → 定稿产物 → `confirm_work_item_plan`（plan Draft→
    /// Confirmed + 建子 WorkItem 会话，SC 流走 CAS 保留门快照）→ stage→Completed → SC
    /// 终态相位 → Completed 节点。
    ///
    /// 两处共用：`handle_confirm`（HumanConfirm 门的 legacy/对话流）与 HTTP confirm 端点
    /// 的 `WorkItemBatchConfirm` 分支（change plan-compile-gate-visibility Task 3——
    /// 端点此前对 WorkItemPlan 走 store-only 兜底，只写 status=Confirmed，stage 停在
    /// AuthorConfirm 且不建子会话，形成「既不能 advance 也无子会话」的半落状态）。
    pub(crate) async fn confirm_work_item_plan_and_complete(
        &mut self,
    ) -> Result<(IssueWorkItemPlan, Vec<WorkspaceSessionRecord>), String> {
        self.complete_active_node(Some("已确认通过".to_string()))
            .await;
        self.mark_latest_artifact_confirmed(Some("human".to_string()));
        let (plan, new_sessions) = self.confirm_work_item_plan().await?;
        self.transition_stage(WorkspaceStage::Completed).await;
        self.persist_single_candidate_terminal_phase(
            crate::product::models::SingleCandidatePhase::Completed,
        );
        let _ = self
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::Completed,
                agent: None,
                stage: WorkspaceStage::Completed,
                round: None,
                title: "WorkItemPlan 已确认".to_string(),
                summary: Some(format!(
                    "plan {} confirmed，已建立 {} 个子 WorkItem session",
                    plan.id,
                    new_sessions.len()
                )),
                status: TimelineNodeStatus::Completed,
            })
            .await;

        Ok((plan, new_sessions))
    }

    /// Confirmed 写入前的聚合 Spec 门禁。Story/Design 的旧 HumanConfirm 路径与
    /// main 引入的 `finalize_current_artifact` 路径均复用它，避免主干的对话式确认
    /// 绕过多仓 involved/change_order 校验。
    pub(crate) fn validate_confirm_aggregate_spec_gate(&self) -> Result<(), String> {
        // Blocker 2 修复：confirm gate 下沉到 product 层（LifecycleStore 的
        // validate_confirm_aggregate_spec），在置 Confirmed **前**拦截多仓 Story/Design
        // 校验失败（involved 空 / 多仓 Design 缺 change_order），杜绝 WebSocket 绕过
        // REQ-PLN-04/05 进入 prepare。失败返回 Err，不置 Confirmed（单仓不校验，红线）。
        if matches!(
            self.session.workspace_type,
            WorkspaceType::Story | WorkspaceType::Design
        ) && let Some(store) = &self.lifecycle_store
        {
            store
                .validate_confirm_aggregate_spec(
                    &self.session.project_id,
                    &self.session.issue_id,
                    &self.session.entity_id,
                    &self.session.workspace_type,
                )
                .map_err(|error| {
                    format!(
                        "confirm gate 校验失败：{}: {}",
                        error.stable_code(),
                        error.message()
                    )
                })?;
        }
        Ok(())
    }

    /// WorkItemPlan 确认：plan/work_items Draft -> Confirmed，并幂等创建子 WorkItem session。
    pub(crate) async fn confirm_work_item_plan(
        &mut self,
    ) -> Result<(IssueWorkItemPlan, Vec<WorkspaceSessionRecord>), String> {
        let lifecycle = self
            .lifecycle_store
            .clone()
            .ok_or("lifecycle_store unavailable")?;
        let project_id = self.session.project_id.clone();
        let issue_id = self.session.issue_id.clone();
        let plan_id = self.session.entity_id.clone();

        let current_plan = lifecycle
            .get_issue_work_item_plan(&project_id, &issue_id, &plan_id)
            .map_err(|e| format!("load plan failed: {e}"))?;
        if current_plan.work_item_ids.is_empty() {
            return Err(
                "cannot confirm WorkItemPlan without compiled WorkItem records; run Final Compile successfully first"
                    .to_string(),
            );
        }
        let plan = match current_plan.status {
            crate::product::models::IssueWorkItemPlanStatus::Draft => {
                lifecycle
                    .confirm_issue_work_item_plan(&project_id, &issue_id, &plan_id)
                    .map_err(|e| format!("confirm plan failed: {e}"))?
                    .0
            }
            crate::product::models::IssueWorkItemPlanStatus::Confirmed => current_plan,
            crate::product::models::IssueWorkItemPlanStatus::ChangeRequested => {
                return Err("cannot confirm a change_requested WorkItemPlan".to_string());
            }
        };

        let _created_sessions = lifecycle
            .ensure_work_item_sessions_for_plan(
                &project_id,
                &issue_id,
                &plan_id,
                self.session.author_provider.clone(),
                self.session.reviewer_provider.clone(),
                self.session.review_rounds,
                self.session.superpowers_enabled,
                self.session.openspec_enabled,
            )
            .map_err(|e| format!("ensure child sessions failed: {e}"))?;
        let plan_work_item_ids: HashSet<String> = plan.work_item_ids.iter().cloned().collect();
        let child_sessions = lifecycle
            .list_workspace_sessions(&project_id, &issue_id)
            .map_err(|e| format!("list child sessions failed: {e}"))?
            .into_iter()
            .filter(|session| {
                session.workspace_type == WorkspaceType::WorkItem
                    && plan_work_item_ids.contains(&session.entity_id)
            })
            .collect::<Vec<_>>();

        if let Some(store) = &self.lifecycle_store
            && self.session.flow_kind
                == crate::product::work_item_plan_policy::WorkItemPlanFlowKind::SingleCandidate
        {
            // SC 计划批准链：Confirmed 落库必须保留 human_gate_snapshot（D11 预算
            // 接续原 session 快照的单一预算源；D16 原 plan session 是唯一门宿主）。
            // REQ-GCE-03 修订链重开同一门依赖该快照在场，因此这里用只改 status 的
            // CAS 写入，绕开通用终态清理（update_workspace_session_status 会清快照）。
            // 非_SC/legacy 路径继续走通用清理，语义不变。
            if let Ok(expected) = store.get_workspace_session(&self.session.session_id)
                && let Ok(record) = store.compare_and_update_workspace_session_status(
                    &expected,
                    WorkspaceSessionStatus::Confirmed,
                )
            {
                self.session.session_status = record.status;
            }
        } else if let Some(store) = &self.lifecycle_store
            && let Ok(record) = store.update_workspace_session_status(
                &self.session.session_id,
                WorkspaceSessionStatus::Confirmed,
            )
        {
            self.session.session_status = record.status;
        }

        Ok((plan, child_sessions))
    }

    pub fn handle_abort(&mut self) {
        // 诊断打点（claude×轻 握手谜团第 2 轮，不改行为）：engine 级 abort API
        // （当前 workspace web 流未接入，保留位点以防未来接入时不可见）。
        eprintln!(
            "[aria-cancellation] workspace engine_handle_abort trigger=engine_handle_abort session_id={} role=none",
            self.session.session_id
        );
        self.cancel.cancel();
    }

    pub fn set_provider(&mut self, role: &str, provider: ProviderName) -> Result<(), String> {
        if self.session.stage != WorkspaceStage::PrepareContext {
            return Err("provider selection is locked after generation starts".to_string());
        }

        match role {
            "author" => {
                self.session.permission_modes.author = permission_mode_for_provider(
                    &provider,
                    self.session.permission_modes.author.clone(),
                );
                self.session.author_provider = provider;
                Ok(())
            }
            "reviewer" => {
                self.session.permission_modes.reviewer = permission_mode_for_provider(
                    &provider,
                    self.session.permission_modes.reviewer.clone(),
                );
                self.session.reviewer_provider = Some(provider);
                Ok(())
            }
            _ => Err(format!("unknown provider role: {role}")),
        }?;

        if let Some(store) = &self.lifecycle_store {
            // C2 Task 5（REQ-CRO-05）：三值透传——缺 reviewer 持久化空 effective。
            store
                .update_workspace_session_providers(
                    &self.session.session_id,
                    self.session.author_provider.clone(),
                    self.session.reviewer_provider.clone(),
                )
                .map_err(|error| format!("persist provider selection failed: {error}"))?;
            store
                .update_workspace_session_permission_modes(
                    &self.session.session_id,
                    self.session.permission_modes.clone(),
                )
                .map_err(|error| format!("persist permission mode selection failed: {error}"))?;
        }

        Ok(())
    }

    /// 普通工件版本写入（SC 失败包 A4 之外的普通路径）。
    ///
    /// 版本分配与落盘在 `LifecycleStore::with_artifact_versions_write` 的同锁 closure 内
    /// 完成：锁内以磁盘真值分配 `max(raw.version)+1`、合并新版本、整表保存。锁、准入、
    /// raw 读取、版本号溢出与保存任一失败，都在提交引擎内存、登记 ref、广播事件之前
    /// 返回 `Err`（`artifact_versions_write_failed: ...`），调用方必须显式处理。
    ///
    /// 保存成功后以磁盘合并结果为权威提交 `session.artifact` / `artifact_versions`；
    /// 其后的 ref 登记与事件广播保持既有 best-effort 语义（本次不顺带收紧全局）。
    pub async fn update_artifact(&mut self, payload: ArtifactPayload) -> Result<ArtifactRef, String> {
        let session_id = self.session.session_id.clone();
        let generated_by = self.session.author_provider.clone();
        let source_node_id = self
            .active_node_id
            .clone()
            .unwrap_or_else(|| "timeline_node_unknown".to_string());
        let created_at = chrono::Utc::now().to_rfc3339();

        let (version, saved_versions) = match self.lifecycle_store.as_ref() {
            Some(store) => {
                let payload_for_write = payload.clone();
                store
                    .with_artifact_versions_write(&session_id, |ctx| {
                        // 锁内分配：max(raw)+1，溢出由 next_version 返回类型化错误。
                        let version = ctx.next_version()?;
                        let mut proposed = ctx.raw().to_vec();
                        for entry in &mut proposed {
                            // 普通旧版本退出 current；A4 条目恒为 is_current=false，
                            // 这里不会改动其 metadata。
                            entry.is_current = false;
                        }
                        proposed.push(ArtifactVersion {
                            version,
                            payload: payload_for_write.clone(),
                            generated_by: generated_by.clone(),
                            reviewed_by: None,
                            review_verdict: None,
                            confirmed_by: None,
                            is_current: true,
                            created_at: created_at.clone(),
                            source_node_id: source_node_id.clone(),
                        });
                        let saved = ctx.save(&proposed)?;
                        Ok((version, saved))
                    })
                    .map_err(|error| format!("artifact_versions_write_failed: {error}"))?
            }
            None => {
                // 无持久存储时只维护内存版本表：同样是 checked max+1，溢出直接报错，
                // 不伪造持久化回退。
                let version = match self.artifact_versions.iter().map(|entry| entry.version).max() {
                    None => 1,
                    Some(max) => max.checked_add(1).ok_or_else(|| {
                        "artifact_versions_write_failed: artifact_versions version space exhausted"
                            .to_string()
                    })?,
                };
                let mut proposed = self.artifact_versions.clone();
                for entry in &mut proposed {
                    entry.is_current = false;
                }
                proposed.push(ArtifactVersion {
                    version,
                    payload: payload.clone(),
                    generated_by: generated_by.clone(),
                    reviewed_by: None,
                    review_verdict: None,
                    confirmed_by: None,
                    is_current: true,
                    created_at: created_at.clone(),
                    source_node_id: source_node_id.clone(),
                });
                (version, proposed)
            }
        };

        // 保存成功后才提交引擎内存（磁盘合并结果为权威）。
        self.session.artifact = Some(payload.clone());
        self.artifact_versions = saved_versions;
        let artifact_ref = ArtifactRef {
            artifact_id: format!("artifact_version_{version:03}"),
            version,
        };
        let _ = self
            .persist_artifact_ref(&source_node_id, artifact_ref.clone())
            .await;
        let _ = self
            .event_tx
            .send(EngineEvent::ArtifactUpdate {
                version,
                payload: payload.clone(),
            })
            .await;
        Ok(artifact_ref)
    }
}

#[cfg(test)]
mod update_artifact_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use tempfile::TempDir;

    /// 持久引擎 fixture：真实 `LifecycleStore` + 真实 checkpoint store；保留事件接收端，
    /// 便于断言失败路径没有广播 `ArtifactUpdate`。
    fn persistent_engine() -> (
        TempDir,
        LifecycleStore,
        WorkspaceEngine,
        mpsc::Receiver<EngineEvent>,
        String,
    ) {
        let tmp = TempDir::new().unwrap();
        let checkpoint_store = Arc::new(CheckpointStore::new(tmp.path().to_path_buf()));
        let lifecycle_store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
        let (tx, rx) = mpsc::channel(64);
        let session_record = lifecycle_store
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: "project_0001".to_string(),
                issue_id: "issue_0001".to_string(),
                entity_id: "story_spec_0001".to_string(),
                workspace_type: WorkspaceType::Story,
                author_provider: ProviderName::ClaudeCode,
                reviewer_provider: Some(ProviderName::Codex),
                review_rounds: 2,
                superpowers_enabled: true,
                openspec_enabled: true,
                work_item_plan_options: None,
            })
            .unwrap();
        let session_id = session_record.id.clone();
        let session = WorkspaceSession::from_record(session_record);
        let engine = WorkspaceEngine::new_persistent(
            checkpoint_store,
            lifecycle_store.clone(),
            tx,
            session,
        );
        (tmp, lifecycle_store, engine, rx, session_id)
    }

    /// `artifact_versions.json` 的真实落盘路径（由 store 自己解析，避免测试写死目录布局）。
    fn artifact_versions_path(store: &LifecycleStore, session_id: &str) -> std::path::PathBuf {
        store
            .workspace_timeline_root_for_session(session_id)
            .expect("workspace timeline root")
            .join("artifact_versions.json")
    }

    fn story_payload() -> ArtifactPayload {
        ArtifactPayload::Markdown {
            markdown: "# Story Spec".to_string(),
            diff: None,
        }
    }

    fn version_at_max() -> ArtifactVersion {
        ArtifactVersion {
            version: u32::MAX,
            payload: story_payload(),
            generated_by: ProviderName::ClaudeCode,
            reviewed_by: None,
            review_verdict: None,
            confirmed_by: None,
            is_current: true,
            created_at: "2026-01-01T00:00:00Z".to_string(),
            source_node_id: "timeline_node_001".to_string(),
        }
    }

    fn saw_artifact_update(rx: &mut mpsc::Receiver<EngineEvent>) -> bool {
        let mut saw = false;
        while let Ok(event) = rx.try_recv() {
            if matches!(event, EngineEvent::ArtifactUpdate { .. }) {
                saw = true;
            }
        }
        saw
    }

    #[tokio::test]
    async fn update_artifact_surfaces_raw_read_failure_without_touching_memory() {
        let (_tmp, store, mut engine, mut rx, session_id) = persistent_engine();
        let versions_path = artifact_versions_path(&store, &session_id);
        std::fs::create_dir_all(versions_path.parent().unwrap()).unwrap();
        let corrupt = b"{ this is not json";
        std::fs::write(&versions_path, corrupt).unwrap();

        let node_id = engine
            .create_timeline_node(TimelineNodeDraft {
                node_type: TimelineNodeType::AuthorRun,
                agent: Some(ProviderName::ClaudeCode),
                stage: WorkspaceStage::Running,
                round: None,
                title: "Story 生成".to_string(),
                summary: None,
                status: TimelineNodeStatus::Active,
            })
            .await;

        let error = engine
            .update_artifact(story_payload())
            .await
            .expect_err("损坏的 artifact_versions raw 必须上浮为 Err");

        assert!(
            error.starts_with("artifact_versions_write_failed:"),
            "错误必须带步骤前缀：{error}"
        );
        assert!(engine.session.artifact.is_none(), "失败不得提交 session.artifact");
        assert!(
            engine.artifact_versions.is_empty(),
            "失败不得提交 engine.artifact_versions"
        );
        assert_eq!(
            std::fs::read(&versions_path).unwrap(),
            corrupt.to_vec(),
            "失败不得改写磁盘真值"
        );
        let registered_ref = store
            .load_node_detail(&session_id, &node_id)
            .ok()
            .and_then(|detail| detail.artifact_ref);
        assert!(
            registered_ref.is_none(),
            "失败路径不得登记 artifact ref：{registered_ref:?}"
        );
        assert!(
            !saw_artifact_update(&mut rx),
            "失败路径不得广播 ArtifactUpdate"
        );
    }

    #[tokio::test]
    async fn update_artifact_surfaces_version_space_exhaustion() {
        let (_tmp, store, mut engine, mut rx, session_id) = persistent_engine();
        store
            .save_artifact_versions(&session_id, &[version_at_max()])
            .unwrap();

        let error = engine
            .update_artifact(story_payload())
            .await
            .expect_err("版本号溢出必须上浮为 Err");

        assert!(
            error.starts_with("artifact_versions_write_failed:"),
            "错误必须带步骤前缀：{error}"
        );
        assert!(
            error.contains("version_space_exhausted"),
            "错误必须指明版本空间耗尽：{error}"
        );
        assert!(engine.session.artifact.is_none(), "失败不得提交 session.artifact");
        assert!(
            engine.artifact_versions.is_empty(),
            "失败不得提交 engine.artifact_versions"
        );
        assert!(
            !saw_artifact_update(&mut rx),
            "失败路径不得广播 ArtifactUpdate"
        );
    }

    #[tokio::test]
    async fn update_artifact_without_lifecycle_store_rejects_version_overflow() {
        let (_tmp, _store, mut engine, mut rx, _session_id) = persistent_engine();
        engine.lifecycle_store = None;
        engine.artifact_versions = vec![version_at_max()];

        let error = engine
            .update_artifact(story_payload())
            .await
            .expect_err("无持久存储时版本号溢出必须上浮为 Err");

        assert!(
            error.starts_with("artifact_versions_write_failed:"),
            "错误必须带步骤前缀：{error}"
        );
        assert!(engine.session.artifact.is_none(), "失败不得提交 session.artifact");
        assert_eq!(
            engine.artifact_versions.len(),
            1,
            "失败不得改写既有内存版本表"
        );
        assert!(
            !saw_artifact_update(&mut rx),
            "失败路径不得广播 ArtifactUpdate"
        );
    }

    #[tokio::test]
    async fn update_artifact_persists_version_then_commits_memory() {
        let (_tmp, store, mut engine, mut rx, session_id) = persistent_engine();

        let artifact_ref = engine
            .update_artifact(story_payload())
            .await
            .expect("健康路径的 artifact 写入必须成功");

        assert_eq!(artifact_ref.version, 1);
        assert_eq!(artifact_ref.artifact_id, "artifact_version_001");
        assert!(
            matches!(
                engine.session.artifact.as_ref(),
                Some(ArtifactPayload::Markdown { .. })
            ),
            "成功后才提交 session.artifact"
        );
        assert_eq!(engine.artifact_versions.len(), 1);
        assert!(engine.artifact_versions[0].is_current);
        let persisted = store.list_artifact_versions(&session_id).unwrap();
        assert_eq!(persisted.len(), 1);
        assert_eq!(persisted[0].version, 1);
        assert!(
            artifact_versions_path(&store, &session_id).exists(),
            "成功路径必须真实落盘"
        );
        assert!(
            saw_artifact_update(&mut rx),
            "成功路径必须广播 ArtifactUpdate"
        );
    }
}
