// P16 消费者侧公开视图用例（2026-11-13）。
// 落位：src/product/workspace_engine/tests/sc_failure_publication_rebuild.rs
// 挂载：src/product/workspace_engine/sc_failure_artifacts.rs 末尾
//   #[cfg(test)]
//   #[path = "tests/sc_failure_publication_rebuild.rs"]
//   mod publication_rebuild_tests;
//
// 共享真实 fixture：主模块挂载的唯一 fixture 实例（super::publication_fixture，
// 不复制装配）；engine 助手复用 publisher 用例模块的 engine_for_fixture。
//
// 契约（消费者视角，全部走真实 store / 真实 engine / 真实文件；不注入前端状态、
// 不造 REST 载荷、不私有构造状态）：
//   ④ 真实发布 → 丢弃发布期内存状态、从 durable store 重建 engine → 公开
//      session_state 里的包版本与真实发布包逐字段一致（版本号 + 作用域 + 全部
//      轮次引用 + 证据引用），普通 current 产物不受影响；WS 序列化视图与公开
//      视图同形（扁平载荷键 + 完整包正文）。
//   ⑤ pending（缺回执）与坏回执都不得进入公开版本表/摘要：pending 在 store raw
//      表里仍在；坏回执显式投影 policy_diagnostics（稳定码），普通条目照常可见。
//
// 只新增这 2 条消费者转换：不复刻 publisher 用例（写入顺序/故障 pending/载荷不
// 一致拒发）。

use super::publication_fixture::{
    ORDINARY_MARKDOWN, ordinary_version, publication_fixture, receipt_from_disk, receipt_path,
    seed_timeline_node,
};
use super::publication_tests::engine_for_fixture;
use crate::product::work_item_plan_policy::PolicyDiagnostic;
use crate::product::workspace_engine::WorkspaceEngine;
use crate::web::workspace_ws_types::{
    ArtifactPayload, ArtifactVersion, ArtifactVersionSummary, ScFailureRepairPackage, WsOutMessage,
};

/// 公开视图切片：只取消费者真正读到的字段（版本表 / 摘要 / 当前产物 / 诊断）。
struct PublicArtifactView {
    artifact: Option<ArtifactPayload>,
    versions: Vec<ArtifactVersion>,
    summaries: Vec<ArtifactVersionSummary>,
    diagnostics: Vec<PolicyDiagnostic>,
}

/// 读公开 session_state（不私有构造状态、不改引擎内部状态）。
fn public_artifact_view(engine: &WorkspaceEngine) -> PublicArtifactView {
    match engine.build_session_state() {
        WsOutMessage::SessionState {
            artifact,
            artifact_versions,
            artifact_version_summaries,
            policy_diagnostics,
            ..
        } => PublicArtifactView {
            artifact,
            versions: artifact_versions,
            summaries: artifact_version_summaries,
            diagnostics: policy_diagnostics,
        },
        other => panic!("expected session_state, got {other:?}"),
    }
}

/// 版本行里的 SC 修复包载荷（非包行返回 None）。
fn sc_package_of(version: &ArtifactVersion) -> Option<&ScFailureRepairPackage> {
    match &version.payload {
        ArtifactPayload::ScFailureRepairPackage {
            sc_failure_repair_package,
        } => Some(sc_failure_repair_package.as_ref()),
        _ => None,
    }
}

