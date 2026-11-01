use super::*;
use crate::product::entity_deletion_store::DeletionEntityKind;
use std::path::Path;
use tempfile::TempDir;

/// 写一条文件（建父目录）。
fn write(path: &Path, body: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, body.as_bytes()).unwrap();
}

/// 墓碑（`phase` 可选 `deleted` / `purging`）。
fn write_tombstone(root: &Path, kind: &str, id: &str, phase: &str) {
    let path = root
        .join("projects/project_0001/issues/issue_0001/entity-deletions")
        .join(kind)
        .join(format!("{id}.json"));
    let body = format!(
        r#"{{"project_id":"project_0001","issue_id":"issue_0001","entity_kind":"{kind}","entity_id":"{id}","phase":"{phase}","started_at":"2026-10-28T10:00:00Z","deleted_at":"2026-10-28T10:05:00Z"}}"#
    );
    write(&path, &body);
}

/// 台账绑定指向**已删 plan**（L1 墓碑四条全成立）→ 跳过该绑定 + 一条告警。
#[test]
fn live_ledger_session_ids_skips_bindings_pointing_to_deleted_plan() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007", "deleted");
    let scope = EntityScope::new(
        "project_0001",
        "issue_0001",
        DeletionEntityKind::Plan,
        "issue_work_item_plan_0007",
    );
    let ids: BTreeSet<String> = ["s_gone".to_string()].into_iter().collect();

    let (live, warnings) = live_ledger_session_ids(&paths, &scope, ids).expect("台账容错");

    assert!(live.is_empty(), "已删实体的 session 不得算进待清理集合");
    assert_eq!(warnings.len(), 1, "每条被跳过的绑定一条告警");
    let warning = &warnings.items()[0];
    assert_eq!(warning.code, DANGLING_REFERENCE_WARNING_CODE);
    assert_eq!(warning.record_type, "entity_identity_session_binding");
    assert_eq!(warning.target_kind, "plan");
    assert_eq!(warning.target_id, "issue_work_item_plan_0007");
    assert_eq!(warning.action, "skipped");
    assert!(
        warning
            .record_path
            .ends_with("state/entity-identity/sessions/s_gone.json"),
        "{}",
        warning.record_path
    );
    assert_eq!(warning.code, DANGLING_REFERENCE_WARNING_CODE);
    assert_eq!(warning.record_type, "entity_identity_session_binding");
    assert_eq!(warning.target_kind, "plan");
    assert_eq!(warning.target_id, "issue_work_item_plan_0007");
    assert_eq!(warning.action, "skipped");
    assert!(
        warning
            .record_path
            .ends_with("state/entity-identity/sessions/s_gone.json"),
        "{}",
        warning.record_path
    );
}

/// 删除**进行中**（`purging`）→ 证据不足，不算「已删」→ 绑定必须保留。
#[test]
fn live_ledger_session_ids_keeps_bindings_when_plan_deletion_is_in_progress() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    write_tombstone(temp.path(), "plan", "issue_work_item_plan_0007", "purging");
    let scope = EntityScope::new(
        "project_0001",
        "issue_0001",
        DeletionEntityKind::Plan,
        "issue_work_item_plan_0007",
    );
    let ids: BTreeSet<String> = ["s_purging".to_string()].into_iter().collect();

    let (live, warnings) = live_ledger_session_ids(&paths, &scope, ids).expect("台账容错");

    assert_eq!(live.len(), 1, "删除进行中的绑定不得被跳过");
    assert_eq!(warnings.len(), 0, "证据不足不得产生悬空告警");
}

/// 没有墓碑（实体仍在或从未删过）→ 绑定必须保留。
#[test]
fn live_ledger_session_ids_keeps_bindings_without_tombstone() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let scope = EntityScope::new(
        "project_0001",
        "issue_0001",
        DeletionEntityKind::Plan,
        "issue_work_item_plan_0007",
    );
    let ids: BTreeSet<String> = ["s_alive".to_string()].into_iter().collect();

    let (live, warnings) = live_ledger_session_ids(&paths, &scope, ids).expect("台账容错");

    assert_eq!(live.len(), 1, "无墓碑的绑定不得被跳过");
    assert_eq!(warnings.len(), 0, "无墓碑不得产生悬空告警");
}

/// 读失败照旧上抛（REQ-DRG-24 / REQ-DRG-05）：墓碑文件存在但**内容不是合法
/// JSON** 时，`live_ledger_session_ids` MUST 返回 `Err`，MUST NOT 把它当成
/// 「目标已删」而跳过绑定。
///
/// 「一个都没跳」的可观测形态：跳过路径返回 `Ok((空集合, 逐条告警))`——因此
/// 拿到 `Err` 即证明没有任何绑定被跳过，且错误必须是**解析失败原文**（不是
/// 「不存在」）。
#[test]
fn live_ledger_session_ids_propagates_tombstone_parse_failure_without_skipping() {
    let temp = TempDir::new().unwrap();
    let paths = ProductAppPaths::new(temp.path());
    let tombstone = temp
        .path()
        .join("projects/project_0001/issues/issue_0001/entity-deletions/plan")
        .join("issue_work_item_plan_0007.json");
    write(&tombstone, "{ this is not json");
    let scope = EntityScope::new(
        "project_0001",
        "issue_0001",
        DeletionEntityKind::Plan,
        "issue_work_item_plan_0007",
    );
    let ids: BTreeSet<String> = ["s_a".to_string(), "s_b".to_string()].into_iter().collect();

    let error = live_ledger_session_ids(&paths, &scope, ids)
        .expect_err("墓碑解析失败 MUST 原样上抛，MUST NOT 跳过绑定");

    assert!(
        matches!(error, ProductStoreError::Json(_)),
        "解析失败必须以 JSON 错误上抛：{error:?}"
    );
    assert!(
        !matches!(error, ProductStoreError::NotFound { .. }),
        "读失败 MUST NOT 被折叠成「目标不存在」：{error:?}"
    );
    assert!(
        error.to_string().starts_with("product_store_json:"),
        "错误文案必须保留既有读取失败口径：{error}"
    );
}
