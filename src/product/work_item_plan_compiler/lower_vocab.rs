//! lower 侧 canonical enum 词表（T3 enum 诊断可执行化）。
//!
//! 三个 canonical enum 字段的**单一事实源**：每张表列出该字段的全部
//! `(字面量, 枚举值)`；解析（一次表查）与越界诊断回显的「完整合法集」
//! 共用同一张表——不另写一份易漂移的字符串。

use super::CompilerDiagnostic;
use super::diagnostic;
use crate::product::work_item_contract::{BlockerRoute, ContractCompatibilityPolicy, EvidenceKind};

/// `compatibility_policy` 的完整合法值表（字面量 ↔ 枚举值）。
pub(super) const COMPATIBILITY_POLICY_CASES: &[(&str, ContractCompatibilityPolicy)] = &[
    ("require_all", ContractCompatibilityPolicy::RequireAll),
    ("require_any", ContractCompatibilityPolicy::RequireAny),
];

/// `route`（Blockers.route）的完整合法值表（字面量 ↔ 枚举值）。
pub(super) const BLOCKER_ROUTE_CASES: &[(&str, BlockerRoute)] = &[
    ("coder_rework", BlockerRoute::CoderRework),
    ("verification_retry", BlockerRoute::VerificationRetry),
    ("plan_repair_current", BlockerRoute::PlanRepairCurrent),
    ("plan_repair_upstream", BlockerRoute::PlanRepairUpstream),
    ("subgraph_replan", BlockerRoute::SubgraphReplan),
    ("story_amendment", BlockerRoute::StoryAmendment),
    ("design_amendment", BlockerRoute::DesignAmendment),
    ("operational_gate", BlockerRoute::OperationalGate),
];

/// `required_evidence` 的完整合法值表（字面量 ↔ 枚举值）。
pub(super) const REQUIRED_EVIDENCE_CASES: &[(&str, EvidenceKind)] = &[
    ("source_diff", EvidenceKind::SourceDiff),
    (
        "non_zero_test_execution",
        EvidenceKind::NonZeroTestExecution,
    ),
    ("manual_check", EvidenceKind::ManualCheck),
    ("handoff_field", EvidenceKind::HandoffField),
];

/// 一次表查：命中返回对应枚举值，未命中返回 None（由调用方产诊断）。
fn lookup<T: Clone>(cases: &[(&str, T)], value: &str) -> Option<T> {
    cases
        .iter()
        .find(|(allowed, _)| *allowed == value)
        .map(|(_, variant)| variant.clone())
}

/// canonical enum 越界诊断（T3）：文本同时给出「模型实际写了什么」与
/// 「该字段全部允许值」——作者据此一处自纠，不必反查 schema。
/// 合法集直接取自解析用的同一张表（单一事实源）。
fn enum_out_of_range<T>(
    field: &str,
    cases: &[(&str, T)],
    actual_value: &str,
    line: usize,
    repair_example: &str,
) -> CompilerDiagnostic {
    let values = cases.iter().map(|(value, _)| *value).collect::<Vec<_>>();
    let allowed = values.join(" / ");
    diagnostic(
        field,
        &format!("字段值不符合 canonical enum。实际值 `{actual_value}`；允许值：{allowed}。"),
        line,
        repair_example,
    )
}

/// 解析 `compatibility_policy`：合法值返回对应枚举值，越界产诊断并返回 None。
pub(super) fn parse_compatibility_policy(
    value: &str,
    diagnostics: &mut Vec<CompilerDiagnostic>,
    line: usize,
) -> Option<ContractCompatibilityPolicy> {
    match lookup(COMPATIBILITY_POLICY_CASES, value) {
        Some(variant) => Some(variant),
        None => {
            diagnostics.push(enum_out_of_range(
                "compatibility_policy",
                COMPATIBILITY_POLICY_CASES,
                value,
                line,
                "- compatibility_policy: require_all",
            ));
            None
        }
    }
}

/// 解析 `route`（Blockers.route）：合法值返回对应枚举值，越界产诊断并返回 None。
pub(super) fn parse_blocker_route(
    value: &str,
    diagnostics: &mut Vec<CompilerDiagnostic>,
    line: usize,
) -> Option<BlockerRoute> {
    match lookup(BLOCKER_ROUTE_CASES, value) {
        Some(variant) => Some(variant),
        None => {
            diagnostics.push(enum_out_of_range(
                "route",
                BLOCKER_ROUTE_CASES,
                value,
                line,
                "- route: operational_gate",
            ));
            None
        }
    }
}

