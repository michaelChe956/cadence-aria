// 本片段通过 `include!` 进入 `aggregate_initialization` 模块；自带 `use`
// 以免改动父模块文件（片5 独占本片段）。
use crate::product::cadence_skills::CadenceSkillsError;
use crate::product::logical_codebase::environment_probe::{
    AggregateEnvironmentProbe, CodeGraphEnvironmentProbe,
};
use crate::product::logical_codebase::root_recipe_receipt::RootRecipeSnapshotEntry;

/// Adapts the shared Cadence skills manager to the aggregate coordinator's
/// durable machine-skills step. The manager owns source cloning/updating and
/// all three managed link layers; this adapter only records deterministic
/// evidence for the operation.
struct CadenceAggregateSkillsPreparation {
    manager: Arc<CadenceSkillsManager>,
}

/// 把 `CadenceSkillsError` 的结构化 `stage` / `layer` 映射成稳定子码
/// `cause`，避免失败面只剩一句不可行动的自然语言。`reason_code` 仍是
/// `aggregate_machine_skills_failed`（与旧 `SkillsPreparation` 共用同一臂）。
///
/// task 7.8：三类确定性失败都必须保留结构化来源 ——
/// `skills_unavailable_{stage}` / `skills_update_failed_{stage}` /
/// `skills_sync_failed_{layer}`，**不得**压成通用串。
fn skills_preparation_error(
    error: CadenceSkillsError,
    home: Option<&str>,
) -> AggregateInitializationError {
    let cause = match &error {
        CadenceSkillsError::Unavailable { stage, .. } => format!("skills_unavailable_{stage}"),
        CadenceSkillsError::UpdateFailed { stage, .. } => format!("skills_update_failed_{stage}"),
        CadenceSkillsError::SyncFailed { layer, .. } => format!("skills_sync_failed_{layer}"),
    };
    AggregateInitializationError::SkillsPreparationDiagnosed {
        // 确定性失败的 detail 可能夹带 CLI 输出/绝对路径：统一走全仓唯一
        // 「清洗 → 密钥脱敏 → 路径降级 → 限长」入口（REQ-BOOT-13）。
        reason: crate::cross_cutting::diagnostic_redaction::sanitize_diagnostic(
            &error.to_string(),
            &crate::cross_cutting::diagnostic_redaction::RedactionContext::new(home, None),
        ),
        retryable: true,
        cause,
    }
}

/// skills 失败面里的 HOME 前缀（由 `<home>/.agents/Cadence-skills` 反推），
/// 交给 `sanitize_diagnostic` 把绝对路径降级成 `~`。
fn skills_home_prefix(paths: &CadenceSkillsPaths) -> Option<String> {
    paths
        .repository_root()
        .parent()
        .and_then(std::path::Path::parent)
        .map(|home| home.to_string_lossy().into_owned())
}

/// skills 证据摘要（源目录 / 三层链接）失败：不是 provider 问题，仍按
/// machine_skills 失败上报，`cause` 统一为 `skills_digest_failed`。
fn skills_digest_error(stage: &str, error: impl std::fmt::Display) -> AggregateInitializationError {
    AggregateInitializationError::SkillsPreparationDiagnosed {
        reason: format!("skill {stage} digest failed: {error}"),
        retryable: true,
        cause: "skills_digest_failed".to_string(),
    }
}

#[async_trait::async_trait]
impl AggregateSkillsPreparation for CadenceAggregateSkillsPreparation {
    async fn prepare_skills(
        &self,
        _project_id: &str,
        _operation_id: &str,
        cancellation: CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
        let home = skills_home_prefix(self.manager.paths());
        let result = self
            .manager
            .prepare(cancellation)
            .await
            .map_err(|error| skills_preparation_error(error, home.as_deref()))?;
        let source_digest = digest_tree(self.manager.paths().source_root())
            .map_err(|error| skills_digest_error("source", error))?;
        let link_digest = digest_link_layers(self.manager.paths())
            .map_err(|error| skills_digest_error("link", error))?;
        Ok(MachineSkillsPreparation {
            source_digest,
            link_digest,
            skills_root: result.skills_root,
            warnings: result.warnings,
        })
    }
}

struct GatewayFactoryProviderTurnDriver {
    factory: Option<Arc<LogicalCodebaseGatewayFactory>>,
    /// C4 Task 10：admission 预检所需的持久化路径（provider turn 前检查
    /// 实际成员 `.claude/rules/language.md`、policy digest 与 capability；
    /// 缺失时保持无预检旧行为——仅 factory 本身缺失的降级组装）。
    paths: Option<ProductAppPaths>,
}

impl GatewayFactoryProviderTurnDriver {
    fn new(
        factory: Option<Arc<LogicalCodebaseGatewayFactory>>,
        paths: Option<ProductAppPaths>,
    ) -> Self {
        Self { factory, paths }
    }
}