/// P16 消费者转换④：真实发布 → 从 durable 状态重建 engine → 公开 session_state
/// 里的包版本逐字段等于真实发布包；普通 current 产物不变；WS 序列化视图与公开
/// 视图同形（扁平载荷 + 完整包正文）。
#[tokio::test]
async fn published_package_is_public_after_durable_engine_rebuild() {
    let fixture = publication_fixture();
    seed_timeline_node(&fixture.store, &fixture.session_id);
    fixture
        .store
        .save_artifact_versions(
            &fixture.session_id,
            std::slice::from_ref(&ordinary_version()),
        )
        .expect("seed ordinary current version");

    let (mut engine, _event_rx) = engine_for_fixture(&fixture);
    let published = engine
        .publish_sc_failure_repair_package(fixture.package.clone())
        .await
        .expect("publish repair package");

    // 重建：丢弃发布期的全部内存状态，只从 durable store 重新装配 engine。
    let (rebuilt, _rebuilt_rx) = engine_for_fixture(&fixture);
    let view = public_artifact_view(&rebuilt);

    // 公开版本表里恰有一条包版本，载荷逐字段等于真实发布包。
    let package_versions: Vec<&ArtifactVersion> = view
        .versions
        .iter()
        .filter(|version| sc_package_of(version).is_some())
        .collect();
    assert_eq!(package_versions.len(), 1, "公开视图必须恰有一条包版本");
    let public_version = package_versions[0];
    assert_eq!(public_version.version, published.version);
    assert!(!public_version.is_current, "包版本不得抢占 current");
    let public_package = sc_package_of(public_version).expect("package payload");
    assert_eq!(
        public_package, &fixture.package,
        "公开视图的包必须与真实发布包逐字段一致"
    );

    // 消费者读到的字段：作用域 + 全部轮次引用 + 证据引用（不是长度、不是摘要）。
    assert_eq!(public_package.session_id, fixture.scope.session_id);
    assert_eq!(public_package.project_id, fixture.scope.project_id);
    assert_eq!(public_package.issue_id, fixture.scope.issue_id);
    assert_eq!(public_package.plan_id, fixture.scope.plan_id);
    assert_eq!(public_package.chain_id, fixture.chain.chain_id);
    assert_eq!(public_package.rounds.len(), fixture.package.rounds.len());
    for (public_round, published_round) in public_package
        .rounds
        .iter()
        .zip(fixture.package.rounds.iter())
    {
        assert_eq!(
            public_round.identity.round_id,
            published_round.identity.round_id
        );
        assert_eq!(
            public_round.identity.run_ref,
            published_round.identity.run_ref
        );
        assert_eq!(
            public_round.identity.node_id,
            published_round.identity.node_id
        );
        assert_eq!(
            public_round.identity.chain_id,
            published_round.identity.chain_id
        );
        assert_eq!(
            public_round.identity.role_run_seq,
            published_round.identity.role_run_seq
        );
        assert_eq!(
            public_round.identity.workspace_session_id,
            published_round.identity.workspace_session_id
        );
        assert_eq!(
            public_round.identity.round_ordinal,
            published_round.identity.round_ordinal
        );
        assert_eq!(
            public_round.identity.provider,
            published_round.identity.provider
        );
        assert_eq!(public_round.raw, published_round.raw);
        assert_eq!(public_round.source, published_round.source);
        assert_eq!(public_round.launch_state, published_round.launch_state);
        assert_eq!(public_round.failure_class, published_round.failure_class);
        assert_eq!(
            public_round.blocking_reasons,
            published_round.blocking_reasons
        );
        assert_eq!(
            public_round.repair_instructions,
            published_round.repair_instructions
        );
        assert_eq!(
            public_round.teaching_decision,
            published_round.teaching_decision
        );
        assert_eq!(public_round.prompts, published_round.prompts);
        assert_eq!(
            public_round.existing_failure_id,
            published_round.existing_failure_id
        );
        assert_eq!(public_round.attempt_key, published_round.attempt_key);
    }
    assert_eq!(
        public_package.last_provider_output,
        fixture.package.last_provider_output
    );
    assert_eq!(
        public_package.last_failure_source,
        fixture.package.last_failure_source
    );
    assert_eq!(
        public_package.teaching_prompts,
        fixture.package.teaching_prompts
    );
    assert_eq!(public_package.completeness, fixture.package.completeness);
    assert_eq!(
        public_package.missing_evidence,
        fixture.package.missing_evidence
    );
    assert_eq!(
        public_package.evidence_digest,
        fixture.package.evidence_digest
    );
    assert_eq!(
        public_package.evidence_through_entry_key,
        fixture.package.evidence_through_entry_key
    );

    // 摘要视图：包版本有真实摘要（字节数/轮次数/完整性），普通 current 版本原样在列。
    let package_summary = view
        .summaries
        .iter()
        .find(|summary| summary.version == published.version)
        .expect("package summary");
    assert!(!package_summary.is_current);
    let preview: serde_json::Value =
        serde_json::from_str(&package_summary.markdown_preview).expect("summary preview is json");
    assert_eq!(
        preview["round_count"].as_u64(),
        Some(fixture.package.rounds.len() as u64)
    );
    assert_eq!(preview["completeness"].as_str(), Some("complete"));
    let ordinary_summary = view
        .summaries
        .iter()
        .find(|summary| summary.version == 1)
        .expect("ordinary summary");
    assert!(ordinary_summary.is_current, "普通 current 版本不得被包改写");

    // 当前产物仍是普通 markdown（A4 包不得替换 current 产物）。
    let Some(ArtifactPayload::Markdown { markdown, .. }) = view.artifact.as_ref() else {
        panic!("公开的 current 产物必须仍是普通 markdown 载荷");
    };
    assert_eq!(markdown.as_str(), ORDINARY_MARKDOWN);

    // store raw：普通版本行原样，普通 current 未被包改写。
    let raw = fixture
        .store
        .list_artifact_versions(&fixture.session_id)
        .expect("raw artifact versions");
    let ordinary = raw
        .iter()
        .find(|version| version.version == 1)
        .expect("ordinary version row");
    assert!(ordinary.is_current);
    assert_eq!(ordinary.payload, ordinary_version().payload);

    // WS 序列化视图与公开视图同形：扁平载荷键 + 完整包正文（不是长度、不是摘要）。
    let wire = serde_json::to_value(rebuilt.build_session_state()).expect("session state json");
    assert_eq!(wire["type"], "session_state");
    let wire_versions = wire["artifact_versions"]
        .as_array()
        .expect("wire artifact versions");
    let wire_package = wire_versions
        .iter()
        .find(|entry| entry.get("sc_failure_repair_package").is_some())
        .expect("wire package version");
    assert_eq!(
        wire_package["version"].as_u64(),
        Some(published.version as u64)
    );
    assert_eq!(
        wire_package["sc_failure_repair_package"]["package_id"].as_str(),
        Some(fixture.package.package_id.as_str())
    );
    assert_eq!(
        wire_package["sc_failure_repair_package"]["session_id"].as_str(),
        Some(fixture.scope.session_id.as_str())
    );
    assert_eq!(
        wire_package["sc_failure_repair_package"]["evidence_digest"].as_str(),
        Some(fixture.package.evidence_digest.as_str())
    );
    let first_round = fixture
        .package
        .rounds
        .first()
        .expect("fixture package has a round");
    assert_eq!(
        wire_package["sc_failure_repair_package"]["rounds"][0]["identity"]["round_id"].as_str(),
        Some(first_round.identity.round_id.as_str())
    );
    assert_eq!(
        wire_package["sc_failure_repair_package"]["rounds"][0]["identity"]["run_ref"].as_str(),
        Some(first_round.identity.run_ref.as_str())
    );
    assert_eq!(
        wire_package["sc_failure_repair_package"]["rounds"][0]["identity"]["node_id"].as_str(),
        Some(first_round.identity.node_id.as_str())
    );
}

