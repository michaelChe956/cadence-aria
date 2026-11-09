//! 知识库 `manifest.yaml` 的读取与结构化投影，以及 skill 契约里
//! `coverage.initialization` 八项结构不变量的判定（读侧底座，Task 2）。
//!
//! 语义约定：
//! - `manifest.yaml` 不存在 ⇒ `exists == false`（不报错）；
//! - 文件存在但 YAML 不可解析 / 缺 `schema_version` ⇒ `exists == true` 且
//!   `schema_version == None`，由调用方映射为「版本不支持」失败面，
//!   不得静默当作空知识库；
//! - [`manifest_invariants`] 只判「结构与取值」；产物是否真的齐全属 skill
//!   门禁，平台不重复实现。
//!
//! 解析直接用仓库既有依赖 `serde_yaml`（0.9.34，经裁决从
//! `[dev-dependencies]` 移入 `[dependencies]`，版本逐字不变）：manifest
//! 由 skill 机器生成、含嵌套映射与「对象列表」两种结构，完整 YAML 解析
//! 的正确性优于任何手写子集。这里只提取本模块需要的字段：
//! `schema_version` / `generated_at` / `coverage.initialization.*` /
//! `scope.{configurations,api,pages}.status` / `update.last_change_package`
//! 与 `update.processed_packages` 是否存在，其余字段一律不读。

use std::path::Path;

use serde_yaml::Value;

use crate::product::json_store::ProductStoreError;

/// `initialization.status` 允许的取值（skill 契约）。
const INITIALIZATION_STATUSES: [&str; 2] = ["in_progress", "complete"];
/// `initialization.global_validation` 允许的取值（skill 契约）。
const GLOBAL_VALIDATION_STATUSES: [&str; 3] = ["pending", "failed", "passed"];
/// `completed_stages` 的固定顺序（`global-validation` 只能最后）。
const STAGE_ORDER: [&str; 5] = ["base-info", "api", "pages", "overview", "global-validation"];
/// `skipped_stages` 只允许跳过的领域阶段。
const SKIPPABLE_STAGES: [&str; 2] = ["api", "pages"];
/// scope 状态里表示「本领域不适用」的取值。
const NOT_APPLICABLE: &str = "不适用";

/// `manifest.yaml` 的结构化只读快照（知识库状态投影 DTO）。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KnowledgeBaseManifestSnapshot {
    pub exists: bool,
    pub schema_version: Option<String>,
    /// in_progress | complete
    pub initialization_status: Option<String>,
    /// pending | failed | passed
    pub global_validation: Option<String>,
    pub completed_stages: Vec<String>,
    pub skipped_stages: Vec<KnowledgeBaseSkippedStage>,
    pub completed_at: Option<String>,
    /// 全量 | 指定 | 不适用
    pub configurations_scope: Option<String>,
    pub api_scope: Option<String>,
    pub pages_scope: Option<String>,
    pub generated_at: Option<String>,
    pub has_last_change_package: bool,
    pub has_processed_packages: bool,
}

/// `skipped_stages` 的单项：只有 stage + reason 两个字段。
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct KnowledgeBaseSkippedStage {
    pub stage: String,
    pub reason: String,
}

/// 读取 `<kb_root>/manifest.yaml` 并投影为结构化快照。
///
/// 「文件在、但读不出合法内容」（含非 UTF-8、YAML 不可解析）一律降级为
/// `exists == true` + `schema_version == None`（版本未知），只有环境级
/// IO 故障才返回 `Err`。
pub fn read_manifest(kb_root: &Path) -> Result<KnowledgeBaseManifestSnapshot, ProductStoreError> {
    let manifest_path = kb_root.join("manifest.yaml");
    if !manifest_path.exists() {
        return Ok(KnowledgeBaseManifestSnapshot::default());
    }
    let raw = match std::fs::read_to_string(&manifest_path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            return Ok(degraded_snapshot());
        }
        Err(error) => {
            return Err(ProductStoreError::Io(format!(
                "read {}: {error}",
                manifest_path.display()
            )));
        }
    };
    Ok(parse_manifest(&raw))
}