#[async_trait::async_trait]
impl AggregateProviderTurnDriver for GatewayFactoryProviderTurnDriver {
    async fn run_turn(
        &self,
        request: AggregateProviderTurnRequest<'_>,
    ) -> Result<
        crate::product::logical_codebase::aggregate_initialization_coordinator::AggregateProviderTurnOutcome,
        AggregateInitializationError,
    > {
        let AggregateProviderTurnRequest {
            project_id,
            operation_id,
            step,
            preflight,
            lc_id,
            provider,
            bootstrap,
            cancellation,
        } = request;
        let Some(factory) = self.factory.as_ref() else {
            return Err(AggregateInitializationError::ProviderTurn {
                step,
                reason: "logical codebase gateway factory is not configured".to_string(),
                retryable: false,
            });
        };
        // Task 4（REQ-BOOT-05）：末命令已 Allowed 审计时走显式恢复分支——
        // 不开新审计窗口、不组装 gateway、不启动 provider，只续发布与
        // receipt（launch 计数不增、不追加第二条 04、recorded_at/
        // finalized_at 不变）。此检查先于 gateway 组装，恢复不依赖
        // receipt 目录的可读性。
        if let (Some(lc_id), Some(paths)) = (lc_id, self.paths.as_ref()) {
            let canonical_root = std::path::PathBuf::from(&preflight.aggregate_root);
            if step_owns_final_recipe_command(step)
                && can_resume_audited_final_command(
                    paths,
                    lc_id,
                    project_id,
                    operation_id,
                    &canonical_root,
                )
                .map_err(|error| root_recipe_publication_failure(step, error))?
            {
                verify_resume_against_final_audit(
                    paths,
                    lc_id,
                    project_id,
                    operation_id,
                    &canonical_root,
                )
                .map_err(|error| root_recipe_publication_failure(step, error))?;
                finalize_published_root_recipe(
                    paths,
                    lc_id,
                    project_id,
                    operation_id,
                    &canonical_root,
                )
                .await
                .map_err(|error| root_recipe_publication_failure(step, error))?;
                return Ok(
                    crate::product::logical_codebase::aggregate_initialization_coordinator::AggregateProviderTurnOutcome {
                        summary: "root policy publication resumed after the audited final command"
                            .to_string(),
                        report: crate::product::logical_codebase::aggregate_initialization_coordinator::RecipeCommandReport::Absent,
                    },
                );
            }
        }
        let gateway = factory
            .build_for_lc_material_prep(project_id, lc_id)
            .map_err(|error| AggregateInitializationError::ProviderTurn {
                step,
                reason: format!("logical codebase gateway factory build failed: {error}"),
                retryable: true,
            })?;
        // 片4（REQ-BOOT-07）：driver 按 request.provider 构造——经 C-2 集中
        // 映射 `ProviderRef::from_provider_name`（四家显式、Fake fail-closed、
        // 无 `_ =>` 静默回退），provider 不进 driver 字段（state 级单例）。
        let provider_ref = crate::product::logical_codebase::ProviderRef::from_provider_name(
            provider,
            "cap_managed_snapshot",
        )
        .map_err(|error| AggregateInitializationError::ProviderTurn {
            step,
            reason: format!("aggregate recipe provider {provider:?} is not launchable: {error}"),
            retryable: false,
        })?;
        // C4 Task 10 / Task 1.4：经 with_provider_and_admission 组装——
        // provider turn 前先做实际成员规则/policy/capability 的 admission
        // 预检（Task 8），缺失/漂移时 fail-closed，provider 保持零启动。
        // Task 1.8（D2/BOOT-03）：paths + lc_id 齐备的生产面再叠加 root
        // recipe 命令审计窗口（per-command before/after 快照 receipt）。
        // Task 4（REQ-BOOT-05）：末命令 turn 成功后，command04 Allowed
        // 审计先 durable 落盘、全部审计窗口关闭，再发布权威政策正文并
        // 冻结最终 receipt；发布失败强制传播为可重试 ProviderTurn 失败
        // （绝不 warn-only 补发）。任一侧缺失（legacy 别名解析失败的
        // 降级组装）保持无审计旧路径。
        if let (Some(lc_id), Some(paths)) = (lc_id, self.paths.as_ref()) {
            let watches = begin_root_recipe_command_watches(operation_id, step, preflight)
                .map_err(|error| root_recipe_audit_failure(step, error))?;
            let driver = GatewayBackedAggregateProviderTurnDriver::with_provider_and_admission(
                Arc::new(gateway),
                provider_ref.clone(),
                paths.clone(),
            );
            let outcome = driver
                .run_turn(AggregateProviderTurnRequest {
                    project_id,
                    operation_id,
                    step,
                    preflight,
                    lc_id: Some(lc_id),
                    bootstrap,
                    cancellation,
                    provider,
                })
                .await?;
            // 命令未执行完成（turn 失败/取消）不落 receipt：无执行即无证据，
            // 重试窗口重新审计；同 index 同内容重放幂等。
            // turn summary 一并传入：零变更失败的诊断要带 provider 的真实输出
            // （零变更本身不说明原因，见 root_recipe_audit_windows.inc.rs）。
            complete_root_recipe_command_receipts(
                paths,
                lc_id,
                project_id,
                watches,
                step,
                provider,
                &outcome,
            )?;
            if step_owns_final_recipe_command(step) {
                // Task 4：末命令唯一收口（先审计后发布——发布产物
                // policy/** 不进任何命令审计窗口）。中断 seam 仅测试编译。
                #[cfg(test)]
                if root_policy_test_faults::trip(
                    operation_id,
                    root_policy_test_faults::Phase::AfterCommandAudit,
                ) {
                    return Err(AggregateInitializationError::ProviderTurn {
                        step,
                        reason: "test-injected interruption after the final command audit"
                            .to_string(),
                        retryable: true,
                    });
                }
                finalize_published_root_recipe(
                    paths,
                    lc_id,
                    project_id,
                    operation_id,
                    &std::path::PathBuf::from(&preflight.aggregate_root),
                )
                .await
                .map_err(|error| root_recipe_publication_failure(step, error))?;
            }
            return Ok(outcome);
        }
        let driver = GatewayBackedAggregateProviderTurnDriver::with_provider(
            Arc::new(gateway),
            provider_ref,
        );
        driver
            .run_turn(AggregateProviderTurnRequest {
                project_id,
                operation_id,
                step,
                preflight,
                lc_id,
                bootstrap,
                cancellation,
                provider,
            })
            .await
    }
}

