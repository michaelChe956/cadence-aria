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

    pub(crate) const PROJECT: &str = "project_0001";

    /// 统一的测试时间戳常量与构造器。
    pub(crate) const AT_0800: &str = "2026-10-03T08:00:00Z";
    pub(super) const AT_0830: &str = "2026-10-03T08:30:00Z";
    pub(crate) const AT_0900: &str = "2026-10-03T09:00:00Z";
    pub(super) const AT_0930: &str = "2026-10-03T09:30:00Z";
    pub(crate) const AT_1000: &str = "2026-10-03T10:00:00Z";
    pub(super) const AT_1100: &str = "2026-10-03T11:00:00Z";
    pub(super) const AT_1200: &str = "2026-10-03T12:00:00Z";
    pub(super) const AT_1300: &str = "2026-10-03T13:00:00Z";
    pub(super) const AT_1400: &str = "2026-10-03T14:00:00Z";

    fn ts(clock: &str) -> String {
        format!("2026-10-03T{clock}:00Z")
    }

    pub(crate) fn sha256_hex_of(bytes: &[u8]) -> String {
        format!("sha256:{:x}", Sha256::digest(bytes))
    }

    /// 根规则 fixture:入口 + 排序规则集 + 各类干扰材料(provider 副本/
    /// 凭据/模板/成员规则/非 md/其他规则目录),期望正文由
    /// [`expected_policy_text`] 逐字节钉定。
    pub(crate) fn write_root_fixture(root: &Path) {
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
    pub(super) fn expected_policy_text() -> String {
        let mut expected = String::from("# 聚合政策\n\n");
        for (relative_path, content) in [
            (
                "AGENTS.md",
                "# 根规则\r\n\r\n规则正文见 .claude/rules/ 目录。\r\n",
            ),
            ("CLAUDE.md", "provider 兼容副本不重复激活\n"),
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
    pub(crate) fn seed_final_step_operation(
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
    pub(crate) fn seed_operation_for_manifest(
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
    pub(crate) fn publisher(
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
}
