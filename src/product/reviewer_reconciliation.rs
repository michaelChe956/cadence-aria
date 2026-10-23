//! 批2 Task 18:三域对账与人工门采信判定(纯判定面 + durable 事实装载)。
//!
//! 定位:恢复成功 ≠ 可采信。本模块消费 T16/T17 已固化的 durable 事实
//! (窗口记录/违规 journal/baseline manifest),产出**三域独立结果**与
//! 采信建议;不写任何存储——落盘动作归 `reviewer_window` 状态机
//! (journal-first)。交付/approve 消费者(T19+)必须消费本模块/窗口
//! 记录的 T18 同源结论,不得只看模型 verdict。
//!
//! 三域:
//! - **worktree 恢复域**:T16 差异恢复完整;临时构建残留(extras/
//!   fixed entries)可恢复,不构成违规。
//! - **Git 观察域**:HEAD/index/refs/objects/reflog/config/子模块
//!   gitdir 恢复至观察基线;窗口内新增 ref(自提交对象)与任何被恢复
//!   回写的 Git 域 mutation 均为违规信号(通道冻结期间不存在合法
//!   Git mutation)。
//! - **平台合法写入域**:报告/role-run/窗口/journal 精确 allowlist
//!   (`platform_write_paths`+归属)内且仅预期变化;allowlist 外平台
//!   写入(含直写 `.aria`)由观察方落违规 journal,本域消费判定。
//!
//! 判定规则(brief 逐字):
//! - 仅**完整恢复 + 三域通过 + 记录一致 + 无违规**才 `Accepted`;
//! - 自提交/改 Git/平台元数据/通道绕过/可观察 journal 篡改 → 整轮
//!   `Invalidated`(恢复成功也不变,人工门收口);
//! - 恢复未完成/瞬时错误(T16 ⚠2:`last_error`+游标带出)→ `NotReady`
//!   停等(保持 `RecoveryRequired` 与锁,拒绝补偿不释放);
//! - 违规已观测但 journal 未落盘(`violation_pending`)→ `Invalidated`
//!   fail-closed(journal 落盘前不得采信);
//! - 外部 push/网络副作用明示不可回滚——不伪造「交付零外部影响」。
//!
//! 篡改检测边界(R8):本模块仅对**可观察范围**(journal 序号连续性/
//! 窗号一致性/同源指纹)fail-closed;不承诺同 UID 恶意重写整文件必被
//! 检测。

use crate::product::reviewer_snapshot::{
    ReviewerBaselineManifest, ReviewerSnapshotError, ReviewerSnapshotStore,
};
use crate::product::reviewer_window::manifest_fingerprint;
use crate::product::reviewer_window_store::{
    ReportAcceptance, ReviewerWindowRecord, ReviewerWindowState, ReviewerWindowStore,
    ViolationJournalEntry,
};

// ---------------------------------------------------------------------------
// 事实与结果
// ---------------------------------------------------------------------------

/// baseline manifest 的装载结局(区分「不存在」与「读不了」:
/// 前者配合已有 baseline 绑定构成篡改信号,后者 fail-closed 停等)。
#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ManifestFacts {
    Loaded(ReviewerBaselineManifest),
    Missing,
    Unreadable(String),
}

/// 三域对账的输入事实(全部 durable 来源;纯函数判定可测)。
pub(crate) struct ReconciliationFacts {
    pub(crate) record: ReviewerWindowRecord,
    pub(crate) journal: Vec<ViolationJournalEntry>,
    pub(crate) manifest: ManifestFacts,
}

/// 单域独立结果。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DomainResult {
    pub(crate) name: &'static str,
    pub(crate) passed: bool,
    pub(crate) reasons: Vec<String>,
}

/// 采信建议(终局语义见各变体文档)。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum AcceptanceRecommendation {
    /// 仅完整恢复+三域通过+记录一致+无违规。调用方仍须经
    /// `record_report_acceptance(Accepted)` + `close_accepted` 落盘。
    Accepted { window_id: String },
    /// 整轮作废(恢复成功也不变):违规/自提交/改 Git/平台元数据/
    /// 通道绕过/可观察篡改/洗白形态。`manual_gate` 恒真——收口必须经
    /// 人工门(作废关闭/作废后新轮),旧报告永久 Invalidated。
    Invalidated {
        window_id: String,
        manual_gate: bool,
        reasons: Vec<String>,
    },
    /// 停等(非终局):恢复未开始/未完成/瞬时错误(T16 ⚠2 游标带出)/
    /// manifest 读不了。窗口保持 `RecoveryRequired` 与锁。
    NotReady {
        window_id: String,
        blockers: Vec<String>,
    },
}

/// 三域对账输出。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReconciliationOutcome {
    pub(crate) worktree: DomainResult,
    pub(crate) git: DomainResult,
    pub(crate) platform: DomainResult,
    /// 记录一致性(baseline 指纹同源核验/身份对齐/平台 allowlist 对齐/
    /// journal 与违规绑定对齐)。
    pub(crate) records_consistent: bool,
    pub(crate) consistency_reasons: Vec<String>,
    pub(crate) recommendation: AcceptanceRecommendation,
}

// ---------------------------------------------------------------------------
// 纯判定
// ---------------------------------------------------------------------------