/// Task 4（aggregate-policy-root-publication）：末命令收口链的确定性
/// 中断注入（仅 `cfg(test)` 编译，生产无任何开关）。按具体 operation
/// 作用域隔离（不用全局 failpoint），单发语义：命中即消费，保证显式
/// Continue 后发布可恢复。
#[cfg(test)]
mod root_policy_test_faults {
    use std::sync::Mutex;

    /// 可注入中断的收口阶段。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum Phase {
        /// 末命令 Allowed 审计已 durable、发布开始前（恢复窗口：命令审计后）。
        AfterCommandAudit,
        /// 最终 receipt 签发阶段的写失败注入（发布保留、末步 Failed 的
        /// 负例；不占位文件路径，避免毒化 receipt 目录的其他读者）。
        ReceiptWrite,
        /// 发布已完成（output/locator/current）、最终 receipt 签发前
        ///（恢复窗口：正文与 artifact 后）。
        AfterPublication,
        /// 最终 receipt 已签发、turn 返回成功前（恢复窗口：receipt 后）。
        AfterFinalReceipt,
    }

    static ARMED: Mutex<Vec<(String, Phase)>> = Mutex::new(Vec::new());

    /// 以具体 operation 为界注入：并行测试的各 operation 互不污染。
    pub(super) fn arm(operation_id: &str, phase: Phase) {
        ARMED
            .lock()
            .expect("root policy fault mutex")
            .push((operation_id.to_string(), phase));
    }

    /// 命中（同 operation 且同阶段）即消费该条目（单发）；未命中返回
    /// false，不影响其他 operation 的注入。
    pub(super) fn trip(operation_id: &str, phase: Phase) -> bool {
        let mut armed = ARMED.lock().expect("root policy fault mutex");
        let Some(position) = armed.iter().position(|(armed_operation, armed_phase)| {
            armed_operation == operation_id && *armed_phase == phase
        }) else {
            return false;
        };
        armed.remove(position);
        true
    }
}

/// 固定命令索引中的末命令（全局最大 command index——固定顺序下即
/// OpenspecAndExamples 的命令 4）。
fn final_recipe_command() -> (AggregateInitializationStepKind, usize, &'static str) {
    crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index()
        .into_iter()
        .max_by_key(|(_, command_index, _)| *command_index)
        .expect("the frozen root recipe command index is non-empty")
}

/// Task 4（REQ-BOOT-05）：末命令唯一收口——在 blocking 任务中读取同 LC
/// manifest、调用 I1 `publish_recipe_policy` 发布权威政策正文、复验
/// 输出/根 locator 字节/current artifact/来源摘要/root 身份与 rule
/// digest，再以 I3 `finalize` 签发最终 receipt（policy digest 取
/// `published.digest`、rule digest 经当前 `root_rule_digest` 复验、
/// `finalized_at` 复用候选 `artifact.created_at`；同参数重放幂等）。
/// 候选时间只在 publication 尚不存在时首次冻结，已存在时读取复用。
/// 任何阶段失败都上抛，由调用方映射为可重试 ProviderTurn 失败；blocking
/// 任务 join 失败同样传播（不以 warn 吞掉）。
async fn finalize_published_root_recipe(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
    operation_id: &str,
    canonical_root: &std::path::Path,
) -> Result<(), crate::product::json_store::ProductStoreError> {
    let paths = paths.clone();
    let lc_id = lc_id.to_string();
    let project_id = project_id.to_string();
    let operation_id = operation_id.to_string();
    let canonical_root = canonical_root.to_path_buf();
    tokio::task::spawn_blocking(move || {
        finalize_published_root_policy_blocking(
            &paths,
            &lc_id,
            &project_id,
            &operation_id,
            &canonical_root,
        )
    })
    .await
    .map_err(|error| {
        crate::product::json_store::ProductStoreError::Io(format!(
            "root policy finalize blocking task failed: {error}"
        ))
    })?
}

/// 跳过式聚合初始化的政策发布收口实现（owner 2026-10-29）：聚合根已完成
/// Cadence-skills 初始化时四条命令确实没必要跑，只做聚合政策（读政策源 →
/// 合成 → 发布 artifact）并签发「跳过式」最终 receipt。
///
/// 与末命令收口共用 [`publish_and_verify_root_policy`]：政策正文/来源/根
/// 身份/rule digest 的复验一条不少；差别只在 receipt 语义（无命令审计事实，
/// 代之以跳过判定依据的复验）。
struct SkippedAggregatePolicyPublisher {
    paths: ProductAppPaths,
}

