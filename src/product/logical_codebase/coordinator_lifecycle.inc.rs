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
            trust: None,
        }
    }

    /// Task 1.4（REQ-REG-14）：注入五步 recipe 的 trust 硬前置门。registry
    /// 的 durable facts 按 (project_id, lc_id) 调用点 scope，故实例可安全
    /// 进入 state 级依赖图并在 `for_lc` 派生时原样复用。
    pub fn with_trust(
        mut self,
        trust: Arc<dyn crate::product::logical_codebase::ProviderTrustPrecondition>,
    ) -> Self {
        self.trust = Some(trust);
        self
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
            trust: self.trust.clone(),
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
        let operation = self.load_operation(project_id, operation_id)?;
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
        self.advance_remaining(project_id, operation_id, &cancellation)
            .await
    }

    /// C4 Task 4：显式 Continue——从 Failed（中断/可重试失败）的 durable
    /// operation 续跑。reopen 只重置 Failed 步骤；Completed 步骤（含
    /// provider turn 与 checkpoint）原样保留，续跑不重复执行。这是显式
    /// 动作，GET/投影绝不调用。
    pub async fn execute_remaining(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: CancellationToken,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        let operation = self.load_operation(project_id, operation_id)?;
        match operation.status {
            AggregateInitializationOperationStatus::Failed => {
                self.operations
                    .reopen_for_resume(project_id, operation_id, (self.clock)())?;
            }
            AggregateInitializationOperationStatus::Created
            | AggregateInitializationOperationStatus::Running => {}
            other => {
                return Err(AggregateInitializationError::state(
                    operation_id,
                    format!(
                        "operation is already {} and cannot be resumed",
                        serialise_status(other)
                    ),
                ));
            }
        }
        self.advance_remaining(project_id, operation_id, &cancellation)
            .await
    }

    /// Task 1.4（REQ-REG-14/REQ-BOOT-03）：trust 硬前置门 + 五步 root recipe
    /// 启动。先执行全部所选 trust gate（durable waiting 事实与审计由 gate
    /// 自行落盘）；任一 trust 未 Ready 即返回可重试 [`TrustWaiting`] 且绝不
    /// `begin`/`execute`——五步 operation 保持未创建、provider 零启动。全部
    /// Ready 后才创建/推进固定 Claude Code 的五步 recipe。
    ///
    /// [`TrustWaiting`]: AggregateInitializationError::TrustWaiting
    pub async fn execute_with_trust(
        &self,
        operation_id: String,
        project_id: &str,
        input: AggregateInitializationOperationInput,
        providers: &[crate::product::models::ProviderName],
        cancellation: CancellationToken,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        let Some(trust) = self.trust.clone() else {
            return Err(AggregateInitializationError::state(
                operation_id,
                "trust precondition is not configured for this coordinator",
            ));
        };
        // registry 的 durable facts 按 (project_id, lc_id) 调用点 scope：
        // 未 scope 的 coordinator 使用 legacy 别名 LC 标签（与
        // `AggregateInitializationOperationStore::new` 的落盘布局一致）。
        let lc_id = self
            .lc_id
            .clone()
            .unwrap_or_else(|| legacy_logical_codebase_id(project_id));
        // LC 根准入冻结的 canonical 聚合根：优先 canonicalize，根暂不可得时
        // 保持原样（后续 aggregate_preflight 会 fail-closed 拦截无效根）。
        let canonical_root = std::fs::canonicalize(&input.provider_context_root)
            .unwrap_or_else(|_| input.provider_context_root.clone());
        match trust.ensure_before_recipe(
            project_id,
            &operation_id,
            &lc_id,
            &canonical_root,
            providers,
        ) {
            crate::product::logical_codebase::ProviderTrustPreparationResult::Ready { .. } => {}
            crate::product::logical_codebase::ProviderTrustPreparationResult::Waiting { waiting } => {
                tracing::warn!(
                    project_id,
                    operation_id = %operation_id,
                    reason_code = %waiting.reason_code,
                    "aggregate initialization trust gate waiting; five-step recipe stays unstarted"
                );
                return Err(AggregateInitializationError::TrustWaiting {
                    waiting: Box::new(waiting),
                });
            }
        }
        let operation = self.begin(operation_id, project_id, input)?;
        self.execute(project_id, &operation.operation_id, cancellation)
            .await
    }

    /// 按五步顺序推进所有未完成步骤；Completed 步骤直接跳过（幂等重入与
    /// 续跑共用）。顺序保证仍由 `mark_step_running` 的前置完成检查执行。
    async fn advance_remaining(
        &self,
        project_id: &str,
        operation_id: &str,
        cancellation: &CancellationToken,
    ) -> Result<AggregateInitializationOperation, AggregateInitializationError> {
        let operation = self.load_operation(project_id, operation_id)?;
        let manifest = self.load_manifest(project_id, &operation)?;
        let step_completed = |kind: AggregateInitializationStepKind| {
            operation
                .steps
                .get(kind.index())
                .is_some_and(|step| step.status == AggregateInitializationStepStatus::Completed)
        };

        // machine_skills: deterministic, never a provider turn.
        if !step_completed(AggregateInitializationStepKind::MachineSkills)
            && let Err(error) = self
                .run_machine_skills(project_id, operation_id, cancellation)
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

        // aggregate_preflight: deterministic, never a provider turn. 续跑时
        // 直接复用 durable checkpoint 的 member projections 快照。
        let preflight =
            if step_completed(AggregateInitializationStepKind::AggregatePreflight) {
                self.load_persisted_preflight(operation_id)?
                    .ok_or_else(|| {
                        AggregateInitializationError::state(
                            operation_id,
                            "preflight checkpoint artifact is missing for resume",
                        )
                    })?
            } else {
                match self.run_aggregate_preflight(
                    project_id,
                    operation_id,
                    &manifest,
                    cancellation,
                ) {
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
            if step_completed(step) {
                continue;
            }
            self.run_provider_turn(project_id, operation_id, step, &preflight, cancellation)
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

    fn load_operation(
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

    /// 读取 aggregate_preflight 步骤 durable checkpoint 的成员投影快照。
    fn load_persisted_preflight(
        &self,
        operation_id: &str,
    ) -> Result<Option<AggregatePreflightSnapshot>, AggregateInitializationError> {
        let path = self.artifact_path(operation_id, "preflight.json")?;
        if !path.exists() {
            return Ok(None);
        }
        crate::product::json_store::read_json(&path)
            .map(Some)
            .map_err(AggregateInitializationError::from)
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
        // Task 1.4（REQ-BOOT-04）：root recipe 的每个 provider turn 都在自举
        // 相位运行。凭据必须从「与 coordinator 同一 lc scope 构造的 durable
        // store」派生（footgun 防线）：start_step 之后目标 step 已 Running 并
        // 记录 input digest，from_running_operation 据此做全维校验；任何漂移
        // 都 fail-closed 为可重试 provider turn 失败并保留 durable 失败事实。
        let canonical_root = PathBuf::from(&preflight.aggregate_root);
        let lc_label = self
            .lc_id
            .clone()
            .unwrap_or_else(|| legacy_logical_codebase_id(project_id));
        let bootstrap = match BootstrapPhaseCredential::from_running_operation(
            &self.operations,
            project_id,
            operation_id,
            step,
            &lc_label,
            &canonical_root,
        ) {
            Ok(credential) => credential,
            Err(
                crate::product::logical_codebase::provider_admission_preflight::ProviderAdmissionError::Store(store_error),
            ) => return Err(AggregateInitializationError::Store(store_error)),
            Err(waiting) => {
                let reason = format!("bootstrap phase credential denied: {waiting:?}");
                let record = AggregateInitializationError::ProviderTurn {
                    step,
                    reason: reason.clone(),
                    retryable: true,
                }
                .into_error_record();
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
                    reason,
                    retryable: true,
                });
            }
        };
        let cancellation_token = cancellation.clone();
        let turn_result = match self
            .provider
            .run_turn(AggregateProviderTurnRequest {
                project_id,
                operation_id,
                step,
                preflight,
                lc_id: self.lc_id.as_deref(),
                bootstrap,
                cancellation: cancellation_token,
            })
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
