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
//! `scope.{configurations,api,pages}.status`，以及 `update.last_change_package`
//! 映射与 `update.processed_packages` 列表是否非空（内容判定），其余一律不读。

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
    // 不做 exists() 预检：stat 故障（如 EACCES）不能被误报成「无知识库」，
    // 统一在读取处分流——NotFound（含读取间隙被删）归「无知识库」。
    match std::fs::read_to_string(&manifest_path) {
        Ok(raw) => Ok(parse_manifest(&raw)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Ok(KnowledgeBaseManifestSnapshot::default())
        }
        Err(error) if error.kind() == std::io::ErrorKind::InvalidData => {
            // 非 UTF-8：文件在但读不出合法内容，按版本未知降级。
            Ok(degraded_snapshot())
        }
        Err(error) => Err(ProductStoreError::Io(format!(
            "read {}: {error}",
            manifest_path.display()
        ))),
    }
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
    for (index, skipped) in snapshot.skipped_stages.iter().enumerate() {
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
        if snapshot.skipped_stages[..index]
            .iter()
            .any(|prev| prev.stage == skipped.stage)
        {
            violations.push(format!(
                "`coverage.initialization.skipped_stages` 含重复阶段：{}",
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

    // ⑤ scope 与 completed/skipped 的一致性：不区分 status（scope 缺失按适用
    //    从严）——「不适用 ⇒ 必须且只能在 skipped_stages」「适用 ⇒ 不得跳过」。
    for (domain, scope) in [
        ("api", &snapshot.api_scope),
        ("pages", &snapshot.pages_scope),
    ] {
        let done = completed.iter().any(|stage| stage == domain);
        let skipped = snapshot
            .skipped_stages
            .iter()
            .any(|item| item.stage == domain);
        if scope.as_deref() == Some(NOT_APPLICABLE) {
            if done {
                violations.push(format!(
                    "`scope.{domain}.status` 为「不适用」，但 {domain} 出现在 `completed_stages`"
                ));
            }
            if !skipped {
                violations.push(format!(
                    "`scope.{domain}.status` 为「不适用」，但 {domain} 未记录在 `skipped_stages`"
                ));
            }
        } else if skipped {
            violations.push(format!(
                "`scope.{domain}.status` 非「不适用」，但 {domain} 出现在 `skipped_stages`（适用领域不得跳过）"
            ));
        }
    }

    // ⑥/⑦/⑧ 按 status 分支收尾。
    match snapshot.initialization_status.as_deref() {
        Some("in_progress") => {
            if snapshot.completed_at.is_some() {
                violations.push("`status` 为 in_progress，但 `completed_at` 已有值".to_string());
            }
            if completed.iter().any(|stage| stage == "global-validation") {
                violations.push(
                    "`status` 为 in_progress，但 `completed_stages` 已含 global-validation"
                        .to_string(),
                );
            }
            if snapshot.global_validation.as_deref() == Some("passed") {
                violations.push(
                    "`status` 为 in_progress，但 `global_validation` 已是 passed（只允许 pending 或 failed）"
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
            for (domain, scope) in [
                ("api", &snapshot.api_scope),
                ("pages", &snapshot.pages_scope),
            ] {
                // 「不适用 ⇒ 必须且只能在 skipped_stages」「适用 ⇒ 不得跳过」
                // 已由 ⑤ 无条件判定；complete 下只需补「适用 ⇒ 必须完成」。
                if scope.as_deref() != Some(NOT_APPLICABLE)
                    && !completed.iter().any(|stage| stage == domain)
                {
                    violations.push(format!(
                        "`status` 为 complete，但适用领域 {domain} 未出现在 completed_stages"
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
        completed_stages: string_list_at(
            &root,
            &["coverage", "initialization", "completed_stages"],
        ),
        skipped_stages: skipped_stages_at(&root, &["coverage", "initialization", "skipped_stages"]),
        completed_at: scalar_at(&root, &["coverage", "initialization", "completed_at"]),
        configurations_scope: scalar_at(&root, &["scope", "configurations", "status"]),
        api_scope: scalar_at(&root, &["scope", "api", "status"]),
        pages_scope: scalar_at(&root, &["scope", "pages", "status"]),
        generated_at: scalar_at(&root, &["generated_at"]),
        has_last_change_package: non_empty_mapping_at(&root, &["update", "last_change_package"]),
        has_processed_packages: non_empty_sequence_at(&root, &["update", "processed_packages"]),
    }
}

/// 取嵌套映射路径上的非空字符串标量（路径缺失或非字符串 ⇒ None）。
fn scalar_at(root: &Value, path: &[&str]) -> Option<String> {
    let text = node_at(root, path)?.as_str()?.trim();
    (!text.is_empty()).then(|| text.to_string())
}

/// 判断嵌套映射路径上的映射是否非空（key 存在且映射至少含一个键）。
///
/// skill 模板固定写 `update.last_change_package: {}`，key 存在性对每个真实
/// manifest 恒为 true，因此按内容判定，全新知识库不会误报为「有变更包」。
fn non_empty_mapping_at(root: &Value, path: &[&str]) -> bool {
    node_at(root, path)
        .and_then(Value::as_mapping)
        .is_some_and(|mapping| !mapping.is_empty())
}

/// 判断嵌套映射路径上的序列是否非空（`update.processed_packages: []` 同理）。
fn non_empty_sequence_at(root: &Value, path: &[&str]) -> bool {
    node_at(root, path)
        .and_then(Value::as_sequence)
        .is_some_and(|sequence| !sequence.is_empty())
}

/// 取嵌套映射路径上的字符串列表。路径缺失按空列表；key 存在但非序列
/// （含 null）折叠为单个空串、序列内非字符串项转为字符串形态入列——两者
/// 都注定过不了不变量 ③ 的已知阶段判定，损坏不会被静默吞掉。
fn string_list_at(root: &Value, path: &[&str]) -> Vec<String> {
    let Some(value) = node_at(root, path) else {
        return Vec::new();
    };
    let Some(sequence) = value.as_sequence() else {
        return vec![String::new()];
    };
    sequence
        .iter()
        .map(|item| match item.as_str() {
            Some(text) => text.to_string(),
            None => serde_yaml::to_string(item)
                .map(|text| text.trim().to_string())
                .unwrap_or_default(),
        })
        .collect()
}

/// 取嵌套映射路径上的 skipped_stages 列表。路径缺失按空列表；key 存在但
/// 非序列（含 null）折叠为单个残缺项；缺 stage/reason 或取值非字符串的
/// 残缺项以空串入列——都交给不变量 ④ 报违规，不静默吞掉。
fn skipped_stages_at(root: &Value, path: &[&str]) -> Vec<KnowledgeBaseSkippedStage> {
    let Some(value) = node_at(root, path) else {
        return Vec::new();
    };
    let Some(sequence) = value.as_sequence() else {
        return vec![KnowledgeBaseSkippedStage::default()];
    };
    sequence
        .iter()
        .map(|item| KnowledgeBaseSkippedStage {
            stage: item
                .get("stage")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            reason: item
                .get("reason")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        })
        .collect()
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
        KnowledgeBaseManifestSnapshot, KnowledgeBaseSkippedStage, manifest_invariants,
        read_manifest,
    };
    use crate::product::json_store::ProductStoreError;
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
  last_change_package:
    change_package_id: CHANGE-20261109-0001
    idempotency_key: CHANGE-20261109-0001
    processed_at: \"2026-11-09T00:10:00Z\"
  processed_packages:
    - change_package_id: CHANGE-20261109-0001
      idempotency_key: CHANGE-20261109-0001
      processed_at: \"2026-11-09T00:10:00Z\"
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

    /// 全新知识库（bootstrap 模板原样）：update 两键存在但内容为空。
    const FRESH_KB_FIXTURE: &str = "\
schema_version: \"4.0\"
coverage:
  initialization:
    status: in_progress
    global_validation: pending
    completed_stages: []
    skipped_stages: []
    completed_at: \"\"
update:
  last_change_package: {}
  processed_packages: []
";

    /// 测试夹具只允许落 /tmp：每个用例独占一个目录，写入 manifest.yaml。
    fn write_manifest_fixture(name: &str, content: impl AsRef<[u8]>) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("cadence-kb-manifest-{}-{name}", std::process::id()));
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
        assert_eq!(
            snapshot.completed_at.as_deref(),
            Some("2026-11-09T00:05:00Z")
        );
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
        assert_eq!(
            snapshot.generated_at.as_deref(),
            Some("2026-11-09T00:00:00Z")
        );
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
        assert_eq!(
            snapshot.initialization_status.as_deref(),
            Some("in_progress")
        );
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
        snapshot.completed_stages = vec!["base-info".to_string(), "global-validation".to_string()];
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

    #[test]
    fn manifest_snapshot_reports_fresh_update_block_as_no_change() {
        let kb_root = write_manifest_fixture("fresh", FRESH_KB_FIXTURE);
        let snapshot = read_manifest(&kb_root).expect("读取全新知识库 manifest");
        assert!(snapshot.exists);
        // skill 模板固定写 last_change_package: {} / processed_packages: []：
        // key 存在不算数，内容为空 ⇒ 没有待处理变更包。
        assert!(!snapshot.has_last_change_package);
        assert!(!snapshot.has_processed_packages);
    }

    #[test]
    fn manifest_snapshot_treats_non_utf8_as_version_unknown() {
        let kb_root = write_manifest_fixture("non-utf8", [0xff_u8, 0xfe, 0x00, 0xd8]);
        let snapshot = read_manifest(&kb_root).expect("读取非 UTF-8 manifest 不应报错");
        assert!(snapshot.exists);
        assert_eq!(snapshot.schema_version, None);
        assert_eq!(snapshot.initialization_status, None);
    }

    #[test]
    fn manifest_snapshot_reports_io_error_when_manifest_unreadable() {
        let dir = std::env::temp_dir().join(format!(
            "cadence-kb-manifest-{}-not-readable",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        // manifest.yaml 处放一个目录：read_to_string 得到 EISDIR 之类的
        // 环境 IO 故障，应报 Err(Io)，不得折叠成「无知识库」或降级快照。
        std::fs::create_dir_all(dir.join("manifest.yaml")).expect("创建目录型夹具");
        let result = read_manifest(&dir);
        assert!(
            matches!(result, Err(ProductStoreError::Io(_))),
            "期望 Io 错误，实际：{result:?}"
        );
    }

    #[test]
    fn manifest_invariants_flag_non_sequence_completed_stages() {
        // key 存在但不是序列：损坏必须可见，不得折叠为空列表后静默过门禁。
        let kb_root = write_manifest_fixture(
            "completed-scalar",
            "coverage:\n  initialization:\n    status: in_progress\n    global_validation: pending\n    completed_stages: base-info\n",
        );
        let snapshot = read_manifest(&kb_root).expect("读取类型损坏 manifest");
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations
                .iter()
                .any(|item| item.contains("completed_stages")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_flag_non_string_stage_items() {
        let kb_root = write_manifest_fixture(
            "stage-number",
            "coverage:\n  initialization:\n    status: in_progress\n    global_validation: pending\n    completed_stages: [base-info, 42]\n",
        );
        let snapshot = read_manifest(&kb_root).expect("读取含数字阶段的 manifest");
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations.iter().any(|item| item.contains("未知阶段")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_flag_non_sequence_skipped_stages() {
        let kb_root = write_manifest_fixture(
            "skipped-scalar",
            "coverage:\n  initialization:\n    status: in_progress\n    global_validation: pending\n    skipped_stages: api\n",
        );
        let snapshot = read_manifest(&kb_root).expect("读取类型损坏 manifest");
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations
                .iter()
                .any(|item| item.contains("skipped_stages")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_reject_invalid_global_validation() {
        let mut snapshot = base_snapshot();
        snapshot.global_validation = Some("bogus".to_string());
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations
                .iter()
                .any(|item| item.contains("global_validation")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_reject_na_domain_in_completed_stages() {
        let mut snapshot = base_snapshot();
        snapshot.api_scope = Some("不适用".to_string());
        snapshot.completed_stages = vec!["base-info".to_string(), "api".to_string()];
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations
                .iter()
                .any(|item| item.contains("不适用") && item.contains("completed_stages")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_reject_in_progress_with_completed_at() {
        let mut snapshot = base_snapshot();
        snapshot.completed_at = Some("2026-11-09T00:05:00Z".to_string());
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations.iter().any(|item| item.contains("completed_at")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_reject_in_progress_with_global_validation_stage() {
        let mut snapshot = base_snapshot();
        snapshot.completed_stages = vec![
            "base-info".to_string(),
            "overview".to_string(),
            "global-validation".to_string(),
        ];
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations
                .iter()
                .any(|item| item.contains("global-validation")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_reject_in_progress_with_passed_validation() {
        let mut snapshot = base_snapshot();
        snapshot.global_validation = Some("passed".to_string());
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations
                .iter()
                .any(|item| item.contains("in_progress") && item.contains("passed")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_reject_duplicate_skipped_stages() {
        let mut snapshot = base_snapshot();
        snapshot.api_scope = Some("不适用".to_string());
        snapshot.skipped_stages = vec![
            KnowledgeBaseSkippedStage {
                stage: "api".to_string(),
                reason: "接口领域不适用".to_string(),
            },
            KnowledgeBaseSkippedStage {
                stage: "api".to_string(),
                reason: "接口领域不适用".to_string(),
            },
        ];
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations.iter().any(|item| item.contains("重复")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_reject_na_domain_missing_from_skipped_in_progress() {
        let mut snapshot = base_snapshot();
        snapshot.api_scope = Some("不适用".to_string());
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations
                .iter()
                .any(|item| item.contains("未记录在 `skipped_stages`")),
            "violations: {violations:?}"
        );
    }

    #[test]
    fn manifest_invariants_reject_applicable_domain_skipped_in_progress() {
        let mut snapshot = base_snapshot();
        snapshot.api_scope = Some("指定".to_string());
        snapshot.skipped_stages = vec![KnowledgeBaseSkippedStage {
            stage: "api".to_string(),
            reason: "不该跳过".to_string(),
        }];
        let violations = manifest_invariants(&snapshot);
        assert!(
            violations.iter().any(|item| item.contains("不得跳过")),
            "violations: {violations:?}"
        );
    }
}
