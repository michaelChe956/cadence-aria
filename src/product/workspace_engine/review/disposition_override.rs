//! REQ-WSC-12/14（k3 双裁定 P0-2B / REQ-WSC-12 返修触发路径）：**平台裁决**
//! owner 已处置事实，而不是只把清单塞进 prompt 指望模型听话。
//!
//! 现场（三轮 reviewer 连报同一条范围外事实）证明「只靠 prompt 注入」不收敛：
//! 模型仍可照旧输出 `actionable`。因此本模块在**会话上下文**（能读到 durable
//! 已处置清单的地方）做一层归一：
//!
//! 1. `finding_key ∈ 该 session 已处置清单` → 强制 `advisory`（owner 事实优先，
//!    不依赖模型标对）。
//! 2. 归一之后 `revise` 但**没有任何 actionable finding** → 归一回来（`pass`），
//!    绝不驱作者返修（REQ-WSC-12「revise 归一」）。
//!
//! 归一后的 verdict 才进入 `latest_review_verdict`（必关闭清单 / 下一轮 prompt）、
//! `classify_finding`（指纹闸门 / transition budget）与 policy 路由。
//!
//! **不落 durable**：durable 只保 provider 原始 verdict + 已处置清单两事实；
//! 归一结果是读路径投影（刷新后由同一清单重新算出同一结果）。

use std::collections::BTreeSet;

use crate::product::lifecycle_store::LifecycleStore;
use crate::product::models::HumanDispositionRecord;
use crate::web::workspace_ws_types::review::{
    ReviewFinding, ReviewFindingDisposition, ReviewFindingSeverity, ReviewVerdict,
    ReviewVerdictType, WorkItemPlanReviewVerdict,
};

use super::super::parsers::{review_gate_for_findings, work_item_plan_review_routing};

/// 已处置清单 → 身份集合（只保留形态合法的平台 key）。
pub(crate) fn human_disposition_key_set(records: &[HumanDispositionRecord]) -> BTreeSet<String> {
    records
        .iter()
        .map(|record| record.finding_key.as_str())
        .filter(|key| HumanDispositionRecord::is_valid_finding_key(key))
        .map(ToString::to_string)
        .collect()
}

/// 会话级归一唯一入口：owner 已处置事实 + 「revise 无 actionable」归一。
///
/// 幂等：重复调用结果不变（归一后再归一不再改动）。
pub(crate) fn normalize_review_verdict_for_dispositions(
    verdict: &mut ReviewVerdict,
    excluded_keys: &BTreeSet<String>,
) {
    force_owner_excluded_findings_advisory(verdict, excluded_keys);
    renormalize_verdict_after_disposition_change(verdict);
}

fn force_owner_excluded_findings_advisory(
    verdict: &mut ReviewVerdict,
    excluded_keys: &BTreeSet<String>,
) {
    if excluded_keys.is_empty() {
        return;
    }
    for finding in &mut verdict.findings {
        if !excluded_keys.contains(&finding.computed_finding_key()) {
            continue;
        }
        // owner 事实优先：即便模型（或 P0-1 加固）把它算作 actionable，也按
        // 已处置处理。`required_action` 保留原文（呈现/复盘需要），但它在
        // 平台的所有判定面上都已不计入。
        finding.disposition = Some(ReviewFindingDisposition::Advisory);
    }
}

fn has_actionable_blocking(findings: &[ReviewFinding]) -> bool {
    findings.iter().any(|finding| {
        finding.is_actionable()
            && matches!(
                finding.severity,
                ReviewFindingSeverity::Blocking | ReviewFindingSeverity::MustFix
            )
    })
}

fn has_actionable(findings: &[ReviewFinding]) -> bool {
    findings.iter().any(ReviewFinding::is_actionable)
}

