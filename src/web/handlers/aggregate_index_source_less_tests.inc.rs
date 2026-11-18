// 片5 测试：聚合索引卡片投影的结构化跳过告警 + 世代取法。
//
// 本文件被 `aggregate_index.rs` 尾部的 `#[cfg(test)] mod source_less_tests`
// `include!` 引入（large_file_guard 1200 行上限外挪），与该文件同一模块
// 作用域；文件内不再包 `mod tests`。过滤器：`aggregate_index::source_less_tests`。

use super::*;
use crate::product::app_paths::ProductAppPaths;
use crate::product::logical_codebase::aggregate_index::{
    AggregateIndexMemberSnapshot, AggregateIndexStore,
};
use crate::product::logical_codebase::types::{
    CheckoutAvailability, CheckoutKind, CodebaseMemberRecord, MemberStatus,
    RepositoryCheckoutRecord, RepositorySourceIdentity, RepositoryType,
};
use crate::product::logical_codebase::{
    LogicalCodebaseStore, LogicalRepositoryId, RepositoryCheckoutId,
};
use uuid::Uuid;

const LC: &str = "logical_codebase_0001";

fn ids() -> (LogicalRepositoryId, RepositoryCheckoutId) {
    (
        LogicalRepositoryId(Uuid::new_v4()),
        RepositoryCheckoutId(Uuid::new_v4()),
    )
}

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

