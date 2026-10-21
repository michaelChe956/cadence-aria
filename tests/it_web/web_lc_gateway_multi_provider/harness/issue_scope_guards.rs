//! T23 夹具范围守卫(pi r2/codex r2 现场,诊断 diag-t23-r2-newreds)
//! ——大文件守卫:harness 已按祖父条款登记 14966 行上限(棘轮只许删
//! 不许涨),本修复的语义选答 Preserve 档、plan target 基数守卫与
//! issue 只读纪律句整体落到本文件,harness 只留薄接线。
//!
//! 现场根因(两处产品行为均正确/冻结契约,本文件只修夹具):
//! - pi r2:语义自动应答把 issue 的排除条款「投票」推翻——Q1 选了
//!   「beta 需要修改,纳入本需求范围」(唯一非负向项),Q2 四项全含
//!   「不」全 Negative,兜底盲取 index 0「以我的选择为准」;involved
//!   被回写为 [alpha,beta] → plan 合法拆出多 target → coding 旧 group
//!   单对象入口按冻结契约 409(repository_routing_ambiguous),full_chain
//!   coding fresh 永远 denied(产品样假红)。
//! - codex r2:plan 作者期用 shell 重定向写 /tmp/plan_candidate.md 被
//!   Author 岗位检测型 deny 面正确拦截(author_file_change_detected,
//!   SIGKILL)。产品纪律不得放松(spec 明文禁止 read-only/拒
//!   commandExecution 替代),夹具侧把只读约束显式写进 issue 文本
//!   (恒进 story/design/plan 的 [issue] 段)降低触发率。

use std::collections::BTreeSet;

use axum::http::Method;

use serde_json::{Value, json};

use super::{EvidenceCell, PROJECT_ID, StageObservation, fp, request_json};

/// T23(codex r2)修法A:issue 文本只读纪律句——Author 岗位(含 plan
/// authoring)的检测型纪律前置成显式提示;「规划/作者阶段」限定不
/// 约束 coder 阶段(b45 描述尾部允许 coder 用 tee 追加 CHECK-EVIDENCE)。
pub(crate) const ISSUE_AUTHOR_READONLY_CLAUSE: &str = "规划/作者阶段为只读:禁止以任何方式写文件(含 shell 重定向、临时文件、apply_patch);候选内容直接在回复中输出。";

/// Preserve 档关键词(pi r2 现场选项逐字):命中任一 = 「保持 Issue
/// 边界」选项。lowercase 匹配(Issue→issue)。窄清单——不得放宽到
/// 「保持」/「不变」/「零调整」一类词(r62 Q5「A. 不调整…零调整即零
/// 兼容风险」是实义 Negative 档,误纳会翻掉既有选答测试)。
const PRESERVE_ISSUE_BOUNDARY_MARKERS: [&str; 4] =
    ["零改动", "保持 issue 边界", "以 issue 为准", "不属交付范围"];

/// 标签是否属「保持 Issue 边界」(Preserve)档。判定必须先于 Negative
/// 档(「不属交付范围」含「不」、「保持 Issue 边界」含「保持」,旧口径
/// 会把正解挤出局);选择序最高优先(Preserve > SelectAll > Confirm >
/// 首个非负向)。
pub(super) fn is_preserve_issue_boundary_option(label: &str) -> bool {
    let normalized = label.trim().to_lowercase();
    PRESERVE_ISSUE_BOUNDARY_MARKERS
        .iter()
        .any(|marker| normalized.contains(marker))
}

/// 全 Negative 兜底时按选项标签原文(绕过 tier 覆盖,如宣告类降级)
/// 找 Preserve 档首项——替代无条件的 index 0。
pub(super) fn preserve_option_index(options: &[Value]) -> Option<usize> {
    options.iter().position(|option| {
        is_preserve_issue_boundary_option(option.get("label").and_then(Value::as_str).unwrap_or(""))
    })
}

