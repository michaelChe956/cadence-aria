/// 发布链确定性 IO 故障注入(仅 `cfg(test)` 编译,生产无任何开关)。
/// 单发语义:命中即消费,保证重试后发布可恢复。
#[cfg(test)]
mod publish_faults {
    use crate::product::json_store::ProductStoreError;
    use std::path::{Path, PathBuf};
    use std::sync::Mutex;

    /// 可注入故障的发布阶段。
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum FaultPhase {
        SourceRead,
        OutputWrite,
        LocatorPublish,
        ArtifactSave,
    }

    static ARMED: Mutex<Option<(PathBuf, FaultPhase)>> = Mutex::new(None);

    /// 注入以测试专属 tempdir 为界:并行测试互不污染。
    pub(super) fn arm(root: &Path, phase: FaultPhase) {
        *ARMED.lock().expect("publish fault mutex") = Some((root.to_path_buf(), phase));
    }

    /// 命中(路径在注入 root 下且阶段相同)即消费(单发);未命中照常执行。
    pub(super) fn trip(path: &Path, phase: FaultPhase) -> Option<ProductStoreError> {
        let mut armed = ARMED.lock().expect("publish fault mutex");
        match armed.as_ref() {
            Some((root, armed_phase)) if *armed_phase == phase && path.starts_with(root) => {
                let error = ProductStoreError::Io(format!("injected publish fault at {phase:?}"));
                *armed = None;
                Some(error)
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_initialization::{
        AggregateInitializationOperation, AggregateInitializationOperationInput,
        AggregateInitializationStepKind,
    };
    use crate::product::logical_codebase::aggregate_initialization_store::AggregateInitializationOperationStore;
    use uuid::Uuid;

    #[test]
    fn envelope_freezes_policy_target_roots_dialect_and_managed_config_digest() {
        let artifact = AggregatePolicyArtifact::bootstrap(
            "project_0001",
            "logical_0001",
            "2026-08-09T00:00:00Z".into(),
        );
        let envelope = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::CodingTargetWrite,
            PolicyTarget::checkout(
                "logical_repo",
                "checkout",
                "/work/api/.worktrees/aria-issues/issue_1",
            ),
            PathBuf::from("/lc-root"),
            vec![std::path::PathBuf::from("/aggregate")],
            vec![std::path::PathBuf::from(
                "/work/api/.worktrees/aria-issues/issue_1",
            )],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:settings".into(),
            "2026-08-09T00:00:00Z".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();

        assert_eq!(envelope.policy_digest, artifact.digest);
        assert_eq!(envelope.writable_roots.len(), 1);
        assert_eq!(envelope.action, SessionPolicyAction::CodingTargetWrite);
        assert!(
            serde_json::to_value(&envelope)
                .unwrap()
                .get("config_artifact_ref")
                .is_some()
        );
    }

    #[test]
    fn bootstrap_digest_is_canonical_sha256_of_policy_text() {
        let artifact =
            AggregatePolicyArtifact::bootstrap("p1", "l1", "2026-08-09T00:00:00Z".into());
        let expected = format!(
            "sha256:{:x}",
            Sha256::digest(BOOTSTRAP_POLICY_TEXT.as_bytes())
        );
        assert_eq!(artifact.digest, expected);
        assert_eq!(artifact.revision, 1);
        assert!(artifact.policy_id.contains("p1"));
        assert!(artifact.policy_id.contains("l1"));
    }

    #[test]
    fn read_only_actions_reject_any_writable_root() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let target = PolicyTarget::checkout("repo", "co", "/work/repo");
        let error = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::PlanningReadOnly,
            target.clone(),
            PathBuf::from("/lc-root"),
            vec![PathBuf::from("/work/repo")],
            vec![PathBuf::from("/work/repo")],
            ProviderDialect::CodexCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap_err();
        assert!(
            matches!(error, ProductStoreError::InvalidRecord { ref reason, .. } if reason.starts_with(POLICY_ENVELOPE_INVALID_ROOTS))
        );

        let ok = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::ReviewReadOnly,
            target,
            PathBuf::from("/lc-root"),
            vec![PathBuf::from("/work/repo")],
            vec![],
            ProviderDialect::CodexCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();
        assert!(ok.writable_roots.is_empty());
    }

    #[test]
    fn coding_action_requires_single_writable_root_equal_to_target() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let target = PolicyTarget::checkout("repo", "co", "/work/repo");

        // wrong root
        let err = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::CodingTargetWrite,
            target.clone(),
            PathBuf::from("/lc-root"),
            vec![],
            vec![PathBuf::from("/elsewhere")],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap_err();
        assert!(
            matches!(err, ProductStoreError::InvalidRecord { ref reason, .. } if reason.contains("policy_envelope_invalid_roots"))
        );

        // two roots
        let err = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::CodingTargetWrite,
            target.clone(),
            PathBuf::from("/lc-root"),
            vec![],
            vec![PathBuf::from("/work/repo"), PathBuf::from("/other")],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap_err();
        assert!(
            matches!(err, ProductStoreError::InvalidRecord { ref reason, .. } if reason.contains("policy_envelope_invalid_roots"))
        );

        // correct single root
        let ok = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::CodingTargetWrite,
            target,
            PathBuf::from("/lc-root"),
            vec![],
            vec![PathBuf::from("/work/repo")],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();
        assert_eq!(ok.writable_roots, vec![PathBuf::from("/work/repo")]);
    }

    #[test]
    fn empty_config_artifact_ref_is_rejected() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let err = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::PlanningReadOnly,
            PolicyTarget::checkout("repo", "co", "/work/repo"),
            PathBuf::from("/lc-root"),
            vec![],
            vec![],
            ProviderDialect::ClaudeCodeCliV1,
            String::new(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap_err();
        assert!(
            matches!(err, ProductStoreError::InvalidRecord { ref reason, .. } if reason.contains("policy_envelope_invalid_roots"))
        );
    }

    #[test]
    fn store_roundtrips_and_recomputes_digest_on_save() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregatePolicyArtifactStore::new(ProductAppPaths::new(temp.path()));
        let artifact = AggregatePolicyArtifact::bootstrap(
            "project_0001",
            "logical_0001",
            "2026-08-09T00:00:00Z".into(),
        );

        store.save("project_0001", &artifact).unwrap();
        let loaded = store.get("project_0001").unwrap().unwrap();
        assert_eq!(loaded, artifact);
        assert!(
            temp.path()
                .join("projects/project_0001/logical-codebase/aggregate-policy.json")
                .exists()
        );

        // caller-supplied arbitrary digest is rejected on save
        let mut bad = artifact.clone();
        bad.digest = "sha256:deadbeef".into();
        assert!(store.save("project_0001", &bad).is_err());
    }

