// P11 严格 publisher 行为用例（RED，2026-11-13）。
// 落位：src/product/workspace_engine/tests/sc_failure_publication.rs
// 挂载：src/product/workspace_engine/sc_failure_artifacts.rs 末尾
//   #[cfg(test)]
//   #[path = "tests/sc_failure_publication.rs"]
//   mod publication_tests;
//
// 共享真实 fixture：同目录 sc_failure_publication_fixture.rs（`#[path] mod fixture;`）。
// 本文件只放断言与 engine 侧助手：真实 store / 真实 engine / 真实文件故障，无
// provider/模型调用。
//
// 契约：
// ① 同包重放复用同一 version + created_at；普通 current 版本不受影响；节点
//    artifact_ref 与发布回执一致；ArtifactUpdate 只在回执落盘之后广播。
// ② 节点详情落盘被真实文件系统故障挡住时，包停留在 raw 表（pending），
//    publication_incomplete=true，且不广播 ArtifactUpdate。
// ③ 调用方给的包与真实证据不一致时拒发（不信任调用方载荷），raw 表不增长。
//
// 载荷比较走 serde JSON（不钉 `ArtifactPayload` 的枚举变体名）：变体名由 P11
// enum 提交与 P12/P17 消费者钉，本文件只钉「落盘载荷就是包本体」这一事实。

use super::publication_fixture::{
    NODE_ID, ordinary_version, publication_fixture, receipt_files, receipt_from_disk,
    seed_timeline_node,
};
use crate::product::checkpoint_store::CheckpointStore;
use crate::product::models::ArtifactRef;
use crate::product::workspace_engine::{EngineEvent, WorkspaceEngine, WorkspaceSession};
use crate::web::workspace_ws_types::{ArtifactPayload, ScFailureRepairPackage};
use std::sync::Arc;
use tokio::sync::mpsc;

fn payload_json(payload: &ArtifactPayload) -> serde_json::Value {
    serde_json::to_value(payload).expect("payload json")
}

/// 落盘载荷的 wire 形状：带 wrapper 的对象 `{"sc_failure_repair_package": {...}}`。
fn package_payload_json(package: &ScFailureRepairPackage) -> serde_json::Value {
    serde_json::json!({ "sc_failure_repair_package": package })
}

/// 真实 engine：持久 store + 事件通道（回执落盘后才广播 ArtifactUpdate）。
/// `pub(super)`：同主模块下的 rebuild 消费者用例模块复用同一助手（不复制装配）。
pub(super) fn engine_for_fixture(
    fixture: &super::publication_fixture::PublicationFixture,
) -> (WorkspaceEngine, mpsc::Receiver<EngineEvent>) {
    let (event_tx, event_rx) = mpsc::channel(32);
    let record = fixture
        .store
        .get_workspace_session(&fixture.session_id)
        .expect("session record");
    let mut engine = WorkspaceEngine::new_persistent(
        Arc::new(CheckpointStore::new(
            fixture._tmp.path().join("checkpoints"),
        )),
        fixture.store.clone(),
        event_tx,
        WorkspaceSession::from_record(record),
    );
    engine.active_node_id = Some(NODE_ID.to_string());
    (engine, event_rx)
}

fn drain(rx: &mut mpsc::Receiver<EngineEvent>) -> Vec<EngineEvent> {
    let mut events = Vec::new();
    while let Ok(event) = rx.try_recv() {
        events.push(event);
    }
    events
}

fn artifact_updates(events: &[EngineEvent]) -> Vec<(u32, &ArtifactPayload)> {
    events
        .iter()
        .filter_map(|event| match event {
            EngineEvent::ArtifactUpdate { version, payload } => Some((*version, payload)),
            _ => None,
        })
        .collect()
}

