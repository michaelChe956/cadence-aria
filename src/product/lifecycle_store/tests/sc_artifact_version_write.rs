// P14a1 RED: 集中 artifact_versions.json 分配/保存同锁闭环（Plan §2.5 P14 / D1、D2 已裁）。
// 落位：`src/product/lifecycle_store/tests/sc_artifact_version_write.rs`（独立新文件，不追加到 tests.rs）。
// 挂载：`workspace_timeline_artifacts.inc.rs` 末尾
//   #[cfg(test)]
//   #[path = "tests/sc_artifact_version_write.rs"]
//   mod sc_artifact_version_write;
//   —— 该 .inc.rs 由 `workspace.rs:1220` 以 `include!` 在模块作用域拼接；`#[path]` 无论按
//   「被 include 文件所在目录」还是「workspace.rs 所在目录」解析，都得到同一个
//   `src/product/lifecycle_store/tests/sc_artifact_version_write.rs`（两文件同目录，结论唯一；
//   同形先例 `sc_failure_evidence.rs:891-896`）。
// 模块路径：crate::product::lifecycle_store::workspace::sc_artifact_version_write
// 门禁：cargo test --lib lifecycle_store::workspace::sc_artifact_version_write::
// 断言只落消费者可见面（公开 API / 磁盘字节 / 错误 variant），不接私有锁 seam/hook。
// A4 保留/拒绝/重放用例属 P14a2（需 ArtifactPayload::ScFailureRepairPackage），此处不含。

use std::time::Duration;

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;
use crate::product::lifecycle_store::{CreateWorkspaceSessionInput, LifecycleStore};
use crate::product::models::{ProviderName, WorkspaceSessionRecord, WorkspaceType};
use crate::web::workspace_ws_types::{ArtifactPayload, ArtifactVersion};
use tempfile::TempDir;

const P14_PROJECT: &str = "project_p14";
const P14_ISSUE: &str = "issue_p14";

fn p14_version(version: u32, markdown: &str) -> ArtifactVersion {
    ArtifactVersion {
        version,
        payload: ArtifactPayload::Markdown {
            markdown: markdown.to_string(),
            diff: None,
        },
        generated_by: ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: version == 1,
        created_at: "2026-11-13T00:00:00Z".to_string(),
        source_node_id: "node_p14".to_string(),
    }
}

fn p14_create_session(store: &LifecycleStore, entity_id: &str) -> WorkspaceSessionRecord {
    store
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: P14_PROJECT.to_string(),
            issue_id: P14_ISSUE.to_string(),
            entity_id: entity_id.to_string(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Codex,
            reviewer_provider: Some(ProviderName::ClaudeCode),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: None,
        })
        .unwrap()
}

fn p14_store() -> (TempDir, LifecycleStore, WorkspaceSessionRecord) {
    let tmp = TempDir::new().unwrap();
    let store = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let session = p14_create_session(&store, "plan_p14");
    (tmp, store, session)
}

fn p14_artifact_path(store: &LifecycleStore, session_id: &str) -> std::path::PathBuf {
    store
        .app_paths()
        .issue_lifecycle_root(P14_PROJECT, P14_ISSUE)
        .join("workspace-timelines")
        .join(session_id)
        .join("artifact_versions.json")
}

fn p14_versions(store: &LifecycleStore, session_id: &str) -> Vec<u32> {
    store
        .list_artifact_versions(session_id)
        .unwrap()
        .iter()
        .map(|version| version.version)
        .collect()
}

// 独立线程持有某 session 写锁并在 closure 内阻塞；返回 (holder, release)。
fn p14_hold_write_lock(
    paths: ProductAppPaths,
    session_id: String,
) -> (std::thread::JoinHandle<()>, std::sync::mpsc::Sender<()>) {
    let (entered_tx, entered_rx) = std::sync::mpsc::channel::<()>();
    let (release_tx, release_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        LifecycleStore::new(paths)
            .with_artifact_versions_write(&session_id, |_ctx| {
                entered_tx.send(()).unwrap();
                release_rx.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            })
            .unwrap();
    });
    entered_rx
        .recv_timeout(Duration::from_secs(5))
        .expect("holder must enter the write closure");
    (holder, release_tx)
}

