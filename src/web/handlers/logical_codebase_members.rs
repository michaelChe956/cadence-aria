use axum::Json;
use axum::extract::{Path, State};
use serde::Serialize;
use std::collections::HashMap;

use super::aggregate_index::{is_current_main_checkout, projected_record, skipped_member_names};
use super::support::{
    default_logical_codebase_id, product_app_paths, product_store_api_error, resolve_lc_authority,
};
use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::validate_relative_id;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexRecord, AggregateIndexStatus, AggregateIndexStore,
};
use crate::product::logical_codebase::failure_copy::{
    MEMBER_SOURCE_LESS_CODE, SourceLessAlert, member_source_less_member_copy,
};
use crate::product::logical_codebase::types::CodebaseMemberRecord;
use crate::product::logical_codebase::{LogicalCodebaseStore, LogicalRepositoryId, MemberStatus};
use crate::web::error::{ApiError, ApiResult};
use crate::web::state::WebAppState;

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseMemberDto {
    pub logical_repository_id: String,
    /// R8 契约补充：issue 创建对话框 primary 成员选择所需（server 侧恒 Some；
    /// Option 仅用于向前兼容旧客户端解码与未来未投影场景）。
    pub physical_repository_id: Option<String>,
    pub alias: String,
    pub status: MemberStatus,
    /// 已验收聚合索引世代确认「无源码、未纳入代码搜索」时的**结构化**告警
    /// （与卡片 `source_less_alert`、证据 `not_searched` 同构）。
    ///
    /// `None` = 无该事实（无可读世代 / 世代里没有跳过事实 / 旧世代 /
    /// 快照 checkout 已不是该成员当前可用的 Main 检出 / 读记录失败）。
    /// **不承载任何选择资格**：复选、提交、禁用、主仓校验一律不受它影响。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub index_alert: Option<SourceLessAlert>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseMembersResponse {
    pub members: Vec<LogicalCodebaseMemberDto>,
}

pub async fn list_logical_codebase_members(
    State(state): State<WebAppState>,
    Path(project_id): Path<String>,
) -> ApiResult<Json<LogicalCodebaseMembersResponse>> {
    let paths = product_app_paths(&state);
    let logical_codebase_id = default_logical_codebase_id(&paths, &project_id)?;
    list_logical_codebase_members_for_lc(&state, &project_id, &logical_codebase_id)
}

/// v1.3 canonical endpoint: members are resolved per logical codebase.
pub async fn list_lc_logical_codebase_members(
    State(state): State<WebAppState>,
    Path((project_id, logical_codebase_id)): Path<(String, String)>,
) -> ApiResult<Json<LogicalCodebaseMembersResponse>> {
    let paths = product_app_paths(&state);
    // C4 Task 2：canonical 路由先经唯一 authority resolver 冻结 LC 身份
    //（conflict fail-closed；只读列表不触发写入）。
    resolve_lc_authority(&paths, &project_id, &logical_codebase_id)?;
    list_logical_codebase_members_for_lc(&state, &project_id, &logical_codebase_id)
}

fn list_logical_codebase_members_for_lc(
    state: &WebAppState,
    project_id: &str,
    logical_codebase_id: &str,
) -> ApiResult<Json<LogicalCodebaseMembersResponse>> {
    validate_project_id(project_id)?;
    let paths = product_app_paths(state);
    let store = LogicalCodebaseStore::new(paths.clone());
    if store
        .load_lc_manifest(project_id, logical_codebase_id)
        .map_err(product_store_api_error)?
        .is_none()
    {
        return Ok(Json(LogicalCodebaseMembersResponse {
            members: Vec::new(),
        }));
    }
    let members = store
        .list_lc_members(project_id, logical_codebase_id)
        .map_err(product_store_api_error)?;
    // 标注是**附加信息**：只读 durable 世代与成员/检出记录（不扫盘、不启
    // CodeGraph、不写任何状态），且不改任何选择资格字段。
    let alerts = source_less_alerts(&paths, project_id, logical_codebase_id, &members);
    let members = members
        .into_iter()
        .map(|member| LogicalCodebaseMemberDto {
            logical_repository_id: member.logical_repository_id.0.to_string(),
            physical_repository_id: Some(member.physical_repository_id),
            alias: member.alias,
            status: member.status,
            index_alert: alerts.get(&member.logical_repository_id).cloned(),
        })
        .collect();
    Ok(Json(LogicalCodebaseMembersResponse { members }))
}