/// create_issue 默认描述(原 harness 内联文本 + 只读纪律句;句子恒进
/// story/design/plan 的 [issue] 段,锚点无关)。
pub(super) fn default_issue_description() -> String {
    format!(
        "在 alpha 仓内实现会话过期签发、判定与用户提示;本需求不修改 beta 仓(beta 仅按既有接口消费,不属本需求交付范围)。{ISSUE_AUTHOR_READONLY_CLAUSE}四家五阶段真实矩阵驱动 issue。"
    )
}

/// T23 修法C:plan target 基数判定(lifecycle 分组视图进)。>1 个非空
/// `target_repository_id` = 夹具范围漂移(issue 排除条款被语义应答/作者
/// 扩写推翻);单 target / 无 work item 返回 None(不拦截)。
pub(super) fn plan_target_scope_drift(lifecycle_body: &Value) -> Option<String> {
    let targets: BTreeSet<&str> = lifecycle_body["work_item_repository_groups"]
        .as_array()
        .map(|groups| {
            groups
                .iter()
                .filter_map(|group| group["target_repository_id"].as_str())
                .collect()
        })
        .unwrap_or_default();
    (targets.len() > 1).then(|| {
        format!(
            "plan work items 落在多个 target 仓({targets:?});系夹具语义应答/作者扩写推翻 Issue 排除条款所致的范围漂移(修法:Preserve 档),非产品缺陷——coding 旧 group 单对象入口按冻结契约拒绝多 target"
        )
    })
}

