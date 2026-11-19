// `failure_copy` 新增失败码 / 告警码 / 封套解析入口的红测试片段。
//
// 由 `failure_copy.rs` 的既有 `mod tests` 以 `include!` 引入，因此用例全路径
// 仍是 `product::logical_codebase::failure_copy::tests::<fn>`，
// 过滤器仍是 `cargo test --locked --lib failure_copy`。
//
// 本文件只写断言，不写中文成品：中文文案的唯一来源是
// `failure_copy_table.inc.rs` 的码表。

/// 新失败码 `aggregate_index_member_source_excluded`：命中具体模板，
/// 三段非空、分诊是环境问题、标题不含机器码、动作指向「把源码合入主 checkout」。
#[test]
fn source_excluded_code_hits_concrete_template() {
    let copy = describe_code(
        "aggregate_index_member_source_excluded",
        Some(
            "source exists only under .worktrees/ for included members [b]; \
             move it into the member main checkout",
        ),
    );
    assert!(!copy.title_zh.trim().is_empty(), "缺问题");
    assert!(!copy.reason_zh.trim().is_empty(), "缺原因");
    assert!(!copy.action_zh.trim().is_empty(), "缺怎么做");
    assert_eq!(copy.triage, FailureTriage::Environment);
    assert!(
        !contains_machine_code(&copy.title_zh),
        "标题里出现了机器码：{}",
        copy.title_zh
    );
    assert!(copy.action_zh.contains("合入"), "{}", copy.action_zh);
    assert_eq!(copy.machine_line, "aggregate_index_member_source_excluded");
}

/// 新失败码 `aggregate_index_source_scan_incomplete`：说的是「无法确认」，
/// 不是「没有源码」；动作指向主检出的可读性。
#[test]
fn scan_incomplete_code_says_unconfirmed_not_absent() {
    let copy = describe_code(
        "aggregate_index_source_scan_incomplete",
        Some("source scan incomplete for included members [b]: permission denied"),
    );
    assert!(copy.reason_zh.contains("不等于"), "{}", copy.reason_zh);
    assert!(
        !copy.reason_zh.contains("没有源码"),
        "盘检未完成不得宣称该成员没有源码：{}",
        copy.reason_zh
    );
    assert!(copy.action_zh.contains("可读性"), "{}", copy.action_zh);
    assert_eq!(copy.triage, FailureTriage::Environment);
    assert_eq!(copy.machine_line, "aggregate_index_source_scan_incomplete");
}

#[test]
fn source_scan_copies_distinguish_measured_limit_read_error_and_legacy_mixed_facts() {
    let code = "aggregate_index_source_scan_incomplete";
    for (fact, expected) in [
        ("source scan limit reached; for included members [a]", "盘检达到上限"),
        ("source scan read incomplete; for included members [a]", "未能完整读取"),
        ("coverage state unknown for included members [a]; no indexed files and no disk scan evidence", "缺少覆盖结果"),
        ("source scan incomplete (permission/io/depth/entry/time limit) for included members [a]", "没能确认"),
    ] {
        let copy = describe(FailureInput::new(code).with_detail(fact).with_subject("a"));
        assert!(copy.title_zh.contains(expected), "{fact}: {}", copy.title_zh);
        assert!(copy.title_zh.contains('a'));
        assert_eq!(copy.machine_line, code);
    }
}

/// 新失败码 `aggregate_index_all_members_source_less`：全员确认无源码 ⇒ 索引不可激活；
/// 动作必须给出「加入含源码的成员」这一步。
#[test]
fn all_members_source_less_code_hits_concrete_template() {
    let copy = describe_code(
        "aggregate_index_all_members_source_less",
        Some("all included members [a, b] have no source"),
    );
    assert!(!copy.title_zh.trim().is_empty(), "缺问题");
    assert!(!copy.reason_zh.trim().is_empty(), "缺原因");
    assert!(!copy.action_zh.trim().is_empty(), "缺怎么做");
    assert_eq!(copy.triage, FailureTriage::Environment);
    assert!(
        copy.action_zh.contains("含源码的成员"),
        "{}",
        copy.action_zh
    );
    assert_eq!(copy.machine_line, "aggregate_index_all_members_source_less");
}

