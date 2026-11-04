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
        // codegraph 环境前置是「被阻断未开始」而不是失败：两条索引步骤各自投影
        // 为 NotStarted + 无动作 + 无 failure，通知由下方合成**恰好一条**引导
        // （文案取自 failure_copy 同一码表，不另抄中文）。
        let codegraph_block = self.codegraph_block_code(project_id, logical_codebase_id)?;

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
        let mut notices = project_notices(&steps, codegraph_block.as_deref());
        // A（REQ-REG-10）：advance/手动「准备」停在「未开始」步骤时，durable
        // stop 记录补上「为什么停 + 下一步做什么」——被停步骤本身没有
        // failure（那是「未开始」，不是失败），通知不能从步骤 failure 派生，
        // 文案仍取 failure_copy 同一码表（provider 选择门/信任门/配方终态
        // 各自的稳定码都有成品 copy）。
        let advance_stop = load_advance_stop(&self.paths, project_id, logical_codebase_id)?
            .filter(|record| {
                steps.iter().any(|step| {
                    step.step == record.step
                        && step.status == LogicalCodebaseBootstrapStepStatus::NotStarted
                })
            });
        if let Some(record) = advance_stop.as_ref() {
            notices.push(advance_stop_notice(record, &steps));
        }

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
            advance_stop,
        })
    }

    /// codegraph 环境前置的阻断码：片5 的 `EnvironmentPrecondition` 把
    /// `codegraph_missing` / `codegraph_version_mismatch` 落成 operation 的
    /// `error.reason_code`（且全部 step 保持 Pending）。它不是失败事实，
    /// 因此投影为「被阻断未开始」，**不伪造 `failure`**。
    fn codegraph_block_code(
        &self,
        project_id: &str,
        logical_codebase_id: &str,
    ) -> Result<Option<String>, ProductStoreError> {
        let operations =
            crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore::for_lc(
                self.paths.clone(),
                logical_codebase_id,
            )
            .list(project_id)?;
        Ok(operations
            .first()
            .and_then(|latest| latest.error.as_ref())
            .map(|error| error.reason_code.as_str())
            .filter(|code| matches!(*code, "codegraph_missing" | "codegraph_version_mismatch"))
            .map(str::to_string))
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
                    failure: Some(BootstrapFailure::new(
                        "identity_migration_failed".to_string(),
                        sanitize_failure_detail(
                            &journal
                                .last_error
                                .clone()
                                .unwrap_or_else(|| "身份迁移没有留下更详细的诊断输出。".to_string()),
                            None,
                        ),
                        false,
                        "none".to_string(),
                    )),
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
            let detail = sanitize_failure_detail(&detail, Some(&resolution.authority_root));
            BootstrapStepProjection {
                step: LogicalCodebaseBootstrapStep::RulesPolicy,
                status: LogicalCodebaseBootstrapStepStatus::WaitingForHuman,
                object_id: policy.policy_id.clone(),
                checkpoint: Some(policy_checkpoint.clone()),
                failure: Some(BootstrapFailure::new(
                    reason_code.to_string(),
                    detail,
                    true,
                    "none".to_string(),
                )),
                allowed_actions,
            }
        };

        // 源 1（最终 policy 可解析）：identity/digest 由 store 落盘边界校验，
        // 不可解析的 durable 事实按 store 错误 fail-closed 上抛；引用与正文
        // 漂移（并发修订窗口）核验为等待而非就绪。
        let policy_store =
            crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
                self.paths.clone(),
                logical_codebase_id,
            );
        let artifact = match policy_store.get(project_id)? {
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
        // R2:重新发布记录(纯读)。它只用于「已显式 supersede 的旧冻结摘要」
        // 收敛，绝不改写历史 receipt/引用。
        let republication = policy_store.republication(project_id)?;
        let mut superseded_by: Option<&crate::product::logical_codebase::policy::AggregatePolicyRepublication> =
            None;
        if artifact.digest != policy.policy_digest {
            let resolved = republication
                .as_ref()
                .is_some_and(|record| record.covers_artifact(&artifact));
            if !resolved {
                return Ok(waiting(
                    "policy_reference_digest_drift",
                    format!(
                        "authority reference froze digest {} but the current artifact digest is {}",
                        policy.policy_digest, artifact.digest
                    ),
                    policy_reference_drift_actions(),
                ));
            }
            superseded_by = republication.as_ref();
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
                    failure: Some(operation_failure(latest, Some(&resolution.authority_root))),
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
            let resolved = republication.as_ref().is_some_and(|record| {
                record.supersedes_receipt_policy(
                    &latest.operation_id,
                    &artifact,
                    &receipt.policy_digest,
                )
            });
            if !resolved {
                return Ok(waiting(
                    "policy_digest_drift",
                    format!(
                        "receipt froze policy digest {} but the current artifact digest is {}",
                        receipt.policy_digest, artifact.digest
                    ),
                    policy_digest_drift_actions(),
                ));
            }
            superseded_by = republication.as_ref();
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
            let resolved = republication.as_ref().is_some_and(|record| {
                record.supersedes_receipt_rule(
                    &latest.operation_id,
                    &artifact,
                    &receipt.rule_digest,
                    &rule_digest,
                )
            });
            if !resolved {
                return Ok(waiting(
                    "rule_digest_drift",
                    format!(
                        "receipt froze rule digest {} but the current root rule digest is {}",
                        receipt.rule_digest, rule_digest
                    ),
                    policy_digest_drift_actions(),
                ));
            }
            superseded_by = republication.as_ref();
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

        // R2:被显式重新发布 supersede 的 receipt 不再代表当前权威正文——
        // Completed 投影改报重新发布后的 artifact 身份(历史 receipt 原样保留)。
        let checkpoint = match superseded_by {
            Some(record) => BootstrapCheckpoint {
                object_id: artifact.policy_id.clone(),
                input_digest: Some(artifact.digest.clone()),
                output_artifact_ref: Some(
                    crate::product::logical_codebase::policy::POLICY_REPUBLICATION_FILE.to_string(),
                ),
                expected_membership_revision: Some(latest.input.manifest_revision),
                completed_at: Some(record.created_at.clone()),
            },
            None => BootstrapCheckpoint {
                object_id: policy.policy_id.clone(),
                input_digest: Some(receipt.policy_digest.clone()),
                output_artifact_ref: Some(format!(
                    "aggregate-recipe-receipts/{}.json",
                    latest.operation_id
                )),
                expected_membership_revision: Some(latest.input.manifest_revision),
                completed_at: Some(receipt.finalized_at.clone()),
            },
        };
        let object_id = match superseded_by {
            Some(record) => record.policy_id.clone(),
            None => policy.policy_id.clone(),
        };
        Ok(BootstrapStepProjection {
            step: LogicalCodebaseBootstrapStep::RulesPolicy,
            status: LogicalCodebaseBootstrapStepStatus::Completed,
            object_id,
            checkpoint: Some(checkpoint),
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
        // codegraph 环境前置：索引这一步根本没开始，不能投影成失败
        // （REQ-BOOT-08 的「被阻断未开始」形态）。
        let codegraph_blocked = latest
            .error
            .as_ref()
            .map(|error| error.reason_code.as_str())
            .is_some_and(|code| {
                matches!(code, "codegraph_missing" | "codegraph_version_mismatch")
            });
        if codegraph_blocked {
            return Ok(BootstrapStepProjection {
                step: LogicalCodebaseBootstrapStep::MemberIndex,
                status: LogicalCodebaseBootstrapStepStatus::NotStarted,
                object_id: latest.operation_id.clone(),
                checkpoint: None,
                failure: None,
                allowed_actions: Vec::new(),
            });
        }
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
            (status == LogicalCodebaseBootstrapStepStatus::Failed).then(|| operation_failure(latest, None));
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
        // codegraph 缺位/版本不符时索引根本不会开始（片5 的探针在索引前拦下），
        // 这里保持「未开始 + 无动作」，把引导交给合成的那一条通知。
        let codegraph_blocked = {
            let operations =
                crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore::for_lc(
                    self.paths.clone(),
                    logical_codebase_id,
                )
                .list(project_id)?;
            operations
                .first()
                .and_then(|latest| latest.error.as_ref())
                .map(|error| error.reason_code.as_str())
                .is_some_and(|code| {
                    matches!(code, "codegraph_missing" | "codegraph_version_mismatch")
                })
        };
        if codegraph_blocked {
            return Ok(BootstrapStepProjection {
                step: LogicalCodebaseBootstrapStep::AggregateIndexActive,
                status: LogicalCodebaseBootstrapStepStatus::NotStarted,
                object_id: format!("aggregate-index:{logical_codebase_id}"),
                checkpoint: None,
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
                AggregateIndexStatus::Failed => {
                    let warning = record
                        .warning
                        .clone()
                        .unwrap_or_else(|| "first aggregate index build failed".to_string());
                    // 三个业务码各自透出自己的稳定 reason_code（REQ-LAYOUT-01）：
                    // 目录布局不被支持、成员代码覆盖不足各自有精确动作文案；
                    // 其余 CLI 失败保持通用 aggregate_index_failed。沿既有失败
                    // 卡通道，不新建卡型。
                    let reason_code = if warning.contains("aggregate_index_layout_unsupported") {
                        "aggregate_index_layout_unsupported"
                    } else if warning.contains("aggregate_index_member_coverage_failed") {
                        "aggregate_index_member_coverage_failed"
                    } else {
                        "aggregate_index_failed"
                    };
                    (
                        LogicalCodebaseBootstrapStepStatus::Failed,
                        record.aggregate_index_id.clone(),
                        Some(BootstrapFailure::new(
                            reason_code.to_string(),
                            sanitize_failure_detail(&warning, None),
                            true,
                            "aggregate_index_cli".to_string(),
                        )),
                    )
                }
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

/// A（REQ-REG-10）：由 durable stop 记录合成**恰好一条**「为什么停」通知。
/// 步骤行仍是 `not_started`（没有 failure 可折叠），通知卡是唯一能同时
/// 携带成品文案（问题/原因/怎么做/机器码）与原始诊断的通道；allowed_
/// actions 即被停步骤当前可点的按钮，next_step 指向依赖顺序的下一步。
fn advance_stop_notice(
    record: &BootstrapAdvanceStopRecord,
    steps: &[BootstrapStepProjection],
) -> LogicalCodebaseBootstrapNotice {
    let copy = describe_code(&record.reason_code, None);
    let detail = record
        .detail
        .as_deref()
        .map(|detail| sanitize_failure_detail(detail, None));
    let step = steps
        .iter()
        .find(|step| step.step == record.step);
    let next_step = LogicalCodebaseBootstrapStep::V1
        .iter()
        .position(|candidate| *candidate == record.step)
        .and_then(|index| LogicalCodebaseBootstrapStep::V1.get(index + 1).copied());
    LogicalCodebaseBootstrapNotice {
        key: format!(
            "bootstrap:advance-stop:{}:{}",
            record.step.as_str(),
            record.reason_code
        ),
        step: record.step,
        object_id: step
            .map(|step| step.object_id.clone())
            .unwrap_or_else(|| record.step.as_str().to_string()),
        reason_code: record.reason_code.clone(),
        summary: render_detail(&copy, detail.as_deref()),
        external_side_effect: "none".to_string(),
        allowed_actions: step.map(|step| step.allowed_actions.clone()).unwrap_or_default(),
        next_step,
        created_at: record.stopped_at.clone(),
    }
}

fn logical_policy_missing_object_id(project_id: &str) -> String {
    format!("aggregate-policy:{project_id}")
}

/// 通知主文案：中文三段（成品 copy）+ **原始诊断段**。
///
/// 通知卡没有独立的折叠区（不像步骤行由 `BootstrapFailureLine` 折叠 detail），
/// 所以诊断里的可执行信息（例如「调用 `POST /api/projects/…/initializations`
/// 并携带新的 idempotency_key」里的实际 API 路径）**必须**进主文案，否则用户
/// 只看到「怎么做」而拿不到具体入口。
///
/// 已经是成品序列化（`operation_failure` 产出的 detail 自带「问题：」开头）
/// 时原样复用，避免二次包裹。
fn notice_summary(failure: &BootstrapFailure) -> String {
    match failure.copy.as_ref() {
        Some(_) if failure.detail.starts_with("问题：") => failure.detail.clone(),
        Some(copy) => render_detail(copy, Some(&failure.detail)),
        None => failure.detail.clone(),
    }
}

/// 由失败/等待步骤派生通知（key = lc:step:object 稳定去重键），并为
/// codegraph 环境前置合成**恰好一条**阻断引导通知。
///
/// 被阻断的索引步骤不会有 `failure`（那是「未开始」不是「失败」），所以
/// 通知不能从步骤 failure 派生；文案仍来自 `failure_copy` 同一码表。
fn project_notices(
    steps: &[BootstrapStepProjection],
    codegraph_block: Option<&str>,
) -> Vec<LogicalCodebaseBootstrapNotice> {
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
            .map(notice_summary)
            .unwrap_or_else(|| {
                let copy = describe_code("bootstrap_waiting_for_human", None);
                render_detail(&copy, None)
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
    if let Some(code) = codegraph_block {
        let copy = describe_code(code, None);
        let object_id = steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::MemberIndex)
            .map(|step| step.object_id.clone())
            .unwrap_or_else(|| code.to_string());
        notices.push(LogicalCodebaseBootstrapNotice {
            key: format!("bootstrap:codegraph:{code}"),
            step: LogicalCodebaseBootstrapStep::MemberIndex,
            object_id,
            reason_code: code.to_string(),
            summary: render_detail(&copy, None),
            external_side_effect: "none".to_string(),
            allowed_actions: Vec::new(),
            next_step: Some(LogicalCodebaseBootstrapStep::AggregateIndexActive),
            created_at: String::new(),
        });
    }
    notices
}

/// R2/R5：政策摘要漂移时的可执行动作集。
/// `Republish` 是唯一自愈通道（用当前源重新合成并发布新 revision），
/// `Retry`/`Revalidate` 保留为既有语义（仅重新投影）。
fn policy_digest_drift_actions() -> Vec<BootstrapActionKind> {
    // 漂移的唯一自愈通道就是重发；重试/核验对政策漂移是空转，不再暴露。
    vec![BootstrapActionKind::Republish]
}

/// R2/R5：authority 参考摘要漂移时的可执行动作集。
/// 参考漂移通常可由 `Revalidate` 自愈（artifact 已被替换），
/// 若替换未落地则用 `Republish` 重发。
fn policy_reference_drift_actions() -> Vec<BootstrapActionKind> {
    // 引用漂移同样只能靠重发生成新的权威引用。
    vec![BootstrapActionKind::Republish]
}

/// 失败详情统一脱敏：7 条密钥规则 + home/聚合根前缀降级 + 4096 字节/40 行上限。
///
/// 诊断文本来自 provider CLI / Git / codegraph，可能带令牌与用户 home 绝对
/// 路径；唯一实现在 `crate::cross_cutting::diagnostic_redaction`。
fn sanitize_failure_detail(detail: &str, root: Option<&std::path::Path>) -> String {
    let home = crate::product::logical_codebase::provider_trust_adapters::resolve_owner_home_from_env()
        .map(|path| path.to_string_lossy().into_owned());
    let root = root.map(|path| path.to_string_lossy().into_owned());
    crate::cross_cutting::diagnostic_redaction::sanitize_diagnostic(
        detail,
        &crate::cross_cutting::diagnostic_redaction::RedactionContext::new(
            home.as_deref(),
            root.as_deref(),
        ),
    )
}

/// 聚合初始化 operation 的失败投影：**唯一**文案来源出成品 copy，
/// 再用同一序列化器渲染「问题/原因/怎么做/机器码/诊断」文本。
fn operation_failure(
    latest: &crate::product::logical_codebase::aggregate_initialization::AggregateInitializationOperation,
    root: Option<&std::path::Path>,
) -> BootstrapFailure {
    let Some(error) = latest.error.as_ref() else {
        let copy = describe_code("aggregate_initialization_failed", None);
        return BootstrapFailure::with_copy(
            "aggregate_initialization_failed".to_string(),
            render_detail(&copy, None),
            true,
            "aggregate_initialization_provider_turn".to_string(),
            copy,
        );
    };
    let copy = describe(FailureInput::from(error));
    let diagnostic = sanitize_failure_detail(
        &crate::product::logical_codebase::failure_copy::record_diagnostic(error),
        root,
    );
    BootstrapFailure::with_copy(
        error.reason_code.clone(),
        render_detail(&copy, Some(&diagnostic)),
        error.retryable,
        "aggregate_initialization_provider_turn".to_string(),
        copy,
    )
}
