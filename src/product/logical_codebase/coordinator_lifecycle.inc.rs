impl AggregateInitializationCoordinator {
    pub fn new(
        paths: ProductAppPaths,
        operations: AggregateInitializationOperationStore,
        skills: Arc<dyn AggregateSkillsPreparation>,
        preflight: Arc<dyn AggregatePreflightService>,
        provider: Arc<dyn AggregateProviderTurnDriver>,
        clock: Arc<Clock>,
    ) -> Self {
        Self::with_detector(
            paths,
            operations,
            skills,
            preflight,
            provider,
            Arc::new(DeterministicRepositoryTypeDetector::new()),
            clock,
        )
    }

    /// Construct a coordinator with an explicit repository-type detector,
    /// allowing tests and future integrations to override profile detection
    /// while keeping the five stable step IDs unchanged.
    pub fn with_detector(
        paths: ProductAppPaths,
        operations: AggregateInitializationOperationStore,
        skills: Arc<dyn AggregateSkillsPreparation>,
        preflight: Arc<dyn AggregatePreflightService>,
        provider: Arc<dyn AggregateProviderTurnDriver>,
        detector: Arc<dyn RepositoryTypeDetector>,
        clock: Arc<Clock>,
    ) -> Self {
        Self {
            paths,
            lc_id: None,
            operations,
            skills,
            preflight,
            provider,
            detector,
            clock,
        }
    }

    /// Re-scopes the durable operation store, manifest/member reads and the
    /// deterministic preflight service to one logical codebase subtree, while
    /// reusing the same skills/provider/detector/clock components.
    pub fn for_lc(&self, lc_id: impl Into<String>) -> Self {
        let lc_id = lc_id.into();
        let preflight = self
            .preflight
            .rescoped(&lc_id)
            .unwrap_or_else(|| Arc::clone(&self.preflight));
        Self {
            paths: self.paths.clone(),
            lc_id: Some(lc_id.clone()),
            operations: AggregateInitializationOperationStore::for_lc(
                self.paths.clone(),
                lc_id,
            ),
            skills: Arc::clone(&self.skills),
            preflight,
            provider: Arc::clone(&self.provider),
            detector: Arc::clone(&self.detector),
            clock: Arc::clone(&self.clock),
        }
    }

    fn authority_store(&self) -> LogicalCodebaseStore {
        match &self.lc_id {
            Some(lc_id) => LogicalCodebaseStore::for_lc(self.paths.clone(), lc_id.clone()),
            None => LogicalCodebaseStore::new(self.paths.clone()),
        }
    }

    /// Create the operation idempotently. Returns the persisted record whether
    /// it was newly created or matched an existing idempotent request.
    pub fn begin(
        &self,
        operation_id: String,
        project_id: &str,
        input: AggregateInitializationOperationInput,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        validate_relative_id(project_id).map_err(|error| {
            AggregateInitializationError::state(
                operation_id.clone(),
                format!("invalid project id: {error}"),
            )
        })?;
        validate_relative_id(&operation_id).map_err(|error| {
            AggregateInitializationError::state(
                operation_id.clone(),
                format!("invalid operation id: {error}"),
            )
        })?;
        let operation = AggregateInitializationOperation::new(
            operation_id,
            project_id.to_string(),
            input,
            (self.clock)(),
        );
        self.operations
            .create_idempotent(operation)
            .map_err(AggregateInitializationError::from)
    }

    pub fn get(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        self.operations
            .get(project_id, operation_id)
            .map_err(|error| match error {
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })
    }

    /// Advance the operation through every remaining step in strict order. The
    /// operation must already exist (created via [`Self::begin`]). Machine skills
    /// and preflight run as deterministic Cadence code; exactly three provider
    /// turns run afterwards. Failures mark the operation failed and leave later
    /// steps `Pending`.
    pub async fn execute(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: CancellationToken,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        let operation =
            self.operations
                .get(project_id, operation_id)
                .map_err(|error| match error {
                    ProductStoreError::NotFound { id, .. } => {
                        AggregateInitializationError::not_found(id)
                    }
                    other => AggregateInitializationError::Store(other),
                })?;
        if operation.status == AggregateInitializationOperationStatus::Created {
            self.operations
                .mark_running(project_id, operation_id, (self.clock)())?;
        } else if operation.status != AggregateInitializationOperationStatus::Running {
            return Err(AggregateInitializationError::state(
                operation_id,
                format!(
                    "operation is already {} and cannot be re-executed",
                    serialise_status(operation.status)
                ),
            ));
        }

        let manifest = self.load_manifest(project_id, &operation)?;

        // machine_skills: deterministic, never a provider turn.
        if let Err(error) = self
            .run_machine_skills(project_id, operation_id, &cancellation)
            .await
        {
            tracing::warn!(
                project_id,
                operation_id,
                error = %error,
                "aggregate initialization failed during machine skills preparation"
            );
            return Err(error);
        }
        if cancellation.is_cancelled() {
            return self.fail_interrupted(project_id, operation_id);
        }

        // aggregate_preflight: deterministic, never a provider turn.
        let preflight = match self
            .run_aggregate_preflight(project_id, operation_id, &manifest, &cancellation)
        {
            Ok(preflight) => preflight,
            Err(error) => {
                tracing::warn!(
                    project_id,
                    operation_id,
                    error = %error,
                    "aggregate initialization failed during aggregate preflight"
                );
                return Err(error);
            }
        };
        if cancellation.is_cancelled() {
            return self.fail_interrupted(project_id, operation_id);
        }

        // Three provider turns, all after the deterministic steps.
        for step in [
            AggregateInitializationStepKind::PreCheck,
            AggregateInitializationStepKind::RuleAndMcpConfig,
            AggregateInitializationStepKind::OpenspecAndExamples,
        ] {
            self.run_provider_turn(project_id, operation_id, step, &preflight, &cancellation)
                .await?;
            if cancellation.is_cancelled() {
                return self.fail_interrupted(project_id, operation_id);
            }
        }

        let operation = self
            .operations
            .finish_completed(project_id, operation_id, (self.clock)())
            .map_err(|error| match error {
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })?;
        Ok(operation)
    }

    pub fn cancel(
        &self,
        project_id: &str,
        operation_id: &str,
        reason_code: &str,
        detail: Option<String>,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        let now = (self.clock)();
        self.operations
            .cancel(
                project_id,
                operation_id,
                AggregateCancellationRecord {
                    reason_code: reason_code.to_string(),
                    cancelled_at: now.clone(),
                    detail,
                },
                now,
            )
            .map_err(|error| match error {
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })
    }

    pub fn recover_interrupted(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        self.operations
            .recover_interrupted(project_id, operation_id, (self.clock)())
            .map_err(|error| match error {
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })
    }

    /// Resolve the aggregate initialization profile from read-only member
    /// checkout signals. The detector only reads each member's main checkout
    /// root; it never recurses, follows symlinks outside the root, executes
    /// package scripts, runs `pnpm install`, Node or Java. The five stable
    /// step IDs are unaffected — only the template/precheck selection changes.
    pub fn preflight_profile(
        &self,
        project_id: &str,
    ) -> Result<AggregateInitializationProfile, AggregateInitializationError> {
        validate_relative_id(project_id).map_err(|error| {
            AggregateInitializationError::state(project_id, format!("invalid project id: {error}"))
        })?;
        let _manifest = self
            .authority_store()
            .load_manifest(project_id)
            .map_err(|error| AggregateInitializationError::Preflight {
                reason: format!("manifest could not be loaded: {error}"),
                retryable: true,
            })?
            .ok_or_else(|| AggregateInitializationError::Preflight {
                reason: "logical codebase manifest is missing; register members first".to_string(),
                retryable: false,
            })?;
        let store = self.authority_store();
        let members = store.list_members(project_id).map_err(|error| {
            AggregateInitializationError::Preflight {
                reason: format!("members could not be loaded: {error}"),
                retryable: true,
            }
        })?;
        let checkouts = store.list_checkouts(project_id).map_err(|error| {
            AggregateInitializationError::Preflight {
                reason: format!("checkouts could not be loaded: {error}"),
                retryable: true,
            }
        })?;
        let mut evidence = Vec::with_capacity(members.len());
        for member in &members {
            let main = checkouts
                .iter()
                .find(|checkout| checkout.logical_repository_id == member.logical_repository_id)
                .ok_or_else(|| AggregateInitializationError::Preflight {
                    reason: format!(
                        "member {} has no recorded checkout",
                        member.logical_repository_id.0
                    ),
                    retryable: false,
                })?;
            let detected = self.detector.detect(
                &main.canonical_path,
                &member.logical_repository_id.0.to_string(),
            )?;
            evidence.push(detected);
        }
        resolve_aggregate_profile(&evidence)
    }

    /// Profile-specific preflight command templates for the resolved profile.
    /// Frontend pnpm/Vite never includes Maven/Gradle commands.
    pub fn preflight_commands(
        &self,
        project_id: &str,
    ) -> Result<Vec<String>, AggregateInitializationError> {
        let profile = self.preflight_profile(project_id)?;
        Ok(profile_preflight_commands(profile))
    }

    async fn run_machine_skills(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<MachineSkillsPreparation, AggregateInitializationError> {
        let step = AggregateInitializationStepKind::MachineSkills;
        let input_digest = self.input_digest(project_id, operation_id, step, "skills:v1");
        self.start_step(project_id, operation_id, step, &input_digest)?;
        let cancellation_token = cancellation.clone();
        let result = self
            .skills
            .prepare_skills(project_id, operation_id, cancellation_token)
            .await?;
        let output_ref = self.machine_skills_output_ref(operation_id);
        self.checkpoint_output(project_id, operation_id, step, output_ref)?;
        // Persist the immutable skill summary alongside the operation artifact.
        self.persist_machine_skills(operation_id, &result)?;
        self.complete_step(project_id, operation_id, step)?;
        Ok(result)
    }

    fn run_aggregate_preflight(
        &self,
        project_id: &str,
        operation_id: &str,
        manifest: &LogicalCodebaseManifest,
        cancellation: &CancellationToken,
    ) -> Result<AggregatePreflightSnapshot, AggregateInitializationError> {
        let step = AggregateInitializationStepKind::AggregatePreflight;
        let input_digest = self.input_digest(
            project_id,
            operation_id,
            step,
            &format!("manifest:{}", manifest.membership_revision),
        );
        self.start_step(project_id, operation_id, step, &input_digest)?;
        let snapshot = self.preflight.inspect(project_id, manifest, cancellation)?;
        let output_ref = self.preflight_output_ref(operation_id);
        self.checkpoint_output(project_id, operation_id, step, output_ref)?;
        self.persist_preflight(operation_id, &snapshot)?;
        self.complete_step(project_id, operation_id, step)?;
        Ok(snapshot)
    }

    async fn run_provider_turn(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        preflight: &AggregatePreflightSnapshot,
        cancellation: &CancellationToken,
    ) -> Result<(), AggregateInitializationError> {
        let input_digest = self.input_digest(project_id, operation_id, step, "provider:v1");
        self.start_step(project_id, operation_id, step, &input_digest)?;
        let cancellation_token = cancellation.clone();
        let turn_result = match self
            .provider
            .run_turn(
                project_id,
                operation_id,
                step,
                preflight,
                self.lc_id.as_deref(),
                cancellation_token,
            )
            .await
        {
            Ok(summary) => summary,
            Err(error) => {
                let record = error.into_error_record();
                let failed = self.operations.finish_failed(
                    project_id,
                    operation_id,
                    Some(step),
                    record,
                    (self.clock)(),
                );
                if let Err(store_error) = failed {
                    return Err(store_error.into());
                }
                return Err(AggregateInitializationError::ProviderTurn {
                    step,
                    reason: "provider turn failed and operation was marked failed".to_string(),
                    retryable: true,
                });
            }
        };
        let output_ref = self.provider_output_ref(operation_id, step, &turn_result);
        self.checkpoint_output(project_id, operation_id, step, output_ref)?;
        self.complete_step(project_id, operation_id, step)?;
        Ok(())
    }

    fn start_step(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        input_digest: &str,
    ) -> Result<(), AggregateInitializationError> {
        self.operations
            .mark_step_running(
                project_id,
                operation_id,
                step,
                input_digest.to_string(),
                (self.clock)(),
            )
            .map_err(|error| match error {
                ProductStoreError::IdentityMismatch { .. } => AggregateInitializationError::state(
                    operation_id,
                    format!("step {} cannot start out of order", step.as_str()),
                ),
                ProductStoreError::NotFound { id, .. } => {
                    AggregateInitializationError::not_found(id)
                }
                other => AggregateInitializationError::Store(other),
            })?;
        Ok(())
    }

    fn checkpoint_output(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        output_ref: String,
    ) -> Result<(), AggregateInitializationError> {
        self.operations
            .checkpoint_step_output(project_id, operation_id, step, output_ref, (self.clock)())
            .map(|_| ())
            .map_err(AggregateInitializationError::from)
    }

    fn complete_step(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
    ) -> Result<(), AggregateInitializationError> {
        self.operations
            .mark_step_completed(project_id, operation_id, step, (self.clock)())
            .map(|_| ())
            .map_err(AggregateInitializationError::from)
    }

    fn fail_interrupted(
        &self,
        project_id: &str,
        operation_id: &str,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        self.operations
            .recover_interrupted(project_id, operation_id, (self.clock)())
            .map_err(AggregateInitializationError::from)?;
        Err(AggregateInitializationError::Cancelled)
    }

    fn load_manifest(
        &self,
        project_id: &str,
        operation: &AggregateInitializationOperation,
    ) -> Result<LogicalCodebaseManifest, AggregateInitializationError> {
        let store = self.authority_store();
        store
            .load_manifest(project_id)
            .map_err(|error| {
                AggregateInitializationError::state(
                    &operation.operation_id,
                    format!("manifest could not be loaded: {error}"),
                )
            })?
            .ok_or_else(|| {
                AggregateInitializationError::state(
                    &operation.operation_id,
                    "logical codebase manifest is missing; register members first",
                )
            })
    }

    fn input_digest(
        &self,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        input: &str,
    ) -> String {
        let mut hasher = Sha256::new();
        hasher.update(input.as_bytes());
        let digest = hasher.finalize();
        format!(
            "aggregate-init:{}:{}:{}:{:x}",
            project_id,
            operation_id,
            step.as_str(),
            digest
        )
    }

    fn machine_skills_output_ref(&self, operation_id: &str) -> String {
        format!("aggregate-initializations/{operation_id}/machine_skills.json")
    }

    fn preflight_output_ref(&self, operation_id: &str) -> String {
        format!("aggregate-initializations/{operation_id}/preflight.json")
    }

    fn provider_output_ref(
        &self,
        operation_id: &str,
        step: AggregateInitializationStepKind,
        _summary: &str,
    ) -> String {
        format!(
            "aggregate-initializations/{operation_id}/{}.json",
            step.as_str()
        )
    }

    fn persist_machine_skills(
        &self,
        operation_id: &str,
        preparation: &MachineSkillsPreparation,
    ) -> Result<(), AggregateInitializationError> {
        let path = self.artifact_path(operation_id, "machine_skills.json")?;
        crate::product::json_store::write_json(&path, preparation)
            .map_err(AggregateInitializationError::from)
    }

    fn persist_preflight(
        &self,
        operation_id: &str,
        snapshot: &AggregatePreflightSnapshot,
    ) -> Result<(), AggregateInitializationError> {
        let path = self.artifact_path(operation_id, "preflight.json")?;
        crate::product::json_store::write_json(&path, snapshot)
            .map_err(AggregateInitializationError::from)
    }

    fn artifact_path(
        &self,
        operation_id: &str,
        name: &str,
    ) -> Result<PathBuf, AggregateInitializationError> {
        validate_relative_id(operation_id).map_err(|error| {
            AggregateInitializationError::state(
                operation_id,
                format!("invalid operation id: {error}"),
            )
        })?;
        Ok(self
            .paths
            .aggregate_initializations_root("")
            .join(operation_id)
            .join(name))
    }
}

fn serialise_status(status: AggregateInitializationOperationStatus) -> &'static str {
    match status {
        AggregateInitializationOperationStatus::Created => "created",
        AggregateInitializationOperationStatus::Running => "running",
        AggregateInitializationOperationStatus::Completed => "completed",
        AggregateInitializationOperationStatus::Failed => "failed",
        AggregateInitializationOperationStatus::Cancelled => "cancelled",
    }
}
