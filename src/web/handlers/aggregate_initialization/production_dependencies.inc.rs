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
        // C4 Task 10：经 claude_code_with_admission 组装——provider turn 前
        // 先做实际成员规则/policy/capability 的 admission 预检（Task 8），
        // 缺失/漂移时 fail-closed，provider 保持零启动。lc_id 缺失（legacy
        // 别名解析失败）时无法定位 per-LC authority，保持无预检旧路径。
        let driver = match (lc_id, self.paths.as_ref()) {
            (Some(_), Some(paths)) => {
                GatewayBackedAggregateProviderTurnDriver::claude_code_with_admission(
                    Arc::new(gateway),
                    "cap_managed_snapshot",
                    paths.clone(),
                )
            }
            _ => GatewayBackedAggregateProviderTurnDriver::claude_code(
                Arc::new(gateway),
                "cap_managed_snapshot",
            ),
        };
        driver
            .run_turn(
                project_id,
                operation_id,
                step,
                preflight,
                lc_id,
                cancellation,
            )
            .await
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
        let coordinator = Arc::new(AggregateInitializationCoordinator::new(
            paths.clone(),
            operations,
            skills,
            preflight,
            provider,
            clock,
        ));
        let index = Arc::new(AggregateIndexOperation::new(
            paths.clone(),
            CodeGraphCli::new(state.command_runner.clone(), "codegraph".to_string()),
            CodeGraphExcludeGenerator,
        ));
        let trust = production_provider_trust_precondition(state, &paths)?;
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
