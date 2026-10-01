/// Adapts the shared Cadence skills manager to the aggregate coordinator's
/// durable machine-skills step. The manager owns source cloning/updating and
/// all three managed link layers; this adapter only records deterministic
/// evidence for the operation.
struct CadenceAggregateSkillsPreparation {
    manager: Arc<CadenceSkillsManager>,
}

#[async_trait::async_trait]
impl AggregateSkillsPreparation for CadenceAggregateSkillsPreparation {
    async fn prepare_skills(
        &self,
        _project_id: &str,
        _operation_id: &str,
        cancellation: CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
        let result = self.manager.prepare(cancellation).await.map_err(|error| {
            AggregateInitializationError::SkillsPreparation {
                reason: error.to_string(),
                retryable: true,
            }
        })?;
        let source_digest = digest_tree(self.manager.paths().source_root()).map_err(|error| {
            AggregateInitializationError::SkillsPreparation {
                reason: format!("skill source digest failed: {error}"),
                retryable: true,
            }
        })?;
        let link_digest = digest_link_layers(self.manager.paths()).map_err(|error| {
            AggregateInitializationError::SkillsPreparation {
                reason: format!("skill link digest failed: {error}"),
                retryable: true,
            }
        })?;
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
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        preflight: &AggregatePreflightSnapshot,
        lc_id: Option<&str>,
        bootstrap: crate::product::logical_codebase::provider_admission_preflight::BootstrapPhaseCredential,
        cancellation: CancellationToken,
    ) -> Result<String, AggregateInitializationError> {
        let Some(factory) = self.factory.as_ref() else {
            return Err(AggregateInitializationError::ProviderTurn {
                step,
                reason: "logical codebase gateway factory is not configured".to_string(),
                retryable: false,
            });
        };
        let gateway = factory.build_for_lc(project_id, lc_id).map_err(|error| {
            AggregateInitializationError::ProviderTurn {
                step,
                reason: format!("logical codebase gateway factory build failed: {error}"),
                retryable: true,
            }
        })?;
        // C4 Task 10 / Task 1.4：经 claude_code_with_admission 组装——
        // provider turn 前先做实际成员规则/policy/capability 的 admission
        // 预检（Task 8），缺失/漂移时 fail-closed，provider 保持零启动。
        // Task 1.8（D2/BOOT-03）：paths + lc_id 齐备的生产面再叠加 root
        // recipe 命令审计窗口（per-command before/after 快照 receipt），
        // 末命令 turn 成功后 finalize 最终 receipt（readiness 源 3）。
        // 任一侧缺失（legacy 别名解析失败的降级组装）保持无审计旧路径。
        if let (Some(lc_id), Some(paths)) = (lc_id, self.paths.as_ref()) {
            let watches = begin_root_recipe_command_watches(operation_id, step, preflight)
                .map_err(|error| root_recipe_audit_failure(step, error))?;
            let driver = GatewayBackedAggregateProviderTurnDriver::claude_code_with_admission(
                Arc::new(gateway),
                "cap_managed_snapshot",
                paths.clone(),
            );
            let summary = driver
                .run_turn(
                    project_id,
                    operation_id,
                    step,
                    preflight,
                    Some(lc_id),
                    bootstrap,
                    cancellation,
                )
                .await?;
            // 命令未执行完成（turn 失败/取消）不落 receipt：无执行即无证据，
            // 重试窗口重新审计；同 index 同内容重放幂等。
            complete_root_recipe_command_receipts(paths, lc_id, project_id, watches)
                .map_err(|error| root_recipe_audit_failure(step, error))?;
            if step_owns_final_recipe_command(step) {
                try_finalize_root_recipe_receipt(
                    paths,
                    lc_id,
                    project_id,
                    operation_id,
                    &std::path::PathBuf::from(&preflight.aggregate_root),
                );
            }
            return Ok(summary);
        }
        let driver = GatewayBackedAggregateProviderTurnDriver::claude_code(
            Arc::new(gateway),
            "cap_managed_snapshot",
        );
        driver
            .run_turn(
                project_id,
                operation_id,
                step,
                preflight,
                lc_id,
                bootstrap,
                cancellation,
            )
            .await
    }
}

/// Task 1.8（D2）：本 step 在固定 root recipe 命令索引中的命令
/// （全局 index + 命令文本）。确定性 step 无命令（空表）。
fn root_recipe_step_commands(step: AggregateInitializationStepKind) -> Vec<(usize, &'static str)> {
    crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index()
        .into_iter()
        .filter(|(command_step, _, _)| *command_step == step)
        .map(|(_, command_index, command)| (command_index, command))
        .collect()
}