/// 判定 `coverage.initialization` 的八项结构不变量，返回违规描述列表
/// （空 = 合法）。只判结构与取值，不校验磁盘产物是否真的齐全。
pub fn manifest_invariants(snapshot: &KnowledgeBaseManifestSnapshot) -> Vec<String> {
    let mut violations = Vec::new();
    let rank = |stage: &str| STAGE_ORDER.iter().position(|known| *known == stage);
    let completed = &snapshot.completed_stages;

    // ① status 取值只能是 in_progress / complete。
    match snapshot.initialization_status.as_deref() {
        None => violations.push(
            "`coverage.initialization.status` 缺失（只允许 in_progress 或 complete）".to_string(),
        ),
        Some(status) if !INITIALIZATION_STATUSES.contains(&status) => violations.push(format!(
            "`coverage.initialization.status` 取值非法：{status}（只允许 in_progress 或 complete）"
        )),
        _ => {}
    }

    // ② global_validation 取值只能是 pending / failed / passed。
    match snapshot.global_validation.as_deref() {
        None => violations.push(
            "`coverage.initialization.global_validation` 缺失（只允许 pending、failed 或 passed）"
                .to_string(),
        ),
        Some(value) if !GLOBAL_VALIDATION_STATUSES.contains(&value) => violations.push(format!(
            "`coverage.initialization.global_validation` 取值非法：{value}（只允许 pending、failed 或 passed）"
        )),
        _ => {}
    }

    // ③ completed_stages：成员合法、无重复、保持固定顺序。
    for (index, stage) in completed.iter().enumerate() {
        if rank(stage).is_none() {
            violations.push(format!(
                "`coverage.initialization.completed_stages` 含未知阶段：{stage}（只允许 base-info、api、pages、overview、global-validation）"
            ));
        }
        if completed[..index].contains(stage) {
            violations.push(format!(
                "`coverage.initialization.completed_stages` 含重复阶段：{stage}"
            ));
        }
    }
    for pair in completed.windows(2) {
        if let (Some(prev), Some(next)) = (rank(&pair[0]), rank(&pair[1]))
            && prev > next
        {
            violations.push(format!(
                "`coverage.initialization.completed_stages` 顺序非法：{} 不能排在 {} 之后",
                pair[1], pair[0]
            ));
        }
    }

    // ④ skipped_stages：只有 stage+reason、stage 只能 api/pages、reason 非空、
    //    不与 completed_stages 重叠。
    for skipped in &snapshot.skipped_stages {
        if !SKIPPABLE_STAGES.contains(&skipped.stage.as_str()) {
            violations.push(format!(
                "`coverage.initialization.skipped_stages` 含非法阶段：{}（只允许 api 或 pages）",
                skipped.stage
            ));
        }
        if skipped.reason.trim().is_empty() {
            violations.push(format!(
                "`coverage.initialization.skipped_stages` 中 {} 的 reason 为空",
                skipped.stage
            ));
        }
        if completed.iter().any(|stage| stage == &skipped.stage) {
            violations.push(format!(
                "`coverage.initialization.skipped_stages` 的 {} 同时出现在 completed_stages",
                skipped.stage
            ));
        }
    }

    // ⑤ 不适用领域只能出现在 skipped_stages（api / pages 同理）。
    for (domain, scope) in [("api", &snapshot.api_scope), ("pages", &snapshot.pages_scope)] {
        if scope.as_deref() == Some(NOT_APPLICABLE)
            && completed.iter().any(|stage| stage == domain)
        {
            violations.push(format!(
                "`scope.{domain}.status` 为「不适用」，但 {domain} 出现在 completed_stages"
            ));
        }
    }

    // ⑥/⑦/⑧ 按 status 分支收尾。
    match snapshot.initialization_status.as_deref() {
        Some("in_progress") => {
            if snapshot.completed_at.is_some() {
                violations
                    .push("`status` 为 in_progress，但 `completed_at` 已有值".to_string());
            }
            if completed.iter().any(|stage| stage == "global-validation") {
                violations.push(
                    "`status` 为 in_progress，但 `completed_stages` 已含 global-validation"
                        .to_string(),
                );
            }
        }
        Some("complete") => {
            for required in ["base-info", "overview", "global-validation"] {
                if !completed.iter().any(|stage| stage == required) {
                    violations.push(format!(
                        "`status` 为 complete，但 `completed_stages` 缺少必填阶段：{required}"
                    ));
                }
            }
            for (domain, scope) in [("api", &snapshot.api_scope), ("pages", &snapshot.pages_scope)]
            {
                let done = completed.iter().any(|stage| stage == domain);
                let skipped = snapshot
                    .skipped_stages
                    .iter()
                    .any(|item| item.stage == domain);
                if scope.as_deref() == Some(NOT_APPLICABLE) {
                    if !done && !skipped {
                        violations.push(format!(
                            "`status` 为 complete，但「不适用」领域 {domain} 未记录在 `skipped_stages`"
                        ));
                    }
                } else if !done {
                    violations.push(format!(
                        "`status` 为 complete，但适用领域 {domain} 未出现在 `completed_stages`"
                    ));
                } else if skipped {
                    violations.push(format!(
                        "`status` 为 complete，但适用领域 {domain} 出现在 `skipped_stages`"
                    ));
                }
            }
            if snapshot.global_validation.as_deref() != Some("passed") {
                violations.push(format!(
                    "`status` 为 complete，但 `global_validation` 不是 passed（当前为 {}）",
                    snapshot.global_validation.as_deref().unwrap_or("缺失")
                ));
            }
            if snapshot
                .completed_at
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
            {
                violations.push("`status` 为 complete，但 `completed_at` 为空".to_string());
            }
        }
        _ => {}
    }
    violations
}