/// 该 LC 的 durable 世代记录（按 `updated_at` / `aggregate_index_id` 降序）。
///
/// 只读盘上已有记录：读不到 ⇒ 空集（降级为空，不报错、不猜）。
fn generations(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
) -> Vec<AggregateIndexRecord> {
    let mut records = AggregateIndexStore::for_lc(paths.clone(), logical_codebase_id)
        .records(project_id)
        .unwrap_or_default();
    records.retain(|record| record.status != AggregateIndexStatus::Superseded);
    records.sort_by(|left, right| {
        right
            .updated_at
            .cmp(&left.updated_at)
            .then_with(|| right.aggregate_index_id.cmp(&left.aggregate_index_id))
    });
    records
}

/// 逐仓结构化标注（REQ-MRE-01）：被投影世代里「经盘检确认无源码」的成员，
/// 按（成员 ID，**当前**可用的 Main checkout ID）双键匹配快照才贴标。
///
/// 口径**全部复用卡片投影的共享实现**（片5 `projected_record` /
/// `skipped_member_names` / `is_current_main_checkout`）：不得在本文件另写
/// 一套「谁算被跳过」的判定，避免两处漂移。
///
/// 只读既有 durable 记录与成员/检出记录：不盘检、不启动 CodeGraph、不写状态。
/// 世代缺失 / 无跳过事实 / 快照 checkout 已更换 / 记录读不到 ⇒ 不标注。
fn source_less_alerts(
    paths: &ProductAppPaths,
    project_id: &str,
    logical_codebase_id: &str,
    members: &[CodebaseMemberRecord],
) -> HashMap<LogicalRepositoryId, SourceLessAlert> {
    let mut alerts = HashMap::new();
    let records = generations(paths, project_id, logical_codebase_id);
    // 世代选择与卡片投影同一规则（`projected_record`）：不得自写「Active 优先」。
    let Some(projected) = projected_record(&records) else {
        return alerts;
    };
    // 共享口径：被跳过的成员名单（成员/检出读不到 ⇒ 空集，不报错）。
    let skipped = skipped_member_names(paths, project_id, logical_codebase_id, Some(projected))
        .unwrap_or_default();
    if skipped.is_empty() {
        return alerts;
    }
    // 精确到成员行：快照上的跳过事实 + 该成员当前可用的 Main 检出（共享判定）。
    let store = LogicalCodebaseStore::for_lc(paths.clone(), logical_codebase_id);
    for member in members {
        if !skipped.iter().any(|alias| alias == &member.alias) {
            continue;
        }
        let skipped_by_snapshot = projected.member_snapshots.iter().any(|snapshot| {
            snapshot.logical_repository_id == member.logical_repository_id
                && snapshot.skip_reason.is_some()
                && is_current_main_checkout(&store, project_id, member, snapshot.checkout_id)
        });
        if !skipped_by_snapshot {
            continue;
        }
        alerts.insert(
            member.logical_repository_id,
            SourceLessAlert {
                code: MEMBER_SOURCE_LESS_CODE.to_string(),
                message_zh: member_source_less_member_copy(&member.alias),
            },
        );
    }
    alerts
}