/// 末命令判定：本 step 持有全局最大 command index（固定顺序下即
/// OpenspecAndExamples 的命令 4）——最终 receipt 的 finalize 挂点。
fn step_owns_final_recipe_command(step: AggregateInitializationStepKind) -> bool {
    let step_max = root_recipe_step_commands(step)
        .iter()
        .map(|(command_index, _)| *command_index)
        .max();
    let global_max = crate::product::logical_codebase::aggregate_initialization_store::root_recipe_command_index()
        .iter()
        .map(|(_, command_index, _)| *command_index)
        .max();
    step_max.is_some_and(|step_max| Some(step_max) == global_max)
}

/// 命令审计窗口开启：canonical root 冻结 + before 全量快照。IO/命令身份
/// 漂移 fail-closed 为可重试 ProviderTurn 失败（不可观测绝不静默）。
fn begin_root_recipe_command_watches(
    operation_id: &str,
    step: AggregateInitializationStepKind,
    preflight: &AggregatePreflightSnapshot,
) -> Result<
    Vec<crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandWatch>,
    crate::product::json_store::ProductStoreError,
> {
    let auditor =
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeFilesystemAuditor::new();
    let canonical_root = std::path::PathBuf::from(&preflight.aggregate_root);
    root_recipe_step_commands(step)
        .into_iter()
        .map(|(command_index, command)| {
            auditor.before_command(operation_id, &canonical_root, step, command_index, command)
        })
        .collect()
}

/// 命令成功后逐条落 per-command receipt（拒绝也 durable 保留证据）。
/// 同 index 同内容重放幂等；同 index 不同内容冲突不覆盖（fail-closed）。
fn complete_root_recipe_command_receipts(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
    watches: Vec<crate::product::logical_codebase::root_recipe_receipt::RootRecipeCommandWatch>,
) -> Result<(), crate::product::json_store::ProductStoreError> {
    let auditor =
        crate::product::logical_codebase::root_recipe_receipt::RootRecipeFilesystemAuditor::new();
    let store = crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
        paths.clone(),
        lc_id.to_string(),
    );
    for watch in watches {
        let receipt = auditor.after_command(watch, chrono::Utc::now().to_rfc3339())?;
        store.append_command(project_id, receipt)?;
    }
    Ok(())
}

/// 末命令 turn 成功后 finalize 最终 receipt（readiness 源 3 的闭环）。
/// policy digest 取 authority 冻结的聚合 policy artifact 正文 digest；
/// rule digest 经 [`root_recipe_receipt::root_rule_digest`] 计算——与
/// readiness 投影共用同一实现（Task 1.6 契约）。失败只 warn：readiness
/// 投影以 root_receipt_missing/root_rule_missing 等待面呈现（BOOT-03
/// 「recipe 成功但 readiness 材料未齐」），不回滚已完成的 operation。
fn try_finalize_root_recipe_receipt(
    paths: &ProductAppPaths,
    lc_id: &str,
    project_id: &str,
    operation_id: &str,
    provider_context_root: &std::path::Path,
) {
    let outcome = (|| {
        let policy_digest =
            crate::product::logical_codebase::policy::AggregatePolicyArtifactStore::for_lc(
                paths.clone(),
                lc_id.to_string(),
            )
            .get(project_id)?
            .ok_or_else(
                || crate::product::json_store::ProductStoreError::InvalidRecord {
                    kind: "root_recipe_receipt",
                    reason: "aggregate policy artifact is missing at finalize time".to_string(),
                },
            )?
            .digest;
        let canonical_root = std::fs::canonicalize(provider_context_root)
            .unwrap_or_else(|_| provider_context_root.to_path_buf());
        let rule_digest = crate::product::logical_codebase::root_recipe_receipt::root_rule_digest(
            &canonical_root,
        )?
        .ok_or_else(
            || crate::product::json_store::ProductStoreError::InvalidRecord {
                kind: "root_recipe_receipt",
                reason: format!(
                    "{} is absent under the canonical root {}",
                    crate::product::logical_codebase::root_recipe_receipt::ROOT_RULE_ENTRY_FILE,
                    canonical_root.display()
                ),
            },
        )?;
        crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
            paths.clone(),
            lc_id.to_string(),
        )
        .finalize(
            project_id,
            operation_id,
            &policy_digest,
            &rule_digest,
            chrono::Utc::now().to_rfc3339(),
        )
        .map(|_| ())
    })();
    if let Err(error) = outcome {
        tracing::warn!(
            project_id,
            operation_id,
            lc_id,
            error = %error,
            "root recipe receipt finalize deferred; readiness projection keeps waiting"
        );
    }
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
            home,
            state.command_runner.clone(),
            environment,
        ));
        let skills: Arc<dyn AggregateSkillsPreparation> =
            Arc::new(CadenceAggregateSkillsPreparation { manager });
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
            .with_trust(trust.clone()),
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
