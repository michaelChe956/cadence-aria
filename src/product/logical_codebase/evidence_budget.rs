// C-4 跨仓只读证据中介：attempt 级 token 预算 ledger（Task 4）。
//
// 契约 REQ-COD-05「token 预算」；设计 §4.4。每个 attempt 落一个
// `evidence-budget.json`（attempt 分区，仿 `coding_attempt_store::attempt_dir` 模式）：
// 字段 `attempt_id` / `cumulative_chars` / `queries` 摘要数组（time/query/result_chars/
// remaining，serde snake_case）。单 web 进程假设下用进程内 `Mutex` 串行化读写
// （仿 `role_run_event.rs` 的 `ROLE_RUN_EVENT_LOG_MUTEX` 先例）+ `json_store::write_json`
// 原子写（temp + rename）。
//
// 语义：首次消费从盘读取（缺文件则视为 `cumulative_chars = 0` 初始化）；本次消费
// 累计后 ≤ `EVIDENCE_ATTEMPT_CHAR_QUOTA`（恰等也算）→ `Accepted{remaining}` 并追加一条
// queries 摘要；超限 → `Exhausted` 且不写入本次消费（ledger 保持原值）。

use std::path::PathBuf;
use std::sync::Mutex;

use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::product::app_paths::ProductAppPaths;
use crate::product::coding_models::CodingExecutionAttempt;
use crate::product::json_store::{read_json, write_json};
use crate::product::logical_codebase::evidence_index::EvidenceError;

/// 单次查询返回文本的字符上限（超出尾部截断）。本 Task 定义，T6 截断使用。
pub const EVIDENCE_QUERY_RESULT_CHAR_LIMIT: usize = 12_000;

/// 单个 attempt 累计证据字符配额（设计 §4.4 Q4=A）。
pub const EVIDENCE_ATTEMPT_CHAR_QUOTA: usize = 120_000;

/// attempt 分区预算 ledger 文件名。
const EVIDENCE_BUDGET_FILE: &str = "evidence-budget.json";

/// 进程内互斥：单 web 进程假设下的写者互斥（仿 `ROLE_RUN_EVENT_LOG_MUTEX` 先例）。
static EVIDENCE_BUDGET_MUTEX: Mutex<()> = Mutex::new(());

/// 单条已接受查询的摘要（对应 ledger `queries` 数组元素）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvidenceBudgetQueryEntry {
    pub time: String,
    pub query: String,
    pub result_chars: usize,
    pub remaining: usize,
}

/// attempt 分区预算 ledger 的落盘记录。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct EvidenceBudgetRecord {
    pub attempt_id: String,
    pub cumulative_chars: usize,
    pub queries: Vec<EvidenceBudgetQueryEntry>,
}

/// 一次预算消费的结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetOutcome {
    /// 本次消费计入累计预算，`remaining` 为消费后剩余配额。
    Accepted { remaining: usize },
    /// 累计超配额，本次消费被拒绝且不落盘。
    Exhausted,
}

/// attempt 级 token 预算 ledger（消费查询结果字符数并原子落盘）。
pub struct EvidenceBudgetLedger {
    paths: ProductAppPaths,
}

impl EvidenceBudgetLedger {
    pub fn new(paths: ProductAppPaths) -> Self {
        Self { paths }
    }

    /// 消费 `result_chars` 个结果字符。返回 `Accepted`（含剩余配额）或 `Exhausted`
    /// （累计超配额，ledger 保持原值不写入本次消费）。
    pub fn consume(
        &self,
        attempt: &CodingExecutionAttempt,
        query: &str,
        result_chars: usize,
    ) -> Result<BudgetOutcome, EvidenceError> {
        let path = self.budget_path(attempt);
        let _guard = EVIDENCE_BUDGET_MUTEX
            .lock()
            .map_err(|error| EvidenceError::Io {
                message: format!("lock evidence budget ledger: {error}"),
            })?;

        let mut record = self.load_or_init(attempt, &path)?;
        let new_cumulative = record.cumulative_chars.saturating_add(result_chars);
        if new_cumulative > EVIDENCE_ATTEMPT_CHAR_QUOTA {
            return Ok(BudgetOutcome::Exhausted);
        }

        let remaining = EVIDENCE_ATTEMPT_CHAR_QUOTA - new_cumulative;
        record.cumulative_chars = new_cumulative;
        record.queries.push(EvidenceBudgetQueryEntry {
            time: Utc::now().to_rfc3339(),
            query: query.to_string(),
            result_chars,
            remaining,
        });
        write_json(&path, &record).map_err(|error| EvidenceError::Io {
            message: format!("write evidence budget ledger {}: {error}", path.display()),
        })?;

        Ok(BudgetOutcome::Accepted { remaining })
    }

    /// 读取 ledger（缺文件时按 `cumulative_chars = 0` 初始化）。
    fn load_or_init(
        &self,
        attempt: &CodingExecutionAttempt,
        path: &std::path::Path,
    ) -> Result<EvidenceBudgetRecord, EvidenceError> {
        if path.exists() {
            return read_json(path).map_err(|error| EvidenceError::Io {
                message: format!("read evidence budget ledger {}: {error}", path.display()),
            });
        }
        Ok(EvidenceBudgetRecord {
            attempt_id: attempt.id.clone(),
            cumulative_chars: 0,
            queries: Vec::new(),
        })
    }

