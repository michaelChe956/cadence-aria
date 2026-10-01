//! C4 Task 3：LC 冷启动五步 durable projection 与 action contract。
//!
//! `LogicalCodebaseBootstrapProjection` 不落独立的 `bootstrap.json` 状态机；
//! 它只组合既有 durable facts——LC record、identity migration journal、
//! registration batch、manifest/checkout、聚合 policy artifact、aggregate
//! initialization 的 checkpoint/member projections 与 aggregate index 记录。
//! GET 投影零写入；显式 action（`LogicalCodebaseBootstrapService::apply`）才
//! 推进或修复，并把 command replay 映射回既有 batch/initialization/index 的
//! idempotency 身份。
//!
//! 注意：现有 `AggregateInitializationStepKind::V1` 的五步
//! （machine_skills → aggregate_preflight → pre_check → rule_and_mcp_config →
//! openspec_and_examples）是独立的 durable 协议，与本模块的 C4 冷启动五步
//! （identity → manifest/checkout → rules/policy → member index → aggregate
//! index active）互不冒充、不共享名称。

use std::path::PathBuf;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexError, AggregateIndexStatus, AggregateIndexStore,
};

fn map_index_error(error: AggregateIndexError) -> ProductStoreError {
    ProductStoreError::InvalidRecord {
        kind: "aggregate_index",
        reason: error.to_string(),
    }
}
use crate::product::logical_codebase::provider_admission_preflight::BootstrapActionKind;
use crate::product::logical_codebase::repository_routing::{
    AuthorityPolicyReference, RepositoryAuthorityResolver, RepositoryRoutingRequest,
    RepositoryTargetKind,
};

/// C4 冷启动五步（依赖顺序固定，见计划 Global Constraints）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalCodebaseBootstrapStep {
    Identity,
    ManifestCheckout,
    RulesPolicy,
    MemberIndex,
    AggregateIndexActive,
}

impl LogicalCodebaseBootstrapStep {
    /// Canonical five-step order for the logical codebase cold-start chain.
    pub const V1: [Self; 5] = [
        Self::Identity,
        Self::ManifestCheckout,
        Self::RulesPolicy,
        Self::MemberIndex,
        Self::AggregateIndexActive,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::ManifestCheckout => "manifest_checkout",
            Self::RulesPolicy => "rules_policy",
            Self::MemberIndex => "member_index",
            Self::AggregateIndexActive => "aggregate_index_active",
        }
    }
}

/// 步骤状态投影（来自 durable facts，非独立状态机）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LogicalCodebaseBootstrapStepStatus {
    NotStarted,
    Running,
    Completed,
    Failed,
    WaitingForHuman,
}

/// 既有 durable record 上的 checkpoint 证据（input digest / output artifact /
/// expected membership revision）。只读投影，不新写文件。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapCheckpoint {
    pub object_id: String,
    pub input_digest: Option<String>,
    pub output_artifact_ref: Option<String>,
    pub expected_membership_revision: Option<u64>,
    pub completed_at: Option<String>,
}

/// 失败/等待原因投影：reason_code 稳定，external_side_effect 描述可能的
/// provider/index 外部副作用（"none" 表示纯内部 durable 事实）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapFailure {
    pub reason_code: String,
    pub detail: String,
    pub retryable: bool,
    pub external_side_effect: String,
}

/// 单个冷启动步骤的只读投影。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapStepProjection {
    pub step: LogicalCodebaseBootstrapStep,
    pub status: LogicalCodebaseBootstrapStepStatus,
    pub object_id: String,
    pub checkpoint: Option<BootstrapCheckpoint>,
    pub failure: Option<BootstrapFailure>,
    pub allowed_actions: Vec<BootstrapActionKind>,
}

/// 通知 DTO（C4 Task 9 首定义于此，供 projection/inbox 复用）：事实先落盘、
/// 通知后投影；notice key/object/action 稳定，前端据此去重。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogicalCodebaseBootstrapNotice {
    pub key: String,
    pub step: LogicalCodebaseBootstrapStep,
    pub object_id: String,
    pub reason_code: String,
    pub summary: String,
    pub external_side_effect: String,
    pub allowed_actions: Vec<BootstrapActionKind>,
    pub next_step: Option<LogicalCodebaseBootstrapStep>,
    pub created_at: String,
}

/// LC 冷启动总投影：GET `/logical-codebases/{lc_id}/bootstrap` 的唯一来源。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct LogicalCodebaseBootstrapProjection {
    pub project_id: String,
    pub logical_codebase_id: String,
    pub authority_root: PathBuf,
    pub membership_revision: Option<u64>,
    pub policy: Option<AuthorityPolicyReference>,
    pub steps: Vec<BootstrapStepProjection>,
    pub planning_ready: bool,
    pub notices: Vec<LogicalCodebaseBootstrapNotice>,
}

