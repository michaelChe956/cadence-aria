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
}

impl GatewayFactoryProviderTurnDriver {
    fn new(factory: Option<Arc<LogicalCodebaseGatewayFactory>>) -> Self {
        Self { factory }
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
        GatewayBackedAggregateProviderTurnDriver::claude_code(
            Arc::new(gateway),
            "cap_managed_snapshot",
        )
        .run_turn(project_id, operation_id, step, preflight, lc_id, cancellation)
        .await
    }
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
        let provider: Arc<dyn AggregateProviderTurnDriver> = Arc::new(
            GatewayFactoryProviderTurnDriver::new(state.gateway_factory().cloned()),
        );
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
            paths,
            CodeGraphCli::new(state.command_runner.clone(), "codegraph".to_string()),
            CodeGraphExcludeGenerator,
        ));
        Ok(Self::with_index(
            coordinator,
            InitializationRunRegistry::default(),
            index,
        ))
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
