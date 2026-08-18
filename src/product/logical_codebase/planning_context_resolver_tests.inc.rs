#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
    };
    use crate::product::logical_codebase::planning_context_set::InventoryInjectionBudget;
    use crate::product::logical_codebase::policy::{
        AggregatePolicyArtifactStore, PolicyTarget, SessionPolicyAction,
    };
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
        IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore, MemberStatus,
        RepositoryCheckoutId, RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
    };
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex};
    use tempfile::TempDir;
    use uuid::Uuid;

    /// 稳定 UUID：禁止运行时随机，保证测试可复现；ID 组成磁盘路径前受
    /// `validate_relative_id` 约束（本测试使用 `project_0001` / `issue_0001` 等稳定 id）。
    const API_MEMBER_UUID: Uuid = stable_uuid(0x0001);

    const fn stable_uuid(seed: u16) -> Uuid {
        let mut bytes = [0u8; 16];
        bytes[14] = (seed >> 8) as u8;
        bytes[15] = seed as u8;
        // version 7 + variant 10xx，满足 Uuid::from_bytes 的合法构造。
        bytes[6] = 0x70;
        bytes[8] = 0x80;
        Uuid::from_bytes(bytes)
    }

    struct ResolverFixture {
        // 保留 temp 以持有临时目录生命周期；paths 派生自 temp.path()。
        #[allow(dead_code)]
        temp: TempDir,
        paths: ProductAppPaths,
        api_member_id: LogicalRepositoryId,
        cached_policy_digest: Option<String>,
    }

    impl ResolverFixture {
        fn resolver(&self) -> PlanningContextResolver {
            PlanningContextResolver::new(self.paths.clone())
        }

        fn aggregate_root(&self) -> PathBuf {
            self.temp.path().join("aggregate-root")
        }

        /// planning 只读启动使用的 provider ref。与 gateway `ProviderRef` 对齐,
        /// planning 走 ClaudeCode(Codex danger-full-access 在 gateway 路由级被阻断)。
        fn provider_ref(&self) -> crate::product::logical_codebase::provider_gateway::ProviderRef {
            crate::product::logical_codebase::provider_gateway::ProviderRef::claude_code(
                "cap_claude_code_1_4_0",
            )
        }

        /// planning 启动携带的托管配置 artifact 引用(envelope 冻结其 digest)。
        fn config_artifact_ref(&self) -> String {
            "sha256:managed-config-artifact".to_string()
        }

        fn membership_revision(&self) -> u64 {
            1
        }

        fn policy_digest(&self) -> String {
            self.cached_policy_digest
                .clone()
                .expect("write_active_manifest_index_and_policy must run first")
        }

        /// 写入单成员 manifest（api，active）+ 显式 selection(issue_0001 → api) +
        /// active aggregate index（membership_revision 与 manifest 对齐）+ 政策
        /// bootstrap artifact，覆盖 resolver 的所有必读依赖。
        fn write_active_manifest_index_and_policy(&mut self) {
            let store = LogicalCodebaseStore::new(self.paths.clone());
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.aggregate_root(),
                vec![self.api_member_id],
            );
            store.save_manifest("project_0001", &manifest).unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(self.api_member_id, "api", MemberStatus::Active),
                )
                .unwrap();
            store
                .save_checkout("project_0001", &self.api_checkout())
                .unwrap();

            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![self.api_member_id],
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();

            // active aggregate index：成员快照与 api member 对齐。
            let index = active_index_record("project_0001", self.api_member_id);
            AggregateIndexStore::new(self.paths.clone())
                .create("project_0001", index.clone())
                .unwrap();
            let mut activated = index.clone();
            activated.status = AggregateIndexStatus::Active;
            AggregateIndexStore::new(self.paths.clone())
                .replace_active("project_0001", activated)
                .unwrap();

            // 政策 bootstrap artifact。
            let policy = AggregatePolicyArtifactStore::new(self.paths.clone())
                .ensure_bootstrap(&manifest)
                .unwrap();
            self.cached_policy_digest = Some(policy.digest);
        }

        /// 写入一个 issue（issue_empty）的显式空 selection，使 resolver 对该 issue
        /// 解析出空有效成员集合，触发 fail-closed blocker。
        fn write_selection_with_no_effective_members(&self) {
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_empty",
                Vec::new(),
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();
        }

        /// 两成员场景：api（active，在 manifest.member_ids）+ web（status=Removed，不在
        /// manifest.member_ids）。selection 显式 include 两者 → api 有效、web 失效，
        /// 触发 selection/snapshot 失效标记（REQ-PLN-02）。active aggregate index 与政策
        /// artifact 覆盖 resolver 其余必读依赖。
        fn write_active_manifest_index_and_policy_with_removed_member(&mut self) {
            let web = LogicalRepositoryId(stable_uuid(0x0002));
            let store = LogicalCodebaseStore::new(self.paths.clone());
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.aggregate_root(),
                vec![self.api_member_id],
            );
            store.save_manifest("project_0001", &manifest).unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(self.api_member_id, "api", MemberStatus::Active),
                )
                .unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(web, "web", MemberStatus::Removed),
                )
                .unwrap();
            store
                .save_checkout("project_0001", &self.api_checkout())
                .unwrap();

            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![self.api_member_id, web],
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();

            // active aggregate index：成员快照仅含 api（有效成员），web 为失效成员。
            let index = active_index_record("project_0001", self.api_member_id);
            AggregateIndexStore::new(self.paths.clone())
                .create("project_0001", index.clone())
                .unwrap();
            let mut activated = index.clone();
            activated.status = AggregateIndexStatus::Active;
            AggregateIndexStore::new(self.paths.clone())
                .replace_active("project_0001", activated)
                .unwrap();

            let policy = AggregatePolicyArtifactStore::new(self.paths.clone())
                .ensure_bootstrap(&manifest)
                .unwrap();
            self.cached_policy_digest = Some(policy.digest);
        }

        /// 真实 tombstone 语义（Plan 1 `apply_delete_tombstone`）：web 仍留在
        /// manifest.member_ids 但 status=Tombstoned；selection 只 include api。resolver
        /// 必须按 Active 过滤使 web 进入 invalid → selection/snapshot 失效 → resume
        /// 强制 StaleContext。
        fn write_active_manifest_index_and_policy_with_tombstoned_member_in_manifest(&mut self) {
            let web = LogicalRepositoryId(stable_uuid(0x0002));
            let store = LogicalCodebaseStore::new(self.paths.clone());
            let manifest = LogicalCodebaseManifest::new(
                "project_0001",
                self.aggregate_root(),
                // web 仍在 manifest（tombstone 不修改 manifest.member_ids）。
                vec![self.api_member_id, web],
            );
            store.save_manifest("project_0001", &manifest).unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(self.api_member_id, "api", MemberStatus::Active),
                )
                .unwrap();
            store
                .save_member(
                    "project_0001",
                    &self.member_record(web, "web", MemberStatus::Tombstoned),
                )
                .unwrap();
            store
                .save_checkout("project_0001", &self.api_checkout())
                .unwrap();

            // selection 不含删除成员：失效由 manifest 不一致兑底触发。
            let selection = IssueCodebaseSelection::explicit(
                "project_0001",
                "issue_0001",
                vec![self.api_member_id],
                Vec::new(),
                Vec::new(),
                None,
            );
            IssueCodebaseSelectionStore::new(self.paths.clone())
                .save(&selection)
                .unwrap();

            // active aggregate index：成员快照仅含 api（有效成员），web 为失效成员。
            let index = active_index_record("project_0001", self.api_member_id);
            AggregateIndexStore::new(self.paths.clone())
                .create("project_0001", index.clone())
                .unwrap();
            let mut activated = index.clone();
            activated.status = AggregateIndexStatus::Active;
            AggregateIndexStore::new(self.paths.clone())
                .replace_active("project_0001", activated)
                .unwrap();

            let policy = AggregatePolicyArtifactStore::new(self.paths.clone())
                .ensure_bootstrap(&manifest)
                .unwrap();
            self.cached_policy_digest = Some(policy.digest);
        }

        /// 模拟成员变更：manifest membership_revision 1 → 2 并同步推进 active aggregate
        /// index 的 membership_revision 与成员 checkout revision，使 planning snapshot
        /// 指纹漂移（membership/index/checkout 任一变化都会改变 access_fingerprint）。
        /// 与 `write_active_manifest_index_and_policy` 保持同一项目数据，供 resume 测试使用。
        fn change_membership_revision(&self) {
            let store = LogicalCodebaseStore::new(self.paths.clone());
            let mut manifest = store.load_manifest("project_0001").unwrap().unwrap();
            manifest.membership_revision = 2;
            store.save_manifest("project_0001", &manifest).unwrap();

            let index_store = AggregateIndexStore::new(self.paths.clone());
            let mut index = index_store.active("project_0001").unwrap().unwrap();
            index.membership_revision = 2;
            for snapshot in &mut index.member_snapshots {
                snapshot.revision = "def456".to_string();
            }
            index.updated_at = "2026-08-10T01:00:00Z".to_string();
            index_store.replace_active("project_0001", index).unwrap();
        }

        /// 模拟 checkout identity 更换：仅变更 active aggregate index 成员的
        /// checkout_id（revision/dirty/availability/membership 均不变）。B2 验证
        /// checkout_id 参与指纹哈希，避免 checkout 更换被漂移检测绕过。
        fn change_checkout_id(&self) {
            let index_store = AggregateIndexStore::new(self.paths.clone());
            let mut index = index_store.active("project_0001").unwrap().unwrap();
            for snapshot in &mut index.member_snapshots {
                snapshot.checkout_id = RepositoryCheckoutId(stable_uuid(0x0002));
            }
            index.updated_at = "2026-08-10T02:00:00Z".to_string();
            index_store.replace_active("project_0001", index).unwrap();
        }

        fn member_record(
            &self,
            id: LogicalRepositoryId,
            alias: &str,
            status: MemberStatus,
        ) -> CodebaseMemberRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            let checkout_path = self.aggregate_root().join(alias);
            CodebaseMemberRecord {
                logical_repository_id: id,
                physical_repository_id: format!("repository_{alias}"),
                alias: alias.to_string(),
                role: "service".to_string(),
                ordinal: 1,
                source_identity: RepositorySourceIdentity::from_git_parts(
                    &checkout_path,
                    checkout_path.join(".git"),
                    Some(format!("ssh://git@example.test/acme/{alias}.git")),
                ),
                repo_type: RepositoryType::Backend,
                tech_stack: vec!["rust".to_string()],
                owner: None,
                tags: Vec::new(),
                default_ref: None,
                checkout_ids: vec![RepositoryCheckoutId(Uuid::nil())],
                status,
                created_at: now.clone(),
                updated_at: now,
            }
        }

        fn api_checkout(&self) -> RepositoryCheckoutRecord {
            let now = "2026-08-10T00:00:00Z".to_string();
            RepositoryCheckoutRecord {
                checkout_id: RepositoryCheckoutId(Uuid::nil()),
                logical_repository_id: self.api_member_id,
                physical_repository_id: "repository_api".to_string(),
                kind: CheckoutKind::Main,
                canonical_path: self.aggregate_root().join("api"),
                checkout_path_hash: "sha256:checkout".to_string(),
                git_dir_identity: "sha256:git-dir".to_string(),
                revision: Some("abc123".to_string()),
                availability: CheckoutAvailability::Available,
                observed_at: now.clone(),
                created_at: now.clone(),
                updated_at: now,
            }
        }
    }

    fn active_index_record(
        project_id: &str,
        member_id: LogicalRepositoryId,
    ) -> AggregateIndexRecord {
        let now = "2026-08-10T00:00:00Z".to_string();
        AggregateIndexRecord::building(
            "aggregate_index_0001".to_string(),
            project_id.to_string(),
            1,
            vec![AggregateIndexMemberSnapshot::indexed(
                member_id,
                RepositoryCheckoutId(Uuid::nil()),
                "abc123".to_string(),
                false,
                now,
            )],
            "2026-08-10T00:00:00Z".to_string(),
        )
    }

    struct ScriptedFreshness {
        store: AggregateIndexStore,
        next: Arc<Mutex<Vec<AggregateIndexFreshness>>>,
        sync_count: Arc<Mutex<usize>>,
    }

    impl PlanningIndexFreshness for ScriptedFreshness {
        fn assess(&self, project_id: &str) -> Result<AggregateIndexFreshness, AggregateIndexError> {
            if let Some(assessment) = self.next.lock().unwrap().pop() {
                return Ok(assessment);
            }
            let record = self.store.active_required(project_id)?;
            Ok(AggregateIndexFreshness::active(record))
        }

        fn sync_if_stale(&self, project_id: &str) -> Result<AggregateIndexRecord, AggregateIndexError> {
            *self.sync_count.lock().unwrap() += 1;
            let mut record = self.store.active_required(project_id)?;
            record.aggregate_index_id = "aggregate_index_fresh".to_string();
            record.status = AggregateIndexStatus::Active;
            self.store.replace_active(project_id, record.clone())?;
            Ok(record)
        }
    }

    fn resolver_fixture() -> ResolverFixture {
        let temp = tempfile::tempdir().unwrap();
        ResolverFixture {
            paths: ProductAppPaths::new(temp.path()),
            temp,
            api_member_id: LogicalRepositoryId(API_MEMBER_UUID),
            cached_policy_digest: None,
        }
    }

    #[tokio::test]
    async fn stale_planning_read_syncs_then_builds_but_degraded_only_carries_warning() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        let store = AggregateIndexStore::new(fixture.paths.clone());
        let stale = store.active_required("project_0001").unwrap();
        let sync_count = Arc::new(Mutex::new(0));
        let resolver = PlanningContextResolver::with_freshness_service(
            fixture.paths.clone(),
            Arc::new(ScriptedFreshness {
                store: store.clone(),
                next: Arc::new(Mutex::new(vec![AggregateIndexFreshness::stale(
                    stale.clone(),
                    "fixture_stale",
                )])),
                sync_count: sync_count.clone(),
            }),
        );
        let resolved = resolver
            .build_with_fresh_index("project_0001", "issue_0001", &[])
            .await
            .unwrap();
        assert_eq!(resolved.aggregate_index_id, "aggregate_index_fresh");
        assert_eq!(*sync_count.lock().unwrap(), 1);

        let warning = "sync command failed";
        let active_id = store
            .active_required("project_0001")
            .unwrap()
            .aggregate_index_id;
        let degraded = store
            .mark_status(
                "project_0001",
                &active_id,
                AggregateIndexStatus::Degraded,
                Some(warning.to_string()),
            )
            .unwrap();
        let degraded_resolver = PlanningContextResolver::with_freshness_service(
            fixture.paths.clone(),
            Arc::new(ScriptedFreshness {
                store: store.clone(),
                next: Arc::new(Mutex::new(vec![AggregateIndexFreshness::degraded(
                    degraded,
                    warning,
                )])),
                sync_count: sync_count.clone(),
            }),
        );
        let degraded_context = degraded_resolver
            .build_with_fresh_index("project_0001", "issue_0001", &[])
            .await
            .unwrap();
        assert!(degraded_context
            .inventory_injection
            .rendered
            .contains("aggregate index warning: sync command failed"));
        assert_eq!(*sync_count.lock().unwrap(), 1);
    }

    #[test]
    fn resolver_produces_single_snapshot_cwd_and_inventory_for_all_artifacts() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();

        let resolved = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();
        assert_eq!(resolved.cwd, fixture.aggregate_root());
        assert_eq!(
            resolved.snapshot.membership_revision,
            fixture.membership_revision()
        );
        assert_eq!(resolved.snapshot.policy_digest, fixture.policy_digest());
        assert!(
            resolved.inventory_injection.rendered.len()
                <= InventoryInjectionBudget::DEFAULT.hard_bytes
        );
        assert_eq!(
            resolved.best_effort_readonly_status,
            BestEffortReadonlyStatus::BestEffortConfigured
        );
    }

    #[test]
    fn planning_launch_has_no_write_roots_and_cwd_is_aggregate_root() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        let resolved = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();

        let request =
            resolved.launch_request(fixture.provider_ref(), fixture.config_artifact_ref());
        assert_eq!(request.action, SessionPolicyAction::PlanningReadOnly);
        assert!(request.writable_roots.is_empty());
        assert_eq!(request.readable_roots, vec![fixture.aggregate_root()]);
        assert_eq!(
            request.target,
            PolicyTarget::aggregate_root(fixture.aggregate_root())
        );
    }

    #[test]
    fn resolver_rejects_primary_fallback_when_selection_empty() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        fixture.write_selection_with_no_effective_members();

        let error = fixture
            .resolver()
            .build("project_0001", "issue_empty", &[])
            .unwrap_err();
        assert!(error.to_string().contains("effective_member_empty"));
    }

    #[test]
    fn resume_with_matching_fingerprint_reuses_context_and_mismatch_rebuilds() {
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        let first = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();
        let persisted_fingerprint = first.snapshot.access_fingerprint.clone();
        // build 返回的 snapshot 携带冻结指纹，与落盘一致（Task 11 一致性修正）。
        assert!(!persisted_fingerprint.is_empty());

        let same = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(same, ResumeDecision::SameContext(_)));

        fixture.change_membership_revision(); // 模拟成员变更，指纹漂移
        let stale = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(
            stale,
            ResumeDecision::StaleContext { reason, .. } if reason != persisted_fingerprint
        ));
    }

    #[test]
    fn resume_is_readonly_and_rejected_reconnect_stays_stale() {
        // B1/TOCTOU 修复：resume 先加载既有 snapshot（load 只读、不写）再比较，禁止
        // 先写后比。因此漂移首次拒绝后，同一会话重连（未启动新会话）仍是 StaleContext，
        // 绝不因 snapshot 被 build 提前更新而误判 SameContext。
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        let first = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();
        let persisted_fingerprint = first.snapshot.access_fingerprint.clone();

        fixture.change_membership_revision(); // 指纹漂移

        let stale = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(stale, ResumeDecision::StaleContext { .. }));

        // resume 不落盘：拒绝后落盘快照仍为旧指纹（未被 build 更新）。
        let store = PlanningContextSnapshotStore::new(fixture.paths.clone());
        let persisted = store.load("project_0001", "issue_0001").unwrap().unwrap();
        assert_eq!(persisted.access_fingerprint, persisted_fingerprint);

        // 重连（未启动新会话）仍是 StaleContext。
        let again = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(again, ResumeDecision::StaleContext { .. }));
    }

    #[test]
    fn checkout_identity_change_triggers_fingerprint_drift() {
        // B2 修复：access_fingerprint_value 哈希 checkout_id。checkout identity 更换而
        // revision/dirty/availability 不变时，也必须触发漂移（StaleContext）。
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy();
        fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();

        let same = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(same, ResumeDecision::SameContext(_)));

        fixture.change_checkout_id(); // 仅更换 checkout identity
        let stale = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(stale, ResumeDecision::StaleContext { .. }));
    }

    #[test]
    fn removed_member_marks_snapshot_invalidated_and_resume_forces_stale_rebuild() {
        // REQ-PLN-02：规划后成员删除/停用 → build 结果附失效警告，落盘快照标记失效；
        // resume 强制 StaleContext 重建，绝不沿用可能已失效的旧上下文。
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy_with_removed_member();

        let resolved = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();
        assert!(resolved.invalidation.is_some());
        assert_eq!(
            resolved
                .invalidation
                .as_ref()
                .map(|record| record.reason.as_str()),
            Some("member_removed")
        );
        assert_eq!(resolved.context_resolution.invalid_member_ids.len(), 1);
        assert_eq!(resolved.context_resolution.set.len(), 1);

        // build 已落盘失效快照 → resume 走 invalidated 分支强制 StaleContext。
        let decision = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(
            decision,
            ResumeDecision::StaleContext { reason, .. }
                if reason.starts_with("invalidated:")
        ));

        // selection 失效标记也已自动写入。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.paths.clone());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }

    #[test]
    fn tombstoned_member_in_manifest_invalidates_snapshot_and_forces_stale_rebuild() {
        // REQ-PLN-02 fix round 1：真实 tombstone（status=Tombstoned 但仍留在
        // manifest.member_ids）→ resolver 按 Active 过滤 → snapshot 失效 + resume 强制
        // StaleContext；selection 由 manifest 不一致兑底标记失效。
        let mut fixture = resolver_fixture();
        fixture.write_active_manifest_index_and_policy_with_tombstoned_member_in_manifest();

        let resolved = fixture
            .resolver()
            .build("project_0001", "issue_0001", &[])
            .unwrap();
        assert!(resolved.invalidation.is_some());
        assert_eq!(
            resolved
                .invalidation
                .as_ref()
                .map(|record| record.reason.as_str()),
            Some("member_removed")
        );
        // set 只含 api（Tombstoned 成员被排除）。
        assert_eq!(resolved.context_resolution.set.len(), 1);
        assert_eq!(resolved.context_resolution.set[0].alias, "api");
        // manifest.member_ids 含 web 但 status=Tombstoned → invalid_member_ids 含 web。
        let web = LogicalRepositoryId(stable_uuid(0x0002));
        assert!(
            resolved
                .context_resolution
                .invalid_member_ids
                .contains(&web)
        );

        // build 已落盘失效快照 → resume 走 invalidated 分支强制 StaleContext。
        let decision = fixture
            .resolver()
            .resume("project_0001", "issue_0001")
            .unwrap();
        assert!(matches!(
            decision,
            ResumeDecision::StaleContext { reason, .. }
                if reason.starts_with("invalidated:")
        ));

        // selection 失效标记也已写入（manifest 不一致兑底，selection 自身不含删除成员）。
        let selection_store = IssueCodebaseSelectionStore::new(fixture.paths.clone());
        assert!(
            selection_store
                .is_invalidated("project_0001", "issue_0001")
                .unwrap()
        );
    }
}
