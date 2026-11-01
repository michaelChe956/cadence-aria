/// 2026-10-29：逻辑代码库列表条目。`GET .../logical-codebases?include_deleted=true`
/// 会同时带出墓碑 LC（`deleted=true` + `deleted_at`），供 UI 的「已删除」分区渲染。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseListEntryDto {
    pub id: String,
    pub name: String,
    pub aggregate_root: String,
    pub created_at: String,
    pub deleted: bool,
    /// 仅 `deleted=true` 时存在（软删时刻）。
    pub deleted_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseListResponse {
    pub logical_codebases: Vec<LogicalCodebaseListEntryDto>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct CreateLogicalCodebaseRequest {
    pub name: String,
    pub aggregate_root: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseDto {
    pub id: String,
    pub name: String,
    pub aggregate_root: String,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseMemberDto {
    pub logical_repository_id: String,
    pub physical_repository_id: String,
    pub alias: String,
    pub status: String,
    pub ordinal: u32,
    pub created_at: String,
    pub updated_at: String,
}

/// LC 详情：record + 成员列表 + 初始化/索引状态汇总（manifest 待首批登记创建 →
/// manifest_present=false 即未初始化；active_aggregate_index_id 汇总索引状态）。
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct LogicalCodebaseDetailDto {
    pub id: String,
    pub name: String,
    pub aggregate_root: String,
    pub created_at: String,
    pub manifest_present: bool,
    pub membership_revision: Option<u64>,
    pub active_aggregate_index_id: Option<String>,
    pub member_count: usize,
    pub members: Vec<LogicalCodebaseMemberDto>,
}