fn validate_project_id(project_id: &str) -> ApiResult<()> {
    validate_relative_id(project_id).map_err(|error| {
        ApiError::validation("invalid_project_id", format!("invalid project id: {error}"))
    })
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use crate::product::logical_codebase::aggregate_index::{
        AggregateIndexMemberSnapshot, AggregateIndexRecord, AggregateIndexStatus,
        AggregateIndexStore,
    };
    use crate::product::logical_codebase::types::{
        CheckoutAvailability, CheckoutKind, RepositoryCheckoutRecord,
    };
    use crate::product::logical_codebase::{
        CodebaseMemberRecord, LogicalCodebaseManifest, LogicalCodebaseStore, LogicalRepositoryId,
        MemberStatus, RepositoryCheckoutId, RepositorySourceIdentity, RepositoryType,
    };
    use crate::product::project_store::{CreateProjectInput, ProjectStore};
    use crate::web::app::build_web_router;
    use crate::web::runtime::WebRuntime;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::Value;
    use tempfile::tempdir;
    use tower::ServiceExt;
    use uuid::Uuid;

    fn seed_lc(root: &std::path::Path) -> (ProductAppPaths, String) {
        let paths = ProductAppPaths::new(root.join(".aria"));
        ProjectStore::new(paths.clone())
            .create(CreateProjectInput {
                name: "members test".to_string(),
                description: None,
            })
            .unwrap();
        let record = LogicalCodebaseStore::new(paths.clone())
            .create(
                "project_0001",
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "Platform".to_string(),
                    aggregate_root: root.join("aggregate-root"),
                },
            )
            .unwrap();
        (paths, record.id)
    }

    async fn get(app: &axum::Router, uri: &str) -> axum::http::Response<Body> {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("GET")
                    .uri(uri)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    async fn body_json(response: axum::http::Response<Body>) -> Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    fn app(root: &std::path::Path) -> axum::Router {
        let root = root.to_path_buf();
        build_web_router(WebAppState::new(root.clone(), WebRuntime::new_fake(root)))
    }

    fn member(id: LogicalRepositoryId, alias: &str, status: MemberStatus) -> CodebaseMemberRecord {
        CodebaseMemberRecord {
            logical_repository_id: id,
            physical_repository_id: format!("physical-{alias}"),
            alias: alias.to_string(),
            role: "service".to_string(),
            ordinal: 1,
            source_identity: RepositorySourceIdentity {
                scheme: "test".to_string(),
                key_digest: format!("digest-{alias}"),
                canonical_git_dir: "/tmp/test/.git".into(),
                canonical_origin: None,
                first_seen_path_hash: format!("hash-{alias}"),
            },
            repo_type: RepositoryType::Unknown,
            tech_stack: Vec::new(),
            owner: None,
            tags: Vec::new(),
            default_ref: None,
            checkout_ids: Vec::new(),
            status,
            created_at: "2026-08-14T00:00:00Z".to_string(),
            updated_at: "2026-08-14T00:00:00Z".to_string(),
        }
    }

    #[tokio::test]
    async fn list_members_lc_without_manifest_returns_empty_array() {
        let root = tempdir().unwrap();
        let (paths, lc_id) = seed_lc(root.path());
        let store = LogicalCodebaseStore::for_lc(paths, lc_id.clone());
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        store
            .save_member(
                "project_0001",
                &member(logical_repository_id, "orphan", MemberStatus::Active),
            )
            .unwrap();
        let response = get(
            &app(root.path()),
            &format!("/api/projects/project_0001/logical-codebases/{lc_id}/members"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            serde_json::json!({ "members": [] })
        );
    }

    #[tokio::test]
    async fn list_members_lc_with_manifest_projects_projection_fields() {
        let root = tempdir().unwrap();
        let (paths, lc_id) = seed_lc(root.path());
        let logical_repository_id = LogicalRepositoryId(Uuid::new_v4());
        let store = LogicalCodebaseStore::for_lc(paths, lc_id.clone());
        store
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    root.path().join("aggregate-root"),
                    vec![logical_repository_id],
                ),
            )
            .unwrap();
        store
            .save_member(
                "project_0001",
                &member(logical_repository_id, "api", MemberStatus::Removed),
            )
            .unwrap();

        let response = get(
            &app(root.path()),
            &format!("/api/projects/project_0001/logical-codebases/{lc_id}/members"),
        )
        .await;

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            body_json(response).await,
            serde_json::json!({
                "members": [{
                    "logical_repository_id": logical_repository_id.0.to_string(),
                    "physical_repository_id": "physical-api",
                    "alias": "api",
                    "status": "removed"
                }]
            })
        );
    }

    /// 成员夹具：挂一个当前检出。
    fn member_with_checkout(
        id: LogicalRepositoryId,
        alias: &str,
        checkout: RepositoryCheckoutId,
    ) -> CodebaseMemberRecord {
        let mut record = member(id, alias, MemberStatus::Active);
        record.checkout_ids = vec![checkout];
        record
    }

    /// 成员的 Main 检出记录（可用性由调用方给）。
    fn main_checkout(
        member: LogicalRepositoryId,
        checkout: RepositoryCheckoutId,
        availability: CheckoutAvailability,
    ) -> RepositoryCheckoutRecord {
        RepositoryCheckoutRecord {
            checkout_id: checkout,
            logical_repository_id: member,
            physical_repository_id: "physical-fixture".to_string(),
            kind: CheckoutKind::Main,
            canonical_path: std::path::PathBuf::from("/tmp/fixture-checkout"),
            checkout_path_hash: "fixture-hash".to_string(),
            git_dir_identity: "fixture-identity".to_string(),
            revision: None,
            availability,
            observed_at: "2026-11-18T00:00:00Z".to_string(),
            created_at: "2026-11-18T00:00:00Z".to_string(),
            updated_at: "2026-11-18T00:00:00Z".to_string(),
        }
    }

    /// 聚合索引成员快照（`source_less` = 经盘检确认无源码）。
    fn snapshot(
        member: LogicalRepositoryId,
        checkout: RepositoryCheckoutId,
        source_less: bool,
    ) -> AggregateIndexMemberSnapshot {
        let mut snapshot = AggregateIndexMemberSnapshot::indexed(
            member,
            checkout,
            "a".repeat(40),
            false,
            "2026-11-18T00:00:00Z".to_string(),
        );
        if source_less {
            snapshot.mark_source_less();
        }
        snapshot
    }

    /// 一代聚合索引记录（状态与时间由调用方给）。
    fn generation(
        aggregate_index_id: &str,
        status: AggregateIndexStatus,
        updated_at: &str,
        snapshots: Vec<AggregateIndexMemberSnapshot>,
    ) -> AggregateIndexRecord {
        let mut record = AggregateIndexRecord::building(
            aggregate_index_id.to_string(),
            "project_0001".to_string(),
            3,
            snapshots,
            updated_at.to_string(),
        );
        record.status = status;
        record
    }

    /// 落盘 LC manifest + 成员 + 检出（聚合索引世代另行落盘）。
    fn seed_members(
        paths: &ProductAppPaths,
        root: &std::path::Path,
        lc_id: &str,
        members: &[CodebaseMemberRecord],
        checkouts: &[RepositoryCheckoutRecord],
    ) {
        let store = LogicalCodebaseStore::for_lc(paths.clone(), lc_id);
        store
            .save_manifest(
                "project_0001",
                &LogicalCodebaseManifest::new(
                    "project_0001",
                    root.join("aggregate-root"),
                    members
                        .iter()
                        .map(|member| member.logical_repository_id)
                        .collect(),
                ),
            )
            .unwrap();
        for member in members {
            store.save_member("project_0001", member).unwrap();
        }
        for checkout in checkouts {
            store.save_checkout("project_0001", checkout).unwrap();
        }
    }

    /// 在临时目录里递归找夹具落盘文件（纯测试辅助）。
    fn find_json(root: &std::path::Path, file: &str) -> std::path::PathBuf {
        fn walk(dir: &std::path::Path, file: &str) -> Option<std::path::PathBuf> {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    if let Some(hit) = walk(&path, file) {
                        return Some(hit);
                    }
                } else if path.file_name().and_then(|name| name.to_str()) == Some(file) {
                    return Some(path);
                }
            }
            None
        }
        walk(root, file).expect("fixture json must exist on disk")
    }

    async fn members_body(root: &std::path::Path, lc_id: &str) -> Value {
        let response = get(
            &app(root),
            &format!("/api/projects/project_0001/logical-codebases/{lc_id}/members"),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        body_json(response).await
    }

    /// #1 被跳过的成员 ⇒ 带结构化告警（点名成员）；未被跳过的成员不带。
    #[tokio::test]
    async fn members_endpoint_annotates_skipped_member_with_structured_alert() {
        let root = tempdir().unwrap();
        let (paths, lc_id) = seed_lc(root.path());
        let member_api = LogicalRepositoryId(Uuid::new_v4());
        let member_gray = LogicalRepositoryId(Uuid::new_v4());
        let checkout_api = RepositoryCheckoutId(Uuid::new_v4());
        let checkout_gray = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths,
            root.path(),
            &lc_id,
            &[
                member_with_checkout(member_api, "api", checkout_api),
                member_with_checkout(member_gray, "gray-config", checkout_gray),
            ],
            &[
                main_checkout(member_api, checkout_api, CheckoutAvailability::Available),
                main_checkout(member_gray, checkout_gray, CheckoutAvailability::Available),
            ],
        );
        AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
            .create(
                "project_0001",
                generation(
                    "aggregate_index_gen",
                    AggregateIndexStatus::Active,
                    "2026-11-18T01:00:00Z",
                    vec![
                        snapshot(member_api, checkout_api, false),
                        snapshot(member_gray, checkout_gray, true),
                    ],
                ),
            )
            .unwrap();

        let body = members_body(root.path(), &lc_id).await;
        let members = body["members"].as_array().expect("成员列表");
        let gray = members
            .iter()
            .find(|member| member["alias"] == "gray-config")
            .expect("被跳过的成员必须在列表里");
        assert_eq!(
            gray["index_alert"]["code"], "aggregate_index_member_source_less",
            "跳过事实必须落成结构化告警：{}",
            gray["index_alert"]
        );
        let message = gray["index_alert"]["message_zh"]
            .as_str()
            .unwrap_or_default();
        assert!(
            message.contains("gray-config"),
            "告警必须点名成员：{message}"
        );
        assert!(
            message.contains("未纳入代码搜索"),
            "告警文案取自服务端成品：{message}"
        );
        let api = members
            .iter()
            .find(|member| member["alias"] == "api")
            .expect("正常成员必须在列表里");
        assert!(
            api["index_alert"].is_null(),
            "未被跳过的成员不得带标注：{}",
            api["index_alert"]
        );
    }

    /// #2 没有任何世代 ⇒ 不标注，且成员列表既有字段完全不变。
    #[tokio::test]
    async fn members_endpoint_omits_alert_without_accepted_generation() {
        let root = tempdir().unwrap();
        let (paths, lc_id) = seed_lc(root.path());
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths,
            root.path(),
            &lc_id,
            &[member_with_checkout(member_id, "api", checkout_id)],
            &[main_checkout(
                member_id,
                checkout_id,
                CheckoutAvailability::Available,
            )],
        );

        assert_eq!(
            members_body(root.path(), &lc_id).await,
            serde_json::json!({
                "members": [{
                    "logical_repository_id": member_id.0.to_string(),
                    "physical_repository_id": "physical-api",
                    "alias": "api",
                    "status": "active"
                }]
            }),
            "无任何世代 ⇒ 成员列表与既有字段一字不变"
        );
    }

    /// #3 世代存在但没有跳过事实 ⇒ 不标注。
    #[tokio::test]
    async fn members_endpoint_omits_alert_for_non_skipped_members() {
        let root = tempdir().unwrap();
        let (paths, lc_id) = seed_lc(root.path());
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths,
            root.path(),
            &lc_id,
            &[member_with_checkout(member_id, "api", checkout_id)],
            &[main_checkout(
                member_id,
                checkout_id,
                CheckoutAvailability::Available,
            )],
        );
        AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
            .create(
                "project_0001",
                generation(
                    "aggregate_index_gen",
                    AggregateIndexStatus::Active,
                    "2026-11-18T01:00:00Z",
                    vec![snapshot(member_id, checkout_id, false)],
                ),
            )
            .unwrap();

        let body = members_body(root.path(), &lc_id).await;
        assert!(
            body["members"][0]["index_alert"].is_null(),
            "世代里没有跳过事实 ⇒ 不标注：{}",
            body["members"][0]["index_alert"]
        );
    }

    /// #4 标注是附加信息：带标注与不带标注的成员都照旧可选（状态不变、都在列表里）。
    #[tokio::test]
    async fn members_endpoint_keeps_every_member_selectable_regardless_of_alert() {
        let root = tempdir().unwrap();
        let (paths, lc_id) = seed_lc(root.path());
        let member_api = LogicalRepositoryId(Uuid::new_v4());
        let member_gray = LogicalRepositoryId(Uuid::new_v4());
        let checkout_api = RepositoryCheckoutId(Uuid::new_v4());
        let checkout_gray = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths,
            root.path(),
            &lc_id,
            &[
                member_with_checkout(member_api, "api", checkout_api),
                member_with_checkout(member_gray, "gray-config", checkout_gray),
            ],
            &[
                main_checkout(member_api, checkout_api, CheckoutAvailability::Available),
                main_checkout(member_gray, checkout_gray, CheckoutAvailability::Available),
            ],
        );
        AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
            .create(
                "project_0001",
                generation(
                    "aggregate_index_gen",
                    AggregateIndexStatus::Active,
                    "2026-11-18T01:00:00Z",
                    vec![
                        snapshot(member_api, checkout_api, false),
                        snapshot(member_gray, checkout_gray, true),
                    ],
                ),
            )
            .unwrap();

        let body = members_body(root.path(), &lc_id).await;
        let members = body["members"].as_array().expect("成员列表");
        assert_eq!(members.len(), 2, "带标注的成员不得被过滤掉");
        for member in members {
            assert_eq!(member["status"], "active", "标注不得改变成员资格：{member}");
        }
        let gray = members
            .iter()
            .find(|member| member["alias"] == "gray-config")
            .expect("被跳过的成员必须在列表里");
        assert!(
            !gray["index_alert"].is_null(),
            "被跳过的成员必须带标注：{gray}"
        );
        assert_eq!(gray["physical_repository_id"], "physical-gray-config");
    }

    /// #5 标注按 LC 作用域隔离：查 LC-2 不得出现 LC-1 的标注。
    #[tokio::test]
    async fn members_endpoint_scopes_alert_to_the_current_logical_codebase() {
        let root = tempdir().unwrap();
        let (paths, lc_one) = seed_lc(root.path());
        let lc_two = LogicalCodebaseStore::new(paths.clone())
            .create(
                "project_0001",
                crate::product::logical_codebase::LogicalCodebaseCreateInput {
                    name: "Secondary".to_string(),
                    aggregate_root: root.path().join("secondary-root"),
                },
            )
            .unwrap()
            .id;

        let member_one = LogicalRepositoryId(Uuid::new_v4());
        let checkout_one = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths,
            root.path(),
            &lc_one,
            &[member_with_checkout(
                member_one,
                "gray-config",
                checkout_one,
            )],
            &[main_checkout(
                member_one,
                checkout_one,
                CheckoutAvailability::Available,
            )],
        );
        AggregateIndexStore::for_lc(paths.clone(), lc_one.clone())
            .create(
                "project_0001",
                generation(
                    "aggregate_index_lc_one",
                    AggregateIndexStatus::Active,
                    "2026-11-18T01:00:00Z",
                    vec![snapshot(member_one, checkout_one, true)],
                ),
            )
            .unwrap();

        let member_two = LogicalRepositoryId(Uuid::new_v4());
        let checkout_two = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths,
            root.path(),
            &lc_two,
            &[member_with_checkout(member_two, "api", checkout_two)],
            &[main_checkout(
                member_two,
                checkout_two,
                CheckoutAvailability::Available,
            )],
        );
        AggregateIndexStore::for_lc(paths.clone(), lc_two.clone())
            .create(
                "project_0001",
                generation(
                    "aggregate_index_lc_two",
                    AggregateIndexStatus::Active,
                    "2026-11-18T01:00:00Z",
                    vec![snapshot(member_two, checkout_two, false)],
                ),
            )
            .unwrap();

        let body_two = members_body(root.path(), &lc_two).await;
        assert!(
            body_two["members"][0]["index_alert"].is_null(),
            "另一个 LC 的标注不得串过来：{}",
            body_two["members"][0]["index_alert"]
        );
        let body_one = members_body(root.path(), &lc_one).await;
        assert!(
            !body_one["members"][0]["index_alert"].is_null(),
            "本 LC 的标注必须存在：{}",
            body_one["members"][0]["index_alert"]
        );
    }

    /// #6 成员主 checkout 已更换 ⇒ 不标注（不得按裸成员 ID 贴标）。
    #[tokio::test]
    async fn members_endpoint_omits_alert_when_main_checkout_changed() {
        let root = tempdir().unwrap();
        let (paths, lc_id) = seed_lc(root.path());
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let old_checkout = RepositoryCheckoutId(Uuid::new_v4());
        let new_checkout = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths,
            root.path(),
            &lc_id,
            &[member_with_checkout(member_id, "gray-config", new_checkout)],
            &[
                main_checkout(member_id, old_checkout, CheckoutAvailability::Available),
                main_checkout(member_id, new_checkout, CheckoutAvailability::Available),
            ],
        );
        AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
            .create(
                "project_0001",
                generation(
                    "aggregate_index_gen",
                    AggregateIndexStatus::Active,
                    "2026-11-18T01:00:00Z",
                    vec![snapshot(member_id, old_checkout, true)],
                ),
            )
            .unwrap();

        let body = members_body(root.path(), &lc_id).await;
        assert!(
            body["members"][0]["index_alert"].is_null(),
            "快照的 checkout 已不是当前 Main 检出 ⇒ 不标注：{}",
            body["members"][0]["index_alert"]
        );
    }

    /// #7 老世代（盘上无 `skip_reason` 键）⇒ 可读且不标注。
    #[tokio::test]
    async fn members_endpoint_omits_alert_for_legacy_generation() {
        let root = tempdir().unwrap();
        let (paths, lc_id) = seed_lc(root.path());
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths,
            root.path(),
            &lc_id,
            &[member_with_checkout(member_id, "gray-config", checkout_id)],
            &[main_checkout(
                member_id,
                checkout_id,
                CheckoutAvailability::Available,
            )],
        );
        AggregateIndexStore::for_lc(paths.clone(), lc_id.clone())
            .create(
                "project_0001",
                generation(
                    "aggregate_index_legacy_gen",
                    AggregateIndexStatus::Active,
                    "2026-11-18T01:00:00Z",
                    vec![snapshot(member_id, checkout_id, false)],
                ),
            )
            .unwrap();
        // 夹具自证：盘上 JSON 是老形态（无 `skip_reason` 键）。
        let disk =
            std::fs::read_to_string(find_json(root.path(), "aggregate_index_legacy_gen.json"))
                .unwrap();
        assert!(
            !disk.contains("skip_reason"),
            "夹具必须是老形态 JSON：{disk}"
        );

        let body = members_body(root.path(), &lc_id).await;
        assert!(
            body["members"][0]["index_alert"].is_null(),
            "老世代没有跳过事实 ⇒ 不标注：{}",
            body["members"][0]["index_alert"]
        );
    }

    /// #8 标注只取「当前被投影的那一份」世代（与卡片投影同一选择规则）：
    /// 更新但已 Superseded 的世代 + 较旧的 Degraded LKG ⇒ 取 LKG；
    /// 最新 Building（快照无跳过）+ 较旧 Degraded ⇒ 取 Building（无标注）。
    #[tokio::test]
    async fn members_endpoint_uses_the_shared_projected_record_rule() {
        let root = tempdir().unwrap();
        let (paths, lc_id) = seed_lc(root.path());
        let member_id = LogicalRepositoryId(Uuid::new_v4());
        let checkout_id = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths,
            root.path(),
            &lc_id,
            &[member_with_checkout(member_id, "gray-config", checkout_id)],
            &[main_checkout(
                member_id,
                checkout_id,
                CheckoutAvailability::Available,
            )],
        );
        let store = AggregateIndexStore::for_lc(paths.clone(), lc_id.clone());
        store
            .create(
                "project_0001",
                generation(
                    "aggregate_index_lkg",
                    AggregateIndexStatus::Degraded,
                    "2026-11-18T01:00:00Z",
                    vec![snapshot(member_id, checkout_id, true)],
                ),
            )
            .unwrap();
        store
            .create(
                "project_0001",
                generation(
                    "aggregate_index_superseded",
                    AggregateIndexStatus::Superseded,
                    "2026-11-18T02:00:00Z",
                    Vec::new(),
                ),
            )
            .unwrap();

        let body = members_body(root.path(), &lc_id).await;
        assert_eq!(
            body["members"][0]["index_alert"]["code"], "aggregate_index_member_source_less",
            "标注必须来自被投影的 Degraded LKG 世代（不是最新的 Superseded）：{}",
            body["members"][0]["index_alert"]
        );

        // 另起一套：最新 Building（快照无跳过事实）+ 较旧 Degraded（含跳过事实）。
        let root_two = tempdir().unwrap();
        let (paths_two, lc_two) = seed_lc(root_two.path());
        let member_two = LogicalRepositoryId(Uuid::new_v4());
        let checkout_two = RepositoryCheckoutId(Uuid::new_v4());
        seed_members(
            &paths_two,
            root_two.path(),
            &lc_two,
            &[member_with_checkout(
                member_two,
                "gray-config",
                checkout_two,
            )],
            &[main_checkout(
                member_two,
                checkout_two,
                CheckoutAvailability::Available,
            )],
        );
        let store_two = AggregateIndexStore::for_lc(paths_two.clone(), lc_two.clone());
        store_two
            .create(
                "project_0001",
                generation(
                    "aggregate_index_degraded_old",
                    AggregateIndexStatus::Degraded,
                    "2026-11-18T01:00:00Z",
                    vec![snapshot(member_two, checkout_two, true)],
                ),
            )
            .unwrap();
        store_two
            .create(
                "project_0001",
                generation(
                    "aggregate_index_building",
                    AggregateIndexStatus::Building,
                    "2026-11-18T02:00:00Z",
                    vec![snapshot(member_two, checkout_two, false)],
                ),
            )
            .unwrap();

        let body_two = members_body(root_two.path(), &lc_two).await;
        assert!(
            body_two["members"][0]["index_alert"].is_null(),
            "被投影的是 Building 世代（无跳过事实）⇒ 不标注：{}",
            body_two["members"][0]["index_alert"]
        );
    }
}
