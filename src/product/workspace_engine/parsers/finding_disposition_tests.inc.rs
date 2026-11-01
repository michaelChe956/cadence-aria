    // ─── REQ-WSC-12/13：disposition 语义、平台硬规则与 finding_key 身份 ───

    /// REQ-WSC-12：`disposition` 缺失＝actionable（fail-safe）——历史记录
    /// 与旧 wire 因此仍可阻断，绝不因新字段缺席放宽。
    #[test]
    fn missing_disposition_counts_as_actionable() {
        let verdict = parse_review_value(
            &serde_json::json!({
                "verdict": "pass",
                "summary": "look",
                "findings": [{
                    "severity": "must_fix",
                    "message": "缺少失败路径",
                    "required_action": "补充失败路径"
                }]
            }),
            "",
        )
        .expect("legacy payload without disposition must parse");

        assert_eq!(verdict.findings[0].disposition, None);
        assert!(verdict.findings[0].is_actionable());
        // 缺失 disposition 的 must_fix 依旧把 pass 归一为返修（fail-safe）。
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
    }

    /// REQ-WSC-12 平台硬规则：`advisory` 即便被模型标成 must_fix 也不阻断
    /// ——判定在平台侧，不依赖模型自觉标对；owner 仍能看见该 finding。
    #[test]
    fn advisory_finding_never_blocks_even_when_severity_is_must_fix() {
        let verdict = parse_review_value(
            &serde_json::json!({
                "verdict": "pass",
                "summary": "look",
                "findings": [{
                    "severity": "must_fix",
                    "disposition": "advisory",
                    "message": "仓库里那份前端规范文档的『中文 locale 尚未配置』缺口需回写",
                    "required_action": "提示：该文档当前状态由流程侧回写"
                }]
            }),
            "",
        )
        .expect("advisory finding must parse");

        assert!(!verdict.findings[0].is_actionable());
        assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
        assert_eq!(verdict.verdict, ReviewVerdictType::Pass);
    }

    /// REQ-WSC-12：WorkItemPlan 单候选路径同款——advisory 不让 pass 重开为
    /// Revise（阻断判定的第二处落点）。
    #[test]
    fn advisory_finding_does_not_reopen_single_candidate_pass() {
        let verdict = parse_work_item_plan_review_value(
            &serde_json::json!({
                "verdict": "pass",
                "review_scope": "outline",
                "generation_round_id": "round_0001",
                "summary": "ok",
                "findings": [{
                    "severity": "blocking",
                    "disposition": "advisory",
                    "message": "其它文档的当前状态不是本 WI 缺口",
                    "required_action": "提示：不在本次交付范围"
                }]
            }),
            "",
            &[],
            WorkItemPlanReviewScope::Outline,
        )
        .expect("advisory finding must parse");

        let extension = verdict.work_item_plan_review.expect("extension");
        assert_eq!(extension.verdict, WorkItemPlanReviewVerdict::Pass);
        assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
    }

    /// REQ-WSC-12：非法 disposition 值走既有 `invalid_finding_field` 路径。
    #[test]
    fn invalid_disposition_value_is_invalid_finding_field() {
        let error = parse_review_envelope(&serde_json::json!({
            "verdict": "revise",
            "findings": [{
                "severity": "must_fix",
                "message": "bad disposition",
                "disposition": "maybe"
            }]
        }))
        .unwrap_err();

        assert_eq!(error.code(), "invalid_finding_field");
        assert_eq!(
            error,
            ReviewStructuredOutputError::InvalidFindingField {
                field: "findings[0].disposition".to_string(),
                details: "must be actionable or advisory".to_string(),
            }
        );
    }

    /// REQ-WSC-13：finding_key＝sha256(severity␁message) 前 16 位小写 hex，
    /// 且 live（解析产物）与 rebuild（durable verdict 反序列化）算出同一个。
    #[test]
    fn finding_key_is_content_fingerprint_and_stable_across_live_and_rebuild() {
        let payload = serde_json::json!({
            "verdict": "revise",
            "summary": "look",
            "findings": [{
                "severity": "must_fix",
                "message": "缺少失败路径",
                "evidence": "当前产物遗漏",
                "required_action": "补充失败路径"
            }]
        });
        let live = parse_review_value(&payload, "").expect("live parse");
        let live_key = live.findings[0].computed_finding_key();
        assert_eq!(live_key.len(), 16);
        assert!(
            crate::product::models::HumanDispositionRecord::is_valid_finding_key(&live_key),
            "finding_key must be 16 lowercase hex: {live_key}"
        );
        // wire 投影（live 事件构造点）带同一 key——前端「排除此项」与呈现共用。
        let event =
            crate::product::workspace_engine::session_state::review_complete_event_from_verdict(
                "node_0001".to_string(),
                1,
                &live,
            );
        let crate::product::workspace_engine::EngineEvent::ReviewComplete { findings, .. } = event
        else {
            panic!("expected review complete event");
        };
        assert_eq!(findings[0].finding_key.as_deref(), Some(live_key.as_str()));
        let wired = serde_json::json!({
            "verdict": "revise",
            "comments": "",
            "summary": "look",
            "findings": findings,
        });
        assert_eq!(
            wired["findings"][0]["finding_key"].as_str(),
            Some(live_key.as_str())
        );

        // rebuild：durable verdict（字段可缺失）反序列化后重算必须逐字相等。
        let mut durable = wired.clone();
        durable["findings"][0]
            .as_object_mut()
            .expect("finding object")
            .remove("finding_key");
        let rebuilt =
            crate::product::workspace_engine::parsers::deserialize_historical_review_verdict(
                durable,
            )
            .expect("historical deserialize");
        assert_eq!(
            rebuilt.findings[0].computed_finding_key(),
            live_key,
            "live and rebuild must agree on finding identity"
        );
        assert_eq!(
            rebuilt.findings[0].finding_key.as_deref(),
            Some(live_key.as_str())
        );
    }

    /// P0-1 加固（REQ-WSC-12 平台硬规则）：`contract_gap` / `self_contradiction`
    /// 是本 WI 可通过修订收敛的契约类缺口——模型把它们标成 advisory，平台
    /// 仍按 actionable 处理（不留「advisory 漏检后门」）。
    #[test]
    fn contract_gap_marked_advisory_is_still_actionable() {
        let verdict = parse_review_value(
            &serde_json::json!({
                "verdict": "pass",
                "summary": "look",
                "findings": [{
                    "severity": "must_fix",
                    "disposition": "advisory",
                    "category": "contract_gap",
                    "message": "本 WI 的能力契约缺少「非数字 id → 400」验收标准",
                    "required_action": "补充该验收标准"
                }]
            }),
            "",
        )
        .expect("contract gap must parse");

        assert!(verdict.findings[0].is_actionable());
        assert_eq!(verdict.review_gate, ReviewGate::RequiresRevision);
        assert_eq!(verdict.verdict, ReviewVerdictType::Revise);
    }

    /// P0-1 加固不放宽普通 advisory：非契约类（other/无 category）的 advisory
    /// 仍不阻断——否则「范围外事实报但降级」的裁定被反转。
    #[test]
    fn non_contract_advisory_stays_non_blocking() {
        let verdict = parse_review_value(
            &serde_json::json!({
                "verdict": "pass",
                "summary": "look",
                "findings": [{
                    "severity": "must_fix",
                    "disposition": "advisory",
                    "category": "other",
                    "message": "仓库里那份产品文档的当前状态",
                    "required_action": "提示：该状态由流程侧回写"
                }]
            }),
            "",
        )
        .expect("advisory must parse");

        assert!(!verdict.findings[0].is_actionable());
        assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
    }

    /// REQ-WSC-12 第三条：「revise 但无任何 actionable」不得驱作者返修——
    /// 归一在会话级完成（此处钉解析层的输入形态：全 advisory 的 revise 判
    /// 定仍可解析，归一由 `apply_session_owner_dispositions` 负责）。
    #[test]
    fn revise_payload_with_only_advisory_findings_parses() {
        let verdict = parse_review_value(
            &serde_json::json!({
                "verdict": "revise",
                "summary": "look",
                "findings": [{
                    "severity": "must_fix",
                    "disposition": "advisory",
                    "category": "other",
                    "message": "仅供参考",
                    "required_action": "提示"
                }]
            }),
            "",
        )
        .expect("revise with advisory-only findings must parse");

        assert!(!verdict.findings[0].is_actionable());
        // 解析层不驱返修（无 actionable blocking/must_fix）。
        assert_eq!(verdict.review_gate, ReviewGate::UserConfirmAllowed);
    }
