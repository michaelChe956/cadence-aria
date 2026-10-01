/// 显式 bootstrap 动作服务（C4 Task 3 契约 + Task 4 已落地链路的接线）。
///
/// 动作不创建新的 durable 状态机：command replay 与推进都映射回既有
/// registration batch / aggregate initialization / aggregate index 记录。
#[derive(Clone)]
pub struct LogicalCodebaseBootstrapService {
    paths: ProductAppPaths,
    /// C4 Task 6：member-index 步骤“Running 但内存 run 不活跃”的判定探针
    ///（web 层注入 run registry 视角；缺省视为活跃——不知道就不动）。
    member_index_run_active: Option<std::sync::Arc<dyn Fn(&str, &str, &str) -> bool + Send + Sync>>,
    /// G3（终局关闸缺口）：aggregate_index_active 步 Retry 的重建派发器
    ///（web 层注入 LC 隔离的 `AggregateIndexOperation::build_with_command_id`
    /// 闭包；缺省 None 保持既有仅重放语义——NotFound fail-closed）。
    aggregate_index_rebuild: Option<AggregateIndexRebuildDispatcher>,
}

/// G3：重建派发器契约——`(project_id, command_id,
/// expected_membership_revision)` → 落盘后的聚合索引记录。生产实现是
/// `build_with_command_id`（同 command 幂等重放、revision 冲突 fail-closed）。
pub type AggregateIndexRebuildDispatcher = std::sync::Arc<
    dyn Fn(
            &str,
            &str,
            u64,
        ) -> Result<
            crate::product::logical_codebase::aggregate_index::AggregateIndexRecord,
            AggregateIndexError,
        > + Send
        + Sync,
>;

