//! enrollment REST 入口（P0 1.2，REQ-WIGA-01/02）：PUT/GET/绑定三端点。
//!
//! PUT 在 CAS 之前做精确作用域校验（issue 存在、确认的 story/design
//! id+version、design 引用 story、单 logical member 恰一且与 target 一致）；
//! Disable 只查存在与 revision。绑定只接受显式 POST 的 plan/session，
//! 旧 plan 不隐式认领。GET 是读投影：不存在返回 200+null 而非 404。

use std::collections::BTreeSet;

use axum::Json;
use axum::extract::{Path, State};
use chrono::DateTime;
use serde_json::json;

use crate::product::issue_automation_store::IssueAutomationStore;
use crate::product::issue_store::IssueStore;
use crate::product::json_store::{ProductStoreError, validate_relative_id};
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::logical_codebase::RepositoryRouting;
use crate::product::models::LifecycleConfirmationStatus;
use crate::product::models::WorkspaceType;
use crate::product::models::automation::{
    EnrollmentError, EnrollmentSource, EnrollmentWriteCommand, IssueAutomationEnrollment,
};
use crate::product::work_item_plan_policy::RunPolicy;
use crate::web::error::{ApiError, ApiResult};
use crate::web::handlers::lifecycle::preflight::{
    SingleCandidatePreflightDecision, logical_repository_ids_for_preflight,
    preflight_single_repository_candidate,
};
use crate::web::state::WebAppState;
use crate::web::types::{EnrollmentBindRequest, EnrollmentPutRequest};

use super::support::{product_app_paths, product_store_api_error};

pub async fn put_automation_enrollment(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<EnrollmentPutRequest>,
) -> ApiResult<Json<IssueAutomationEnrollment>> {
    validate_request_ids(&project_id, &issue_id)?;
    validate_enrollment_scope(&state, &project_id, &issue_id, &request.command)?;
    IssueAutomationStore::new(product_app_paths(&state))
        .compare_and_set(
            &project_id,
            &issue_id,
            request.expected_revision,
            request.command,
        )
        .map(Json)
        .map_err(enrollment_api_error)
}

pub async fn get_automation_enrollment(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
) -> ApiResult<Json<Option<IssueAutomationEnrollment>>> {
    validate_request_ids(&project_id, &issue_id)?;
    ensure_issue_exists(&state, &project_id, &issue_id)?;
    IssueAutomationStore::new(product_app_paths(&state))
        .get(&project_id, &issue_id)
        .map(Json)
        .map_err(product_store_api_error)
}

pub async fn post_automation_enrollment_binding(
    State(state): State<WebAppState>,
    Path((project_id, issue_id)): Path<(String, String)>,
    Json(request): Json<EnrollmentBindRequest>,
) -> ApiResult<Json<IssueAutomationEnrollment>> {
    validate_request_ids(&project_id, &issue_id)?;
    if let Err(error) = validate_relative_id(&request.plan_id) {
        return Err(binding_target_invalid("plan_id", error));
    }
    if let Err(error) = validate_relative_id(&request.session_id) {
        return Err(binding_target_invalid("session_id", error));
    }
    ensure_issue_exists(&state, &project_id, &issue_id)?;

    let paths = product_app_paths(&state);
    let store = IssueAutomationStore::new(paths.clone());
    let enrollment = store
        .get(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .ok_or_else(|| enrollment_not_found())?;
    if !enrollment.enabled {
        return Err(enrollment_conflict(
            None,
            "automation enrollment is disabled",
        ));
    }

    // 绑定前复查授权 source 未漂移（精确 id+version，不猜 latest）。
    let lifecycle = LifecycleStore::new(paths.clone());
    validate_confirmed_source_refs(&lifecycle, &project_id, &issue_id, &enrollment.source)?;

    // 只绑定显式指定的 plan/session：目标存在、同 issue、类型/策略/身份一致，
    // 且 plan 不早于 enrollment 创建（旧 plan 不能隐式认领）。
    let plan = lifecycle
        .list_issue_work_item_plans(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|plan| plan.id == request.plan_id)
        .ok_or_else(|| binding_target_not_found("plan"))?;
    let session = lifecycle
        .list_workspace_sessions(&project_id, &issue_id)
        .map_err(product_store_api_error)?
        .into_iter()
        .find(|session| session.id == request.session_id)
        .ok_or_else(|| binding_target_not_found("session"))?;
    if session.workspace_type != WorkspaceType::WorkItemPlan {
        return Err(invalid_scope(
            "binding target session must be a work item plan session",
        ));
    }
    if session.entity_id != plan.id {
        return Err(invalid_scope(
            "binding target session entity must match the plan",
        ));
    }
    if session.run_policy != RunPolicy::Interactive {
        // REQ-WIGA-02：AutoIfValid 不代表人工授权。
        return Err(enrollment_conflict(
            None,
            "binding target session must use the interactive run policy",
        ));
    }
    if created_before(&plan.created_at, &enrollment.created_at) {
        return Err(invalid_scope(
            "plan created before the enrollment cannot be claimed implicitly",
        ));
    }

    store
        .bind_plan(
            &project_id,
            &issue_id,
            request.expected_revision,
            &request.plan_id,
            &request.session_id,
        )
        .map(Json)
        .map_err(enrollment_api_error)
}

fn validate_request_ids(project_id: &str, issue_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id)
        .map_err(|_| ApiError::validation("invalid_project_id", "project id must be relative"))?;
    validate_relative_id(issue_id)
        .map_err(|_| ApiError::validation("invalid_issue_id", "issue id must be relative"))?;
    Ok(())
}

