// ---------------------------------------------------------------------------
// R1:根政策发布原子化与 revision 递增的回归测试。
//
// 独立文件（policy.rs / policy_tests.inc.rs 已接近 large_file_guard 的
// 1200 行上限）：只钉定「store 读到的 {artifact, 回执, 正文} 三者同版本」
// 与「正文未变时重放不新增 revision」两条不变量。
// ---------------------------------------------------------------------------

#[cfg(test)]
mod publish_atomic_tests {
    use super::tests::{
        AT_0800, AT_0900, AT_1000, PROJECT, publisher, seed_final_step_operation,
        seed_operation_for_manifest, write_root_fixture,
    };
    use super::*;
    use crate::product::app_paths::ProductAppPaths;

    struct AtomicFixture {
        temp: tempfile::TempDir,
        paths: ProductAppPaths,
        canonical: PathBuf,
        manifest: LogicalCodebaseManifest,
        store: AggregatePolicyArtifactStore,
    }

    impl AtomicFixture {
        fn new() -> Self {
            let temp = tempfile::tempdir().unwrap();
            let root = temp.path().join("lc-root");
            std::fs::create_dir_all(&root).unwrap();
            write_root_fixture(&root);
            let canonical = std::fs::canonicalize(&root).unwrap();
            let paths = ProductAppPaths::new(temp.path());
            let store = AggregatePolicyArtifactStore::for_lc(paths.clone(), "lc_atomic");
            let manifest =
                seed_final_step_operation(&paths, "lc_atomic", "op_atomic_a", &canonical);
            store.ensure_bootstrap(&manifest).unwrap();
            Self {
                temp,
                paths,
                canonical,
                manifest,
                store,
            }
        }

        fn publish(
            &self,
            operation_id: &str,
            created_at: &str,
        ) -> Result<AggregatePolicyArtifact, ProductStoreError> {
            self.store.publish_recipe_policy(
                &self.manifest,
                operation_id,
                &self.canonical,
                created_at.to_string(),
            )
        }

        fn next_operation(&self, operation_id: &str) {
            seed_operation_for_manifest(&self.paths, "lc_atomic", operation_id, &self.manifest);
        }

        fn locator(&self, revision: u64) -> PathBuf {
            self.canonical.join(format!(
                "policy/{PROJECT}/{}/{revision}",
                self.manifest.logical_codebase_id
            ))
        }
    }

    /// R1.2:artifact 提交失败必须整体回滚——不得留下「正文/回执新、记录
    /// 旧」的半成品；未注入故障的重放仍能提交同一 revision。
    #[test]
    fn publish_failure_leaves_no_orphan_body_or_receipt() {
        let fixture = AtomicFixture::new();
        let publish = publisher(&fixture.store, &fixture.canonical);
        let first = fixture.publish("op_atomic_a", AT_0800).unwrap();
        assert_eq!(first.revision, 2);
        fixture.next_operation("op_atomic_b");
        // 改源产生新 revision，才会真正走 artifact 提交点。
        std::fs::write(fixture.canonical.join(".claude/rules/b.md"), "规则 B 变更\n").unwrap();

        publish_faults::arm(fixture.temp.path(), publish_faults::FaultPhase::ArtifactSave);
        assert!(fixture.publish("op_atomic_b", AT_0900).is_err());

        assert!(
            !fixture.locator(3).exists(),
            "失败的发布不得在新 revision 路径留下正文"
        );
        assert!(
            fixture
                .store
                .get_recipe_policy_publication(PROJECT, "op_atomic_b")
                .unwrap()
                .is_none(),
            "失败的发布不得留下回执"
        );
        assert_eq!(fixture.store.get(PROJECT).unwrap().unwrap(), first);
        let frozen = fixture
            .store
            .get_recipe_policy_publication(PROJECT, "op_atomic_a")
            .unwrap()
            .expect("首条回执仍在");
        assert_eq!(frozen.artifact, first);
        assert_eq!(
            std::fs::read(fixture.locator(2)).unwrap(),
            first.policy_text.as_bytes(),
            "旧 revision 正文不得被改写或删除"
        );
        // publisher 只用于一致性签名,避免未使用告警。
        let _ = publish;

        let second = fixture.publish("op_atomic_b", AT_1000).unwrap();
        assert_eq!(second.revision, 3);
        assert_eq!(
            std::fs::read(fixture.locator(3)).unwrap(),
            second.policy_text.as_bytes()
        );
        assert_eq!(fixture.store.get(PROJECT).unwrap().unwrap(), second);
    }

    /// R1.1/R1.2：候选落后于 current（或超前且无前驱）时必须在任何写盘前停等。
    #[test]
    fn candidate_that_cannot_advance_current_fails_closed() {
        let fixture = AtomicFixture::new();
        let first = fixture.publish("op_atomic_a", AT_0800).unwrap();
        assert_eq!(first.revision, 2);
        fixture.next_operation("op_atomic_ahead");
        std::fs::write(fixture.canonical.join(".claude/rules/b.md"), "规则 B 变更\n").unwrap();
        let ahead = fixture.publish("op_atomic_ahead", AT_0900).unwrap();
        assert_eq!(ahead.revision, 3);

        // 与 current 相同 → 幂等放行。
        ensure_candidate_committable(PROJECT, Some(&ahead), &ahead).unwrap();

        // 落后一版 → superseded 停等。
        let error = ensure_candidate_committable(PROJECT, Some(&ahead), &first)
            .expect_err("落后候选必须停等");
        assert!(
            error.to_string().contains("policy_publication_superseded"),
            "必须是可识别的 superseded 冲突:{error}"
        );

        // 无 current 时只有 revision 1 合法。
        let error = ensure_candidate_committable(PROJECT, None, &first)
            .expect_err("无前驱的 revision 2 必须停等");
        assert!(
            error.to_string().contains("policy_publication_superseded"),
            "必须是可识别的 superseded 冲突:{error}"
        );
    }

    /// R1.3:正文未变时同 operation 重放不新增 revision、不改写既有字节。
    #[test]
    fn replay_with_unchanged_text_keeps_revision_and_bytes() {
        let fixture = AtomicFixture::new();
        let first = fixture.publish("op_atomic_a", AT_0800).unwrap();
        let locator = fixture.locator(2);
        let frozen_body = std::fs::read(&locator).unwrap();

        let second = fixture.publish("op_atomic_a", AT_0900).unwrap();
        assert_eq!(second, first);
        assert!(
            !fixture.locator(3).exists(),
            "正文未变的幂等重放不得新增 revision"
        );
        assert_eq!(std::fs::read(&locator).unwrap(), frozen_body);
        assert_eq!(fixture.store.get(PROJECT).unwrap().unwrap(), first);
    }
}