/// 显式 bootstrap 动作请求：command_id + 目标身份 + expected
/// revision/object identity；同一命令键重放必须返回同一 durable 结果。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapActionRequest {
    pub command_id: String,
    pub project_id: String,
    pub logical_codebase_id: String,
    pub step: LogicalCodebaseBootstrapStep,
    pub action: BootstrapActionKind,
    pub expected_revision: Option<u64>,
    pub expected_object_id: String,
}

/// 动作结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BootstrapActionOutcome {
    Accepted,
    Replayed,
    WaitingForHuman,
    Completed,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct BootstrapActionResult {
    pub command_id: String,
    pub outcome: BootstrapActionOutcome,
    pub projection: LogicalCodebaseBootstrapProjection,
}

/// bootstrap 动作错误：结构化 conflict（稳定码）或底层 store 错误。
#[derive(Debug)]
pub enum BootstrapActionError {
    Conflict { code: String, detail: String },
    Store(ProductStoreError),
}

impl std::fmt::Display for BootstrapActionError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Conflict { code, detail } => write!(formatter, "{code}: {detail}"),
            Self::Store(error) => write!(formatter, "{error}"),
        }
    }
}

impl From<ProductStoreError> for BootstrapActionError {
    fn from(error: ProductStoreError) -> Self {
        Self::Store(error)
    }
}

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
        if let Some(journal) = journal.as_ref() {
            if journal.phase == crate::product::logical_codebase::IdentityMigrationPhase::Failed {
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
            AggregateInitializationStepRecord, AggregateInitializationStepStatus,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::aggregate_index::AggregateIndexRecord;
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateInitializationOperation, AggregateInitializationOperationInput,
        AggregateInitializationStepKind,
    };
    use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
    use crate::product::logical_codebase::store::LogicalCodebaseStore;
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, MemberStatus,
        RepositoryCheckoutRecord, RepositoryType,
    };
    use crate::product::logical_codebase::{
        IdentityMigrationJournal, IdentityMigrationJournalStore, IdentityMigrationPhase,
        LogicalCodebaseCreateInput, LogicalCodebaseManifest, LogicalRepositoryId,
        RepositoryCheckoutId,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};

    fn git(cwd: &std::path::Path, arguments: &[&str]) {
        let output = std::process::Command::new("git")
            .current_dir(cwd)
            .args(arguments)
            .output()
            .expect("git must start");
        assert!(
            output.status.success(),
            "git {arguments:?} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }

    fn init_git_repository_with_commit(path: &std::path::Path) {
        std::fs::create_dir_all(path.join(".claude/rules")).unwrap();
        git(path, &["init", "-b", "main"]);
        git(path, &["config", "user.email", "bootstrap@test.local"]);
        git(path, &["config", "user.name", "Bootstrap Test"]);
        std::fs::write(path.join("README.md"), "# member\n").unwrap();
        // 真实成员规则材料（Task 10 起 rules_policy 步只读检查成员规则）。
        std::fs::write(path.join(".claude/rules/language.md"), "# rule\n").unwrap();
        git(path, &["add", "."]);
        git(path, &["commit", "-m", "init"]);
    }

    fn aria_inventory(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut inventory = std::collections::BTreeMap::new();
        let mut stack = vec![root.join(".aria")];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    inventory.insert(
                        relative.to_string_lossy().into_owned(),
                        std::fs::read(&path).unwrap_or_default(),
                    );
                }
            }
        }
        inventory
    }

    fn create_project_and_lc(paths: &ProductAppPaths, aggregate_root: &std::path::Path) -> String {
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "bootstrap-project".to_string(),
                description: None,
            })
            .unwrap();
        std::fs::create_dir_all(aggregate_root).unwrap();
        LogicalCodebaseStore::new(paths.clone())
            .create(
                "project_0001",
                LogicalCodebaseCreateInput {
                    name: "bootstrap-lc".to_string(),
                    aggregate_root: aggregate_root.to_path_buf(),
                },
            )
            .unwrap()
            .id
    }

    #[test]
    fn bootstrap_projection_reports_missing_active_index_without_manual_seed() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));

        let before = aria_inventory(temp.path());
        let projection = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .expect("empty LC must still project");
        let after = aria_inventory(temp.path());

        // 投影零写入：`.aria` durable inventory 字节不变。
        assert_eq!(before, after);

        // 五个步骤全部可投影（无手工 seed）。
        assert_eq!(projection.steps.len(), 5);
        let statuses: Vec<_> = projection
            .steps
            .iter()
            .map(|step| (step.step, step.status))
            .collect();
        for (step, status) in &statuses {
            assert_ne!(
                *status,
                LogicalCodebaseBootstrapStepStatus::Completed,
                "{} must not be completed on an empty LC",
                step.as_str()
            );
        }
        // aggregate step 为 NotStarted（无任何 index 记录）。
        let aggregate = statuses
            .iter()
            .find(|(step, _)| *step == LogicalCodebaseBootstrapStep::AggregateIndexActive)
            .expect("aggregate step present");
        assert_eq!(aggregate.1, LogicalCodebaseBootstrapStepStatus::NotStarted);
        assert!(!projection.planning_ready);
        assert_eq!(projection.policy, None);
        assert_eq!(projection.membership_revision, None);
        // 空投影的等待通知不产生（NotStarted 非等待事实）。
        assert!(projection.notices.is_empty());
    }

    #[test]
    fn bootstrap_projection_maps_existing_registration_and_initialization_checkpoints() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let aggregate_root = temp.path().join("aggregate-root");
        let lc_id = create_project_and_lc(&paths, &aggregate_root);

        // 真实成员仓 + manifest/member/checkout（identity 与 manifest 事实）。
        let repo = aggregate_root.join("repo");
        init_git_repository_with_commit(&repo);
        let canonical = std::fs::canonicalize(&repo).unwrap();
        let source =
            crate::product::repository_store::resolve_repository_source(&canonical).unwrap();
        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);
        let mut manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
        manifest.member_ids = vec![member_id];
        lc_store.save_manifest("project_0001", &manifest).unwrap();
        lc_store
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    alias: "repo".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        lc_store
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: crate::product::id::repo_hash_for_path(
                        canonical.to_string_lossy().as_ref(),
                    ),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-09-29T00:00:00Z".to_string(),
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        let policy = crate::product::logical_codebase::policy::AggregatePolicyArtifact::bootstrap(
            "project_0001",
            &manifest.logical_codebase_id.to_string(),
            "2026-09-29T00:00:00Z".to_string(),
        );
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            &lc_id,
        )
        .save("project_0001", &policy)
        .unwrap();

        // 既有 aggregate initialization 五步全完成（durable checkpoint 来源）。
        let init_store = AggregateInitializationOperationStore::for_lc(paths.clone(), &lc_id);
        let operation = AggregateInitializationOperation::new(
            "aggregate_initialization_bootstrap_0001".to_string(),
            "project_0001".to_string(),
            AggregateInitializationOperationInput {
                idempotency_key: "bootstrap-0001".to_string(),
                manifest_revision: manifest.membership_revision,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: Some("sha256:profile".to_string()),
                provider_context_root: aggregate_root.clone(),
                provider: "claude_code".to_string(),
            },
            "2026-09-29T00:00:00Z".to_string(),
        );
        init_store.create_idempotent(operation).unwrap();
        init_store
            .mark_running(
                "project_0001",
                "aggregate_initialization_bootstrap_0001",
                "2026-09-29T00:01:00Z".to_string(),
            )
            .unwrap();
        for step in AggregateInitializationStepKind::V1 {
            init_store
                .mark_step_running(
                    "project_0001",
                    "aggregate_initialization_bootstrap_0001",
                    step,
                    format!("aggregate-init:project_0001:op:{}:input", step.as_str()),
                    "2026-09-29T00:02:00Z".to_string(),
                )
                .unwrap();
            init_store
                .checkpoint_step_output(
                    "project_0001",
                    "aggregate_initialization_bootstrap_0001",
                    step,
                    format!("aggregate-initializations/op/{}.json", step.as_str()),
                    "2026-09-29T00:03:00Z".to_string(),
                )
                .unwrap();
            init_store
                .mark_step_completed(
                    "project_0001",
                    "aggregate_initialization_bootstrap_0001",
                    step,
                    "2026-09-29T00:04:00Z".to_string(),
                )
                .unwrap();
        }
        init_store
            .finish_completed(
                "project_0001",
                "aggregate_initialization_bootstrap_0001",
                "2026-09-29T00:05:00Z".to_string(),
            )
            .unwrap();

        // Task 1.6（REQ-BOOT-03）：readiness 三源材料——根规则 + 最终 receipt
        //（真实 auditor 四命令审计后 finalize）。
        std::fs::write(aggregate_root.join("AGENTS.md"), "# aggregate root rules\n").unwrap();
        let rule_digest = crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(
            &std::fs::canonicalize(&aggregate_root).unwrap(),
        )
        .unwrap()
        .expect("root rule digest");
        let receipt = finalize_root_receipt(
            &paths,
            &lc_id,
            "aggregate_initialization_bootstrap_0001",
            &aggregate_root,
            &policy.digest,
            &rule_digest,
        );
        assert_eq!(receipt.policy_digest, policy.digest);

        let projection = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .unwrap();

        let identity = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::Identity)
            .unwrap();
        assert_eq!(
            identity.status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );

        let manifest_step = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::ManifestCheckout)
            .unwrap();
        assert_eq!(
            manifest_step.status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );
        // object id 来自 durable manifest 记录，而非新文件。
        assert_eq!(
            manifest_step.object_id,
            manifest.logical_codebase_id.to_string()
        );

        let rules = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::RulesPolicy)
            .unwrap();
        assert_eq!(rules.status, LogicalCodebaseBootstrapStepStatus::Completed);
        assert!(rules.object_id.starts_with("policy/project_0001/"));
        // Task 1.6：完成 checkpoint 锚定最终 receipt（冻结 digest 与 finalize
        // 时间），receipt 是本步的 durable 输出物。
        let rules_checkpoint = rules.checkpoint.as_ref().unwrap();
        assert_eq!(
            rules_checkpoint.input_digest.as_deref(),
            Some(policy.digest.as_str())
        );
        assert_eq!(
            rules_checkpoint.output_artifact_ref.as_deref(),
            Some("aggregate-recipe-receipts/aggregate_initialization_bootstrap_0001.json")
        );
        assert_eq!(
            rules_checkpoint.completed_at.as_deref(),
            Some(receipt.finalized_at.as_str())
        );

        let member_index = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::MemberIndex)
            .unwrap();
        assert_eq!(
            member_index.status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );
        // object/checkpoint 来自既有 operation（AggregatePreflight 步的 digest/ref）。
        assert_eq!(
            member_index.object_id,
            "aggregate_initialization_bootstrap_0001"
        );
        let checkpoint = member_index.checkpoint.as_ref().unwrap();
        assert_eq!(
            checkpoint.output_artifact_ref.as_deref(),
            Some("aggregate-initializations/op/aggregate_preflight.json")
        );
        assert!(checkpoint.input_digest.is_some());

        // 无 active index：aggregate step 仍未完成，planning_ready=false。
        let aggregate = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::AggregateIndexActive)
            .unwrap();
        assert_eq!(
            aggregate.status,
            LogicalCodebaseBootstrapStepStatus::NotStarted
        );
        assert!(!projection.planning_ready);

        // Task 1.6（REQ-REG-10）：AggregateIndexActive 独立完成后 readiness
        // 闭环——五步全部 Completed，planning_ready=true。
        let index_store = AggregateIndexStore::for_lc(paths.clone(), &lc_id);
        index_store
            .create(
                "project_0001",
                AggregateIndexRecord::building(
                    "aggregate_index_bootstrap_0001".to_string(),
                    "project_0001".to_string(),
                    manifest.membership_revision,
                    Vec::new(),
                    "2026-09-29T00:06:00Z".to_string(),
                ),
            )
            .unwrap();
        index_store
            .mark_status(
                "project_0001",
                "aggregate_index_bootstrap_0001",
                AggregateIndexStatus::Active,
                None,
            )
            .unwrap();
        let ready = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .unwrap();
        assert!(ready.planning_ready);
        for step in &ready.steps {
            assert_eq!(
                step.status,
                LogicalCodebaseBootstrapStepStatus::Completed,
                "step {} must complete for the readiness loop",
                step.step.as_str()
            );
        }
    }

    #[test]
    fn aggregate_initialization_v1_steps_are_not_renamed_to_c4_steps() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));

        let init_store = AggregateInitializationOperationStore::for_lc(paths.clone(), &lc_id);
        let operation = AggregateInitializationOperation::new(
            "aggregate_initialization_names_0001".to_string(),
            "project_0001".to_string(),
            AggregateInitializationOperationInput {
                idempotency_key: "names-0001".to_string(),
                manifest_revision: 1,
                policy_digest: "sha256:policy".to_string(),
                profile_evidence_digest: None,
                provider_context_root: temp.path().join("aggregate-root"),
                provider: "claude_code".to_string(),
            },
            "2026-09-29T00:00:00Z".to_string(),
        );
        init_store.create_idempotent(operation).unwrap();
        let persisted = init_store
            .get("project_0001", "aggregate_initialization_names_0001")
            .unwrap();
        // 既有五步协议保持原名（V1 顺序），未被 C4 步骤冒充。
        let persisted_kinds: Vec<_> = persisted.steps.iter().map(|step| step.step_id).collect();
        assert_eq!(
            persisted_kinds,
            AggregateInitializationStepKind::V1.to_vec()
        );

        let projection = LogicalCodebaseBootstrapProjector::new(paths.clone())
            .project("project_0001", &lc_id)
            .unwrap();
        // bootstrap 投影是独立的 C4 五步，与 V1 协议不同名。
        let bootstrap_steps: Vec<_> = projection.steps.iter().map(|step| step.step).collect();
        assert_eq!(bootstrap_steps, LogicalCodebaseBootstrapStep::V1.to_vec());
        let bootstrap_names: Vec<_> = bootstrap_steps.iter().map(|s| s.as_str()).collect();
        let v1_names: Vec<String> = persisted_kinds
            .iter()
            .map(|s| s.as_str().to_string())
            .collect();
        assert_ne!(bootstrap_names, v1_names);
    }

    // ---- Task 1.6（REQ-BOOT-03/REQ-REG-10）：recipe/readiness 双状态机闭环 ----

    struct ReadinessFixture {
        temp: tempfile::TempDir,
        paths: ProductAppPaths,
        lc_id: String,
        aggregate_root: std::path::PathBuf,
        manifest: LogicalCodebaseManifest,
        policy: crate::product::logical_codebase::policy::AggregatePolicyArtifact,
        operation_id: String,
    }

    impl ReadinessFixture {
        fn project(&self) -> LogicalCodebaseBootstrapProjection {
            LogicalCodebaseBootstrapProjector::new(self.paths.clone())
                .project("project_0001", &self.lc_id)
                .expect("readiness projection must stay read-only and total")
        }

        fn rules_step(projection: &LogicalCodebaseBootstrapProjection) -> &BootstrapStepProjection {
            projection
                .steps
                .iter()
                .find(|step| step.step == LogicalCodebaseBootstrapStep::RulesPolicy)
                .expect("rules_policy step present")
        }
    }

    /// 登记成员 + bootstrap policy + 五步全 Completed 的 recipe operation：
    /// readiness 三源谓词的全部 durable 前置（最终 receipt 除外）。
    fn readiness_fixture(operation_id: &str) -> ReadinessFixture {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let aggregate_root = temp.path().join("aggregate-root");
        let lc_id = create_project_and_lc(&paths, &aggregate_root);

        let repo = aggregate_root.join("repo");
        init_git_repository_with_commit(&repo);
        let canonical = std::fs::canonicalize(&repo).unwrap();
        let source =
            crate::product::repository_store::resolve_repository_source(&canonical).unwrap();
        let member_id = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let lc_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc_id);
        let mut manifest =
            LogicalCodebaseManifest::new("project_0001", aggregate_root.clone(), Vec::new());
        manifest.member_ids = vec![member_id];
        lc_store.save_manifest("project_0001", &manifest).unwrap();
        lc_store
            .save_member(
                "project_0001",
                &CodebaseMemberRecord {
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    alias: "repo".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout_id],
                    status: MemberStatus::Active,
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        lc_store
            .save_checkout(
                "project_0001",
                &RepositoryCheckoutRecord {
                    checkout_id,
                    logical_repository_id: member_id,
                    physical_repository_id: "repository_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: crate::product::id::repo_hash_for_path(
                        canonical.to_string_lossy().as_ref(),
                    ),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-09-29T00:00:00Z".to_string(),
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();

        let policy = crate::product::logical_codebase::policy::AggregatePolicyArtifact::bootstrap(
            "project_0001",
            &manifest.logical_codebase_id.to_string(),
            "2026-10-01T00:00:00Z".to_string(),
        );
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            &lc_id,
        )
        .save("project_0001", &policy)
        .unwrap();

        let init_store = AggregateInitializationOperationStore::for_lc(paths.clone(), &lc_id);
        init_store
            .create_idempotent(AggregateInitializationOperation::new(
                operation_id.to_string(),
                "project_0001".to_string(),
                AggregateInitializationOperationInput {
                    idempotency_key: format!("readiness-{operation_id}"),
                    manifest_revision: manifest.membership_revision,
                    policy_digest: policy.digest.clone(),
                    profile_evidence_digest: Some("sha256:profile".to_string()),
                    provider_context_root: aggregate_root.clone(),
                    provider: "claude_code".to_string(),
                },
                "2026-10-01T00:01:00Z".to_string(),
            ))
            .unwrap();
        init_store
            .mark_running(
                "project_0001",
                operation_id,
                "2026-10-01T00:02:00Z".to_string(),
            )
            .unwrap();
        for step in AggregateInitializationStepKind::V1 {
            init_store
                .mark_step_running(
                    "project_0001",
                    operation_id,
                    step,
                    format!("readiness:{operation_id}:{}", step.as_str()),
                    "2026-10-01T00:03:00Z".to_string(),
                )
                .unwrap();
            init_store
                .checkpoint_step_output(
                    "project_0001",
                    operation_id,
                    step,
                    format!("aggregate-initializations/op/{}.json", step.as_str()),
                    "2026-10-01T00:04:00Z".to_string(),
                )
                .unwrap();
            init_store
                .mark_step_completed(
                    "project_0001",
                    operation_id,
                    step,
                    "2026-10-01T00:05:00Z".to_string(),
                )
                .unwrap();
        }
        init_store
            .finish_completed(
                "project_0001",
                operation_id,
                "2026-10-01T00:06:00Z".to_string(),
            )
            .unwrap();

        ReadinessFixture {
            temp,
            paths,
            lc_id,
            aggregate_root,
            manifest,
            policy,
            operation_id: operation_id.to_string(),
        }
    }

    /// 用真实 auditor 走完四条命令审计并 `finalize` 最终 receipt（Task 1.5
    /// 生产链路的同构 fixture：命令无副作用 → 四条全部 Allowed）。
    fn finalize_root_receipt(
        paths: &ProductAppPaths,
        lc_id: &str,
        operation_id: &str,
        aggregate_root: &std::path::Path,
        policy_digest: &str,
        rule_digest: &str,
    ) -> crate::product::logical_codebase::RootRecipeReceipt {
        let store =
            crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(paths.clone(), lc_id);
        let auditor = crate::product::logical_codebase::RootRecipeFilesystemAuditor::new();
        let canonical_root = std::fs::canonicalize(aggregate_root).unwrap();
        for (index, (step, command_index, command)) in
            crate::product::logical_codebase::root_recipe_command_index()
                .into_iter()
                .enumerate()
        {
            let watch = auditor
                .before_command(operation_id, &canonical_root, step, command_index, command)
                .unwrap();
            let receipt = auditor
                .after_command(watch, format!("2026-10-01T00:10:{index:02}Z"))
                .unwrap();
            assert_eq!(
                receipt.verdict,
                crate::product::logical_codebase::RootRecipeCommandVerdict::Allowed
            );
            store.append_command("project_0001", receipt).unwrap();
        }
        store
            .finalize(
                "project_0001",
                operation_id,
                policy_digest,
                rule_digest,
                "2026-10-01T00:20:00Z".to_string(),
            )
            .unwrap()
    }

    /// 全树字节快照（含成员 `.git` 与 app-data）：readiness GET 零写入的
    /// 强断言面——不只限 `.aria`。
    fn full_tree_inventory(root: &std::path::Path) -> std::collections::BTreeMap<String, Vec<u8>> {
        let mut inventory = std::collections::BTreeMap::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            for entry in std::fs::read_dir(&dir).unwrap().flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let relative = path.strip_prefix(root).unwrap().to_path_buf();
                    inventory.insert(
                        relative.to_string_lossy().into_owned(),
                        std::fs::read(&path).unwrap_or_default(),
                    );
                }
            }
        }
        inventory
    }

    /// Task 1.6（REQ-BOOT-03）：recipe operation Completed 但最终 root
    /// receipt 缺失时，RulesPolicy 保持可操作等待（root_receipt_missing）、
    /// `planning_ready == false`；MemberIndex 步仍独立由五步 checkpoint 判定
    /// Completed——两套五步状态机互不冒充。
    #[test]
    fn recipe_completed_without_root_receipt_keeps_planning_not_ready() {
        let fixture = readiness_fixture("aggregate_initialization_readiness_0001");
        // 根规则在场：隔离「receipt 缺失」这一唯一 readiness 缺口。
        std::fs::write(
            fixture.aggregate_root.join("AGENTS.md"),
            "# aggregate root rules\n",
        )
        .unwrap();

        let projection = fixture.project();
        let rules = ReadinessFixture::rules_step(&projection);
        assert_eq!(
            rules.status,
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman
        );
        let failure = rules.failure.as_ref().expect("failure reason");
        assert_eq!(failure.reason_code, "root_receipt_missing");
        assert!(!rules.allowed_actions.is_empty());
        assert!(rules.allowed_actions.contains(&BootstrapActionKind::Retry));
        assert!(
            rules
                .allowed_actions
                .contains(&BootstrapActionKind::Revalidate)
        );

        let member_index = projection
            .steps
            .iter()
            .find(|step| step.step == LogicalCodebaseBootstrapStep::MemberIndex)
            .expect("member_index step present");
        assert_eq!(
            member_index.status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );

        assert!(!projection.planning_ready);
        let notice = projection
            .notices
            .iter()
            .find(|notice| notice.step == LogicalCodebaseBootstrapStep::RulesPolicy)
            .expect("waiting rules_policy step must surface an actionable notice");
        assert_eq!(notice.reason_code, "root_receipt_missing");
        assert!(!notice.allowed_actions.is_empty());
    }

    /// Task 1.6（REQ-BOOT-03）：receipt 冻结的 canonical root/policy/rule
    /// 身份与当前事实漂移（或根规则缺失）时，RulesPolicy 逐项给出稳定
    /// reason_code 且 `planning_ready == false`；材料恢复一致后谓词可重入
    /// 地回到 Completed（不是单向锁）。
    #[test]
    fn policy_rule_digest_drift_keeps_planning_not_ready() {
        let fixture = readiness_fixture("aggregate_initialization_readiness_0002");
        let entry = fixture.aggregate_root.join("AGENTS.md");
        let root_rule_text = "# aggregate root rules\n";
        std::fs::write(&entry, root_rule_text).unwrap();
        let rule_digest = crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(
            &std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
        )
        .unwrap()
        .expect("root rule digest");
        let receipt = finalize_root_receipt(
            &fixture.paths,
            &fixture.lc_id,
            &fixture.operation_id,
            &fixture.aggregate_root,
            &fixture.policy.digest,
            &rule_digest,
        );
        assert_eq!(receipt.policy_digest, fixture.policy.digest);
        assert_eq!(receipt.rule_digest, rule_digest);

        // 基线：三源一致 → RulesPolicy Completed；planning_ready 仍受
        // AggregateIndexActive 独立门控（无 active index）。
        let ready = fixture.project();
        assert_eq!(
            ReadinessFixture::rules_step(&ready).status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );
        assert!(
            !ready.planning_ready,
            "aggregate index gate must stay independent"
        );

        // 根规则内容漂移 → rule_digest_drift。
        std::fs::write(&entry, "# aggregate root rules (drifted)\n").unwrap();
        let drifted = fixture.project();
        let rules = ReadinessFixture::rules_step(&drifted);
        assert_eq!(
            rules.status,
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman
        );
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "rule_digest_drift"
        );
        assert!(!drifted.planning_ready);

        // 根规则缺失 → root_rule_missing。
        std::fs::remove_file(&entry).unwrap();
        let missing = fixture.project();
        assert_eq!(
            ReadinessFixture::rules_step(&missing)
                .failure
                .as_ref()
                .unwrap()
                .reason_code,
            "root_rule_missing"
        );
        assert!(!missing.planning_ready);

        // 恢复一致 → Completed（可重入谓词）。
        std::fs::write(&entry, root_rule_text).unwrap();
        assert_eq!(
            ReadinessFixture::rules_step(&fixture.project()).status,
            LogicalCodebaseBootstrapStepStatus::Completed
        );

        // policy 升级（digest 前移）→ receipt 冻结摘要漂移。
        let revised = fixture.policy.with_revised_policy(
            "# Aggregate policy (revised)\n",
            "2026-10-01T00:30:00Z".to_string(),
        );
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            fixture.paths.clone(),
            &fixture.lc_id,
        )
        .save("project_0001", &revised)
        .unwrap();
        let policy_drift = fixture.project();
        let rules = ReadinessFixture::rules_step(&policy_drift);
        assert_eq!(
            rules.status,
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman
        );
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "policy_digest_drift"
        );
        assert!(!policy_drift.planning_ready);

        // authority root 漂移（manifest 指向别的 root）→ receipt 冻结的
        // canonical root 失配。
        let other_root = fixture.temp.path().join("other-root");
        std::fs::create_dir_all(&other_root).unwrap();
        let mut moved = fixture.manifest.clone();
        moved.provider_context_root = other_root;
        LogicalCodebaseStore::for_lc(fixture.paths.clone(), &fixture.lc_id)
            .save_manifest("project_0001", &moved)
            .unwrap();
        let authority_drift = fixture.project();
        let rules = ReadinessFixture::rules_step(&authority_drift);
        assert_eq!(
            rules.failure.as_ref().unwrap().reason_code,
            "root_receipt_authority_drift"
        );
        assert!(!authority_drift.planning_ready);
    }

    /// Task 1.6（REQ-REG-10）：readiness GET 投影零写入、可重复读、零副作用
    /// 通道——不启动 provider/index/checkout；等待项给出可操作 allowed
    /// actions；bootstrap 服务侧的 run 探针/重建派发计数保持不变。
    #[test]
    fn readiness_projection_is_read_only() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let fixture = readiness_fixture("aggregate_initialization_readiness_0003");
        std::fs::write(
            fixture.aggregate_root.join("AGENTS.md"),
            "# aggregate root rules\n",
        )
        .unwrap();

        let inventory_before = full_tree_inventory(fixture.temp.path());

        let probe_calls = std::sync::Arc::new(AtomicUsize::new(0));
        let rebuilds = std::sync::Arc::new(AtomicUsize::new(0));
        let _service = LogicalCodebaseBootstrapService::new(fixture.paths.clone())
            .with_member_index_run_probe({
                let probe_calls = probe_calls.clone();
                std::sync::Arc::new(move |_project: &str, _lc: &str, _operation: &str| {
                    probe_calls.fetch_add(1, Ordering::SeqCst);
                    true
                })
            })
            .with_aggregate_index_rebuild({
                let rebuilds = rebuilds.clone();
                std::sync::Arc::new(move |_project: &str, _command: &str, _revision: u64| {
                    rebuilds.fetch_add(1, Ordering::SeqCst);
                    Err(AggregateIndexError::Failed {
                        code: "unexpected_rebuild",
                        message: "readiness GET must not dispatch rebuilds".to_string(),
                    })
                })
            });

        let first = fixture.project();
        let second = fixture.project();
        assert_eq!(
            first, second,
            "repeated GET must re-read the same durable facts without side effects"
        );
        assert!(!first.planning_ready);
        let rules = ReadinessFixture::rules_step(&first);
        assert_eq!(
            rules.status,
            LogicalCodebaseBootstrapStepStatus::WaitingForHuman
        );
        assert!(!rules.allowed_actions.is_empty());
        for action in &rules.allowed_actions {
            assert!(
                ["prepare", "continue", "retry", "revalidate", "repair"].contains(&action.as_str()),
                "allowed actions must stay operable product verbs"
            );
        }

        let inventory_after = full_tree_inventory(fixture.temp.path());
        assert_eq!(
            inventory_before, inventory_after,
            "readiness GET must not write any durable fact (app data or aggregate root)"
        );
        assert_eq!(probe_calls.load(Ordering::SeqCst), 0);
        assert_eq!(rebuilds.load(Ordering::SeqCst), 0);
    }

    fn seed_failed_aggregate_index_record(
        paths: &ProductAppPaths,
        lc_id: &str,
        command_id: &str,
    ) -> String {
        let store = AggregateIndexStore::for_lc(paths.clone(), lc_id);
        let mut record = AggregateIndexRecord::building(
            "aggregate_index_g3_failed_0001".to_string(),
            "project_0001".to_string(),
            2,
            Vec::new(),
            chrono::Utc::now().to_rfc3339(),
        );
        record.command_id = Some(command_id.to_string());
        store.create("project_0001", record).unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_g3_failed_0001",
                AggregateIndexStatus::Failed,
                Some("codegraph_version_mismatch: expected 1.6.0, got 1.6.1".to_string()),
            )
            .unwrap();
        "aggregate_index_g3_failed_0001".to_string()
    }

    fn g3_retry_request(lc_id: &str, command_id: &str) -> BootstrapActionRequest {
        BootstrapActionRequest {
            command_id: command_id.to_string(),
            project_id: "project_0001".to_string(),
            logical_codebase_id: lc_id.to_string(),
            step: LogicalCodebaseBootstrapStep::AggregateIndexActive,
            action: BootstrapActionKind::Retry,
            expected_revision: Some(2),
            expected_object_id: "aggregate_index_g3_failed_0001".to_string(),
        }
    }

    /// G3（终局关闸缺口）：aggregate_index_active 步失败后，bootstrap
    /// actions retry 此前仅按 command_id 查重放记录——找不到即 NotFound
    /// 恒 500，无重建触发分支（现场 cmd-gapfix-a03-retry-1/2）。修复：
    /// 注入重建派发器（复用 `build_with_command_id` 的幂等命令语义）后，
    /// retry 用新 command_id 派发重建；同 command 重放不重复派发。
    #[test]
    fn aggregate_index_retry_dispatches_rebuild_and_replays_same_command() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_aggregate_index_record(&paths, &lc_id, "cmd-gapfix-a03-first");

        let dispatches = std::sync::Arc::new(AtomicUsize::new(0));
        let store_for_rebuild = AggregateIndexStore::for_lc(paths.clone(), &lc_id);
        let dispatches_for_rebuild = dispatches.clone();
        let dispatcher =
            std::sync::Arc::new(move |project_id: &str, command_id: &str, revision: u64| {
                dispatches_for_rebuild.fetch_add(1, Ordering::SeqCst);
                // 复刻 build_with_command_id 成功效果：落一条携带该命令
                // 身份的 Active 记录（membership_revision 对齐请求）。
                let mut record = AggregateIndexRecord::building(
                    format!("aggregate_index_retry_{command_id}"),
                    project_id.to_string(),
                    revision,
                    Vec::new(),
                    chrono::Utc::now().to_rfc3339(),
                );
                record.status = AggregateIndexStatus::Active;
                record.command_id = Some(command_id.to_string());
                store_for_rebuild
                    .create(project_id, record.clone())
                    .map(|_| record)
            });
        let service = LogicalCodebaseBootstrapService::new(paths.clone())
            .with_aggregate_index_rebuild(dispatcher);

        // 修复前现场：NotFound 恒 500（无重建分支）。
        let request = g3_retry_request(&lc_id, "cmd-gapfix-a03-retry-1");
        let outcome = service.dispatch_action(&request).expect("retry rebuilds");
        assert_eq!(outcome, BootstrapActionOutcome::Completed);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
        // 重建派发参数经 durable 事实复核（project/command/revision）。
        let rebuilt = AggregateIndexStore::for_lc(paths.clone(), &lc_id)
            .get(
                "project_0001",
                "aggregate_index_retry_cmd-gapfix-a03-retry-1",
            )
            .expect("rebuilt record")
            .expect("rebuilt record present");
        assert_eq!(rebuilt.status, AggregateIndexStatus::Active);
        assert_eq!(
            rebuilt.command_id.as_deref(),
            Some("cmd-gapfix-a03-retry-1")
        );
        assert_eq!(rebuilt.membership_revision, 2);

        // 同 command 重放：返回同一 Active 事实（Completed），不再派发。
        let replay = service
            .dispatch_action(&request)
            .expect("replay returns durable result");
        assert_eq!(replay, BootstrapActionOutcome::Completed);
        assert_eq!(dispatches.load(Ordering::SeqCst), 1);
    }

    /// G3 语义收口：未注入重建派发器（旧构造）时保持既有 NotFound——
    /// 不静默假成功；由 web 层生产注入收口。
    #[test]
    fn aggregate_index_retry_without_dispatcher_keeps_not_found() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let lc_id = create_project_and_lc(&paths, &temp.path().join("aggregate-root"));
        seed_failed_aggregate_index_record(&paths, &lc_id, "cmd-gapfix-a03-first");

        let service = LogicalCodebaseBootstrapService::new(paths);
        let error = service
            .dispatch_action(&g3_retry_request(&lc_id, "cmd-gapfix-a03-retry-2"))
            .expect_err("no dispatcher keeps fail-closed NotFound");
        assert!(matches!(
            error,
            ProductStoreError::NotFound {
                kind: "aggregate_index_command",
                ..
            }
        ));
    }
}
