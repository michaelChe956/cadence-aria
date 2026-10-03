//! HTTP projection and synchronous rebuild endpoints for aggregate indexes.

use super::support::{default_logical_codebase_id, product_app_paths, resolve_lc_authority};
use super::*;

use axum::Json;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexError, AggregateIndexRecord, AggregateIndexStatus,
};
use crate::web::error::ApiError;
use crate::web::state::WebAppState;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub struct AggregateIndexActiveResponse {
    pub state: &'static str,
    pub revision: Option<u64>,
    pub indexed_at: Option<String>,
    pub warning: Option<String>,
}

pub async fn get_active_aggregate_index(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    get_active_aggregate_index_for_lc(&state, &project_id, &logical_codebase_id)
}

/// v1.3 canonical endpoint: the active projection is resolved per logical
/// codebase.
pub async fn get_lc_active_aggregate_index(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    // C4 Task 2：canonical 路由先经唯一 authority resolver 冻结 LC 身份
    //（conflict fail-closed），再读纯投影。
    resolve_lc_authority(&paths, &project_id, &logical_codebase_id)?;
    get_active_aggregate_index_for_lc(&state, &project_id, &logical_codebase_id)
}

fn get_active_aggregate_index_for_lc(
    state: &WebAppState,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<Response> {
    let paths = product_app_paths(state);
    validate_project_id(project_id)?;
    let response = read_active_projection(&paths, project_id, logical_codebase_id)?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

pub async fn rebuild_aggregate_index(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    rebuild_aggregate_index_for_lc(state, project_id, logical_codebase_id).await
}

/// v1.3 canonical endpoint: the rebuild is resolved per logical codebase.
pub async fn rebuild_lc_aggregate_index(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    // C4 Task 2：canonical 路由先经唯一 authority resolver 冻结 LC 身份
    //（conflict fail-closed），再执行显式 rebuild 动作。
    resolve_lc_authority(&paths, &project_id, &logical_codebase_id)?;
    rebuild_aggregate_index_for_lc(state, project_id, logical_codebase_id).await
}

async fn rebuild_aggregate_index_for_lc(
    state: WebAppState,
    project_id: String,
    logical_codebase_id: String,
) -> ApiResult<Response> {
    let paths = product_app_paths(&state);
    validate_project_id(&project_id)?;
    let rebuild_key = format!("{project_id}/{logical_codebase_id}");
    let _lease = state
        .aggregate_index_rebuilds
        .try_register(&rebuild_key)
        .ok_or_else(|| {
            ApiError::runtime(
                "aggregate_index_rebuild_in_progress",
                "aggregate index rebuild is already in progress",
                serde_json::json!({}),
            )
        })?;
    let dependencies = state
        .aggregate_initialization_dependencies()
        .for_lc(logical_codebase_id.clone());
    let operation = dependencies.index.clone();
    let project_id_for_worker = project_id.clone();
    let result = tokio::task::spawn_blocking(move || operation.rebuild(&project_id_for_worker))
        .await
        .map_err(|error| {
            ApiError::runtime(
                "aggregate_index_unavailable",
                format!("aggregate index rebuild worker failed: {error}"),
                serde_json::json!({}),
            )
        })?;
    if let Err(error) = result {
        return Err(aggregate_index_api_error(error));
    }
    // Keep the lease until after the durable active projection is read. This
    // makes a same-codebase request observe either rebuilding or the new state,
    // never a transient gap between operation completion and response creation.
    let response = read_active_projection(&paths, &project_id, &logical_codebase_id)?;
    Ok((StatusCode::OK, Json(response)).into_response())
}

fn read_active_projection(
    paths: &crate::product::app_paths::ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<AggregateIndexActiveResponse> {
    let store = crate::product::logical_codebase::aggregate_index::AggregateIndexStore::for_lc(
        paths.clone(),
        logical_codebase_id,
    );
    let mut records = store
        .records(project_id)
        .map_err(aggregate_index_api_error)?;
    records.retain(|record| record.status != AggregateIndexStatus::Superseded);
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.aggregate_index_id.cmp(&left.aggregate_index_id))
    });

    let latest = records.first().cloned();
    // degraded LKG 优先于 active/stale 投影：只要存在一个 degraded generation，
    // 最近一次刷新已失败（成功 rebuild 会把 degraded 前代翻成 superseded）。
    let degraded_record = records
        .iter()
        .find(|candidate| candidate.status == AggregateIndexStatus::Degraded)
        .cloned();
    let response = match latest {
        None => missing_response(None),
        Some(record) if record.status == AggregateIndexStatus::Building => {
            projection("rebuilding", &record, None)
        }
        _ if degraded_record.is_some() => {
            projection("degraded", &degraded_record.expect("checked above"), None)
        }
        Some(record) if record.status == AggregateIndexStatus::Failed => {
            let good = records
                .iter()
                .find(|candidate| {
                    matches!(
                        candidate.status,
                        AggregateIndexStatus::Active
                            | AggregateIndexStatus::Stale
                            | AggregateIndexStatus::Degraded
                    )
                })
                .cloned();
            match good {
                None => missing_response(record.warning),
                Some(good) => projection("degraded", &good, record.warning),
            }
        }
        Some(record) => projection(
            match record.status {
                AggregateIndexStatus::Active => "active",
                AggregateIndexStatus::Stale => "stale",
                AggregateIndexStatus::Degraded => "degraded",
                AggregateIndexStatus::Building => "rebuilding",
                AggregateIndexStatus::Superseded | AggregateIndexStatus::Failed => "missing",
            },
            &record,
            None,
        ),
    };
    Ok(response)
}

