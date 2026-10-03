/// 只读组合投影器：从既有 durable records 计算五步状态，不写任何 store。
pub struct LogicalCodebaseBootstrapProjector {
    paths: ProductAppPaths,
}

impl LogicalCodebaseBootstrapProjector {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    pub fn project(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<LogicalCodebaseBootstrapProjection, ProductStoreError> {
        // 身份经唯一 authority resolver 冻结：LC 不存在/冲突一律结构化错误，
        // 不猜路径，不读其他 LC 子树。
        let resolution = RepositoryAuthorityResolver::new(self.paths.clone()).resolve(
            RepositoryRoutingRequest {
                project_id: project_id.to_string(),
                issue_id: None,
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(logical_codebase_id.to_string()),
                logical_repository_id: None,
                checkout_id: None,
            },
        )?;

        let identity_step = self.project_identity_step(project_id, logical_codebase_id)?;
        let manifest_step =
            self.project_manifest_checkout_step(project_id, logical_codebase_id, &resolution)?;
        let rules_step =
            self.project_rules_policy_step(project_id, logical_codebase_id, &resolution)?;
        let member_index_step = self.project_member_index_step(project_id, logical_codebase_id)?;
        let aggregate_step = self.project_aggregate_index_step(project_id, logical_codebase_id)?;

        let steps = vec![
            identity_step,
            manifest_step,
            rules_step,
            member_index_step,
            aggregate_step,
        ];
        let planning_ready = steps
            .iter()
            .all(|step| step.status == LogicalCodebaseBootstrapStepStatus::Completed);
        let notices = project_notices(&steps);

        Ok(LogicalCodebaseBootstrapProjection {
            project_id: project_id.to_string(),
            logical_codebase_id: logical_codebase_id.to_string(),
            authority_root: resolution.authority_root.clone(),
            membership_revision: resolution
                .manifest
                .as_ref()
                .map(|manifest| manifest.membership_revision),
            policy: resolution.policy.clone(),
            steps,
            planning_ready,
            notices,
        })
    }

    /// identity：LC record + identity migration journal。Failed journal 投影
    /// Failed（等待 Task 7 的 repair）；active 成员 ≥1 视为身份完成。
    fn project_identity_step(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<BootstrapStepProjection, ProductStoreError> {
        let journal = crate::product::logical_codebase::IdentityMigrationJournalStore::new(
            self.paths.clone(),
        )
        .load(project_id)?;
        if let Some(journal) = journal.as_ref()
            && journal.phase == crate::product::logical_codebase::IdentityMigrationPhase::Failed {
                return Ok(BootstrapStepProjection {
                    step: LogicalCodebaseBootstrapStep::Identity,
                    status: LogicalCodebaseBootstrapStepStatus::Failed,
                    object_id: journal.migration_id.clone(),
                    checkpoint: None,
                    failure: Some(BootstrapFailure {
                        reason_code: "identity_migration_failed".to_string(),
                        detail: journal
                            .last_error
                            .clone()
                            .unwrap_or_else(|| "identity migration failed".to_string()),
                        retryable: false,
                        external_side_effect: "none".to_string(),
                    }),
                    allowed_actions: vec![BootstrapActionKind::Repair],
                });
            }

        let members = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
            self.paths.clone(),
            logical_codebase_id,
        )
        .list_members(project_id)?;
        let active_members = members
            .iter()
            .filter(|member| {
                member.status == crate::product::logical_codebase::types::MemberStatus::Active
            })
            .count();
        let object_id = journal
            .map(|journal| journal.migration_id)
            .unwrap_or_else(|| logical_codebase_id.to_string());
        if active_members == 0 {
            return Ok(BootstrapStepProjection {
                step: LogicalCodebaseBootstrapStep::Identity,
                status: LogicalCodebaseBootstrapStepStatus::NotStarted,
                object_id,
                checkpoint: None,
                failure: None,
                allowed_actions: vec![BootstrapActionKind::Prepare],
            });
        }
        Ok(BootstrapStepProjection {
            step: LogicalCodebaseBootstrapStep::Identity,
            status: LogicalCodebaseBootstrapStepStatus::Completed,
            object_id,
            checkpoint: Some(BootstrapCheckpoint {
                object_id: logical_codebase_id.to_string(),
                input_digest: None,
                output_artifact_ref: None,
                expected_membership_revision: None,
                completed_at: None,
            }),
            failure: None,
            allowed_actions: Vec::new(),
        })
    }

    /// manifest/checkout：manifest 存在且 checkout ≥1 为完成；无 manifest 时
    /// 等待登记（Prepare）。
    fn project_manifest_checkout_step(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        resolution: &crate::product::logical_codebase::repository_routing::RepositoryAuthorityResolution,
    ) -> Result<BootstrapStepProjection, ProductStoreError> {
        let store = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
            self.paths.clone(),
            logical_codebase_id,
        );
        let checkouts = store.list_checkouts(project_id)?;
        match resolution.manifest.as_ref() {
            Some(manifest) => {
                let object_id = manifest.logical_codebase_id.to_string();
                Ok(BootstrapStepProjection {
                    step: LogicalCodebaseBootstrapStep::ManifestCheckout,
                    status: if checkouts.is_empty() {
                        LogicalCodebaseBootstrapStepStatus::WaitingForHuman
                    } else {
                        LogicalCodebaseBootstrapStepStatus::Completed
                    },
                    object_id: object_id.clone(),
                    checkpoint: Some(BootstrapCheckpoint {
                        object_id,
                        input_digest: None,
                        output_artifact_ref: None,
                        expected_membership_revision: Some(manifest.membership_revision),
                        completed_at: None,
                    }),
                    failure: None,
                    allowed_actions: if checkouts.is_empty() {
                        vec![BootstrapActionKind::Continue]
                    } else {
                        Vec::new()
                    },
                })
            }
            None => Ok(BootstrapStepProjection {
                step: LogicalCodebaseBootstrapStep::ManifestCheckout,
                status: LogicalCodebaseBootstrapStepStatus::NotStarted,
                object_id: logical_codebase_id.to_string(),
                checkpoint: None,
                failure: None,
                allowed_actions: vec![BootstrapActionKind::Prepare],
            }),
        }
    }