    #[test]
    fn save_rejects_non_advancing_revision() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregatePolicyArtifactStore::new(ProductAppPaths::new(temp.path()));
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        store.save("p1", &artifact).unwrap();

        let mut duplicate = artifact.clone();
        duplicate.revision = 1; // same revision
        assert!(store.save("p1", &duplicate).is_err());
    }

    #[test]
    fn ensure_bootstrap_is_idempotent_and_refuses_mismatched_identity() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregatePolicyArtifactStore::new(ProductAppPaths::new(temp.path()));
        let manifest =
            LogicalCodebaseManifest::new("project_0001", temp.path().to_path_buf(), vec![]);

        let first = store.ensure_bootstrap(&manifest).unwrap();
        assert_eq!(first.revision, 1);
        let second = store.ensure_bootstrap(&manifest).unwrap();
        assert_eq!(first, second);

        // a different logical-codebase identity is not overwritten
        let mut other = manifest.clone();
        other.logical_codebase_id = Uuid::new_v4();
        assert!(matches!(
            store.ensure_bootstrap(&other),
            Err(ProductStoreError::IdentityMismatch { .. })
        ));
    }

    /// `with_revised_policy` 提升 revision、重算 canonical digest 与 policy_id,
    /// 且可作为合法 successor 被保存(gateway spawn 前复验测试依赖此路径)。
    #[test]
    fn with_revised_policy_advances_revision_and_recomputes_digest() {
        let temp = tempfile::tempdir().unwrap();
        let store = AggregatePolicyArtifactStore::new(ProductAppPaths::new(temp.path()));
        let manifest =
            LogicalCodebaseManifest::new("project_0001", temp.path().to_path_buf(), vec![]);
        let bootstrap = store.ensure_bootstrap(&manifest).unwrap();

        let revised =
            bootstrap.with_revised_policy("# revision 2 policy text\n", "2026-08-10T00:00:00Z");
        assert_eq!(revised.revision, 2);
        assert_ne!(revised.digest, bootstrap.digest);
        assert!(revised.policy_id.ends_with("/2"));
        // digest 是新 policy_text 的 canonical sha256
        let expected = format!("sha256:{:x}", Sha256::digest(b"# revision 2 policy text\n"));
        assert_eq!(revised.digest, expected);
        // 可作为 successor 保存
        store.save("project_0001", &revised).unwrap();
        let reloaded = store.get("project_0001").unwrap().unwrap();
        assert_eq!(reloaded, revised);
    }

    /// 存量 envelope JSON(无 `authority_root` 键)反序列化不失败:serde 缺省为
    /// `PathBuf::default()`,新字段不破坏旧记录读取。
    #[test]
    fn legacy_envelope_json_without_authority_root_deserializes_with_default() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let envelope = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::PlanningReadOnly,
            PolicyTarget::aggregate_root(PathBuf::from("/aggregate")),
            PathBuf::from("/lc-root"),
            vec![PathBuf::from("/aggregate")],
            vec![],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:cfg".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();

        let mut json = serde_json::to_value(&envelope).unwrap();
        json.as_object_mut()
            .expect("envelope serializes to an object")
            .remove("authority_root");
        let restored: SessionPolicyEnvelope =
            serde_json::from_value(json).expect("legacy envelope JSON must deserialize");

        assert_eq!(restored.authority_root, PathBuf::new());
        assert_eq!(restored.policy_id, envelope.policy_id);
    }

    /// `recompute_config_digest` 与 `new` 内部冻结的 config_digest 算法一致,
    /// 供 gateway spawn 前复验托管配置未被篡改。
    #[test]
    fn recompute_config_digest_matches_envelope_frozen_value() {
        let artifact = AggregatePolicyArtifact::bootstrap("p1", "l1", "now".into());
        let envelope = SessionPolicyEnvelope::new(
            &artifact,
            SessionPolicyAction::PlanningReadOnly,
            PolicyTarget::checkout("repo", "co", "/work/repo"),
            PathBuf::from("/lc-root"),
            vec![],
            vec![],
            ProviderDialect::ClaudeCodeCliV1,
            "sha256:managed-config".into(),
            "now".into(),
            PathBuf::from("/authority-root"),
        )
        .unwrap();
        let recomputed =
            SessionPolicyEnvelope::recompute_config_digest("sha256:managed-config").unwrap();
        assert_eq!(recomputed, envelope.config_digest);
    }

    // Task 1(REQ-BOOT-05/REQ-ENV-12):recipe_policy_* 根政策发布测试

    const PROJECT: &str = "project_0001";

    /// 统一的测试时间戳常量与构造器。
    const AT_0800: &str = "2026-10-03T08:00:00Z";
    const AT_0830: &str = "2026-10-03T08:30:00Z";
    const AT_0900: &str = "2026-10-03T09:00:00Z";
    const AT_0930: &str = "2026-10-03T09:30:00Z";
    const AT_1000: &str = "2026-10-03T10:00:00Z";
    const AT_1100: &str = "2026-10-03T11:00:00Z";
    const AT_1200: &str = "2026-10-03T12:00:00Z";
    const AT_1300: &str = "2026-10-03T13:00:00Z";
    const AT_1400: &str = "2026-10-03T14:00:00Z";

    fn ts(clock: &str) -> String {
        format!("2026-10-03T{clock}:00Z")
    }

    fn sha256_hex_of(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    /// 根规则 fixture:入口 + 排序规则集 + 各类干扰材料(provider 副本/
    /// 凭据/模板/成员规则/非 md/其他规则目录),期望正文由
    /// [`expected_policy_text`] 逐字节钉定。
    fn write_root_fixture(root: &Path) {
        for dir in [
            ".claude/rules/sub",
            ".agents/rules",
            "cadence/project-rules/examples",
            "member",
        ] {
            std::fs::create_dir_all(root.join(dir)).unwrap();
        }
        for (relative, content) in [
            (
                "AGENTS.md",
                "# 根规则\r\n\r\n规则正文见 .claude/rules/ 目录。\r\n",
            ),
            (".claude/rules/a.md", "规则 A\n"),
            (".claude/rules/b.md", "规则 B\n"),
            (
                ".claude/rules/language.md",
                "---\nfront matter 原样保留\n---\n\n# 语言规则\n",
            ),
            (".claude/rules/sub/nested.md", "嵌套规则\n"),
            (".claude/rules/notes.txt", "非 md 笔记\n"),
            ("CLAUDE.md", "provider 兼容副本不重复激活\n"),
            (".mcp.json", "{\"mcp\":\"凭据\"}\n"),
            (".agents/rules/agent.md", "其他规则目录不进入政策\n"),
            (
                "cadence/project-rules/examples/example.md",
                "示例模板不成为政策约束\n",
            ),
            ("member/AGENTS.md", "成员仓规则不进入政策\n"),
        ] {
            std::fs::write(root.join(relative), content).unwrap();
        }
    }

    /// 冻结格式(Global Constraints):固定标题 + 每来源 `## 来源:<相对
    /// 路径>` + 完整原字节 + LF 分隔;入口第一,其余按相对路径字节序。
    fn expected_policy_text() -> String {
        let mut expected = String::from("# 聚合政策\n\n");
        for (relative_path, content) in [
            (
                "AGENTS.md",
                "# 根规则\r\n\r\n规则正文见 .claude/rules/ 目录。\r\n",
            ),
            (".claude/rules/a.md", "规则 A\n"),
            (".claude/rules/b.md", "规则 B\n"),
            (
                ".claude/rules/language.md",
                "---\nfront matter 原样保留\n---\n\n# 语言规则\n",
            ),
            (".claude/rules/sub/nested.md", "嵌套规则\n"),
        ] {
            expected.push_str("## 来源：");
            expected.push_str(relative_path);
            expected.push_str("\n\n");
            expected.push_str(content);
            expected.push_str("\n\n");
        }
        expected
    }

    /// 构造同 scope、末步 OpenspecAndExamples 处于 Running 的 operation,
    /// 返回与该 root 关联的新 manifest(不放宽生产状态核验)。
    fn seed_final_step_operation(
        paths: &ProductAppPaths,
        lc_id: &str,
        operation_id: &str,
        canonical_root: &Path,
    ) -> LogicalCodebaseManifest {
        let manifest = LogicalCodebaseManifest::new(PROJECT, canonical_root.to_path_buf(), vec![]);
        seed_operation_for_manifest(paths, lc_id, operation_id, &manifest);
        manifest
    }

    /// 以既有 manifest(同一 LC 身份)追加同 scope、末步 Running 的
    /// operation:同一 LC 的多个 operation 共享同一 manifest UUID/root,
    /// 与生产形态一致。
    fn seed_operation_for_manifest(
        paths: &ProductAppPaths,
        lc_id: &str,
        operation_id: &str,
        manifest: &LogicalCodebaseManifest,
    ) {
        let store = AggregateInitializationOperationStore::for_lc(paths.clone(), lc_id);
        let operation = AggregateInitializationOperation::new(
            operation_id.to_string(),
            PROJECT.to_string(),
            AggregateInitializationOperationInput {
                idempotency_key: format!("idem-{operation_id}"),
                manifest_revision: manifest.membership_revision,
                policy_digest: "sha256:bootstrap".to_string(),
                profile_evidence_digest: None,
                provider_context_root: manifest.provider_context_root.clone(),
                provider: "claude_code".to_string(),
            },
            ts("00:00"),
        );
        store.create_idempotent(operation).unwrap();
        store
            .mark_running(PROJECT, operation_id, ts("00:01"))
            .unwrap();
        let steps = AggregateInitializationStepKind::V1;
        for step in &steps[..steps.len() - 1] {
            let input = format!("aggregate-init:{operation_id}:{}", step.as_str());
            let output = format!(
                "aggregate-initializations/{operation_id}/{}.json",
                step.as_str()
            );
            store
                .mark_step_running(PROJECT, operation_id, *step, input, ts("00:02"))
                .unwrap();
            store
                .checkpoint_step_output(PROJECT, operation_id, *step, output, ts("00:03"))
                .unwrap();
            store
                .mark_step_completed(PROJECT, operation_id, *step, ts("00:04"))
                .unwrap();
        }
        let final_input = format!("aggregate-init:{operation_id}:openspec_and_examples");
        store
            .mark_step_running(
                PROJECT,
                operation_id,
                steps[steps.len() - 1],
                final_input,
                ts("00:05"),
            )
            .unwrap();
    }

    /// 绑定 store/root 的发布调用器:测试内以 `publish(&manifest, op, at)`
    /// 一行式驱动发布路径。
    fn publisher(
        store: &AggregatePolicyArtifactStore,
        canonical_root: &Path,
    ) -> impl Fn(
        &LogicalCodebaseManifest,
        &str,
        &str,
    ) -> Result<AggregatePolicyArtifact, ProductStoreError> {
        move |manifest, operation_id, created_at| {
            store.publish_recipe_policy(
                manifest,
                operation_id,
                canonical_root,
                created_at.to_string(),
            )
        }
    }

    fn output_path_for(paths: &ProductAppPaths, lc_id: &str, operation_id: &str) -> PathBuf {
        paths
            .logical_codebases_root(PROJECT)
            .join(lc_id)
            .join("aggregate-initializations")
            .join(operation_id)
            .join("policy-publication.json")
    }

    fn tamper_output_json(path: &Path, mutate: impl FnOnce(&mut serde_json::Value)) {
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
        mutate(&mut value);
        std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    }

    /// 来源字节集完整性:入口第一、嵌套排序、CRLF/front matter 原样;
    /// 干扰材料不入正文;正文不含 revision/policy_id/时间/绝对 root。
    #[test]
    fn recipe_policy_collects_full_sorted_source_bytes() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_collect", &canonical);

        let artifact = publisher(&store, &canonical)(&manifest, "op_collect", AT_0800).unwrap();

        let expected = expected_policy_text();
        assert_eq!(artifact.policy_text.as_bytes(), expected.as_bytes());
        assert_eq!(artifact.digest, sha256_hex_of(expected.as_bytes()));
        for absent in [
            "兼容副本",
            "mcp",
            "示例模板",
            "成员仓规则",
            "其他规则目录",
            "非 md 笔记",
        ] {
            assert!(
                !artifact.policy_text.contains(absent),
                "正文不得包含干扰材料:{absent}"
            );
        }
        assert!(!artifact.policy_text.contains(&artifact.policy_id));
        assert!(!artifact.policy_text.contains("2026-10-03"));
        assert!(!artifact.policy_text.contains(temp.path().to_str().unwrap()));
    }

    /// locator 与原字节 digest:根下 policy_id 与正文逐字节一致;digest 等于独立 SHA-256。
    #[test]
    fn recipe_policy_publishes_locator_and_raw_digest() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_locator", &canonical);
        let bootstrap = store.ensure_bootstrap(&manifest).unwrap();

        let artifact = publisher(&store, &canonical)(&manifest, "op_locator", AT_0800).unwrap();

        let locator = canonical.join(&artifact.policy_id);
        assert_eq!(
            std::fs::read(&locator).unwrap(),
            artifact.policy_text.as_bytes()
        );
        assert_eq!(artifact.revision, 2);
        assert_eq!(
            artifact.digest,
            sha256_hex_of(artifact.policy_text.as_bytes())
        );
        assert_eq!(
            artifact.policy_id,
            format!("policy/{PROJECT}/{}/2", manifest.logical_codebase_id)
        );
        assert!(!artifact.policy_id.contains("lc_0001"));

        let output = store
            .get_recipe_policy_publication(PROJECT, "op_locator")
            .unwrap()
            .expect("publication output");
        assert_eq!(output.sources[0].relative_path, "AGENTS.md");
        let entry_digest =
            sha256_hex_of("# 根规则\r\n\r\n规则正文见 .claude/rules/ 目录。\r\n".as_bytes());
        assert_eq!(output.rule_digest, entry_digest);
        assert_eq!(output.sources[0].digest, output.rule_digest);
        assert_ne!(output.rule_digest, artifact.digest);
        assert_eq!(output.base_policy.revision, 1);
        assert_eq!(output.base_policy.policy_id, bootstrap.policy_id);
        assert_eq!(output.artifact, artifact);
        assert_eq!(output.canonical_root, canonical);
    }

    /// 同 operation 重入复用候选与时间:更晚时间重入返回逐字节相等的
    /// artifact;output/locator 字节不变;ensure_bootstrap 返回真正文。
    #[test]
    fn recipe_policy_same_operation_reentry_keeps_candidate_and_time() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_reentry", &canonical);
        store.ensure_bootstrap(&manifest).unwrap();

        let publish = publisher(&store, &canonical);
        let first = publish(&manifest, "op_reentry", AT_0800).unwrap();
        let output_path = output_path_for(&paths, "lc_0001", "op_reentry");
        let locator = canonical.join(&first.policy_id);
        let frozen_output = std::fs::read(&output_path).unwrap();
        let frozen_locator = std::fs::read(&locator).unwrap();

        let second = publish(&manifest, "op_reentry", AT_0930).unwrap();
        assert_eq!(second, first);
        assert_eq!(std::fs::read(&output_path).unwrap(), frozen_output);
        assert_eq!(std::fs::read(&locator).unwrap(), frozen_locator);
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), first);

        let bootstrap_view = store.ensure_bootstrap(&manifest).unwrap();
        assert_eq!(bootstrap_view, first);
        assert!(!bootstrap_view.is_bootstrap_placeholder());
    }

    /// 实际新 operation 即使正文不变也递增 revision、换 locator;旧字节不变。
    #[test]
    fn recipe_policy_new_operation_advances_even_when_text_unchanged() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_first", &canonical);
        store.ensure_bootstrap(&manifest).unwrap();
        seed_operation_for_manifest(&paths, "lc_0001", "op_second", &manifest);

        let publish = publisher(&store, &canonical);
        let first = publish(&manifest, "op_first", AT_0800).unwrap();

        let first_locator = canonical.join(&first.policy_id);
        let first_output = output_path_for(&paths, "lc_0001", "op_first");
        let frozen_locator = std::fs::read(&first_locator).unwrap();
        let frozen_output = std::fs::read(&first_output).unwrap();

        let second = publish(&manifest, "op_second", AT_0900).unwrap();
        assert_eq!(second.revision, first.revision + 1);
        assert_ne!(second.policy_id, first.policy_id);
        assert_eq!(second.digest, first.digest);
        assert_eq!(std::fs::read(&first_locator).unwrap(), frozen_locator);
        assert_eq!(std::fs::read(&first_output).unwrap(), frozen_output);
        let output_b = store
            .get_recipe_policy_publication(PROJECT, "op_second")
            .unwrap()
            .expect("second publication");
        assert_eq!(output_b.artifact, second);
    }

    /// 来源缺失/空/纯白/缺 language/不可读/非法 UTF-8 一律失败;current
    /// 不变、无候选正文、不落 locator;不可读走 IO seam,不依赖 chmod。
    #[test]
    fn recipe_policy_rejects_missing_empty_invalid_utf8_and_unreadable_sources() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let good_root = temp.path().join("good-root");
        std::fs::create_dir_all(&good_root).unwrap();
        write_root_fixture(&good_root);
        let good_canonical = std::fs::canonicalize(&good_root).unwrap();
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_baseline", &good_canonical);
        let baseline = store.ensure_bootstrap(&manifest).unwrap();

        fn apply_negative_case(name: &str, root: &Path) {
            write_root_fixture(root);
            match name {
                "missing_agents" => std::fs::remove_file(root.join("AGENTS.md")).unwrap(),
                "empty_entry" => std::fs::write(root.join("AGENTS.md"), "").unwrap(),
                "whitespace_entry" => std::fs::write(root.join("AGENTS.md"), " \r\n\t").unwrap(),
                "missing_language" => {
                    std::fs::remove_file(root.join(".claude/rules/language.md")).unwrap()
                }
                "missing_rules_dir" => std::fs::remove_dir_all(root.join(".claude/rules")).unwrap(),
                "entry_is_directory" => {
                    std::fs::remove_file(root.join("AGENTS.md")).unwrap();
                    std::fs::create_dir_all(root.join("AGENTS.md")).unwrap();
                }
                "rules_dir_is_file" => {
                    std::fs::remove_dir_all(root.join(".claude/rules")).unwrap();
                    std::fs::write(root.join(".claude/rules"), "not a dir").unwrap();
                }
                "invalid_utf8" => {
                    std::fs::write(root.join("AGENTS.md"), b"# \xF0\x9F\x92\xA9\xFF\xFE\n").unwrap()
                }
                _ => unreachable!("unknown negative case {name}"),
            }
        }

        let cases = [
            "missing_agents",
            "empty_entry",
            "whitespace_entry",
            "missing_language",
            "missing_rules_dir",
            "entry_is_directory",
            "rules_dir_is_file",
            "invalid_utf8",
        ];
        for name in cases {
            let root = temp.path().join(name);
            std::fs::create_dir_all(&root).unwrap();
            apply_negative_case(name, &root);
            let canonical = std::fs::canonicalize(&root).unwrap();
            let operation_id = format!("op_{name}");
            let case_manifest =
                seed_final_step_operation(&paths, "lc_0001", &operation_id, &canonical);
            assert!(
                publisher(&store, &canonical)(&case_manifest, &operation_id, AT_0800).is_err(),
                "case {name} must fail"
            );
            let current = store.get(PROJECT);
            assert_eq!(
                current.unwrap().unwrap(),
                baseline,
                "case {name}: current 不变"
            );
            let published = store.get_recipe_policy_publication(PROJECT, &operation_id);
            assert!(published.unwrap().is_none(), "case {name}: 无候选成功正文");
            assert!(
                !canonical.join("policy").exists(),
                "case {name}: 不落 locator"
            );
        }
        // 不可读来源:确定性 IO seam 注入(不依赖 chmod 对 root 失效)。
        let seam_manifest =
            seed_final_step_operation(&paths, "lc_0001", "op_unreadable", &good_canonical);
        publish_faults::arm(temp.path(), publish_faults::FaultPhase::SourceRead);
        assert!(
            publisher(&store, &good_canonical)(&seam_manifest, "op_unreadable", AT_0800).is_err()
        );
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), baseline);
        let seam_output = store.get_recipe_policy_publication(PROJECT, "op_unreadable");
        assert!(seam_output.unwrap().is_none());
        assert!(!good_canonical.join("policy").exists());
    }

    /// symlink(来源/父目录/目标)、非普通/不同字节目标均拒绝且用户字节不变;相同字节可复用。
    #[test]
    fn recipe_policy_rejects_symlink_and_existing_different_locator() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_symlink", &canonical);
        store.ensure_bootstrap(&manifest).unwrap();
        let uuid = manifest.logical_codebase_id.to_string();
        let locator = canonical.join(format!("policy/{PROJECT}/{uuid}/2"));
        let candidate = expected_policy_text();
        let publish = publisher(&store, &canonical);
        // 来源文件 symlink:fail-closed,用户 symlink 不被动。
        let rules_a = canonical.join(".claude/rules/a.md");
        let real_target = temp.path().join("real-a.md");
        std::fs::write(&real_target, "规则 A\n").unwrap();
        std::fs::remove_file(&rules_a).unwrap();
        std::os::unix::fs::symlink(&real_target, &rules_a).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        assert!(std::fs::symlink_metadata(&rules_a).unwrap().is_symlink());
        std::fs::remove_file(&rules_a).unwrap();
        std::fs::write(&rules_a, "规则 A\n").unwrap();

        // 来源目录 symlink:fail-closed。
        let sub = canonical.join(".claude/rules/sub");
        let real_dir = temp.path().join("real-sub");
        std::fs::create_dir_all(&real_dir).unwrap();
        std::fs::remove_dir_all(&sub).unwrap();
        std::os::unix::fs::symlink(&real_dir, &sub).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        std::fs::remove_file(&sub).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("nested.md"), "嵌套规则\n").unwrap();

        // 政策父目录 symlink(root/policy 指向外部目录):fail-closed。
        let outside = temp.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, canonical.join("policy")).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        std::fs::remove_file(canonical.join("policy")).unwrap();

        // 目标为 symlink:fail-closed,链接不被动。
        std::fs::create_dir_all(locator.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(temp.path().join("elsewhere.txt"), &locator).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        assert!(std::fs::symlink_metadata(&locator).unwrap().is_symlink());
        std::fs::remove_file(&locator).unwrap();

        // 目标为目录(非普通文件):fail-closed。
        std::fs::create_dir_all(&locator).unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        std::fs::remove_dir_all(&locator).unwrap();

        // 目标为不同字节的普通文件:冲突不覆盖,用户字节不变。
        std::fs::write(&locator, b"user-owned bytes\n").unwrap();
        assert!(publish(&manifest, "op_symlink", AT_0800).is_err());
        assert_eq!(std::fs::read(&locator).unwrap(), b"user-owned bytes\n");

        // 相同字节普通目标:复验后复用,发布成功。
        std::fs::write(&locator, candidate.as_bytes()).unwrap();
        let artifact = publish(&manifest, "op_symlink", AT_0800).unwrap();
        assert_eq!(std::fs::read(&locator).unwrap(), candidate.as_bytes());
        assert_eq!(artifact.policy_text, candidate);
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), artifact);
    }

    /// source/清单/operation/root/scope/UUID/base 漂移与 revision 溢出均冲突;不覆盖 current。
    #[test]
    fn recipe_policy_rejects_source_identity_base_drift_and_overflow() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_drift", &canonical);
        store.ensure_bootstrap(&manifest).unwrap();
        let publish = publisher(&store, &canonical);
        let first = publish(&manifest, "op_drift", AT_0800).unwrap();
        let output_path = output_path_for(&paths, "lc_0001", "op_drift");
        let frozen = std::fs::read(&output_path).unwrap();
        let current = first.clone();
        let original_entry = "# 根规则\r\n\r\n规则正文见 .claude/rules/ 目录。\r\n";

        // source 字节漂移。
        std::fs::write(canonical.join("AGENTS.md"), "# 漂移后的根规则\n").unwrap();
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        assert_eq!(std::fs::read(&output_path).unwrap(), frozen);
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), current);
        std::fs::write(canonical.join("AGENTS.md"), original_entry).unwrap();

        // source 清单漂移(新增规则文件)。
        std::fs::write(canonical.join(".claude/rules/zz.md"), "新规则\n").unwrap();
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        assert_eq!(std::fs::read(&output_path).unwrap(), frozen);
        std::fs::remove_file(canonical.join(".claude/rules/zz.md")).unwrap();

        // operation 引用漂移(篡改 output 的 operation_id)。
        tamper_output_json(&output_path, |value| {
            value["operation_id"] = serde_json::json!("op_other")
        });
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        std::fs::write(&output_path, &frozen).unwrap();

        // root 引用漂移(同 operation 换 canonical root 参数)。
        let other_root = temp.path().join("other-root");
        std::fs::create_dir_all(&other_root).unwrap();
        write_root_fixture(&other_root);
        let other_canonical = std::fs::canonicalize(&other_root).unwrap();
        assert!(publisher(&store, &other_canonical)(&manifest, "op_drift", AT_0830).is_err());

        // UUID 引用漂移(同 root、不同 manifest UUID)。
        let drifted_manifest =
            LogicalCodebaseManifest::new(PROJECT, canonical.to_path_buf(), vec![]);
        assert!(publish(&drifted_manifest, "op_drift", AT_0830).is_err());

        // scope 引用漂移(篡改 output 的 lc_id)。
        tamper_output_json(&output_path, |value| {
            value["lc_id"] = serde_json::json!("lc_other")
        });
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        std::fs::write(&output_path, &frozen).unwrap();

        // base 三元引用漂移(候选 revision 不再与 base 自洽)。
        tamper_output_json(&output_path, |value| {
            value["base_policy"]["revision"] = serde_json::json!(7)
        });
        assert!(publish(&manifest, "op_drift", AT_0830).is_err());
        std::fs::write(&output_path, &frozen).unwrap();
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), current);

        // revision 溢出:base revision 为 u64::MAX 时 checked_add 拒绝。
        let overflow_store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_overflow");
        let overflow_manifest =
            seed_final_step_operation(&paths, "lc_overflow", "op_overflow", &canonical);
        let max_artifact = AggregatePolicyArtifact {
            policy_id: format!(
                "policy/{PROJECT}/{}/{}",
                overflow_manifest.logical_codebase_id,
                u64::MAX
            ),
            project_id: PROJECT.to_string(),
            logical_codebase_id: overflow_manifest.logical_codebase_id.to_string(),
            revision: u64::MAX,
            digest: sha256_hex_of(b"max\n"),
            policy_text: "max\n".to_string(),
            created_at: "2026-10-03T00:00:00Z".to_string(),
        };
        overflow_store.save(PROJECT, &max_artifact).unwrap();
        assert!(
            publisher(&overflow_store, &canonical)(&overflow_manifest, "op_overflow", AT_0900)
                .is_err()
        );
        assert_eq!(overflow_store.get(PROJECT).unwrap().unwrap(), max_artifact);
        assert!(
            overflow_store
                .get_recipe_policy_publication(PROJECT, "op_overflow")
                .unwrap()
                .is_none()
        );
    }

    /// output/正文/current 三阶段故障分别保留已完成前缀;失败不返回成功;
    /// 清 staging 后 output 仍在;重入复用同候选与时间。
    #[test]
    fn recipe_policy_durable_write_failure_preserves_resume_evidence() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");

        let publish = publisher(&store, &canonical);
        // 阶段一:不可变 output 写入失败 → 无候选落盘。
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_resume", &canonical);
        let bootstrap = store.ensure_bootstrap(&manifest).unwrap();
        publish_faults::arm(temp.path(), publish_faults::FaultPhase::OutputWrite);
        assert!(publish(&manifest, "op_resume", AT_0800).is_err());
        assert!(!output_path_for(&paths, "lc_0001", "op_resume").exists());
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), bootstrap);
        // 重入(output 未冻结):发布成功,时间取本次调用。
        let resumed = publish(&manifest, "op_resume", AT_1000).unwrap();
        assert_eq!(resumed.revision, 2);
        assert_eq!(resumed.created_at, "2026-10-03T10:00:00Z");
        // 阶段二:locator 发布失败 → output 前缀保留,current 不变。
        seed_operation_for_manifest(&paths, "lc_0001", "op_resume2", &manifest);
        publish_faults::arm(temp.path(), publish_faults::FaultPhase::LocatorPublish);
        assert!(publish(&manifest, "op_resume2", AT_1100).is_err());
        let output2 = output_path_for(&paths, "lc_0001", "op_resume2");
        assert!(output2.exists());
        assert!(
            !canonical
                .join(format!(
                    "policy/{PROJECT}/{}/3",
                    manifest.logical_codebase_id
                ))
                .exists()
        );
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), resumed);

        // 清 staging 后 output 仍在。
        let staging = output2.parent().unwrap().join("staging");
        std::fs::create_dir_all(&staging).unwrap();
        std::fs::write(staging.join("partial.txt"), "partial").unwrap();
        std::fs::remove_dir_all(&staging).unwrap();
        assert!(output2.exists());

        // 重入复用同候选与首次冻结的时间。
        let done2 = publish(&manifest, "op_resume2", AT_1200).unwrap();
        assert_eq!(done2.revision, 3);
        assert_eq!(done2.created_at, "2026-10-03T11:00:00Z");
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), done2);
        // 阶段三:current artifact 替换失败 → output/locator 前缀保留。
        seed_operation_for_manifest(&paths, "lc_0001", "op_resume3", &manifest);
        publish_faults::arm(temp.path(), publish_faults::FaultPhase::ArtifactSave);
        assert!(publish(&manifest, "op_resume3", AT_1300).is_err());
        let output3 = output_path_for(&paths, "lc_0001", "op_resume3");
        let locator4 = canonical.join(format!(
            "policy/{PROJECT}/{}/4",
            manifest.logical_codebase_id
        ));
        assert!(output3.exists());
        assert_eq!(
            std::fs::read(&locator4).unwrap(),
            expected_policy_text().as_bytes()
        );
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), done2);

        let done3 = publish(&manifest, "op_resume3", AT_1400).unwrap();
        assert_eq!(done3.revision, 4);
        assert_eq!(done3.created_at, "2026-10-03T13:00:00Z");
        assert_eq!(store.get(PROJECT).unwrap().unwrap(), done3);
    }

    /// 发布与 save/ensure_bootstrap 共用 scope 锁:并发后只有合法
    /// successor 或冲突,无丢失更新、无重复候选;不同 scope 不共锁。
    #[test]
    fn recipe_policy_all_writers_share_scope_lock() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path().join("lc-root");
        std::fs::create_dir_all(&root).unwrap();
        write_root_fixture(&root);
        let canonical = std::fs::canonicalize(&root).unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_0001");
        let manifest = seed_final_step_operation(&paths, "lc_0001", "op_race", &canonical);
        let bootstrap = store.ensure_bootstrap(&manifest).unwrap();

        let (sender, receiver) = std::sync::mpsc::channel::<(
            &'static str,
            Result<Option<AggregatePolicyArtifact>, ProductStoreError>,
        )>();
        let mut handles = Vec::new();
        for index in 0..3 {
            let store = store.clone();
            let bootstrap = bootstrap.clone();
            let sender = sender.clone();
            handles.push(std::thread::spawn(move || {
                let candidate = bootstrap.with_revised_policy(
                    format!("并发修订 {index}\n"),
                    format!("2026-10-03T09:00:0{index}Z"),
                );
                sender
                    .send(("save", store.save(PROJECT, &candidate).map(|()| None)))
                    .unwrap();
            }));
        }
        for _ in 0..2 {
            let store = store.clone();
            let manifest = manifest.clone();
            let canonical = canonical.clone();
            let sender = sender.clone();
            handles.push(std::thread::spawn(move || {
                let published = store
                    .publish_recipe_policy(&manifest, "op_race", &canonical, AT_0930.to_string())
                    .map(Some);
                sender.send(("publish", published)).unwrap();
            }));
        }
        for _ in 0..2 {
            let store = store.clone();
            let manifest = manifest.clone();
            let sender = sender.clone();
            handles.push(std::thread::spawn(move || {
                sender
                    .send(("bootstrap", store.ensure_bootstrap(&manifest).map(Some)))
                    .unwrap();
            }));
        }
        drop(sender);
        for handle in handles {
            handle.join().expect("writer thread must not panic");
        }

        let mut save_wins = 0;
        let mut published: Option<AggregatePolicyArtifact> = None;
        for (kind, result) in receiver.into_iter() {
            match kind {
                "save" => match result {
                    Ok(_) => save_wins += 1,
                    Err(
                        ProductStoreError::InvalidRecord { .. }
                        | ProductStoreError::IdentityMismatch { .. },
                    ) => {}
                    Err(other) => panic!("save 只允许合法 successor 或冲突:{other}"),
                },
                "publish" => {
                    let artifact = result
                        .expect("同 operation 并发发布必须重入复用成功")
                        .expect("publish 结果必须携带 artifact");
                    if let Some(previous) = &published {
                        assert_eq!(previous, &artifact, "不得产生重复候选");
                    } else {
                        published = Some(artifact);
                    }
                }
                "bootstrap" => {
                    result.expect("ensure_bootstrap 并发必须幂等成功");
                }
                _ => unreachable!(),
            }
        }
        assert!(save_wins <= 1, "revision 2 只能被一个 save 赢得");
        let current = store.get(PROJECT).unwrap().unwrap();
        assert!(current.revision == 2 || current.revision == 3);
        let output = store
            .get_recipe_policy_publication(PROJECT, "op_race")
            .unwrap()
            .expect("唯一发布输出");
        assert_eq!(output.artifact, current, "发布候选与 current 一致");

        // 不同 scope 不共锁:持 A scope 锁时 B scope 仍可完成写入。
        let store_b = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_other");
        let root_b = temp.path().join("root-b");
        std::fs::create_dir_all(&root_b).unwrap();
        let manifest_b = LogicalCodebaseManifest::new(PROJECT, root_b, vec![]);
        let lock_a = paths
            .logical_codebases_root(PROJECT)
            .join("lc_0001")
            .join(".aggregate-policy.lock");
        let (held_tx, held_rx) = std::sync::mpsc::channel::<()>();
        let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
        let blocker = std::thread::spawn(move || {
            crate::product::coding_attempt_store::locking::with_exact_exclusive_lock(
                &lock_a,
                || {
                    held_tx.send(()).unwrap();
                    release_rx.recv().unwrap();
                    Ok::<(), ProductStoreError>(())
                },
            )
            .expect("scope A lock holder must succeed");
        });
        held_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("scope A 锁必须被持有");
        let (b_tx, b_rx) = std::sync::mpsc::channel();
        let ensure_b = std::thread::spawn(move || {
            let artifact = store_b.ensure_bootstrap(&manifest_b).unwrap();
            b_tx.send(artifact).unwrap();
        });
        let b_artifact = b_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("不同 scope 不得共享锁");
        ensure_b.join().unwrap();
        release_tx.send(()).unwrap();
        blocker.join().unwrap();
        assert_eq!(b_artifact.revision, 1);
    }

    /// 桩 digest 常量与精确谓词稳定:常量等于桩原字节独立 SHA-256;
    /// bootstrap 命中;关键词/换行变体/真正文 revision 1 不命中;不随 revision 漂移。
    #[test]
    fn recipe_policy_bootstrap_digest_and_exact_predicate_are_stable() {
        assert_eq!(
            BOOTSTRAP_POLICY_DIGEST,
            sha256_hex_of(BOOTSTRAP_POLICY_TEXT.as_bytes())
        );

        let placeholder =
            AggregatePolicyArtifact::bootstrap("p1", "l1", "2026-10-03T00:00:00Z".into());
        assert!(placeholder.is_bootstrap_placeholder());

        let keyword_only = placeholder.with_revised_policy(
            "Allow planning read-only and coding target-write sessions under the logical codebase.\n",
            "t",
        );
        assert!(!keyword_only.is_bootstrap_placeholder());

        let crlf_variant =
            placeholder.with_revised_policy(BOOTSTRAP_POLICY_TEXT.replace('\n', "\r\n"), "t");
        assert!(!crlf_variant.is_bootstrap_placeholder());

        let real_text = "# 聚合政策\n\n## 来源：AGENTS.md\n\n真实根规则\n\n";
        let real = AggregatePolicyArtifact {
            policy_id: "policy/p1/l1/1".to_string(),
            project_id: "p1".to_string(),
            logical_codebase_id: "l1".to_string(),
            revision: 1,
            digest: sha256_hex_of(real_text.as_bytes()),
            policy_text: real_text.to_string(),
            created_at: "t".to_string(),
        };
        assert!(!real.is_bootstrap_placeholder());

        // 桩判定不比较 revision/年龄:同一桩正文在任何 revision 都命中。
        let mut at_five = placeholder;
        at_five.revision = 5;
        at_five.policy_id = "policy/p1/l1/5".to_string();
        assert!(at_five.is_bootstrap_placeholder());
    }
}