#[async_trait::async_trait]
impl crate::product::logical_codebase::aggregate_initialization_coordinator::AggregatePolicyPublication
    for SkippedAggregatePolicyPublisher
{
    async fn publish_skipped(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
        canonical_root: &std::path::Path,
        decision: &crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitialization,
    ) -> Result<(), AggregateInitializationError> {
        let paths = self.paths.clone();
        let project_id = project_id.to_string();
        let lc_id = lc_id.to_string();
        let operation_id = operation_id.to_string();
        let canonical_root = canonical_root.to_path_buf();
        let decision = decision.clone();
        tokio::task::spawn_blocking(move || {
            publish_skipped_root_policy_blocking(
                &paths,
                &lc_id,
                &project_id,
                &operation_id,
                &canonical_root,
                decision,
            )
        })
        .await
        .map_err(|error| AggregateInitializationError::PolicyPublication {
            reason: format!("skipped root policy publication task failed: {error}"),
            retryable: true,
        })?
        .map_err(|error| AggregateInitializationError::PolicyPublication {
            reason: format!("skipped root policy publication failed: {error}"),
            retryable: true,
        })
    }

    /// 本 operation 是否已有命令审计事实（跳过判定窗口的反证来源）。
    fn has_audited_commands(
        &self,
        project_id: &str,
        operation_id: &str,
        lc_id: &str,
    ) -> Result<bool, AggregateInitializationError> {
        crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
            self.paths.clone(),
            lc_id.to_string(),
        )
        .list_commands(project_id, operation_id)
        .map(|commands| !commands.is_empty())
        .map_err(|error| AggregateInitializationError::PolicyPublication {
            reason: format!("root recipe command receipts could not be read: {error}"),
            retryable: true,
        })
    }
}

/// [`SkippedAggregatePolicyPublisher::publish_skipped`] 的同步主体（blocking
/// 任务内执行）。
fn publish_skipped_root_policy_blocking(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
    operation_id: &str,
    canonical_root: &std::path::Path,
    decision: crate::product::logical_codebase::aggregate_initialization::AggregateSkillsInitialization,
) -> Result<(), crate::product::json_store::ProductStoreError> {
    let (artifact, rule_digest) = publish_and_verify_root_policy(
        paths,
        lc_id,
        project_id,
        operation_id,
        canonical_root,
    )?;
    // 跳过式 receipt：签发时再复验两个判定文件摘要（store 内 fail-closed），
    // 且本 operation 绝不允许已有命令审计事实。
    crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        paths.clone(),
        lc_id.to_string(),
    )
    .finalize_skipped(
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeSkipFinalization {
            project_id,
            operation_id,
            canonical_root,
            decision,
            policy_digest: &artifact.digest,
            rule_digest: &rule_digest,
            finalized_at: artifact.created_at.clone(),
        },
    )?;
    Ok(())
}

/// [`finalize_published_root_recipe`] 的同步主体（blocking 任务内执行）。
fn finalize_published_root_policy_blocking(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
    operation_id: &str,
    canonical_root: &std::path::Path,
) -> Result<(), crate::product::json_store::ProductStoreError> {
    let (artifact, rule_digest) = publish_and_verify_root_policy(
        paths,
        lc_id,
        project_id,
        operation_id,
        canonical_root,
    )?;
    #[cfg(test)]
    if root_policy_test_faults::trip(operation_id, root_policy_test_faults::Phase::ReceiptWrite) {
        return Err(crate::product::json_store::ProductStoreError::Io(
            "injected receipt write failure".to_string(),
        ));
    }
    // I3：最终 receipt 只在复验全部通过后签发。
    crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        paths.clone(),
        lc_id.to_string(),
    )
    .finalize(
        project_id,
        operation_id,
        &artifact.digest,
        &rule_digest,
        artifact.created_at.clone(),
    )?;
    #[cfg(test)]
    if root_policy_test_faults::trip(
        operation_id,
        root_policy_test_faults::Phase::AfterFinalReceipt,
    ) {
        return Err(crate::product::json_store::ProductStoreError::Io(
            "injected interruption after the final receipt".to_string(),
        ));
    }
    Ok(())
}

/// 发布权威聚合政策正文并独立复验冻结事实（完整 recipe 收口与跳过式
/// 收口共用同一条链）。返回 `(artifact, root rule digest)`；任何不一致
/// 都上抛，绝不让未复验的内容冒充权威政策。
fn publish_and_verify_root_policy(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
    operation_id: &str,
    canonical_root: &std::path::Path,
) -> Result<
    (
        crate::product::logical_codebase::policy::AggregatePolicyArtifact,
        String,
    ),
    crate::product::json_store::ProductStoreError,