/// 三域对账纯函数(不触存储)。规则见模块文档;终态窗口按已定格的
/// `report_acceptance` 投影(ClosedAccepted→Accepted,其余终态→
/// Invalidated),重复对账结果稳定。
pub(crate) fn reconcile(facts: &ReconciliationFacts) -> ReconciliationOutcome {
    let record = &facts.record;
    let binding = record.restore.as_ref();
    let mut invalidated_reasons: Vec<String> = Vec::new();
    let mut blockers: Vec<String> = Vec::new();
    let mut consistency_reasons: Vec<String> = Vec::new();

    // -- 终态:已定格,按**实际绑定事实**投影(幂等;F5 修复轮 1:不再
    // 恒 false);ClosedAccepted 遇非空 journal/pending → 倒填防线,
    // fail-closed 投影 Invalidated。 ------------------------------
    if record.state.is_terminal() {
        let binding_ok = binding.is_some_and(|binding| binding.completed);
        let git_clean = binding.is_some_and(|binding| {
            binding.git_restored_mutation_paths.is_empty()
                && binding.git_extra_refs_observed.is_empty()
        });
        let git_reasons: Vec<String> = binding
            .map(|binding| {
                binding
                    .git_restored_mutation_paths
                    .iter()
                    .map(|path| format!("Git 域 mutation: {path}"))
                    .chain(
                        binding
                            .git_extra_refs_observed
                            .iter()
                            .map(|r| format!("新增 ref: {r}")),
                    )
                    .collect()
            })
            .unwrap_or_default();
        let platform_reasons: Vec<String> = facts
            .journal
            .iter()
            .filter(|entry| entry.kind.starts_with("platform_"))
            .map(|entry| format!("平台合法写入域违规: {}", entry.detail))
            .collect();
        let journal_dirty = !facts.journal.is_empty() || record.violation_pending.is_some();
        let recommendation = if record.state == ReviewerWindowState::ClosedAccepted
            && record.report_acceptance == ReportAcceptance::Accepted
            && !journal_dirty
        {
            AcceptanceRecommendation::Accepted {
                window_id: record.window_id.clone(),
            }
        } else {
            let mut reasons = vec![format!("终态 {} 已定格(旧报告永久)", record.state.as_str())];
            if journal_dirty {
                reasons.push(
                    "终态下违规 journal/pending 非空(倒填防线,fail-closed 投影作废)".to_string(),
                );
            }
            AcceptanceRecommendation::Invalidated {
                window_id: record.window_id.clone(),
                manual_gate: false,
                reasons,
            }
        };
        return ReconciliationOutcome {
            worktree: domain("worktree", binding_ok, Vec::new()),
            git: domain(
                "git",
                binding_ok && git_clean && git_reasons.is_empty(),
                git_reasons,
            ),
            platform: domain(
                "platform",
                binding_ok && platform_reasons.is_empty(),
                platform_reasons,
            ),
            records_consistent: true,
            consistency_reasons: Vec::new(),
            recommendation,
        };
    }

    // -- 违规停等标记:journal 写失败 → fail-closed 作废 ------------
    if let Some(pending) = record.violation_pending.as_ref() {
        invalidated_reasons.push(format!(
            "违规已观测但 journal 未落盘(fail-closed 作废): kind={} entry={}",
            pending.kind, pending.entry_id
        ));
    }

    // -- journal 完整性(可观察范围 fail-closed) --------------------
    let journal_ok = journal_integrity_ok(&facts.journal, &record.window_id);
    if !journal_ok {
        invalidated_reasons.push(format!(
            "违规 journal 序号断裂或窗号不符(观察到 {} 条)",
            facts.journal.len()
        ));
    }
    let from_violation = binding.is_some_and(|binding| binding.from_violation)
        || record.state == ReviewerWindowState::ViolationRecorded;
    if from_violation && facts.journal.is_empty() {
        invalidated_reasons
            .push("from_violation 为真但 journal 为空(可观察缺失,篡改面 fail-closed)".to_string());
    }
    if !from_violation && !facts.journal.is_empty() && record.violation_pending.is_none() {
        invalidated_reasons.push(format!(
            "journal 非空({} 条)但绑定未标违规(洗白形态,整轮作废)",
            facts.journal.len()
        ));
    }
    for entry in &facts.journal {
        invalidated_reasons.push(format!(
            "违规[{}#{}] kind={} path={} cursor={}: {}",
            entry.window_id, entry.seq, entry.kind, entry.path, entry.restore_cursor, entry.detail
        ));
        // 外部副作用明示:push 通道违规不可回滚。
        if entry.kind.contains("push") {
            invalidated_reasons
                .push("外部 push/网络副作用不可回滚——不伪称交付零外部影响".to_string());
        }
    }

    // -- 恢复完成度 --------------------------------------------------
    let completed = binding.is_some_and(|binding| binding.completed);
    let outcome = binding
        .map(|binding| binding.outcome.as_str())
        .unwrap_or("");
    match binding {
        None => blockers.push("恢复未开始(restore 绑定缺失)".to_string()),
        Some(binding) if !binding.completed => match binding.outcome.as_str() {
            "in_progress" => blockers.push("恢复进行中".to_string()),
            "recovery_required" => blockers.push(format!(
                "T16 恢复停等: {} (cursor={}:{})",
                binding.detail,
                binding.cursor_phase.as_deref().unwrap_or("-"),
                binding
                    .entry_index
                    .map(|index| index.to_string())
                    .unwrap_or_else(|| "-".to_string())
            )),
            "error" => blockers.push(format!(
                // T16 ⚠2:条目级 Io Err 计入非通过,last_error+游标带出。
                "恢复瞬时错误(计入非通过): {} (last_error={:?} cursor={}:{})",
                binding.detail,
                record.last_error,
                binding.cursor_phase.as_deref().unwrap_or("-"),
                binding
                    .entry_index
                    .map(|index| index.to_string())
                    .unwrap_or_else(|| "-".to_string())
            )),
            other => blockers.push(format!("恢复结局未知: {other}")),
        },
        _ => {}
    }
    if record.last_error.is_some() && completed {
        blockers.push(format!(
            "last_error 残留(计入非通过): {:?}",
            record.last_error
        ));
    }

    // -- 记录一致性 ----------------------------------------------------
    let mut consistent = true;
    match &facts.manifest {
        ManifestFacts::Loaded(manifest) => {
            if let Some(baseline) = record.baseline.as_ref() {
                // 审查硬约束 3:同源规范化 JSON 内容指纹,不按文件字节复算。
                let fingerprint = manifest_fingerprint(manifest);
                if fingerprint != baseline.manifest_sha256 {
                    consistent = false;
                    consistency_reasons.push("baseline 指纹不一致(同源核验失败)".to_string());
                }
            } else {
                consistent = false;
                consistency_reasons.push("记录缺 baseline 绑定".to_string());
            }
            if manifest.window_id != record.window_id
                || manifest.attempt_id != record.attempt_id
                || manifest.run_id != record.run_id
            {
                consistent = false;
                consistency_reasons.push("manifest 与窗口记录身份不对齐".to_string());
            }
            // 平台 allowlist 对齐:预枚举路径+归属必须与基线一致。
            let record_paths = platform_allowlist(record);
            let manifest_paths = platform_allowlist_of_manifest(manifest);
            if record_paths != manifest_paths {
                consistent = false;
                consistency_reasons.push(format!(
                    "平台合法写入域 allowlist 不一致: 记录{:?} vs 基线{:?}",
                    record_paths, manifest_paths
                ));
            }
        }
        ManifestFacts::Missing => {
            if record.baseline.is_some() {
                consistent = false;
                consistency_reasons
                    .push("manifest 缺失但 baseline 绑定存在(证据链断裂,fail-closed)".to_string());
            }
        }
        ManifestFacts::Unreadable(detail) => {
            blockers.push(format!("manifest 不可读(停等,不采信不抹数据): {detail}"));
        }
    }

    // -- Git 域事实(自提交/改 Git → 整轮作废) -----------------------
    let git_mutations = binding
        .map(|binding| binding.git_restored_mutation_paths.clone())
        .unwrap_or_default();
    let extra_refs = binding
        .map(|binding| binding.git_extra_refs_observed.clone())
        .unwrap_or_default();
    for path in &git_mutations {
        invalidated_reasons.push(format!("改 Git(冻结窗口内漂移,已恢复也不洗白): {path}"));
    }
    for extra in &extra_refs {
        invalidated_reasons.push(format!(
            "Reviewer 自提交对象证据(窗口内新增 ref,保留取证): {extra}"
        ));
    }

    // -- 三域独立结果 --------------------------------------------------
    let worktree_reasons: Vec<String> = facts
        .journal
        .iter()
        .filter(|entry| entry.kind.starts_with("worktree_"))
        .map(|entry| format!("worktree 域违规: {}", entry.detail))
        .collect();
    let worktree = domain(
        "worktree",
        completed && worktree_reasons.is_empty(),
        worktree_reasons,
    );
    let git_reasons: Vec<String> = git_mutations
        .iter()
        .map(|path| format!("Git 域 mutation: {path}"))
        .chain(extra_refs.iter().map(|r| format!("新增 ref: {r}")))
        .chain(
            facts
                .journal
                .iter()
                .filter(|entry| entry.kind.starts_with("git_"))
                .map(|entry| format!("Git 域违规: {}", entry.detail)),
        )
        .collect();
    let git = domain("git", completed && git_reasons.is_empty(), git_reasons);
    let platform_reasons: Vec<String> = facts
        .journal
        .iter()
        .filter(|entry| entry.kind.starts_with("platform_"))
        .map(|entry| format!("平台合法写入域违规: {}", entry.detail))
        .collect();
    let platform_verified = binding
        .map(|binding| {
            !binding.platform_verified.is_empty() || record.platform_write_paths.is_empty()
        })
        .unwrap_or(false);
    let platform = domain(
        "platform",
        completed && platform_reasons.is_empty() && platform_verified,
        platform_reasons,
    );

    // -- 终局 ----------------------------------------------------------
    let recommendation = if !invalidated_reasons.is_empty() {
        AcceptanceRecommendation::Invalidated {
            window_id: record.window_id.clone(),
            manual_gate: true,
            reasons: invalidated_reasons,
        }
    } else if !blockers.is_empty() || !completed {
        AcceptanceRecommendation::NotReady {
            window_id: record.window_id.clone(),
            blockers: blockers
                .into_iter()
                .chain(std::iter::once(format!("恢复未完整(outcome={outcome})")))
                .collect(),
        }
    } else if !consistent || !worktree.passed || !git.passed || !platform.passed {
        AcceptanceRecommendation::Invalidated {
            window_id: record.window_id.clone(),
            manual_gate: true,
            reasons: consistency_reasons
                .clone()
                .into_iter()
                .chain(
                    worktree
                        .reasons
                        .iter()
                        .chain(git.reasons.iter())
                        .chain(platform.reasons.iter())
                        .cloned(),
                )
                .collect(),
        }
    } else {
        AcceptanceRecommendation::Accepted {
            window_id: record.window_id.clone(),
        }
    };

    ReconciliationOutcome {
        worktree,
        git,
        platform,
        records_consistent: consistent,
        consistency_reasons,
        recommendation,
    }
}

fn domain(name: &'static str, passed: bool, reasons: Vec<String>) -> DomainResult {
    DomainResult {
        name,
        passed,
        reasons,
    }
}

/// journal 序号从 1 连续且窗号全部相符(与 reviewer_window 同规则)。
fn journal_integrity_ok(entries: &[ViolationJournalEntry], window_id: &str) -> bool {
    entries
        .iter()
        .enumerate()
        .all(|(index, entry)| entry.seq == (index + 1) as u64 && entry.window_id == window_id)
}

fn platform_allowlist(record: &ReviewerWindowRecord) -> Vec<(String, String)> {
    let mut paths: Vec<(String, String)> = record
        .platform_write_paths
        .iter()
        .map(|spec| {
            (
                spec.path.to_string_lossy().into_owned(),
                spec.ownership.clone(),
            )
        })
        .collect();
    paths.sort();
    paths
}

fn platform_allowlist_of_manifest(manifest: &ReviewerBaselineManifest) -> Vec<(String, String)> {
    let mut paths: Vec<(String, String)> = manifest
        .platform_write_baseline
        .iter()
        .map(|entry| (entry.path.clone(), entry.ownership.clone()))
        .collect();
    paths.sort();
    paths
}

// ---------------------------------------------------------------------------
// durable 事实装载(T19+ 消费入口:交付/approve 必查此处,而非模型 verdict)
// ---------------------------------------------------------------------------

/// 从 durable store 装载三域事实并判定。manifest 读失败(非缺失)→
/// `NotReady` 停等(fail-closed);窗口记录/journal 读失败 → Err。
pub(crate) fn reconcile_from_stores(
    window_store: &ReviewerWindowStore,
    snapshot_store: &ReviewerSnapshotStore,
    window_id: &str,
) -> Result<ReconciliationOutcome, ReconciliationError> {
    let record = window_store
        .load_window(window_id)
        .map_err(ReconciliationError::WindowStore)?;
    let journal = window_store
        .load_violations(window_id)
        .map_err(ReconciliationError::WindowStore)?;
    let manifest = match snapshot_store.load_manifest(window_id) {
        Ok(manifest) => ManifestFacts::Loaded(manifest),
        Err(ReviewerSnapshotError::Store(
            crate::product::json_store::ProductStoreError::NotFound { .. },
        )) => ManifestFacts::Missing,
        Err(error) => ManifestFacts::Unreadable(error.to_string()),
    };
    Ok(reconcile(&ReconciliationFacts {
        record,
        journal,
        manifest,
    }))
}

#[derive(Debug, thiserror::Error)]
pub(crate) enum ReconciliationError {
    #[error("reviewer_reconciliation_window_store: {0}")]
    WindowStore(crate::product::json_store::ProductStoreError),
}

