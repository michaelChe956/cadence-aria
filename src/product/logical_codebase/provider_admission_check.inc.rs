fn bootstrap_waiting(reason_code: &str, detail: String) -> ProviderAdmissionError {
    ProviderAdmissionError::Waiting {
        reason_code: reason_code.to_string(),
        detail,
        missing_materials: Vec::new(),
        allowed_actions: vec![BootstrapActionKind::Revalidate, BootstrapActionKind::Retry],
    }
}

fn ensure_bootstrap_operation_running(
    operation: &AggregateInitializationOperation,
) -> Result<(), ProviderAdmissionError> {
    if operation.status != AggregateInitializationOperationStatus::Running {
        return Err(bootstrap_waiting(
            "bootstrap_operation_not_running",
            format!(
                "aggregate initialization operation {} is {:?}, not Running",
                operation.operation_id, operation.status
            ),
        ));
    }
    Ok(())
}

fn ensure_bootstrap_step_is_provider_turn(
    step: AggregateInitializationStepKind,
) -> Result<(), ProviderAdmissionError> {
    if !step.is_provider_turn() {
        return Err(bootstrap_waiting(
            "bootstrap_step_not_provider_turn",
            format!(
                "bootstrap step {step:?} is deterministic and must not derive a provider credential"
            ),
        ));
    }
    Ok(())
}

fn bootstrap_step_record<'a>(
    operation: &'a AggregateInitializationOperation,
    step: AggregateInitializationStepKind,
) -> Result<&'a AggregateInitializationStepRecord, ProviderAdmissionError> {
    let record = operation.steps.get(step.index()).ok_or_else(|| {
        bootstrap_waiting(
            "bootstrap_step_not_running",
            format!(
                "operation {} has no record for step {step:?}",
                operation.operation_id
            ),
        )
    })?;
    if record.status != AggregateInitializationStepStatus::Running {
        return Err(bootstrap_waiting(
            "bootstrap_step_not_running",
            format!(
                "bootstrap step {step:?} of operation {} is {:?}, not Running",
                operation.operation_id, record.status
            ),
        ));
    }
    Ok(record)
}

fn bootstrap_roots_match(credential_root: &Path, durable_root: &Path) -> bool {
    if credential_root == durable_root {
        return true;
    }
    match (
        std::fs::canonicalize(credential_root),
        std::fs::canonicalize(durable_root),
    ) {
        (Ok(left), Ok(right)) => left == right,
        _ => false,
    }
}

/// 准入预检结果：材料齐备且 gateway validate 通过时 `ready == true`。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ProviderAdmissionPreflightResult {
    pub target: ResolvedTargetIdentity,
    pub authority_root: PathBuf,
    pub policy: AuthorityPolicyReference,
    pub rules: Vec<ProviderRuleReference>,
    pub capability_snapshot_ref: String,
    pub ready: bool,
    pub missing_materials: Vec<String>,
    pub allowed_actions: Vec<BootstrapActionKind>,
}

/// 预检失败：waiting 是可操作的持久等待事实（携带缺失材料与允许动作），
/// 其余为不可恢复的存储/路由错误。
#[derive(Debug)]
pub enum ProviderAdmissionError {
    Waiting {
        reason_code: String,
        detail: String,
        missing_materials: Vec<String>,
        allowed_actions: Vec<BootstrapActionKind>,
    },
    Store(ProductStoreError),
}

impl From<ProductStoreError> for ProviderAdmissionError {
    fn from(error: ProductStoreError) -> Self {
        Self::Store(error)
    }
}

/// LC-scoped provider admission preflight：resolver + policy + gateway。
pub struct LogicalCodebaseProviderAdmissionPreflight {
    paths: ProductAppPaths,
    lc_id: String,
    gateway: Arc<LogicalCodebaseProviderGateway>,
}

impl LogicalCodebaseProviderAdmissionPreflight {
    pub fn new(
        paths: ProductAppPaths,
        lc_id: impl Into<String>,
        gateway: Arc<LogicalCodebaseProviderGateway>,
    ) -> Self {
        Self {
            paths,
            lc_id: lc_id.into(),
            gateway,
        }
    }

