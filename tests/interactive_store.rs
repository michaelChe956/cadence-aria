use cadence_aria::interactive::models::{
    ArtifactIndexEntry, ArtifactStatus, ContentType, InteractionTurn, NodeRun, NodeRunStatus,
    TaskSession, TurnStatus, WorkspaceProjection,
};
use cadence_aria::interactive::store::InteractiveStore;
use serde_json::json;
use tempfile::tempdir;

use cadence_aria::interactive::models::RuntimeCheckpoint;

#[test]
fn interactive_store_round_trips_session_turn_node_and_projection() {
    let workspace = tempdir().expect("workspace");
    let store = InteractiveStore::new(workspace.path(), "task_0001");

    let session = TaskSession {
        session_id: "sess_task_0001".to_string(),
        task_id: "task_0001".to_string(),
        created_at: "2026-05-07T00:00:00Z".to_string(),
        status: "idle".to_string(),
        turn_ids: vec!["turn_0001".to_string()],
        active_turn_id: Some("turn_0001".to_string()),
    };
    store.write_session(&session).expect("write session");

    let turn = InteractionTurn {
        turn_id: "turn_0001".to_string(),
        session_id: "sess_task_0001".to_string(),
        node_id: "N16".to_string(),
        provider_type: "codex".to_string(),
        prompt_snapshot: "实现 fibonacciSquareSum".to_string(),
        input_summary: json!({"allowed_write_scope": ["src/", "tests/"]}),
        checkpoint_before: Some("ckpt_0001".to_string()),
        provider_run_id: Some("run_n16_0001".to_string()),
        output_artifact_refs: vec!["coding_report_work_wt_001_0001".to_string()],
        changed_files: vec!["src/fibonacciSquareSum.js".to_string()],
        status: TurnStatus::Completed,
        dropped: false,
        created_at: "2026-05-07T00:00:01Z".to_string(),
        updated_at: "2026-05-07T00:00:02Z".to_string(),
    };
    store.write_turn(&turn).expect("write turn");

    let node = NodeRun {
        node_run_id: "nrun_0001".to_string(),
        node_id: "N16".to_string(),
        turn_id: Some("turn_0001".to_string()),
        provider_run_id: Some("run_n16_0001".to_string()),
        input_refs: vec!["plan_projection_0001".to_string()],
        output_schema: Some("schema://aria/artifacts/coding_report/v1".to_string()),
        artifact_refs: vec!["coding_report_work_wt_001_0001".to_string()],
        status: NodeRunStatus::Completed,
        duration_ms: Some(42),
        diagnostic_refs: Vec::new(),
        dropped: false,
        created_at: "2026-05-07T00:00:01Z".to_string(),
        updated_at: "2026-05-07T00:00:02Z".to_string(),
    };
    store.write_node_run(&node).expect("write node");

    let projection = WorkspaceProjection {
        workspace_root: workspace.path().to_string_lossy().to_string(),
        active_task_id: Some("task_0001".to_string()),
        active_session_id: Some("sess_task_0001".to_string()),
        overview: json!({"phase": "execution", "status": "running"}),
        sessions: vec![session.clone()],
        timeline: vec![json!({"kind": "node", "node_id": "N16"})],
        artifact_index: vec![ArtifactIndexEntry {
            artifact_ref: "coding_report_work_wt_001_0001".to_string(),
            artifact_kind: "coding_report".to_string(),
            producer_node: Some("N16".to_string()),
            path: ".aria/runtime/tasks/task_0001/artifacts/execution/0000.json".to_string(),
            summary: "编码报告".to_string(),
            status: ArtifactStatus::Active,
            content_type: ContentType::Json,
            traceability_refs: Vec::new(),
            dropped: false,
        }],
        diagnostics: Vec::new(),
        available_actions: vec!["rollback_previous_turn".to_string()],
    };
    store
        .write_projection(&projection)
        .expect("write projection");

    assert_eq!(
        store.read_session("sess_task_0001").expect("read session"),
        session
    );
    assert_eq!(store.read_turn("turn_0001").expect("read turn"), turn);
    assert_eq!(store.read_node_run("nrun_0001").expect("read node"), node);
    assert_eq!(
        store.read_projection().expect("read projection"),
        projection
    );
}

