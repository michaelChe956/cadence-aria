//! 聚合索引世代 JSON 的受限投影（T3 诊断摘要，不是原始 JSON）。
//!
//! - 只认两种规范布局的**直接**世代文件（见 [`classify`]）：
//!   `<project>/logical-codebase/aggregate-indexes/<id>.json`（旧项目级别名）
//!   与 `<project>/logical-codebases/<lc>/aggregate-indexes/<id>.json`（v1.3）。
//!   `aggregate-indexes` **不进**通用递归白名单：嵌套一层的同名目录不再收录。
//! - 投影前先核对作用域身份：文件基名必须就是世代 ID，正文 `project_id`
//!   必须等于目录里的项目名；对不上整份丢弃，不改写、不猜。
//! - 只导出：世代身份 / 作用域 / 状态 / 时间 / 成员版本 / 可识别的失败码与
//!   失败分诊 / 实测覆盖数量与耗时 / 逐成员盘检证据。任意 warning 正文、
//!   stderr、SDK 输出、绝对路径、成员快照与 Git 快照正文一律不导出。
//! - 缺字段的旧世代按「缺失」如实留空并记稳定原因，不补造原因；
//!   被写进 degraded LKG 的另一代失败不记成该 LKG 自身的重建失败。

use std::path::{Component, Path};

use serde::Deserialize;
use serde_json::{Map, Value, json};

use crate::product::logical_codebase::aggregate_index::AggregateIndexStatus;
use crate::product::logical_codebase::aggregate_index::scan_diagnostics::{
    SourceScanEvidence, scan_failure_fact,
};
use crate::product::logical_codebase::failure_copy;

/// 世代文件的作用域布局。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum GenerationLayout {
    /// 旧的项目级别名布局（`logical-codebase/`）。
    ProjectScope,
    /// v1.3 的每 LC 布局（`logical-codebases/<lc>/`）。
    LogicalCodebaseScope,
}

/// 一个世代文件的作用域事实（由路径推出，正文还要再核对一遍）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GenerationScope {
    pub(super) layout: GenerationLayout,
    pub(super) project_id: String,
    pub(super) logical_codebase_id: Option<String>,
    /// 文件基名（去掉 `.json`），必须等于正文里的世代 ID。
    pub(super) index_id: String,
}

/// 投影结果：白名单事实 + 局部缺失/不可识别的原因（供清单如实说明）。
#[derive(Debug, Clone)]
pub(super) struct Projection {
    pub(super) value: Value,
    pub(super) notes: Vec<String>,
}

/// ID 字符集：ASCII 字母数字与 `.` `-` `_`（不含路径分隔符与空白）。
fn is_safe_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

/// 稳定失败码字符集：ASCII 小写字母数字与 `_`。
fn is_stable_code(code: &str) -> bool {
    !code.is_empty()
        && code.len() <= 64
        && code
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
}

/// 成员短名：短、无控制字符、无路径分隔符（绝对路径与相对路径都不得出现）。
fn is_member_label(label: &str) -> bool {
    !label.is_empty()
        && label.len() <= 64
        && label != "."
        && label != ".."
        && !label.contains('/')
        && !label.contains('\\')
        && !label.contains(':')
        && !label.chars().any(char::is_control)
}

/// 判定相对路径是否为「直接的世代 JSON」。
///
/// 只有两种规范布局的直接子文件才算；再深一层（嵌套目录）不算，
/// 通用递归白名单也不收 `aggregate-indexes`。
pub(super) fn classify(rel: &Path) -> Option<GenerationScope> {
    if rel.parent()?.file_name()? != "aggregate-indexes" || rel.extension()? != "json" {
        return None;
    }
    let mut parts: Vec<&str> = Vec::with_capacity(5);
    for component in rel.components() {
        match component {
            Component::Normal(name) => parts.push(name.to_str()?),
            _ => return None,
        }
    }
    let (layout, project_id, logical_codebase_id, file) = match parts.as_slice() {
        [project, "logical-codebase", "aggregate-indexes", file] => {
            (GenerationLayout::ProjectScope, *project, None, *file)
        }
        [project, "logical-codebases", lc, "aggregate-indexes", file] => (
            GenerationLayout::LogicalCodebaseScope,
            *project,
            Some(*lc),
            *file,
        ),
        _ => return None,
    };
    let index_id = file.strip_suffix(".json")?;
    if !is_safe_id(project_id) || !is_safe_id(index_id) {
        return None;
    }
    if let Some(lc) = logical_codebase_id
        && !is_safe_id(lc)
    {
        return None;
    }
    Some(GenerationScope {
        layout,
        project_id: project_id.to_string(),
        logical_codebase_id: logical_codebase_id.map(str::to_string),
        index_id: index_id.to_string(),
    })
}

