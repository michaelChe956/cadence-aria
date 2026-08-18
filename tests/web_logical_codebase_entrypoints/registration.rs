use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use axum::http::{Method, StatusCode};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::project_store::{CreateProjectInput, ProjectStore};
use cadence_aria::web::app::build_web_router;
use cadence_aria::web::runtime::WebRuntime;
use cadence_aria::web::state::WebAppState;
use serde_json::json;
use tempfile::{TempDir, tempdir};

use crate::{assert_error, request};

struct Fixture {
    _workspace: TempDir,
    root: PathBuf,
    git_root: PathBuf,
    candidates: Vec<String>,
    app: axum::Router,
}

impl Fixture {
    fn new() -> Self {
        let workspace = tempdir().unwrap();
        let root = workspace.path().join("aggregate-root");
        fs::create_dir_all(&root).unwrap();
        let clean = root.join("clean");
        let dirty = root.join("dirty");
        let nested = root.join("nested-parent");
        let nested_child = nested.join("child");
        let non_git = root.join("docs");
        let _missing = root.join("missing");
        let outside = workspace.path().join("outside");
        for path in [&clean, &dirty, &nested, &nested_child, &non_git, &outside] {
            fs::create_dir_all(path).unwrap();
        }
        init_git(&clean);
        init_git(&dirty);
        init_git(&nested);
        init_git(&nested_child);
        fs::write(clean.join("README.md"), "clean").unwrap();
        commit(&clean, "clean");
        fs::write(dirty.join("README.md"), "dirty").unwrap();
        fs::write(outside.join("README.md"), "outside").unwrap();

        let paths = ProductAppPaths::new(workspace.path().join(".aria"));
        ProjectStore::new(paths)
            .create(CreateProjectInput {
                name: "multi".into(),
                description: None,
                multi_repo: true,
            })
            .unwrap();
        let state = WebAppState::new(
            workspace.path().to_path_buf(),
            WebRuntime::new_fake(workspace.path().to_path_buf()),
        );
        Self {
            _workspace: workspace,
            root: root.clone(),
            git_root: clean,
            candidates: vec![
                root.join("clean").to_string_lossy().into_owned(),
                root.join("docs").to_string_lossy().into_owned(),
                root.join("clean").to_string_lossy().into_owned(),
                root.join("nested-parent").to_string_lossy().into_owned(),
                root.join("nested-parent/child")
                    .to_string_lossy()
                    .into_owned(),
                root.join("dirty").to_string_lossy().into_owned(),
                root.join("missing").to_string_lossy().into_owned(),
                outside.to_string_lossy().into_owned(),
            ],
            app: build_web_router(state),
        }
    }

    fn root(&self) -> &Path {
        &self.root
    }
    fn git_root(&self) -> &Path {
        &self.git_root
    }
}

fn init_git(path: &Path) {
    let status = Command::new("git")
        .args(["init", "-q"])
        .current_dir(path)
        .status()
        .unwrap();
    assert!(status.success());
}

fn git_output(path: &Path, arguments: &[&str]) -> String {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {arguments:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap()
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MemberGitSnapshot {
    head: String,
    status_porcelain: String,
    tracked_files: String,
    untracked_files: String,
    refs: String,
}

impl MemberGitSnapshot {
    fn capture(root: &Path) -> Self {
        Self {
            head: git_output(root, &["rev-parse", "HEAD"]),
            status_porcelain: git_output(root, &["status", "--porcelain"]),
            tracked_files: git_output(root, &["ls-files"]),
            untracked_files: git_output(root, &["status", "--porcelain", "--untracked"])
                .lines()
                .filter(|line| line.starts_with("?? "))
                .collect::<Vec<_>>()
                .join("\n"),
            refs: git_output(root, &["for-each-ref", "--format=%(refname) %(objectname)"]),
        }
    }
}

fn commit(path: &Path, message: &str) {
    let add = Command::new("git")
        .args(["add", "."])
        .current_dir(path)
        .status()
        .unwrap();
    assert!(add.success());
    let commit = Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.com",
            "commit",
            "-qm",
            message,
        ])
        .current_dir(path)
        .status()
        .unwrap();
    assert!(commit.success());
}