/// 归一后重算判定面（与解析期同一判定函数，不另立第二口径）：
///
/// - `pass` + actionable blocking/must_fix → 返修族（解析期同规则）；
/// - `revise` + 无任何 actionable → `pass`（REQ-WSC-12 第三条：「revise 但无
///   actionable」不得驱作者返修，也不占 transition budget）。
/// - `revise_batch` / `plan_reopen_required` **不参与**归一——它们是专用 legacy
///   返修机制（既有契约用例依赖其既有行为），`revise_batch` 的归一留在 policy
///   路由侧，不在此处重算。
fn renormalize_verdict_after_disposition_change(verdict: &mut ReviewVerdict) {
    let actionable_blocking = has_actionable_blocking(&verdict.findings);
    let any_actionable = has_actionable(&verdict.findings);

    if let Some(mut extension) = verdict.work_item_plan_review.clone() {
        let normalized = match &extension.verdict {
            WorkItemPlanReviewVerdict::Pass if actionable_blocking => {
                match extension.review_scope {
                    crate::web::workspace_ws_types::review::WorkItemPlanReviewScope::Outline
                    | crate::web::workspace_ws_types::review::WorkItemPlanReviewScope::Item => {
                        WorkItemPlanReviewVerdict::Revise
                    }
                    crate::web::workspace_ws_types::review::WorkItemPlanReviewScope::Batch => {
                        WorkItemPlanReviewVerdict::ReviseBatch
                    }
                }
            }
            // REQ-WSC-12 第三条只覆盖「有 findings 但全非 actionable」；空
            // findings 的 `revise` 是既有 legacy 返修入口，MUST NOT 被归一掉。
            // `revise_batch` / `plan_reopen_required` 是专用 legacy 返修机制，
            // 同样不在本归一面内。
            WorkItemPlanReviewVerdict::Revise
                if !verdict.findings.is_empty() && !any_actionable =>
            {
                WorkItemPlanReviewVerdict::Pass
            }
            other => other.clone(),
        };
        if normalized != extension.verdict {
            let (generic_verdict, review_gate, review_action, gates) =
                work_item_plan_review_routing(&normalized, &extension.review_scope);
            extension.verdict = normalized;
            extension.review_action = review_action;
            extension.gates = gates;
            verdict.verdict = generic_verdict;
            verdict.review_gate = review_gate;
            verdict.work_item_plan_review = Some(extension);
        }
        return;
    }

    // 通用评审（Story/Design/单候选 legacy 之外）：同口径归一。
    let normalized_verdict = match &verdict.verdict {
        ReviewVerdictType::Pass if actionable_blocking => ReviewVerdictType::Revise,
        ReviewVerdictType::Revise if !verdict.findings.is_empty() && !any_actionable => {
            ReviewVerdictType::Pass
        }
        other => other.clone(),
    };
    let review_gate = review_gate_for_findings(&normalized_verdict, &verdict.findings, false);
    verdict.verdict = normalized_verdict;
    verdict.review_gate = review_gate;
}

/// 用 durable 清单归一一条 verdict（恢复/刷新路径与 live 路径同源）。
pub(crate) fn normalize_review_verdict_with_store(
    store: &LifecycleStore,
    session_id: &str,
    mut verdict: ReviewVerdict,
) -> ReviewVerdict {
    let excluded = store
        .list_human_dispositions(session_id)
        .map(|records| human_disposition_key_set(&records))
        .unwrap_or_default();
    normalize_review_verdict_for_dispositions(&mut verdict, &excluded);
    verdict
}

impl super::super::WorkspaceEngine {
    /// 该会话当前生效的已处置事实身份集合（读失败＝空集：不误降级，模型
    /// 报什么就按什么处理——fail-safe 方向与 disposition 缺失一致）。
    pub(crate) fn session_human_disposition_keys(&self) -> BTreeSet<String> {
        let Some(store) = self.lifecycle_store.as_ref() else {
            return BTreeSet::new();
        };
        store
            .list_human_dispositions(&self.session.session_id)
            .map(|records| human_disposition_key_set(&records))
            .unwrap_or_default()
    }

    /// 收口一次评审：记消息 → **持久化 provider 原始 verdict**（durable 只保
    /// 原始事实）→ 再按 owner 已处置清单做平台裁决 → 设为 latest verdict。
    /// 裁决结果不回写 durable（P0-3 同一纪律：读路径产生的事实不落盘）。
    pub(crate) async fn finalize_review_verdict(
        &mut self,
        node_id: &str,
        readable_output: String,
        verdict: ReviewVerdict,
    ) -> ReviewVerdict {
        self.record_review_message(readable_output);
        let persisted = serde_json::to_value(&verdict).unwrap_or(serde_json::Value::Null);
        let _ = self.persist_review_verdict(node_id, persisted).await;
        let verdict = self.apply_session_owner_dispositions(verdict);
        self.latest_review_verdict = Some(verdict.clone());
        verdict
    }