> {
    use crate::product::json_store::ProductStoreError;
    let invalid = |reason: String| ProductStoreError::InvalidRecord {
        kind: "aggregate_policy_publication",
        reason,
    };
    // 同 LC manifest：发布身份（project/UUID/canonical root）的唯一权威。
    let manifest = crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
        paths.clone(),
        lc_id.to_string(),
    )
    .load_manifest(project_id)?
    .ok_or_else(|| invalid(format!("logical codebase {lc_id} manifest is missing")))?;
    let policy_store =
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            lc_id.to_string(),
        );
    // 候选时间：publication 已存在时读取复用 artifact.created_at；尚不
    // 存在时只首次冻结当前时间（I1 对同 operation 重入复用同候选）。
    let created_at = policy_store
        .get_recipe_policy_publication(project_id, operation_id)?
        .map(|output| output.artifact.created_at)
        .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
    let artifact =
        policy_store.publish_recipe_policy(&manifest, operation_id, canonical_root, created_at)?;

    // 独立复验冻结事实：不可变输出、根 locator 原字节、current artifact、
    // 来源摘要与 root/rule 双摘要——发布链内部已核验，收口再读一次。
    let output = policy_store
        .get_recipe_policy_publication(project_id, operation_id)?
        .ok_or_else(|| invalid(format!("publication output of {operation_id} disappeared")))?;
    if output.artifact != artifact {
        return Err(invalid(format!(
            "publication output of {operation_id} does not match the published artifact"
        )));
    }
    if output.canonical_root != canonical_root {
        return Err(invalid(format!(
            "publication output froze root {} but the recipe root is {}",
            output.canonical_root.display(),
            canonical_root.display()
        )));
    }
    let locator = canonical_root.join(&artifact.policy_id);
    let published = std::fs::read(&locator).map_err(|error| {
        ProductStoreError::Io(format!("read back {}: {error}", locator.display()))
    })?;
    if published != artifact.policy_text.as_bytes() {
        return Err(invalid(format!(
            "published policy bytes at {} do not match the artifact text",
            locator.display()
        )));
    }
    let current = policy_store
        .get(project_id)?
        .ok_or_else(|| invalid("current aggregate policy artifact is missing".to_string()))?;
    if current != artifact {
        return Err(invalid(format!(
            "current aggregate policy artifact is {} (revision {}) but the publication froze {} (revision {})",
            current.policy_id, current.revision, artifact.policy_id, artifact.revision
        )));
    }
    for source in &output.sources {
        let bytes = std::fs::read(canonical_root.join(&source.relative_path)).map_err(|error| {
            ProductStoreError::Io(format!(
                "read policy source {}: {error}",
                source.relative_path
            ))
        })?;
        if format!("sha256:{:x}", Sha256::digest(&bytes)) != source.digest {
            return Err(invalid(format!(
                "policy source {} drifted since the frozen publication",
                source.relative_path
            )));
        }
    }
    let rule_digest =
        crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(canonical_root)?
            .ok_or_else(|| {
                invalid(format!(
                    "{} is absent under the canonical root {}",
                    crate::product::logical_codebase::root_recipe_receipt::ROOT_RULE_ENTRY_FILE,
                    canonical_root.display()
                ))
            })?;
    if rule_digest != output.rule_digest {
        return Err(invalid(format!(
            "root rule digest drifted since the frozen publication of {operation_id}"
        )));
    }
    #[cfg(test)]
    if root_policy_test_faults::trip(
        operation_id,
        root_policy_test_faults::Phase::AfterPublication,
    ) {
        return Err(ProductStoreError::Io(
            "injected interruption after policy publication".to_string(),
        ));
    }
    Ok((artifact, rule_digest))
}

/// Task 4：判断末命令是否已有同 operation 的 Allowed 审计（显式恢复的
/// 前置事实）。`Ok(false)` 只表示当前 operation 尚无末命令审计（走原
/// provider 路径）；既有审计 Rejected、命令身份或 canonical root 漂移
/// 必须报错——不能返回 false 再执行 provider 把已拒绝的命令重跑一遍。
fn can_resume_audited_final_command(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
    operation_id: &str,
    canonical_root: &std::path::Path,
) -> Result<bool, crate::product::json_store::ProductStoreError> {
    let invalid = |reason: String| crate::product::json_store::ProductStoreError::InvalidRecord {
        kind: "root_recipe_command_receipt",
        reason,
    };
    let (final_step, final_index, final_command) = final_recipe_command();
    let Some(audit) = crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        paths.clone(),
        lc_id.to_string(),
    )
    .list_commands(project_id, operation_id)?
    .into_iter()
    .find(|receipt| receipt.command_index == final_index) else {
        return Ok(false);
    };
    if audit.step != final_step || audit.command != final_command {
        return Err(invalid(format!(
            "final command audit of {operation_id} drifted from the frozen root recipe index"
        )));
    }
    if audit.canonical_root != canonical_root {
        return Err(invalid(format!(
            "final command audit froze canonical root {} but the resume root is {}",
            audit.canonical_root.display(),
            canonical_root.display()
        )));
    }
    if audit.verdict
        != crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandVerdict::Allowed
    {
        return Err(invalid(format!(
            "final command of {operation_id} was audited {:?}; a rejected command \
             cannot be resumed as audited-allowed",
            audit.verdict
        )));
    }
    Ok(true)
}

/// 恢复分支「旧摘要无法复验」专用 record kind：重试无用 ⇒ `retryable: false`。
const LEGACY_DIGEST_UNVERIFIABLE_KIND: &str = "aggregate_policy_publication_legacy_digest";

/// 旧 receipt（改造前口径：文件条目带内容摘要）与新快照（元数据口径：
/// `content_digest` 恒 `None`）之间摘要**不可比**：其余元数据一致的条目既
/// 不能判「已变更」（把存量 operation 的恢复一律误报），也不能判「没变」。
fn legacy_content_digest_unverifiable(old: &RootRecipeSnapshotEntry, fresh: &RootRecipeSnapshotEntry) -> bool {
    if old.content_digest.is_none() || fresh.content_digest.is_some() {
        return false;
    }
    let mut without_digest = old.clone();
    without_digest.content_digest = None;
    without_digest == *fresh
}