fn projection(
    state: &'static str,
    record: &AggregateIndexRecord,
    warning: Option<String>,
) -> AggregateIndexActiveResponse {
    AggregateIndexActiveResponse {
        state,
        revision: Some(record.membership_revision),
        indexed_at: Some(record.updated_at.clone()),
        warning: warning.or_else(|| record.warning.clone()),
    }
}

fn missing_response(warning: Option<String>) -> AggregateIndexActiveResponse {
    AggregateIndexActiveResponse {
        state: "missing",
        revision: None,
        indexed_at: None,
        warning,
    }
}

fn validate_project_id(project_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })?;
    Ok(())
}

fn aggregate_index_api_error(error: AggregateIndexError) -> ApiError {
    let code = match error {
        AggregateIndexError::Failed { code, .. } | AggregateIndexError::Degraded { code, .. } => {
            code
        }
    };
    ApiError::runtime(
        "aggregate_index_unavailable",
        error.to_string(),
        serde_json::json!({
            "reason_code": code,
        }),
    )
}

#[cfg(test)]
mod projection_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_index::AggregateIndexStore;

    fn record(
        aggregate_index_id: &str,
        status: AggregateIndexStatus,
        updated_at: &str,
    ) -> AggregateIndexRecord {
        let mut record = AggregateIndexRecord::building(
            aggregate_index_id.to_string(),
            "project_0001".to_string(),
            3,
            Vec::new(),
            updated_at.to_string(),
        );
        record.status = status;
        record
    }

    #[test]
    fn read_active_projection_reports_missing_when_no_generation_exists() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "missing");
        assert_eq!(response.revision, None);
    }

    #[test]
    fn read_active_projection_maps_first_build_failure_to_missing_with_warning() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_failed_gen",
                    AggregateIndexStatus::Failed,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_failed_gen",
                AggregateIndexStatus::Failed,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        // 无 LKG：首建失败投影 missing，但保留可操作 warning。
        assert_eq!(response.state, "missing");
        assert!(
            response
                .warning
                .as_deref()
                .unwrap_or_default()
                .contains("codegraph_init_failed")
        );
    }

    #[test]
    fn read_active_projection_maps_rebuild_failure_to_degraded_last_known_good() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_lkg",
                    AggregateIndexStatus::Active,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();
        // rebuild 失败现场：LKG 被 degrade，新 generation 以 Stale + warning 落盘。
        store
            .mark_status(
                "project_0001",
                "aggregate_index_lkg",
                AggregateIndexStatus::Degraded,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_refresh_gen",
                    AggregateIndexStatus::Stale,
                    "2026-09-28T02:00:00Z",
                ),
            )
            .unwrap();
        store
            .mark_status(
                "project_0001",
                "aggregate_index_refresh_gen",
                AggregateIndexStatus::Stale,
                Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string()),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "degraded");
        assert_eq!(response.revision, Some(3));
        assert!(
            response
                .warning
                .as_deref()
                .unwrap_or_default()
                .contains("codegraph_init_failed")
        );
    }

    #[test]
    fn read_active_projection_reports_rebuilding_while_generation_is_building() {
        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let store = AggregateIndexStore::for_lc(paths.clone(), "logical_codebase_0001");
        store
            .create(
                "project_0001",
                record(
                    "aggregate_index_building_gen",
                    AggregateIndexStatus::Building,
                    "2026-09-28T01:00:00Z",
                ),
            )
            .unwrap();

        let response =
            read_active_projection(&paths, "project_0001", "logical_codebase_0001").unwrap();
        assert_eq!(response.state, "rebuilding");
    }

    // ---- C4 Task 2：旧 project 级布局不得战胜显式 LC 解析 ----

    #[test]
    fn old_project_layout_never_wins_over_explicit_lc_resolution() {
        use crate::product::id::repo_hash_for_path;
        use crate::product::issue_store::CreateProductIssueInput;
        use crate::product::logical_codebase::issue_selection::IssueCodebaseSelectionStore;
        use crate::product::logical_codebase::policy::PolicyTarget;
        use crate::product::logical_codebase::provider_gateway::{
            ProviderRef, SessionLaunchRequest,
        };
        use crate::product::logical_codebase::types::{
            CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, MemberStatus,
            RepositoryCheckoutRecord, RepositoryType,
        };
        use crate::product::logical_codebase::{
            IssueCodebaseSelection, LogicalCodebaseCreateInput, LogicalCodebaseManifest,
            LogicalCodebaseStore, LogicalRepositoryId, PlanningContextSetResolver,
            PolicyTargetResolver, ProductionPolicyTargetResolver, RepositoryCheckoutId,
        };
        use crate::product::project_store::{CreateProjectInput, ProjectStore};

        let temp = tempfile::tempdir().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let project_id = ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "legacy-layout-project".to_string(),
                description: None,
            })
            .unwrap()
            .id;

        // 真实成员仓（git dir 身份用于 source identity）。
        let repo = temp.path().join("workspace").join("repo-shared");
        std::fs::create_dir_all(&repo).unwrap();
        for arguments in [
            vec!["init", "-b", "main"],
            vec!["config", "user.email", "legacy@test.local"],
            vec!["config", "user.name", "Legacy Test"],
        ] {
            let output = std::process::Command::new("git")
                .current_dir(&repo)
                .args(&arguments)
                .output()
                .unwrap();
            assert!(output.status.success());
        }
        std::fs::write(repo.join("README.md"), "# shared\n").unwrap();
        let output = std::process::Command::new("git")
            .current_dir(&repo)
            .args(["add", "."])
            .output()
            .unwrap();
        assert!(output.status.success());
        let output = std::process::Command::new("git")
            .current_dir(&repo)
            .args(["commit", "-m", "init"])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let canonical = std::fs::canonicalize(&repo).unwrap();
        let source =
            crate::product::repository_store::resolve_repository_source(&canonical).unwrap();

        // 旧 project 级布局：project-level manifest + 同 source 成员，并迁移出别名 LC。
        let legacy_store = LogicalCodebaseStore::new(paths.clone());
        let legacy_member = LogicalRepositoryId(uuid::Uuid::new_v4());
        let mut legacy_manifest =
            LogicalCodebaseManifest::new(&project_id, temp.path().join("workspace"), Vec::new());
        legacy_manifest.member_ids = vec![legacy_member];
        legacy_store
            .save_manifest(&project_id, &legacy_manifest)
            .unwrap();
        legacy_store
            .save_member(
                &project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: legacy_member,
                    physical_repository_id: "repository_legacy_member".to_string(),
                    alias: "repo-shared".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: Vec::new(),
                    status: MemberStatus::Active,
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        legacy_store.migrate_legacy(&project_id).unwrap();

        // 显式新 LC：不同 root，但成员 source 与旧布局重叠。
        let explicit_root = temp.path().join("explicit-root");
        std::fs::create_dir_all(&explicit_root).unwrap();
        let lc2 = LogicalCodebaseStore::new(paths.clone())
            .create(
                &project_id,
                LogicalCodebaseCreateInput {
                    name: "explicit".to_string(),
                    aggregate_root: explicit_root.clone(),
                },
            )
            .unwrap()
            .id;
        let lc2_store = LogicalCodebaseStore::for_lc(paths.clone(), &lc2);
        let member2 = LogicalRepositoryId(uuid::Uuid::new_v4());
        let checkout2 = RepositoryCheckoutId(uuid::Uuid::new_v4());
        let mut manifest2 =
            LogicalCodebaseManifest::new(&project_id, explicit_root.clone(), Vec::new());
        manifest2.member_ids = vec![member2];
        lc2_store.save_manifest(&project_id, &manifest2).unwrap();
        lc2_store
            .save_member(
                &project_id,
                &CodebaseMemberRecord {
                    logical_repository_id: member2,
                    physical_repository_id: "repository_explicit_member".to_string(),
                    alias: "repo-shared".to_string(),
                    role: "member".to_string(),
                    ordinal: 1,
                    source_identity: source.clone(),
                    repo_type: RepositoryType::Unknown,
                    tech_stack: Vec::new(),
                    owner: None,
                    tags: Vec::new(),
                    default_ref: None,
                    checkout_ids: vec![checkout2],
                    status: MemberStatus::Active,
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();
        lc2_store
            .save_checkout(
                &project_id,
                &RepositoryCheckoutRecord {
                    checkout_id: checkout2,
                    logical_repository_id: member2,
                    physical_repository_id: "repository_explicit_member".to_string(),
                    kind: CheckoutKind::Main,
                    canonical_path: canonical.clone(),
                    checkout_path_hash: repo_hash_for_path(canonical.to_string_lossy().as_ref()),
                    git_dir_identity: source.git_dir_identity(),
                    revision: None,
                    availability: CheckoutAvailability::Available,
                    observed_at: "2026-09-29T00:00:00Z".to_string(),
                    created_at: "2026-09-29T00:00:00Z".to_string(),
                    updated_at: "2026-09-29T00:00:00Z".to_string(),
                },
            )
            .unwrap();

        // issue 显式归属 lc2。
        let issue = crate::product::issue_store::IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: project_id.clone(),
                repo_id: None,
                logical_codebase_id: Some(lc2.clone()),
                title: "explicit issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();
        IssueCodebaseSelectionStore::for_lc(paths.clone(), &lc2)
            .save(
                &IssueCodebaseSelection::all_members(&project_id, &issue.id, None)
                    .for_logical_codebase(&lc2),
            )
            .unwrap();

        let expected_conflict = "repository_routing_legacy_conflict";

        // 入口 1：规划/工作区上下文（PlanningContextSetResolver）。
        let planning =
            PlanningContextSetResolver::new(paths.clone()).resolve(&project_id, &issue.id);
        match planning {
            Err(crate::product::json_store::ProductStoreError::Conflict { kind, .. }) => {
                assert_eq!(kind, expected_conflict);
            }
            other => panic!("planning must fail closed with legacy conflict, got {other:?}"),
        }

        // 入口 2：aggregate index canonical GET 的身份校验。
        let index_error = super::super::support::resolve_lc_authority(&paths, &project_id, &lc2)
            .expect_err("aggregate index GET identity check must fail closed");
        assert_eq!(index_error.code, expected_conflict);

        // 入口 3：policy target resolver（provider spawn 前复验）。
        let request = SessionLaunchRequest::planning(
            project_id.clone(),
            ProviderRef::claude_code("cap_snapshot"),
            PolicyTarget::checkout(
                member2.0.to_string(),
                checkout2.0.to_string(),
                canonical.clone(),
            ),
            vec![canonical.clone()],
            "sha256:managed-config-artifact",
        );
        let policy_error = ProductionPolicyTargetResolver::for_lc(paths.clone(), &lc2)
            .resolve_and_revalidate(&request)
            .expect_err("policy target resolver must fail closed");
        match policy_error {
            crate::product::logical_codebase::ProviderGatewayError::Target(reason) => {
                assert!(
                    reason.contains(expected_conflict),
                    "policy resolver error must carry the stable conflict code: {reason}"
                );
            }
            other => panic!("expected Target error, got {other:?}"),
        }
    }
}