impl LogicalCodebaseBootstrapService {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self {
            paths,
            member_index_run_active: None,
            aggregate_index_rebuild: None,
        }
    }

    /// 注入 member-index run 活跃探针（参数：project/lc/operation id）。
    pub fn with_member_index_run_probe(
        mut self,
        probe: std::sync::Arc<dyn Fn(&str, &str, &str) -> bool + Send + Sync>,
    ) -> Self {
        self.member_index_run_active = Some(probe);
        self
    }

    /// 注入 aggregate_index_active 步 Retry 的重建派发器（G3）。
    pub fn with_aggregate_index_rebuild(
        mut self,
        dispatcher: AggregateIndexRebuildDispatcher,
    ) -> Self {
        self.aggregate_index_rebuild = Some(dispatcher);
        self
    }

    pub async fn apply(
        &self,
        request: BootstrapActionRequest,
    ) -> Result<BootstrapActionResult, BootstrapActionError> {
        let projection = tokio::task::spawn_blocking({
            let paths = self.paths.clone();
            let project_id = request.project_id.clone();
            let logical_codebase_id = request.logical_codebase_id.clone();
            move || {
                LogicalCodebaseBootstrapProjector::new(paths)
                    .project(&project_id, &logical_codebase_id)
            }
        })
        .await
        .map_err(|error| {
            BootstrapActionError::Store(ProductStoreError::Io(format!(
                "bootstrap projection task failed: {error}"
            )))
        })??;

        // expected revision 门：过期 revision 一律 fail-closed，不产生新事实。
        if let Some(expected_revision) = request.expected_revision {
            let current = projection.membership_revision;
            if current != Some(expected_revision) {
                return Err(BootstrapActionError::Conflict {
                    code: "bootstrap_stale_revision".to_string(),
                    detail: format!(
                        "expected membership revision {expected_revision} but durable projection carries {current:?}"
                    ),
                });
            }
        }

        // G3：aggregate_index_active 的 Retry 重建派发会同步执行 CodeGraph
        // CLI（有界预算内可达数分钟）——放 blocking 线程池，不占 async
        // worker；其余步骤保持同步派发（重放判定只读，无长阻塞）。
        let outcome = if request.step == LogicalCodebaseBootstrapStep::AggregateIndexActive {
            let service = self.clone();
            let dispatch_request = request.clone();
            tokio::task::spawn_blocking(move || service.dispatch_action(&dispatch_request))
                .await
                .map_err(|error| {
                    BootstrapActionError::Store(ProductStoreError::Io(format!(
                        "bootstrap aggregate index dispatch task failed: {error}"
                    )))
                })?
                .map_err(BootstrapActionError::Store)?
        } else {
            self.dispatch_action(&request)
                .map_err(BootstrapActionError::Store)?
        };
        // 动作后重新投影，响应携带最新 durable 事实。
        let projection = LogicalCodebaseBootstrapProjector::new(self.paths.clone())
            .project(&request.project_id, &request.logical_codebase_id)?;
        Ok(BootstrapActionResult {
            command_id: request.command_id,
            outcome,
            projection,
        })
    }

    /// 把动作路由到既有 durable 链；不重跑已完成 provider turn。
    fn dispatch_action(
        &self,
        request: &BootstrapActionRequest,
    ) -> Result<BootstrapActionOutcome, ProductStoreError> {
        match request.step {
            LogicalCodebaseBootstrapStep::MemberIndex => self.dispatch_member_index_action(request),
            LogicalCodebaseBootstrapStep::AggregateIndexActive => {
                self.dispatch_aggregate_index_action(request)
            }
            LogicalCodebaseBootstrapStep::Identity
            | LogicalCodebaseBootstrapStep::ManifestCheckout
            | LogicalCodebaseBootstrapStep::RulesPolicy => {
                self.dispatch_registration_action(request)
            }
        }
    }

    /// member index：显式动作才推进——同 command 先查 operation 上的
    /// action 审计（replay 不再推进）；Failed 经 `reopen_for_resume` 显式
    /// 重开；Running 仅当 run 探针判定“内存 run 不活跃”（页面关闭/进程
    /// 中断）时由显式 Continue/Retry 调 `recover_interrupted` 落盘中断
    /// 事实；GET 从不触达本分支（纯投影约束）。
    fn dispatch_member_index_action(
        &self,
        request: &BootstrapActionRequest,
    ) -> Result<BootstrapActionOutcome, ProductStoreError> {
        let store =
            crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore::for_lc(
                self.paths.clone(),
                &request.logical_codebase_id,
            );
        let operation = store.get(&request.project_id, &request.expected_object_id)?;
        use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperationStatus;
        // 同 command 重放：返回同一 durable 结果，不再推进状态。
        if operation
            .action_records
            .iter()
            .any(|record| record.command_id == request.command_id)
        {
            return Ok(BootstrapActionOutcome::Replayed);
        }
        let action_applies = matches!(
            request.action,
            BootstrapActionKind::Continue | BootstrapActionKind::Retry
        );
        match operation.status {
            AggregateInitializationOperationStatus::Completed => {
                Ok(BootstrapActionOutcome::Replayed)
            }
            AggregateInitializationOperationStatus::Failed if action_applies => {
                store.reopen_for_resume(
                    &request.project_id,
                    &request.expected_object_id,
                    chrono::Utc::now().to_rfc3339(),
                )?;
                self.record_member_index_action(&store, request)?;
                Ok(BootstrapActionOutcome::Accepted)
            }
            AggregateInitializationOperationStatus::Running
                if action_applies
                    && !self.member_index_run_active(
                        &request.project_id,
                        &request.logical_codebase_id,
                        &request.expected_object_id,
                    ) =>
            {
                // 显式恢复：把中断事实落盘（含 staging 清理），随后 GET 投影
                // 出 Failed + 允许 Retry。
                store.recover_interrupted(
                    &request.project_id,
                    &request.expected_object_id,
                    chrono::Utc::now().to_rfc3339(),
                )?;
                self.record_member_index_action(&store, request)?;
                Ok(BootstrapActionOutcome::Accepted)
            }
            _ => Ok(BootstrapActionOutcome::WaitingForHuman),
        }
    }

    fn member_index_run_active(&self, project_id: &str, lc_id: &str, operation_id: &str) -> bool {
        self.member_index_run_active
            .as_ref()
            .map(|probe| probe(project_id, lc_id, operation_id))
            .unwrap_or(true)
    }

    fn record_member_index_action(
        &self,
        store: &crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore,
        request: &BootstrapActionRequest,
    ) -> Result<(), ProductStoreError> {
        store.record_action(
            &request.project_id,
            &request.expected_object_id,
            crate::product::logical_codebase::aggregate_initialization::AggregateInitializationActionRecord {
                command_id: request.command_id.clone(),
                action: format!("{:?}", request.action).to_lowercase(),
                outcome: "accepted".to_string(),
                applied_at: chrono::Utc::now().to_rfc3339(),
            },
        )?;
        Ok(())
    }

    /// aggregate index：以 Task 5 的 durable command identity 判定 replay；
    /// Building 进行中不做隐式推进（等待 single-writer 完成）。
    ///
    /// G3（终局关闸缺口）：无同 command 重放记录时不再直接 NotFound——
    /// 注入重建派发器（生产实现 `build_with_command_id`）后按请求
    /// command_id 派发重建（同 command 幂等、membership revision 冲突
    /// fail-closed、Building 并发拒绝），修复「retry 仅查重放、恒 500、
    /// 唯一恢复路径退化为 canonical rebuild 端点」的断链。未注入派发器
    /// 保持既有 NotFound（不假成功）。
    fn dispatch_aggregate_index_action(
        &self,
        request: &BootstrapActionRequest,
    ) -> Result<BootstrapActionOutcome, ProductStoreError> {
        let store = AggregateIndexStore::for_lc(self.paths.clone(), &request.logical_codebase_id);
        for record in store
            .records(&request.project_id)
            .map_err(map_index_error)?
        {
            if record.command_id.as_deref() != Some(request.command_id.as_str()) {
                continue;
            }
            return Ok(match record.status {
                AggregateIndexStatus::Building => BootstrapActionOutcome::WaitingForHuman,
                AggregateIndexStatus::Active => BootstrapActionOutcome::Completed,
                _ => BootstrapActionOutcome::Replayed,
            });
        }
        let Some(rebuild) = self.aggregate_index_rebuild.clone() else {
            return Err(ProductStoreError::NotFound {
                kind: "aggregate_index_command",
                id: request.command_id.clone(),
            });
        };
        // 期望 revision：优先请求显式携带；缺省从当前 durable manifest
        // 解析（重试请求必经投影面，正常都带 revision；解析不到 fail-closed）。
        let expected_revision = request.expected_revision.or_else(|| {
            crate::product::logical_codebase::store::LogicalCodebaseStore::for_lc(
                self.paths.clone(),
                &request.logical_codebase_id,
            )
            .load_manifest(&request.project_id)
            .ok()
            .flatten()
            .map(|manifest| manifest.membership_revision)
        });
        let Some(expected_revision) = expected_revision else {
            return Err(ProductStoreError::NotFound {
                kind: "aggregate_index_manifest",
                id: request.logical_codebase_id.clone(),
            });
        };
        let record = rebuild(&request.project_id, &request.command_id, expected_revision)
            .map_err(map_index_error)?;
        Ok(match record.status {
            AggregateIndexStatus::Active => BootstrapActionOutcome::Completed,
            _ => BootstrapActionOutcome::Replayed,
        })
    }

    /// identity/manifest/rules：已有 registration batch 的 Continue 走
    /// `resume_batch_checked`（显式 expected revision 幂等 resume）；终态
    /// batch 幂等 replay；无 durable 对象时等待用户在登记链提交材料。
    fn dispatch_registration_action(
        &self,
        request: &BootstrapActionRequest,
    ) -> Result<BootstrapActionOutcome, ProductStoreError> {
        let coordinator =
            crate::product::logical_codebase::LogicalCodebaseRegistrationCoordinator::for_lc(
                self.paths.clone(),
                &request.logical_codebase_id,
            );
        match coordinator.get_batch(&request.project_id, &request.expected_object_id) {
            Ok(batch) => {
                use crate::product::logical_codebase::RegistrationBatchStatus;
                match batch.status {
                    RegistrationBatchStatus::Completed | RegistrationBatchStatus::Cancelled => {
                        Ok(BootstrapActionOutcome::Replayed)
                    }
                    RegistrationBatchStatus::Queued
                    | RegistrationBatchStatus::Running
                    | RegistrationBatchStatus::PartialFailed
                        if request.action == BootstrapActionKind::Continue =>
                    {
                        let expected = request.expected_revision.unwrap_or(0);
                        coordinator.resume_batch_checked(
                            &request.project_id,
                            &request.expected_object_id,
                            expected,
                        )?;
                        Ok(BootstrapActionOutcome::Accepted)
                    }
                    _ => Ok(BootstrapActionOutcome::WaitingForHuman),
                }
            }
            Err(ProductStoreError::NotFound { .. }) => Ok(BootstrapActionOutcome::WaitingForHuman),
            Err(error) => Err(error),
        }
    }
}
