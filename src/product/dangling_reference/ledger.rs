//! 销账留痕台账（REQ-DRG-16；技术方案 §3.5.2）。
//!
//! 落点：`{state_root}/entity-deletions/dangling-reference/cleanup.jsonl` ——
//! 状态根下**单一全局**的追加式 JSONL 台账（目录由本能力新建）。台账
//! MUST NOT 写进 issue 级删除墓碑目录、MUST NOT 扩 `DeletionEntityKind`、
//! MUST NOT 改写既有墓碑记录。
//!
//! 三条硬约束：
//! 1. **追加而非覆盖**：每次清理追加**一行**紧凑 JSON（`schema_version` 行版本
//!    字段与既有根级 JSONL 审计同规范），MUST NOT 复用 `write_json` 的整文件
//!    覆盖语义；
//! 2. **幂等去重**：追加前对既有行做**全量比对**（`record_path` + `target_id` +
//!    `action` 三元组），命中即返回 `Ok(false)` 且不写——同一清理动作重入
//!    不追加重复行（REQ-DRG-15 的留痕侧语义）；
//! 3. **保留期 + 按大小轮转**：活动文件达到
//!    [`DANGLING_REFERENCE_LEDGER_MAX_BYTES`] 时先轮转为
//!    `cleanup.jsonl.<UTC 紧凑时间戳>` 分片（`std::fs::rename`，同时间戳追加
//!    序号、绝不覆盖既有分片），随后清理早于
//!    [`DANGLING_REFERENCE_LEDGER_RETENTION_DAYS`] 天的分片。
//!
//! 本模块只做判定的落盘面：不决定删什么（判定见销账规则），也不执行删除。
//! 读取失败（活动文件不可读 / 行解析失败 / stat 失败）一律原样上抛，MUST NOT
//! 被当成「台账为空」——否则会静默追加重复行。

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Duration, NaiveDateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::json_store::ProductStoreError;

/// 留痕保留期（天）：早于此时长的轮转分片在每次追加时清理（设计 §7.2 R7）。
pub(crate) const DANGLING_REFERENCE_LEDGER_RETENTION_DAYS: u64 = 30;

/// 活动台账文件大小上限（字节）：达到即轮转出分片（16 MiB）。
pub(crate) const DANGLING_REFERENCE_LEDGER_MAX_BYTES: u64 = 16 * 1024 * 1024;

/// 轮转分片文件名里的紧凑 UTC 时间戳格式（`cleanup.jsonl.<stamp>`）。
const SEGMENT_STAMP_FORMAT: &str = "%Y%m%dT%H%M%SZ";

/// 「因哪个实体被删」（设计 §3.5.2 行形态的 `deleted_entity` 子结构）。
///
/// 解析不到作用域时对应子字段为 `None`（不猜）；`kind` / `id` 恒有值。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct DeletedEntityRef {
    /// 被删实体种类（墓碑路径段口径：`story` / `design` / `plan` 等）。
    pub(crate) kind: String,
    /// 被删实体 id。
    pub(crate) id: String,
    /// 被删实体所属 project（解析不到 → `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) project_id: Option<String>,
    /// 被删实体所属 issue（解析不到 → `None`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) issue_id: Option<String>,
}

