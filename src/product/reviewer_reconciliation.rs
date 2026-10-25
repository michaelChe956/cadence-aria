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
    include!("reviewer_reconciliation_tests_core.inc.rs");
    include!("reviewer_reconciliation_tests_recovery.inc.rs");
}