    /// rules/policy（Task 1.6，REQ-BOOT-03/REQ-REG-10）：只读 canonical root
    /// 的 root authority 三源谓词——最终 policy artifact 可解析且与 resolver
    /// 冻结引用一致、最新 recipe operation 的最终 receipt 在场、receipt 冻结
    /// 的 canonical root/policy digest/rule digest 与当前事实一致。成员
    /// checkout 的 `.claude/rules/language.md` 遍历已移除：成员规则检查归属
    /// provider admission 预检（Task 8），readiness 不再读成员仓。
    ///
    /// recipe operation Completed 但任一材料缺失/漂移时投影 WaitingForHuman
    /// （稳定 reason_code + 可操作 Prepare/Retry/Revalidate），绝不静默视为
    /// 就绪；MemberIndex/AggregateIndexActive 步保持独立 checkpoint 判定，
    /// 两套五步状态机互不冒充。本检查零写入、零 provider 启动。
    fn project_rules_policy_step(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
        resolution: &crate::product::logical_codebase::repository_routing::RepositoryAuthorityResolution,
    ) -> Result<BootstrapStepProjection, ProductStoreError> {
        use crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperationStatus;

        let policy = match resolution.policy.as_ref() {
            None => {
                return Ok(BootstrapStepProjection {
                    step: LogicalCodebaseBootstrapStep::RulesPolicy,
                    status: LogicalCodebaseBootstrapStepStatus::NotStarted,
                    object_id: logical_policy_missing_object_id(project_id),
                    checkpoint: None,
                    failure: None,
                    allowed_actions: vec![BootstrapActionKind::Prepare],
                });
            }
            Some(policy) => policy,
        };
        let policy_checkpoint = BootstrapCheckpoint {
            object_id: policy.policy_id.clone(),
            input_digest: Some(policy.policy_digest.clone()),
            output_artifact_ref: None,
            expected_membership_revision: None,
            completed_at: None,
        };
        let waiting = |reason_code: &str,
                       detail: String,
                       allowed_actions: Vec<BootstrapActionKind>|
         -> BootstrapStepProjection {
            BootstrapStepProjection {
                step: LogicalCodebaseBootstrapStep::RulesPolicy,
                status: LogicalCodebaseBootstrapStepStatus::WaitingForHuman,
                object_id: policy.policy_id.clone(),
                checkpoint: Some(policy_checkpoint.clone()),
                failure: Some(BootstrapFailure {
                    reason_code: reason_code.to_string(),
                    detail,
                    retryable: true,
                    external_side_effect: "none".to_string(),
                }),
                allowed_actions,
            }
        };

        // 源 1（最终 policy 可解析）：identity/digest 由 store 落盘边界校验，
        // 不可解析的 durable 事实按 store 错误 fail-closed 上抛；引用与正文
        // 漂移（并发修订窗口）核验为等待而非就绪。
        let artifact =
            match crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
                self.paths.clone(),
                logical_codebase_id,
            )
            .get(project_id)?
            {
                Some(artifact) => artifact,
                None => {
                    return Ok(waiting(
                        "aggregate_policy_artifact_missing",
                        format!(
                            "authority reference froze policy {} but the artifact is gone",
                            policy.policy_id
                        ),
                        vec![
                            BootstrapActionKind::Prepare,
                            BootstrapActionKind::Revalidate,
                        ],
                    ));
                }
            };
        if artifact.digest != policy.policy_digest {
            return Ok(waiting(
                "policy_reference_digest_drift",
                format!(
                    "authority reference froze digest {} but the current artifact digest is {}",
                    policy.policy_digest, artifact.digest
                ),
                vec![BootstrapActionKind::Revalidate],
            ));
        }

