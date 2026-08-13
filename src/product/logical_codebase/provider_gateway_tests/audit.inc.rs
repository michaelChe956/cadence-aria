// 从 provider_gateway_tests.rs 拆出的 audit/managed-settings 测试段
// （large_file_guard 1200 行红线，T11 fix round 2）。共享 mod task13_gateway_hardening 作用域。
    /// 配置来源审计:`ConfigSourceAudit` 记录最终 argv 与 config digest,且
    /// argv 非空、config digest 与 envelope 冻结值一致。仅 Aria-owned
    /// (user/project/local/env/mcp)来源被标注;非 Aria 来源(如 managed settings)
    /// 被标注为 `managed_settings_active=true` 并携带警告,绝不假装已覆盖。
    #[test]
    fn config_source_audit_records_argv_config_digest_and_provenance() {
        let audit = ConfigSourceAudit::from_launch(
            &[
                "claude".to_string(),
                "--permission-prompt-tool=stdio".to_string(),
            ],
            "sha256:managed-config-artifact",
            ConfigSourceProvenance {
                user_settings: true,
                project_settings: true,
                local_settings: false,
                env_overrides: true,
                managed_settings_active: false,
                managed_settings_warning: None,
                mcp_sources: vec![ConfigSourceKind::AriaOwnedBundle],
            },
        );

        assert_eq!(audit.argv, vec!["claude", "--permission-prompt-tool=stdio"]);
        assert!(audit.config_digest.starts_with("sha256:"));
        assert!(!audit.provenance.managed_settings_active);
        assert!(audit.provenance.is_aria_owned_only());
    }

    /// 配置来源审计:解析 provider `/status` 的 `Setting sources` 时发现 managed
    /// settings(非 Aria-owned),标注 `managed_settings_active=true` + 警告,且
    /// `is_aria_owned_only()` 返回 false(绝不假装已覆盖)。该已知 gap 仍可被
    /// policy 配置为拒绝启动(`ManagedSettingsActive` 错误)。
    #[test]
    fn config_source_audit_flags_managed_settings_without_pretending_override() {
        let provenance =
            ConfigSourceProvenance::detect_from_setting_sources(&["User", "Project", "Managed"]);
        assert!(provenance.managed_settings_active);
        assert!(
            provenance
                .managed_settings_warning
                .as_ref()
                .is_some_and(|warning| warning.contains("managed settings")
                    && !warning.contains("overridden")
                    && !warning.contains("覆盖")),
            "warning must not claim override: {:?}",
            provenance.managed_settings_warning
        );
        assert!(!provenance.is_aria_owned_only());
    }

    /// 配置来源审计(Task 11 语义修正):managed settings 不再 fail-closed,而是
    /// 在 `GatewayRunAudit` 追加标注(携带 config digest)后放行。`enforce_config_source_policy`
    /// 返回 `Ok`,且 `managed_settings_annotations` 记录该已知 gap——绝不假装已覆盖,
    /// 但不阻断启动。
    #[test]
    fn gateway_annotates_managed_settings_active_without_blocking() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let request = SessionLaunchRequest::planning(
            fixture.manifest().project_id,
            ProviderRef::claude_code("cap_claude_code_1_4_0"),
            PolicyTarget::checkout("logical_repo_0001", "checkout_0001", worktree),
            vec![fixture.paths.root().to_path_buf()],
            "sha256:managed-config-artifact",
        );
        let validated = fixture.gateway().validate(request).unwrap();

        let provenance = ConfigSourceProvenance::detect_from_setting_sources(&["User", "Managed"]);
        let audit = ConfigSourceAudit::from_launch(
            &["claude".to_string()],
            "sha256:managed-config-artifact",
            provenance,
        );
        let gateway = fixture.gateway();
        gateway
            .enforce_config_source_policy(&validated, &audit)
            .expect("managed settings must be annotated, not blocked");

        let annotations = fixture.gateway_audit().managed_settings_annotations();
        assert!(
            !annotations.is_empty(),
            "managed settings must be annotated in the audit"
        );
        assert!(
            annotations
                .iter()
                .any(|annotation| annotation.contains("managed settings")),
            "annotation must mention managed settings: {annotations:?}"
        );
    }

    /// Task 11 审计聚合:`start_streaming` 成功启动后,audit entry 冻结该次启动的
    /// config digest(Some)与最终 argv(无真实 argv 源,应为空 Vec)。
    #[tokio::test]
    async fn start_streaming_audit_entry_carries_config_digest_and_argv() {
        let fixture = gateway_fixture();
        fixture.install_bootstrap_policy();
        let worktree = fixture.real_worktree();
        let launch = fixture.validated_planning_streaming_input(worktree.clone());

        fixture
            .gateway()
            .start_streaming(launch, CancellationToken::new())
            .await
            .expect("streaming launch");

        let audit = fixture.gateway_audit();
        let entries = audit.entries.lock().unwrap();
        let entry = entries
            .iter()
            .find(|entry| entry.stack == GatewayRunStack::Stream)
            .expect("stream entry");
        assert!(
            entry.config_digest.is_some(),
            "config digest must be recorded: {entry:?}"
        );
        assert!(
            entry.argv.is_empty(),
            "no real argv source in launch inputs, expected empty argv: {:?}",
            entry.argv
        );
    }