/// Task 4：恢复分支的全树复验——重新拍当前 canonical root 快照，与末条
/// Allowed 审计的 after snapshot 比较；只有由 I1 不可变输出证明的精确
/// policy locator 及其新增目录可扣除，其余任何新增/删除/变更都拒绝。
/// 来源文件同样在全树比较范围内，sources 与旧末条 snapshot 匹配由此
/// 一并保证。
fn verify_resume_against_final_audit(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
    operation_id: &str,
    canonical_root: &std::path::Path,
) -> Result<(), crate::product::json_store::ProductStoreError> {
    let invalid = |reason: String| crate::product::json_store::ProductStoreError::InvalidRecord {
        kind: "aggregate_policy_publication",
        reason,
    };
    let (final_step, final_index, final_command) = final_recipe_command();
    let audit = crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        paths.clone(),
        lc_id.to_string(),
    )
    .list_commands(project_id, operation_id)?
    .into_iter()
    .find(|receipt| receipt.command_index == final_index)
    .ok_or_else(|| invalid(format!("final command audit of {operation_id} disappeared")))?;
    // 重新拍全树快照（与 auditor 同一实现/预算；watch 只用于快照，不落盘）。
    let auditor =
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeFilesystemAuditor::new();
    let fresh = auditor
        .before_command(
            operation_id,
            canonical_root,
            final_step,
            final_index,
            final_command,
        )?
        .before;
    // 可扣除集合：I1 不可变输出证明的精确 locator 及其每一级父目录。
    let mut deductible: Vec<String> = Vec::new();
    if let Some(output) =
        crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
            paths.clone(),
            lc_id.to_string(),
        )
        .get_recipe_policy_publication(project_id, operation_id)?
    {
        let mut prefix = String::new();
        for segment in output.artifact.policy_id.split('/') {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(segment);
            deductible.push(prefix.clone());
        }
    }
    let old: std::collections::BTreeMap<&str, &RootRecipeSnapshotEntry> = audit
        .after_snapshot
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    let current: std::collections::BTreeMap<&str, &RootRecipeSnapshotEntry> = fresh
        .entries
        .iter()
        .map(|entry| (entry.path.as_str(), entry))
        .collect();
    for (path, entry) in &current {
        if deductible.iter().any(|allowed| allowed == path) {
            continue; // 发布新增的 locator 及其父目录
        }
        match old.get(path) {
            Some(old_entry) if *old_entry == *entry => {}
            Some(old_entry) if legacy_content_digest_unverifiable(old_entry, entry) => {
                return Err(crate::product::json_store::ProductStoreError::InvalidRecord {
                    kind: LEGACY_DIGEST_UNVERIFIABLE_KIND,
                    reason: format!(
                        "path {path} of the audited final command of {operation_id} carries a legacy \
                         content digest that the metadata-only audit cannot re-verify; re-run the \
                         audited final command to record a fresh audit snapshot, then resume"
                    ),
                });
            }
            Some(_) => {
                return Err(invalid(format!(
                    "path {path} changed since the audited final command of {operation_id}"
                )));
            }
            None => {
                return Err(invalid(format!(
                    "path {path} appeared since the audited final command of {operation_id}"
                )));
            }
        }
    }
    for path in old.keys() {
        if !current.contains_key(path) {
            return Err(invalid(format!(
                "path {path} disappeared since the audited final command of {operation_id}"
            )));
        }
    }
    Ok(())
}

/// 审计窗口失败 → 可重试 ProviderTurn 失败（operation 落 durable Failed，
/// 保留重试面）。
fn root_recipe_audit_failure(
    step: AggregateInitializationStepKind,
    error: crate::product::json_store::ProductStoreError,
) -> AggregateInitializationError {
    AggregateInitializationError::ProviderTurn {
        step,
        reason: format!("root recipe filesystem audit failed: {error}"),
        retryable: true,
    }
}

/// Task 4（REQ-BOOT-05）：末命令发布/收口失败 → 可重试 ProviderTurn
/// 失败（operation 落 durable Failed，保留显式 Continue/Retry 恢复面；
/// 发布失败不签发最终 receipt、不投影为根规则/政策已完成）。
/// 例外：恢复分支的「旧摘要无法复验」重试无用（必须先重新审计）⇒ 不可重试。
fn root_recipe_publication_failure(
    step: AggregateInitializationStepKind,
    error: crate::product::json_store::ProductStoreError,
) -> AggregateInitializationError {
    let retryable = !matches!(
        &error,
        crate::product::json_store::ProductStoreError::InvalidRecord { kind, .. }
            if *kind == LEGACY_DIGEST_UNVERIFIABLE_KIND
    );
    AggregateInitializationError::ProviderTurn {
        step,
        reason: format!("root recipe policy publication failed: {error}"),
        retryable,
    }
}