        // 源 2（recipe operation 生命周期）：与 MemberIndex 步同一 durable
        // 源、独立判定。
        let operations =
            crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore::for_lc(
                self.paths.clone(),
                logical_codebase_id,
            )
            .list(project_id)?;
        let Some(latest) = operations.first() else {
            return Ok(waiting(
                "root_recipe_operation_missing",
                format!(
                    "policy {} is ready but no root recipe operation has run under \
                     logical codebase {logical_codebase_id}",
                    policy.policy_id
                ),
                vec![BootstrapActionKind::Prepare],
            ));
        };
        match latest.status {
            AggregateInitializationOperationStatus::Completed => {}
            AggregateInitializationOperationStatus::Created
            | AggregateInitializationOperationStatus::Running => {
                return Ok(BootstrapStepProjection {
                    step: LogicalCodebaseBootstrapStep::RulesPolicy,
                    status: LogicalCodebaseBootstrapStepStatus::Running,
                    object_id: latest.operation_id.clone(),
                    checkpoint: Some(policy_checkpoint.clone()),
                    failure: None,
                    allowed_actions: Vec::new(),
                });
            }
            AggregateInitializationOperationStatus::Failed => {
                return Ok(BootstrapStepProjection {
                    step: LogicalCodebaseBootstrapStep::RulesPolicy,
                    status: LogicalCodebaseBootstrapStepStatus::Failed,
                    object_id: latest.operation_id.clone(),
                    checkpoint: Some(policy_checkpoint.clone()),
                    failure: Some(BootstrapFailure {
                        reason_code: latest
                            .error
                            .as_ref()
                            .map(|error| error.reason_code.clone())
                            .unwrap_or_else(|| "root_recipe_failed".to_string()),
                        detail: latest
                            .error
                            .as_ref()
                            .map(|error| {
                                format!(
                                    "stage {}: {}",
                                    error.stage,
                                    error
                                        .stderr_summary
                                        .as_deref()
                                        .unwrap_or("no stderr summary")
                                )
                            })
                            .unwrap_or_else(|| {
                                "the latest root recipe operation failed".to_string()
                            }),
                        retryable: latest
                            .error
                            .as_ref()
                            .map(|error| error.retryable)
                            .unwrap_or(true),
                        external_side_effect: "aggregate_initialization_provider_turn".to_string(),
                    }),
                    allowed_actions: vec![BootstrapActionKind::Retry],
                });
            }
            AggregateInitializationOperationStatus::Cancelled => {
                return Ok(waiting(
                    "root_recipe_cancelled",
                    format!(
                        "the latest root recipe operation {} was cancelled before \
                         readiness materials were frozen",
                        latest.operation_id
                    ),
                    vec![BootstrapActionKind::Continue],
                ));
            }
        }