fn record(
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

fn source_identity() -> RepositorySourceIdentity {
    RepositorySourceIdentity {
        scheme: "git_dir_only_v1".to_string(),
        key_digest: "fixture-digest".to_string(),
        canonical_git_dir: std::path::PathBuf::from("/tmp/fixture-git-dir"),
        canonical_origin: None,
        first_seen_path_hash: "fixture-path-hash".to_string(),
    }
}

fn member_record(
    member: LogicalRepositoryId,
    alias: &str,
    checkout_ids: Vec<RepositoryCheckoutId>,
) -> CodebaseMemberRecord {
    CodebaseMemberRecord {
        logical_repository_id: member,
        physical_repository_id: format!("repository_{alias}"),
        alias: alias.to_string(),
        role: "member".to_string(),
        ordinal: 1,
        source_identity: source_identity(),
        repo_type: RepositoryType::Unknown,
        tech_stack: Vec::new(),
        owner: None,
        tags: Vec::new(),
        default_ref: None,
        checkout_ids,
        status: MemberStatus::Active,
        created_at: "2026-11-18T00:00:00Z".to_string(),
        updated_at: "2026-11-18T00:00:00Z".to_string(),
    }
}

fn main_checkout(
    member: LogicalRepositoryId,
    checkout: RepositoryCheckoutId,
    availability: CheckoutAvailability,
) -> RepositoryCheckoutRecord {
    RepositoryCheckoutRecord {
        checkout_id: checkout,
        logical_repository_id: member,
        physical_repository_id: "repository_fixture".to_string(),
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

fn register_member(
    paths: &ProductAppPaths,
    member: &CodebaseMemberRecord,
    checkouts: &[RepositoryCheckoutRecord],
) {
    let store = LogicalCodebaseStore::for_lc(paths.clone(), LC);
    store.save_member("project_0001", member).unwrap();
    for checkout in checkouts {
        store.save_checkout("project_0001", checkout).unwrap();
    }
}

/// 在临时目录里递归找夹具落盘文件（深度有限，纯测试辅助）。
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

/// #1 有跳过成员 ⇒ 结构化告警点名成员；响应本身不承载成员名单（R8）。
#[test]
fn read_active_projection_reports_structured_alert_for_skipped_members() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let (member, checkout) = ids();
    register_member(
        &paths,
        &member_record(member, "gray-config", vec![checkout]),
        &[main_checkout(
            member,
            checkout,
            CheckoutAvailability::Available,
        )],
    );
    AggregateIndexStore::for_lc(paths.clone(), LC)
        .create(
            "project_0001",
            record(
                "aggregate_index_gen",
                AggregateIndexStatus::Active,
                "2026-11-18T01:00:00Z",
                vec![snapshot(member, checkout, true)],
            ),
        )
        .unwrap();

    let response = read_active_projection(&paths, "project_0001", LC).unwrap();
    let alert = response
        .source_less_alert
        .as_ref()
        .expect("有跳过事实 ⇒ 必须出结构化告警");
    assert_eq!(alert.code, "aggregate_index_member_source_less");
    assert!(
        alert.message_zh.contains("已跳过无源码成员"),
        "告警文案：{}",
        alert.message_zh
    );
    assert!(
        alert.message_zh.contains("gray-config"),
        "告警必须点名被跳过成员：{}",
        alert.message_zh
    );
    let payload = serde_json::to_value(&response).unwrap();
    assert!(
        payload.get("skipped_members").is_none(),
        "R8：响应不得再出现 skipped_members 字段：{payload}"
    );
    // 告警对象本身也只有 code + message_zh，不携带成员名单。
    let alert_value = payload.get("source_less_alert").expect("告警已断言存在");
    assert!(alert_value.get("members").is_none());
    assert!(alert_value.get("skipped_members").is_none());
}

/// #2 无跳过事实（成员正常收录）⇒ 不造告警。
#[test]
fn read_active_projection_has_no_alert_without_skip_facts() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let (member, checkout) = ids();
    register_member(
        &paths,
        &member_record(member, "gray-config", vec![checkout]),
        &[main_checkout(
            member,
            checkout,
            CheckoutAvailability::Available,
        )],
    );
    AggregateIndexStore::for_lc(paths.clone(), LC)
        .create(
            "project_0001",
            record(
                "aggregate_index_gen",
                AggregateIndexStatus::Active,
                "2026-11-18T01:00:00Z",
                vec![snapshot(member, checkout, false)],
            ),
        )
        .unwrap();

    let response = read_active_projection(&paths, "project_0001", LC).unwrap();
    assert_eq!(response.state, "active");
    assert!(
        response.source_less_alert.is_none(),
        "无跳过事实 ⇒ 不造告警"
    );
}

/// #3 重建中不出提示（状态矩阵）：即使被投影的 Building 世代与旧世代
/// 都带跳过事实，`source_less_alert` 也必须缺省。
#[test]
fn read_active_projection_omits_alert_while_rebuilding() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let (member, checkout) = ids();
    register_member(
        &paths,
        &member_record(member, "gray-config", vec![checkout]),
        &[main_checkout(
            member,
            checkout,
            CheckoutAvailability::Available,
        )],
    );
    let store = AggregateIndexStore::for_lc(paths.clone(), LC);
    store
        .create(
            "project_0001",
            record(
                "aggregate_index_old_gen",
                AggregateIndexStatus::Active,
                "2026-11-18T01:00:00Z",
                vec![snapshot(member, checkout, true)],
            ),
        )
        .unwrap();
    store
        .create(
            "project_0001",
            record(
                "aggregate_index_building_gen",
                AggregateIndexStatus::Building,
                "2026-11-18T02:00:00Z",
                vec![snapshot(member, checkout, true)],
            ),
        )
        .unwrap();

    let response = read_active_projection(&paths, "project_0001", LC).unwrap();
    assert_eq!(response.state, "rebuilding");
    assert!(
        response.source_less_alert.is_none(),
        "重建中不出跳过提示（用户正在行动）：{:?}",
        response.source_less_alert
    );
}

/// #4 degraded LKG 带历史跳过 ⇒ 告警走「上次可用索引」措辞，且与
/// 失败三段（`failure_copy`）并存、互不覆盖（REQ-IND-04）。
#[test]
fn read_active_projection_labels_degraded_skip_as_last_known_good() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let (member, checkout) = ids();
    register_member(
        &paths,
        &member_record(member, "gray-config", vec![checkout]),
        &[main_checkout(
            member,
            checkout,
            CheckoutAvailability::Available,
        )],
    );
    let mut generation = record(
        "aggregate_index_lkg",
        AggregateIndexStatus::Degraded,
        "2026-11-18T01:00:00Z",
        vec![snapshot(member, checkout, true)],
    );
    generation.warning =
        Some("aggregate_index_failed:codegraph_init_failed: cli exploded".to_string());
    AggregateIndexStore::for_lc(paths.clone(), LC)
        .create("project_0001", generation)
        .unwrap();

    let response = read_active_projection(&paths, "project_0001", LC).unwrap();
    assert_eq!(response.state, "degraded");
    assert!(
        response.failure_copy.is_some(),
        "失败三段必须与跳过告警并存"
    );
    let alert = response
        .source_less_alert
        .expect("degraded LKG 的跳过事实也要出告警");
    assert_eq!(alert.code, "aggregate_index_member_source_less");
    assert!(
        alert.message_zh.contains("上次可用索引"),
        "degraded 必须用历史措辞：{}",
        alert.message_zh
    );
    assert!(
        alert.message_zh.contains("gray-config"),
        "历史措辞同样点名成员：{}",
        alert.message_zh
    );
}

