#[cfg(test)]
mod phase_credential_tests {
    use super::*;
    use super::tests::*;
    use crate::product::logical_codebase::policy::SessionPolicyAction;
    use crate::product::logical_codebase::provider_gateway::ProviderRef;

    // ---- Task 1.2（REQ-BOOT-04/D1）：phase credential 与 BootstrapExecutor ----

    use crate::cross_cutting::streaming_provider::{
        ProviderToolPolicy, ToolPolicyGuardError, ToolPolicyIntent, canonical_tool_policy,
        validate_tool_policy_for_role,
    };
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateCancellationRecord, AggregateInitializationErrorRecord,
        AggregateInitializationOperation, AggregateInitializationOperationInput,
        AggregateInitializationStepKind,
    };
    use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
    use crate::protocol::contracts::AdapterRole;

    const BOOTSTRAP_TS: &str = "2026-10-01T00:01:00Z";

    /// 构造 durable Running operation：MachineSkills/AggregatePreflight 已
    /// 完成，目标 step（默认 PreCheck）运行中并带 input digest——这是
    /// `BootstrapPhaseCredential` 的唯一合法派生面。
    fn running_bootstrap_operation(
        fixture: &AdmissionFixture,
    ) -> (AggregateInitializationOperationStore, String) {
        running_bootstrap_operation_at_step(fixture, AggregateInitializationStepKind::PreCheck)
    }

    fn running_bootstrap_operation_at_step(
        fixture: &AdmissionFixture,
        step: AggregateInitializationStepKind,
    ) -> (AggregateInitializationOperationStore, String) {
        let store = AggregateInitializationOperationStore::for_lc(
            fixture.paths.clone(),
            fixture.lc_id.clone(),
        );
        let operation_id = format!("op-{}", uuid::Uuid::new_v4().simple());
        let input = AggregateInitializationOperationInput {
            idempotency_key: format!("bootstrap-{operation_id}"),
            manifest_revision: 1,
            policy_digest: "sha256:bootstrap-fixture-policy".to_string(),
            profile_evidence_digest: None,
            provider_context_root: std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
            provider: "claude_code".to_string(),
        };
        store
            .create_idempotent(AggregateInitializationOperation::new(
                operation_id.clone(),
                fixture.project_id.clone(),
                input,
                BOOTSTRAP_TS.to_string(),
            ))
            .expect("create bootstrap operation");
        store
            .mark_running(&fixture.project_id, &operation_id, BOOTSTRAP_TS.to_string())
            .expect("mark operation running");
        // 前置步骤按 V1 顺序完成，直到目标 step 可以运行。
        for predecessor in AggregateInitializationStepKind::V1 {
            if predecessor == step {
                break;
            }
            complete_bootstrap_step(&store, &fixture.project_id, &operation_id, predecessor);
        }
        store
            .mark_step_running(
                &fixture.project_id,
                &operation_id,
                step,
                format!("sha256:input-{}", step.as_str()),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("mark bootstrap step running");
        (store, operation_id)
    }

    fn complete_bootstrap_step(
        store: &AggregateInitializationOperationStore,
        project_id: &str,
        operation_id: &str,
        step: AggregateInitializationStepKind,
    ) {
        store
            .mark_step_running(
                project_id,
                operation_id,
                step,
                format!("sha256:input-{}", step.as_str()),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("mark predecessor running");
        store
            .checkpoint_step_output(
                project_id,
                operation_id,
                step,
                format!("artifact-{step:?}"),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("checkpoint predecessor");
        store
            .mark_step_completed(project_id, operation_id, step, BOOTSTRAP_TS.to_string())
            .expect("complete predecessor");
    }

    /// 完成剩余全部步骤并 `finish_completed`（Completed 终态夹具）。
    pub(super) fn finish_bootstrap_operation(
        store: &AggregateInitializationOperationStore,
        fixture: &AdmissionFixture,
        operation_id: &str,
    ) {
        let operation = store
            .get(&fixture.project_id, operation_id)
            .expect("load operation for completion");
        for (index, step) in AggregateInitializationStepKind::V1.into_iter().enumerate() {
            // 已 Completed 的步骤不可重复 mark_step_running（store 拒绝非
            // Pending 起始状态）；只推进尚未完成的步骤。
            if operation.steps[index].status
                == crate::product::logical_codebase::aggregate_initialization::AggregateInitializationStepStatus::Completed
            {
                continue;
            }
            complete_bootstrap_step(store, &fixture.project_id, operation_id, step);
        }
        store
            .finish_completed(&fixture.project_id, operation_id, BOOTSTRAP_TS.to_string())
            .expect("finish completed");
    }

    pub(super) fn derived_credential(
        fixture: &AdmissionFixture,
    ) -> (
        AggregateInitializationOperationStore,
        String,
        BootstrapPhaseCredential,
    ) {
        let (store, operation_id) = running_bootstrap_operation(fixture);
        let credential = BootstrapPhaseCredential::from_running_operation(
            &store,
            &fixture.project_id,
            &operation_id,
            AggregateInitializationStepKind::PreCheck,
            &fixture.lc_id,
            &std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
        )
        .expect("derive bootstrap credential from running operation");
        (store, operation_id, credential)
    }

    pub(super) fn assert_bootstrap_waiting<T: std::fmt::Debug>(
        result: Result<T, ProviderAdmissionError>,
        expected_reason: &str,
    ) {
        match result {
            Err(ProviderAdmissionError::Waiting { reason_code, .. }) => {
                assert_eq!(reason_code, expected_reason);
            }
            other => panic!("expected waiting {expected_reason}, got {other:?}"),
        }
    }

    #[test]
    fn bootstrap_phase_credential_rejects_completed_or_drifted_operation() {
        let fixture = admission_fixture();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();

        // 基线：Running + step 运行中 + digest/root/LC 匹配 → 唯一可派生面。
        let (_store, _operation_id, credential) = derived_credential(&fixture);
        assert_eq!(credential.step, AggregateInitializationStepKind::PreCheck);

        // Completed 终态 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        finish_bootstrap_operation(&store, &fixture, &operation_id);
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_operation_not_running",
        );

        // Failed 终态 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        store
            .finish_failed(
                &fixture.project_id,
                &operation_id,
                Some(AggregateInitializationStepKind::PreCheck),
                AggregateInitializationErrorRecord::interrupted(),
                BOOTSTRAP_TS.to_string(),
            )
            .expect("finish failed");
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_operation_not_running",
        );

        // Cancelled 终态 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        store
            .cancel(
                &fixture.project_id,
                &operation_id,
                AggregateCancellationRecord {
                    reason_code: "test_cancelled".to_string(),
                    cancelled_at: BOOTSTRAP_TS.to_string(),
                    detail: None,
                },
                BOOTSTRAP_TS.to_string(),
            )
            .expect("cancel operation");
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_operation_not_running",
        );

        // step 漂移：durable 运行中的是 RuleAndMcpConfig，凭据声明 PreCheck → 拒绝。
        let (store, operation_id) = running_bootstrap_operation_at_step(
            &fixture,
            AggregateInitializationStepKind::RuleAndMcpConfig,
        );
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_step_not_running",
        );

        // 非 provider turn step（MachineSkills 运行中）→ 拒绝：确定性步骤
        // 不得派生 spawn 凭据。
        let (store, operation_id) = running_bootstrap_operation_at_step(
            &fixture,
            AggregateInitializationStepKind::MachineSkills,
        );
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::MachineSkills,
                &fixture.lc_id,
                &canonical_root,
            ),
            "bootstrap_step_not_provider_turn",
        );

        // root 漂移：凭据 root 与 operation 的 provider_context_root 不一致 → 拒绝。
        let (store, operation_id) = running_bootstrap_operation(&fixture);
        let member_root = std::fs::canonicalize(&fixture.member_root).unwrap();
        assert_bootstrap_waiting(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                &operation_id,
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &member_root,
            ),
            "bootstrap_root_drift",
        );

        // input digest 漂移：已派生凭据与当前 durable 记录 digest 不一致 →
        // spawn 前重核验拒绝（fail-closed，不可复用旧凭据）。
        let (store, _operation_id, credential) = derived_credential(&fixture);
        let drifted_digest = BootstrapPhaseCredential {
            input_digest: "sha256:drifted-input".to_string(),
            ..credential.clone()
        };
        assert_bootstrap_waiting(
            drifted_digest.reverify_against_running_operation(&store, &fixture.lc_id),
            "bootstrap_input_digest_drift",
        );

        // LC 漂移：凭据 LC 身份与核验 scope 不一致 → 拒绝。
        let drifted_lc = BootstrapPhaseCredential {
            logical_codebase_id: uuid::Uuid::new_v4().to_string(),
            ..credential
        };
        assert_bootstrap_waiting(
            drifted_lc.reverify_against_running_operation(&store, &fixture.lc_id),
            "bootstrap_logical_codebase_mismatch",
        );

        // 伪造 operation id → durable 缺失走 Store 错误（fail-closed）。
        assert!(matches!(
            BootstrapPhaseCredential::from_running_operation(
                &store,
                &fixture.project_id,
                "op-missing",
                AggregateInitializationStepKind::PreCheck,
                &fixture.lc_id,
                &canonical_root,
            ),
            Err(ProviderAdmissionError::Store(_))
        ));
    }

    /// 路B 主断言(fix-admission-member-rules-scope,先红;场景身份显式替换
    /// lc-root-initialization 旧 Scenario「bootstrap phase 只豁免根规则」):
    /// 有效 bootstrap credential 解除裸仓首建死锁——裸仓(recipe 首建:根
    /// 规则尚未生成、全部成员缺 language.md、最终 receipt 不存在,政策为
    /// 自举桩)持有由 durable Running operation 派生且重验有效的凭据时,
    /// 成员规则门与根规则存在性一并无豁免空间地按既有自举边界进入
    /// (fresh-LC 预期),policy/capability/gateway/cwd 照常必检。
    #[test]
    fn bootstrap_credential_releases_bare_repo_cold_start_deadlock() {
        // 裸仓:默认 fixture(placeholder 政策),不写根规则、不写成员规则、
        // 不发布 receipt——这些正是 root recipe 要生成的。
        let fixture = admission_fixture();
        let (_store, _operation_id, credential) = derived_credential(&fixture);

        let result = fixture
            .preflight()
            .check(
                &fixture.launch_request(),
                &ProviderAdmissionPhase::AggregateBootstrap(credential),
            )
            .expect("valid bootstrap credential must waive the bare-repo member rules gate");
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        // 豁免是审计而非静默:成员缺失与根规则缺失都保留非阻断事实。
        assert_eq!(result.missing_member_rules.len(), 1);
        assert_eq!(result.missing_root_rules.len(), 1);
        assert!(result.missing_materials.is_empty());
        assert!(result.allowed_actions.is_empty());
        assert_eq!(result.rules.len(), 1);
        assert!(result.rules[0].digest.is_none());
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // 对照(旧行为保留):成员规则在场时 bootstrap 相位同样 ready,根
        // 规则缺失仅记录事实。
        let fixture_rules = admission_fixture();
        fixture_rules.write_language_rules("# language\n");
        let (_store_rules, _op_rules, credential_rules) = derived_credential(&fixture_rules);
        let result_rules = fixture_rules
            .preflight()
            .check(
                &fixture_rules.launch_request(),
                &ProviderAdmissionPhase::AggregateBootstrap(credential_rules),
            )
            .expect("bootstrap phase must waive missing root rules");
        assert!(result_rules.ready);
        assert_eq!(result_rules.missing_root_rules.len(), 1);
        assert!(result_rules.missing_member_rules.is_empty());
        assert!(result_rules.missing_materials.is_empty());

        // Normal 对照组(自举桩政策,存量迁移前):成员规则在场时根规则缺失
        // 不阻断——根规则门只在 #8 已发布真实政策后生效(旧语义零回归);
        // 裸仓(成员缺失)的 Normal 请求仍被成员门拦(placeholder 不是
        // 「根材料有效」证明,路A 边界)。
        assert!(
            fixture_rules
                .preflight()
                .check(
                    &fixture_rules.launch_request(),
                    &ProviderAdmissionPhase::Normal
                )
                .expect("placeholder policy keeps legacy normal admission")
                .ready
        );

        // Task 3a:#8 已发布真实政策(非桩 artifact + receipt + locator)后,
        // Normal 相位根规则缺失必须阻断(root_rules_missing)。
        let fixture_published = admission_fixture();
        fixture_published.write_language_rules("# language\n");
        publish_root_policy_with_receipt(&fixture_published, "op_waive_published");
        assert_bootstrap_waiting(
            fixture_published
                .preflight()
                .check(
                    &fixture_published.launch_request(),
                    &ProviderAdmissionPhase::Normal,
                )
                .map(|_| ()),
            "root_rules_missing",
        );
        assert_eq!(fixture_published.streaming_adapter.start_count(), 0);

        // (supersede)旧「成员规则缺失任何相位阻断」条款已被本 change 取代:
        // bootstrap 相位凭据豁免见上;gateway 相位分流见
        // bootstrap_phase_gateway_uses_root_recipe_channel_not_normal_matrix。

        // 豁免不覆盖 policy：aggregate policy artifact 缺失仍 waiting。
        let fixture_no_policy = admission_fixture_without_policy();
        let (_store_no_policy, _op_no_policy, credential_no_policy) =
            derived_credential(&fixture_no_policy);
        assert_bootstrap_waiting(
            fixture_no_policy
                .preflight()
                .check(
                    &fixture_no_policy.launch_request(),
                    &ProviderAdmissionPhase::AggregateBootstrap(credential_no_policy),
                )
                .map(|_| ()),
            "aggregate_policy_artifact_missing",
        );

        // 豁免不覆盖 capability：gateway capability 谓词拒绝仍 waiting。
        let fixture_denied = admission_fixture();
        fixture_denied.capabilities.deny();
        let (_store_denied, _op_denied, credential_denied) = derived_credential(&fixture_denied);
        assert_bootstrap_waiting(
            fixture_denied
                .preflight()
                .check(
                    &fixture_denied.launch_request(),
                    &ProviderAdmissionPhase::AggregateBootstrap(credential_denied),
                )
                .map(|_| ()),
            "provider_capability_not_satisfied",
        );

        // 凭据失效优先于豁免：operation Completed 后，凭据重核验失败——
        // 绝不降级为普通 session 或带病放行。
        let fixture_stale = admission_fixture();
        let (store_stale, _op_stale, stale_credential) = derived_credential(&fixture_stale);
        finish_bootstrap_operation(
            &store_stale,
            &fixture_stale,
            stale_credential.operation_id(),
        );
        assert_bootstrap_waiting(
            fixture_stale
                .preflight()
                .check(
                    &fixture_stale.launch_request(),
                    &ProviderAdmissionPhase::AggregateBootstrap(stale_credential),
                )
                .map(|_| ()),
            "bootstrap_operation_not_running",
        );
        assert_eq!(fixture_stale.streaming_adapter.start_count(), 0);
    }

    #[test]
    fn bootstrap_executor_marker_rejects_ordinary_executor_policy_in_lc_root_recipe() {
        let fixture = admission_fixture();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let (_store, _operation_id, credential) = derived_credential(&fixture);
        let marker = BootstrapExecutorMarker::new(
            credential,
            SessionPolicyAction::CodingTargetWrite,
            canonical_root,
            "root-recipe:pre_check:command-1",
        )
        .expect("complete bootstrap executor marker");
        let marker_policy = ProviderToolPolicy {
            intent: ToolPolicyIntent::BootstrapExecutorMarker(marker),
        };

        // LC root recipe 的自举通道：Executor + 完整 marker 是唯一放行形态。
        assert!(
            validate_tool_policy_for_role(&AdapterRole::Executor, Some(&marker_policy)).is_ok()
        );

        // 普通 Executor 策略（deny）仍被拒绝——marker 通道不是给普通
        // Executor/Coder 带策略的豁免口。
        let deny = ProviderToolPolicy::deny_file_write_builtins();
        assert!(validate_tool_policy_for_role(&AdapterRole::Executor, Some(&deny)).is_err());

        // 策略角色携带 marker：marker 不是策略（PolicyRequired）。
        for role in [
            AdapterRole::Orchestrator,
            AdapterRole::Reviewer,
            AdapterRole::WorkItemSplitter,
        ] {
            let rejected = matches!(
                validate_tool_policy_for_role(&role, Some(&marker_policy)),
                Err(ToolPolicyGuardError::PolicyRequired { .. })
            );
            assert!(
                rejected,
                "policy role {role:?} must not substitute the marker for its policy"
            );
        }

        // Handoff 不得使用自举通道。
        assert!(matches!(
            validate_tool_policy_for_role(&AdapterRole::Handoff, Some(&marker_policy)),
            Err(ToolPolicyGuardError::PolicyForbidden { .. })
        ));

        // marker 不投影为 canonical deny 策略：不进入策略会话 argv/审计通道。
        assert!(canonical_tool_policy("claude-code", &marker_policy).is_err());
        // 普通 Executor/Coder 无策略照常放行（既有双向语义零变化）。
        assert!(validate_tool_policy_for_role(&AdapterRole::Executor, None).is_ok());
    }

    #[test]
    fn bootstrap_executor_without_credential_or_receipt_context_is_rejected() {
        let fixture = admission_fixture();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let (_store, _operation_id, credential) = derived_credential(&fixture);

        // 凭据是 marker 的必带字段（类型面）：不存在「无凭据 marker」的
        // 构造形态——`BootstrapExecutorMarker::new` 只接受由 durable
        // Running operation 派生的 credential。
        // receipt context 缺失（空白）→ 构造即拒绝。
        assert_eq!(
            BootstrapExecutorMarker::new(
                credential.clone(),
                SessionPolicyAction::CodingTargetWrite,
                canonical_root.clone(),
                "   ",
            ),
            Err(BootstrapExecutorMarkerError::EmptyReceiptContext)
        );

        // canonical root 缺失 → 构造即拒绝。
        assert_eq!(
            BootstrapExecutorMarker::new(
                credential.clone(),
                SessionPolicyAction::CodingTargetWrite,
                std::path::PathBuf::new(),
                "root-recipe:pre_check:command-1",
            ),
            Err(BootstrapExecutorMarkerError::EmptyCanonicalRoot)
        );

        // 非写权限 action → 拒绝：BootstrapExecutor = 有写权限的 Executor。
        assert_eq!(
            BootstrapExecutorMarker::new(
                credential.clone(),
                SessionPolicyAction::PlanningReadOnly,
                canonical_root.clone(),
                "root-recipe:pre_check:command-1",
            ),
            Err(BootstrapExecutorMarkerError::InvalidAction(
                SessionPolicyAction::PlanningReadOnly
            ))
        );

        // spawn 前守卫对结构不完整 marker 再次拒绝（纵深防御：即使绕过
        // validating constructor，三 adapter 真实子进程前仍 fail-closed）。
        let complete = BootstrapExecutorMarker::new(
            credential,
            SessionPolicyAction::CodingTargetWrite,
            canonical_root,
            "root-recipe:pre_check:command-1",
        )
        .expect("complete marker");
        let degenerate = BootstrapExecutorMarker {
            receipt_context: String::new(),
            ..complete
        };
        let degenerate_policy = ProviderToolPolicy {
            intent: ToolPolicyIntent::BootstrapExecutorMarker(degenerate),
        };
        assert!(matches!(
            validate_tool_policy_for_role(&AdapterRole::Executor, Some(&degenerate_policy)),
            Err(ToolPolicyGuardError::BootstrapMarkerInvalid { .. })
        ));
    }

    /// 路A(fix-admission-member-rules-scope,名实更正):普通已发布政策 +
    /// canonical root 自身规则缺失 → root_rules_missing 停等、provider 零
    /// 启动——成员豁免不是根豁免,根材料缺失+成员缺失并存仍 fail-closed。
    /// 形态保持 cwd≠target 分离(root cwd + 成员 Git checkout target)与
    /// root 内自引用 symlink alias cwd,钉住 admission 复验判据是 canonical
    /// 相等而非字面相等。补齐根规则(language.md)后同一分离形态请求
    /// ready 且成员缺失仅审计,证明分离形态本身不是阻断源。
    #[test]
    fn ordinary_published_root_missing_zero_spawns() {
        let fixture = admission_fixture();
        // 已发布政策 + 匹配最终 receipt,但 canonical root 自身的
        // .claude/rules/language.md 缺失(publish_root_policy_with_receipt
        // 不建根 language);成员规则同样缺失。
        publish_root_policy_with_receipt(&fixture, "op_ordinary_root_missing");
        // cwd=root 的自引用 symlink alias（词法上位于 authority 内，canonical
        // 解析回 root 本体）。
        let cwd_alias = fixture.aggregate_root.join("root-alias");
        std::os::unix::fs::symlink(&fixture.aggregate_root, &cwd_alias).unwrap();
        // 成员身份从 LC 子树权威 store 读取（fixture 未保留 id 字段）。
        let lc = LogicalCodebaseStore::for_lc(fixture.paths.clone(), &fixture.lc_id);
        let members = lc.list_members(&fixture.project_id).unwrap();
        let member = members.first().expect("fixture seeds one member");
        let checkouts = lc.list_checkouts(&fixture.project_id).unwrap();
        let checkout = checkouts
            .iter()
            .find(|checkout| member.checkout_ids.contains(&checkout.checkout_id))
            .expect("member has a recorded checkout");
        let canonical_member = std::fs::canonicalize(&fixture.member_root).unwrap();

        let separated_request = SessionLaunchRequest {
            project_id: fixture.project_id.clone(),
            provider: ProviderRef::claude_code("snapshot-admission-test"),
            action: SessionPolicyAction::PlanningReadOnly,
            target: crate::product::logical_codebase::policy::PolicyTarget::checkout(
                member.logical_repository_id.0.to_string(),
                checkout.checkout_id.0.to_string(),
                canonical_member.clone(),
            ),
            working_directory: cwd_alias.clone(),
            readable_roots: vec![fixture.aggregate_root.clone()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:admission-managed-config".to_string(),
        };

        // 普通相位 + 已发布政策 + 根规则缺失(+成员规则缺失) → waiting
        // (root_rules_missing),provider 零启动。等待判据优先钉根规则缺失
        // (若 cwd alias 复验失败会以 spawn_revalidation_drift 出现,
        // reason_code 断言可捕捉)。
        let error = fixture
            .preflight()
            .check(&separated_request, &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting { reason_code, .. } => {
                assert_eq!(reason_code, "root_rules_missing");
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // 对照：补齐根规则(language.md)后同一分离形态请求 ready——分离形态
        // (root cwd ≠ member target、symlink alias cwd)不是阻断源;此时成员
        // 规则缺失仅保留非阻断审计;admission 本身仍零启动。
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        std::fs::create_dir_all(canonical_root.join(".claude/rules")).unwrap();
        std::fs::write(
            canonical_root.join(".claude/rules/language.md"),
            "# 语言规则\n\n- ordinary 对照:根规则正文。\n",
        )
        .unwrap();
        let result = fixture
            .preflight()
            .check(&separated_request, &ProviderAdmissionPhase::Normal)
            .expect("separated form with root rules present must be ready");
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        assert_eq!(result.missing_member_rules.len(), 1);
        assert!(result.missing_materials.is_empty());
        assert!(result.allowed_actions.is_empty());
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

}