        // 源 3（最终 receipt 三重身份一致）：receipt 缺失即 BOOT-03 的
        // 「recipe 成功但 readiness 材料未齐」。
        let receipt_store = crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
            self.paths.clone(),
            logical_codebase_id,
        );
        let Some(receipt) = receipt_store.get(project_id, &latest.operation_id)? else {
            return Ok(waiting(
                "root_receipt_missing",
                format!(
                    "root recipe operation {} completed without a finalized root receipt",
                    latest.operation_id
                ),
                vec![BootstrapActionKind::Retry, BootstrapActionKind::Revalidate],
            ));
        };
        if receipt.canonical_root != resolution.authority_root {
            return Ok(waiting(
                "root_receipt_authority_drift",
                format!(
                    "receipt froze canonical root {} but the current authority root is {}",
                    receipt.canonical_root.display(),
                    resolution.authority_root.display()
                ),
                vec![BootstrapActionKind::Revalidate],
            ));
        }
        if receipt.policy_digest != artifact.digest {
            return Ok(waiting(
                "policy_digest_drift",
                format!(
                    "receipt froze policy digest {} but the current artifact digest is {}",
                    receipt.policy_digest, artifact.digest
                ),
                vec![BootstrapActionKind::Retry, BootstrapActionKind::Revalidate],
            ));
        }
        let rule_digest = crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(
            &resolution.authority_root,
        )?;
        let Some(rule_digest) = rule_digest else {
            return Ok(waiting(
                "root_rule_missing",
                format!(
                    "{} is absent under the canonical root {}",
                    crate::product::logical_codebase::root_recipe_receipt::ROOT_RULE_ENTRY_FILE,
                    resolution.authority_root.display()
                ),
                vec![BootstrapActionKind::Retry, BootstrapActionKind::Revalidate],
            ));
        };
        if receipt.rule_digest != rule_digest {
            return Ok(waiting(
                "rule_digest_drift",
                format!(
                    "receipt froze rule digest {} but the current root rule digest is {}",
                    receipt.rule_digest, rule_digest
                ),
                vec![BootstrapActionKind::Retry, BootstrapActionKind::Revalidate],
            ));
        }

        // Task 2（REQ-BOOT-06）：唯一桩等待 guard,位于原 Completed 返回
        // 之前、上方全部既有检查（缺件/漂移/生命周期分流）之后——三源
        // 一致但当前权威正文仍是已知自举桩时,Completed 让位于显式迁移
        // 等待。等待零写入、零 provider 启动：不写迁移标记、不改旧
        // receipt/checkpoint 身份;detail 指引唯一的存量迁移入口（初始
        // 化 API + 新 idempotency_key）,Completed 无可见启动按钮,
        // RulesPolicy 通用 Retry 不是根配方入口。
        if artifact.is_bootstrap_placeholder() {
            return Ok(waiting(
                "aggregate_policy_bootstrap_placeholder",
                format!(
                    "当前聚合政策仍是已知自举桩正文（digest {}）,不能冒充根配方权威\
                     正文,readiness 等待显式迁移：请调用 POST \
                     /api/projects/{project_id}/logical-codebases/{logical_codebase_id}/initializations \
                     并携带新的 idempotency_key 重新跑真实根配方；Completed 状态没有\
                     可见的启动按钮,RulesPolicy 的通用 Retry 不是根配方入口,不会自动\
                     重跑。旧 operation/receipt 事实原样保留,本检查零写入、零 provider \
                     启动。",
                    artifact.digest
                ),
                vec![BootstrapActionKind::Retry, BootstrapActionKind::Revalidate],
            ));
        }

        Ok(BootstrapStepProjection {
            step: LogicalCodebaseBootstrapStep::RulesPolicy,
            status: LogicalCodebaseBootstrapStepStatus::Completed,
            object_id: policy.policy_id.clone(),
            checkpoint: Some(BootstrapCheckpoint {
                object_id: policy.policy_id.clone(),
                input_digest: Some(receipt.policy_digest.clone()),
                output_artifact_ref: Some(format!(
                    "aggregate-recipe-receipts/{}.json",
                    latest.operation_id
                )),
                expected_membership_revision: Some(latest.input.manifest_revision),
                completed_at: Some(receipt.finalized_at.clone()),
            }),
            failure: None,
            allowed_actions: Vec::new(),
        })
    }

    /// member index：既有 aggregate initialization 的 deterministic
    /// preflight（AggregatePreflight 步）checkpoint 与 member projections。
    fn project_member_index_step(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<BootstrapStepProjection, ProductStoreError> {
        let operations =
            crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore::for_lc(
                self.paths.clone(),
                logical_codebase_id,
            )
            .list(project_id)?;
        let Some(latest) = operations.first() else {
            return Ok(BootstrapStepProjection {
                step: LogicalCodebaseBootstrapStep::MemberIndex,
                status: LogicalCodebaseBootstrapStepStatus::NotStarted,
                object_id: format!("member-index:{logical_codebase_id}"),
                checkpoint: None,
                failure: None,
                allowed_actions: vec![BootstrapActionKind::Prepare],
            });
        };
        use crate::product::logical_codebase::aggregate_initialization::{
            AggregateInitializationOperationStatus, AggregateInitializationStepKind,
        };
        let status = match latest.status {
            AggregateInitializationOperationStatus::Completed => {
                LogicalCodebaseBootstrapStepStatus::Completed
            }
            AggregateInitializationOperationStatus::Failed => {
                LogicalCodebaseBootstrapStepStatus::Failed
            }
            AggregateInitializationOperationStatus::Cancelled => {
                LogicalCodebaseBootstrapStepStatus::WaitingForHuman
            }
            AggregateInitializationOperationStatus::Created
            | AggregateInitializationOperationStatus::Running => {
                LogicalCodebaseBootstrapStepStatus::Running
            }
        };
        let preflight_checkpoint = latest
            .steps
            .iter()
            .find(|step| step.step_id == AggregateInitializationStepKind::AggregatePreflight)
            .map(|step| BootstrapCheckpoint {
                object_id: latest.operation_id.clone(),
                input_digest: step.input_digest.clone(),
                output_artifact_ref: step.output_artifact_ref.clone(),
                expected_membership_revision: None,
                completed_at: step.completed_at.clone(),
            });
        let failure =
            (status == LogicalCodebaseBootstrapStepStatus::Failed).then(|| BootstrapFailure {
                reason_code: latest
                    .error
                    .as_ref()
                    .map(|error| error.reason_code.clone())
                    .unwrap_or_else(|| "aggregate_initialization_failed".to_string()),
                detail: latest
                    .error
                    .as_ref()
                    .map(|error| {
                        format!(
                            "stage {}: {}",
                            error.stage,
                            error
                                .stderr_summary
                                .as_deref()
                                .unwrap_or("no stderr summary")
                        )
                    })
                    .unwrap_or_else(|| "aggregate initialization failed".to_string()),
                retryable: latest
                    .error
                    .as_ref()
                    .map(|error| error.retryable)
                    .unwrap_or(true),
                external_side_effect: "aggregate_initialization_provider_turn".to_string(),
            });
        let allowed_actions = match status {
            LogicalCodebaseBootstrapStepStatus::Failed => vec![BootstrapActionKind::Retry],
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman => {
                vec![BootstrapActionKind::Continue]
            }
            LogicalCodebaseBootstrapStepStatus::NotStarted => vec![BootstrapActionKind::Prepare],
            _ => Vec::new(),
        };
        Ok(BootstrapStepProjection {
            step: LogicalCodebaseBootstrapStep::MemberIndex,
            status,
            object_id: latest.operation_id.clone(),
            checkpoint: preflight_checkpoint,
            failure,
            allowed_actions,
        })
    }

    /// aggregate index active：Active → Completed；Building → Running；最新
    /// Failed → Failed；Stale/Degraded → WaitingForHuman（保留 LKG 事实）；
    /// 无记录 → NotStarted。
    fn project_aggregate_index_step(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<BootstrapStepProjection, ProductStoreError> {
        let store = AggregateIndexStore::for_lc(self.paths.clone(), logical_codebase_id);
        if let Some(active) = store.active(project_id).map_err(map_index_error)? {
            return Ok(BootstrapStepProjection {
                step: LogicalCodebaseBootstrapStep::AggregateIndexActive,
                status: LogicalCodebaseBootstrapStepStatus::Completed,
                object_id: active.aggregate_index_id.clone(),
                checkpoint: Some(BootstrapCheckpoint {
                    object_id: active.aggregate_index_id.clone(),
                    input_digest: Some(active.config_digest.clone()),
                    output_artifact_ref: None,
                    expected_membership_revision: Some(active.membership_revision),
                    completed_at: Some(active.updated_at.clone()),
                }),
                failure: None,
                allowed_actions: Vec::new(),
            });
        }
        let records = store.records(project_id).map_err(map_index_error)?;
        let latest_non_superseded = records
            .iter()
            .find(|record| record.status != AggregateIndexStatus::Superseded);
        let (status, object_id, failure) = match latest_non_superseded {
            Some(record) => match record.status {
                AggregateIndexStatus::Building => (
                    LogicalCodebaseBootstrapStepStatus::Running,
                    record.aggregate_index_id.clone(),
                    None,
                ),
                AggregateIndexStatus::Failed => (
                    LogicalCodebaseBootstrapStepStatus::Failed,
                    record.aggregate_index_id.clone(),
                    Some(BootstrapFailure {
                        reason_code: "aggregate_index_failed".to_string(),
                        detail: record
                            .warning
                            .clone()
                            .unwrap_or_else(|| "first aggregate index build failed".to_string()),
                        retryable: true,
                        external_side_effect: "aggregate_index_cli".to_string(),
                    }),
                ),
                AggregateIndexStatus::Stale | AggregateIndexStatus::Degraded => (
                    LogicalCodebaseBootstrapStepStatus::WaitingForHuman,
                    record.aggregate_index_id.clone(),
                    None,
                ),
                AggregateIndexStatus::Active | AggregateIndexStatus::Superseded => (
                    LogicalCodebaseBootstrapStepStatus::NotStarted,
                    record.aggregate_index_id.clone(),
                    None,
                ),
            },
            None => (
                LogicalCodebaseBootstrapStepStatus::NotStarted,
                format!("aggregate-index:{logical_codebase_id}"),
                None,
            ),
        };
        let allowed_actions = match status {
            LogicalCodebaseBootstrapStepStatus::Failed => vec![BootstrapActionKind::Retry],
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman => {
                vec![BootstrapActionKind::Retry, BootstrapActionKind::Revalidate]
            }
            LogicalCodebaseBootstrapStepStatus::NotStarted => vec![BootstrapActionKind::Prepare],
            _ => Vec::new(),
        };
        Ok(BootstrapStepProjection {
            step: LogicalCodebaseBootstrapStep::AggregateIndexActive,
            status,
            object_id,
            checkpoint: None,
            failure,
            allowed_actions,
        })
    }
}

fn logical_policy_missing_object_id(project_id: &str) -> String {
    format!("aggregate-policy:{project_id}")
}

/// 由失败/等待步骤派生通知（key = lc:step:object 稳定去重键）。
fn project_notices(steps: &[BootstrapStepProjection]) -> Vec<LogicalCodebaseBootstrapNotice> {
    let mut notices = Vec::new();
    for step in steps {
        if step.status == LogicalCodebaseBootstrapStepStatus::Completed
            || step.status == LogicalCodebaseBootstrapStepStatus::Running
            || step.status == LogicalCodebaseBootstrapStepStatus::NotStarted
        {
            continue;
        }
        let reason_code = step
            .failure
            .as_ref()
            .map(|failure| failure.reason_code.clone())
            .unwrap_or_else(|| "bootstrap_waiting_for_human".to_string());
        let summary = step
            .failure
            .as_ref()
            .map(|failure| failure.detail.clone())
            .unwrap_or_else(|| {
                format!(
                    "step {} is waiting for an explicit action",
                    step.step.as_str()
                )
            });
        let external_side_effect = step
            .failure
            .as_ref()
            .map(|failure| failure.external_side_effect.clone())
            .unwrap_or_else(|| "none".to_string());
        let next_step = LogicalCodebaseBootstrapStep::V1
            .iter()
            .position(|candidate| *candidate == step.step)
            .and_then(|index| LogicalCodebaseBootstrapStep::V1.get(index + 1).copied());
        notices.push(LogicalCodebaseBootstrapNotice {
            key: format!(
                "bootstrap:{}:{}:{}",
                step.step.as_str(),
                step.object_id,
                reason_code
            ),
            step: step.step,
            object_id: step.object_id.clone(),
            reason_code,
            summary,
            external_side_effect,
            allowed_actions: step.allowed_actions.clone(),
            next_step,
            created_at: String::new(),
        });
    }
    notices
}