/// 「文件存在但读不出合法内容」的降级快照：版本未知，其余字段全空。
fn degraded_snapshot() -> KnowledgeBaseManifestSnapshot {
    KnowledgeBaseManifestSnapshot {
        exists: true,
        ..KnowledgeBaseManifestSnapshot::default()
    }
}

/// 解析 manifest 文本：YAML 不可解析或根不是映射 ⇒ 降级快照（版本未知）。
fn parse_manifest(raw: &str) -> KnowledgeBaseManifestSnapshot {
    let root: Value = match serde_yaml::from_str(raw) {
        Ok(value) => value,
        Err(_) => return degraded_snapshot(),
    };
    if !root.is_mapping() {
        return degraded_snapshot();
    }
    KnowledgeBaseManifestSnapshot {
        exists: true,
        schema_version: scalar_at(&root, &["schema_version"]),
        initialization_status: scalar_at(&root, &["coverage", "initialization", "status"]),
        global_validation: scalar_at(&root, &["coverage", "initialization", "global_validation"]),
        completed_stages: string_list_at(&root, &["coverage", "initialization", "completed_stages"])
            .unwrap_or_default(),
        skipped_stages: skipped_stages_at(&root, &["coverage", "initialization", "skipped_stages"])
            .unwrap_or_default(),
        completed_at: scalar_at(&root, &["coverage", "initialization", "completed_at"]),
        configurations_scope: scalar_at(&root, &["scope", "configurations", "status"]),
        api_scope: scalar_at(&root, &["scope", "api", "status"]),
        pages_scope: scalar_at(&root, &["scope", "pages", "status"]),
        generated_at: scalar_at(&root, &["generated_at"]),
        has_last_change_package: key_exists(&root, &["update", "last_change_package"]),
        has_processed_packages: key_exists(&root, &["update", "processed_packages"]),
    }
}

/// 取嵌套映射路径上的非空字符串标量（路径缺失或非字符串 ⇒ None）。
fn scalar_at(root: &Value, path: &[&str]) -> Option<String> {
    let text = node_at(root, path)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// 判断嵌套映射路径上的 key 是否存在（值是什么不关心）。
fn key_exists(root: &Value, path: &[&str]) -> bool {
    node_at(root, path).is_some()
}

/// 取嵌套映射路径上的字符串列表（序列里的非字符串项被忽略）。
fn string_list_at(root: &Value, path: &[&str]) -> Option<Vec<String>> {
    let sequence = node_at(root, path)?.as_sequence()?;
    Some(
        sequence
            .iter()
            .filter_map(|item| item.as_str().map(str::to_string))
            .collect(),
    )
}

/// 取嵌套映射路径上的 skipped_stages 列表；缺 stage/reason 的残缺项
/// 以空串入列，交给不变量判定去报。
fn skipped_stages_at(root: &Value, path: &[&str]) -> Option<Vec<KnowledgeBaseSkippedStage>> {
    let sequence = node_at(root, path)?.as_sequence()?;
    Some(
        sequence
            .iter()
            .map(|item| KnowledgeBaseSkippedStage {
                stage: item.get("stage").and_then(Value::as_str).unwrap_or_default().to_string(),
                reason: item
                    .get("reason")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_string(),
            })
            .collect(),
    )
}

/// 沿嵌套映射路径下钻到目标节点。
fn node_at<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    let mut current = root;
    for &key in path {
        current = current.get(key)?;
    }
    Some(current)
}