/// 留痕行形态（REQ-DRG-16 三要素 + 证据等级；逐字对齐设计 §3.5.2）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct LedgerRow {
    /// 行格式版本，固定 1（与既有根级 JSONL 审计行同规范）。
    pub(crate) schema_version: u32,
    /// 幂等去重键 = [`cleanup_id`]（重入同键不重复追加）。
    pub(crate) cleanup_id: String,
    /// 何时（RFC3339）。
    pub(crate) cleaned_at: String,
    /// 销账动作（`delete_record` / `delete_record_group`）。
    pub(crate) action: String,
    /// 被清理记录的类型（判据表稳定标识）。
    pub(crate) record_type: String,
    /// 被清理记录的路径（相对应用状态根）。
    pub(crate) record_path: String,
    /// 引用目标类型。
    pub(crate) target_kind: String,
    /// 引用目标 id。
    pub(crate) target_id: String,
    /// 本次删除所依据的证据等级（`L1` / `L2`）。
    pub(crate) evidence_level: String,
    /// 因哪个实体被删。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) deleted_entity: Option<DeletedEntityRef>,
    /// 触发来源稳定短码（恒为 `dangling_reference_cleanup`）。
    pub(crate) trigger: String,
    /// 本批清理批次标识（供「清理后再巡检」对账）。
    pub(crate) run_id: String,
}

/// 留痕台账句柄：活动文件路径 + 轮转上限 + 保留期。
#[derive(Debug, Clone)]
pub(crate) struct DanglingReferenceLedger {
    path: PathBuf,
    max_bytes: u64,
    retention_days: u64,
}

impl DanglingReferenceLedger {
    /// 以生产默认上限（16 MiB / 30 天）构造。
    pub(crate) fn new(paths: &ProductAppPaths) -> Self {
        Self::with_limits(
            paths,
            DANGLING_REFERENCE_LEDGER_MAX_BYTES,
            DANGLING_REFERENCE_LEDGER_RETENTION_DAYS,
        )
    }

    /// 以显式上限构造（测试与 owner 调参用）。
    pub(crate) fn with_limits(
        paths: &ProductAppPaths,
        max_bytes: u64,
        retention_days: u64,
    ) -> Self {
        Self {
            path: paths.dangling_reference_ledger_path(),
            max_bytes,
            retention_days,
        }
    }

