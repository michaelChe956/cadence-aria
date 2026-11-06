impl WorkspaceEngine {
    fn plan_reviewer_window_store(
        &self,
    ) -> Result<crate::product::reviewer_window_store::ReviewerWindowStore, String> {
        let store = self.lifecycle_store.as_ref().ok_or_else(|| {
            "plan_review_window_unavailable: no persistent lifecycle store".to_string()
        })?;
        Ok(
            crate::product::reviewer_window_store::ReviewerWindowStore::new(
                store
                    .app_paths()
                    .root()
                    .join(crate::product::reviewer_window_store::REVIEWER_WINDOW_STORE_DIR),
            ),
        )
    }

    fn plan_reviewer_snapshot_store(
        &self,
    ) -> Result<crate::product::reviewer_snapshot::ReviewerSnapshotStore, String> {
        let store = self.lifecycle_store.as_ref().ok_or_else(|| {
            "plan_review_window_unavailable: no persistent lifecycle store".to_string()
        })?;
        Ok(
            crate::product::reviewer_snapshot::ReviewerSnapshotStore::new(
                store
                    .app_paths()
                    .root()
                    .join(PLAN_REVIEWER_SNAPSHOT_STORE_DIR),
            ),
        )
    }

    /// 规划面 quiesce 目标:主 scope=本次 review run(空登记 Quiescent 即
    /// 静止证明;与 coding 面同构,不含 aria.worktree 协调 scope——规划
    /// 面无 Coder 并行窗登记)。B1-1:同一 scope 亦是规划 reviewer launch
    /// 的受信归属来源(`begin_plan_review_window` 持有,原开窗后即丢弃)。
    fn plan_review_main_scope(
        &self,
        window_id: &str,
    ) -> Result<crate::cross_cutting::provider_resources::ProviderRunResourceScope, String> {
        use crate::cross_cutting::provider_resources::{
            ProviderRunResourceScope, ResourceScopeKey,
        };
        let store = self.lifecycle_store.as_ref().ok_or_else(|| {
            "plan_review_window_unavailable: no persistent lifecycle store".to_string()
        })?;
        let private_root = store
            .app_paths()
            .root()
            .join("plan-review-scopes")
            .join(window_id);
        std::fs::create_dir_all(&private_root)
            .map_err(|error| format!("plan_review_scope_create_failed: {error}"))?;
        let key = ResourceScopeKey {
            scope_id: format!("plan-review-{window_id}"),
            run_id: self
                .active_run_id
                .clone()
                .unwrap_or_else(|| self.session.session_id.clone()),
            boot_identity: format!("aria-engine-{}", std::process::id()),
        };
        ProviderRunResourceScope::open_for_recovery(
            &key,
            Some(window_id),
            &private_root,
            &store
                .app_paths()
                .root()
                .join(PLAN_PROVIDER_RESOURCE_STORE_DIR),
            None,
        )
        .map_err(|error| format!("plan_review_scope_open_failed: {error}"))
    }

    /// B1-1(波 2):规划 reviewer 本次 launch 的受信资源归属铸造——从
    /// 活动窗持有的 plan-review 主 scope(开窗实际归属:同 window/run/
    /// boot)生成唯一资源 ID 载体;首发/repair/静默重试与 gateway/direct
    /// 两形态共用。无活动窗(测试 seam)= `None`(真实 adapter 由窗口
    /// 守卫拒绝,不以无归属静默降级为生产语义)。
    fn plan_review_launch_ownership(
        &self,
    ) -> Option<crate::cross_cutting::provider_resources::TrustedLaunchOwnership> {
        self.active_review_window.as_ref().map(|active| {
            crate::cross_cutting::provider_resources::TrustedLaunchOwnership::mint(
                std::sync::Arc::clone(&active.launch_scope),
                &format!("plan review provider process: {}", active.provider),
            )
        })
    }

    /// 规划审查窗口目标解析——与 `review_launch_target_and_cwd` 同源起步:
    /// 目标带 `.git` 原样返回(单仓唯一出口,也是成员 checkout/direct 形态);
    /// 聚合根视图(非 git 的 provider_context_root,Story focus=None/Design
    /// involved 空/WorkItemPlan 多 target 非唯一三形态可达)时经实体四事实
    /// resolver 落到本次评审实际的唯一/focus 成员 checkout;**无唯一成员不再
    /// 回落任何单一仓库**(REQ-PLN-11 设计 A 条目 1:多仓已无「主仓/主成员」
    /// 概念)——返回专用错误码(聚合根规划评审窗口目标待 owner 裁定),不得
    /// 以 IdentityDrift 形态误报。
    fn plan_review_window_worktree(&self) -> Result<std::path::PathBuf, String> {
        let target = self
            .session
            .repository_path
            .clone()
            .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());
        if target.join(".git").exists() {
            return Ok(target);
        }
        // 聚合根视图形态:必须解析出唯一/focus 业务仓;聚合视图(label 级)
        // 与无唯一成员同属"无窗口对账域",一律专用码停等。
        let Some(store) = self.lifecycle_store.as_ref() else {
            return Err(format!(
                "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: aggregate root view without a persistent lifecycle store; owner ruling pending"
            ));
        };
        let business =
            crate::product::workspace_repository::resolve_workspace_repository_context_for_entity(
                &store.app_paths(),
                store,
                &self.session.project_id,
                &self.session.issue_id,
                &self.session.entity_id,
                self.session.workspace_type.clone(),
            )
            .and_then(|context| context.business_repository);
        let repository = match business {
            Ok(repository)
                if !crate::product::workspace_repository::is_aggregate_root_view_repository(
                    &repository,
                ) =>
            {
                repository
            }
            Ok(_) => {
                return Err(format!(
                    "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: aggregate root view without a unique member repository; owner ruling pending"
                ));
            }
            Err(error) => {
                return Err(format!(
                    "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: aggregate root view without a unique member repository (business resolution failed: {error}); owner ruling pending"
                ));
            }
        };
        if !repository.path.join(".git").exists() {
            return Err(format!(
                "{PLAN_REVIEW_WINDOW_TARGET_UNRESOLVED}: resolved member checkout {} is not a git worktree; owner ruling pending",
                repository.path.display()
            ));
        }
        Ok(repository.path)
    }

    /// 入口开窗(T17 冻结顺序):必须早于 `ensure_review_invocation_scope`/
    /// `build_review_input`/`load_plan_review_context` 的一切材料采集。
    /// 失败 fail-closed——不以无窗继续 review。
    pub(crate) async fn begin_plan_review_window(
        &mut self,
        reviewer: &ProviderName,
        worktree: std::path::PathBuf,
    ) -> Result<(), String> {
        // 窗口 ID 全局唯一:走 `role_run_window_id`(attempt 位=workspace
        // session id,role run 位=plan run id)。此前此处自持
        // `plan-<run>-w01` 格式化副本——窗口 store 目录跨会话共享,
        // 不同会话的同号 plan run 会算出同一 ID 并撞车(旧记录命中 →
        // `reviewer_window_conflict`,plan review fail-closed 走不下去)。
        let window_id = crate::product::reviewer_window_store::role_run_window_id(
            &self.session.session_id,
            self.active_run_id
                .as_deref()
                .unwrap_or(&self.session.session_id),
        );
        let worktree = worktree.canonicalize().unwrap_or(worktree);
        let provider = serde_json::to_string(reviewer)
            .map(|raw| raw.trim_matches('"').to_string())
            .unwrap_or_else(|_| format!("{reviewer:?}"));
        // B1-1:开窗打开的主 scope 以 Arc 持有——quiesce 目标借引用消费,
        // 窗口生命周期内保留给规划 reviewer launch 铸造受信归属(此前
        // 开窗消费后即丢弃)。
        let main_scope = std::sync::Arc::new(self.plan_review_main_scope(&window_id)?);
        let target_refs = [&*main_scope];
        let request = crate::product::reviewer_window::ReviewerWindowBeginRequest {
            window_id: &window_id,
            attempt_id: &self.session.session_id,
            run_id: self
                .active_run_id
                .as_deref()
                .unwrap_or(&self.session.session_id),
            provider: &provider,
            worktree: &worktree,
            quiesce_targets: &target_refs,
            platform_write_paths: Vec::new(),
            budget: crate::product::reviewer_snapshot::SnapshotBudget {
                max_entries: 200_000,
                max_total_bytes: 2 * 1024 * 1024 * 1024,
            },
            drain_timeout: std::time::Duration::from_secs(60),
        };
        let window = crate::product::reviewer_window::begin_review_window(
            &self.plan_reviewer_window_store()?,
            &self.plan_reviewer_snapshot_store()?,
            &request,
        )
        .await
        .map_err(|error| format!("plan_review_window_begin_failed: {error}"))?;
        let run_id = self
            .active_run_id
            .clone()
            .unwrap_or_else(|| self.session.session_id.clone());
        self.active_review_window = Some(ActivePlanReviewWindow {
            handle: window,
            worktree,
            provider,
            run_id,
            launch_scope: main_scope,
        });
        Ok(())
    }

    /// 材料采集/provider 启动前同源复验(窗口 Running+指纹一致)。
    fn assert_plan_review_window(&self) -> Result<(), String> {
        // Task 2(REQ-ENV-14/PLN-11):同源复验点形态分派——多仓(lc)形态
        // 无评审窗,复验「材料集合可解析且非空」;单仓保持窗口 Running+
        // 材料指纹一致复验,函数体逐字不变。
        if self.is_logical_codebase_issue() {
            return self.assert_plan_review_material();
        }
        let Some(active) = self.active_review_window.as_ref() else {
            return Ok(());
        };
        let expectation = crate::product::reviewer_window::WindowExpectation {
            window_id: &active.handle.window_id,
            attempt_id: &self.session.session_id,
            run_id: &active.run_id,
            provider: &active.provider,
            worktree: &active.worktree,
        };
        let proof = crate::product::reviewer_window::assert_review_window_current(
            &self.plan_reviewer_window_store()?,
            &self.plan_reviewer_snapshot_store()?,
            &expectation,
            &[crate::product::reviewer_window_store::ReviewerWindowState::Running],
        )
        .map_err(|error| format!("plan_review_window_not_current: {error}"))?;
        if proof.manifest_sha256 != active.handle.manifest_sha256 {
            return Err("plan_review_window_material_fingerprint_drift".to_string());
        }
        Ok(())
    }

    /// Task 2(REQ-ENV-14/PLN-11):多仓(lc)形态材料断言——无评审窗路径
    /// 下,provider 启动前以「参与仓库集合可解析且非空」替代窗口 Running
    /// 复验。lifecycle store 缺失、解析失败或集合为空一律 fail-closed
    /// (`plan_review_material_unresolved: <原因>`),绝不以空材料静默
    /// 继续,也不产任何 plan_review_window_* 错误码。
    fn assert_plan_review_material(&self) -> Result<(), String> {
        let Some(store) = self.lifecycle_store.as_ref() else {
            return Err("plan_review_material_unresolved: lifecycle_store_unavailable".to_string());
        };
        let resolution =
            crate::product::logical_codebase::PlanningContextSetResolver::new(store.app_paths())
                .resolve(
                    self.session.project_id.as_str(),
                    self.session.issue_id.as_str(),
                )
                .map_err(|cause| format!("plan_review_material_unresolved: {cause}"))?;
        if resolution.set.is_empty() {
            return Err(
                "plan_review_material_unresolved: resolved member set is empty".to_string(),
            );
        }
        Ok(())
    }

    /// reviewer 输入的 ReviewerVerify 窗口合同铸造(prompt 八点共用的
    /// 唯一窗口来源)。REQ-ENV-13(设计 A 条目 17)三分支:
    /// ① 有活动窗口 → 窗口 id(单仓路径,canonical 摘要逐字不变);
    /// ② 无窗口且多仓(lc)形态 → 每会话 UUID 身份(同一会话跨轮次/
    ///    repair 恒定,digest 域即该 UUID);
    /// ③ 无窗口且单仓 → None(由四家启动检查/网关复验拒绝,不得借
    ///    会话身份放行)。
    pub(crate) fn reviewer_verify_policy(&self) -> Option<ProviderToolPolicy> {
        if let Some(active) = self.active_review_window.as_ref() {
            return Some(ProviderToolPolicy::reviewer_verify(
                active.handle.window_id.clone(),
            ));
        }
        if self.is_logical_codebase_issue() {
            return Some(ProviderToolPolicy::reviewer_verify(
                self.session.review_identity_id.clone(),
            ));
        }
        None
    }

    /// 收口:干净恢复+释放(技术回收;规划面 verdict 无报告采信投影,
    /// 消费面采信由后续交付门承担——与 legacy group 面同口径披露)。
    ///
    /// F11(波3):一切「收口未完成」出口(store 失效/收口保锁/恢复
    /// 人工门/弃置失败)在 tracing::warn 之外同步经既有 EngineEvent
    /// 事件流发用户可见注意事件(带 window_id/原因/人工入口)。锁与
    /// generation 语义不变——投影不解锁、不迁移状态、不丢窗口引用;
    /// workspace plan 面以 session 为锚,不照搬 coding 专用
    /// `project_window_manual_attention`(那依赖 coding attempt 状态机)。
    async fn close_plan_review_window(&mut self, reason: &str) {
        let Some(active) = self.active_review_window.take() else {
            return;
        };
        let window = active.handle;
        let session_id = self.session.session_id.clone();
        let (Ok(window_store), Ok(snapshot_store)) = (
            self.plan_reviewer_window_store(),
            self.plan_reviewer_snapshot_store(),
        ) else {
            tracing::warn!("plan_review_window_close_store_unavailable");
            Self::emit_plan_review_window_attention(
                &self.event_tx,
                &session_id,
                &window.window_id,
                "close_store_unavailable",
                &format!("收口原因「{reason}」;持久 lifecycle store 不可用,窗口未收口"),
                "恢复 lifecycle store 后重试收口;窗口记录 reviewer-windows/windows/<本窗>.json(<aria> 下),锁与 generation 保持",
            )
            .await;
            return;
        };
        let record = match crate::product::reviewer_window::begin_quiesce(
            &window_store,
            &window.window_id,
            window.generation,
            reason,
        ) {
            Ok(record) => record,
            Err(error) => {
                tracing::warn!(%error, window = %window.window_id, "plan_review_window_close_kept_lock");
                Self::emit_plan_review_window_attention(
                    &self.event_tx,
                    &session_id,
                    &window.window_id,
                    "close_kept_lock",
                    &format!("begin_quiesce 失败: {error};窗口保持锁未收口"),
                    "核对窗口记录后人工重试收口或弃置;记录 reviewer-windows/windows/<本窗>.json(<aria> 下)",
                )
                .await;
                return;
            }
        };
        match crate::product::reviewer_window::restore_window(
            &window_store,
            &snapshot_store,
            &window.window_id,
            record.generation,
        )
        .await
        {
            Ok(summary) => {
                if summary.manual_gate {
                    tracing::warn!(
                        window = %window.window_id,
                        "plan_review_window_violation_manual_gate"
                    );
                    Self::emit_plan_review_window_attention(
                        &self.event_tx,
                        &session_id,
                        &window.window_id,
                        "manual_gate",
                        "恢复完整但违规事实已落 journal:整轮作废,需人工裁决后才能关闭窗口",
                        "人工裁决 Invalidate(作废关闭)或 RequireNewReview(作废并开新一轮评审);违规 journal reviewer-windows/violations/<本窗>.json(<aria> 下);裁决前锁保持",
                    )
                    .await;
                    return;
                }
                if let Err(error) = crate::product::reviewer_window::abandon_window(
                    &window_store,
                    &window.window_id,
                    summary.record.generation,
                    reason,
                ) {
                    tracing::warn!(%error, window = %window.window_id, "plan_review_window_abandon_failed");
                    Self::emit_plan_review_window_attention(
                        &self.event_tx,
                        &session_id,
                        &window.window_id,
                        "abandon_failed",
                        &format!("恢复已完成但弃置收口失败: {error};窗口保持非终态"),
                        "重试收口或人工处置;窗口记录 reviewer-windows/windows/<本窗>.json(<aria> 下)",
                    )
                    .await;
                }
            }
            Err(error) => {
                tracing::warn!(%error, window = %window.window_id, "plan_review_window_close_kept_lock");
                Self::emit_plan_review_window_attention(
                    &self.event_tx,
                    &session_id,
                    &window.window_id,
                    "close_kept_lock",
                    &format!("恢复失败: {error};窗口保持锁未收口"),
                    "处置后重入收口续接;窗口记录 reviewer-windows/windows/<本窗>.json(<aria> 下)",
                )
                .await;
            }
        }
    }

    /// F11(波3):规划评审窗口的用户可见注意事件——复用既有
    /// EngineEvent::ExecutionEvent 事件流,status=Failed 表「需人工处理」。
    /// 发送 best-effort(`let _`,与既有事件发射一致),不影响窗口状态机。
    async fn emit_plan_review_window_attention(
        event_tx: &mpsc::Sender<EngineEvent>,
        session_id: &str,
        window_id: &str,
        attention: &str,
        reason: &str,
        manual_entry: &str,
    ) {
        let _ = event_tx
            .send(EngineEvent::ExecutionEvent {
                event: crate::cross_cutting::streaming_provider::ProviderExecutionEvent {
                    event_id: format!(
                        "plan_review_window_attention_{attention}_{session_id}_{window_id}"
                    ),
                    kind: crate::cross_cutting::streaming_provider::ProviderExecutionEventKind::Provider,
                    status: crate::cross_cutting::streaming_provider::ProviderExecutionEventStatus::Failed,
                    title: format!("规划评审窗口需人工处理({attention})"),
                    detail: Some(format!(
                        "window_id: {window_id}\n原因: {reason}\n人工入口: {manual_entry}"
                    )),
                    command: None,
                    cwd: None,
                    output: None,
                    exit_code: None,
                },
                node_id: None,
                agent: None,
            })
            .await;
    }
}
