//! Task R2（v1.3 §4）：统一 codebases 列表 + 逻辑代码库 CRUD。
//!
//! 单仓零变化：`GET /codebases` 的单仓条目只是 repos.json 的呈现层投影；逻辑条目
//! 来自 LogicalCodebaseStore，member_count 从 manifest active 成员计。
use super::support::*;
use super::*;

use crate::product::logical_codebase::{
    CodebaseMemberRecord, LogicalCodebaseCreateInput, LogicalCodebaseFeature,
    LogicalCodebaseManifest, LogicalCodebaseRecord, LogicalCodebaseStore, MemberStatus,
};
use crate::product::project_store::ProjectStore;
use crate::product::repository_store::RepositoryStore;
use crate::web::types::{
    CodebaseListResponse, CodebaseSummaryDto, CreateLogicalCodebaseRequest,
    LogicalCodebaseDetailDto, LogicalCodebaseDto, LogicalCodebaseMemberDto,
};

pub async fn list_codebases(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<CodebaseListResponse>> {
    let app_paths = product_app_paths(&state);
    ProjectStore::new(app_paths.clone())
        .get(&project_id)
        .map_err(product_store_api_error)?;
    let mut codebases = Vec::new();
    // 单仓零变化：禁用逻辑 feature，list 退化为纯 repos.json 只读投影，绝不触发
    // v1.2 的懒情 identity-schema 迁移（否则 LC 存在后会改写 repos.json）。
    for repository in RepositoryStore::with_logical_codebase_feature(
        app_paths.clone(),
        LogicalCodebaseFeature::disabled(),
    )
    .list(&project_id)
    .map_err(product_store_api_error)?
    {
        codebases.push(CodebaseSummaryDto {
            id: repository.id.clone(),
            name: repository.name,
            kind: "single_repo".to_string(),
            repository_id: Some(repository.id),
            logical_codebase_id: None,
            member_count: None,
        });
    }
    let authority = LogicalCodebaseStore::new(app_paths);
    for record in authority
        .list(&project_id)
        .map_err(product_store_api_error)?
    {
        let member_count = active_member_count(&authority, &project_id, &record.id)
            .map_err(product_store_api_error)?;
        codebases.push(CodebaseSummaryDto {
            id: record.id.clone(),
            name: record.name,
            kind: "logical".to_string(),
            repository_id: None,
            logical_codebase_id: Some(record.id),
            member_count: Some(member_count),
        });
    }
    Ok(Json(CodebaseListResponse { codebases }))
}

pub async fn create_logical_codebase(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
    Json(request): Json<CreateLogicalCodebaseRequest>,
) -> ApiResult<Json<LogicalCodebaseDto>> {
    if request.name.trim().is_empty() {
        return Err(ApiError::validation(
            "logical_codebase_name_required",
            "name must not be empty",
        ));
    }
    if request.aggregate_root.trim().is_empty() {
        return Err(ApiError::validation(
            "aggregate_root_required",
            "aggregate_root must not be empty",
        ));
    }
    let app_paths = product_app_paths(&state);
    ProjectStore::new(app_paths.clone())
        .get(&project_id)
        .map_err(product_store_api_error)?;
    // 创建空子树：manifest 待首批登记原子创建（D7 不变）。
    let record = LogicalCodebaseStore::new(app_paths)
        .create(
            &project_id,
            LogicalCodebaseCreateInput {
                name: request.name,
                aggregate_root: PathBuf::from(request.aggregate_root),
            },
        )
        .map_err(product_store_api_error)?;
    Ok(Json(logical_codebase_dto(record)))
}

pub async fn get_logical_codebase(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Json<LogicalCodebaseDetailDto>> {
    let app_paths = product_app_paths(&state);
    ProjectStore::new(app_paths.clone())
        .get(&project_id)
        .map_err(product_store_api_error)?;
    let authority = LogicalCodebaseStore::new(app_paths);
    let record = authority
        .get(&project_id, &logical_codebase_id)
        .map_err(product_store_api_error)?
        .ok_or_else(logical_codebase_not_found)?;
    let manifest = authority
        .load_lc_manifest(&project_id, &logical_codebase_id)
        .map_err(product_store_api_error)?;
    let members = authority
        .list_lc_members(&project_id, &logical_codebase_id)
        .map_err(product_store_api_error)?;
    let active_members = active_members(&manifest, &members);
    Ok(Json(LogicalCodebaseDetailDto {
        id: record.id,
        name: record.name,
        aggregate_root: record.aggregate_root.to_string_lossy().into_owned(),
        created_at: record.created_at,
        manifest_present: manifest.is_some(),
        membership_revision: manifest.as_ref().map(|value| value.membership_revision),
        active_aggregate_index_id: manifest
            .as_ref()
            .and_then(|value| value.active_aggregate_index_id.clone()),
        member_count: active_members.len(),
        members: members
            .into_iter()
            .map(logical_codebase_member_dto)
            .collect(),
    }))
}

pub async fn delete_logical_codebase(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Json<serde_json::Value>> {
    let app_paths = product_app_paths(&state);
    ProjectStore::new(app_paths.clone())
        .get(&project_id)
        .map_err(product_store_api_error)?;
    // 软删/tombstone：成员仓零 git 副作用；不存在 → 404 logical_codebase_not_found。
    LogicalCodebaseStore::new(app_paths)
        .delete_soft(&project_id, &logical_codebase_id)
        .map_err(product_store_api_error)?;
    Ok(Json(json!({"status":"deleted"})))
}

fn logical_codebase_not_found() -> ApiError {
    ApiError::runtime(
        "logical_codebase_not_found",
        "logical codebase not found",
        json!({}),
    )
}

fn logical_codebase_dto(record: LogicalCodebaseRecord) -> LogicalCodebaseDto {
    LogicalCodebaseDto {
        id: record.id,
        name: record.name,
        aggregate_root: record.aggregate_root.to_string_lossy().into_owned(),
        created_at: record.created_at,
    }
}

fn logical_codebase_member_dto(member: CodebaseMemberRecord) -> LogicalCodebaseMemberDto {
    LogicalCodebaseMemberDto {
        logical_repository_id: member.logical_repository_id.0.to_string(),
        physical_repository_id: member.physical_repository_id,
        alias: member.alias,
        status: member_status_text(member.status).to_string(),
        ordinal: member.ordinal,
        created_at: member.created_at,
        updated_at: member.updated_at,
    }
}

fn member_status_text(status: MemberStatus) -> &'static str {
    match status {
        MemberStatus::Active => "active",
        MemberStatus::Removed => "removed",
        MemberStatus::Tombstoned => "tombstoned",
    }
}

/// member_count 从 manifest active 成员计：manifest 缺失（待首批登记创建）→ 0。
fn active_member_count(
    authority: &LogicalCodebaseStore,
    project_id: &str,
    logical_codebase_id: &str,
) -> Result<usize, crate::product::json_store::ProductStoreError> {
    let manifest = authority.load_lc_manifest(project_id, logical_codebase_id)?;
    let members = authority.list_lc_members(project_id, logical_codebase_id)?;
    Ok(active_members(&manifest, &members).len())
}

fn active_members<'a>(
    manifest: &'a Option<LogicalCodebaseManifest>,
    members: &'a [CodebaseMemberRecord],
) -> Vec<&'a CodebaseMemberRecord> {
    let Some(manifest) = manifest else {
        return Vec::new();
    };
    members
        .iter()
        .filter(|member| {
            member.status == MemberStatus::Active
                && manifest.member_ids.contains(&member.logical_repository_id)
        })
        .collect()
}