    /// 预检一次 provider 启动请求。只读 durable 事实 + gateway `validate`
    /// （validate 不启动 provider）；任何缺失/漂移都返回 waiting 事实而非
    /// 让运行时 Failed。
    ///
    /// `phase`（Task 1.2，REQ-BOOT-04）：`AggregateBootstrap` 携带有效凭据时
    /// **只豁免「根规则尚未生成」的存在性检查**——凭据先对 durable Running
    /// operation 重核验（status/step/input digest/LC/root），漂移即 waiting；
    /// authority/policy/capability/gateway/cwd/target 全部照常必检。
    pub fn check(
        &self,
        request: &SessionLaunchRequest,
        phase: &ProviderAdmissionPhase,
    ) -> Result<ProviderAdmissionPreflightResult, ProviderAdmissionError> {
        // 0. 相位凭据先核验（REQ-BOOT-04）：AggregateBootstrap 凭据必须仍与
        //    durable Running operation 一致；失效/漂移凭据 fail-closed，
        //    绝不降级为普通 session 或放宽其他维度。
        let waive_missing_root_rules = match phase {
            ProviderAdmissionPhase::Normal => false,
            ProviderAdmissionPhase::AggregateBootstrap(credential) => {
                if credential.project_id != request.project_id {
                    return Err(bootstrap_waiting(
                        "bootstrap_project_mismatch",
                        format!(
                            "credential project {} does not match launch request project {}",
                            credential.project_id, request.project_id
                        ),
                    ));
                }
                let operations = AggregateInitializationOperationStore::for_lc(
                    self.paths.clone(),
                    self.lc_id.clone(),
                );
                credential.reverify_against_running_operation(&operations, &self.lc_id)?;
                true
            }
        };
        let resolution = RepositoryAuthorityResolver::new(self.paths.clone()).resolve(
            RepositoryRoutingRequest {
                project_id: request.project_id.clone(),
                issue_id: None,
                kind: RepositoryTargetKind::LogicalCodebase,
                repository_id: None,
                logical_codebase_id: Some(self.lc_id.clone()),
                logical_repository_id: parse_member_id(&request.target.logical_repository_id),
                checkout_id: parse_checkout_id(&request.target.checkout_id),
            },
        )?;

        let mut missing_materials = Vec::new();
        let mut allowed_actions = Vec::new();
        // 根规则存在性缺失（Task 1.2 与其他材料分离：自举相位凭据是唯一
        // 豁免面，其余维度永不豁免）。
        let mut missing_rules = Vec::new();

        // 1. manifest：冷启动未登记时投影 waiting（Prepare）。
        let manifest =
            resolution
                .manifest
                .as_ref()
                .ok_or_else(|| ProviderAdmissionError::Waiting {
                    reason_code: "logical_codebase_manifest_missing".to_string(),
                    detail: format!(
                        "logical codebase {} has no manifest; register members first",
                        self.lc_id
                    ),
                    missing_materials: vec![format!(
                        "logical-codebases/{}/manifest.json",
                        self.lc_id
                    )],
                    allowed_actions: vec![BootstrapActionKind::Prepare],
                })?;

        // 2. 实际成员规则：每个 active 成员的 main checkout 必须有
        //    `.claude/rules/language.md`（与 single_candidate_author 同一路径）。
        let logical = LogicalCodebaseStore::for_lc(self.paths.clone(), &self.lc_id);
        let members = logical.list_members(&request.project_id)?;
        let checkouts = logical.list_checkouts(&request.project_id)?;
        let mut rules = Vec::new();
        for member in &members {
            if member.status != MemberStatus::Active {
                continue;
            }
            let Some(checkout) = checkouts
                .iter()
                .find(|checkout| {
                    member.checkout_ids.contains(&checkout.checkout_id)
                        && checkout.kind == CheckoutKind::Main
                })
                .or_else(|| {
                    checkouts
                        .iter()
                        .find(|checkout| member.checkout_ids.contains(&checkout.checkout_id))
                })
            else {
                missing_materials.push(format!(
                    "member {} ({}) has no recorded checkout",
                    member.alias, member.logical_repository_id.0
                ));
                continue;
            };
            let rule_path = checkout.canonical_path.join(".claude/rules/language.md");
            match std::fs::read(&rule_path) {
                Ok(bytes) => rules.push(ProviderRuleReference {
                    member_id: member.logical_repository_id,
                    checkout_id: checkout.checkout_id,
                    path: rule_path,
                    digest: Some(format!("sha256:{:x}", sha2::Sha256::digest(&bytes))),
                }),
                Err(_) => {
                    missing_rules.push(format!(
                        "member {} missing {}",
                        member.alias,
                        rule_path.display()
                    ));
                    rules.push(ProviderRuleReference {
                        member_id: member.logical_repository_id,
                        checkout_id: checkout.checkout_id,
                        path: rule_path,
                        digest: None,
                    });
                }
            }
        }
        if waive_missing_root_rules {
            // BOOT-04（Task 1.2）：有效自举凭据只豁免「根规则尚未生成」——
            // 缺失规则保持为记录事实（digest=None），不构成阻断材料，也
            // 不污染后续维度的 missing_materials；其余维度照常必检。
        } else if !missing_rules.is_empty() {
            missing_materials.extend(missing_rules.clone());
            allowed_actions.push(BootstrapActionKind::Prepare);
            allowed_actions.push(BootstrapActionKind::Retry);
        }

        // 3. 聚合 policy artifact：digest/revision 由 store 校验后冻结进引用。
        let policy = resolution.policy.clone().ok_or_else(|| {
            let mut materials = missing_materials.clone();
            materials.push(format!(
                "logical-codebases/{}/aggregate-policy.json",
                self.lc_id
            ));
            ProviderAdmissionError::Waiting {
                reason_code: "aggregate_policy_artifact_missing".to_string(),
                detail: format!(
                    "logical codebase {} has no aggregate policy artifact",
                    self.lc_id
                ),
                missing_materials: materials,
                allowed_actions: vec![BootstrapActionKind::Prepare, BootstrapActionKind::Retry],
            }
        })?;

        // 4. 真实 gateway 谓词：bootstrap capability 记录不满足 snapshot/action
        //    时在此拒绝（validate 不启动 provider）。
        let validated = self.gateway.validate(request.clone()).map_err(|error| {
            let reason_code = match &error {
                ProviderGatewayError::UnsupportedCapability(_) => {
                    "provider_capability_not_satisfied"
                }
                ProviderGatewayError::PolicyMissing(_) => "aggregate_policy_artifact_missing",
                _ => "provider_gateway_denied",
            };
            let mut materials = missing_materials.clone();
            if matches!(error, ProviderGatewayError::PolicyMissing(_)) {
                materials.push(format!(
                    "logical-codebases/{}/aggregate-policy.json",
                    self.lc_id
                ));
            }
            ProviderAdmissionError::Waiting {
                reason_code: reason_code.to_string(),
                detail: error.to_string(),
                missing_materials: materials,
                allowed_actions: vec![BootstrapActionKind::Revalidate, BootstrapActionKind::Retry],
            }
        })?;

        // 5. envelope 与 resolver 冻结值比对：authority root / policy digest 漂移
        //    在 spawn 前转为 waiting（Revalidate），绝不回落旧路径。
        let envelope = validated.envelope();
        if envelope.authority_root != resolution.authority_root {
            return Err(ProviderAdmissionError::Waiting {
                reason_code: "authority_root_drift".to_string(),
                detail: format!(
                    "envelope authority root {} does not match resolver-frozen root {}",
                    envelope.authority_root.display(),
                    resolution.authority_root.display()
                ),
                missing_materials: missing_materials.clone(),
                allowed_actions: vec![BootstrapActionKind::Revalidate],
            });
        }
        if envelope.policy_digest != policy.policy_digest
            || envelope.policy_revision != policy.policy_revision
        {
            return Err(ProviderAdmissionError::Waiting {
                reason_code: "policy_digest_drift".to_string(),
                detail: format!(
                    "envelope policy {}/{} does not match authority policy {}/{}",
                    envelope.policy_id,
                    envelope.policy_revision,
                    policy.policy_id,
                    policy.policy_revision
                ),
                missing_materials: missing_materials.clone(),
                allowed_actions: vec![BootstrapActionKind::Revalidate],
            });
        }

        // 6. cwd 复验（pub(crate) 拓宽后由 admission 显式调用）：policy
        //    revision/digest、capability、config digest、cwd 的 TOCTOU 全维度
        //    复核；复验 cwd 取请求的独立 `working_directory`（Task 2.5 字段，
        //    Task 2.1 接线——cwd≠target 的 LC root 形态由此复验，现状消费方
        //    cwd==target 零变化）。失败转 waiting，provider 保持零启动。
        let cwd = if request.working_directory.is_absolute() {
            request.working_directory.clone()
        } else {
            manifest.provider_context_root.join(&request.working_directory)
        };
        self.gateway
            .revalidate_before_spawn(&validated, &cwd, false)
            .map_err(|error| ProviderAdmissionError::Waiting {
                reason_code: "spawn_revalidation_drift".to_string(),
                detail: error.to_string(),
                missing_materials: missing_materials.clone(),
                allowed_actions: vec![BootstrapActionKind::Revalidate, BootstrapActionKind::Retry],
            })?;

        if !missing_materials.is_empty() {
            return Err(ProviderAdmissionError::Waiting {
                reason_code: "member_language_rules_missing".to_string(),
                detail: format!(
                    "logical codebase {} has members without .claude/rules/language.md",
                    self.lc_id
                ),
                missing_materials,
                allowed_actions,
            });
        }

        Ok(ProviderAdmissionPreflightResult {
            target: resolution.target,
            authority_root: resolution.authority_root,
            policy,
            rules,
            capability_snapshot_ref: validated.capability_snapshot_ref().to_string(),
            ready: true,
            missing_materials,
            allowed_actions: Vec::new(),
        })
    }
}

fn parse_member_id(value: &str) -> Option<LogicalRepositoryId> {
    uuid::Uuid::parse_str(value).ok().map(LogicalRepositoryId)
}

fn parse_checkout_id(value: &str) -> Option<RepositoryCheckoutId> {
    uuid::Uuid::parse_str(value).ok().map(RepositoryCheckoutId)
}