/// 第六条布局规则覆盖四个失败码；告警码不得混进失败码集合。
#[test]
fn layout_rules_sixth_entry_covers_the_four_failure_codes() {
    let rule = LAYOUT_RULES
        .iter()
        .find(|rule| rule.rule_zh == "聚合索引至少需要一个已覆盖成员；确认无源码的成员可以跳过")
        .expect("第六条布局规则必须存在");
    for code in [
        "aggregate_index_member_coverage_failed",
        "aggregate_index_member_source_excluded",
        "aggregate_index_all_members_source_less",
        "aggregate_index_source_scan_incomplete",
    ] {
        assert!(rule.codes.contains(&code), "第六条未覆盖 {code}");
    }
    assert!(
        !rule.codes.contains(&"aggregate_index_member_source_less"),
        "告警码不得混进失败码集合"
    );
}

/// 第六条规则依据逐字使用新措辞；四个失败码的原因段都必须带这句依据。
#[test]
fn layout_rule_basis_sentence_uses_new_wording() {
    const NEW_RULE: &str = "聚合索引至少需要一个已覆盖成员；确认无源码的成员可以跳过";
    let rule = LAYOUT_RULES
        .iter()
        .find(|rule| rule.rule_zh == NEW_RULE)
        .expect("第六条布局规则必须逐字使用新措辞");
    for code in rule.codes {
        let copy = describe_code(code, None);
        assert!(
            copy.reason_zh.contains(NEW_RULE),
            "{code} 未给出第六条规则依据：{}",
            copy.reason_zh
        );
    }
}

/// 新事实前缀（磁盘上有源码但没收录）⇒ 选「有源码但没被收录」变体，
/// `machine_line` 仍写原始稳定码（内部变体键不得外泄）。
#[test]
fn coverage_failure_prefers_disk_source_variant_for_new_fact() {
    let copy = describe_code(
        "aggregate_index_member_coverage_failed",
        Some(
            "source found on disk; no indexed files under included members [b]; \
             check member paths and ignore rules, then rebuild",
        ),
    );
    assert!(copy.title_zh.contains("有源码"), "{}", copy.title_zh);
    assert_eq!(copy.machine_line, "aggregate_index_member_coverage_failed");
    assert!(!copy.machine_line.contains(".disk_source"));
}

/// 旧事实前缀 ⇒ 沿用既有成品：事实句与动作句逐字不变，且不得编造磁盘证据。
#[test]
fn coverage_failure_keeps_legacy_variant_for_old_fact() {
    let copy = describe_code(
        "aggregate_index_member_coverage_failed",
        Some(
            "no indexable code file under included members [b]; \
             each included member needs at least 1 indexable code file",
        ),
    );
    assert_eq!(copy.title_zh, "成员仓里没有可索引的代码文件");
    assert!(
        copy.reason_zh
            .contains("被纳入的成员下找不到可索引的源码文件，索引会得到空结果。"),
        "{}",
        copy.reason_zh
    );
    assert_eq!(
        copy.action_zh,
        "把真实源码放进成员仓（.js/.ts/.py/.go/.rs 等）；.html/.json/.md 不算源码；\
         只放在 .worktrees/ 下的代码同样不算。放好后点「重试」。"
    );
    assert!(!copy.reason_zh.contains("磁盘"), "{}", copy.reason_zh);
}

/// 变体 key 只用于查表：归因分支仍收 canonical 码（布局码 + 已确认的
/// `store_*` 成因仍判为产品缺陷），`machine_line` 不带内部变体键。
#[test]
fn coverage_variant_does_not_replace_canonical_key_in_attribution() {
    let copy = describe_code(
        "aggregate_index_member_coverage_failed",
        Some("source found on disk; no indexed files under included members [b]"),
    );
    assert!(
        copy.reason_zh
            .contains("聚合索引至少需要一个已覆盖成员；确认无源码的成员可以跳过"),
        "{}",
        copy.reason_zh
    );
    assert!(!copy.machine_line.contains(".disk_source"));
    let attributed = describe(
        FailureInput::new("aggregate_index_member_coverage_failed")
            .with_detail("source found on disk; no indexed files under included members [b]")
            .with_cause("store_unreadable"),
    );
    assert_eq!(attributed.triage, FailureTriage::ProductDefect);
    assert!(
        attributed.reason_zh.contains("检查过程本身失败"),
        "{}",
        attributed.reason_zh
    );
}

/// 四个聚合索引失败码都能从机器事实里点名成员（成员清单形态）。
#[test]
fn subject_from_message_names_members_for_all_four_codes() {
    for code in [
        "aggregate_index_member_coverage_failed",
        "aggregate_index_member_source_excluded",
        "aggregate_index_source_scan_incomplete",
        "aggregate_index_all_members_source_less",
    ] {
        let subject = subject_from_message(code, "included members [a, b]; whatever")
            .unwrap_or_else(|| panic!("{code} 必须能点名成员"));
        assert_eq!(subject, "a、b", "{code} 点名错误");
    }
}