/// 世代 JSON → 白名单诊断摘要。
///
/// 作用域身份对不上 ⇒ 整份丢弃（`Err`）；其余缺失按「缺失」如实留空，
/// 并把原因写进 `notes`，由清单说明「收录了什么、缺了什么」。
pub(super) fn project(scope: &GenerationScope, body: &Value) -> Result<Projection, &'static str> {
    let Some(object) = body.as_object() else {
        return Err("not_an_object");
    };

    // 作用域身份：基名 ↔ 世代 ID，目录项目名 ↔ 正文 project_id。
    let Some(index_id) = object.get("aggregate_index_id").and_then(Value::as_str) else {
        return Err("missing_identity");
    };
    if !is_safe_id(index_id) {
        return Err("invalid_identity");
    }
    if index_id != scope.index_id {
        return Err("identity_mismatch");
    }
    let Some(project_id) = object.get("project_id").and_then(Value::as_str) else {
        return Err("missing_identity");
    };
    if !is_safe_id(project_id) || project_id != scope.project_id {
        return Err("identity_mismatch");
    }

    let mut notes: Vec<String> = Vec::new();
    let mut out = Map::new();
    out.insert("kind".into(), json!("aggregate_index_generation"));
    out.insert(
        "layout".into(),
        json!(match scope.layout {
            GenerationLayout::ProjectScope => "project_scope",
            GenerationLayout::LogicalCodebaseScope => "logical_codebase_scope",
        }),
    );
    out.insert("aggregate_index_id".into(), json!(index_id));
    out.insert("project_id".into(), json!(project_id));
    if let Some(lc) = scope.logical_codebase_id.as_deref() {
        out.insert("logical_codebase_id".into(), json!(lc));
    }

    // 状态：只认 schema 里的枚举；未知状态不原样透传。
    let status = match object.get("status") {
        Some(raw) => match AggregateIndexStatus::deserialize(raw) {
            Ok(status) => Some(status),
            Err(_) => {
                notes.push("status_unrecognized".to_string());
                None
            }
        },
        None => {
            notes.push("status_missing".to_string());
            None
        }
    };
    if let Some(status) = status
        && let Ok(value) = serde_json::to_value(status)
    {
        out.insert("status".into(), value);
    }

    // 成员版本（membership revision）：旧世代缺这个键就如实留空。
    match object.get("membership_revision") {
        Some(Value::Number(number)) => match number.as_u64() {
            Some(revision) => {
                out.insert("membership_revision".into(), json!(revision));
            }
            None => notes.push("membership_revision_invalid".to_string()),
        },
        Some(_) => notes.push("membership_revision_invalid".to_string()),
        None => notes.push("membership_revision_missing".to_string()),
    }

    // 时间：只导可解析的 RFC3339，原样字符串不直接透传。
    for key in ["created_at", "updated_at"] {
        let Some(text) = object.get(key).and_then(Value::as_str) else {
            continue;
        };
        match chrono::DateTime::parse_from_rfc3339(text.trim()) {
            Ok(parsed) => {
                out.insert(key.into(), json!(parsed.to_rfc3339()));
            }
            Err(_) => notes.push(format!("{key}_invalid")),
        }
    }

    // 失败码：只认共享的失败封套解析（不另写一套解析，也不导出正文）。
    if let Some(warning) = object
        .get("warning")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|warning| !warning.is_empty())
    {
        match failure_copy::parse_failure_envelope(warning) {
            Some((code, fact, _degraded))
                if is_stable_code(&code)
                    && let Some(triage) =
                        failure_copy::registered_diagnostic_triage(&code, &fact) =>
            {
                // degraded LKG 上的 warning 是「另一次重建失败」留下的，
                // 不能记成这个 LKG 自身的重建失败。
                let owner = match status {
                    Some(AggregateIndexStatus::Degraded) => "degrading_attempt",
                    Some(AggregateIndexStatus::Failed) => "own_rebuild",
                    _ => "record",
                };
                let mut failure = json!({"code":code, "class":triage.as_str(), "scope":owner});
                if let Some(kind) = scan_failure_fact(&code, &fact) {
                    failure["fact_kind"] = json!(kind);
                }
                out.insert("failure".into(), failure);
            }
            Some(_) => notes.push("failure_code_unrecognized".to_string()),
            None => notes.push("warning_unrecognized".to_string()),
        }
    }

    // 盘检证据：按类型化 schema 读入；读不出就整段不导出（不局部伪造）。
    if let Some(raw) = object.get("source_scan") {
        match SourceScanEvidence::deserialize(raw) {
            Ok(evidence) => {
                let mut members = Vec::with_capacity(evidence.members.len());
                let mut rejected = 0usize;
                for member in &evidence.members {
                    if !is_member_label(&member.member) {
                        rejected += 1;
                        continue;
                    }
                    let mut entry = Map::new();
                    entry.insert("member".into(), json!(member.member));
                    if let Ok(verdict) = serde_json::to_value(member.verdict) {
                        entry.insert("verdict".into(), verdict);
                    }
                    if let Some(reason) = member.reason
                        && let Ok(value) = serde_json::to_value(reason)
                    {
                        entry.insert("reason".into(), value);
                    }
                    entry.insert(
                        "visibility_started".into(),
                        json!(member.visibility_started),
                    );
                    if let Some(remaining) = member.remaining_ms_at_entry {
                        entry.insert("remaining_ms_at_entry".into(), json!(remaining));
                    }
                    members.push(Value::Object(entry));
                }
                let mut scan = Map::new();
                scan.insert("member_count".into(), json!(evidence.member_count));
                scan.insert("covered_count".into(), json!(evidence.covered_count));
                scan.insert(
                    "coverage_elapsed_ms".into(),
                    json!(evidence.coverage_elapsed_ms),
                );
                scan.insert("scan_elapsed_ms".into(), json!(evidence.scan_elapsed_ms));
                scan.insert("members".into(), Value::Array(members));
                if rejected > 0 {
                    scan.insert("rejected_members".into(), json!(rejected));
                }
                out.insert("source_scan".into(), Value::Object(scan));
            }
            Err(_) => notes.push("source_scan_invalid".to_string()),
        }
    }

    Ok(Projection {
        value: Value::Object(out),
        notes,
    })
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use serde_json::json;

    use super::*;

    fn scope(project: &str, index_id: &str, layout: GenerationLayout) -> GenerationScope {
        GenerationScope {
            layout,
            project_id: project.to_string(),
            logical_codebase_id: match layout {
                GenerationLayout::ProjectScope => None,
                GenerationLayout::LogicalCodebaseScope => Some("lc_0001".to_string()),
            },
            index_id: index_id.to_string(),
        }
    }

    #[test]
    fn classify_accepts_only_the_two_canonical_direct_layouts() {
        assert!(
            classify(Path::new(
                "project_0001/logical-codebase/aggregate-indexes/aggregate_index_1.json"
            ))
            .is_some()
        );
        assert!(
            classify(Path::new(
                "project_0001/logical-codebases/lc_0001/aggregate-indexes/agg_1.json"
            ))
            .is_some()
        );
        // 嵌套一层、别的目录名、非 JSON：都不算世代文件。
        assert!(
            classify(Path::new(
                "project_0001/logical-codebase/aggregate-indexes/nested/agg_1.json"
            ))
            .is_none()
        );
        assert!(classify(Path::new("project_0001/aggregate-indexes/agg_1.json")).is_none());
        assert!(
            classify(Path::new(
                "project_0001/logical-codebase/aggregate-indexes/agg_1.txt"
            ))
            .is_none()
        );
        assert!(
            classify(Path::new(
                "project_0001/logical-codebase/aggregate-indexes/.json"
            ))
            .is_none()
        );
    }

    #[test]
    fn mismatched_identity_is_rejected_not_rewritten() {
        let scope = scope("project_0001", "agg_1", GenerationLayout::ProjectScope);
        let body = json!({"aggregate_index_id": "agg_2", "project_id": "project_0001"});
        assert_eq!(project(&scope, &body).err(), Some("identity_mismatch"));
        let body = json!({"aggregate_index_id": "agg_1", "project_id": "project_0002"});
        assert_eq!(project(&scope, &body).err(), Some("identity_mismatch"));
        let body = json!({"aggregate_index_id": "agg_1"});
        assert_eq!(project(&scope, &body).err(), Some("missing_identity"));
    }

    #[test]
    fn projection_keeps_only_whitelisted_facts() {
        let scope = scope("project_0001", "agg_1", GenerationLayout::ProjectScope);
        let body = json!({
            "aggregate_index_id": "agg_1",
            "project_id": "project_0001",
            "membership_revision": 3,
            "status": "failed",
            "created_at": "2026-11-19T10:00:00Z",
            "updated_at": "2026-11-19T10:00:05Z",
            "warning": "aggregate_index_failed:aggregate_index_source_scan_incomplete: members [a, b]",
            "codegraph_root": "/abs/path/root",
            "member_snapshots": [{"logical_repository_id": "r1", "checkout_id": "c1"}],
            "config_digest": "digest",
        });
        let projection = project(&scope, &body).expect("projected");
        let value = projection.value;
        assert_eq!(value["kind"], "aggregate_index_generation");
        assert_eq!(value["status"], "failed");
        assert_eq!(value["membership_revision"], 3);
        assert_eq!(
            value["failure"]["code"],
            "aggregate_index_source_scan_incomplete"
        );
        assert_eq!(value["failure"]["scope"], "own_rebuild");
        assert!(value.get("codegraph_root").is_none());
        assert!(value.get("member_snapshots").is_none());
        assert!(value.get("config_digest").is_none());
        let text = serde_json::to_string(&value).expect("json");
        assert!(!text.contains("/abs/path"));
    }

    #[test]
    fn degraded_lkg_warning_is_not_attributed_to_its_own_rebuild() {
        let scope = scope("project_0001", "agg_1", GenerationLayout::ProjectScope);
        let body = json!({
            "aggregate_index_id": "agg_1",
            "project_id": "project_0001",
            "status": "degraded",
            "warning": "aggregate_index_degraded:aggregate_index_source_scan_incomplete: members [a]",
        });
        let projection = project(&scope, &body).expect("projected");
        assert_eq!(
            projection.value["failure"]["scope"], "degrading_attempt",
            "degraded LKG 上的另一代失败不得记成 LKG 自身的重建失败"
        );
    }

    #[test]
    fn old_scan_envelope_and_missing_coverage_remain_distinguishable_without_raw_text() {
        let scope = scope("project_0001", "agg_1", GenerationLayout::ProjectScope);
        let mut body =
            json!({"aggregate_index_id":"agg_1", "project_id":"project_0001", "status":"stale"});
        for (fact, kind) in [
            (
                "source scan incomplete (permission/io/depth/entry/time limit) for included members [x]",
                "scan_incomplete",
            ),
            (
                "coverage state unknown for included members [x]",
                "coverage_unknown",
            ),
        ] {
            body["warning"] = json!(format!(
                "aggregate_index_failed:aggregate_index_source_scan_incomplete: {fact}"
            ));
            let result = project(&scope, &body).unwrap().value;
            assert_eq!(result["failure"]["fact_kind"], kind);
            assert!(!result.to_string().contains(fact));
        }
        body["warning"] =
            json!("aggregate_index_failed:private_unregistered_token: arbitrary text");
        let result = project(&scope, &body).unwrap();
        assert!(result.value.get("failure").is_none());
        assert!(
            !result
                .value
                .to_string()
                .contains("private_unregistered_token")
        );
    }

    #[test]
    fn unrecognized_warning_text_is_never_exported() {
        let scope = scope("project_0001", "agg_1", GenerationLayout::ProjectScope);
        let body = json!({
            "aggregate_index_id": "agg_1",
            "project_id": "project_0001",
            "status": "active",
            "warning": "some free text with /abs/path and token sk-live-X",
        });
        let projection = project(&scope, &body).expect("projected");
        assert!(projection.value.get("failure").is_none());
        assert!(
            projection
                .notes
                .iter()
                .any(|note| note == "warning_unrecognized")
        );
        let text = serde_json::to_string(&projection.value).expect("json");
        assert!(!text.contains("sk-live-X"));
        assert!(!text.contains("/abs/path"));
    }

    #[test]
    fn unknown_status_is_not_passed_through() {
        let scope = scope("project_0001", "agg_1", GenerationLayout::ProjectScope);
        let body = json!({
            "aggregate_index_id": "agg_1",
            "project_id": "project_0001",
            "status": "totally-unknown /abs/path",
        });
        let projection = project(&scope, &body).expect("projected");
        assert!(projection.value.get("status").is_none());
        assert!(
            projection
                .notes
                .iter()
                .any(|note| note == "status_unrecognized")
        );
    }

    #[test]
    fn old_record_without_optional_fields_is_projected_without_fabrication() {
        let scope = scope("project_0001", "agg_1", GenerationLayout::ProjectScope);
        let body = json!({
            "aggregate_index_id": "agg_1",
            "project_id": "project_0001",
            "status": "active",
        });
        let projection = project(&scope, &body).expect("projected");
        assert!(projection.value.get("source_scan").is_none());
        assert!(projection.value.get("failure").is_none());
        assert!(
            projection
                .notes
                .iter()
                .any(|note| note == "membership_revision_missing")
        );
    }
}