/// Task 1.3（REQ-REG-14）：生产 trust 前置门装配。真实 home 只作为
/// adapter 默认参数进入（CodexTrustAdapter/KimiTrustAdapter 的
/// `production()`）；fake runtime 下指向 workspace 内安全根，绝不触碰
/// 真实用户 home（与 `aggregate_skills_home` 同一防护语义）。
fn production_provider_trust_precondition(
    state: &WebAppState,
    paths: &ProductAppPaths,
) -> Result<Arc<dyn crate::product::logical_codebase::ProviderTrustPrecondition>, ApiError> {
    let fake_runtime = !state
        .runtime
        .lock()
        .expect("web runtime lock")
        .enforces_real_provider_availability();
    let home = if fake_runtime {
        state.workspace_root.clone()
    } else {
        std::env::var_os("HOME")
            .or_else(|| std::env::var_os("USERPROFILE"))
            .map(std::path::PathBuf::from)
            .filter(|home| home.is_absolute())
            .ok_or_else(|| {
                ApiError::runtime(
                    "provider_trust_home_unavailable",
                    "HOME or USERPROFILE must be an absolute path",
                    serde_json::json!({}),
                )
            })?
    };
    let registry = crate::product::logical_codebase::HomeBackedProviderTrustRegistry::new(
        paths.clone(),
        vec![
            std::sync::Arc::new(
                crate::product::logical_codebase::CodexTrustAdapter::for_home(&home),
            ),
            std::sync::Arc::new(
                crate::product::logical_codebase::KimiTrustAdapter::for_home(&home),
            ),
        ],
    );
    Ok(Arc::new(registry))
}

impl AggregateInitializationDependencies {
    pub fn production(state: &WebAppState) -> Result<Self, ApiError> {
        let paths = product_app_paths(state);
        let home = aggregate_skills_home(state)?;
        let environment = std::env::var("PATH")
            .ok()
            .map(|path| std::collections::BTreeMap::from([("PATH".to_string(), path)]))
            .unwrap_or_default();
        let manager = Arc::new(CadenceSkillsManager::with_dependencies(
            home.clone(),
            state.command_runner.clone(),
            environment,
        ));
        let skills: Arc<dyn AggregateSkillsPreparation> =
            Arc::new(CadenceAggregateSkillsPreparation { manager });
        // REQ-BOOT-09 / D-16：探针与索引操作共用**同一份 CLI 配置**（同一个
        // `state.command_runner` + 同一个 executable 名 `"codegraph"`，与下方
        // `AggregateIndexOperation` 的构造逐字一致）。⚠️ 不是同一个实例：
        // `AggregateIndexOperation` 按值持有 `CodeGraphCli`，真共享要改
        // `aggregate_index/**`（本片禁碰，`Degraded` 语义要求不动）。
        let environment_probe: Arc<dyn AggregateEnvironmentProbe> =
            Arc::new(CodeGraphEnvironmentProbe::new(
                state.command_runner.clone(),
                "codegraph".to_string(),
            ));
        let preflight: Arc<dyn AggregatePreflightService> =
            Arc::new(DeterministicAggregatePreflightService::new(paths.clone()));
        let provider: Arc<dyn AggregateProviderTurnDriver> =
            Arc::new(GatewayFactoryProviderTurnDriver::new(
                state.gateway_factory().cloned(),
                Some(paths.clone()),
            ));
        let operations = AggregateInitializationOperationStore::new(paths.clone());
        let clock: Arc<dyn Fn() -> String + Send + Sync> =
            Arc::new(|| chrono::Utc::now().to_rfc3339());
        let trust = production_provider_trust_precondition(state, &paths)?;
        let coordinator = Arc::new(
            AggregateInitializationCoordinator::new(
                paths.clone(),
                operations,
                skills,
                preflight,
                provider,
                clock,
            )
            // Task 1.4：生产 coordinator 携带 trust 硬前置门，使
            // `execute_with_trust` 在生产依赖图可用（handler 接线见 Task 1.8）。
            .with_trust(trust.clone())
            // owner 2026-10-29：聚合根已做过 Cadence-skills 初始化时跳过
            // 四条命令，只发布聚合政策并签发跳过式 receipt。
            .with_policy_publication(Arc::new(SkippedAggregatePolicyPublisher {
                paths: paths.clone(),
            }))
            // REQ-BOOT-09：codegraph 冷启动前置探测（失败 step: None）。
            .with_environment_probe(environment_probe)
            // 四条命令的链接层校验与 skills manager 的 HOME 同源。
            .with_skills_home(home),
        );
        let index = Arc::new(AggregateIndexOperation::new(
            paths.clone(),
            CodeGraphCli::new(state.command_runner.clone(), "codegraph".to_string()),
            CodeGraphExcludeGenerator,
        ));
        Ok(
            Self::with_index(coordinator, InitializationRunRegistry::default(), index)
                .with_trust(trust),
        )
    }
}

fn aggregate_skills_home(state: &WebAppState) -> Result<std::path::PathBuf, ApiError> {
    let fake_runtime = !state
        .runtime
        .lock()
        .expect("web runtime lock")
        .enforces_real_provider_availability();
    if fake_runtime {
        return Ok(state.workspace_root.clone());
    }
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(std::path::PathBuf::from)
        .filter(|home| home.is_absolute())
        .ok_or_else(|| {
            ApiError::runtime(
                "cadence_skills_home_unavailable",
                "HOME or USERPROFILE must be an absolute path",
                serde_json::json!({}),
            )
        })
}