#[test]
fn interactive_models_cover_checkpoint_and_status_contract() {
    assert_eq!(
        serde_json::to_value(TurnStatus::Dropped).expect("turn"),
        "dropped"
    );
    assert_eq!(
        serde_json::to_value(NodeRunStatus::Started).expect("node"),
        "started"
    );
    assert_eq!(
        serde_json::to_value(NodeRunStatus::Blocked).expect("node"),
        "blocked"
    );
    assert_eq!(
        serde_json::to_value(NodeRunStatus::Dropped).expect("node"),
        "dropped"
    );
    assert_eq!(
        serde_json::to_value(ArtifactStatus::Candidate).expect("artifact"),
        "candidate"
    );
    assert_eq!(
        serde_json::to_value(ArtifactStatus::Rejected).expect("artifact"),
        "rejected"
    );
    assert_eq!(
        serde_json::to_value(ContentType::Source).expect("content"),
        "source"
    );
    assert_eq!(
        serde_json::to_value(ContentType::Test).expect("content"),
        "test"
    );
    assert_eq!(
        serde_json::to_value(ContentType::Log).expect("content"),
        "log"
    );
    assert_eq!(
        serde_json::to_value(ContentType::Unknown).expect("content"),
        "unknown"
    );

    let checkpoint = RuntimeCheckpoint {
        checkpoint_id: "ckpt_0001".to_string(),
        task_id: "task_0001".to_string(),
        session_id: "sess_task_0001".to_string(),
        turn_id: Some("turn_0001".to_string()),
        git_head: Some("abc123".to_string()),
        dirty_summary: json!({"tracked": 0, "untracked": 0}),
        state_snapshot_ref: "state@ckpt_0001.json".to_string(),
        projection_snapshot_ref: "projection@ckpt_0001.json".to_string(),
        artifact_boundary: 3,
        provider_run_boundary: 2,
        node_run_boundary: 1,
        created_at: "2026-05-07T00:00:00Z".to_string(),
    };
    let value = serde_json::to_value(checkpoint).expect("checkpoint json");
    assert_eq!(value["git_head"], "abc123");
    assert_eq!(value["state_snapshot_ref"], "state@ckpt_0001.json");
    assert_eq!(
        value["projection_snapshot_ref"],
        "projection@ckpt_0001.json"
    );
    assert_eq!(value["artifact_boundary"], 3);
    assert_eq!(value["provider_run_boundary"], 2);
    assert_eq!(value["node_run_boundary"], 1);
}

#[test]
fn interactive_store_rejects_invalid_task_id_before_building_task_root() {
    let workspace = tempdir().expect("workspace");
    let store = InteractiveStore::new(workspace.path(), "../outside");

    let error = store.task_root().expect_err("reject invalid task id");
    assert_eq!(error.code, "interactive_store_invalid_id");

    let session = TaskSession {
        session_id: "sess_task_0001".to_string(),
        task_id: "task_0001".to_string(),
        created_at: "2026-05-07T00:00:00Z".to_string(),
        status: "idle".to_string(),
        turn_ids: Vec::new(),
        active_turn_id: None,
    };
    let error = store.write_session(&session).expect_err("reject write");
    assert_eq!(error.code, "interactive_store_invalid_id");
    assert!(!workspace.path().join(".aria/runtime/outside").exists());
}

#[test]
fn interactive_store_rejects_invalid_runtime_object_ids() {
    let workspace = tempdir().expect("workspace");
    let store = InteractiveStore::new(workspace.path(), "task_0001");

    let invalid_session = TaskSession {
        session_id: "../sess".to_string(),
        task_id: "task_0001".to_string(),
        created_at: "2026-05-07T00:00:00Z".to_string(),
        status: "idle".to_string(),
        turn_ids: Vec::new(),
        active_turn_id: None,
    };
    assert_eq!(
        store
            .write_session(&invalid_session)
            .expect_err("reject session")
            .code,
        "interactive_store_invalid_id"
    );
    assert_eq!(
        store.read_turn("turn/0001").expect_err("reject turn").code,
        "interactive_store_invalid_id"
    );
    assert_eq!(
        store
            .read_node_run("nrun\\0001")
            .expect_err("reject node")
            .code,
        "interactive_store_invalid_id"
    );
    assert!(
        !workspace
            .path()
            .join(".aria/runtime/tasks/sess.json")
            .exists()
    );
}
