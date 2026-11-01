#[cfg(test)]
mod hygiene_tests {
    /// F13(波3):`"reviewer-windows"` 目录名单一常量归 reviewer_window_store
    /// 持有,三处消费(coding 引擎/coding_attempt_store 通道守卫/plan 窗口)
    /// 同根同值;字面值保持不变,不留模块私有别名。
    #[test]
    fn f13_reviewer_windows_dir_is_single_shared_constant() {
        assert_eq!(
            crate::product::reviewer_window_store::REVIEWER_WINDOW_STORE_DIR,
            "reviewer-windows",
            "单一常量必须持原目录字面值"
        );
    }
    mod f12 {
        use super::super::*;
        use crate::product::app_paths::ProductAppPaths;
        use crate::product::reviewer_snapshot::{
            GitObservationBaseline, REVIEWER_BASELINE_SCHEMA, RestoreCursor,
            ReviewWorktreeLockIdentity, ReviewerBaselineManifest, ReviewerSnapshotStore,
        };
        use crate::product::reviewer_window_store::{
            REVIEWER_WINDOW_STORE_DIR, ReviewerWindowRecord, ReviewerWindowState,
            ReviewerWindowStore,
        };

        /// 每用例独立临时根(避免用例间残留互染)。
        fn f12_root(tag: &str) -> (tempfile::TempDir, std::path::PathBuf) {
            let root = tempfile::tempdir().expect("f12 root");
            let snap = root.path().join(format!("snap-{tag}"));
            std::fs::create_dir_all(&snap).expect("snap root");
            (root, snap)
        }

        fn make_residue(snap: &std::path::Path, window_id: &str) -> std::path::PathBuf {
            let dir = snap.join("windows").join(window_id);
            std::fs::create_dir_all(dir.join("blobs")).expect("residue dir");
            std::fs::write(dir.join("blobs/aa"), b"half-written").expect("blob");
            dir
        }