#[cfg(test)]
mod tests {
    use super::{
        KnowledgeBaseManifestSnapshot, KnowledgeBaseSkippedStage, manifest_invariants, read_manifest,
    };
    use std::path::PathBuf;

    /// 一份合法的 complete 状态 manifest（skill 机器管理的固定形状）。
    const COMPLETE_FIXTURE: &str = "\
schema_version: \"4.0\"
generated_at: \"2026-11-09T00:00:00Z\"
coverage:
  initialization:
    status: complete
    global_validation: passed
    completed_stages:
      - base-info
      - overview
      - global-validation
    skipped_stages:
      - stage: api
        reason: 接口领域不适用
      - stage: pages
        reason: 页面领域不适用
    completed_at: \"2026-11-09T00:05:00Z\"
scope:
  configurations:
    status: 全量
  api:
    status: 不适用
  pages:
    status: 不适用
update:
  last_change_package: CHANGE-20261109-0001
  processed_packages:
    - CHANGE-20261109-0001
";

    /// 结构合法但缺 schema_version 的 manifest（解析为「版本未知」降级快照）。
    const NO_VERSION_FIXTURE: &str = "\
generated_at: \"2026-11-09T00:00:00Z\"
coverage:
  initialization:
    status: in_progress
    global_validation: pending
    completed_stages: [base-info, api]
";

    /// 测试夹具只允许落 /tmp：每个用例独占一个目录，写入 manifest.yaml。
    fn write_manifest_fixture(name: &str, content: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "cadence-kb-manifest-{}-{name}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("创建夹具目录");
        std::fs::write(dir.join("manifest.yaml"), content).expect("写入夹具 manifest");
        dir
    }

    /// 其余字段全部合法的最小快照，供不变量用例按需覆盖单字段。
    fn base_snapshot() -> KnowledgeBaseManifestSnapshot {
        KnowledgeBaseManifestSnapshot {
            exists: true,
            schema_version: Some("4.0".to_string()),
            initialization_status: Some("in_progress".to_string()),
            global_validation: Some("pending".to_string()),
            completed_stages: Vec::new(),
            skipped_stages: Vec::new(),
            completed_at: None,
            configurations_scope: Some("全量".to_string()),
            api_scope: None,
            pages_scope: None,
            generated_at: Some("2026-11-09T00:00:00Z".to_string()),
            has_last_change_package: false,
            has_processed_packages: false,
        }
    }

    #[test]
    fn manifest_snapshot_reports_missing_manifest() {
        let missing_root =
            std::env::temp_dir().join(format!("cadence-kb-manifest-{}-absent", std::process::id()));
        let snapshot = read_manifest(&missing_root).expect("读取缺失 manifest 不应报错");
        assert!(!snapshot.exists);
        assert_eq!(snapshot.schema_version, None);
        assert_eq!(snapshot.initialization_status, None);
    }