fn ensure_issue_exists(state: &WebAppState, project_id: &str, issue_id: &str) -> ApiResult<()> {
    IssueStore::new(product_app_paths(state))
        .get(project_id, issue_id)
        .map(|_| ())
        .map_err(product_store_api_error)
}

/// Enable 前的精确作用域校验；Disable 只要求 enrollment 存在（store NotFound 兜底），
/// 不重验可能已失效的 source。
fn validate_enrollment_scope(
    state: &WebAppState,
    project_id: &str,
    issue_id: &str,
    command: &EnrollmentWriteCommand,
) -> ApiResult<()> {
    ensure_issue_exists(state, project_id, issue_id)?;
    let EnrollmentWriteCommand::Enable {
        source,
        logical_repository_id,
        ..
    } = command
    else {
        return Ok(());
    };

    let paths = product_app_paths(state);
    let lifecycle = LifecycleStore::new(paths.clone());
    validate_confirmed_source_refs(&lifecycle, project_id, issue_id, source)?;

    // 自动授权仅恰一 logical repository/单 attempt（REQ-WIGA-01/02、REQ-MTG-03）。
    let routing = RepositoryRouting::load_for_issue(&paths, project_id, issue_id)
        .map_err(product_store_api_error)?;
    let RepositoryRouting::Logical {
        manifest,
        selection,
    } = routing
    else {
        return Err(invalid_scope(
            "automation enrollment requires exactly one logical repository; issue has no logical codebase routing",
        ));
    };
    let candidate_ids = logical_repository_ids_for_preflight(&manifest, &selection);
    match preflight_single_repository_candidate(&candidate_ids) {
        SingleCandidatePreflightDecision::Eligible { repository_id }
            if repository_id == logical_repository_id.0.to_string() =>
        {
            Ok(())
        }
        SingleCandidatePreflightDecision::Eligible { .. } => Err(invalid_scope(
            "automation enrollment target must match the issue's single logical repository",
        )),
        SingleCandidatePreflightDecision::Ineligible { reason } => Err(invalid_scope(format!(
            "automation enrollment requires exactly one logical repository: {reason}"
        ))),
    }
}

