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
