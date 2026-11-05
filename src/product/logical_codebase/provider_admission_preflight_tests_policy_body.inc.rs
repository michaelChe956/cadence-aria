#[cfg(test)]
mod policy_body_tests {
    use super::*;
    use super::tests::*;
    use super::phase_credential_tests::{
        assert_bootstrap_waiting, derived_credential, finish_bootstrap_operation,
    };
    use crate::cross_cutting::provider_registry::ProviderRegistry;
    use crate::product::logical_codebase::policy::{ProviderDialect, SessionPolicyAction};
    use crate::product::logical_codebase::provider_gateway::{
        ProviderCapabilitySource, ProviderRef, ProviderRefType,
    };

    // ===== Task 3a(lcg_t03):canonical 政策正文消费与成员规则门 =====

    /// 递归收集 root 下全部相对路径(排序后比较,证明 waiting 前后零物化)。
    fn sorted_tree_files(root: &Path) -> Vec<String> {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
            let entries = std::fs::read_dir(dir).expect("read dir for tree snapshot");
            for entry in entries {
                let path = entry.expect("dir entry").path();
                let relative = path
                    .strip_prefix(root)
                    .expect("path under snapshot root")
                    .to_string_lossy()
                    .to_string();
                if path.is_dir() {
                    out.push(format!("{relative}/"));
                    walk(root, &path, out);
                } else {
                    out.push(relative);
                }
            }
        }
        let mut files = Vec::new();
        walk(root, root, &mut files);
        files.sort();
        files
    }

    /// 给合成 artifact 补一份「流水线发布记录」,让它参与新鲜度判定。
    fn write_publication_record(
        fixture: &AdmissionFixture,
        operation_id: &str,
        artifact: &crate::product::logical_codebase::policy::AggregatePolicyArtifact,
    ) {
        let scope = crate::product::logical_codebase::store::lc_scope_root(
            &fixture.paths,
            &fixture.project_id,
            &Some(fixture.lc_id.clone()),
        )
        .expect("scope root");
        let dir = scope.join("aggregate-initializations").join(operation_id);
        std::fs::create_dir_all(&dir).unwrap();
        let output = crate::product::logical_codebase::policy::AggregatePolicyPublicationOutput {
            project_id: fixture.project_id.clone(),
            lc_id: fixture.lc_id.clone(),
            logical_codebase_id: artifact.logical_codebase_id.clone(),
            operation_id: operation_id.to_string(),
            canonical_root: std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
            base_policy: crate::product::logical_codebase::policy::AggregatePolicyPublicationReference {
                policy_id: String::new(),
                revision: 0,
                digest: String::new(),
            },
            artifact: artifact.clone(),
            sources: Vec::new(),
            rule_digest: String::new(),
        };
        crate::product::json_store::write_json(&dir.join("policy-publication.json"), &output)
            .expect("write publication record");
    }

    /// owner 2026-10-26：规则源变更后，准入必须拒绝用旧政策启动——
    /// 自动重发不成则停等 `provider_policy_stale`（动作只有 republish）。
    #[test]
    fn stale_pipeline_policy_blocks_admission_until_republished() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# language\n");
        let artifact = publish_root_policy_with_receipt(&fixture, "op_stale_gate");
        write_publication_record(&fixture, "op_stale_gate", &artifact);

        // 规则源新增一行 → 已发布 artifact 落后于当前源。
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        std::fs::create_dir_all(canonical_root.join(".claude/rules")).unwrap();
        std::fs::write(
            canonical_root.join(".claude/rules/language.md"),
            "# 语言规则\n\n- 变更后内容。\n",
        )
        .unwrap();

        let error = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .expect_err("stale policy must not be consumed");
        match error {
            ProviderAdmissionError::Waiting {
                reason_code,
                allowed_actions,
                ..
            } => {
                assert_eq!(reason_code, "provider_policy_stale");
                assert_eq!(
                    allowed_actions,
                    vec![BootstrapActionKind::Republish],
                    "只允许重新生成政策这一个动作"
                );
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        assert_eq!(fixture.streaming_adapter.start_count(), 0, "provider 零启动");
    }

    /// 消费侧观察面:admission 判定状态与 provider 启动计数(测试 6 断言组)。
    struct ProviderSpawnObservation {
        status: &'static str,
        provider_spawn_count: usize,
    }

    /// Task 3a(lcg_t03):#8 已发布过真实政策(store 内非桩 artifact)但
    /// canonical root 上 locator 正文/最终 receipt 缺失时,Normal admission
    /// 停等为可操作 waiting(Revalidate/Retry),不物化任何 root 文件。
    #[test]
    fn lcg_t03_root_policy_body_missing_waits_without_materialization() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# language\n");
        let current = fixture
            .policy_store
            .get(&fixture.project_id)
            .unwrap()
            .unwrap();
        let revised = current.with_revised_policy(
            "# 聚合政策\n\npublished but not materialized\n",
            "2026-10-03T00:00:00Z".to_string(),
        );
        fixture
            .policy_store
            .save(&fixture.project_id, &revised)
            .unwrap();

        let root_files_before = sorted_tree_files(&fixture.aggregate_root);
        let waiting = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        assert!(
            matches!(&waiting, ProviderAdmissionError::Waiting { reason_code, .. } if reason_code == "provider_policy_artifact_missing")
        );
        let root_files_after = sorted_tree_files(&fixture.aggregate_root);
        assert_eq!(root_files_before, root_files_after);
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
    }

    /// Task 3a(lcg_t03):已发布政策正文的 locator 被 symlink 调包、正文
    /// 字节漂移或 AGENTS.md rule digest 漂移时,admission 停等
    /// `provider_policy_artifact_missing`,绝不消费被调包正文。
    #[test]
    fn lcg_t03_policy_locator_rejects_symlink_and_digest_drift() {
        // (i) locator 被 symlink 调包(即使字节完全一致):canonical 无
        //     symlink 逃逸,admission 拒绝消费。
        let fixture = admission_fixture();
        fixture.write_language_rules("# language\n");
        let artifact = publish_root_policy_with_receipt(&fixture, "op_lcg_t03_locator");
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let locator = canonical_root.join(&artifact.policy_id);
        let outside = fixture._temp.path().join("outside-copy");
        std::fs::write(&outside, artifact.policy_text.as_bytes()).unwrap();
        std::fs::remove_file(&locator).unwrap();
        std::os::unix::fs::symlink(&outside, &locator).unwrap();
        let error = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        assert!(
            matches!(&error, ProviderAdmissionError::Waiting { reason_code, .. } if reason_code == "provider_policy_artifact_missing"),
            "symlink locator must not be consumed, got {error:?}"
        );
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // (ii) locator 正文字节漂移(digest 漂移):拒绝消费。
        let fixture = admission_fixture();
        fixture.write_language_rules("# language\n");
        let artifact = publish_root_policy_with_receipt(&fixture, "op_lcg_t03_drift");
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let locator = canonical_root.join(&artifact.policy_id);
        std::fs::write(&locator, b"# tampered policy body\n").unwrap();
        let error = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        assert!(
            matches!(&error, ProviderAdmissionError::Waiting { reason_code, .. } if reason_code == "provider_policy_artifact_missing"),
            "digest drift must not be consumed, got {error:?}"
        );

        // (iii) 本夹具的 artifact 是合成产物（无流水线发布记录）。新契约下
        //       「是否落后于当前规则」只对流水线产出的 artifact 判定，故规则源
        //       变更后仍按 artifact 自洽性放行。生产 artifact 均有发布记录，
        //       其新鲜度由准入 3a2 分支（自动重发 / provider_policy_stale）负责。
        let fixture = admission_fixture();
        fixture.write_language_rules("# language\n");
        publish_root_policy_with_receipt(&fixture, "op_lcg_t03_rule");
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        std::fs::write(
            canonical_root.join("AGENTS.md"),
            b"# AGENTS rules\nmutated rule bytes\n",
        )
        .unwrap();
        // 合成 artifact 不做新鲜度判定 → 继续走到材料门（根规则引用被改写）。
        let error = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        assert!(
            matches!(&error, ProviderAdmissionError::Waiting { reason_code, .. } if reason_code == "root_rules_missing"),
            "synthetic artifact: rule materials gate must decide, got {error:?}"
        );
    }

    /// 路B(fix-admission-member-rules-scope,先红;改写 lcg_t03 成员门
    /// 「任何相位阻断」旧断言):bootstrap 相位的 gateway 校验按已重验
    /// phase 分流——AggregateBootstrap 走既有 `validate_root_recipe_request`
    /// (固定 Claude recipe 事实+凭据 durable 重验),合法 recipe 不先被
    /// Normal capability 矩阵拒绝;同一 gateway 的普通会话仍按 Normal
    /// 矩阵拒绝(不伪造 Confirmed、不借 recipe 证据搭便车)。
    #[test]
    fn bootstrap_phase_gateway_uses_root_recipe_channel_not_normal_matrix() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# language\n");
        let (_store, _operation_id, credential) = derived_credential(&fixture);

        // Normal 矩阵整体拒绝(require_supported/resume/write_boundary),
        // RootRecipe 通道照常放行——分流判据是 phase,不是 capability source
        // 的全局开关。
        fixture.capabilities.deny_normal_matrix();

        let result = fixture
            .preflight()
            .check(
                &fixture.launch_request(),
                &ProviderAdmissionPhase::AggregateBootstrap(credential),
            )
            .expect("bootstrap phase must dispatch to the root-recipe gateway channel");
        assert!(result.ready, "missing: {:?}", result.missing_materials);
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // 同一 gateway、同一 capability source 的普通(Normal)请求仍被
        // Normal 矩阵拒绝:recipe 证据不给普通会话。
        let ordinary = fixture
            .preflight()
            .check(&fixture.launch_request(), &ProviderAdmissionPhase::Normal);
        let observation = ProviderSpawnObservation {
            status: match &ordinary {
                Err(ProviderAdmissionError::Waiting { .. }) => "waiting",
                Ok(_) => "ready",
                Err(ProviderAdmissionError::Store(_)) => "store_error",
            },
            provider_spawn_count: fixture.streaming_adapter.start_count(),
        };
        assert_eq!(observation.status, "waiting");
        assert_eq!(observation.provider_spawn_count, 0);
        assert!(
            matches!(&ordinary, Err(ProviderAdmissionError::Waiting { reason_code, .. }) if reason_code == "provider_capability_not_satisfied"),
            "ordinary request must keep the Normal matrix gate, got {ordinary:?}"
        );
    }

    // ===== Task 3b(lcg_t03):early 资格面与 spawn 前 D4 门 =====

    /// 文件背书的只读 trust source:判定前后快照同一 artifact 字节,
    /// 证明 gateway 判定路径对用户级 trust 工件零写入。
    struct FileBackedTrustSource {
        artifact: PathBuf,
    }

    impl crate::product::logical_codebase::provider_trust::ProviderTrustSource
        for FileBackedTrustSource
    {
        fn verify_trusted(
            &self,
            _project_id: &str,
            _lc_id: &str,
            provider: &crate::product::models::ProviderName,
            canonical_root: &Path,
        ) -> Result<
            crate::product::logical_codebase::provider_trust::ProviderTrustVerification,
            ProviderGatewayError,
        > {
            let bytes = std::fs::read(&self.artifact).map_err(|error| {
                ProviderGatewayError::ProviderUnavailable(format!(
                    "read trust artifact {}: {error}",
                    self.artifact.display()
                ))
            })?;
            if bytes != b"trusted\n" {
                return Err(ProviderGatewayError::ProviderUnavailable(
                    "trust artifact no longer trusted".to_string(),
                ));
            }
            Ok(
                crate::product::logical_codebase::provider_trust::ProviderTrustVerification {
                    provider: provider.clone(),
                    canonical_root: canonical_root.to_path_buf(),
                    trust_key: "file-backed-fixture".to_string(),
                    trusted: true,
                    ownership: Some(
                        crate::product::logical_codebase::provider_trust::ProviderTrustOwnership::LcManaged,
                    ),
                    detail: "fixture trusted".to_string(),
                    verified_at: "2026-10-03T00:00:00Z".to_string(),
                },
            )
        }
    }

    /// Task 3b(lcg_t03):early 资格判定只消费合法 capability 证据,
    /// 不需要未来 target/worktree 或 D4;判定全程零写入(policy/capability/
    /// trust 的 durable 字节前后不变);实际 spawn 缺 D4(write_boundary
    /// 分格未 Confirmed)在 validate 即拒绝,provider 零启动。
    #[test]
    fn lcg_t03_early_eligibility_needs_no_future_d4_and_writes_nothing() {
        let fixture = admission_fixture();
        fixture.write_language_rules("# language\n");
        // 用户级 trust 工件在场且 trusted;判定前后字节必须不变。
        let trust_artifact = fixture._temp.path().join("trust-home").join("config.toml");
        std::fs::create_dir_all(trust_artifact.parent().unwrap()).unwrap();
        std::fs::write(&trust_artifact, b"trusted\n").unwrap();
        // early 场景:launch 分格 Confirmed,write_boundary(D4)缺证据。
        fixture.capabilities.deny_write_boundary();

        let gateway = LogicalCodebaseProviderGateway::with_audit(
            fixture.policy_store.clone(),
            fixture.capabilities.clone(),
            Arc::new(PassThroughTargetResolver),
            {
                let mut registry = ProviderRegistry::new();
                registry.register(
                    crate::product::models::ProviderName::ClaudeCode,
                    fixture.streaming_adapter.clone(),
                );
                Arc::new(registry)
            },
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            Arc::new(crate::product::logical_codebase::GatewayRunAudit::new()),
            std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
        )
        .with_readonly_lc_facts(
            crate::product::logical_codebase::RootRecipeReceiptStore::for_lc(
                fixture.paths.clone(),
                &fixture.lc_id,
            ),
            Arc::new(FileBackedTrustSource {
                artifact: trust_artifact.clone(),
            }),
            Some(fixture.lc_id.clone()),
        );

        // attempt 尚未创建(无 worktree/target/D4 事实):early 判定只看
        // capability 证据,返回不携未来 target/worktree 或 D4 的两枚引用。
        let aria_before = sorted_tree_files(fixture.paths.root());
        let trust_before = std::fs::read(&trust_artifact).unwrap();
        let early = gateway
            .action_admission_verdict(
                &ProviderRef::claude_code("snapshot-admission-test"),
                SessionPolicyAction::CodingTargetWrite,
                &crate::protocol::contracts::AdapterRole::Executor,
                crate::cross_cutting::streaming_provider::ProviderPermissionMode::Auto,
            )
            .expect("early eligibility must not require future D4 facts");
        assert_eq!(early.capability_snapshot_ref, "snapshot-admission-test");
        assert_eq!(early.projection_ref, "projection-digest-CodingTargetWrite");
        let aria_after = sorted_tree_files(fixture.paths.root());
        assert_eq!(aria_before, aria_after);
        assert_eq!(std::fs::read(&trust_artifact).unwrap(), trust_before);
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // 实际 spawn 缺 D4:同一 provider/action 的 coding validate 在
        // write_boundary 分格门 fail-closed,provider 零启动。
        let lc = LogicalCodebaseStore::for_lc(fixture.paths.clone(), &fixture.lc_id);
        let members = lc.list_members(&fixture.project_id).unwrap();
        let member = members.first().expect("fixture seeds one member");
        let checkouts = lc.list_checkouts(&fixture.project_id).unwrap();
        let checkout = checkouts
            .iter()
            .find(|checkout| member.checkout_ids.contains(&checkout.checkout_id))
            .expect("member has a recorded checkout");
        let canonical_member = std::fs::canonicalize(&fixture.member_root).unwrap();
        let canonical_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let coding = SessionLaunchRequest {
            project_id: fixture.project_id.clone(),
            provider: ProviderRef::claude_code("snapshot-admission-test"),
            action: SessionPolicyAction::CodingTargetWrite,
            target: crate::product::logical_codebase::policy::PolicyTarget::checkout(
                member.logical_repository_id.0.to_string(),
                checkout.checkout_id.0.to_string(),
                canonical_member.clone(),
            ),
            working_directory: canonical_root,
            readable_roots: vec![fixture.aggregate_root.clone()],
            writable_roots: vec![canonical_member],
            config_artifact_ref: "sha256:admission-managed-config".to_string(),
        };
        let error = gateway.validate(coding).unwrap_err();
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason) if reason.contains("write_boundary")),
            "spawn without D4 evidence must fail closed, got {error:?}"
        );
        assert_eq!(fixture.streaming_adapter.start_count(), 0);
        assert_eq!(std::fs::read(&trust_artifact).unwrap(), trust_before);
    }

    // ===== Task 2b(lcg_t02):unknown normal 矩阵下既有 root recipe 契约保持 =====

    use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence as T02Evidence;
    use crate::product::logical_codebase::policy::ProviderWireDialect as T02WireDialect;
    use crate::product::logical_codebase::production_policy_resolvers::StoreBackedProviderCapabilitySource as T02Source;
    use crate::product::logical_codebase::provider_capability_store::{
        CapabilityEvidence as T02Provenance,
        PROVIDER_CAPABILITY_RECORD_SCHEMA_VERSION as T02SchemaVersion,
        ProviderActionCapability as T02Row, ProviderActionMatrix as T02Matrix,
        ProviderCapabilityRecord as T02Record, ProviderCapabilityStore as T02Store,
        RootRecipeEvidence as T02RecipeEvidence,
    };

    /// root recipe 契约记录:v2 三态矩阵(Planning: Confirmed/Denied/Confirmed;
    /// Coding: Confirmed/Unknown/Denied{root write probe rejected}; Review: 全
    /// Unknown)+ 已交付 recipe 证据;legacy allow 列表仍列出全部 action——
    /// 证明旧列表与 recipe 证据都无法为 Denied 负向分格放行正常会话。
    fn t02_recipe_contract_record() -> T02Record {
        let matrix = T02Matrix::from_rows(vec![
            T02Row {
                action: SessionPolicyAction::PlanningReadOnly,
                launch: T02Evidence::Confirmed,
                resume: T02Evidence::Denied {
                    reason: "boundary probe denied".to_string(),
                },
                write_boundary: T02Evidence::Confirmed,
                projection_digest: "projection-digest-PlanningReadOnly".to_string(),
                evidence_ref: "probe://PlanningReadOnly".to_string(),
            },
            T02Row {
                action: SessionPolicyAction::CodingTargetWrite,
                launch: T02Evidence::Confirmed,
                resume: T02Evidence::Unknown,
                write_boundary: T02Evidence::Denied {
                    reason: "root write probe rejected".to_string(),
                },
                projection_digest: "projection-digest-CodingTargetWrite".to_string(),
                evidence_ref: "probe://CodingTargetWrite".to_string(),
            },
            T02Row {
                action: SessionPolicyAction::ReviewReadOnly,
                launch: T02Evidence::Unknown,
                resume: T02Evidence::Unknown,
                write_boundary: T02Evidence::Unknown,
                projection_digest: "projection-digest-ReviewReadOnly".to_string(),
                evidence_ref: "probe://ReviewReadOnly".to_string(),
            },
        ])
        .unwrap();
        T02Record {
            provider_type: ProviderRefType::ClaudeCode,
            schema_version: T02SchemaVersion,
            version: "1.4.0".to_string(),
            adapter_dialect: ProviderDialect::ClaudeCodeCliV1,
            wire_dialect: T02WireDialect::ClaudeCodeStreamJson,
            capability_snapshot_ref: "cap_managed_snapshot".to_string(),
            evidence: T02Provenance::ProductionVerified,
            resume_evidence:
                crate::product::logical_codebase::provider_gateway::ResumeEvidenceState::Confirmed,
            supported_actions: vec![
                SessionPolicyAction::PlanningReadOnly,
                SessionPolicyAction::CodingTargetWrite,
                SessionPolicyAction::ReviewReadOnly,
            ],
            action_matrix: matrix,
            trust: T02Evidence::Confirmed,
            probed_at: Some("2026-10-03T00:00:00Z".to_string()),
            probe_artifact_ref: Some("probe://artifact-0001".to_string()),
            root_recipe_evidence: T02RecipeEvidence::Delivered {
                evidence_ref: "recipe://evidence-0001".to_string(),
                version: "1.4.0".to_string(),
            },
            execution_semantics:
                crate::cross_cutting::provider_boundary::PROVIDER_EXECUTION_SEMANTICS_HOST_OPEN
                    .to_string(),
        }
    }

    /// 计划断言组(计划 Step 1 244-245 行):normal 全 Unknown/Denied 时,
    /// recipe 相位凭有效凭据 + 已交付 recipe 证据放行;普通请求不得借用
    /// recipe 证据放行,且拒绝发生在 spawn 前(provider 零启动)。recipe 事实
    /// 不回写 normal 矩阵(隔离契约)。
    #[test]
    fn lcg_t02_unknown_normal_matrix_keeps_existing_root_recipe_contract() {
        let fixture = admission_fixture();
        let (_operations, _operation_id, credential) = derived_credential(&fixture);

        // 记录写入**项目级全局** capability store;source 走
        // `global_capability_with_lc_recipe_recheck`(capability 全局 +
        // recipe 凭据 durable 重核验通道仍按 LC)。
        let store = T02Store::new(fixture.paths.clone());
        store.ensure_bootstrap().unwrap();
        store
            .upsert(&t02_recipe_contract_record())
            .unwrap();
        let source = T02Source::global_capability_with_lc_recipe_recheck(fixture.paths.clone(), fixture.project_id.clone(), fixture.lc_id.clone());

        // recipe 相位:同一记录的已交付 recipe 证据 + 有效凭据(durable
        // Running 派生)→ 固定 Claude recipe 事实放行(先于 source 移入 gateway)。
        let valid_credential_existing_recipe_evidence = source.require_root_recipe_supported(
            &ProviderRef::claude_code("cap_managed_snapshot"),
            &credential,
        );
        assert!(
            valid_credential_existing_recipe_evidence.is_ok(),
            "valid credential + delivered recipe evidence must keep the recipe contract: {:?}",
            valid_credential_existing_recipe_evidence
        );

        // store-backed gateway(独立计数 adapter)承载普通会话请求。
        let streaming = Arc::new(CountingStreamingAdapter::new());
        let mut registry = ProviderRegistry::new();
        registry.register(
            crate::product::models::ProviderName::ClaudeCode,
            streaming.clone(),
        );
        let authority_root = std::fs::canonicalize(&fixture.aggregate_root).unwrap();
        let gateway = LogicalCodebaseProviderGateway::with_audit(
            fixture.policy_store.clone(),
            Arc::new(source),
            Arc::new(PassThroughTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            Arc::new(crate::product::logical_codebase::GatewayRunAudit::new()),
            authority_root.clone(),
        );

        // ordinary(正常会话)请求:CodingTargetWrite 的 write_boundary 为
        // Denied{root write probe rejected}——即使旧 supported_actions 列出该
        // action、record 携带已交付 recipe 证据,正常会话也不得借用它们放行。
        let request = SessionLaunchRequest {
            project_id: fixture.project_id.clone(),
            provider: ProviderRef::claude_code("cap_managed_snapshot"),
            action: SessionPolicyAction::CodingTargetWrite,
            target: crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                authority_root.clone(),
            ),
            working_directory: authority_root.clone(),
            readable_roots: vec![authority_root.clone()],
            writable_roots: vec![authority_root.clone()],
            config_artifact_ref: "sha256:recipe-config-artifact".to_string(),
        };
        let ordinary_request_using_recipe_evidence = gateway.validate(request);
        assert!(
            ordinary_request_using_recipe_evidence.is_err(),
            "ordinary request must not ride on recipe evidence: {:?}",
            ordinary_request_using_recipe_evidence
        );
        // 拒绝发生在 spawn 前:provider 零启动。
        assert_eq!(streaming.start_count(), 0);

        // recipe 事实不回写 normal 矩阵:行保持原三态,不产生 Confirmed。
        let reloaded = store
            .get(ProviderRefType::ClaudeCode)
            .unwrap()
            .unwrap();
        assert_eq!(
            reloaded
                .action_matrix
                .row(&SessionPolicyAction::ReviewReadOnly)
                .launch,
            T02Evidence::Unknown
        );
        assert_eq!(
            reloaded
                .action_matrix
                .row(&SessionPolicyAction::CodingTargetWrite)
                .write_boundary,
            T02Evidence::Denied {
                reason: "root write probe rejected".to_string()
            }
        );

        // 路B 扩展(fix-admission-member-rules-scope,先红):完整
        // admission→gateway 接线,不能只测 capability source——Normal 矩阵
        // PlanningReadOnly 分格也 Unknown 的记录 + 已交付 recipe 证据 +
        // 有效凭据(裸成员仓):AggregateBootstrap 相位的 admission 预检经
        // validate_root_recipe_request 放行,合法 recipe 不先被 Normal
        // capability 矩阵拒绝;同一 gateway 的普通(Normal)请求仍被
        // Normal 矩阵拒绝。
        let mut admission_record = t02_recipe_contract_record();
        admission_record.action_matrix = T02Matrix::from_rows(vec![T02Row {
            action: SessionPolicyAction::PlanningReadOnly,
            launch: T02Evidence::Unknown,
            resume: T02Evidence::Unknown,
            write_boundary: T02Evidence::Unknown,
            projection_digest: "projection-digest-PlanningReadOnly".to_string(),
            evidence_ref: "probe://PlanningReadOnly".to_string(),
        }])
        .unwrap();
        admission_record.supported_actions = Vec::new();
        store
            .upsert(&admission_record)
            .unwrap();
        let preflight =
            crate::product::logical_codebase::LogicalCodebaseProviderAdmissionPreflight::new(
                fixture.paths.clone(),
                fixture.lc_id.clone(),
                std::sync::Arc::new(gateway),
            );
        // bootstrap 相位:凭据 + 已交付 recipe 证据 → ready(裸成员仓,
        // 根规则/receipt 由 recipe 生成)。请求 provider ref 与记录的
        // capability snapshot 一致(cap_managed_snapshot)。
        let t02_launch_request = SessionLaunchRequest {
            project_id: fixture.project_id.clone(),
            provider: ProviderRef::claude_code("cap_managed_snapshot"),
            action: SessionPolicyAction::PlanningReadOnly,
            target: crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                std::fs::canonicalize(&fixture.aggregate_root).unwrap(),
            ),
            working_directory: fixture.aggregate_root.clone(),
            readable_roots: vec![fixture.aggregate_root.clone()],
            writable_roots: Vec::new(),
            config_artifact_ref: "sha256:admission-managed-config".to_string(),
        };
        let bootstrap_ready = preflight
            .check(
                &t02_launch_request,
                &ProviderAdmissionPhase::AggregateBootstrap(credential),
            )
            .expect("bootstrap admission must ride the root-recipe gateway channel");
        assert!(
            bootstrap_ready.ready,
            "missing: {:?}",
            bootstrap_ready.missing_materials
        );
        // 普通会话:同一 gateway 仍按 Normal 矩阵拒绝(PlanningReadOnly
        // Unknown),不借 recipe 证据。
        assert_bootstrap_waiting(
            preflight
                .check(&t02_launch_request, &ProviderAdmissionPhase::Normal)
                .map(|_| ()),
            "provider_capability_not_satisfied",
        );
        assert_eq!(streaming.start_count(), 0);
    }

    // ===== Task 2b 第二段(lcg_t02):root-recipe 凭据 durable 重验/漂移 =====

    /// 以三态记录 + 指定 Delivered 版本构造 source fixture（capability 记录
    /// 落**项目级全局**，recipe 凭据重核验通道按 LC）。
    fn t02_recipe_source_with_delivered_version(
        fixture: &AdmissionFixture,
        delivered_version: &str,
    ) -> T02Source {
        let store = T02Store::new(fixture.paths.clone());
        store.ensure_bootstrap().unwrap();
        let mut record = t02_recipe_contract_record();
        record.root_recipe_evidence = T02RecipeEvidence::Delivered {
            evidence_ref: "recipe://evidence-0001".to_string(),
            version: delivered_version.to_string(),
        };
        store.upsert(&record).unwrap();
        T02Source::global_capability_with_lc_recipe_recheck(fixture.paths.clone(), fixture.project_id.clone(), fixture.lc_id.clone())
    }

    /// 凭据每次对 durable Running 重验:Running 放行;operation 完成后
    /// (非 Running)拒绝,稳定码 root_recipe_credential_recheck_denied;
    /// 无重核验通道的 source(with_store,即使记录在场)fail-closed 拒绝,
    /// 稳定码 root_recipe_credential_recheck_unavailable。
    #[test]
    fn lcg_t02_root_recipe_credential_revalidated_against_durable_running() {
        let fixture = admission_fixture();
        let (operations, operation_id, credential) = derived_credential(&fixture);

        let source = t02_recipe_source_with_delivered_version(&fixture, "1.4.0");
        let provider_ref = ProviderRef::claude_code("cap_managed_snapshot");

        // durable Running → 固定 recipe 事实放行。
        source
            .require_root_recipe_supported(&provider_ref, &credential)
            .expect("running operation must keep the recipe contract");

        // operation 终态(Completed)→ 每次重验 fail-closed。
        finish_bootstrap_operation(&operations, &fixture, &operation_id);
        let error = source
            .require_root_recipe_supported(&provider_ref, &credential)
            .unwrap_err();
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason)
                if reason.starts_with("root_recipe_credential_recheck_denied")),
            "completed operation must fail the recheck: {error:?}"
        );

        // 无 durable 重核验通道(with_store 构造):即使记录在场也拒绝。
        let channelless = T02Source::with_store(
            T02Store::new(fixture.paths.clone()),
            fixture.project_id.clone(),
        );
        let error = channelless
            .require_root_recipe_supported(&provider_ref, &credential)
            .unwrap_err();
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason)
                if reason.starts_with("root_recipe_credential_recheck_unavailable")),
            "channel-less source must fail closed: {error:?}"
        );
    }

    /// 已交付 recipe 证据钉定版本与记录版本漂移 → fail-closed,稳定码
    /// root_recipe_evidence_version_drift(凭据有效也不例外)。
    #[test]
    fn lcg_t02_root_recipe_evidence_version_drift_rejected() {
        let fixture = admission_fixture();
        let (_operations, _operation_id, credential) = derived_credential(&fixture);

        // t02_recipe_contract_record 的记录版本为 1.4.0;Delivered 钉 9.9.9。
        let source = t02_recipe_source_with_delivered_version(&fixture, "9.9.9");
        let error = source
            .require_root_recipe_supported(
                &ProviderRef::claude_code("cap_managed_snapshot"),
                &credential,
            )
            .unwrap_err();
        assert!(
            matches!(&error, ProviderGatewayError::UnsupportedCapability(reason)
                if reason.starts_with("root_recipe_evidence_version_drift")),
            "delivered evidence version drift must fail closed: {error:?}"
        );
    }

    // ===== Task 14(lcg_t14):legacy Unknown 行的可操作迁移等待投影 =====

    /// 存量 v1 记录:supported_actions 为空(部分迁移形态),无 v2 矩阵
    /// 字段 → decode 后矩阵全 Unknown、trust Unknown。
    const LCG_T14_V1_LEGACY_RECORDS: &str = r#"[
  {
    "provider_type": "claude_code",
    "version": "0.0.0-managed",
    "adapter_dialect": "claude_code_cli_v1",
    "capability_snapshot_ref": "cap_managed_snapshot",
    "evidence": "fixture_verified",
    "resume_evidence": "confirmed",
    "supported_actions": []
  }
]"#;

    /// Task 14 主断言:legacy 矩阵 Unknown 的存量记录在 admission 侧保持
    /// 冻结 waiting(reason `provider_capability_not_satisfied` + 恒
    /// Revalidate/Retry + 零 spawn),且 capability store 投影出可操作的
    /// 迁移等待材料(逐维 evidence 缺失点名,不扩大 allowed actions)。
    #[test]
    fn lcg_t14_legacy_matrix_unknown_projects_actionable_waiting() {
        use crate::cross_cutting::provider_capabilities::ProviderCapabilityEvidence;
        use crate::product::logical_codebase::policy::ProviderWireDialect;
        use crate::product::logical_codebase::production_policy_resolvers::StoreBackedProviderCapabilitySource;
        use crate::product::logical_codebase::provider_capability_store::{
            PROVIDER_CAPABILITY_ACTION_UNKNOWN, ProviderCapabilityStore,
        };

        let mut fixture = admission_fixture();
        fixture.write_language_rules("# rules\n");

        // 存量 v1 记录写入**机器级** capability store(真实读取路径;
        // REQ-PCR-07：capability 记录唯一作用域 = 机器级)。
        let scope_root = fixture.paths.provider_capabilities_root();
        std::fs::create_dir_all(&scope_root).unwrap();
        std::fs::write(
            scope_root.join("capabilities.json"),
            LCG_T14_V1_LEGACY_RECORDS,
        )
        .unwrap();

        // gateway 换 store-backed capability source(与生产同型)。
        let mut registry = ProviderRegistry::new();
        registry.register(
            crate::product::models::ProviderName::ClaudeCode,
            fixture.streaming_adapter.clone(),
        );
        registry.register(
            crate::product::models::ProviderName::Codex,
            fixture.streaming_adapter.clone(),
        );
        fixture.gateway = Arc::new(LogicalCodebaseProviderGateway::with_audit(
            fixture.policy_store.clone(),
            Arc::new(
                StoreBackedProviderCapabilitySource::global_capability_with_lc_recipe_recheck(
                    fixture.paths.clone(),
                    fixture.project_id.clone(),
                    fixture.lc_id.clone(),
                ),
            ),
            Arc::new(PassThroughTargetResolver),
            Arc::new(registry),
            Arc::new(StubSyncAdapter),
            always_available_gate(),
            Arc::new(crate::product::logical_codebase::GatewayRunAudit::new()),
            std::fs::canonicalize(&fixture.aggregate_root).expect("canonical authority root"),
        ));

        // 存量读取:v1 记录的 launch 分格 = Unknown(不产生 Confirmed)。
        let capability_store = ProviderCapabilityStore::new(fixture.paths.clone());
        let legacy = capability_store
            .get(ProviderRefType::ClaudeCode)
            .unwrap()
            .unwrap();
        let legacy_launch_state = legacy
            .current_action_row(
                ProviderWireDialect::ClaudeCodeStreamJson,
                "0.0.0-managed",
                SessionPolicyAction::PlanningReadOnly,
            )
            .launch;
        assert_eq!(legacy_launch_state, ProviderCapabilityEvidence::Unknown);

        // admission:既有 waiting 面保持冻结(reason + allowed actions),
        // 材料面不含 capability 证据材料;provider 零启动。
        let request = SessionLaunchRequest::planning(
            fixture.project_id.clone(),
            ProviderRef::claude_code("cap_managed_snapshot"),
            crate::product::logical_codebase::policy::PolicyTarget::aggregate_root(
                fixture.aggregate_root.clone(),
            ),
            vec![fixture.aggregate_root.clone()],
            "sha256:admission-managed-config",
        );
        let error = fixture
            .preflight()
            .check(&request, &ProviderAdmissionPhase::Normal)
            .unwrap_err();
        match error {
            ProviderAdmissionError::Waiting {
                reason_code,
                missing_materials,
                allowed_actions,
                ..
            } => {
                assert_eq!(reason_code, "provider_capability_not_satisfied");
                assert!(
                    allowed_actions.contains(&BootstrapActionKind::Revalidate),
                    "frozen waiting face must keep Revalidate: {allowed_actions:?}"
                );
                assert!(
                    allowed_actions.contains(&BootstrapActionKind::Retry),
                    "frozen waiting face must keep Retry: {allowed_actions:?}"
                );
                assert!(
                    !missing_materials
                        .iter()
                        .any(|material| material.contains("probe evidence")),
                    "admission materials must not fabricate capability evidence: {missing_materials:?}"
                );
            }
            other => panic!("expected waiting fact, got {other:?}"),
        }
        assert_eq!(fixture.streaming_adapter.start_count(), 0);

        // Task 14 迁移等待投影(同一事实的可操作面):冻结判别码 + 逐维
        // missing materials + 不扩大的 allowed actions。
        let waiting = legacy
            .migration_waiting(
                ProviderWireDialect::ClaudeCodeStreamJson,
                "0.0.0-managed",
                SessionPolicyAction::PlanningReadOnly,
            )
            .expect("legacy unknown row must project an actionable waiting");
        assert_eq!(waiting.reason_code, PROVIDER_CAPABILITY_ACTION_UNKNOWN);
        let missing_evidence_ref = "probe evidence: claude_code@0.0.0-managed PlanningReadOnly launch cell is unknown; real probe evidence_ref required".to_string();
        assert!(
            waiting.missing_materials.contains(&missing_evidence_ref),
            "launch-cell evidence material must be listed verbatim: {:?}",
            waiting.missing_materials
        );
        assert!(
            waiting
                .missing_materials
                .iter()
                .any(|material| material.starts_with("projection evidence:"))
        );
        assert!(
            waiting
                .missing_materials
                .iter()
                .any(|material| material.starts_with("os evidence:"))
        );
        assert!(
            waiting
                .missing_materials
                .iter()
                .any(|material| material.starts_with("budget evidence:"))
        );
        assert!(
            waiting
                .missing_materials
                .iter()
                .any(|material| material.starts_with("trust evidence:"))
        );
        assert!(
            waiting
                .allowed_actions
                .contains(&BootstrapActionKind::Revalidate)
        );
        assert!(
            waiting
                .allowed_actions
                .contains(&BootstrapActionKind::Retry)
        );
        assert_eq!(
            waiting.allowed_actions.len(),
            2,
            "migration waiting face must not widen allowed actions"
        );
    }
}