fn digest_link_layers(paths: &CadenceSkillsPaths) -> Result<String, std::io::Error> {
    let mut hasher = Sha256::new();
    for root in [
        paths.shared_skills_root(),
        paths.codex_skills_root(),
        paths.claude_skills_root(),
    ] {
        hasher.update(root.to_string_lossy().as_bytes());
        hash_tree(root, root, &mut hasher)?;
    }
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn digest_tree(root: &std::path::Path) -> Result<String, std::io::Error> {
    let mut hasher = Sha256::new();
    hash_tree(root, root, &mut hasher)?;
    Ok(format!("sha256:{:x}", hasher.finalize()))
}

fn hash_tree(
    root: &std::path::Path,
    path: &std::path::Path,
    hasher: &mut Sha256,
) -> Result<(), std::io::Error> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let relative = path.strip_prefix(root).unwrap_or(path);
    hasher.update(relative.to_string_lossy().as_bytes());
    if metadata.file_type().is_symlink() {
        hasher.update(b"symlink");
        hasher.update(std::fs::read_link(path)?.to_string_lossy().as_bytes());
    } else if metadata.is_file() {
        hasher.update(b"file");
        hasher.update(std::fs::read(path)?);
    } else if metadata.is_dir() {
        hasher.update(b"dir");
        let mut entries = std::fs::read_dir(path)?.collect::<Result<Vec<_>, _>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries {
            hash_tree(root, &entry.path(), hasher)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod skills_preparation_diagnosis_tests {
    use super::*;

    /// `CadenceSkillsError::Unavailable{stage}` → `skills_unavailable_<stage>`：
    /// 失败面能给出「拉取/更新哪一步坏了」，而不是一句 provider 未登录。
    #[test]
    fn unavailable_stage_becomes_structured_cause() {
        let error = skills_preparation_error(
            CadenceSkillsError::unavailable(
                "clone",
                "cannot reach the Cadence-skills remote",
                "check the network and rerun the aggregate initialization",
            ),
            None,
        );
        assert!(
            matches!(
                &error,
                AggregateInitializationError::SkillsPreparationDiagnosed { cause, retryable, .. }
                    if cause == "skills_unavailable_clone" && *retryable
            ),
            "expected skills_unavailable_clone, got {error:?}"
        );
    }

    /// `CadenceSkillsError::SyncFailed{layer}` → `skills_sync_failed_<layer>`。
    #[test]
    fn sync_failure_layer_becomes_structured_cause() {
        let error = skills_preparation_error(
            CadenceSkillsError::sync_failed(
                "claude",
                "pre-check",
                "/home/tester/.claude/skills/pre-check",
                "unrelated symlink",
            ),
            Some("/home/tester"),
        );
        assert!(
            matches!(
                &error,
                AggregateInitializationError::SkillsPreparationDiagnosed { cause, .. }
                    if cause == "skills_sync_failed_claude"
            ),
            "expected skills_sync_failed_claude, got {error:?}"
        );
    }

    /// task 7.8：`CadenceSkillsError::UpdateFailed{stage}` **不丢 stage** ——
    /// 更新失败要能看出是准备/下载/安装哪一步坏，而不是通用串。
    #[test]
    fn update_failure_stage_becomes_structured_cause() {
        let error = skills_preparation_error(
            CadenceSkillsError::update_failed(
                "fetch",
                "git fetch exited with 128",
                "check the upstream remote and retry",
            ),
            None,
        );
        assert!(
            matches!(
                &error,
                AggregateInitializationError::SkillsPreparationDiagnosed { cause, reason, .. }
                    if cause == "skills_update_failed_fetch"
                        && reason.contains("fetch")
                        && reason.contains("cadence_skills_update_failed")
            ),
            "expected skills_update_failed_fetch carrying the stage, got {error:?}"
        );
    }

    /// task 7.8 的 detail 必须脱敏：HOME 前缀降级为 `~`，密钥形态被替换；
    /// 用户读到的诊断里不得出现明文 home 绝对路径与令牌。
    #[test]
    fn skills_failure_detail_is_redacted_and_localized() {
        let error = skills_preparation_error(
            CadenceSkillsError::update_failed(
                "fetch",
                "/home/alice/.agents/Cadence-skills: Authorization: Bearer \
                 ghp_012345678901234567890123456789012345 rejected",
                "check the upstream remote and retry",
            ),
            Some("/home/alice"),
        );
        let AggregateInitializationError::SkillsPreparationDiagnosed { reason, .. } = &error else {
            panic!("expected SkillsPreparationDiagnosed, got {error:?}");
        };
        assert!(
            !reason.contains("/home/alice"),
            "HOME 绝对前缀必须降级为 `~`，实际: {reason}"
        );
        assert!(
            !reason.contains("ghp_012345678901234567890123456789012345"),
            "令牌必须被脱敏，实际: {reason}"
        );
        assert!(
            reason.contains("~/"),
            "路径降级后应留下 `~/` 形态，实际: {reason}"
        );
    }

    /// 证据摘要失败（源目录 / 三层链接）统一 `skills_digest_failed`。
    #[test]
    fn digest_failure_uses_digest_cause() {
        let error = skills_digest_error("source", std::io::Error::other("permission denied"));
        assert!(
            matches!(
                &error,
                AggregateInitializationError::SkillsPreparationDiagnosed { cause, reason, .. }
                    if cause == "skills_digest_failed" && reason.contains("permission denied")
            ),
            "expected skills_digest_failed with the io reason, got {error:?}"
        );
    }
}