/// T23 修法C:coding 入口前的 plan target 基数守卫——漂移时以
/// `matrix_fixture_scope_drift` 夹具诊断失败收口(fresh+resume 双格),
/// 不把产品合法 409 呈现成阶段红;查询失败不拦截(交由原逻辑如实
/// 呈现产品行为)。
pub(super) async fn coding_scope_drift_guard(
    env: &super::MatrixEnvironment,
) -> Option<Vec<EvidenceCell>> {
    let (status, body) = request_json(
        &env.app,
        Method::GET,
        &format!(
            "/api/issues/{}/lifecycle?project_id={PROJECT_ID}",
            env.issue_id
        ),
        json!({}),
    )
    .await;
    if !status.is_success() {
        return None;
    }
    let drift = plan_target_scope_drift(&body)?;
    fp("matrix_fixture_scope_drift", format_args!("{drift}"));
    let mut resume = StageObservation::new("coding", &env.provider);
    resume.force_resume = true;
    Some(vec![
        StageObservation::new("coding", &env.provider).denied_cell(format!(
            "matrix_fixture_scope_drift(夹具范围漂移,非产品 409):{drift}"
        )),
        resume.denied_cell("coding fresh 未启动(夹具范围漂移拦截),resume 无会话".to_string()),
    ])
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::super::live_b45_t22_support::b45_kimi_issue_description;
    use super::super::semantic_option_selection;
    use super::*;

    /// T23(pi r2 现场逐字,log :4580/:4585):两题必须选中「保持 Issue
    /// 边界」项。修复前 Q1 选「beta 需要修改,纳入本需求范围」、Q2 全
    /// Negative 兜底盲取 index 0「以我的选择为准」→ involved 扩到
    /// [alpha,beta] → plan 多 target → coding 旧入口契约 409。
    #[test]
    fn semantic_selection_preserves_issue_boundary_r2_wording() {
        let q1 = json!([
            {"id": "opt_0", "label": "beta 零改动；alpha 既有公开 API（cross_repo_greeting 等）不得破坏，会话能力仅新增于 alpha，beta 继续按既有接口消费"},
            {"id": "opt_1", "label": "beta 需适配新的会话接口，但 beta 仓仍不修改（仅验收其能通过既有接口消费）"},
            {"id": "opt_2", "label": "beta 需要修改，纳入本需求范围"},
            {"id": "opt_3", "label": "其他（我来补充）"},
        ]);
        assert_eq!(
            semantic_option_selection(
                "「beta 仅按既有接口消费，不属本需求交付范围」如何验收？",
                q1.as_array().unwrap(),
                false,
            ),
            vec!["opt_0".to_string()],
            "Q1 必须选到「beta 零改动」的保持 Issue 边界项,不得选「beta 需要修改」"
        );
        let q2 = json!([
            {"id": "opt_0", "label": "以我的选择为准：beta 仓修改纳入需求范围，Issue 的“不修改 beta”条款视为被覆盖（involved 含 alpha+beta）"},
            {"id": "opt_1", "label": "以 Issue 为准：beta 不修改；仅验收 beta 通过既有接口消费 alpha 不回归（involved 仅 alpha）"},
            {"id": "opt_2", "label": "beta 仅在必须时做最小适配改动，但仍以“不改 beta”为默认、适配需单独审批（involved 含 alpha+beta，带条件）"},
            {"id": "opt_3", "label": "其他（我来补充）"},
        ]);
        assert_eq!(
            semantic_option_selection(
                "你此前的选择与 Issue 的排除条款冲突，最终以哪个为准？",
                q2.as_array().unwrap(),
                false,
            ),
            vec!["opt_1".to_string()],
            "Q2 全 Negative 兜底必须先找到「以 Issue 为准」的保持项,不得盲取 index 0"
        );
    }

    /// Preserve 项被宣告类判定降级为 Negative 时,全 Negative 兜底按
    /// 标签原文捞回保持项(r1 five-stages 曾靠「兜底 index 0 恰好是
    /// 保持项」侥幸对齐,不再依赖问法运气)。
    #[test]
    fn semantic_selection_all_negative_falls_back_to_preserve() {
        let options = json!([
            {"id": "opt_expand", "label": "不维持现状：beta 纳入并修改（不允许保持不动）"},
            {"id": "opt_keep", "label": "保持 Issue 边界（其他，我会在回答中说明排除细节）"},
        ]);
        assert_eq!(
            semantic_option_selection(
                "验收边界冲突，以哪个为准？",
                options.as_array().unwrap(),
                false,
            ),
            vec!["opt_keep".to_string()],
            "全 Negative 兜底必须先按标签原文找到 Preserve 档,不得盲取 index 0"
        );
    }

    /// T23(codex r2)修法A:两处 issue 描述(默认 + b45 override)都必须
    /// 携带只读纪律句——plan 作者用 shell 重定向写 /tmp 草稿被检测纪律
    /// 正确拦截后,纪律前置进 [issue] 段降低触发率。
    #[test]
    fn issue_descriptions_carry_author_readonly_clause() {
        assert!(
            default_issue_description().contains(ISSUE_AUTHOR_READONLY_CLAUSE),
            "create_issue 默认描述必须含只读纪律句"
        );
        assert!(
            b45_kimi_issue_description().contains(ISSUE_AUTHOR_READONLY_CLAUSE),
            "b45 特化描述(b45_kimi_issue_description)必须含只读纪律句"
        );
    }

    /// T23 修法C:target 基数守卫——>1 个非空 target 报夹具漂移,单
    /// target/空 target 组不拦截。
    #[test]
    fn plan_target_scope_drift_flags_multi_target_only() {
        let single = json!({"work_item_repository_groups": [
            {"target_repository_id": "repo_alpha"},
        ]});
        assert!(plan_target_scope_drift(&single).is_none());
        let multi = json!({"work_item_repository_groups": [
            {"target_repository_id": "repo_alpha"},
            {"target_repository_id": "repo_beta"},
        ]});
        let drift = plan_target_scope_drift(&multi).expect("多 target 必须报漂移");
        assert!(drift.contains("repo_alpha") && drift.contains("repo_beta"));
        let untethered = json!({"work_item_repository_groups": [
            {"target_repository_id": null},
            {"target_repository_id": "repo_alpha"},
        ]});
        assert!(plan_target_scope_drift(&untethered).is_none());
    }
}