    #[test]
    fn manifest_snapshot_reads_schema_and_initialization_block() {
        let kb_root = write_manifest_fixture("complete", COMPLETE_FIXTURE);
        let snapshot = read_manifest(&kb_root).expect("读取夹具 manifest");
        assert!(snapshot.exists);
        assert_eq!(snapshot.schema_version.as_deref(), Some("4.0"));
        assert_eq!(snapshot.initialization_status.as_deref(), Some("complete"));
        assert_eq!(snapshot.global_validation.as_deref(), Some("passed"));
        assert_eq!(
            snapshot.completed_stages,
            ["base-info", "overview", "global-validation"]
        );
        assert_eq!(snapshot.completed_at.as_deref(), Some("2026-11-09T00:05:00Z"));
        assert_eq!(
            snapshot.skipped_stages,
            [
                KnowledgeBaseSkippedStage {
                    stage: "api".to_string(),
                    reason: "接口领域不适用".to_string(),
                },
                KnowledgeBaseSkippedStage {
                    stage: "pages".to_string(),
                    reason: "页面领域不适用".to_string(),
                },
            ]
        );
        assert_eq!(snapshot.configurations_scope.as_deref(), Some("全量"));
        assert_eq!(snapshot.api_scope.as_deref(), Some("不适用"));
        assert_eq!(snapshot.pages_scope.as_deref(), Some("不适用"));
        assert_eq!(snapshot.generated_at.as_deref(), Some("2026-11-09T00:00:00Z"));
        assert!(snapshot.has_last_change_package);
        assert!(snapshot.has_processed_packages);
    }

    #[test]
    fn manifest_snapshot_treats_unparseable_yaml_as_version_unknown() {
        let kb_root = write_manifest_fixture("garbage", "{{{ 不是 YAML\n\t!!!broken");
        let snapshot = read_manifest(&kb_root).expect("读取垃圾 manifest 不应报错");
        assert!(snapshot.exists);
        assert_eq!(snapshot.schema_version, None);
        assert_eq!(snapshot.initialization_status, None);
    }

    #[test]
    fn manifest_snapshot_reports_missing_schema_version_as_none() {
        let kb_root = write_manifest_fixture("no-version", NO_VERSION_FIXTURE);
        let snapshot = read_manifest(&kb_root).expect("读取无版本 manifest");
        assert!(snapshot.exists);
        assert_eq!(snapshot.schema_version, None);
        assert_eq!(snapshot.initialization_status.as_deref(), Some("in_progress"));
        assert_eq!(snapshot.completed_stages, ["base-info", "api"]);
    }

    #[test]
    fn manifest_invariants_reject_unknown_status() {
        let mut snapshot = base_snapshot();
        snapshot.initialization_status = Some("bogus".to_string());
        let violations = manifest_invariants(&snapshot);
        assert!(!violations.is_empty(), "violations: {violations:?}");
    }

    #[test]
    fn manifest_invariants_reject_out_of_order_stages() {
        let mut snapshot = base_snapshot();
        snapshot.completed_stages = vec!["api".to_string(), "base-info".to_string()];
        assert!(!manifest_invariants(&snapshot).is_empty());
    }

    #[test]
    fn manifest_invariants_reject_skipped_overlap() {
        let mut snapshot = base_snapshot();
        snapshot.completed_stages = vec!["base-info".to_string(), "api".to_string()];
        snapshot.skipped_stages = vec![KnowledgeBaseSkippedStage {
            stage: "api".to_string(),
            reason: "接口领域不适用".to_string(),
        }];
        assert!(!manifest_invariants(&snapshot).is_empty());
    }

    #[test]
    fn manifest_invariants_accept_legal_in_progress() {
        let mut snapshot = base_snapshot();
        snapshot.completed_stages = vec!["base-info".to_string()];
        assert!(manifest_invariants(&snapshot).is_empty());
    }

    #[test]
    fn manifest_invariants_reject_complete_missing_required_stage() {
        let mut snapshot = base_snapshot();
        snapshot.initialization_status = Some("complete".to_string());
        snapshot.global_validation = Some("passed".to_string());
        snapshot.completed_stages = vec![
            "base-info".to_string(),
            "global-validation".to_string(),
        ];
        snapshot.completed_at = Some("2026-11-09T00:05:00Z".to_string());
        let violations = manifest_invariants(&snapshot);
        assert!(!violations.is_empty(), "violations: {violations:?}");
    }

    #[test]
    fn manifest_invariants_accept_legal_complete_snapshot() {
        let kb_root = write_manifest_fixture("legal-complete", COMPLETE_FIXTURE);
        let snapshot = read_manifest(&kb_root).expect("读取夹具 manifest");
        assert!(manifest_invariants(&snapshot).is_empty());
    }
}