// ---------------------------------------------------------------------------
// 内测
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cross_cutting::provider_resources::{
        ProviderRunResourceScope, ResourceScopeKey, new_boot_identity,
    };
    use crate::product::coding_models::ReviewReportAcceptance;
    use crate::product::reviewer_snapshot::{
        PlatformWritePath, REVIEWER_BASELINE_SCHEMA, RestoreCursor, RestorePhase,
        ReviewWorktreeLockIdentity, ReviewerSnapshotStore, SnapshotBudget,
    };
    use crate::product::reviewer_window::{
        ViolationRecord, begin_quiesce, begin_review_window, close_accepted,
        record_report_acceptance, record_violation, resolve_manual_gate, restore_window,
    };
    use crate::product::reviewer_window_store::RestoreBinding;
    use crate::product::reviewer_window_store::{
        BaselineBinding, ReportAcceptance, ReviewerWindowStore, ViolationJournalEntry,
        wire_acceptance,
    };
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    // ---------- fixtures ----------

    fn t18_dir() -> tempfile::TempDir {
        let root = Path::new("/tmp/aria-bug45/t18");
        fs::create_dir_all(root).expect("create /tmp/aria-bug45/t18");
        tempfile::tempdir_in(root).expect("t18 fixture dir")
    }

    fn git_ok(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {:?} in {} failed: {}",
            args,
            dir.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn git_sha(dir: &Path, args: &[&str]) -> String {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("spawn git");
        assert!(out.status.success(), "git {args:?} failed");
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    fn sha256_of_file(path: &Path) -> String {
        use sha2::{Digest, Sha256};
        let bytes = fs::read(path).unwrap_or_default();
        hex::encode(Sha256::digest(&bytes))
    }

    struct Fixture {
        worktree: PathBuf,
        window_root: PathBuf,
        snapshot_root: PathBuf,
        base: PathBuf,
        _keep: Vec<tempfile::TempDir>,
    }

    impl Fixture {
        fn store(&self) -> ReviewerWindowStore {
            ReviewerWindowStore::new(&self.window_root)
        }

        fn snapshot(&self) -> ReviewerSnapshotStore {
            ReviewerSnapshotStore::new(&self.snapshot_root)
        }
    }

    fn init_repo(worktree: &Path) {
        fs::write(worktree.join("tracked.txt"), "base\n").unwrap();
        git_ok(worktree, &["init", "-q", "-b", "main"]);
        git_ok(worktree, &["config", "user.email", "t18@aria.invalid"]);
        git_ok(worktree, &["config", "user.name", "t18-fixture"]);
        git_ok(worktree, &["add", "-A"]);
        git_ok(worktree, &["commit", "-qm", "base"]);
    }

    fn fixture() -> Fixture {
        let base = t18_dir();
        let win = t18_dir();
        let snap = t18_dir();
        let worktree = base.path().join("repo");
        fs::create_dir_all(&worktree).unwrap();
        init_repo(&worktree);
        Fixture {
            worktree,
            window_root: win.path().to_path_buf(),
            snapshot_root: snap.path().to_path_buf(),
            base: base.path().to_path_buf(),
            _keep: vec![base, win, snap],
        }
    }

    fn open_scope(base: &Path, scope_id: &str) -> ProviderRunResourceScope {
        let store_root = base.join("scope-store");
        let private = base.join("private").join(scope_id);
        fs::create_dir_all(&private).unwrap();
        let key = ResourceScopeKey {
            scope_id: scope_id.to_string(),
            run_id: format!("run-{scope_id}"),
            boot_identity: new_boot_identity(),
        };
        ProviderRunResourceScope::open_for_recovery(
            &key,
            Some("w-t18"),
            &private,
            &store_root,
            None,
        )
        .expect("open scope")
    }

    fn begin_request<'a>(
        window_id: &'a str,
        attempt: &'a str,
        worktree: &'a Path,
        targets: &'a [&'a ProviderRunResourceScope],
    ) -> crate::product::reviewer_window::ReviewerWindowBeginRequest<'a> {
        crate::product::reviewer_window::ReviewerWindowBeginRequest {
            window_id,
            attempt_id: attempt,
            run_id: "run-1",
            provider: "claude_code",
            worktree,
            quiesce_targets: targets,
            platform_write_paths: Vec::new(),
            budget: SnapshotBudget::generous(),
            drain_timeout: Duration::from_secs(5),
        }
    }

    fn block_on<F: Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    fn minimal_manifest(
        window_id: &str,
    ) -> crate::product::reviewer_snapshot::ReviewerBaselineManifest {
        crate::product::reviewer_snapshot::ReviewerBaselineManifest {
            schema: REVIEWER_BASELINE_SCHEMA.to_string(),
            window_id: window_id.to_string(),
            attempt_id: "attempt-1".to_string(),
            run_id: "run-1".to_string(),
            lock_identity: ReviewWorktreeLockIdentity {
                canonical_worktree: PathBuf::from("/tmp/aria-bug45/t18/repo"),
                gitdir: PathBuf::from("/tmp/aria-bug45/t18/repo/.git"),
                common_dir: PathBuf::from("/tmp/aria-bug45/t18/repo/.git"),
                lease_id: "lease-1".to_string(),
            },
            entries: Vec::new(),
            git_observation: crate::product::reviewer_snapshot::GitObservationBaseline {
                head_symref: Some("refs/heads/main".to_string()),
                head_sha: "0".repeat(40),
                index_blob: "blobs/0".to_string(),
                index_sha256: "0".repeat(64),
                managed_refs: Vec::new(),
                refs_digest: "0".repeat(64),
                index_semantic_sha256: None,
            },
            platform_write_baseline: Vec::new(),
            scope_key: ResourceScopeKey {
                scope_id: "scope".to_string(),
                run_id: "run-1".to_string(),
                boot_identity: "boot".to_string(),
            },
            scope_manifest: Vec::new(),
            quiesced_at: "2026-10-17T00:00:00Z".to_string(),
            captured_at: "2026-10-17T00:00:00Z".to_string(),
            git_annex_observation: None,
            restore_cursor: RestoreCursor {
                phase: RestorePhase::Completed,
                entry_index: 0,
            },
            ignore_rules_freeze:
                crate::product::reviewer_snapshot_domain::IgnoreRulesFreeze::synthetic(),
            domain_boundary: crate::product::reviewer_snapshot_domain::DOMAIN_BOUNDARY_DISCLOSURE
                .to_string(),
        }
    }

    fn facts_record(window_id: &str) -> ReviewerWindowRecord {
        ReviewerWindowRecord::new(
            window_id,
            ReviewWorktreeLockIdentity {
                canonical_worktree: PathBuf::from("/tmp/aria-bug45/t18/repo"),
                gitdir: PathBuf::from("/tmp/aria-bug45/t18/repo/.git"),
                common_dir: PathBuf::from("/tmp/aria-bug45/t18/repo/.git"),
                lease_id: "lease-1".to_string(),
            },
            "attempt-1",
            "run-1",
            "claude_code",
            Vec::new(),
        )
    }

    fn restored_binding() -> RestoreBinding {
        RestoreBinding {
            completed: true,
            from_violation: false,
            outcome: "restored".to_string(),
            cursor_phase: Some("completed".to_string()),
            entry_index: Some(0),
            detail: "ok".to_string(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: vec!["/platform/report.json".to_string()],
        }
    }

    fn baseline_binding(sha: &str) -> BaselineBinding {
        BaselineBinding {
            manifest_path: "windows/w-1/manifest.json".to_string(),
            manifest_sha256: sha.to_string(),
            entry_count: 0,
            total_bytes: 0,
            captured_at: "2026-10-17T00:00:00Z".to_string(),
        }
    }

    fn journal_entry(seq: u64, window_id: &str, kind: &str) -> ViolationJournalEntry {
        ViolationJournalEntry {
            seq,
            entry_id: format!("entry-{seq}"),
            at: "2026-10-17T00:00:00Z".to_string(),
            window_id: window_id.to_string(),
            attempt_id: "attempt-1".to_string(),
            run_id: "run-1".to_string(),
            report_id: Some("code_review_report_0001".to_string()),
            kind: kind.to_string(),
            path: ".aria/direct.json".to_string(),
            before_digest: None,
            after_digest: Some("sha256:ff".to_string()),
            detail: "直写".to_string(),
            restore_cursor: "started+0".to_string(),
        }
    }

    // ---------- 纯判定矩阵 ----------

    #[test]
    fn pure_clean_completed_facts_recommend_accepted() {
        let manifest = minimal_manifest("w-1");
        let mut record = facts_record("w-1");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        record.baseline = Some(baseline_binding(&manifest_fingerprint(&manifest)));
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Loaded(manifest),
        });
        assert!(outcome.worktree.passed);
        assert!(outcome.git.passed);
        assert!(outcome.platform.passed);
        assert!(outcome.records_consistent);
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));
    }

    #[test]
    fn pure_restore_error_binding_not_ready_with_cursor() {
        // T16 ⚠2:条目级 Io Err 计入非通过,last_error+游标带出。
        let mut record = facts_record("w-err");
        record.state = ReviewerWindowState::Restoring;
        record.last_error = Some("Io: read blob".to_string());
        record.restore = Some(RestoreBinding {
            completed: false,
            from_violation: false,
            outcome: "error".to_string(),
            cursor_phase: Some("worktree_restore".to_string()),
            entry_index: Some(3),
            detail: "Io: read blob".to_string(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::NotReady { blockers, .. } => {
                assert!(
                    blockers
                        .iter()
                        .any(|blocker| blocker.contains("worktree_restore")),
                    "blockers 须带出游标: {blockers:?}"
                );
            }
            other => panic!("期望 NotReady,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_git_mutation_and_extra_refs_invalidate_round() {
        let mut record = facts_record("w-git");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(RestoreBinding {
            completed: true,
            from_violation: false,
            outcome: "restored".to_string(),
            cursor_phase: Some("completed".to_string()),
            entry_index: Some(0),
            detail: "ok".to_string(),
            git_restored_mutation_paths: vec![".git/HEAD".to_string()],
            git_extra_refs_observed: vec!["refs/heads/evil".to_string()],
            platform_verified: Vec::new(),
        });
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        assert!(!outcome.git.passed, "Git 观察域必须不通过");
        assert!(outcome.worktree.passed, "worktree 域独立通过");
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated {
                manual_gate,
                reasons,
                ..
            } => {
                assert!(manual_gate);
                assert!(reasons.iter().any(|reason| reason.contains("自提交")));
                assert!(reasons.iter().any(|reason| reason.contains("改 Git")));
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_journal_seq_gap_tamper_fail_closed() {
        let mut record = facts_record("w-gap");
        record.state = ReviewerWindowState::ManualGate;
        record.restore = Some(RestoreBinding {
            completed: true,
            from_violation: true,
            outcome: "restored".to_string(),
            cursor_phase: Some("completed".to_string()),
            entry_index: Some(0),
            detail: "ok".to_string(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        // seq 断裂(1,3):可观察篡改 → fail-closed 作废。
        let journal = vec![
            journal_entry(1, "w-gap", "platform_domain_write"),
            journal_entry(3, "w-gap", "platform_domain_write"),
        ];
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal,
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(reasons.iter().any(|reason| reason.contains("序号断裂")));
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_from_violation_with_empty_journal_fail_closed() {
        let mut record = facts_record("w-miss");
        record.state = ReviewerWindowState::ManualGate;
        record.restore = Some(RestoreBinding {
            completed: true,
            from_violation: true,
            outcome: "restored".to_string(),
            cursor_phase: Some("completed".to_string()),
            entry_index: Some(0),
            detail: "ok".to_string(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(reasons.iter().any(|reason| reason.contains("journal 为空")));
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_journal_without_binding_whitewash_fail_closed() {
        let mut record = facts_record("w-white");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: vec![journal_entry(1, "w-white", "channel_bypass")],
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(reasons.iter().any(|reason| reason.contains("洗白")));
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("channel_bypass"))
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_fingerprint_mismatch_invalidated() {
        let manifest = minimal_manifest("w-hash");
        let mut record = facts_record("w-hash");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        // 故意给错指纹:hash 不一致 → 整轮作废(同源核验 fail-closed)。
        record.baseline = Some(baseline_binding(&"f".repeat(64)));
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Loaded(manifest),
        });
        assert!(!outcome.records_consistent);
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(reasons.iter().any(|reason| reason.contains("指纹不一致")));
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_platform_allowlist_mismatch_invalidated() {
        let mut manifest = minimal_manifest("w-plat");
        manifest.platform_write_baseline = vec![
            crate::product::reviewer_snapshot::PlatformWriteBaselineEntry {
                path: "/platform/report.json".to_string(),
                ownership: "report-store".to_string(),
                state: crate::product::reviewer_snapshot::PlatformWriteState::File {
                    content_sha256: "0".repeat(64),
                    mode: 0o644,
                },
            },
        ];
        let mut record = facts_record("w-plat");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        record.baseline = Some(baseline_binding(&manifest_fingerprint(&manifest)));
        // 记录 allowlist 与基线不一致 → 记录不一致 → 作废。
        record.platform_write_paths = vec![PlatformWritePath {
            path: PathBuf::from("/platform/other.json"),
            ownership: "report-store".to_string(),
        }];
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Loaded(manifest),
        });
        assert!(!outcome.records_consistent);
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("allowlist 不一致"))
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_pending_violation_fail_closed_invalidated() {
        let mut record = facts_record("w-pend");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        record.violation_pending = Some(crate::product::reviewer_window_store::ViolationPending {
            entry_id: "e".to_string(),
            kind: "platform_domain_write".to_string(),
            detail: "journal 写失败".to_string(),
        });
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated {
                manual_gate,
                reasons,
                ..
            } => {
                assert!(manual_gate);
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("journal 未落盘"))
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    #[test]
    fn pure_terminal_window_projects_final_acceptance() {
        let mut record = facts_record("w-done");
        record.state = ReviewerWindowState::ClosedAccepted;
        record.report_acceptance = ReportAcceptance::Accepted;
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));

        let mut record = facts_record("w-done2");
        record.state = ReviewerWindowState::ClosedInvalidated;
        record.report_acceptance = ReportAcceptance::Invalidated;
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: Vec::new(),
            manifest: ManifestFacts::Missing,
        });
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Invalidated { .. }
        ));
    }

    // ---------- E2E(store 实链路) ----------

    #[test]
    fn e2e_build_residue_and_coder_dirty_recovers_to_accepted() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-acc", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // Coder dirty(改 tracked)+ 临时构建残留(untracked)。
        fs::write(fx.worktree.join("tracked.txt"), "coder dirty\n").unwrap();
        fs::create_dir_all(fx.worktree.join("target/debug")).unwrap();
        fs::write(fx.worktree.join("target/debug/residue.o"), "junk\n").unwrap();

        let quiescing = begin_quiesce(&store, "w-acc", handle.generation, "review end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-acc",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(!summary.manual_gate);

        // 三域对账:可采信。
        let outcome = reconcile_from_stores(&store, &snapshot, "w-acc").expect("reconcile");
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));

        // 先 Pending(初值)→ 受信裁决 Accepted → 采信关闭。
        assert_eq!(
            summary.record.report_acceptance,
            ReportAcceptance::PendingAcceptance
        );
        let accepted = record_report_acceptance(
            &store,
            &snapshot,
            "w-acc",
            summary.record.generation,
            ReportAcceptance::Accepted,
        )
        .expect("record accepted");
        assert_eq!(
            wire_acceptance(ReportAcceptance::Accepted),
            ReviewReportAcceptance::Accepted
        );
        let terminal = close_accepted(&store, "w-acc", accepted.generation).expect("close");
        assert_eq!(terminal.state, ReviewerWindowState::ClosedAccepted);
        assert!(store.load_violations("w-acc").unwrap().is_empty());
        assert_eq!(terminal.report_acceptance, ReportAcceptance::Accepted);
    }

    #[test]
    fn e2e_reviewer_self_commit_invalidated_even_after_full_restore() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-selfcommit", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let baseline_head = git_sha(&fx.worktree, &["rev-parse", "HEAD"]);

        // Reviewer 自提交:新分支 + commit(HEAD symref/ref/index/新对象)。
        fs::write(fx.worktree.join("reviewer.txt"), "self commit\n").unwrap();
        git_ok(&fx.worktree, &["checkout", "-q", "-b", "evil"]);
        git_ok(&fx.worktree, &["add", "-A"]);
        git_ok(&fx.worktree, &["commit", "-qm", "reviewer self commit"]);

        let quiescing = begin_quiesce(&store, "w-selfcommit", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-selfcommit",
            quiescing.generation,
        ))
        .expect("restore");
        // 恢复成功也不能洗白:自提交对象存在 → 人工门。
        assert!(summary.manual_gate, "自提交必须进入人工门");
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);
        // HEAD 已恢复基线。
        assert_eq!(git_sha(&fx.worktree, &["rev-parse", "HEAD"]), baseline_head);
        let binding = summary.record.restore.as_ref().expect("binding");
        assert!(
            !binding.git_extra_refs_observed.is_empty()
                || !binding.git_restored_mutation_paths.is_empty(),
            "自提交事实必须结构化入绑定: {binding:?}"
        );

        // journal:恢复前事实已落盘且恢复不删除/覆盖。
        let journal = store.load_violations("w-selfcommit").unwrap();
        assert!(!journal.is_empty(), "自提交证据必须入 journal");
        assert!(
            journal
                .iter()
                .any(|entry| entry.kind == "git_extra_ref" || entry.kind == "git_domain_mutation")
        );

        // 三域对账:整轮 Invalidated。
        let outcome = reconcile_from_stores(&store, &snapshot, "w-selfcommit").expect("reconcile");
        match &outcome.recommendation {
            AcceptanceRecommendation::Invalidated { manual_gate, .. } => assert!(manual_gate),
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
        assert!(!outcome.git.passed);

        // 采信关闭被拒;人工作废关闭 → 旧报告永久 Invalidated。
        assert!(close_accepted(&store, "w-selfcommit", summary.record.generation).is_err());
        let terminal = resolve_manual_gate(
            &store,
            "w-selfcommit",
            summary.record.generation,
            crate::product::reviewer_window::ManualGateDecision::Invalidate,
        )
        .expect("manual invalidate");
        assert_eq!(terminal.state, ReviewerWindowState::ClosedInvalidated);
        assert_eq!(terminal.report_acceptance, ReportAcceptance::Invalidated);
        // 恢复后 journal 仍在(不可删除/覆盖)。
        assert!(!store.load_violations("w-selfcommit").unwrap().is_empty());
    }

    #[test]
    fn e2e_legal_platform_report_write_not_flagged() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let platform_dir = fx.base.join("platform");
        fs::create_dir_all(&platform_dir).unwrap();
        let platform_file = platform_dir.join("report.json");
        fs::write(&platform_file, "v0\n").unwrap();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let mut request = begin_request("w-plat-ok", "attempt-1", &fx.worktree, &targets);
        request.platform_write_paths = vec![PlatformWritePath {
            path: platform_file.clone(),
            ownership: "report-store".to_string(),
        }];
        let handle = block_on(begin_review_window(&store, &snapshot, &request)).expect("begin");

        // 合法平台报告写(allowlist 内,平台 actor):内容漂移按 T16 契约
        // 停等(不伪称三域通过),但 T18 不得误判为违规——不落 journal、
        // 对账为 NotReady 而非 Invalidated;平台稳定后重入恢复完成采信。
        fs::write(&platform_file, "v1-legal-report\n").unwrap();
        // 顺带 worktree 脏(可恢复)。
        fs::write(fx.worktree.join("tracked.txt"), "dirty\n").unwrap();

        let quiescing = begin_quiesce(&store, "w-plat-ok", handle.generation, "end").unwrap();
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-plat-ok",
            quiescing.generation,
        ))
        .expect_err("平台域漂移按 T16 停等,不伪称完成");
        assert!(
            matches!(
                &err,
                crate::product::reviewer_window::ReviewerWindowError::RecoveryStopped { reason, .. }
                    if reason.contains("platform")
            ),
            "{err:?}"
        );
        // 不误判:无违规 journal、对账 NotReady(停等)而非 Invalidated。
        assert!(store.load_violations("w-plat-ok").unwrap().is_empty());
        let record = store.load_window("w-plat-ok").unwrap();
        assert_eq!(record.state, ReviewerWindowState::RecoveryRequired);
        let outcome = reconcile_from_stores(&store, &snapshot, "w-plat-ok").expect("reconcile");
        match &outcome.recommendation {
            AcceptanceRecommendation::NotReady { blockers, .. } => {
                assert!(
                    blockers
                        .iter()
                        .any(|blocker| blocker.contains("platform_domain")),
                    "blockers: {blockers:?}"
                );
            }
            other => panic!("合法平台写不得判 Invalidated/Accepted: {other:?}"),
        }

        // 平台侧稳定(归属方落定基线态)→ 恢复重入完成 → 三域通过。
        fs::write(&platform_file, "v0\n").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-plat-ok",
            record.generation,
        ))
        .expect("restore retry");
        assert!(!summary.manual_gate, "合法平台写不得误判为违规");
        let binding = summary.record.restore.as_ref().expect("binding");
        assert!(
            binding
                .platform_verified
                .contains(&platform_file.to_string_lossy().to_string())
        );

        let outcome = reconcile_from_stores(&store, &snapshot, "w-plat-ok").expect("reconcile");
        assert!(outcome.platform.passed, "平台合法写入域必须通过");
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));
        let accepted = record_report_acceptance(
            &store,
            &snapshot,
            "w-plat-ok",
            summary.record.generation,
            ReportAcceptance::Accepted,
        )
        .expect("accepted");
        assert_eq!(
            close_accepted(&store, "w-plat-ok", accepted.generation)
                .expect("close")
                .state,
            ReviewerWindowState::ClosedAccepted
        );
    }

    #[test]
    fn e2e_observed_violations_journal_fields_and_gate_flow() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-viol2", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        let quiescing = begin_quiesce(&store, "w-viol2", handle.generation, "end").unwrap();
        let violated = record_violation(
            &store,
            &snapshot,
            "w-viol2",
            quiescing.generation,
            ViolationRecord {
                kind: "git_domain_write",
                path: ".git/refs/heads/main",
                before_digest: Some("sha256:base"),
                after_digest: Some("sha256:evil"),
                detail: "直接写 .git 受管 ref",
                report_id: Some("code_review_report_0002"),
            },
        )
        .expect("record violation 1");
        assert_eq!(violated.state, ReviewerWindowState::ViolationRecorded);

        // 同一违规重复收口幂等:journal 只有一条。
        let again = record_violation(
            &store,
            &snapshot,
            "w-viol2",
            violated.generation,
            ViolationRecord {
                kind: "git_domain_write",
                path: ".git/refs/heads/main",
                before_digest: Some("sha256:base"),
                after_digest: Some("sha256:evil"),
                detail: "直接写 .git 受管 ref",
                report_id: Some("code_review_report_0002"),
            },
        )
        .expect("record violation idempotent");
        let journal = store.load_violations("w-viol2").unwrap();
        assert_eq!(journal.len(), 1, "同一违规重复收口只落一条: {journal:?}");
        // journal 条目字段完备(brief:路径/前后摘要/时间/window/attempt/
        // run/report/restore_cursor)。
        let entry = &journal[0];
        assert_eq!(entry.seq, 1);
        assert_eq!(entry.window_id, "w-viol2");
        assert_eq!(entry.attempt_id, "attempt-1");
        assert_eq!(entry.run_id, "run-1");
        assert_eq!(entry.report_id.as_deref(), Some("code_review_report_0002"));
        assert_eq!(entry.path, ".git/refs/heads/main");
        assert_eq!(entry.before_digest.as_deref(), Some("sha256:base"));
        assert_eq!(entry.after_digest.as_deref(), Some("sha256:evil"));
        assert!(!entry.at.is_empty());
        assert!(!entry.restore_cursor.is_empty());

        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-viol2",
            again.generation,
        ))
        .expect("restore");
        assert!(summary.manual_gate);
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);

        // 人工门期间:补偿通道被冻结拒绝 + 租约保持。
        let common_dir =
            crate::product::reviewer_window::resolve_worktree_git_identity(&fx.worktree)
                .expect("identity")
                .2;
        let err = store
            .enter_write_channel(
                &common_dir,
                crate::product::reviewer_window_store::WriteChannelKind::Compensation,
                crate::product::reviewer_window_store::ChannelWriter {
                    writer_id: "comp-1".to_string(),
                    boot_identity: "boot".to_string(),
                    pid: std::process::id(),
                    start_time_ticks: 0,
                    entered_at: "2026-10-17T00:00:00Z".to_string(),
                },
            )
            .expect_err("补偿通道必须被冻结拒绝");
        assert!(matches!(
            err,
            crate::product::reviewer_window_store::ChannelGateError::Blocked { .. }
        ));
        let empty = open_scope(&fx.base, "coder-2");
        let empty_targets: [&ProviderRunResourceScope; 1] = [&empty];
        assert!(
            block_on(begin_review_window(
                &store,
                &snapshot,
                &begin_request("w-second", "attempt-2", &fx.worktree, &empty_targets),
            ))
            .is_err()
        );

        // 采信写入被拒(违规流只能人工门)。
        assert!(
            record_report_acceptance(
                &store,
                &snapshot,
                "w-viol2",
                summary.record.generation,
                ReportAcceptance::Accepted,
            )
            .is_err()
        );

        let outcome = reconcile_from_stores(&store, &snapshot, "w-viol2").expect("reconcile");
        match &outcome.recommendation {
            AcceptanceRecommendation::Invalidated {
                manual_gate,
                reasons,
                ..
            } => {
                assert!(manual_gate);
                assert!(
                    reasons
                        .iter()
                        .any(|reason| reason.contains("git_domain_write"))
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }

        let terminal = resolve_manual_gate(
            &store,
            "w-viol2",
            summary.record.generation,
            crate::product::reviewer_window::ManualGateDecision::RequireNewReview,
        )
        .expect("require new review");
        assert_eq!(terminal.state, ReviewerWindowState::NewReviewRequired);
        assert_eq!(terminal.report_acceptance, ReportAcceptance::Invalidated);

        // 新轮:同树可开新窗口,新 baseline/新 PendingAcceptance;
        // 旧窗口记录与 journal 永久保留。
        let new_handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-new2", "attempt-2", &fx.worktree, &empty_targets),
        ))
        .expect("new round window");
        let new_record = store.load_window("w-new2").unwrap();
        assert_eq!(
            new_record.report_acceptance,
            ReportAcceptance::PendingAcceptance
        );
        let old = store.load_window("w-viol2").unwrap();
        assert_ne!(
            new_handle.manifest_sha256,
            old.baseline.as_ref().unwrap().manifest_sha256,
            "新轮不得沿用旧 baseline 指纹"
        );
        assert_ne!(
            new_handle.manifest_path,
            old.baseline.as_ref().unwrap().manifest_path,
            "新轮不得沿用旧 manifest"
        );
        assert_eq!(old.report_acceptance, ReportAcceptance::Invalidated);
        assert!(!store.load_violations("w-viol2").unwrap().is_empty());
    }

    #[test]
    fn e2e_journal_write_failure_blocks_restore_until_durable() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-jfail", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-jfail", handle.generation, "end").unwrap();

        // 注入 journal 写失败:在 journal 文件路径放置目录。
        let journal_path = fx.window_root.join("violations").join("w-jfail.json");
        fs::create_dir_all(&journal_path).unwrap();
        let err = record_violation(
            &store,
            &snapshot,
            "w-jfail",
            quiescing.generation,
            ViolationRecord {
                kind: "platform_domain_write",
                path: ".aria/direct.json",
                before_digest: None,
                after_digest: None,
                detail: "直写 .aria",
                report_id: None,
            },
        )
        .expect_err("journal 写失败必须报错");
        assert!(
            matches!(
                err,
                crate::product::reviewer_window::ReviewerWindowError::Store(_)
            ),
            "{err:?}"
        );

        // 停等标记已持久化;恢复必须不开始;采信关闭被拒。
        let record = store.load_window("w-jfail").unwrap();
        assert!(
            record.violation_pending.is_some(),
            "violation_pending 必须持久化"
        );
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-jfail",
            record.generation,
        ))
        .expect_err("journal 未落盘不得开始恢复");
        assert!(
            matches!(
                err,
                crate::product::reviewer_window::ReviewerWindowError::ViolationJournalPending { .. }
            ),
            "{err:?}"
        );
        assert!(close_accepted(&store, "w-jfail", record.generation).is_err());

        // 恢复 journal 可写后重入:幂等续接 → 违规流正常收口。
        fs::remove_dir(&journal_path).unwrap();
        let violated = record_violation(
            &store,
            &snapshot,
            "w-jfail",
            record.generation,
            ViolationRecord {
                kind: "platform_domain_write",
                path: ".aria/direct.json",
                before_digest: None,
                after_digest: None,
                detail: "直写 .aria",
                report_id: None,
            },
        )
        .expect("retry record violation");
        assert_eq!(violated.state, ReviewerWindowState::ViolationRecorded);
        assert!(violated.violation_pending.is_none());
        assert_eq!(store.load_violations("w-jfail").unwrap().len(), 1);
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-jfail",
            violated.generation,
        ))
        .expect("restore");
        assert!(summary.manual_gate);
    }

    // ---------- 审查修复轮 1(F1-F5)钉测 ----------

    /// F1:派生违规后 journal 写失败/崩溃 → 重入恢复仍观察得到自提交
    /// 事实(extra ref 留仓),不丢失、不误放行 Accepted。
    #[test]
    fn f1_derived_git_facts_survive_journal_failure_and_restore_reentry() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-f1r", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // Reviewer 自提交(新分支+commit,extra ref 留仓)。
        fs::write(fx.worktree.join("r.txt"), "x\n").unwrap();
        git_ok(&fx.worktree, &["checkout", "-q", "-b", "evil"]);
        git_ok(&fx.worktree, &["add", "-A"]);
        git_ok(&fx.worktree, &["commit", "-qm", "self"]);
        let quiescing = begin_quiesce(&store, "w-f1r", handle.generation, "end").unwrap();

        // 注入 journal 写失败:恢复后派生收口落盘失败,停等标记残留。
        let journal_path = fx.window_root.join("violations").join("w-f1r.json");
        fs::create_dir_all(&journal_path).unwrap();
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-f1r",
            quiescing.generation,
        ))
        .expect_err("journal 阻断时恢复收口必须失败");
        assert!(
            matches!(
                &err,
                crate::product::reviewer_window::ReviewerWindowError::Store(_)
            ),
            "{err:?}"
        );
        let record = store.load_window("w-f1r").unwrap();
        assert!(record.violation_pending.is_some());
        assert!(
            !record
                .restore
                .as_ref()
                .is_some_and(|binding| binding.completed),
            "journal 未落盘不得宣称恢复完整待采信"
        );

        // 崩溃重入(修复 journal 可写):独立观察重算自提交事实 → 落
        // journal → ManualGate;即使 T16 游标已推进,事实不丢。
        fs::remove_dir(&journal_path).unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-f1r",
            record.generation,
        ))
        .expect("reentry restore");
        assert!(summary.manual_gate, "重入后自提交事实不得丢失");
        let journal = store.load_violations("w-f1r").unwrap();
        assert!(
            journal.iter().any(|entry| entry.kind == "git_extra_ref"),
            "{journal:?}"
        );
    }

    /// F4:直写 .git/config → annex 补充面观察 → 整轮作废进人工门。
    #[test]
    fn f4_direct_git_config_write_detected_via_annex_observation() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-f4", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // 通道冻结窗口内直写 .git/config(hooks/fsmonitor 类逃逸面)。
        let config_path = fx.worktree.join(".git").join("config");
        let original = fs::read_to_string(&config_path).unwrap();
        fs::write(
            &config_path,
            format!("{original}[core]\n\tfsmonitor = evil\n"),
        )
        .unwrap();

        let quiescing = begin_quiesce(&store, "w-f4", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-f4",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(summary.manual_gate, "直写 .git/config 必须进人工门");
        let journal = store.load_violations("w-f4").unwrap();
        assert!(
            journal
                .iter()
                .any(|entry| entry.kind == "git_annex_drift" && entry.path.contains("config")),
            "{journal:?}"
        );
        // 对账:整轮 Invalidated,Git 域不通过。
        let outcome = reconcile_from_stores(&store, &snapshot, "w-f4").unwrap();
        assert!(!outcome.git.passed);
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Invalidated { .. }
        ));
    }

    /// F2:pending 属他人 entry 时 record_violation fail-closed 拒绝,
    /// 不得覆盖;同 entry 重试可推进并清除。
    #[test]
    fn f2_record_violation_never_overwrites_foreign_pending() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-f2", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-f2", handle.generation, "end").unwrap();

        // 造 pending(A):journal 阻断写入失败但标记已持久化。
        let journal_path = fx.window_root.join("violations").join("w-f2.json");
        fs::create_dir_all(&journal_path).unwrap();
        let violation_a = ViolationRecord {
            kind: "platform_domain_write",
            path: ".aria/a.json",
            before_digest: None,
            after_digest: None,
            detail: "A",
            report_id: None,
        };
        assert!(
            record_violation(&store, &snapshot, "w-f2", quiescing.generation, violation_a).is_err()
        );
        let record = store.load_window("w-f2").unwrap();
        assert!(record.violation_pending.is_some());

        // pending(A) 存在时记 B → ViolationJournalPending 拒绝(A 不被覆盖)。
        let violation_b = ViolationRecord {
            kind: "git_domain_write",
            path: ".git/x",
            before_digest: None,
            after_digest: None,
            detail: "B",
            report_id: None,
        };
        let err = record_violation(&store, &snapshot, "w-f2", record.generation, violation_b)
            .expect_err("不得覆盖他人 pending");
        assert!(
            matches!(
                err,
                crate::product::reviewer_window::ReviewerWindowError::ViolationJournalPending { .. }
            ),
            "{err:?}"
        );
        assert_eq!(
            store
                .load_window("w-f2")
                .unwrap()
                .violation_pending
                .as_ref()
                .unwrap()
                .kind,
            "platform_domain_write"
        );

        // 同 entry(A)重试 → journal 落盘、标记清除、迁移 ViolationRecorded。
        fs::remove_dir(&journal_path).unwrap();
        let violation_a = ViolationRecord {
            kind: "platform_domain_write",
            path: ".aria/a.json",
            before_digest: None,
            after_digest: None,
            detail: "A",
            report_id: None,
        };
        let violated = record_violation(&store, &snapshot, "w-f2", record.generation, violation_a)
            .expect("retry A");
        assert_eq!(violated.state, ReviewerWindowState::ViolationRecorded);
        assert!(violated.violation_pending.is_none());
    }

    /// F3:本窗重入态(pending 残留)下 journal 非空不构成洗白指控。
    #[test]
    fn f3_reentry_pending_journal_not_accused_as_whitewash() {
        let mut record = facts_record("w-f3");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        record.violation_pending = Some(crate::product::reviewer_window_store::ViolationPending {
            entry_id: "half-appended".to_string(),
            kind: "git_extra_ref".to_string(),
            detail: "append 崩溃残留".to_string(),
        });
        // journal 非空(部分落盘的本窗派生条目)+ pending 残留 →
        // reconcile 不产生「洗白」指控(那是 journal_tamper 虚假污染)。
        let outcome = reconcile(&ReconciliationFacts {
            journal: vec![journal_entry(1, "w-f3", "git_extra_ref")],
            record,
            manifest: ManifestFacts::Missing,
        });
        match outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(
                    !reasons.iter().any(|reason| reason.contains("洗白")),
                    "重入态不得误判洗白: {reasons:?}"
                );
            }
            other => panic!("期望 Invalidated(pending fail-closed),得到 {other:?}"),
        }
    }

    /// F5:终态投影按实际绑定;ClosedAccepted 遇非空 journal 倒填作废。
    #[test]
    fn f5_terminal_projection_uses_binding_and_backfill_guard() {
        let manifest = minimal_manifest("w-f5");
        let mut record = facts_record("w-f5");
        record.state = ReviewerWindowState::ClosedAccepted;
        record.report_acceptance = ReportAcceptance::Accepted;
        record.restore = Some(restored_binding());
        record.baseline = Some(baseline_binding(&manifest_fingerprint(&manifest)));
        let outcome = reconcile(&ReconciliationFacts {
            record: record.clone(),
            journal: Vec::new(),
            manifest: ManifestFacts::Loaded(manifest),
        });
        assert!(outcome.git.passed, "终态投影按绑定事实,不再恒 false");
        assert!(outcome.platform.passed);
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Accepted { .. }
        ));

        // 倒填防线:终态 Accepted + 非空 journal → Invalidated fail-closed。
        let outcome = reconcile(&ReconciliationFacts {
            record,
            journal: vec![journal_entry(1, "w-f5", "channel_bypass")],
            manifest: ManifestFacts::Missing,
        });
        assert!(matches!(
            outcome.recommendation,
            AcceptanceRecommendation::Invalidated { .. }
        ));
    }

    // ---------- 审查修复轮 2(P1/P2/P3)钉测 ----------

    /// P1:index-only 直写(回写后不可再观)经平台漂移停等续接后
    /// 仍 ManualGate——恢复前持久化观察兜底,不静默放行。
    #[test]
    fn p1_index_only_drift_survives_platform_recovery_detour() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let platform_dir = fx.base.join("platform");
        fs::create_dir_all(&platform_dir).unwrap();
        let platform_file = platform_dir.join("report.json");
        fs::write(&platform_file, "v0\n").unwrap();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let mut request = begin_request("w-p1", "attempt-1", &fx.worktree, &targets);
        request.platform_write_paths = vec![PlatformWritePath {
            path: platform_file.clone(),
            ownership: "report-store".to_string(),
        }];
        let handle = block_on(begin_review_window(&store, &snapshot, &request)).expect("begin");

        // Reviewer 直写 .git/index(追加字节:无新对象/无 ref 痕迹——
        // 恢复回写后不可再观)+ 平台域漂移(逼 RecoveryRequired 分支)。
        // index-only 直写(N4 语义可检出形态):写对象 + 直接改 staging
        // (无 ref/HEAD 变化;OID 变 → 语义摘要漂移)。
        fs::write(fx.worktree.join("evil.txt"), "evil\n").unwrap();
        let blob = git_sha(&fx.worktree, &["hash-object", "-w", "evil.txt"]);
        git_ok(
            &fx.worktree,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{blob},evil.txt"),
            ],
        );
        fs::write(&platform_file, "v1-drift\n").unwrap();

        let quiescing = begin_quiesce(&store, "w-p1", handle.generation, "end").unwrap();
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-p1",
            quiescing.generation,
        ))
        .expect_err("平台漂移按 T16 停等");
        assert!(
            matches!(
                &err,
                crate::product::reviewer_window::ReviewerWindowError::RecoveryStopped { reason, .. }
                    if reason.contains("platform")
            ),
            "{err:?}"
        );
        // 恢复前观察已持久化(index 事实在,虽 index 已被 T16 回写)。
        let record = store.load_window("w-p1").unwrap();
        assert!(
            record
                .pre_restore_git_facts
                .iter()
                .any(|entry| entry.path == ".git/index"),
            "pre_restore 必须含 index 事实: {:?}",
            record.pre_restore_git_facts
        );

        // 人工稳定平台 → 重入:内存报告空(index 已回写),持久化恢复
        // 前观察兜底 → ManualGate,不 Accepted。
        fs::write(&platform_file, "v0\n").unwrap();
        let summary = block_on(restore_window(&store, &snapshot, "w-p1", record.generation))
            .expect("reentry");
        assert!(summary.manual_gate, "index-only 直写不得被洗白");
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);
        let journal = store.load_violations("w-p1").unwrap();
        assert!(
            journal.iter().any(|entry| entry.path == ".git/index"),
            "{journal:?}"
        );
        assert!(
            record_report_acceptance(
                &store,
                &snapshot,
                "w-p1",
                summary.record.generation,
                ReportAcceptance::Accepted,
            )
            .is_err()
        );
    }

    /// P1 崩溃变体:恢复入口观察+in_progress 落盘后、gate 收口前 kill
    /// → 重入以持久化恢复前观察续接,仍 ManualGate。
    #[test]
    fn p1_crash_variant_persisted_pre_restore_gates_on_reentry() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p1c", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // index-only 直写(N4 语义可检出形态,同 p1 平台绕行)。
        fs::write(fx.worktree.join("evil.txt"), "evil\n").unwrap();
        let blob = git_sha(&fx.worktree, &["hash-object", "-w", "evil.txt"]);
        git_ok(
            &fx.worktree,
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{blob},evil.txt"),
            ],
        );
        let quiescing = begin_quiesce(&store, "w-p1c", handle.generation, "end").unwrap();

        // 第一次入口:观察+in_progress 已持久化;journal 阻断使收口
        // 失败(等价 kill 现场:Restoring+pre_restore 已在 durable 态)。
        let journal_path = fx.window_root.join("violations").join("w-p1c.json");
        fs::create_dir_all(&journal_path).unwrap();
        let err = block_on(restore_window(
            &store,
            &snapshot,
            "w-p1c",
            quiescing.generation,
        ))
        .expect_err("journal 阻断,收口必须失败");
        assert!(
            matches!(
                &err,
                crate::product::reviewer_window::ReviewerWindowError::Store(_)
            ),
            "{err:?}"
        );
        let record = store.load_window("w-p1c").unwrap();
        assert!(
            record
                .pre_restore_git_facts
                .iter()
                .any(|entry| entry.path == ".git/index")
        );

        // 崩溃后重入(durable 现场续接)→ 仍 ManualGate。
        fs::remove_dir(&journal_path).unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-p1c",
            record.generation,
        ))
        .expect("reentry");
        assert!(summary.manual_gate, "崩溃重入后持久化观察必须兜底");
    }

    /// P2-a:pending/journal 存在但无可复观事实 → 仍 gate,不永久卡
    /// Restoring 也不被采信。
    #[test]
    fn p2a_pending_without_reobservable_facts_still_gates() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p2a", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-p2a", handle.generation, "end").unwrap();

        // 造 pending(A)+ journal 阻断(无可复观 git 事实的纯净仓库)。
        let journal_path = fx.window_root.join("violations").join("w-p2a.json");
        fs::create_dir_all(&journal_path).unwrap();
        assert!(
            record_violation(
                &store,
                &snapshot,
                "w-p2a",
                quiescing.generation,
                ViolationRecord {
                    kind: "channel_bypass",
                    path: "channels/push",
                    before_digest: None,
                    after_digest: None,
                    detail: "push 绕过",
                    report_id: None,
                },
            )
            .is_err()
        );
        fs::remove_dir(&journal_path).unwrap();
        let mut record = store.load_window("w-p2a").unwrap();
        assert!(record.violation_pending.is_some());

        // 白盒构造 P2-a 现场:恢复入口已落(Restoring+in_progress)后、
        // journal 落盘前 kill——pending 残留且无可复观事实(journal 空、
        // 仓库干净)。重入必须 gate 而非永久卡 Restoring/被采信。
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(crate::product::reviewer_window_store::RestoreBinding {
            completed: false,
            from_violation: false,
            outcome: "in_progress".to_string(),
            cursor_phase: None,
            entry_index: None,
            detail: String::new(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        record.bump_transition(ReviewerWindowState::Restoring, "白盒:恢复入口后 kill 现场");
        let expected = record.generation;
        let record = store.save_window_transition(record, expected).unwrap();

        // 重入恢复 → P2-a gate → 清 pending → ManualGate。
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-p2a",
            record.generation,
        ))
        .expect("reentry");
        assert!(summary.manual_gate, "pending 存在即 gate,不得静默放行");
        assert_eq!(summary.record.state, ReviewerWindowState::ManualGate);
        assert!(summary.record.violation_pending.is_none());
    }

    /// P2-b:嵌套路径子模块(modules/vendor/foo)capture/复观不报错。
    #[test]
    fn p2b_nested_submodule_gitdir_observed_without_error() {
        let fx = fixture();
        let nested = fx
            .worktree
            .join(".git")
            .join("modules")
            .join("vendor")
            .join("foo");
        fs::create_dir_all(&nested).unwrap();
        fs::write(nested.join("config"), "[core]\n\tbare = false\n").unwrap();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p2b", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("嵌套子模块 capture 不得失败");
        let quiescing = begin_quiesce(&store, "w-p2b", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-p2b",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(!summary.manual_gate, "无漂移不得误报");
    }

    /// P3-2:git_annex_observation=None 不序列化(旧 manifest JSON 形状不变)。
    #[test]
    fn p3_2_none_annex_omitted_from_json() {
        let manifest = minimal_manifest("w-p32");
        let serialized = serde_json::to_string(&manifest).expect("serialize");
        assert!(!serialized.contains("git_annex_observation"));
        let deserialized: crate::product::reviewer_snapshot::ReviewerBaselineManifest =
            serde_json::from_str(&serialized).expect("deserialize");
        assert!(deserialized.git_annex_observation.is_none());
    }

    /// P3-4:journal 已含 pending.entry_id(陈旧标记)→ 先清再处理新违规。
    #[test]
    fn p3_4_stale_pending_cleared_before_new_violation() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p34", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-p34", handle.generation, "end").unwrap();

        // 造 pending(A)+ journal 阻断。
        let journal_path = fx.window_root.join("violations").join("w-p34.json");
        fs::create_dir_all(&journal_path).unwrap();
        assert!(
            record_violation(
                &store,
                &snapshot,
                "w-p34",
                quiescing.generation,
                ViolationRecord {
                    kind: "platform_domain_write",
                    path: ".aria/a.json",
                    before_digest: None,
                    after_digest: None,
                    detail: "A",
                    report_id: None,
                },
            )
            .is_err()
        );
        let record = store.load_window("w-p34").unwrap();
        let stale_entry_id = record.violation_pending.as_ref().unwrap().entry_id.clone();

        // 白盒补落 A 条目(模拟「落盘成功但清除前崩溃」),再记 B:
        // 陈旧 pending 先清,B 正常推进(不再被挡)。
        fs::remove_dir(&journal_path).unwrap();
        store
            .append_violation(
                "w-p34",
                ViolationJournalEntry {
                    seq: 0,
                    entry_id: stale_entry_id,
                    at: "2026-10-17T00:00:00Z".to_string(),
                    window_id: "w-p34".to_string(),
                    attempt_id: "attempt-1".to_string(),
                    run_id: "run-1".to_string(),
                    report_id: None,
                    kind: "platform_domain_write".to_string(),
                    path: ".aria/a.json".to_string(),
                    before_digest: None,
                    after_digest: None,
                    detail: "A".to_string(),
                    restore_cursor: "started+0".to_string(),
                },
            )
            .unwrap();
        let violated = record_violation(
            &store,
            &snapshot,
            "w-p34",
            record.generation,
            ViolationRecord {
                kind: "git_domain_write",
                path: ".git/x",
                before_digest: None,
                after_digest: None,
                detail: "B",
                report_id: None,
            },
        )
        .expect("陈旧 pending 已清,B 必须可推进");
        assert_eq!(violated.state, ReviewerWindowState::ViolationRecorded);
        assert_eq!(store.load_violations("w-p34").unwrap().len(), 2);
    }

    // ---------- 审查修复轮 3(N1-N4+顺带)钉测 ----------

    /// N2:直写 .git/config.worktree → git_annex_drift → ManualGate。
    #[test]
    fn n2_config_worktree_direct_write_detected() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-n2", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // 窗口内直写 per-worktree 配置面。
        fs::write(
            fx.worktree.join(".git").join("config.worktree"),
            "[core]\n\tfoo\n",
        )
        .unwrap();
        let quiescing = begin_quiesce(&store, "w-n2", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-n2",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(summary.manual_gate, "直写 config.worktree 必须进人工门");
        let journal = store.load_violations("w-n2").unwrap();
        assert!(
            journal
                .iter()
                .any(|entry| entry.kind == "git_annex_drift"
                    && entry.path.contains("worktree_config")),
            "{journal:?}"
        );
    }

    /// N3:修复轮 1 形态 manifest(annex 无 worktree_config_sha256 字段)
    /// 可反序列化,观察跳过该面不误报。
    #[test]
    fn n3_round1_manifest_deserializes_and_skips_missing_face() {
        let mut manifest = minimal_manifest("w-n3");
        manifest.git_annex_observation =
            Some(crate::product::reviewer_snapshot::GitAnnexObservation {
                config_sha256: "a".to_string(),
                common_config_sha256: "b".to_string(),
                // 修复轮 1 形态:无此字段 → JSON 缺失 → 反序列化为空串。
                worktree_config_sha256: String::new(),
                objects_digest: "c".to_string(),
                reflogs_digest: "d".to_string(),
                submodule_gitdirs_digest: "e".to_string(),
            });
        // 手工剔除 worktree_config_sha256 键模拟 round-1 JSON。
        let mut value = serde_json::to_value(&manifest).unwrap();
        let annex = value
            .get_mut("git_annex_observation")
            .unwrap()
            .as_object_mut()
            .unwrap();
        annex.remove("worktree_config_sha256");
        let manifest: crate::product::reviewer_snapshot::ReviewerBaselineManifest =
            serde_json::from_value(value).expect("round-1 manifest 必须可反序列化");
        assert_eq!(
            manifest
                .git_annex_observation
                .as_ref()
                .unwrap()
                .worktree_config_sha256,
            ""
        );

        // 空 before → 观察跳过该面(不把旧窗口当漂移):直接断言比对守卫。
        let mut record = facts_record("w-n3");
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(restored_binding());
        let facts =
            crate::product::reviewer_window::observed_git_violation_facts(&record, &manifest);
        assert!(
            !facts
                .iter()
                .any(|fact| fact.path.contains("worktree_config")),
            "空 before 必须跳过该面: {facts:?}"
        );
    }

    /// N4:常规 git status(stat-refresh)不改语义摘要——不整轮作废;
    /// git add(OID 变)仍被检出。
    #[test]
    fn n4_index_semantic_digest_ignores_stat_refresh_detects_add() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-n4a", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");

        // 场景 1(B2 修复轮 4):touch 已跟踪文件(同内容新 mtime)→
        // git status 触发真实 stat-refresh(index 字节重写)。
        let baseline_index_sha = snapshot
            .load_manifest("w-n4a")
            .unwrap()
            .git_observation
            .index_sha256
            .clone();
        let touched = fx.worktree.join("tracked.txt");
        let content = fs::read_to_string(&touched).unwrap();
        // 触发 stat 过期:同内容经临时文件 rename 回(必然更新 mtime)。
        let tmp = fx.worktree.join("tracked.txt.tmp");
        fs::write(&tmp, &content).unwrap();
        fs::rename(&tmp, &touched).unwrap();
        git_ok(&fx.worktree, &["status", "--porcelain"]);
        // 证明 stat-refresh 确实发生:raw index 字节已≠基线。
        let raw_now = sha256_of_file(&fx.worktree.join(".git").join("index"));
        assert_ne!(
            raw_now, baseline_index_sha,
            "前置失效:git status 未触发 stat-refresh(需 mtime 变化)"
        );
        let quiescing = begin_quiesce(&store, "w-n4a", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-n4a",
            quiescing.generation,
        ))
        .expect("restore");
        assert!(
            !summary.manual_gate,
            "stat-refresh(raw 变、语义不变)不得整轮作废: {:?}",
            store.load_violations("w-n4a").unwrap()
        );
        // 回写后 raw 恢复基线字节(B1:回写仍按 raw 差异执行)。
        assert_eq!(
            sha256_of_file(&fx.worktree.join(".git").join("index")),
            baseline_index_sha
        );

        // 场景 2:git add(新对象入 index,OID 变)仍被检出。
        let fx2 = fixture();
        let store2 = fx2.store();
        let snapshot2 = fx2.snapshot();
        let scope2 = open_scope(&fx2.base, "coder");
        let targets2: [&ProviderRunResourceScope; 1] = [&scope2];
        let handle2 = block_on(begin_review_window(
            &store2,
            &snapshot2,
            &begin_request("w-n4b", "attempt-1", &fx2.worktree, &targets2),
        ))
        .expect("begin");
        fs::write(fx2.worktree.join("smuggled.txt"), "x\n").unwrap();
        git_ok(&fx2.worktree, &["add", "smuggled.txt"]);
        let quiescing2 = begin_quiesce(&store2, "w-n4b", handle2.generation, "end").unwrap();
        let summary2 = block_on(restore_window(
            &store2,
            &snapshot2,
            "w-n4b",
            quiescing2.generation,
        ))
        .expect("restore");
        assert!(summary2.manual_gate, "git add 漂移必须被检出");
        let journal = store2.load_violations("w-n4b").unwrap();
        assert!(
            journal.iter().any(|entry| entry.path == ".git/index"),
            "{journal:?}"
        );
    }

    /// 顺带 2:零事实 pending 清除时落占位条目——reconcile 不误报
    /// journal_tamper(from_violation 真而 journal 空的矛盾)。
    #[test]
    fn incidental2_pending_placeholder_entry_prevents_tamper_false_positive() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-i2", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let quiescing = begin_quiesce(&store, "w-i2", handle.generation, "end").unwrap();

        // 造 pending + 白盒构造 Restoring 现场(同 p2a)。
        let journal_path = fx.window_root.join("violations").join("w-i2.json");
        fs::create_dir_all(&journal_path).unwrap();
        assert!(
            record_violation(
                &store,
                &snapshot,
                "w-i2",
                quiescing.generation,
                ViolationRecord {
                    kind: "channel_bypass",
                    path: "channels/commit",
                    before_digest: None,
                    after_digest: None,
                    detail: "绕过",
                    report_id: None,
                },
            )
            .is_err()
        );
        fs::remove_dir(&journal_path).unwrap();
        let mut record = store.load_window("w-i2").unwrap();
        record.state = ReviewerWindowState::Restoring;
        record.restore = Some(crate::product::reviewer_window_store::RestoreBinding {
            completed: false,
            from_violation: false,
            outcome: "in_progress".to_string(),
            cursor_phase: None,
            entry_index: None,
            detail: String::new(),
            git_restored_mutation_paths: Vec::new(),
            git_extra_refs_observed: Vec::new(),
            platform_verified: Vec::new(),
        });
        record.bump_transition(ReviewerWindowState::Restoring, "白盒 kill 现场");
        let expected = record.generation;
        let record = store.save_window_transition(record, expected).unwrap();

        let summary = block_on(restore_window(&store, &snapshot, "w-i2", record.generation))
            .expect("reentry");
        assert!(summary.manual_gate);
        // 占位条目已落:reconcile 无 journal_tamper 误报。
        let journal = store.load_violations("w-i2").unwrap();
        assert_eq!(journal.len(), 1, "pending 占位条目: {journal:?}");
        let outcome = reconcile_from_stores(&store, &snapshot, "w-i2").unwrap();
        match &outcome.recommendation {
            AcceptanceRecommendation::Invalidated { reasons, .. } => {
                assert!(
                    !reasons.iter().any(
                        |reason| reason.contains("journal 为空") || reason.contains("序号断裂")
                    ),
                    "不得误报篡改: {reasons:?}"
                );
            }
            other => panic!("期望 Invalidated,得到 {other:?}"),
        }
    }

    /// P1(修复轮 5):index 损坏(直写垃圾)不可被 git 解析——恢复不
    /// 卡死:基线 blob 回写修复 + 违规事实在 + raw 归位基线。
    #[test]
    fn p1r_corrupt_index_restored_not_stuck() {
        let fx = fixture();
        let store = fx.store();
        let snapshot = fx.snapshot();
        let scope = open_scope(&fx.base, "coder");
        let targets: [&ProviderRunResourceScope; 1] = [&scope];
        let handle = block_on(begin_review_window(
            &store,
            &snapshot,
            &begin_request("w-p1r", "attempt-1", &fx.worktree, &targets),
        ))
        .expect("begin");
        let baseline_index_sha = snapshot
            .load_manifest("w-p1r")
            .unwrap()
            .git_observation
            .index_sha256
            .clone();

        // 直写垃圾字节:index 不可被 git 解析。
        fs::write(
            fx.worktree.join(".git").join("index"),
            b"garbage-not-an-index",
        )
        .unwrap();
        let quiescing = begin_quiesce(&store, "w-p1r", handle.generation, "end").unwrap();
        let summary = block_on(restore_window(
            &store,
            &snapshot,
            "w-p1r",
            quiescing.generation,
        ))
        .expect("损坏 index 不得中止恢复(基线 blob 是修复源)");

        // index 已被基线字节修复(raw 归位)。
        assert_eq!(
            sha256_of_file(&fx.worktree.join(".git").join("index")),
            baseline_index_sha
        );
        // 违规事实在(语义不可解析保守视为漂移→恢复事实→人工门)。
        assert!(summary.manual_gate, "损坏 index 必须按漂移收口");
        let journal = store.load_violations("w-p1r").unwrap();
        assert!(
            journal.iter().any(|entry| entry.path == ".git/index"),
            "{journal:?}"
        );
    }
}