/// 告警成品四种形态同源登记：逐仓标注 / 卡片一行 / 降级历史 / 证据说明。
#[test]
fn member_source_less_copy_variants_render_from_one_block() {
    let member = member_source_less_member_copy("gray-config");
    assert!(member.contains("gray-config"), "{member}");
    assert!(member.contains("未纳入代码搜索"), "{member}");
    let summary = member_source_less_summary_copy(&["a", "b"]);
    assert!(summary.contains("已跳过无源码成员"), "{summary}");
    assert!(summary.contains("a、b"), "{summary}");
    let history = member_source_less_history_copy(&["a", "b"]);
    assert!(history.contains("上次可用索引"), "{history}");
    assert!(history.contains("a、b"), "{history}");
    let evidence = member_source_less_evidence_copy("gray-config");
    assert!(evidence.contains("未检索聚合索引"), "{evidence}");
    assert!(evidence.contains("gray-config"), "{evidence}");
}

/// 告警是结构化对象 `{code, message_zh}`：码恒定、说明非空；降级态用历史措辞。
#[test]
fn member_source_less_alert_is_structured_with_stable_code() {
    let alert = member_source_less_alert(&["a"], false);
    assert_eq!(alert.code, "aggregate_index_member_source_less");
    assert!(!alert.message_zh.trim().is_empty());
    let degraded = member_source_less_alert(&["a"], true);
    assert_eq!(degraded.code, "aggregate_index_member_source_less");
    assert!(
        degraded.message_zh.contains("上次可用索引"),
        "{}",
        degraded.message_zh
    );
}

/// 告警码不经 `FailureCopy` 五字段：它不在失败码表里，也不进布局规则表。
#[test]
fn alert_code_is_not_a_failure_copy() {
    let copy = describe_code("aggregate_index_member_source_less", None);
    assert_eq!(copy.machine_line, "aggregate_index_member_source_less");
    assert!(
        !copy.follows_layout_rule(),
        "告警码不得命中布局失败面：{}",
        copy.reason_zh
    );
    assert!(
        copy.reason_zh.contains("无法确定具体原因"),
        "告警码应走中性兜底：{}",
        copy.reason_zh
    );
    assert!(
        !LAYOUT_RULES
            .iter()
            .any(|rule| rule.codes.contains(&"aggregate_index_member_source_less")),
        "告警码不得出现在布局规则表里"
    );
}

/// 失败封套 → 内层稳定码 + 原始事实。
#[test]
fn parse_failure_envelope_extracts_inner_code_and_fact() {
    let (code, message, degraded) = parse_failure_envelope(
        "aggregate_index_failed:aggregate_index_member_source_excluded: \
         source exists only under .worktrees/ for included members [b]; \
         move it into the member main checkout",
    )
    .expect("失败封套必须能解析");
    assert_eq!(code, "aggregate_index_member_source_excluded");
    assert!(message.contains("included members [b]"), "{message}");
    assert!(!degraded);
}

/// 降级封套与失败封套必须能区分。
#[test]
fn parse_failure_envelope_distinguishes_degraded_envelope() {
    let (code, message, degraded) = parse_failure_envelope(
        "aggregate_index_degraded:aggregate_index_member_coverage_failed: \
         no indexable code file under included members [b]",
    )
    .expect("降级封套必须能解析");
    assert_eq!(code, "aggregate_index_member_coverage_failed");
    assert!(message.contains("included members [b]"), "{message}");
    assert!(degraded);
}

/// 解析不出内层码 ⇒ `None`（调用方回落到通用码），不得猜码。
#[test]
fn parse_failure_envelope_returns_none_for_unparsable_envelope() {
    assert!(parse_failure_envelope("").is_none());
    assert!(parse_failure_envelope("本次操作未完成").is_none());
    assert!(parse_failure_envelope("aggregate_index_failed:").is_none());
    assert!(parse_failure_envelope("aggregate_index_failed:  : x").is_none());
}