#[test]
fn p14_closure_reads_raw_allocates_max_plus_one_and_saves_merged() {
    let (_tmp, store, session) = p14_store();
    let sid = session.id.clone();
    store
        .save_artifact_versions(&sid, &[p14_version(1, "# v1"), p14_version(5, "# v5")])
        .unwrap();

    let (merged, bytes_first) = store
        .with_artifact_versions_write(&sid, |ctx| {
            assert_eq!(ctx.raw().len(), 2, "raw must be the disk truth");
            assert_eq!(ctx.next_version()?, 6, "must be max(raw.version)+1, not len()+1");
            let mut proposed = ctx.raw().to_vec();
            proposed.push(p14_version(6, "# v6"));
            let merged = ctx.save(&proposed)?;
            let bytes = ctx.read_bytes()?;
            let parsed: Vec<ArtifactVersion> = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(parsed, merged, "read_bytes must be exactly the merged table");
            Ok((merged, bytes))
        })
        .unwrap();
    assert_eq!(merged.iter().map(|v| v.version).collect::<Vec<_>>(), vec![1, 5, 6]);

    let bytes_second = store
        .with_artifact_versions_write(&sid, |ctx| {
            ctx.save(&merged)?;
            ctx.read_bytes()
        })
        .unwrap();
    assert_eq!(bytes_second, bytes_first, "same content must serialize byte-identically");
    assert_eq!(store.list_artifact_versions(&sid).unwrap(), merged);
}

#[test]
fn p14_next_version_overflow_is_typed_error_and_file_untouched() {
    let (_tmp, store, session) = p14_store();
    let sid = session.id.clone();
    store.save_artifact_versions(&sid, &[p14_version(u32::MAX, "# max")]).unwrap();
    let before = std::fs::read(p14_artifact_path(&store, &sid)).unwrap();

    match store.with_artifact_versions_write(&sid, |ctx| ctx.next_version()) {
        Err(ProductStoreError::InvalidRecord { kind, .. }) => assert_eq!(kind, "artifact_versions"),
        other => panic!("expected typed version-overflow error, got {other:?}"),
    }
    assert_eq!(std::fs::read(p14_artifact_path(&store, &sid)).unwrap(), before);
}

#[test]
fn p14_ordinary_save_is_proposed_authoritative_and_does_not_resurrect() {
    let (_tmp, store, session) = p14_store();
    let sid = session.id.clone();
    store
        .save_artifact_versions(&sid, &[p14_version(1, "# old"), p14_version(2, "# two")])
        .unwrap();

    let merged = store
        .with_artifact_versions_write(&sid, |ctx| ctx.save(&[p14_version(1, "# old")]))
        .unwrap();
    assert_eq!(merged, vec![p14_version(1, "# old")], "disk-absent entries must not resurrect");
    assert_eq!(store.list_artifact_versions(&sid).unwrap(), merged);

    // 普通回滚合法：同 version 不同 payload 以 proposed 为准（A4-only 拒绝不得泛化）。
    let rewritten = store
        .with_artifact_versions_write(&sid, |ctx| ctx.save(&[p14_version(1, "# rewritten")]))
        .unwrap();
    assert_eq!(rewritten, vec![p14_version(1, "# rewritten")]);

    let emptied = store
        .with_artifact_versions_write(&sid, |ctx| ctx.save(&[]))
        .unwrap();
    assert!(emptied.is_empty());
    assert!(store.list_artifact_versions(&sid).unwrap().is_empty());
}

#[test]
fn p14_rejected_save_and_closure_error_leave_bytes_intact() {
    let (_tmp, store, session) = p14_store();
    let sid = session.id.clone();
    store.save_artifact_versions(&sid, &[p14_version(1, "# a")]).unwrap();
    let before = std::fs::read(p14_artifact_path(&store, &sid)).unwrap();

    let duplicate = store.with_artifact_versions_write(&sid, |ctx| {
        ctx.save(&[p14_version(1, "# a"), p14_version(1, "# b")])
    });
    assert!(duplicate.is_err(), "duplicate version numbers must be rejected before write");
    assert_eq!(std::fs::read(p14_artifact_path(&store, &sid)).unwrap(), before);

    let closure_error = store.with_artifact_versions_write(&sid, |ctx| {
        assert_eq!(ctx.raw().len(), 1);
        Err::<(), _>(ProductStoreError::Io("p14 injected closure failure".to_string()))
    });
    assert!(closure_error.is_err());
    let after = std::fs::read(p14_artifact_path(&store, &sid)).unwrap();
    assert_eq!(after, before, "aborting a write closure must not erode the durable table");
}