    /// 活动台账文件路径。
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }

    /// 读活动台账的全部行（活动文件不存在 → 空集）。
    ///
    /// 单行解析失败原样上抛（不跳过）：跳过会让同一清理动作被误判为「未留痕」
    /// 而重复追加，且掩盖台账损坏事实。
    ///
    /// 轮转出去的分片**不参与**读取：本入口只反映活动文件，分片按保留期
    /// 独立轮替（设计 §7.2 R7/R8 的去重范围即活动文件）。
    pub(crate) fn rows(&self) -> Result<Vec<LedgerRow>, ProductStoreError> {
        let content = match std::fs::read_to_string(&self.path) {
            Ok(content) => content,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    self.path.display()
                )));
            }
        };
        let mut rows = Vec::new();
        for (index, line) in content.lines().enumerate() {
            if line.trim().is_empty() {
                continue;
            }
            let row = serde_json::from_str::<LedgerRow>(line).map_err(|error| {
                ProductStoreError::Json(format!(
                    "{} line {} parse failed: {error}",
                    self.path.display(),
                    index + 1
                ))
            })?;
            rows.push(row);
        }
        Ok(rows)
    }

    /// 追加一行留痕；`Ok(true)` = 已写入，`Ok(false)` = 同清理动作已留痕（幂等）。
    ///
    /// 顺序：① 与既有行**全量比对**去重 → ② 活动文件达到上限则先轮转 →
    /// ③ 清理超保留期分片 → ④ 追加「一行紧凑 JSON + `\n`」并 flush。
    pub(crate) fn append(
        &self,
        row: &LedgerRow,
        now: DateTime<Utc>,
    ) -> Result<bool, ProductStoreError> {
        if self
            .rows()?
            .iter()
            .any(|existing| same_cleanup(existing, row))
        {
            return Ok(false);
        }
        self.rotate_if_needed(now)?;
        self.purge_expired_segments(now)?;
        self.append_line(row)?;
        Ok(true)
    }

    /// 活动文件达到上限 → 轮转为带 UTC 紧凑时间戳的分片。
    fn rotate_if_needed(&self, now: DateTime<Utc>) -> Result<(), ProductStoreError> {
        let size = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata.len(),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "stat {}: {error}",
                    self.path.display()
                )));
            }
        };
        if size < self.max_bytes {
            return Ok(());
        }
        let stamp = now.format(SEGMENT_STAMP_FORMAT).to_string();
        let mut target = self.dir().join(format!("{}.{stamp}", self.active_name()));
        // 同一时间戳内多次轮转（小上限测试 / 快速清扫）：追加序号，绝不覆盖既有分片。
        let mut suffix = 1u32;
        while path_exists(&target)? {
            target = self
                .dir()
                .join(format!("{}.{stamp}-{suffix}", self.active_name()));
            suffix += 1;
        }
        std::fs::rename(&self.path, &target).map_err(|error| {
            ProductStoreError::Io(format!(
                "rename {} to {}: {error}",
                self.path.display(),
                target.display()
            ))
        })
    }

    /// 清理超出保留期的轮转分片（文件名不可解析时间戳 → 保留，不误删）。
    fn purge_expired_segments(&self, now: DateTime<Utc>) -> Result<(), ProductStoreError> {
        let dir = self.dir();
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => {
                return Err(ProductStoreError::Io(format!(
                    "read {}: {error}",
                    dir.display()
                )));
            }
        };
        let cutoff = now - Duration::days(self.retention_days.min(i64::MAX as u64) as i64);
        for entry in entries {
            let entry = entry.map_err(|error| {
                ProductStoreError::Io(format!("read {} entry: {error}", dir.display()))
            })?;
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(segment_at) = self.segment_stamp(&name) else {
                continue;
            };
            if segment_at >= cutoff {
                continue;
            }
            std::fs::remove_file(entry.path()).map_err(|error| {
                ProductStoreError::Io(format!("remove {}: {error}", entry.path().display()))
            })?;
        }
        Ok(())
    }

    /// 从分片文件名解析轮转时刻；非本台账分片 / 时间戳不可解析 → `None`。
    fn segment_stamp(&self, name: &str) -> Option<DateTime<Utc>> {
        let rest = name
            .strip_prefix(self.active_name().as_str())?
            .strip_prefix('.')?;
        // 同一时间戳的第 n 次轮转带 `-<n>` 序号后缀。
        let core = rest.split('-').next()?;
        let naive = NaiveDateTime::parse_from_str(core, SEGMENT_STAMP_FORMAT).ok()?;
        Some(naive.and_utc())
    }

    /// 追加一行紧凑 JSON + `\n`（逐行文本追加，非整文件覆盖）。
    fn append_line(&self, row: &LedgerRow) -> Result<(), ProductStoreError> {
        if let Some(parent) = self.path.parent()
            && !parent.as_os_str().is_empty()
        {
            std::fs::create_dir_all(parent).map_err(|error| {
                ProductStoreError::Io(format!("create {}: {error}", parent.display()))
            })?;
        }
        let mut line =
            serde_json::to_vec(row).map_err(|error| ProductStoreError::Json(error.to_string()))?;
        line.push(b'\n');
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| {
                ProductStoreError::Io(format!("open {}: {error}", self.path.display()))
            })?;
        file.write_all(&line).map_err(|error| {
            ProductStoreError::Io(format!("write {}: {error}", self.path.display()))
        })?;
        file.flush().map_err(|error| {
            ProductStoreError::Io(format!("flush {}: {error}", self.path.display()))
        })
    }

    /// 活动文件所在目录。
    fn dir(&self) -> PathBuf {
        self.path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// 活动文件名（`cleanup.jsonl`；分片以它为前缀）。
    fn active_name(&self) -> String {
        self.path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "cleanup.jsonl".to_string())
    }
}

/// 同一清理动作判定：`record_path` + `target_id` + `action` 三元组全等。
fn same_cleanup(left: &LedgerRow, right: &LedgerRow) -> bool {
    left.record_path == right.record_path
        && left.target_id == right.target_id
        && left.action == right.action
}