    /// attempt 分区预算 ledger 路径：
    /// `issue_lifecycle_root/{project}/{issue}/coding-attempts/{attempt_id}/evidence-budget.json`
    /// （与 `coding_attempt_store::attempt_dir` 模式一致）。
    fn budget_path(&self, attempt: &CodingExecutionAttempt) -> PathBuf {
        self.paths
            .issue_lifecycle_root(&attempt.project_id, &attempt.issue_id)
            .join("coding-attempts")
            .join(&attempt.id)
            .join(EVIDENCE_BUDGET_FILE)
    }
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

    /// 构造独立 temp 目录 + ledger + attempt fixture。
    fn ledger_fixture() -> (TempDir, EvidenceBudgetLedger, CodingExecutionAttempt) {
        let tmp = TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let ledger = EvidenceBudgetLedger::new(paths);
        let attempt = attempt_fixture();
        (tmp, ledger, attempt)
    }

    fn budget_file_path(paths: &ProductAppPaths) -> PathBuf {
        paths
            .issue_lifecycle_root(PROJECT_ID, ISSUE_ID)
            .join("coding-attempts")
            .join(ATTEMPT_ID)
            .join(EVIDENCE_BUDGET_FILE)
    }

    #[test]
    fn consume_tracks_remaining_up_to_exact_quota() {
        let (_tmp, ledger, attempt) = ledger_fixture();

        assert_eq!(
            ledger.consume(&attempt, "symbol_a", 5_000).unwrap(),
            BudgetOutcome::Accepted { remaining: 115_000 }
        );
        // 累计恰好 120_000 → 最后一次 Accepted（恰等配额也算）。
        assert_eq!(
            ledger.consume(&attempt, "symbol_b", 115_000).unwrap(),
            BudgetOutcome::Accepted { remaining: 0 }
        );
    }

    #[test]
    fn exceed_by_one_is_exhausted_and_ledger_unchanged() {
        let (_tmp, ledger, attempt) = ledger_fixture();

        assert_eq!(
            ledger.consume(&attempt, "symbol_a", 120_000).unwrap(),
            BudgetOutcome::Accepted { remaining: 0 }
        );
        // 超出 1 字符 → Exhausted。
        assert_eq!(
            ledger.consume(&attempt, "symbol_b", 1).unwrap(),
            BudgetOutcome::Exhausted
        );

        // ledger 保持原值：cumulative_chars 仍 120_000，queries 仍 1 条。
        let record: EvidenceBudgetRecord = read_json(&budget_file_path(&ledger.paths)).unwrap();
        assert_eq!(record.cumulative_chars, 120_000);
        assert_eq!(record.queries.len(), 1);
        assert_eq!(record.queries[0].query, "symbol_a");
    }

    #[test]
    fn restart_restores_cumulative_from_disk() {
        let tmp = TempDir::new().expect("tempdir");
        let paths = ProductAppPaths::new(tmp.path().join(".aria"));
        let attempt = attempt_fixture();

        {
            let ledger = EvidenceBudgetLedger::new(paths.clone());
            assert_eq!(
                ledger.consume(&attempt, "symbol_a", 30_000).unwrap(),
                BudgetOutcome::Accepted { remaining: 90_000 }
            );
        } // drop 第一个实例，模拟进程重启。

        let restarted = EvidenceBudgetLedger::new(paths.clone());
        assert_eq!(
            restarted.consume(&attempt, "symbol_b", 10_000).unwrap(),
            BudgetOutcome::Accepted { remaining: 80_000 }
        );

        let record: EvidenceBudgetRecord = read_json(&budget_file_path(&paths)).unwrap();
        assert_eq!(record.attempt_id, ATTEMPT_ID);
        assert_eq!(record.cumulative_chars, 40_000);
        assert_eq!(record.queries.len(), 2);
    }

    #[test]
    fn concurrent_consumes_sum_to_total_without_loss() {
        let (_tmp, ledger, attempt) = ledger_fixture();
        let ledger = Arc::new(ledger);
        let mut handles = Vec::new();
        for i in 0..10 {
            let ledger = Arc::clone(&ledger);
            let attempt = attempt.clone();
            handles.push(thread::spawn(move || {
                ledger
                    .consume(&attempt, &format!("symbol_{i}"), 1_000)
                    .expect("concurrent consume accepted")
            }));
        }

        let outcomes: Vec<BudgetOutcome> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        for outcome in &outcomes {
            assert!(
                matches!(outcome, BudgetOutcome::Accepted { .. }),
                "every concurrent consume must be accepted, got {outcome:?}"
            );
        }

        // 总和一致：10 × 1_000 = 10_000，无丢失。
        let record: EvidenceBudgetRecord = read_json(&budget_file_path(&ledger.paths)).unwrap();
        assert_eq!(record.cumulative_chars, 10_000);
        assert_eq!(record.queries.len(), 10);
    }
}