/// 精确 source 绑定（REQ-WIGA-01）：id 存在、已确认、current_version 与引用一致
/// （版本不存在即拒绝，不猜 latest）；design 必须引用 enrollment 的 story。
fn validate_confirmed_source_refs(
    lifecycle: &LifecycleStore,
    project_id: &str,
    issue_id: &str,
    source: &EnrollmentSource,
) -> ApiResult<()> {
    if source.stories.is_empty() || source.designs.is_empty() {
        return Err(invalid_scope(
            "automation enrollment requires confirmed story and design sources",
        ));
    }
    let stories = lifecycle
        .list_story_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    for story_ref in &source.stories {
        let Some(story) = stories.iter().find(|story| story.id == story_ref.id) else {
            return Err(invalid_scope(format!(
                "story spec {} not found for automation enrollment",
                story_ref.id
            )));
        };
        if story.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(invalid_scope(format!(
                "story spec {} must be confirmed before enabling automation",
                story_ref.id
            )));
        }
        if story.current_version != Some(story_ref.version) {
            return Err(invalid_scope(format!(
                "story spec {} version {} does not match current version {:?}; refusing to guess latest",
                story_ref.id, story_ref.version, story.current_version
            )));
        }
    }
    let designs = lifecycle
        .list_design_specs(project_id, issue_id)
        .map_err(product_store_api_error)?;
    let story_ids = source
        .stories
        .iter()
        .map(|story| story.id.as_str())
        .collect::<BTreeSet<_>>();
    for design_ref in &source.designs {
        let Some(design) = designs.iter().find(|design| design.id == design_ref.id) else {
            return Err(invalid_scope(format!(
                "design spec {} not found for automation enrollment",
                design_ref.id
            )));
        };
        if design.confirmation_status != LifecycleConfirmationStatus::Confirmed {
            return Err(invalid_scope(format!(
                "design spec {} must be confirmed before enabling automation",
                design_ref.id
            )));
        }
        if design.current_version != Some(design_ref.version) {
            return Err(invalid_scope(format!(
                "design spec {} version {} does not match current version {:?}; refusing to guess latest",
                design_ref.id, design_ref.version, design.current_version
            )));
        }
        if !design
            .story_spec_ids
            .iter()
            .any(|story_id| story_ids.contains(story_id.as_str()))
        {
            return Err(invalid_scope(format!(
                "design spec {} must reference the enrolled story specs",
                design_ref.id
            )));
        }
    }
    Ok(())
}

fn created_before(plan_created_at: &str, enrollment_created_at: &str) -> bool {
    match (
        DateTime::parse_from_rfc3339(plan_created_at),
        DateTime::parse_from_rfc3339(enrollment_created_at),
    ) {
        (Ok(plan), Ok(enrollment)) => plan < enrollment,
        _ => plan_created_at < enrollment_created_at,
    }
}

fn invalid_scope(reason: impl Into<String>) -> ApiError {
    ApiError::validation("automation_enrollment_invalid_scope", reason)
}

fn enrollment_conflict(current_revision: Option<u64>, message: &str) -> ApiError {
    ApiError::validation_with_details(
        "automation_enrollment_conflict",
        message,
        json!({ "current_revision": current_revision }),
    )
}

fn enrollment_not_found() -> ApiError {
    ApiError::runtime(
        "automation_enrollment_not_found",
        "automation enrollment not found",
        json!({}),
    )
}

fn binding_target_invalid(field: &str, error: ProductStoreError) -> ApiError {
    ApiError::validation(
        "automation_enrollment_invalid_scope",
        format!("binding target {field} is invalid: {error}"),
    )
}

fn binding_target_not_found(kind: &str) -> ApiError {
    ApiError::runtime(
        "automation_enrollment_not_found",
        format!("binding target {kind} not found"),
        json!({}),
    )
}