    /// 会话级平台裁决：owner 已处置事实 + revise 归一（见模块文档）。
    pub(crate) fn apply_session_owner_dispositions(
        &self,
        mut verdict: ReviewVerdict,
    ) -> ReviewVerdict {
        let excluded = self.session_human_disposition_keys();
        normalize_review_verdict_for_dispositions(&mut verdict, &excluded);
        verdict
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::product::work_item_plan_policy::ReviewFindingCategory;
    use crate::web::workspace_ws_types::review::{
        ReviewGate, WorkItemPlanReviewComplete, WorkItemPlanReviewScope,
    };

    fn finding(severity: ReviewFindingSeverity, message: &str) -> ReviewFinding {
        ReviewFinding {
            severity,
            message: message.to_string(),
            evidence: String::new(),
            required_action: "修这里".to_string(),
            category: None,
            class_hint: None,
            contract_field: None,
            machine_repair: None,
            owner_question: None,
            owner_options: None,
            disposition: None,
            target_work_item_id: None,
            finding_key: None,
        }
    }

    fn extension(verdict: WorkItemPlanReviewVerdict) -> WorkItemPlanReviewComplete {
        let (_, _, review_action, gates) =
            work_item_plan_review_routing(&verdict, &WorkItemPlanReviewScope::Outline);
        WorkItemPlanReviewComplete {
            verdict,
            review_scope: WorkItemPlanReviewScope::Outline,
            target_outline_id: None,
            generation_round_id: "round_0001".to_string(),
            draft_id: None,
            batch_id: None,
            review_action,
            gates,
            affects_items: Vec::new(),
            warnings: Vec::new(),
        }
    }

    fn verdict_with(
        verdict: ReviewVerdictType,
        findings: Vec<ReviewFinding>,
        extension: Option<WorkItemPlanReviewComplete>,
    ) -> ReviewVerdict {
        ReviewVerdict {
            verdict,
            comments: String::new(),
            summary: "look".to_string(),
            findings,
            review_gate: ReviewGate::RequiresRevision,
            work_item_plan_review: extension,
            structured_output_diagnostic: None,
        }
    }

    /// REQ-WSC-14 / P0-2B：模型照旧报 actionable，但该 key 已在 owner 已处置
    /// 清单里 → 平台强制按 advisory 处理（不依赖模型听话）。
    #[test]
    fn owner_excluded_finding_is_forced_advisory() {
        let finding = finding(ReviewFindingSeverity::MustFix, "中文 locale 尚未配置");
        let excluded = BTreeSet::from([finding.computed_finding_key()]);
        let mut verdict = verdict_with(
            ReviewVerdictType::Revise,
            vec![finding],
            Some(extension(WorkItemPlanReviewVerdict::Revise)),
        );

        normalize_review_verdict_for_dispositions(&mut verdict, &excluded);

        assert_eq!(
            verdict.findings[0].disposition,
            Some(ReviewFindingDisposition::Advisory)
        );
        assert!(!verdict.findings[0].is_actionable());
        // 归一后无 actionable → 不驱作者返修。
        assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
        assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
        assert_eq!(
            verdict.work_item_plan_review.as_ref().unwrap().verdict,
            WorkItemPlanReviewVerdict::Pass
        );
    }

    /// 未处置的 finding 下一轮仍阻断（owner 只排除了 A，B 不动）。
    #[test]
    fn unexcluded_actionable_finding_still_blocks() {
        let excluded_finding = finding(ReviewFindingSeverity::MustFix, "范围外文档缺口");
        let real_gap = finding(ReviewFindingSeverity::MustFix, "本 WI 验收标准缺失");
        let excluded = BTreeSet::from([excluded_finding.computed_finding_key()]);
        let mut verdict = verdict_with(
            ReviewVerdictType::Revise,
            vec![excluded_finding, real_gap],
            Some(extension(WorkItemPlanReviewVerdict::Revise)),
        );

        normalize_review_verdict_for_dispositions(&mut verdict, &excluded);

        assert_eq!(
            verdict.findings[1].disposition, None,
            "未处置 finding 不得被顺带降级"
        );
        assert!(verdict.findings[1].is_actionable());
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        assert_eq!(
            verdict.work_item_plan_review.as_ref().unwrap().verdict,
            WorkItemPlanReviewVerdict::Revise
        );
    }

    /// REQ-WSC-12 第三条：模型自己输出 revise、findings 全是 advisory → 归一
    /// 回来，不得驱作者返修（通用评审路径同口径）。
    #[test]
    fn revise_with_only_advisory_findings_normalizes_to_pass() {
        let mut advisory = finding(ReviewFindingSeverity::MustFix, "仅供参考");
        advisory.disposition = Some(ReviewFindingDisposition::Advisory);
        let mut verdict = verdict_with(ReviewVerdictType::Revise, vec![advisory], None);

        normalize_review_verdict_for_dispositions(&mut verdict, &BTreeSet::new());

        assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
        assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
    }

    /// 空清单 = 零改动（不引入新行为，旧会话逐字不变）。
    #[test]
    fn empty_ledger_keeps_verdict_untouched() {
        let mut verdict = verdict_with(
            ReviewVerdictType::Revise,
            vec![finding(ReviewFindingSeverity::Suggestion, "小建议")],
            None,
        );
        verdict.review_gate = review_gate_for_findings(&verdict.verdict, &verdict.findings, false);
        let before = verdict.clone();

        normalize_review_verdict_for_dispositions(&mut verdict, &BTreeSet::new());

        assert_eq!(verdict, before);
    }

    /// P0-1 加固与 owner 事实的优先级：契约类缺口被 owner 排除后，平台仍按
    /// owner 事实降级（加固不得把已处置事实复活成 actionable）。
    #[test]
    fn owner_exclusion_wins_over_contract_gap_hardening() {
        let mut gap = finding(ReviewFindingSeverity::MustFix, "契约缺少 400 分支");
        gap.category = Some(ReviewFindingCategory::ContractGap);
        gap.disposition = Some(ReviewFindingDisposition::Advisory);
        let gap = gap.with_platform_disposition_rules();
        assert!(
            gap.is_actionable(),
            "契约类缺口标 advisory 仍按 actionable 处理（P0-1 加固）"
        );

        let excluded = BTreeSet::from([gap.computed_finding_key()]);
        let mut verdict = verdict_with(ReviewVerdictType::Revise, vec![gap], None);
        normalize_review_verdict_for_dispositions(&mut verdict, &excluded);

        assert!(!verdict.findings[0].is_actionable());
        assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
    }
}
