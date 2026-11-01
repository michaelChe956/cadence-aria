    // ---- C2 Task 10：受限政策读取与重新授权（REQ-ENV-C2-POLICY，#17／BYPASS-17）----

    use crate::product::logical_codebase::policy::{
        AggregatePolicyArtifact, AggregatePolicyArtifactStore,
    };
    // 应用层服务（PolicyAccessError／PolicyReauthorizationRequest／
    // PolicyTextQueryInput／read_policy_text_for_attempt／
    // handle_policy_reauthorization／handle_policy_text_query／
    // load_policy_verification_waiting_fact）定义于本模块，经 `super::*` 可见。

    struct PolicyFixture {
        _tmp: TempDir,
        paths: ProductAppPaths,
        project_id: String,
        issue_id: String,
        attempt: CodingExecutionAttempt,
        policy: AggregatePolicyArtifact,
        worktree: PathBuf,
    }

    fn policy_attempt_record_path(paths: &ProductAppPaths, fx: &PolicyFixture) -> PathBuf {
        paths
            .issue_lifecycle_root(&fx.project_id, &fx.issue_id)
            .join("coding-attempts")
            .join(format!("{}.json", fx.attempt.id))
    }

    fn policy_attempt_partition(paths: &ProductAppPaths, fx: &PolicyFixture) -> PathBuf {
        paths
            .issue_lifecycle_root(&fx.project_id, &fx.issue_id)
            .join("coding-attempts")
            .join(&fx.attempt.id)
    }

    /// LC 归属 attempt 政策 fixture：project + LC + manifest + bootstrap
    /// policy artifact + issue 归属 + attempt（target_snapshot 冻结
    /// policy.digest）。`frozen_digest_override` 模拟 envelope 冻结 digest 与
    /// 当前 artifact 不一致；`with_lc=false` 模拟单仓/无归属 attempt。
    fn policy_setup(
        status: CodingAttemptStatus,
        frozen_digest_override: Option<&str>,
        with_lc: bool,
        with_worktree: bool,
    ) -> PolicyFixture {
        let tmp = TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let project_id = crate::product::project_store::ProjectStore::new(paths.clone())
            .create(crate::product::project_store::CreateProjectInput {
                name: "policy-project".to_string(),
                description: None,
            })
            .unwrap()
            .id;

        let mut policy = None;
        let mut logical_codebase_id = None;
        if with_lc {
            let root = tmp.path().join("aggregate-root");
            std::fs::create_dir_all(&root).unwrap();
            let lc = crate::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
                .create(
                    &project_id,
                    crate::product::logical_codebase::LogicalCodebaseCreateInput {
                        name: "policy-lc".to_string(),
                        aggregate_root: root,
                    },
                )
                .unwrap();
            let manifest = LogicalCodebaseManifest::new(
                &project_id,
                tmp.path().join("aggregate-root"),
                Vec::new(),
            );
            crate::product::logical_codebase::LogicalCodebaseStore::for_lc(
                paths.clone(),
                &lc.id,
            )
            .save_manifest(&project_id, &manifest)
            .unwrap();
            let artifact = AggregatePolicyArtifact::bootstrap(
                &project_id,
                &manifest.logical_codebase_id.to_string(),
                "2026-09-29T00:00:00Z".to_string(),
            );
            AggregatePolicyArtifactStore::for_lc(paths.clone(), &lc.id)
                .save(&project_id, &artifact)
                .unwrap();
            policy = Some(artifact);
            logical_codebase_id = Some(lc.id);
        }

        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(crate::product::issue_store::CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id,
                title: "policy issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();

        let artifact = policy.unwrap_or_else(|| {
            AggregatePolicyArtifact::bootstrap(
                &project_id,
                "unused_logical_codebase",
                "2026-09-29T00:00:00Z".to_string(),
            )
        });
        let frozen_digest =
            frozen_digest_override.unwrap_or(&artifact.digest).to_string();
        let worktree = tmp.path().join("worktree");
        if with_worktree {
            std::fs::create_dir_all(&worktree).unwrap();
        }
        let attempt = CodingExecutionAttempt {
            id: "coding_attempt_policy".to_string(),
            project_id: project_id.clone(),
            issue_id: issue.id.clone(),
            work_item_id: "work_item_policy".to_string(),
            attempt_no: 1,
            scope: CodingAttemptScope::WorkItem,
            status,
            version: 3,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            admission_kind: crate::product::coding_models::CodingAdmissionKind::LegacyGroup,
            stage: CodingExecutionStage::Coding,
            base_branch: "main".to_string(),
            branch_name: format!("aria/issues/{}", issue.id),
            worktree_path: with_worktree.then(|| worktree.clone()),
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            rework_count: 0,
            max_auto_rework: 0,
            work_item_group_id: None,
            current_work_item_id: Some("work_item_policy".to_string()),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-09-29T00:00:00Z".to_string(),
            updated_at: "2026-09-29T00:00:00Z".to_string(),
            target_snapshot: Some(AttemptTargetSnapshot {
                logical_repository_id: LogicalRepositoryId(Uuid::new_v4()),
                checkout_id: RepositoryCheckoutId(Uuid::new_v4()),
                physical_repository_id: "repo_policy".to_string(),
                canonical_path: tmp.path().join("repo"),
                git_dir_identity: "git-dir-policy".to_string(),
                revision: Some("rev-policy".to_string()),
                policy_digest: frozen_digest,
                membership_revision: 1,
                captured_at: "2026-09-29T00:00:00Z".to_string(),
                capture_source: "test".to_string(),
            }),
            start_run_policy: crate::product::coding_models::CodingStartRunPolicy::Manual,
            start_claim: None,
            completed_at: None,
        };
        write_json(
            &paths
                .issue_lifecycle_root(&project_id, &issue.id)
                .join("coding-attempts")
                .join(format!("{}.json", attempt.id)),
            &attempt,
        )
        .expect("write attempt record");

        PolicyFixture {
            _tmp: tmp,
            paths,
            project_id,
            issue_id: issue.id,
            attempt,
            policy: artifact,
            worktree,
        }
    }

    fn rewrite_policy_attempt_status(fx: &PolicyFixture, status: CodingAttemptStatus) {
        let mut attempt = fx.attempt.clone();
        attempt.status = status;
        write_json(&policy_attempt_record_path(&fx.paths, fx), &attempt)
            .expect("rewrite attempt status");
    }

    #[test]
    fn page_policy_read_returns_frozen_digest_text_without_host_paths() {
        let fx = policy_setup(CodingAttemptStatus::Blocked, None, true, false);

        let result = read_policy_text_for_attempt(
            &fx.paths,
            &fx.project_id,
            &fx.issue_id,
            &fx.attempt.id,
        )
        .expect("page policy read succeeds");
        assert_eq!(result.text, fx.policy.policy_text);
        assert_eq!(result.policy_digest, fx.policy.digest);
        assert_eq!(result.policy_revision, fx.policy.revision);

        // 不暴露宿主绝对路径：结果序列化里不得出现 tempdir 路径。
        let rendered = serde_json::to_string(&result).expect("serialize result");
        assert!(
            !rendered.contains(fx._tmp.path().to_string_lossy().as_ref()),
            "policy read result must not leak host absolute paths"
        );
    }

    #[test]
    fn policy_reauthorization_binds_next_rework_run_and_running_read_matches_frozen_digest() {
        let fx = policy_setup(CodingAttemptStatus::Blocked, None, true, true);
        let request = PolicyReauthorizationRequest {
            command_id: "cmd_policy_reauth_0001".to_string(),
            attempt_id: fx.attempt.id.clone(),
            role: "coder".to_string(),
            policy_digest: fx.policy.digest.clone(),
            expected_version: 3,
        };
        let issued = handle_policy_reauthorization(&fx.paths, &fx.project_id, &fx.issue_id, &request)
            .expect("reauthorization issues");
        assert!(issued.expires_at.is_some());
        assert_eq!(
            issued.state,
            crate::product::models::automation::OperationState::Accepted
        );

        // 幂等重放：同 command 同 payload → Replayed，令牌文件不被旋转。
        let token_path = fx.worktree.join(".aria").join("evidence-token");
        let token_first = std::fs::read_to_string(&token_path).expect("worktree token file");
        let replayed =
            handle_policy_reauthorization(&fx.paths, &fx.project_id, &fx.issue_id, &request)
                .expect("replay succeeds");
        assert_eq!(
            replayed.state,
            crate::product::models::automation::OperationState::Replayed
        );
        assert_eq!(
            std::fs::read_to_string(&token_path).expect("token file after replay"),
            token_first,
            "replay must not rotate the issued token"
        );

        // 下一次返修 run（Running）以该令牌读取政策：正文 digest 与 envelope
        // 冻结 digest 一致，返修继续。
        rewrite_policy_attempt_status(&fx, CodingAttemptStatus::Running);
        let result = handle_policy_text_query(
            &fx.paths,
            &PolicyTextQueryInput {
                token: token_first.clone(),
                role: EvidenceRole::Coder,
            },
        )
        .expect("running read succeeds");
        assert_eq!(result.text, fx.policy.policy_text);
        assert_eq!(result.policy_digest, fx.attempt.target_snapshot.as_ref().unwrap().policy_digest);
    }

    #[test]
    fn restricted_policy_read_rejects_wrong_role_expired_or_replaced_authorization() {
        let fx = policy_setup(CodingAttemptStatus::Blocked, None, true, true);
        let issued = handle_policy_reauthorization(
            &fx.paths,
            &fx.project_id,
            &fx.issue_id,
            &PolicyReauthorizationRequest {
                command_id: "cmd_policy_reauth_0002".to_string(),
                attempt_id: fx.attempt.id.clone(),
                role: "coder".to_string(),
                policy_digest: fx.policy.digest.clone(),
                expected_version: 3,
            },
        )
        .expect("reauthorization issues");
        assert_eq!(
            issued.state,
            crate::product::models::automation::OperationState::Accepted
        );
        rewrite_policy_attempt_status(&fx, CodingAttemptStatus::Running);
        let token = std::fs::read_to_string(fx.worktree.join(".aria").join("evidence-token"))
            .expect("worktree token");

        // 错 role（reviewer 持 coder 授权读取）→ 拒绝。
        let wrong_role = handle_policy_text_query(
            &fx.paths,
            &PolicyTextQueryInput {
                token: token.clone(),
                role: EvidenceRole::Reviewer,
            },
        )
        .unwrap_err();
        assert!(matches!(wrong_role, PolicyAccessError::Forbidden { .. }));

        // 过期授权 → 拒绝（提示重新授权）。
        let record_path = policy_attempt_partition(&fx.paths, &fx)
            .join(crate::product::logical_codebase::evidence_token::EVIDENCE_TOKEN_RECORD_FILE);
        let mut record: crate::product::logical_codebase::evidence_token::EvidenceTokenRecord =
            read_json(&record_path).expect("token record");
        record.claims.as_mut().expect("claims").expires_at =
            "2020-01-01T00:00:00Z".to_string();
        write_json(&record_path, &record).expect("expire claims");
        let expired = handle_policy_text_query(
            &fx.paths,
            &PolicyTextQueryInput {
                token: token.clone(),
                role: EvidenceRole::Coder,
            },
        )
        .unwrap_err();
        assert!(matches!(expired, PolicyAccessError::Forbidden { .. }));

        // 重新签发（新 command，等待面上）替换旧授权：旧令牌立即失效 →
        // Unauthorized（哈希记录已旋转，反查无命中）。
        rewrite_policy_attempt_status(&fx, CodingAttemptStatus::Blocked);
        handle_policy_reauthorization(
            &fx.paths,
            &fx.project_id,
            &fx.issue_id,
            &PolicyReauthorizationRequest {
                command_id: "cmd_policy_reauth_0003".to_string(),
                attempt_id: fx.attempt.id.clone(),
                role: "coder".to_string(),
                policy_digest: fx.policy.digest.clone(),
                expected_version: 3,
            },
        )
        .expect("re-issue replaces authorization");
        let replaced = handle_policy_text_query(
            &fx.paths,
            &PolicyTextQueryInput {
                token: token.clone(),
                role: EvidenceRole::Coder,
            },
        )
        .unwrap_err();
        assert!(matches!(replaced, PolicyAccessError::Unauthorized));

        // 错 attempt／未知令牌 → Unauthorized。
        let unknown = handle_policy_text_query(
            &fx.paths,
            &PolicyTextQueryInput {
                token: "0".repeat(64),
                role: EvidenceRole::Coder,
            },
        )
        .unwrap_err();
        assert!(matches!(unknown, PolicyAccessError::Unauthorized));
    }

    #[test]
    fn legacy_token_without_claims_is_rejected_for_restricted_policy_read() {
        let fx = policy_setup(CodingAttemptStatus::Running, None, true, true);
        // 旧记录形态：无 claims 的普通证据令牌。
        let token = crate::product::logical_codebase::evidence_token::issue_evidence_token(
            &fx.paths,
            &fx._tmp.path().join("repo"),
            &fx.attempt,
        )
        .expect("legacy token issue");

        let rejected = handle_policy_text_query(
            &fx.paths,
            &PolicyTextQueryInput {
                token,
                role: EvidenceRole::Coder,
            },
        )
        .unwrap_err();
        // 旧令牌无受限读取授权 → 不误放行，提示重新授权。
        assert!(matches!(rejected, PolicyAccessError::Forbidden { .. }));
    }

    #[test]
    fn policy_read_resolver_unavailable_fails_closed_with_waiting_fact_and_no_fallback() {
        // 单仓/无 LC 归属 attempt：resolver 不可用 → 读取与重新授权均
        // fail-closed 落政策核验等待事实，MUST NOT 回落任何备用路径。
        let fx = policy_setup(CodingAttemptStatus::Blocked, None, false, true);

        let read = read_policy_text_for_attempt(&fx.paths, &fx.project_id, &fx.issue_id, &fx.attempt.id)
            .unwrap_err();
        assert!(matches!(read, PolicyAccessError::ResolverUnavailable { .. }));
        let waiting = load_policy_verification_waiting_fact(&fx.paths, &fx.attempt)
            .expect("waiting fact readable")
            .expect("waiting fact landed");
        assert_eq!(waiting.reason_code, "policy_resolver_unavailable");

        let reauth = handle_policy_reauthorization(
            &fx.paths,
            &fx.project_id,
            &fx.issue_id,
            &PolicyReauthorizationRequest {
                command_id: "cmd_policy_reauth_0004".to_string(),
                attempt_id: fx.attempt.id.clone(),
                role: "coder".to_string(),
                policy_digest: fx.policy.digest.clone(),
                expected_version: 3,
            },
        )
        .expect("reauthorization result");
        assert_eq!(
            reauth.state,
            crate::product::models::automation::OperationState::NeedsHuman
        );
        // 未签发任何受限读取授权（无 claims 令牌记录写入 worktree/attempt）。
        assert!(!fx
            .worktree
            .join(".aria")
            .join("evidence-token")
            .exists());
    }

    #[test]
    fn policy_read_digest_mismatch_with_envelope_fails_closed_with_waiting_fact() {
        // envelope 冻结 digest 与 resolver 当前 digest 不一致 → fail-closed。
        let fx = policy_setup(
            CodingAttemptStatus::Blocked,
            Some("sha256:frozen-digest-different-from-artifact"),
            true,
            true,
        );

        let read = read_policy_text_for_attempt(&fx.paths, &fx.project_id, &fx.issue_id, &fx.attempt.id)
            .unwrap_err();
        assert!(matches!(read, PolicyAccessError::DigestMismatch { .. }));
        let waiting = load_policy_verification_waiting_fact(&fx.paths, &fx.attempt)
            .expect("waiting fact readable")
            .expect("waiting fact landed");
        assert_eq!(waiting.reason_code, "policy_digest_mismatch");
    }

    #[test]
    fn policy_reauthorization_rejects_wrong_version_or_role_without_side_effects() {
        let fx = policy_setup(CodingAttemptStatus::Blocked, None, true, true);

        // 错版本 → Rejected（请刷新），不签发令牌、不落等待事实。
        let version_rejected = handle_policy_reauthorization(
            &fx.paths,
            &fx.project_id,
&fx.issue_id,
            &PolicyReauthorizationRequest {
                command_id: "cmd_policy_reauth_0005".to_string(),
                attempt_id: fx.attempt.id.clone(),
                role: "coder".to_string(),
                policy_digest: fx.policy.digest.clone(),
                expected_version: 99,
            },
        )
        .expect("result returned");
        assert_eq!(
            version_rejected.state,
            crate::product::models::automation::OperationState::Rejected
        );
        assert!(!fx.worktree.join(".aria").join("evidence-token").exists());
        assert!(load_policy_verification_waiting_fact(&fx.paths, &fx.attempt)
            .expect("read waiting fact")
            .is_none());

        // 非法 role → Rejected。
        let role_rejected = handle_policy_reauthorization(
            &fx.paths,
            &fx.project_id,
            &fx.issue_id,
            &PolicyReauthorizationRequest {
                command_id: "cmd_policy_reauth_0006".to_string(),
                attempt_id: fx.attempt.id.clone(),
                role: "root".to_string(),
                policy_digest: fx.policy.digest.clone(),
                expected_version: 3,
            },
        )
        .expect("result returned");
        assert_eq!(
            role_rejected.state,
            crate::product::models::automation::OperationState::Rejected
        );
    }

    #[test]
    fn policy_read_and_reauthorization_never_write_policy_text_or_paths_into_context_notes() {
        let fx = policy_setup(CodingAttemptStatus::Blocked, None, true, true);
        // 成功路径 + fail-closed 路径都执行一遍。
        let _ = read_policy_text_for_attempt(&fx.paths, &fx.project_id, &fx.issue_id, &fx.attempt.id)
            .expect("read succeeds");
        let no_lc = policy_setup(CodingAttemptStatus::Blocked, None, false, true);
        let _ = read_policy_text_for_attempt(
            &no_lc.paths,
            &no_lc.project_id,
            &no_lc.issue_id,
            &no_lc.attempt.id,
        )
        .unwrap_err();

        for fixture in [&fx, &no_lc] {
            let notes_root = policy_attempt_partition(&fixture.paths, fixture).join("context-notes");
            if !notes_root.exists() {
                continue;
            }
            for entry in std::fs::read_dir(&notes_root).expect("read context-notes") {
                let path = entry.expect("dir entry").path();
                let content = std::fs::read_to_string(&path).expect("note content");
                assert!(
                    !content.contains(&fixture.policy.policy_text),
                    "policy text must never land in context notes"
                );
                assert!(
                    !content.contains("aggregate-policy.json"),
                    "policy artifact path must never land in context notes"
                );
            }
        }
    }