#[tokio::test]
async fn registration_preflight_persists_all_candidate_evidence_and_normalizes_dirty() {
    let fixture = Fixture::new();
    let (status, body) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({"aggregate_root": fixture.root(), "candidate_paths": fixture.candidates}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let items = body["items"].as_array().unwrap();
    assert_eq!(items.len(), 8);
    let classes: Vec<&str> = items
        .iter()
        .map(|item| item["class"].as_str().unwrap())
        .collect();
    for class in [
        "eligible",
        "non_git",
        "duplicate",
        "nested",
        "needs_attention",
        "missing",
        "outside_root",
    ] {
        assert!(classes.contains(&class), "missing {class}: {body}");
    }
    assert!(
        items
            .iter()
            .any(|item| item["class"] == "needs_attention" && item["reason"] == "dirty")
    );
    assert!(
        items
            .iter()
            .all(|item| item.get("preflight_revision").is_none())
    );
    let preflight_id = body["preflight_id"].as_str().unwrap();
    let snapshot_path = ProductAppPaths::new(fixture._workspace.path().join(".aria"))
        .registration_preflights_root("project_0001")
        .join(format!("{preflight_id}.json"));
    let snapshot = fs::read_to_string(snapshot_path).unwrap();
    for field in [
        "canonical_path",
        "git_root",
        "source_identity",
        "preflight_revision",
    ] {
        assert!(snapshot.contains(field), "snapshot omitted {field}");
    }
}

#[tokio::test]
async fn registration_submit_uses_frozen_snapshot_and_runs_batch_synchronously() {
    let fixture = Fixture::new();
    let (status, preflight) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({
            "aggregate_root": fixture.root(),
            "candidate_paths": [fixture.git_root(), fixture.root().join("dirty")]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight}");
    let preflight_id = preflight["preflight_id"].as_str().unwrap();
    let (status, batch) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations",
        json!({
            "aggregate_root": fixture.root(),
            "preflight_id": preflight_id,
            "confirmed_paths": [fixture.git_root(), fixture.root().join("dirty")]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    assert_eq!(batch["status"], "completed", "{batch}");
    assert_eq!(batch["items"].as_array().unwrap().len(), 2);
    assert!(
        batch["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|item| { item["status"] == "completed" })
    );
    let manifest = cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(
        ProductAppPaths::new(fixture._workspace.path().join(".aria")),
    )
    .load_manifest("project_0001")
    .unwrap()
    .expect("first synchronous registration creates a manifest");
    assert_eq!(manifest.provider_context_root, fixture.root());
    assert_eq!(manifest.member_ids.len(), 2);
}

#[tokio::test]
async fn registration_submit_without_dirty_confirmation_does_not_attach_dirty_candidate() {
    let fixture = Fixture::new();
    let (status, preflight) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({
            "aggregate_root": fixture.root(),
            "candidate_paths": [fixture.git_root(), fixture.root().join("dirty")]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight}");

    let (status, batch) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations",
        json!({
            "aggregate_root": fixture.root(),
            "preflight_id": preflight["preflight_id"],
            "confirmed_paths": [fixture.git_root()]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    assert_eq!(batch["status"], "completed", "{batch}");
    assert_eq!(batch["items"].as_array().unwrap().len(), 1, "{batch}");
    assert_eq!(batch["items"][0]["status"], "completed", "{batch}");

    let manifest = cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(
        ProductAppPaths::new(fixture._workspace.path().join(".aria")),
    )
    .load_manifest("project_0001")
    .unwrap()
    .expect("registration creates a manifest for the confirmed clean candidate");
    assert_eq!(manifest.member_ids.len(), 1);
}

#[tokio::test]
async fn submit_uses_frozen_snapshot_and_distinguishes_revision_from_identity_drift() {
    let fixture = Fixture::new();
    let web = fixture.root().join("web");
    fs::create_dir_all(&web).unwrap();
    init_git(&web);
    fs::write(web.join("README.md"), "web").unwrap();
    commit(&web, "initial web");
    let (status, preflight) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({
            "aggregate_root": fixture.root(),
            "candidate_paths": [fixture.git_root(), &web]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight}");
    fs::write(fixture.git_root().join("revision.txt"), "changed").unwrap();
    commit(fixture.git_root(), "revision drift");
    let member_snapshots_before = [
        MemberGitSnapshot::capture(fixture.git_root()),
        MemberGitSnapshot::capture(&web),
    ];
    let (status, batch) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations",
        json!({
            "aggregate_root": fixture.root(),
            "preflight_id": preflight["preflight_id"],
            "confirmed_paths": [fixture.git_root(), &web]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{batch}");
    assert_eq!(batch["status"], "partial_failed", "{batch}");
    assert_eq!(batch["items"][0]["status"], "needs_attention", "{batch}");
    assert_eq!(batch["items"][1]["status"], "completed", "{batch}");
    assert_eq!(
        member_snapshots_before,
        [
            MemberGitSnapshot::capture(fixture.git_root()),
            MemberGitSnapshot::capture(&web),
        ],
        "registration must not write member Git state"
    );

    let mobile = fixture.root().join("mobile");
    fs::create_dir_all(&mobile).unwrap();
    init_git(&mobile);
    fs::write(mobile.join("README.md"), "mobile").unwrap();
    commit(&mobile, "initial mobile");
    let (status, conflicting) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({
            "aggregate_root": fixture.root(),
            "candidate_paths": [&mobile]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{conflicting}");
    let remote = Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "ssh://git@example.test/acme/mobile.git",
        ])
        .current_dir(&mobile)
        .status()
        .unwrap();
    assert!(remote.success());
    assert_error(
        request(
            &fixture.app,
            Method::POST,
            "/api/projects/project_0001/logical-codebase/registrations",
            json!({
                "aggregate_root": fixture.root(),
                "preflight_id": conflicting["preflight_id"],
                "confirmed_paths": [&mobile]
            }),
        )
        .await,
        StatusCode::CONFLICT,
        "registration_batch_conflict",
    );
}

#[tokio::test]
async fn registration_identity_drift_aborts_before_attaching_any_member() {
    let fixture = Fixture::new();
    let web = fixture.root().join("web");
    fs::create_dir_all(&web).unwrap();
    init_git(&web);
    fs::write(web.join("README.md"), "web").unwrap();
    commit(&web, "initial web");
    let (status, preflight) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({
            "aggregate_root": fixture.root(),
            "candidate_paths": [fixture.git_root(), &web]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight}");
    let remote = Command::new("git")
        .args([
            "remote",
            "add",
            "origin",
            "ssh://git@example.test/acme/web.git",
        ])
        .current_dir(&web)
        .status()
        .unwrap();
    assert!(remote.success());

    assert_error(
        request(
            &fixture.app,
            Method::POST,
            "/api/projects/project_0001/logical-codebase/registrations",
            json!({
                "aggregate_root": fixture.root(),
                "preflight_id": preflight["preflight_id"],
                "confirmed_paths": [fixture.git_root(), &web]
            }),
        )
        .await,
        StatusCode::CONFLICT,
        "registration_batch_conflict",
    );

    let store = cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(
        ProductAppPaths::new(fixture._workspace.path().join(".aria")),
    );
    assert!(store.load_manifest("project_0001").unwrap().is_none());
    assert!(store.list_members("project_0001").unwrap().is_empty());
}

#[tokio::test]
async fn registration_submit_rejects_manifest_root_mismatch() {
    let fixture = Fixture::new();
    let paths = ProductAppPaths::new(fixture._workspace.path().join(".aria"));
    let (status, preflight) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({
            "aggregate_root": fixture.root(),
            "candidate_paths": [fixture.git_root()]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight}");
    let preflight_id = preflight["preflight_id"].as_str().unwrap();
    let first = cadence_aria::product::logical_codebase::LogicalCodebaseManifest::new(
        "project_0001",
        fixture._workspace.path().join("other-root"),
        vec![cadence_aria::product::logical_codebase::LogicalRepositoryId(uuid::Uuid::new_v4())],
    );
    cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(paths.clone())
        .save_manifest("project_0001", &first)
        .unwrap();
    assert_error(
        request(
            &fixture.app,
            Method::POST,
            "/api/projects/project_0001/logical-codebase/registrations",
            json!({
                "aggregate_root": fixture.root(),
                "preflight_id": preflight_id,
                "confirmed_paths": [fixture.git_root()]
            }),
        )
        .await,
        StatusCode::CONFLICT,
        "aggregate_root_mismatch",
    );
    let store = cadence_aria::product::logical_codebase::LogicalCodebaseStore::new(paths);
    assert_eq!(store.load_manifest("project_0001").unwrap(), Some(first));
    assert!(store.list_members("project_0001").unwrap().is_empty());
}

#[tokio::test]
async fn registration_submit_rejects_missing_preflight_snapshot() {
    let fixture = Fixture::new();
    assert_error(
        request(
            &fixture.app,
            Method::POST,
            "/api/projects/project_0001/logical-codebase/registrations",
            json!({
                "aggregate_root": fixture.root(),
                "preflight_id": "preflight_missing",
                "confirmed_paths": [fixture.git_root()]
            }),
        )
        .await,
        StatusCode::NOT_FOUND,
        "registration_preflight_not_found",
    );
}

#[tokio::test]
async fn registration_preflight_maps_aggregate_root_ownership_conflict_without_internal_code() {
    let fixture = Fixture::new();
    fs::write(fixture.root().join(".aria"), "owned").unwrap();
    assert_error(
        request(
            &fixture.app,
            Method::POST,
            "/api/projects/project_0001/logical-codebase/registrations/preflight",
            json!({"aggregate_root": fixture.root(), "candidate_paths": []}),
        )
        .await,
        StatusCode::CONFLICT,
        "aggregate_root_ownership_conflict",
    );
}

#[tokio::test]
async fn registration_preflight_maps_missing_aggregate_root_to_422_stable_code() {
    let fixture = Fixture::new();
    assert_error(
        request(
            &fixture.app,
            Method::POST,
            "/api/projects/project_0001/logical-codebase/registrations/preflight",
            json!({"aggregate_root": fixture.root().join("does-not-exist"), "candidate_paths": []}),
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "aggregate_root_missing",
    );
}

#[tokio::test]
async fn registration_http_chain_queries_resumes_and_rejects_terminal_cancel() {
    let fixture = Fixture::new();
    let (status, preflight) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations/preflight",
        json!({
            "aggregate_root": fixture.root(),
            "candidate_paths": [fixture.git_root()]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{preflight}");
    let (status, submitted) = request(
        &fixture.app,
        Method::POST,
        "/api/projects/project_0001/logical-codebase/registrations",
        json!({
            "aggregate_root": fixture.root(),
            "preflight_id": preflight["preflight_id"],
            "confirmed_paths": [fixture.git_root()]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{submitted}");
    let batch_id = submitted["batch_id"].as_str().unwrap();

    let (status, queried) = request(
        &fixture.app,
        Method::GET,
        &format!("/api/projects/project_0001/logical-codebase/registrations/{batch_id}"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{queried}");
    assert_eq!(queried["status"], "completed", "{queried}");
    assert_eq!(queried["items"][0]["status"], "completed", "{queried}");

    let (status, resumed) = request(
        &fixture.app,
        Method::POST,
        &format!("/api/projects/project_0001/logical-codebase/registrations/{batch_id}/resume"),
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{resumed}");
    assert_eq!(resumed["status"], "completed", "{resumed}");
    assert_eq!(
        resumed["items"][0]["failure_reason"],
        serde_json::Value::Null
    );

    assert_error(
        request(
            &fixture.app,
            Method::POST,
            &format!("/api/projects/project_0001/logical-codebase/registrations/{batch_id}/cancel"),
            json!({}),
        )
        .await,
        StatusCode::CONFLICT,
        "registration_batch_not_cancelable",
    );
}

#[tokio::test]
async fn registration_batch_not_found_is_stable_404() {
    let fixture = Fixture::new();
    assert_error(
        request(
            &fixture.app,
            Method::GET,
            "/api/projects/project_0001/logical-codebase/registrations/registration_batch_missing",
            json!({}),
        )
        .await,
        StatusCode::NOT_FOUND,
        "registration_batch_not_found",
    );
}

#[tokio::test]
async fn registration_preflight_admits_root_before_classification_and_guards_single_repo() {
    let fixture = Fixture::new();
    assert_error(
        request(
            &fixture.app,
            Method::POST,
            "/api/projects/project_0001/logical-codebase/registrations/preflight",
            json!({"aggregate_root": fixture.git_root(), "candidate_paths": []}),
        )
        .await,
        StatusCode::UNPROCESSABLE_ENTITY,
        "aggregate_root_is_git",
    );

    let paths = ProductAppPaths::new(fixture._workspace.path().join(".aria"));
    ProjectStore::new(paths.clone())
        .create(CreateProjectInput {
            name: "single".into(),
            description: None,
            multi_repo: false,
        })
        .unwrap();
    assert_error(
        request(
            &fixture.app,
            Method::POST,
            "/api/projects/project_0002/logical-codebase/registrations/preflight",
            json!({"aggregate_root": fixture.root(), "candidate_paths": []}),
        )
        .await,
        StatusCode::CONFLICT,
        "logical_codebase_feature_disabled",
    );
    assert!(!paths.logical_codebase_root("project_0002").exists());
}