/// 共用封套解析入口必须保住成员点名（HTTP 出口与冷启动投影同一入口）。
#[test]
fn describe_envelope_keeps_member_naming_from_the_envelope_fact() {
    let envelope = "aggregate_index_failed:aggregate_index_member_coverage_failed: \
                    no indexable code file under included members [gray-config, msha-sdk]; \
                    each included member needs at least 1 indexable code file";
    let (code, copy) = describe_envelope(envelope).expect("失败封套必须能出成品");
    assert_eq!(code, "aggregate_index_member_coverage_failed");
    assert!(copy.reason_zh.contains("gray-config"), "{}", copy.reason_zh);
    assert!(copy.reason_zh.contains("msha-sdk"), "{}", copy.reason_zh);
    assert_eq!(copy.machine_line, "aggregate_index_member_coverage_failed");
    // 对照断言：不带 subject 的 `describe_code` 路径确实不带点名，
    // 证明点名来自 `describe_fact` / `describe_envelope` 这条链。
    let plain = describe_code(
        "aggregate_index_member_coverage_failed",
        Some(
            "no indexable code file under included members [gray-config, msha-sdk]; \
             each included member needs at least 1 indexable code file",
        ),
    );
    assert!(
        !plain.reason_zh.contains("gray-config"),
        "{}",
        plain.reason_zh
    );
    assert!(!plain.reason_zh.contains("msha-sdk"), "{}", plain.reason_zh);
}

/// 多个可读世代并存（`aggregate_index_active_ambiguous`）：成因是**连续重建失败**
/// 留下的多条同状态记录，不是「内部状态异常」；处置是修好失败原因后成功重建一次。
#[test]
fn active_ambiguous_code_blames_failed_rebuilds_not_internal_corruption() {
    let copy = describe_code("aggregate_index_active_ambiguous", None);
    assert!(
        copy.title_zh.contains("可读索引世代"),
        "标题应点名可读世代：{}",
        copy.title_zh
    );
    assert!(
        !copy.title_zh.contains("多个活跃聚合索引"),
        "标题不得沿用旧措辞：{}",
        copy.title_zh
    );
    assert!(
        copy.reason_zh.contains("连续重建失败"),
        "原因应点名连续重建失败：{}",
        copy.reason_zh
    );
    assert!(
        !copy.reason_zh.contains("内部状态异常"),
        "原因不得沿用「内部状态异常」：{}",
        copy.reason_zh
    );
    assert!(
        copy.action_zh.contains("重建索引"),
        "动作应指向重建索引：{}",
        copy.action_zh
    );
    assert!(
        copy.action_zh.contains("不会自动收敛"),
        "动作应说明失败重建不会自动收敛：{}",
        copy.action_zh
    );
    assert!(
        copy.action_zh.contains("不要手工删除"),
        "动作应禁止手工删除记录：{}",
        copy.action_zh
    );
    assert_eq!(copy.triage, FailureTriage::Environment);
    assert_eq!(copy.machine_line, "aggregate_index_active_ambiguous");
}

/// 没有任何可读世代（`aggregate_index_active_missing`）：必须已登记，
/// 不得落进中性兜底。
#[test]
fn active_missing_code_is_registered_and_not_neutral_fallback() {
    let copy = describe_code("aggregate_index_active_missing", None);
    assert!(
        copy.title_zh.contains("没有可用的聚合索引"),
        "标题应说明当前没有可用索引：{}",
        copy.title_zh
    );
    assert!(
        copy.reason_zh.contains("可读的索引世代"),
        "原因应说明没有可读世代：{}",
        copy.reason_zh
    );
    assert!(
        !copy.reason_zh.contains("当前无法确定具体原因"),
        "已登记的码不得落进中性兜底原因：{}",
        copy.reason_zh
    );
    assert!(
        !copy.action_zh.contains("请重试；连续两次仍失败时"),
        "已登记的码不得落进中性兜底动作：{}",
        copy.action_zh
    );
    assert!(
        copy.action_zh.contains("重建索引"),
        "动作应指向重建索引：{}",
        copy.action_zh
    );
    assert_eq!(copy.triage, FailureTriage::Environment);
    assert_eq!(copy.machine_line, "aggregate_index_active_missing");
}

/// 这两个码都不是布局码：不得进布局规则表（十个布局码的集合与断言保持不变）。
#[test]
fn active_ambiguous_and_missing_are_not_layout_codes() {
    for code in [
        "aggregate_index_active_ambiguous",
        "aggregate_index_active_missing",
    ] {
        assert!(
            !LAYOUT_RULES.iter().any(|rule| rule.codes.contains(&code)),
            "{code} 不得出现在布局规则表里"
        );
        let copy = describe_code(code, None);
        assert!(
            !copy.follows_layout_rule(),
            "{code} 不得命中布局失败面：{}",
            copy.reason_zh
        );
    }
}