#[test]
fn p14_public_save_blocks_on_the_same_file_lock() {
    let (_tmp, store, session) = p14_store();
    let sid = session.id.clone();
    store.save_artifact_versions(&sid, &[p14_version(1, "# a")]).unwrap();
    let seeded = std::fs::read(p14_artifact_path(&store, &sid)).unwrap();

    let (holder, release_tx) = p14_hold_write_lock(store.app_paths(), sid.clone());
    let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let contender = {
        let (paths, sid) = (store.app_paths(), sid.clone());
        std::thread::spawn(move || {
            let outcome = LifecycleStore::new(paths)
                .save_artifact_versions(&sid, &[p14_version(1, "# a"), p14_version(2, "# b")])
                .map_err(|error| error.to_string());
            done_tx.send(outcome).unwrap();
        })
    };
    let blocked = done_rx.recv_timeout(Duration::from_millis(200));
    assert!(blocked.is_err(), "public save must block on the same artifact_versions.json file lock");
    assert_eq!(
        std::fs::read(p14_artifact_path(&store, &sid)).unwrap(),
        seeded,
        "the blocked writer must not have written anything"
    );

    release_tx.send(()).unwrap();
    holder.join().unwrap();
    done_rx.recv_timeout(Duration::from_secs(5)).unwrap().unwrap();
    contender.join().unwrap();
    assert_eq!(p14_versions(&store, &sid), vec![1, 2]);
}

#[test]
fn p14_append_allocates_max_plus_one_in_lock_not_caller_version() {
    let (_tmp, store, session) = p14_store();
    let sid = session.id.clone();

    // 空表：分配 1（caller 传 99 被忽略）。
    store.append_artifact_version(&sid, p14_version(99, "# first")).unwrap();
    assert_eq!(p14_versions(&store, &sid), vec![1]);

    // 有空洞：max(raw)+1 = 6，而非 len+1 = 2，也不是 caller 的 99。
    store.save_artifact_versions(&sid, &[p14_version(1, "# v1"), p14_version(5, "# v5")]).unwrap();
    store.append_artifact_version(&sid, p14_version(99, "# sixth")).unwrap();
    assert_eq!(p14_versions(&store, &sid), vec![1, 5, 6]);

    // 连续追加单调，永不撞号。
    store.append_artifact_version(&sid, p14_version(99, "# seventh")).unwrap();
    assert_eq!(p14_versions(&store, &sid), vec![1, 5, 6, 7]);

    // 单独 session 的序号空间独立。
    let other = p14_create_session(&store, "plan_p14_other");
    store.append_artifact_version(&other.id, p14_version(99, "# other")).unwrap();
    assert_eq!(p14_versions(&store, &other.id), vec![1]);
}

#[test]
fn p14_write_lock_is_scoped_to_one_session_file() {
    let (_tmp, store, first) = p14_store();
    let second = p14_create_session(&store, "plan_p14_second");
    let (holder, release_tx) = p14_hold_write_lock(store.app_paths(), first.id.clone());

    let (done_tx, done_rx) = std::sync::mpsc::channel::<Result<(), String>>();
    let other = {
        let (paths, sid) = (store.app_paths(), second.id.clone());
        std::thread::spawn(move || {
            let outcome = LifecycleStore::new(paths)
                .save_artifact_versions(&sid, &[p14_version(1, "# other session")])
                .map_err(|error| error.to_string());
            done_tx.send(outcome).unwrap();
        })
    };
    done_rx
        .recv_timeout(Duration::from_secs(2))
        .expect("a different session's artifact table must not be blocked")
        .unwrap();
    other.join().unwrap();

    release_tx.send(()).unwrap();
    holder.join().unwrap();
}