#[tokio::test]
async fn publish_writes_version_ref_and_receipt_and_replay_reuses_same_version() {
    let fixture = publication_fixture();
    seed_timeline_node(&fixture.store, &fixture.session_id);
    fixture
        .store
        .save_artifact_versions(
            &fixture.session_id,
            std::slice::from_ref(&ordinary_version()),
        )
        .expect("seed ordinary current version");

    let (mut engine, mut event_rx) = engine_for_fixture(&fixture);

    let published = engine
        .publish_sc_failure_repair_package(fixture.package.clone())
        .await
        .expect("publish repair package");
    // artifact_id 沿用既有 `artifact_version_{version:03}` 约定（package_id 不是
    // artifact 身份，只活在载荷与发布回执里）。
    assert_eq!(
        published.artifact_id,
        format!("artifact_version_{:03}", published.version)
    );
    assert!(published.version > 1, "包版本不得覆盖普通 current 版本");

    let versions = fixture
        .store
        .list_artifact_versions(&fixture.session_id)
        .expect("artifact versions");
    assert_eq!(versions.len(), 2, "普通版本 + 包版本，只有两条");
    let package_version = versions
        .iter()
        .find(|version| version.version == published.version)
        .expect("package version");
    assert!(!package_version.is_current, "A4 包版本不得抢占 current");
    assert_eq!(
        payload_json(&package_version.payload),
        package_payload_json(&fixture.package),
        "落盘载荷必须就是带 wrapper 的包本体"
    );
    let ordinary = versions
        .iter()
        .find(|version| version.version == 1)
        .expect("ordinary version");
    assert!(ordinary.is_current, "普通 current 版本不得被 SC 包改写");
    assert_eq!(ordinary.payload, ordinary_version().payload);

    // 节点 artifact_ref：artifact_id 沿用既有 `artifact_version_{version:03}` 约定，
    // 版本必须就是本次发布版本（package_id 不充当 artifact 身份）。
    let detail = fixture
        .store
        .load_node_detail(&fixture.session_id, NODE_ID)
        .expect("node detail");
    assert_eq!(
        detail.artifact_ref,
        Some(ArtifactRef {
            artifact_id: format!("artifact_version_{:03}", published.version),
            version: published.version,
        })
    );

    // 发布回执：直接读回执文件断字段（不新增 store 公开读接口）。
    let receipt = receipt_from_disk(&fixture.store, &fixture.chain, &fixture.package.package_id)
        .expect("published receipt file");
    assert_eq!(receipt.schema_version, 1);
    assert_eq!(receipt.package_id, fixture.package.package_id);
    assert_eq!(receipt.chain_id, fixture.package.chain_id);
    assert_eq!(receipt.node_id, NODE_ID);
    assert_eq!(receipt.version, published.version);
    assert_eq!(
        receipt.package_digest.len(),
        64,
        "回执摘要必须是 64 位十六进制"
    );

    // 发布屏障：回执缺失/不一致必须报错，而不是空回执兜底。
    fixture
        .store
        .validate_sc_artifact_publication(&fixture.scope, package_version)
        .expect("published receipt barrier");

    // ArtifactUpdate 只在回执落盘之后广播，且携带发布版本。
    let events = drain(&mut event_rx);
    let updates = artifact_updates(&events);
    assert!(!updates.is_empty(), "回执落盘后必须广播 ArtifactUpdate");
    assert!(
        updates
            .iter()
            .all(|(version, _)| *version == published.version),
        "广播的必须是本次发布版本"
    );

    // 同包重放：复用同一 version + created_at，raw 表不增长（允许重广播同一版本）。
    let replay = engine
        .publish_sc_failure_repair_package(fixture.package.clone())
        .await
        .expect("replay same package");
    assert_eq!(replay.version, published.version, "同包重放不得另起版本");
    assert_eq!(replay.artifact_id, published.artifact_id);
    let after = fixture
        .store
        .list_artifact_versions(&fixture.session_id)
        .expect("artifact versions after replay");
    assert_eq!(after.len(), 2, "重放不得追加版本行");
    let replayed = after
        .iter()
        .find(|version| version.version == published.version)
        .expect("package version after replay");
    assert_eq!(replayed.created_at, package_version.created_at);
    assert_eq!(replayed.payload, package_version.payload);
}

#[tokio::test]
async fn node_detail_filesystem_fault_keeps_package_pending_without_artifact_update() {
    let fixture = publication_fixture();
    seed_timeline_node(&fixture.store, &fixture.session_id);
    fixture
        .store
        .save_artifact_versions(
            &fixture.session_id,
            std::slice::from_ref(&ordinary_version()),
        )
        .expect("seed ordinary current version");

    // 真实文件系统故障：把节点详情文件路径占成目录 → 节点详情写入必失败。
    let node_detail_path = fixture
        .store
        .workspace_timeline_root_for_session(&fixture.session_id)
        .expect("timeline root")
        .join("timeline_node_details")
        .join(format!("{NODE_ID}.json"));
    std::fs::create_dir_all(&node_detail_path).expect("occupy node detail path");

    let (mut engine, mut event_rx) = engine_for_fixture(&fixture);

    let error = engine
        .publish_sc_failure_repair_package(fixture.package.clone())
        .await
        .expect_err("node detail write fault must not report success");
    assert!(
        error.publication_incomplete,
        "节点详情落盘失败必须标记 publication_incomplete"
    );
    assert!(
        error.code.starts_with("sc_failure_"),
        "错误码必须是稳定契约面: {}",
        error.code
    );
    assert!(!error.step.is_empty());

    // 包停在 raw 表（pending），但不广播成功。
    let versions = fixture
        .store
        .list_artifact_versions(&fixture.session_id)
        .expect("artifact versions");
    let package_row = versions
        .iter()
        .find(|version| payload_json(&version.payload) == package_payload_json(&fixture.package))
        .expect("package version stays pending in the raw table");
    assert!(!package_row.is_current);
    assert!(
        versions
            .iter()
            .any(|version| version.version == 1 && version.is_current),
        "普通 current 版本不得被失败发布改写"
    );
    assert!(
        artifact_updates(&drain(&mut event_rx)).is_empty(),
        "未完成发布不得广播 ArtifactUpdate"
    );
    assert!(
        receipt_from_disk(&fixture.store, &fixture.chain, &fixture.package.package_id).is_none(),
        "未完成发布不得留下发布回执"
    );
}

#[tokio::test]
async fn package_inconsistent_with_real_evidence_is_rejected() {
    let fixture = publication_fixture();
    seed_timeline_node(&fixture.store, &fixture.session_id);

    let (mut engine, mut event_rx) = engine_for_fixture(&fixture);

    let mut tampered = fixture.package.clone();
    tampered.terminal_reason = "被调用方改写的终止原因".to_string();

    let error = engine
        .publish_sc_failure_repair_package(tampered)
        .await
        .expect_err("与真实证据不一致的包必须拒发");
    assert!(
        error.code.starts_with("sc_failure_"),
        "错误码必须是稳定契约面: {}",
        error.code
    );
    assert!(
        fixture
            .store
            .list_artifact_versions(&fixture.session_id)
            .expect("artifact versions")
            .is_empty(),
        "拒发的包不得落进 raw 表"
    );
    assert!(
        artifact_updates(&drain(&mut event_rx)).is_empty(),
        "拒发的包不得广播 ArtifactUpdate"
    );
    assert!(
        receipt_from_disk(&fixture.store, &fixture.chain, &fixture.package.package_id).is_none(),
        "拒发的包不得留下发布回执"
    );
    assert!(
        receipt_files(&fixture.store, &fixture.chain).is_empty(),
        "拒发的包不得留下任何回执文件"
    );
}