/// #5 老世代 JSON（盘上无 `skip_reason` 键）⇒ 可读且不投影出告警。
#[test]
fn read_active_projection_keeps_legacy_generation_free_of_skip_alert() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let (member, checkout) = ids();
    register_member(
        &paths,
        &member_record(member, "gray-config", vec![checkout]),
        &[main_checkout(
            member,
            checkout,
            CheckoutAvailability::Available,
        )],
    );
    AggregateIndexStore::for_lc(paths.clone(), LC)
        .create(
            "project_0001",
            record(
                "aggregate_index_legacy_gen",
                AggregateIndexStatus::Active,
                "2026-11-18T01:00:00Z",
                vec![snapshot(member, checkout, false)],
            ),
        )
        .unwrap();
    // 夹具自证：skip_reason 为 None 时盘上 JSON 就是老形态（无该键）。
    let disk =
        std::fs::read_to_string(find_json(temp.path(), "aggregate_index_legacy_gen.json")).unwrap();
    assert!(
        !disk.contains("skip_reason"),
        "夹具必须是老形态 JSON：{disk}"
    );

    let response = read_active_projection(&paths, "project_0001", LC).unwrap();
    assert_eq!(response.state, "active");
    assert!(
        response.source_less_alert.is_none(),
        "老世代无跳过事实 ⇒ 不造告警"
    );
}

/// #6 告警只取 `projected_record` 选出的那一份：比 degraded 更新的
/// Active 世代带跳过事实，但被投影的是 degraded LKG（无跳过事实）
/// ⇒ 告警与 state 同源，都来自被投影的那一代。
#[test]
fn read_active_projection_alert_comes_from_the_projected_generation() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let (member, checkout) = ids();
    register_member(
        &paths,
        &member_record(member, "gray-config", vec![checkout]),
        &[main_checkout(
            member,
            checkout,
            CheckoutAvailability::Available,
        )],
    );
    let store = AggregateIndexStore::for_lc(paths.clone(), LC);
    store
        .create(
            "project_0001",
            record(
                "aggregate_index_lkg",
                AggregateIndexStatus::Degraded,
                "2026-11-18T01:00:00Z",
                Vec::new(),
            ),
        )
        .unwrap();
    store
        .create(
            "project_0001",
            record(
                "aggregate_index_new_gen",
                AggregateIndexStatus::Active,
                "2026-11-18T02:00:00Z",
                vec![snapshot(member, checkout, true)],
            ),
        )
        .unwrap();

    let response = read_active_projection(&paths, "project_0001", LC).unwrap();
    assert_eq!(
        response.state, "degraded",
        "degraded LKG 优先于更新的 Active（既有选择规则）"
    );
    assert!(
        response.source_less_alert.is_none(),
        "告警只能来自被投影的 degraded 世代，不得扫描其它世代拼事实：{:?}",
        response.source_less_alert
    );
}

/// #7 快照的 checkout 与成员**当前**可用的 Main checkout 不一致
/// ⇒ 不贴标（不得按裸成员 ID 匹配）。
#[test]
fn read_active_projection_drops_alert_when_main_checkout_changed() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let (member, retired_checkout) = ids();
    let (_, current_checkout) = ids();
    register_member(
        &paths,
        &member_record(
            member,
            "gray-config",
            vec![retired_checkout, current_checkout],
        ),
        &[
            // 旧主检出：目录已不在（availability Missing），成员换到了新主检出。
            main_checkout(member, retired_checkout, CheckoutAvailability::Missing),
            main_checkout(member, current_checkout, CheckoutAvailability::Available),
        ],
    );
    AggregateIndexStore::for_lc(paths.clone(), LC)
        .create(
            "project_0001",
            record(
                "aggregate_index_gen",
                AggregateIndexStatus::Active,
                "2026-11-18T01:00:00Z",
                // 快照记录的是旧主检出的跳过事实。
                vec![snapshot(member, retired_checkout, true)],
            ),
        )
        .unwrap();

    let response = read_active_projection(&paths, "project_0001", LC).unwrap();
    assert_eq!(response.state, "active");
    assert!(
        response.source_less_alert.is_none(),
        "快照 checkout ≠ 当前可用 Main checkout ⇒ 不按裸成员 ID 贴标：{:?}",
        response.source_less_alert
    );
}

