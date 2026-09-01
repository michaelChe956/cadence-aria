// C-4 跨仓只读证据中介：attempt 分区审计 jsonl（Task 5）。
//
// 契约 REQ-COD-05「审计」；设计 §5.3。每次受控查询通过后追加一条
// `evidence_query` 审计条目到 attempt 分区 `evidence-audit.jsonl`（仿
// `coding_attempt_store/role_run_event.rs` 的进程内 mutex + sequence + 逐行 JSON
// 追加先例；C-2 `GatewayRunAudit` 为进程内内存结构不落盘，不复用）。
//
// 落盘行为：`append_evidence_audit` 在进程内 `EVIDENCE_AUDIT_LOG_MUTEX` 串行下，
// 先经 `next_jsonl_sequence`（逐行解析既有文件、坏 JSON 行返回 Err 不 panic）算出
// 下一条 sequence，再以「sequence + 记录字段」逐行 JSON 追加。`role` 字段由 T6
// 调用方拼装（含 `role_self_reported` 标记，设计 §5.2），本层原样持久化。
//
// 签名相对简报补充 `attempt: &CodingExecutionAttempt`：attempt 分区路径
// （`issue_lifecycle_root/{project}/{issue}/coding-attempts/{attempt_id}/`）需要
// project_id/issue_id，而 `EvidenceAuditRecord` 只带 attempt_id；与 T3
// `issue_evidence_token`、T4 `EvidenceBudgetLedger` 的显式依赖注入惯例一致。

use std::path::PathBuf;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_attempt_store::{append_jsonl, next_jsonl_sequence};
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::logical_codebase::evidence_index::EvidenceError;

/// attempt 分区审计 jsonl 文件名。
const EVIDENCE_AUDIT_FILE: &str = "evidence-audit.jsonl";

/// 进程内互斥：单 web 进程假设下的写者互斥（仿 `ROLE_RUN_EVENT_LOG_MUTEX` 先例）。
static EVIDENCE_AUDIT_LOG_MUTEX: Mutex<()> = Mutex::new(());

/// 单条证据查询审计记录（对应设计 §5.3 的 `evidence_query` 条目，serde snake_case）。
///
/// `role` 为 Coder/Reviewer 自报标注（含 `role_self_reported` 标记），由 T6 调用方拼装。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvidenceAuditRecord {
    pub attempt_id: String,
    pub role: String,
    pub query: String,
    pub hit_count: usize,
    pub result_chars: usize,
    pub snapshot_refs: Vec<String>,
    pub budget_remaining: usize,
    pub timestamp: String,
}

/// 审计 jsonl 的持久化行：`sequence`（递增序号，仿 role_run_event 先例）+ 记录字段。
#[derive(Serialize)]
struct EvidenceAuditLine<'a> {
    sequence: u64,
    #[serde(flatten)]
    record: &'a EvidenceAuditRecord,
}

/// 追加一条证据查询审计到 attempt 分区 `evidence-audit.jsonl`。
///
/// 在进程内互斥下：先按既有文件续算 sequence（坏 JSON 行 → `Err`，不 panic），
/// 再逐行 JSON 追加「sequence + 记录字段」。
pub fn append_evidence_audit(
    paths: &ProductAppPaths,
    attempt: &CodingExecutionAttempt,
    record: &EvidenceAuditRecord,
) -> Result<(), EvidenceError> {
    let path = audit_path(paths, attempt);
    let _guard = EVIDENCE_AUDIT_LOG_MUTEX
        .lock()
        .map_err(|error| EvidenceError::Io {
            message: format!("lock evidence audit log: {error}"),
        })?;
    let sequence = next_jsonl_sequence(&path).map_err(|error| EvidenceError::Io {
        message: format!("read evidence audit log {}: {error}", path.display()),
    })?;
    let line = EvidenceAuditLine { sequence, record };
    append_jsonl(&path, &line).map_err(|error| EvidenceError::Io {
        message: format!("write evidence audit log {}: {error}", path.display()),
    })?;
    Ok(())
}