/// P16 消费者转换⑤：pending（缺回执）与坏回执都不得进入公开版本表/摘要；
/// pending 在 store raw 表里仍在；坏回执显式投影 policy_diagnostics（稳定码），
/// 普通条目照常可见。
#[tokio::test]
async fn pending_and_corrupt_packages_stay_out_of_the_public_view() {
    // ── pending：发布回执落盘被真实文件系统故障挡住 → 版本行已落 raw，回执未写 ──
    let fixture = publication_fixture();
    seed_timeline_node(&fixture.store, &fixture.session_id);
    fixture
        .store
        .save_artifact_versions(
            &fixture.session_id,
            std::slice::from_ref(&ordinary_version()),
        )
        .expect("seed ordinary current version");
    // 真实回执落盘故障：把回执文件路径占成目录 → 回执写入必失败（真实 store 原因）。
    let pending_receipt_file =
        receipt_path(&fixture.store, &fixture.chain, &fixture.package.package_id);
    std::fs::create_dir_all(&pending_receipt_file).expect("occupy the receipt path");

    let (mut engine, _event_rx) = engine_for_fixture(&fixture);
    let error = engine
        .publish_sc_failure_repair_package(fixture.package.clone())
        .await
        .expect_err("receipt write fault must not report success");
    assert!(
        error.publication_incomplete,
        "回执落盘失败必须标记 publication_incomplete"
    );
    assert_eq!(
        error.code, "sc_failure_receipt_failed",
        "回执故障必须是稳定契约码"
    );
    assert!(!error.step.is_empty());
    assert!(
        !error.detail.is_empty(),
        "回执故障必须带真实 store 原因，不得是空壳错误"
    );
    assert!(
        receipt_from_disk(&fixture.store, &fixture.chain, &fixture.package.package_id).is_none(),
        "未完成发布不得留下可读回执"
    );

    // store raw 表里 pending 版本行仍在（内部 raw 读不被公开视图过滤覆盖）。
    let raw = fixture
        .store
        .list_artifact_versions(&fixture.session_id)
        .expect("raw artifact versions");
    let pending = raw
        .iter()
        .find(|version| sc_package_of(version).is_some())
        .expect("pending package row stays in the raw table");
    let pending_version = pending.version;

    let (rebuilt, _rebuilt_rx) = engine_for_fixture(&fixture);
    let view = public_artifact_view(&rebuilt);
    assert!(
        view.versions
            .iter()
            .all(|version| sc_package_of(version).is_none()),
        "pending 包不得进入公开版本表"
    );
    assert!(
        view.summaries
            .iter()
            .all(|summary| summary.version != pending_version),
        "pending 包不得进入公开摘要"
    );
    assert!(
        view.summaries
            .iter()
            .any(|summary| summary.version == 1 && summary.is_current),
        "普通条目在 pending 时照常可见"
    );
    assert!(
        view.diagnostics
            .iter()
            .all(|diagnostic| !diagnostic.code.starts_with("sc_failure_publication_")),
        "pending 是未发布，不是错误：不得投影发布诊断"
    );

    // ── 坏回执：回执存在但不再绑定包/版本/节点 → 显式诊断 + 普通条目可见 ──
    let corrupt = publication_fixture();
    seed_timeline_node(&corrupt.store, &corrupt.session_id);
    corrupt
        .store
        .save_artifact_versions(
            &corrupt.session_id,
            std::slice::from_ref(&ordinary_version()),
        )
        .expect("seed ordinary current version");
    let (mut corrupt_engine, _corrupt_rx) = engine_for_fixture(&corrupt);
    let published = corrupt_engine
        .publish_sc_failure_repair_package(corrupt.package.clone())
        .await
        .expect("publish repair package");

    // 真实腐坏：改写已落盘回执的摘要字节（回执还在，但不再绑定真实包）。
    let receipt_file = receipt_path(&corrupt.store, &corrupt.chain, &corrupt.package.package_id);
    let mut receipt =
        receipt_from_disk(&corrupt.store, &corrupt.chain, &corrupt.package.package_id)
            .expect("published receipt on disk");
    receipt.package_digest = "f".repeat(64);
    std::fs::write(
        &receipt_file,
        serde_json::to_vec(&receipt).expect("receipt json"),
    )
    .expect("corrupt receipt on disk");

    let (corrupt_rebuilt, _corrupt_rebuilt_rx) = engine_for_fixture(&corrupt);
    let corrupt_view = public_artifact_view(&corrupt_rebuilt);
    assert!(
        corrupt_view
            .versions
            .iter()
            .all(|version| sc_package_of(version).is_none()),
        "坏回执的包不得进入公开版本表"
    );
    assert!(
        corrupt_view
            .summaries
            .iter()
            .all(|summary| summary.version != published.version),
        "坏回执的包不得进入公开摘要"
    );
    assert!(
        corrupt_view
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.code == "sc_failure_publication_receipt_mismatch"),
        "坏回执必须显式投影稳定诊断码（不得吞成空列表成功）: {:?}",
        corrupt_view.diagnostics
    );
    assert!(
        corrupt_view
            .summaries
            .iter()
            .any(|summary| summary.version == 1 && summary.is_current),
        "普通条目在坏回执时照常可见"
    );
}