/// 存在性判定（stat）：IO 失败上抛，MUST NOT 被当成「不存在」。
fn path_exists(path: &Path) -> Result<bool, ProductStoreError> {
    path.try_exists()
        .map_err(|error| ProductStoreError::Io(format!("stat {}: {error}", path.display())))
}

/// 幂等去重键：`sha256(长度前缀 + 单元分隔符 + record_path/target_id/action)`。
///
/// 长度前缀消除拼接边界歧义（`("ab","c")` 与 `("a","bc")` 必不同键）；
/// 碰撞由 `append` 的全量三元组比对兜底（设计 §7.2 R8）。
pub(crate) fn cleanup_id(record_path: &str, target_id: &str, action: &str) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();
    for field in [record_path, target_id, action] {
        hasher.update(field.len().to_string().as_bytes());
        hasher.update(b"\x1f");
        hasher.update(field.as_bytes());
        hasher.update(b"\x1f");
    }
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod ledger_tests {
    use super::*;
    use crate::product::app_paths::ProductAppPaths;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn row(record_path: &str, target_id: &str) -> LedgerRow {
        LedgerRow {
            schema_version: 1,
            cleanup_id: cleanup_id(record_path, target_id, "delete_record"),
            cleaned_at: "2026-10-28T10:00:00Z".to_string(),
            action: "delete_record".to_string(),
            record_type: "automation_plan_intent".to_string(),
            record_path: record_path.to_string(),
            target_kind: "story_spec".to_string(),
            target_id: target_id.to_string(),
            evidence_level: "L1".to_string(),
            deleted_entity: Some(DeletedEntityRef {
                kind: "story".to_string(),
                id: target_id.to_string(),
                project_id: Some("project_0001".to_string()),
                issue_id: Some("issue_0001".to_string()),
            }),
            trigger: "dangling_reference_cleanup".to_string(),
            run_id: "patrol_2026_10_28".to_string(),
        }
    }

    #[test]
    fn dangling_ledger_appends_rows_with_schema_version() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let ledger = DanglingReferenceLedger::new(&paths);
        assert_eq!(
            ledger.path(),
            paths
                .state_root()
                .join("entity-deletions")
                .join("dangling-reference")
                .join("cleanup.jsonl")
        );
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 10, 28, 10, 0, 0)
            .unwrap();
        assert!(
            ledger
                .append(&row("projects/p1/issues/i1/a.json", "story_spec_0005"), now)
                .unwrap()
        );

        let rows = ledger.rows().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].schema_version, 1);
        assert_eq!(rows[0].record_path, "projects/p1/issues/i1/a.json");
        assert_eq!(rows[0].target_id, "story_spec_0005");
        assert_eq!(rows[0].evidence_level, "L1");
        assert_eq!(
            rows[0].deleted_entity.as_ref().unwrap().id,
            "story_spec_0005".to_string()
        );
        let raw = std::fs::read_to_string(ledger.path()).unwrap();
        assert_eq!(raw.lines().count(), 1);
        assert!(raw.contains("\"schema_version\":1"), "{raw}");
    }

    #[test]
    fn dangling_ledger_dedupes_same_cleanup_id() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let ledger = DanglingReferenceLedger::new(&paths);
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 10, 28, 10, 0, 0)
            .unwrap();
        assert!(
            ledger
                .append(&row("projects/p1/issues/i1/a.json", "story_spec_0005"), now)
                .unwrap()
        );
        assert!(
            !ledger
                .append(&row("projects/p1/issues/i1/a.json", "story_spec_0005"), now)
                .unwrap()
        );
        assert_eq!(ledger.rows().unwrap().len(), 1);
        // 重入只留一行：活动文件的实际行数也必须仍是 1（不追加重复行）。
        assert_eq!(
            std::fs::read_to_string(ledger.path())
                .unwrap()
                .lines()
                .count(),
            1
        );
    }

    #[test]
    fn dangling_ledger_rotates_by_size_and_drops_expired_segments() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let ledger = DanglingReferenceLedger::with_limits(&paths, 64, 30);
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 10, 28, 10, 0, 0)
            .unwrap();
        for index in 0..5 {
            assert!(
                ledger
                    .append(
                        &row(
                            &format!("projects/p1/issues/i1/{index}.json"),
                            &format!("story_spec_{index:04}"),
                        ),
                        now,
                    )
                    .unwrap()
            );
        }
        let segments: Vec<_> = std::fs::read_dir(
            paths
                .state_root()
                .join("entity-deletions")
                .join("dangling-reference"),
        )
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().to_string())
        .filter(|name| name != "cleanup.jsonl")
        .collect();
        assert!(!segments.is_empty(), "超过阈值必须轮转出分片");
        assert!(
            segments
                .iter()
                .all(|name| name.starts_with("cleanup.jsonl.")),
            "{segments:?}"
        );

        let expired = paths
            .state_root()
            .join("entity-deletions")
            .join("dangling-reference")
            .join("cleanup.jsonl.20200101T000000Z");
        std::fs::write(&expired, b"{}\n").unwrap();
        ledger
            .append(
                &row("projects/p1/issues/i1/after.json", "story_spec_9999"),
                now,
            )
            .unwrap();
        assert!(!expired.exists(), "超过保留期的分片必须清理");
    }

    /// 行形态逐字对齐技术方案 §3.5.2：一行紧凑 JSON（键序固定、无空格）+ `\n`。
    ///
    /// 这同时是「字段齐全」证据：十二个顶层字段与 `deleted_entity` 的四个子字段
    /// 全部落盘，`cleanup_id` 与 [`cleanup_id`] 的取值一致。
    #[test]
    fn dangling_ledger_line_shape_matches_design() {
        let temp = TempDir::new().unwrap();
        let paths = ProductAppPaths::new(temp.path());
        let ledger = DanglingReferenceLedger::new(&paths);
        let now = chrono::Utc
            .with_ymd_and_hms(2026, 10, 28, 10, 0, 0)
            .unwrap();
        assert!(
            ledger
                .append(&row("projects/p1/issues/i1/a.json", "story_spec_0005"), now)
                .unwrap()
        );

        let expected = format!(
            concat!(
                "{{\"schema_version\":1,\"cleanup_id\":\"{}\",",
                "\"cleaned_at\":\"2026-10-28T10:00:00Z\",",
                "\"action\":\"delete_record\",",
                "\"record_type\":\"automation_plan_intent\",",
                "\"record_path\":\"projects/p1/issues/i1/a.json\",",
                "\"target_kind\":\"story_spec\",",
                "\"target_id\":\"story_spec_0005\",",
                "\"evidence_level\":\"L1\",",
                "\"deleted_entity\":{{\"kind\":\"story\",\"id\":\"story_spec_0005\",",
                "\"project_id\":\"project_0001\",\"issue_id\":\"issue_0001\"}},",
                "\"trigger\":\"dangling_reference_cleanup\",",
                "\"run_id\":\"patrol_2026_10_28\"}}\n"
            ),
            cleanup_id(
                "projects/p1/issues/i1/a.json",
                "story_spec_0005",
                "delete_record"
            )
        );
        let raw = std::fs::read_to_string(ledger.path()).unwrap();
        assert_eq!(raw, expected);
        for field in [
            "schema_version",
            "cleanup_id",
            "cleaned_at",
            "action",
            "record_type",
            "record_path",
            "target_kind",
            "target_id",
            "evidence_level",
            "deleted_entity",
            "trigger",
            "run_id",
        ] {
            assert!(
                raw.contains(&format!("\"{field}\":")),
                "缺字段 {field}：{raw}"
            );
        }
        assert_eq!(raw.lines().count(), 1);
    }
}