/// attempt 分区审计 jsonl 路径：
/// `issue_lifecycle_root/{project}/{issue}/coding-attempts/{attempt_id}/evidence-audit.jsonl`
/// （与 `coding_attempt_store::attempt_dir` 模式一致）。
fn audit_path(paths: &ProductAppPaths, attempt: &CodingExecutionAttempt) -> PathBuf {
    paths
        .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
        .join("coding-attempts")
        .join(&attempt.id)
        .join(EVIDENCE_AUDIT_FILE)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::thread;

    use tempfile::TempDir;

    use super::*;
    use crate::product::coding_models::{
        CodingAttemptScope, CodingAttemptStatus, CodingExecutionStage,
    };
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::ProviderConfigSnapshot;

    const PROJECT_ID: &str = "project_0001";
    const ISSUE_ID: &str = "issue_0001";
    const ATTEMPT_ID: &str = "coding_attempt_0001";

    fn attempt_fixture() -> CodingExecutionAttempt {
        CodingExecutionAttempt {
            id: ATTEMPT_ID.to_string(),
            project_id: PROJECT_ID.to_string(),
            issue_id: ISSUE_ID.to_string(),
            work_item_id: "work_item_0001".to_string(),
            attempt_no: 1,
            scope: CodingAttemptScope::WorkItem,
            status: CodingAttemptStatus::Running,
            version: 0,
            manual_recovery_reason: None,
            admission_ticket_consumed_at: None,
            admission_kind: crate::product::coding_models::CodingAdmissionKind::LegacyGroup,
            stage: CodingExecutionStage::Coding,
            base_branch: "main".to_string(),
            branch_name: "aria/issues/issue_0001".to_string(),
            worktree_path: None,
            provider_config_snapshot: ProviderConfigSnapshot {
                author: ProviderName::Fake,
                reviewer: None,
                review_rounds: 0,
                permission_modes: Default::default(),
            },
            rework_count: 0,
            max_auto_rework: 0,
            work_item_group_id: None,
            current_work_item_id: Some("work_item_0001".to_string()),
            active_unit_id: None,
            head_commit: None,
            pushed_remote: None,
            review_request_id: None,
            provider_conversations: Vec::new(),
            created_at: "2026-08-11T00:00:00Z".to_string(),
            updated_at: "2026-08-11T00:00:00Z".to_string(),
            target_snapshot: None,
            completed_at: None,
        }
    }

    fn audit_fixture() -> (TempDir, ProductAppPaths, CodingExecutionAttempt) {
        let tmp = TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let attempt = attempt_fixture();
        (tmp, paths, attempt)
    }

    fn audit_file_path(paths: &ProductAppPaths) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(ATTEMPT_ID)
            .join(EVIDENCE_AUDIT_FILE)
    }

    fn record(query: &str, timestamp: &str) -> EvidenceAuditRecord {
        EvidenceAuditRecord {
            attempt_id: ATTEMPT_ID.to_string(),
            role: "coder(role_self_reported)".to_string(),
            query: query.to_string(),
            hit_count: 1,
            result_chars: 120,
            snapshot_refs: vec!["web/src/app.ts".to_string()],
            budget_remaining: 119_880,
            timestamp: timestamp.to_string(),
        }
    }

    fn seed_audit_file(paths: &ProductAppPaths, content: &str) {
        let path = audit_file_path(paths);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).expect("create audit parent dir");
        }
        std::fs::write(path, content).expect("seed audit file");
    }

    #[test]
    fn append_writes_two_valid_json_lines_with_incrementing_sequence() {
        let (_tmp, paths, attempt) = audit_fixture();

        let first = record("symbol_a", "2026-08-14T00:00:01Z");
        let second = record("symbol_b", "2026-08-14T00:00:02Z");
        append_evidence_audit(&paths, &attempt, &first).expect("first append");
        append_evidence_audit(&paths, &attempt, &second).expect("second append");

        let content = std::fs::read_to_string(audit_file_path(&paths)).expect("read audit");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2, "file must contain exactly 2 lines");

        let mut sequences = Vec::new();
        let mut records = Vec::new();
        for line in &lines {
            let value: serde_json::Value =
                serde_json::from_str(line).expect("each line must be valid JSON");
            sequences.push(value["sequence"].as_u64().expect("sequence must be u64"));
            records.push(
                serde_json::from_value::<EvidenceAuditRecord>(value.clone())
                    .expect("line must deserialize into record"),
            );
        }
        assert_eq!(sequences, vec![1, 2], "sequence must increment");
        assert_eq!(records, vec![first, second]);
    }

    #[test]
    fn sequence_continues_from_existing_file() {
        let (_tmp, paths, attempt) = audit_fixture();

        let existing = format!(
            "{}\n{}\n",
            serde_json::json!({
                "sequence": 1,
                "attempt_id": ATTEMPT_ID,
                "role": "reviewer(role_self_reported)",
                "query": "existing_a",
                "hit_count": 0,
                "result_chars": 0,
                "snapshot_refs": [],
                "budget_remaining": 120_000,
                "timestamp": "2026-08-14T00:00:00Z"
            }),
            serde_json::json!({
                "sequence": 2,
                "attempt_id": ATTEMPT_ID,
                "role": "reviewer(role_self_reported)",
                "query": "existing_b",
                "hit_count": 0,
                "result_chars": 0,
                "snapshot_refs": [],
                "budget_remaining": 120_000,
                "timestamp": "2026-08-14T00:00:00Z"
            })
        );
        seed_audit_file(&paths, &existing);
        append_evidence_audit(
            &paths,
            &attempt,
            &record("symbol_c", "2026-08-14T00:00:03Z"),
        )
        .expect("append after existing");

        let content = std::fs::read_to_string(audit_file_path(&paths)).expect("read audit");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 3);

        let third: serde_json::Value =
            serde_json::from_str(lines[2]).expect("third line valid JSON");
        assert_eq!(
            third["sequence"].as_u64(),
            Some(3),
            "sequence must continue from existing file"
        );
    }

    #[test]
    fn corrupt_existing_line_returns_err_without_panic() {
        let (_tmp, paths, attempt) = audit_fixture();

        let corrupt = "{ this is not valid json\n";
        seed_audit_file(&paths, corrupt);

        let result = append_evidence_audit(
            &paths,
            &attempt,
            &record("symbol_a", "2026-08-14T00:00:01Z"),
        );
        assert!(
            matches!(result, Err(EvidenceError::Io { .. })),
            "corrupt line must yield Err, got {result:?}"
        );

        let content = std::fs::read_to_string(audit_file_path(&paths)).expect("read audit");
        assert_eq!(content, corrupt, "corrupt file must remain untouched");
    }

    #[test]
    fn concurrent_appends_produce_expected_line_count() {
        let (_tmp, paths, attempt) = audit_fixture();
        let paths = Arc::new(paths);
        let attempt = Arc::new(attempt);

        let mut handles = Vec::new();
        for i in 0..5 {
            let paths = Arc::clone(&paths);
            let attempt = Arc::clone(&attempt);
            handles.push(thread::spawn(move || {
                append_evidence_audit(
                    &paths,
                    &attempt,
                    &record(&format!("symbol_{i}"), "2026-08-14T00:00:01Z"),
                )
                .expect("concurrent append accepted")
            }));
        }

        for handle in handles {
            handle.join().expect("thread join");
        }

        let content = std::fs::read_to_string(audit_file_path(&paths)).expect("read audit");
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 5, "5 concurrent appends must yield 5 lines");

        let mut sequences: Vec<u64> = lines
            .iter()
            .map(|line| {
                let value: serde_json::Value =
                    serde_json::from_str(line).expect("each line valid JSON");
                value["sequence"].as_u64().expect("sequence u64")
            })
            .collect();
        sequences.sort_unstable();
        assert_eq!(sequences, vec![1, 2, 3, 4, 5]);
    }
}