        /// 红线①:崩溃半成品(未发布本窗目录)必须清理,且幂等。
        #[test]
        fn f12_cleanup_removes_unpublished_residue_and_is_idempotent() {
            let (_keep, snap) = f12_root("unpub");
            let dir = make_residue(&snap, "w-f12");

            assert_eq!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-f12"),
                Ok(true),
                "F12 红:未发布本窗残留必须被清理"
            );
            assert!(!dir.exists(), "清理后目录消失");
            assert_eq!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-f12"),
                Ok(false),
                "重复清理幂等:目录已不存在=无事可做"
            );
        }

        /// 红线②:baseline=None ≠ manifest 未发布——已发布 manifest 是
        /// F4 同窗采用路径的可恢复证据,不得删除。
        #[test]
        fn f12_cleanup_keeps_published_manifest_dir() {
            let (_keep, snap) = f12_root("pub");
            let dir = make_residue(&snap, "w-pub");
            std::fs::write(dir.join("manifest.json"), b"{}").expect("published manifest");

            assert_eq!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-pub"),
                Ok(false),
                "manifest 已发布:不清理"
            );
            assert!(dir.join("manifest.json").is_file(), "可恢复证据保留");
        }

        /// 红线③:邻窗目录零接触。
        #[test]
        fn f12_cleanup_keeps_neighbor_windows() {
            let (_keep, snap) = f12_root("nb");
            let own = make_residue(&snap, "w-own");
            let neighbor = make_residue(&snap, "w-neighbor");

            assert_eq!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-own"),
                Ok(true)
            );
            assert!(!own.exists());
            assert!(
                neighbor.join("blobs/aa").exists(),
                "邻窗未发布目录也不得误删"
            );
        }

        /// 红线④:恶意 window_id(路径穿越/分隔符/空)拒绝清理,目标零副作用。
        #[test]
        fn f12_cleanup_rejects_malicious_window_ids() {
            let (_keep, snap) = f12_root("evil");
            let escape = _keep.path().join("escape-target");
            std::fs::create_dir_all(&escape).expect("escape target");

            for bad in ["../escape-target", "a/b", ""] {
                assert!(
                    cleanup_unpublished_snapshot_window_dir(&snap, bad).is_err(),
                    "恶意 window_id {bad:?} 必须拒绝"
                );
            }
            assert!(escape.exists(), "越界目标零副作用");
        }

        /// 红线⑤:符号链接窗口目录拒绝穿透删除。
        #[cfg(unix)]
        #[test]
        fn f12_cleanup_refuses_symlinked_window_dir() {
            let (keep, snap) = f12_root("link");
            let target = make_residue(keep.path(), "real-target");
            std::fs::create_dir_all(snap.join("windows")).expect("windows dir");
            #[cfg(unix)]
            std::os::unix::fs::symlink(&target, snap.join("windows").join("w-link"))
                .expect("symlink");

            assert!(
                cleanup_unpublished_snapshot_window_dir(&snap, "w-link").is_err(),
                "符号链接目录必须拒绝穿透删除"
            );
            assert!(target.exists(), "链接目标不被删");
        }

        /// 布局钉:本模块重建的 `windows/<window_id>` 布局必须与
        /// `ReviewerSnapshotStore` 真实布局一致(store 能读到我们写的 manifest)。
        #[test]
        fn f12_snapshot_layout_pin_matches_store() {
            let (_keep, snap) = f12_root("pin");
            let identity = ReviewWorktreeLockIdentity {
                canonical_worktree: std::path::PathBuf::from("/tmp/f12-pin-worktree"),
                gitdir: std::path::PathBuf::from("/tmp/f12-pin-worktree/.git"),
                common_dir: std::path::PathBuf::from("/tmp/f12-pin-worktree/.git"),
                lease_id: "f12-pin-lease".to_string(),
            };
            let manifest = ReviewerBaselineManifest {
                schema: REVIEWER_BASELINE_SCHEMA.to_string(),
                window_id: "w-f12-pin".to_string(),
                attempt_id: "attempt-pin".to_string(),
                run_id: "run-pin".to_string(),
                lock_identity: identity,
                entries: Vec::new(),
                git_observation: GitObservationBaseline {
                    head_symref: None,
                    head_sha: "0".repeat(40),
                    index_blob: "blobs/0".to_string(),
                    index_sha256: "0".repeat(64),
                    managed_refs: Vec::new(),
                    refs_digest: "0".repeat(64),
                    index_semantic_sha256: None,
                },
                platform_write_baseline: Vec::new(),
                git_annex_observation: None,
                scope_key: crate::cross_cutting::provider_resources::ResourceScopeKey {
                    scope_id: "f12-pin-scope".to_string(),
                    run_id: "run-pin".to_string(),
                    boot_identity: "f12-pin-boot".to_string(),
                },
                scope_manifest: Vec::new(),
                quiesced_at: "2026-10-19T00:00:00Z".to_string(),
                captured_at: "2026-10-19T00:00:00Z".to_string(),
                restore_cursor: RestoreCursor::start(),
                ignore_rules_freeze:
                    crate::product::reviewer_snapshot_domain::IgnoreRulesFreeze::synthetic(),
                domain_boundary:
                    crate::product::reviewer_snapshot_domain::DOMAIN_BOUNDARY_DISCLOSURE.to_string(),
            };
            let dir = snap.join("windows").join("w-f12-pin");
            std::fs::create_dir_all(&dir).expect("pin dir");
            std::fs::write(
                dir.join("manifest.json"),
                serde_json::to_string(&manifest).expect("serialize manifest"),
            )
            .expect("write manifest");

            let loaded = ReviewerSnapshotStore::new(&snap)
                .load_manifest("w-f12-pin")
                .expect("store 必须按同布局读到本模块写的 manifest");
            assert_eq!(loaded.window_id, "w-f12-pin");
        }

        fn f12_engine(root: &std::path::Path) -> CodingWorkspaceEngine {
            let (event_tx, _event_rx) = tokio::sync::mpsc::channel(64);
            CodingWorkspaceEngine::new(
                crate::product::coding_attempt_store::CodingAttemptStore::new(
                    ProductAppPaths::new(root.join(".aria")),
                ),
                crate::product::git_workspace_service::GitWorkspaceService::new(),
                event_tx,
            )
        }

        fn f12_preparing_record(root: &std::path::Path, window_id: &str) {
            let store = ReviewerWindowStore::new(
                ProductAppPaths::new(root.join(".aria"))
                    .root()
                    .join(REVIEWER_WINDOW_STORE_DIR),
            );
            let identity = ReviewWorktreeLockIdentity {
                canonical_worktree: root.join("wt"),
                gitdir: root.join("wt/.git"),
                common_dir: root.join("wt/.git"),
                lease_id: format!("f12-lease-{window_id}"),
            };
            store
                .create_or_takeover_window(&ReviewerWindowRecord::new(
                    window_id,
                    identity,
                    "attempt-f12",
                    "run-f12",
                    "claude_code",
                    Vec::new(),
                ))
                .expect("preparing record");
        }

        /// 集成红线:启动补偿弃置无 baseline 窗前,清理未发布本窗 snapshot
        /// 残留;邻窗不误删。
        #[tokio::test]
        async fn f12_startup_compensation_cleans_unpublished_residue() {
            let root = tempfile::tempdir().expect("root");
            f12_preparing_record(root.path(), "f12-win");
            let snapshot_root = ProductAppPaths::new(root.path().join(".aria"))
                .root()
                .join(REVIEWER_SNAPSHOT_STORE_DIR);
            let own = make_residue(&snapshot_root, "f12-win");
            let neighbor = make_residue(&snapshot_root, "f12-neighbor");

            let outcomes = f12_engine(root.path())
                .compensate_open_reviewer_windows_at_startup()
                .await;

            assert_eq!(outcomes.len(), 1);
            assert!(
                outcomes[0].released,
                "无 baseline Preparing 窗仍必须弃置释放"
            );
            assert!(!own.exists(), "F12 红:未发布本窗残留必须在弃置前清理");
            assert!(neighbor.exists(), "邻窗残留不误删");
            let record = ReviewerWindowStore::new(
                ProductAppPaths::new(root.path().join(".aria"))
                    .root()
                    .join(REVIEWER_WINDOW_STORE_DIR),
            )
            .load_window("f12-win")
            .expect("record");
            assert_eq!(record.state, ReviewerWindowState::ClosedInvalidated);
        }

        /// 集成对照:manifest 已发布的目录在弃置后仍保留(F4 可恢复证据)。
        #[tokio::test]
        async fn f12_startup_compensation_keeps_published_manifest() {
            let root = tempfile::tempdir().expect("root");
            f12_preparing_record(root.path(), "f12-pub");
            let snapshot_root = ProductAppPaths::new(root.path().join(".aria"))
                .root()
                .join(REVIEWER_SNAPSHOT_STORE_DIR);
            let own = make_residue(&snapshot_root, "f12-pub");
            std::fs::write(own.join("manifest.json"), b"{}").expect("published manifest");

            let outcomes = f12_engine(root.path())
                .compensate_open_reviewer_windows_at_startup()
                .await;

            assert_eq!(outcomes.len(), 1);
            assert!(outcomes[0].released, "已发布 manifest 不阻塞弃置收口");
            assert!(
                own.join("manifest.json").is_file(),
                "已发布 manifest 是可恢复证据,不得误删"
            );
        }
    }
}