/// #8 只读投影：没有成员目录、没有 CodeGraph 库、没有任何 CLI runner，
/// 仅凭 durable 世代记录也能完成读取（不扫盘、不启进程、不 panic）。
#[test]
fn read_active_projection_does_not_scan_disk_or_start_codegraph() {
    let temp = tempfile::tempdir().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let (member, checkout) = ids();
    AggregateIndexStore::for_lc(paths.clone(), LC)
        .create(
            "project_0001",
            record(
                "aggregate_index_gen",
                AggregateIndexStatus::Active,
                "2026-11-18T01:00:00Z",
                vec![snapshot(member, checkout, true)],
            ),
        )
        .unwrap();

    let response = read_active_projection(&paths, "project_0001", LC)
        .expect("只读投影必须成功：不扫盘、不启 CodeGraph");
    assert_eq!(response.state, "active");
}

/// #9 `projected_record` 直测：Superseded 永不被选中；Degraded 优先于
/// 更新的 Active/Stale；Building 最新时优先（即使有旧 Degraded）；
/// Failed 取 LKG，无 LKG ⇒ None；空 ⇒ None。
#[test]
fn projected_record_excludes_superseded_and_prefers_degraded_over_newer_active() {
    fn generation(
        id: &str,
        status: AggregateIndexStatus,
        updated_at: &str,
    ) -> AggregateIndexRecord {
        record(id, status, updated_at, Vec::new())
    }
    fn picked(records: &[AggregateIndexRecord]) -> Option<&str> {
        projected_record(records).map(|record| record.aggregate_index_id.as_str())
    }

    // Superseded 排最前也永不被选中。
    assert_eq!(
        picked(&[
            generation(
                "gen_superseded",
                AggregateIndexStatus::Superseded,
                "2026-11-18T03:00:00Z"
            ),
            generation(
                "gen_active",
                AggregateIndexStatus::Active,
                "2026-11-18T01:00:00Z"
            ),
        ]),
        Some("gen_active")
    );
    // Degraded 优先于更新的 Active / Stale。
    assert_eq!(
        picked(&[
            generation(
                "gen_active",
                AggregateIndexStatus::Active,
                "2026-11-18T02:00:00Z"
            ),
            generation(
                "gen_degraded",
                AggregateIndexStatus::Degraded,
                "2026-11-18T01:00:00Z"
            ),
        ]),
        Some("gen_degraded")
    );
    assert_eq!(
        picked(&[
            generation(
                "gen_stale",
                AggregateIndexStatus::Stale,
                "2026-11-18T02:00:00Z"
            ),
            generation(
                "gen_degraded",
                AggregateIndexStatus::Degraded,
                "2026-11-18T01:00:00Z"
            ),
        ]),
        Some("gen_degraded")
    );
    // Building 最新时优先（用户正在行动），即使存在旧 Degraded。
    assert_eq!(
        picked(&[
            generation(
                "gen_building",
                AggregateIndexStatus::Building,
                "2026-11-18T02:00:00Z"
            ),
            generation(
                "gen_degraded",
                AggregateIndexStatus::Degraded,
                "2026-11-18T01:00:00Z"
            ),
        ]),
        Some("gen_building")
    );
    // Failed 取 last-known-good；无 LKG ⇒ None。
    assert_eq!(
        picked(&[
            generation(
                "gen_failed",
                AggregateIndexStatus::Failed,
                "2026-11-18T02:00:00Z"
            ),
            generation(
                "gen_lkg",
                AggregateIndexStatus::Active,
                "2026-11-18T01:00:00Z"
            ),
        ]),
        Some("gen_lkg")
    );
    assert_eq!(
        picked(&[generation(
            "gen_failed",
            AggregateIndexStatus::Failed,
            "2026-11-18T02:00:00Z"
        )]),
        None
    );
    assert_eq!(picked(&[]), None);
}