/// 解析 `required_evidence`：合法值返回对应枚举值，越界产诊断并返回 None。
pub(super) fn parse_evidence_kind(
    value: &str,
    diagnostics: &mut Vec<CompilerDiagnostic>,
    line: usize,
) -> Option<EvidenceKind> {
    match lookup(REQUIRED_EVIDENCE_CASES, value) {
        Some(variant) => Some(variant),
        None => {
            diagnostics.push(enum_out_of_range(
                "required_evidence",
                REQUIRED_EVIDENCE_CASES,
                value,
                line,
                "- required_evidence: source_diff",
            ));
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// canonical enum 变体的 canonical 拼写（serde `rename_all = "snake_case"`）：
    /// 合法值表必须与之逐字一致——canonical enum 的「canonical」由 serde
    /// 定义，不靠手写字符串复述。
    fn canonical_snake_case<T: serde::Serialize>(value: &T) -> String {
        serde_json::to_value(value)
            .expect("canonical enum 变体必须可序列化")
            .as_str()
            .expect("canonical enum 变体必须序列化为字符串")
            .to_string()
    }

    /// 一组变体的 canonical 拼写（用于与生产侧合法值表逐字比对）。
    fn canonical_snake_cases<T: serde::Serialize>(values: &[T]) -> Vec<String> {
        values.iter().map(canonical_snake_case).collect()
    }

    /// 合法值表必须与 serde canonical 拼写逐字一致（顺序也一致）。
    fn assert_allowed_values<T>(actual: &[(&str, T)], expected: &[String]) {
        let actual: Vec<&str> = actual.iter().map(|(value, _)| *value).collect();
        let expected: Vec<&str> = expected.iter().map(String::as_str).collect();
        assert_eq!(actual, expected);
    }

    /// 表中全部合法值（诊断回显用）。
    fn allowed_values<'a, T>(cases: &'a [(&'a str, T)]) -> Vec<&'a str> {
        cases.iter().map(|(value, _)| *value).collect()
    }

    /// 越界诊断必须回显实际非法值与该字段全部合法值——作者据此一处自纠。
    fn assert_enum_diagnostic(
        diagnostic: &CompilerDiagnostic,
        field: &str,
        actual_value: &str,
        line: usize,
        allowed_values: &[&str],
    ) {
        assert_eq!(diagnostic.code, "lowering_error");
        assert_eq!(diagnostic.field, field);
        assert_eq!(diagnostic.line, line);
        let message = diagnostic.message.as_str();
        let has_actual = message.contains(actual_value);
        assert!(has_actual, "{field}: {actual_value} | {message}");
        for allowed_value in allowed_values {
            let has_allowed = message.contains(*allowed_value);
            assert!(has_allowed, "{field}: {allowed_value} | {message}");
        }
    }

    #[test]
    fn canonical_enum_diagnostic_reports_actual_value_and_full_allowed_set() {
        let mut diagnostics = Vec::new();
        let policy = parse_compatibility_policy("maybe", &mut diagnostics, 11);
        let route = parse_blocker_route("teleport", &mut diagnostics, 12);
        let evidence = parse_evidence_kind("vibes", &mut diagnostics, 13);
        assert!(policy.is_none());
        assert!(route.is_none());
        assert!(evidence.is_none());
        assert_eq!(diagnostics.len(), 3);
        assert_enum_diagnostic(
            &diagnostics[0],
            "compatibility_policy",
            "maybe",
            11,
            &allowed_values(COMPATIBILITY_POLICY_CASES),
        );
        assert_enum_diagnostic(
            &diagnostics[1],
            "route",
            "teleport",
            12,
            &allowed_values(BLOCKER_ROUTE_CASES),
        );
        assert_enum_diagnostic(
            &diagnostics[2],
            "required_evidence",
            "vibes",
            13,
            &allowed_values(REQUIRED_EVIDENCE_CASES),
        );
    }

    #[test]
    fn canonical_enum_legal_values_parse_to_original_enum_without_diagnostics() {
        let mut diagnostics = Vec::new();
        assert_eq!(
            parse_compatibility_policy("require_all", &mut diagnostics, 21),
            Some(ContractCompatibilityPolicy::RequireAll)
        );
        assert_eq!(
            parse_compatibility_policy("require_any", &mut diagnostics, 22),
            Some(ContractCompatibilityPolicy::RequireAny)
        );
        assert_eq!(
            parse_blocker_route("operational_gate", &mut diagnostics, 23),
            Some(BlockerRoute::OperationalGate)
        );
        assert_eq!(
            parse_evidence_kind("non_zero_test_execution", &mut diagnostics, 24),
            Some(EvidenceKind::NonZeroTestExecution)
        );
        assert!(diagnostics.is_empty());
    }

    #[test]
    fn canonical_enum_allowed_set_matches_serde_canonical_spellings() {
        // 合法值表与 serde 的 canonical snake_case 拼写逐字一致，且每个
        // 合法值都经同一次表查解析出对应枚举值——合法集与解析表同源，
        // 不存在第二份手写字符串。
        let compatibility = [
            ContractCompatibilityPolicy::RequireAll,
            ContractCompatibilityPolicy::RequireAny,
        ];
        let routes = [
            BlockerRoute::CoderRework,
            BlockerRoute::VerificationRetry,
            BlockerRoute::PlanRepairCurrent,
            BlockerRoute::PlanRepairUpstream,
            BlockerRoute::SubgraphReplan,
            BlockerRoute::StoryAmendment,
            BlockerRoute::DesignAmendment,
            BlockerRoute::OperationalGate,
        ];
        let evidences = [
            EvidenceKind::SourceDiff,
            EvidenceKind::NonZeroTestExecution,
            EvidenceKind::ManualCheck,
            EvidenceKind::HandoffField,
        ];

        let compatibility_values = canonical_snake_cases(&compatibility);
        let route_values = canonical_snake_cases(&routes);
        let evidence_values = canonical_snake_cases(&evidences);
        assert_allowed_values(COMPATIBILITY_POLICY_CASES, &compatibility_values);
        assert_allowed_values(BLOCKER_ROUTE_CASES, &route_values);
        assert_allowed_values(REQUIRED_EVIDENCE_CASES, &evidence_values);

        let mut diagnostics = Vec::new();
        for (value, variant) in compatibility_values.iter().zip(compatibility.iter()) {
            let parsed = parse_compatibility_policy(value, &mut diagnostics, 31);
            assert_eq!(parsed, Some((*variant).clone()));
        }
        for (value, variant) in route_values.iter().zip(routes.iter()) {
            let parsed = parse_blocker_route(value, &mut diagnostics, 32);
            assert_eq!(parsed, Some((*variant).clone()));
        }
        for (value, variant) in evidence_values.iter().zip(evidences.iter()) {
            let parsed = parse_evidence_kind(value, &mut diagnostics, 33);
            assert_eq!(parsed, Some((*variant).clone()));
        }
        assert!(diagnostics.is_empty());
    }
}