fn enrollment_api_error(error: EnrollmentError) -> ApiError {
    match error {
        EnrollmentError::Conflict { current_revision } => {
            enrollment_conflict(current_revision, "automation enrollment revision conflict")
        }
        EnrollmentError::InvalidScope(reason) => invalid_scope(reason),
        EnrollmentError::NotFound => enrollment_not_found(),
        EnrollmentError::Store(error) => product_store_api_error(error),
    }
}
#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tempfile::TempDir;
    use tower::ServiceExt;

    use crate::product::app_paths::ProductAppPaths;
    use crate::product::issue_store::{CreateProductIssueInput, IssueStore};
    use crate::product::lifecycle_store::{
        AppendSpecVersionInput, CreateDesignSpecInput, CreateIssueWorkItemPlanInput,
        CreateStorySpecInput, CreateWorkspaceSessionInput, LifecycleStore,
        WorkItemPlanSessionOptions,
    };
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
        AggregateIndexStore,
    };
    use crate::product::logical_codebase::policy::AggregatePolicyArtifactStore;
    use crate::product::logical_codebase::{
        CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, IssueCodebaseSelection,
        IssueCodebaseSelectionStore, LogicalCodebaseManifest, LogicalCodebaseStore,
        LogicalRepositoryId, MemberStatus, RepositoryCheckoutId, RepositoryCheckoutRecord,
        RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::models::{
        IssueWorkItemPlanOptions, IssueWorkItemPlanStatus, LifecycleConfirmationStatus,
        ProviderName, RepositoryRecord, WorkspaceType,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::product::work_item_plan_policy::{RunPolicy, WorkItemPlanFlowKind};
    use crate::web::app::build_web_router;
    use crate::web::runtime::WebRuntime;
    use crate::web::state::WebAppState;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const REPOSITORY_ID: &str = "repo-1";
    const SINGLE_LOGICAL_ID: &str = "00000000-0000-0000-0000-000000000001";

    struct Fixture {
        _root: TempDir,
        paths: ProductAppPaths,
        lifecycle: LifecycleStore,
        story_id: String,
        design_id: String,
    }

    impl Fixture {
        fn router(&self) -> axum::Router {
            let root = self._root.path();
            build_web_router(WebAppState::new(
                root.to_path_buf(),
                WebRuntime::new_fake(root.to_path_buf()),
            ))
        }
    }

    /// 播种：project + issue + N 成员 logical codebase + 已确认 story/design（version 1）。
    fn seed_fixture(member_count: usize, confirm_design: bool) -> Fixture {
        let root = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(root.path().join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "enrollment test project".to_string(),
                description: None,
            })
            .unwrap();
        IssueStore::new(paths.clone())
            .create(CreateProductIssueInput {
                project_id: PROJECT_ID.to_string(),
                repo_id: Some(REPOSITORY_ID.to_string()),
                logical_codebase_id: None,
                title: "enrollment test issue".to_string(),
                description: None,
                change_id: None,
                base_branch: None,
            })
            .unwrap();

        let names = ["checkout-enroll-a", "checkout-enroll-b"];
        let members = names
            .into_iter()
            .take(member_count)
            .enumerate()
            .map(|(index, name)| {
                (
                    LogicalRepositoryId(uuid::Uuid::from_u128(index as u128 + 1)),
                    name,
                )
            })
            .collect::<Vec<_>>();
        if !members.is_empty() {
            seed_logical_codebase(&paths, &members);
        }

        let lifecycle = LifecycleStore::new(paths.clone());
        let story = lifecycle
            .create_story_spec(CreateStorySpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                repository_id: REPOSITORY_ID.to_string(),
                title: "enrollment story".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        lifecycle
            .append_version(AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: story.id.clone(),
                markdown: "# story".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            })
            .unwrap();
        lifecycle
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &story.id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();

        let design = lifecycle
            .create_design_spec(CreateDesignSpecInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                story_spec_ids: vec![story.id.clone()],
                title: "enrollment design".to_string(),
                aggregate_codebase: None,
            })
            .unwrap();
        lifecycle
            .append_version(AppendSpecVersionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: design.id.clone(),
                markdown: "# design".to_string(),
                provider_run_refs: Vec::new(),
                review_refs: Vec::new(),
                confirmed_by: None,
            })
            .unwrap();
        if confirm_design {
            lifecycle
                .update_spec_confirmation_status(
                    PROJECT_ID,
                    ISSUE_ID,
                    &design.id,
                    LifecycleConfirmationStatus::Confirmed,
                )
                .unwrap();
        }

        Fixture {
            _root: root,
            paths,
            lifecycle,
            story_id: story.id,
            design_id: design.id,
        }
    }

    fn seed_logical_codebase(paths: &ProductAppPaths, members: &[(LogicalRepositoryId, &str)]) {
        let manifest = LogicalCodebaseManifest::new(
            PROJECT_ID,
            paths.root().join("aggregate-root"),
            members.iter().map(|(id, _)| *id).collect(),
        );
        LogicalCodebaseStore::new(paths.clone())
            .save_manifest(PROJECT_ID, &manifest)
            .unwrap();
        let now = "2026-09-26T00:00:00Z".to_string();
        let mut repositories = Vec::new();
        for (index, (logical_id, checkout_name)) in members.iter().enumerate() {
            let physical_repository_id = format!("physical-{checkout_name}");
            let checkout_path = paths.root().join(checkout_name);
            let source_identity = RepositorySourceIdentity::from_git_parts(
                &checkout_path,
                checkout_path.join(".git"),
                None,
            );
            let checkout_id = RepositoryCheckoutId(uuid::Uuid::new_v4());
            LogicalCodebaseStore::new(paths.clone())
                .save_member(
                    PROJECT_ID,
                    &CodebaseMemberRecord {
                        logical_repository_id: *logical_id,
                        physical_repository_id: physical_repository_id.clone(),
                        alias: REPOSITORY_ID.to_string(),
                        role: "repository".to_string(),
                        ordinal: (index + 1) as u32,
                        source_identity: source_identity.clone(),
                        repo_type: RepositoryType::Unknown,
                        tech_stack: Vec::new(),
                        owner: None,
                        tags: Vec::new(),
                        default_ref: None,
                        checkout_ids: vec![checkout_id],
                        status: MemberStatus::Active,
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .unwrap();
            LogicalCodebaseStore::new(paths.clone())
                .save_checkout(
                    PROJECT_ID,
                    &RepositoryCheckoutRecord {
                        checkout_id,
                        logical_repository_id: *logical_id,
                        physical_repository_id: physical_repository_id.clone(),
                        kind: CheckoutKind::Main,
                        canonical_path: checkout_path.clone(),
                        checkout_path_hash: format!("sha256:{checkout_name}"),
                        git_dir_identity: source_identity.git_dir_identity(),
                        revision: Some("abcdef".to_string()),
                        availability: CheckoutAvailability::Available,
                        observed_at: now.clone(),
                        created_at: now.clone(),
                        updated_at: now.clone(),
                    },
                )
                .unwrap();
            repositories.push(RepositoryRecord {
                id: physical_repository_id,
                project_id: PROJECT_ID.to_string(),
                name: REPOSITORY_ID.to_string(),
                path: checkout_path,
                repo_hash: format!("sha256:{checkout_name}"),
                runtime_root: paths.root().join(checkout_name).join(".aria/runtime"),
                default_policy_preset: "manual-write".to_string(),
                default_provider_mode: "fake".to_string(),
                created_at: now.clone(),
                logical_repository_id: Some(*logical_id),
                identity_schema_version: 1,
                primary_checkout_id: Some(checkout_id),
                updated_at: now.clone(),
            });
        }
        crate::product::json_store::write_json(
            &paths.project_root(PROJECT_ID).join("repos.json"),
            &repositories,
        )
        .unwrap();
        // active aggregate index + bootstrap policy：prepare 的 planning resolver 必读。
        let mut index = AggregateIndexRecord::building(
            "aggregate_index_0001".to_string(),
            PROJECT_ID.to_string(),
            1,
            members
                .iter()
                .map(|(logical_id, _)| {
                    AggregateIndexMemberSnapshot::indexed(
                        *logical_id,
                        RepositoryCheckoutId(uuid::Uuid::nil()),
                        "abc123".to_string(),
                        false,
                        now.clone(),
                    )
                })
                .collect(),
            now.clone(),
        );
        index.status = AggregateIndexStatus::Active;
        let index_store = AggregateIndexStore::new(paths.clone());
        index_store.create(PROJECT_ID, index.clone()).unwrap();
        index_store.replace_active(PROJECT_ID, index).unwrap();
        AggregatePolicyArtifactStore::new(paths.clone())
            .ensure_bootstrap(&manifest)
            .unwrap();
        IssueCodebaseSelectionStore::new(paths.clone())
            .save(&IssueCodebaseSelection::explicit(
                PROJECT_ID,
                ISSUE_ID,
                members.iter().map(|(id, _)| *id).collect(),
                Vec::new(),
                Vec::new(),
                None,
            ))
            .unwrap();
    }

    fn enrollment_body(
        fixture: &Fixture,
        story_version: u32,
        design_version: u32,
    ) -> serde_json::Value {
        serde_json::json!({
            "expected_revision": null,
            "command": {
                "type": "enable",
                "selection_key": "human-choice-1",
                "source": {
                    "stories": [{"id": fixture.story_id, "version": story_version}],
                    "designs": [{"id": fixture.design_id, "version": design_version}]
                },
                "options": {
                    "author_provider": "fake",
                    "reviewer_provider": "fake",
                    "review_rounds": 1,
                    "superpowers_enabled": false,
                    "openspec_enabled": false,
                    "plan_options": {
                        "include_integration_tests": true,
                        "include_e2e_tests": false,
                        "force_frontend_backend_split": false,
                        "require_execution_plan_confirm": false
                    }
                },
                "logical_repository_id": SINGLE_LOGICAL_ID
            }
        })
    }

    async fn put_enrollment(
        app: &axum::Router,
        body: serde_json::Value,
    ) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment"
                    ))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn get_enrollment(app: &axum::Router) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment"
                    ))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn post_binding(
        app: &axum::Router,
        body: serde_json::Value,
    ) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!(
                        "/api/projects/{PROJECT_ID}/issues/{ISSUE_ID}/automation-enrollment/binding"
                    ))
                    .header("content-type", "application/json")
                    .body(Body::from(serde_json::to_vec(&body).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    /// 直接经 store 显式新建 plan + WorkItemPlan 会话（绑定只认显式对象）。
    fn create_plan_and_session(fixture: &Fixture, run_policy: RunPolicy) -> (String, String) {
        let plan = fixture
            .lifecycle
            .create_issue_work_item_plan(CreateIssueWorkItemPlanInput {
                id: None,
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                source_story_spec_ids: vec![fixture.story_id.clone()],
                source_design_spec_ids: vec![fixture.design_id.clone()],
                options: IssueWorkItemPlanOptions {
                    include_integration_tests: true,
                    include_e2e_tests: false,
                    force_frontend_backend_split: false,
                    require_execution_plan_confirm: false,
                },
                status: IssueWorkItemPlanStatus::Draft,
                work_item_ids: Vec::new(),
                repository_profile_ref: None,
                verification_plan_ids: Vec::new(),
                dependency_graph: Vec::new(),
                created_from_provider_run: None,
                validator_findings: Vec::new(),
            })
            .unwrap();
        let session = fixture
            .lifecycle
            .create_workspace_session(CreateWorkspaceSessionInput {
                project_id: PROJECT_ID.to_string(),
                issue_id: ISSUE_ID.to_string(),
                entity_id: plan.id.clone(),
                workspace_type: WorkspaceType::WorkItemPlan,
                author_provider: ProviderName::Fake,
                reviewer_provider: ProviderName::Fake,
                review_rounds: 1,
                superpowers_enabled: false,
                openspec_enabled: false,
                work_item_plan_options: Some(WorkItemPlanSessionOptions {
                    flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                    run_policy,
                    rollout_snapshot: false,
                }),
            })
            .unwrap();
        (plan.id, session.id)
    }

    async fn response_json(response: axum::http::Response<Body>) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn enrollment_file_exists(fixture: &Fixture) -> bool {
        fixture
            .paths
            .issue_root(PROJECT_ID, ISSUE_ID)
            .join("automation-enrollment.json")
            .exists()
    }

    #[tokio::test]
    async fn automation_enrollment_http_get_returns_null_when_absent() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let response = get_enrollment(&app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert!(payload.is_null());
    }

    #[tokio::test]
    async fn automation_enrollment_http_put_rejects_unconfirmed_or_stale_source() {
        let fixture = seed_fixture(1, false);
        let app = fixture.router();

        // design 未确认 → 422，且不落 enrollment 文件。
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // 确认 design 后，引用不存在的版本（2 > current 1）→ 422，不猜 latest。
        fixture
            .lifecycle
            .update_spec_confirmation_status(
                PROJECT_ID,
                ISSUE_ID,
                &fixture.design_id,
                LifecycleConfirmationStatus::Confirmed,
            )
            .unwrap();
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 2)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // 未知 story id 同样拒绝。
        let mut body = enrollment_body(&fixture, 1, 1);
        body["command"]["source"]["stories"][0]["id"] = serde_json::json!("story_spec_9999");
        let response = put_enrollment(&app, body).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));
    }

    #[tokio::test]
    async fn automation_enrollment_http_put_is_idempotent_and_conflicts_on_divergence() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let first = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(first.status(), StatusCode::OK);
        let first_body = response_json(first).await;
        assert_eq!(first_body["policy_revision"], 1);
        assert!(first_body["enabled"].as_bool().unwrap());

        // 同键同 payload 重发：200，同 enrollment_id/revision。
        let second = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(second.status(), StatusCode::OK);
        let second_body = response_json(second).await;
        assert_eq!(second_body["enrollment_id"], first_body["enrollment_id"]);
        assert_eq!(second_body["policy_revision"], 1);

        // 异 options + 旧 revision → 409，details 携带当前 revision。
        let mut divergent = enrollment_body(&fixture, 1, 1);
        divergent["command"]["options"]["review_rounds"] = serde_json::json!(2);
        divergent["expected_revision"] = serde_json::json!(99);
        let response = put_enrollment(&app, divergent).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_enrollment_conflict");
        assert_eq!(payload["details"]["current_revision"], 1);

        // Disable 后以旧 revision Enable → 409。
        let disable = serde_json::json!({
            "expected_revision": 1,
            "command": {"type": "disable"}
        });
        let response = put_enrollment(&app, disable).await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response_json(response).await["policy_revision"], 2);
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // GET 投影读取当前（禁用）记录。
        let response = get_enrollment(&app).await;
        assert_eq!(response.status(), StatusCode::OK);
        let payload = response_json(response).await;
        assert_eq!(payload["enabled"], false);
        assert_eq!(payload["policy_revision"], 2);
    }

    #[tokio::test]
    async fn automation_enrollment_http_put_rejects_zero_or_multi_target() {
        // 0 logical member（无 manifest/selection → Legacy 路由）→ 422。
        let fixture = seed_fixture(0, true);
        let app = fixture.router();
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        assert!(!enrollment_file_exists(&fixture));

        // 2 logical member → 422 且 enrollment 文件不存在。
        let fixture = seed_fixture(2, true);
        let app = fixture.router();
        let response = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let payload = response_json(response).await;
        assert_eq!(payload["code"], "automation_enrollment_invalid_scope");
        assert!(!enrollment_file_exists(&fixture));
    }

    #[tokio::test]
    async fn automation_enrollment_http_binding_binds_explicit_interactive_plan_session() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let enrollment = response_json(enable).await;
        let revision = enrollment["policy_revision"].as_u64().unwrap();

        // AutoIfValid 会话不得代表授权（REQ-WIGA-02）→ 409。
        let (auto_plan, auto_session) = create_plan_and_session(&fixture, RunPolicy::AutoIfValid);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": auto_plan,
                "session_id": auto_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // Interactive plan/session 显式绑定成功。
        let (plan_id, session_id) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let bound = response_json(response).await;
        assert_eq!(bound["plan_id"], plan_id.as_str());
        assert_eq!(bound["session_id"], session_id.as_str());
        let bound_revision = bound["policy_revision"].as_u64().unwrap();

        // 同键重试 → 200 同 revision；重绑他者 plan → 409。
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": bound_revision,
                "plan_id": plan_id,
                "session_id": session_id,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response_json(response).await["policy_revision"],
            bound_revision
        );

        let (other_plan, other_session) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": bound_revision,
                "plan_id": other_plan,
                "session_id": other_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::CONFLICT);

        // 绑定目标不存在（未知 plan/session）→ 404。
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": bound_revision,
                "plan_id": "plan_9999",
                "session_id": "session_9999",
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn automation_enrollment_http_binding_rejects_plan_created_before_enrollment() {
        let fixture = seed_fixture(1, true);
        let app = fixture.router();

        // 旧 plan（先于 enrollment 创建）不能被认领 → 422。
        let (old_plan, old_session) = create_plan_and_session(&fixture, RunPolicy::Interactive);
        let enable = put_enrollment(&app, enrollment_body(&fixture, 1, 1)).await;
        assert_eq!(enable.status(), StatusCode::OK);
        let revision = response_json(enable).await["policy_revision"]
            .as_u64()
            .unwrap();
        let response = post_binding(
            &app,
            serde_json::json!({
                "expected_revision": revision,
                "plan_id": old_plan,
                "session_id": old_session,
            }),
        )
        .await;
        assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }
}
