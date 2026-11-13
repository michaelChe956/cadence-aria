// A4 载荷（SC 失败修复包）扁平化往返用例（2026-11-13）。
// 落位：src/web/workspace_ws_types/tests/artifact_version_sc_failure_roundtrip.rs
// 挂载：src/web/workspace_ws_types/tests.rs
//   mod artifact_version_sc_failure_roundtrip;
//
// 容量片：原 artifact_version_roundtrip.rs 承载既有 3 条往返用例 + 本文件新增的
// A4 用例后为 462 行（> 每 test 文件 400 行上限）→ A4 用例独立成文件，既有
// 往返用例原样留在原文件（不改断言）。
//
// 契约：A4 载荷在扁平化的 ArtifactVersion 上逐字段往返——wire 形状是版本对象上的
// `sc_failure_repair_package` 键（没有 "payload" 包装层），读回后版本号 + 作用域 +
// 全部轮次引用 + 证据引用逐字不变（不是长度、不是摘要、不是序列化回声）；既有
// markdown 变体不受 A4 变体影响。

use super::*;

use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureClass, ScFailureRepairPackage, ScFailureSourceEvidence,
    ScPackageCompleteness, ScPromptDisposition, ScRoundEvidence, ScRoundIdentity,
    ScRoundLaunchState, ScTeachingDecision, ScTeachingPromptEvidence,
};

#[test]
fn artifact_version_roundtrips_with_flattened_sc_failure_repair_package() {
    let package = sc_failure_repair_package_fixture();
    package
        .validate()
        .expect("fixture package must satisfy the frozen package invariants");

    let version = ArtifactVersion {
        version: 2,
        payload: ArtifactPayload::ScFailureRepairPackage {
            sc_failure_repair_package: Box::new(package.clone()),
        },
        generated_by: ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: false,
        created_at: "2026-11-13T00:00:00Z".to_string(),
        source_node_id: "node_sc_0001".to_string(),
    };

    let json = serde_json::to_value(&version).unwrap();
    assert!(
        !json.as_object().unwrap().contains_key("payload"),
        "A4 载荷必须扁平在版本对象上，不得有 payload 包装层"
    );
    let wire = &json["sc_failure_repair_package"];
    assert_eq!(
        wire["package_id"].as_str(),
        Some(package.package_id.as_str())
    );
    assert_eq!(wire["chain_id"].as_str(), Some(package.chain_id.as_str()));
    assert_eq!(
        wire["session_id"].as_str(),
        Some(package.session_id.as_str())
    );
    assert_eq!(
        wire["project_id"].as_str(),
        Some(package.project_id.as_str())
    );
    assert_eq!(wire["issue_id"].as_str(), Some(package.issue_id.as_str()));
    assert_eq!(wire["plan_id"].as_str(), Some(package.plan_id.as_str()));
    assert_eq!(
        wire["evidence_digest"].as_str(),
        Some(package.evidence_digest.as_str())
    );

    let back: ArtifactVersion = serde_json::from_value(json).unwrap();
    assert_eq!(back, version, "A4 版本必须逐字段往返");

    // 消费者读到的字段：版本号 + 作用域 + 全部轮次引用 + 证据引用。
    let ArtifactPayload::ScFailureRepairPackage {
        sc_failure_repair_package,
    } = &back.payload
    else {
        panic!("A4 版本必须读回为 SC 失败修复包载荷");
    };
    assert_eq!(back.version, 2);
    assert_eq!(sc_failure_repair_package.package_id, package.package_id);
    assert_eq!(sc_failure_repair_package.chain_id, package.chain_id);
    assert_eq!(sc_failure_repair_package.session_id, package.session_id);
    assert_eq!(sc_failure_repair_package.project_id, package.project_id);
    assert_eq!(sc_failure_repair_package.issue_id, package.issue_id);
    assert_eq!(sc_failure_repair_package.plan_id, package.plan_id);
    assert_eq!(
        sc_failure_repair_package.terminal_class,
        package.terminal_class
    );
    assert_eq!(
        sc_failure_repair_package.terminal_reason,
        package.terminal_reason
    );
    assert_eq!(sc_failure_repair_package.completeness, package.completeness);
    assert_eq!(
        sc_failure_repair_package.evidence_log_ref,
        package.evidence_log_ref
    );
    assert_eq!(
        sc_failure_repair_package.evidence_digest,
        package.evidence_digest
    );
    assert_eq!(
        sc_failure_repair_package.evidence_through_entry_key,
        package.evidence_through_entry_key
    );
    assert_eq!(
        sc_failure_repair_package.last_provider_output,
        package.last_provider_output
    );
    assert_eq!(
        sc_failure_repair_package.last_failure_source,
        package.last_failure_source
    );
    assert_eq!(
        sc_failure_repair_package.teaching_prompts,
        package.teaching_prompts
    );
    assert_eq!(
        sc_failure_repair_package.missing_evidence,
        package.missing_evidence
    );
    assert_eq!(
        sc_failure_repair_package.rounds.len(),
        package.rounds.len(),
        "轮次数必须逐条保留"
    );
    for (round, expected) in sc_failure_repair_package
        .rounds
        .iter()
        .zip(package.rounds.iter())
    {
        assert_eq!(round.identity.round_id, expected.identity.round_id);
        assert_eq!(round.identity.run_ref, expected.identity.run_ref);
        assert_eq!(round.identity.node_id, expected.identity.node_id);
        assert_eq!(round.identity.chain_id, expected.identity.chain_id);
        assert_eq!(round.identity.role_run_seq, expected.identity.role_run_seq);
        assert_eq!(
            round.identity.workspace_session_id,
            expected.identity.workspace_session_id
        );
        assert_eq!(
            round.identity.round_ordinal,
            expected.identity.round_ordinal
        );
        assert_eq!(round.identity.provider, expected.identity.provider);
        assert_eq!(round.launch_state, expected.launch_state);
        assert_eq!(round.failure_class, expected.failure_class);
        assert_eq!(round.blocking_reasons, expected.blocking_reasons);
        assert_eq!(round.repair_instructions, expected.repair_instructions);
        assert_eq!(round.raw, expected.raw);
        assert_eq!(round.source, expected.source);
        assert_eq!(round.existing_failure_id, expected.existing_failure_id);
        assert_eq!(round.attempt_key, expected.attempt_key);
        assert_eq!(round.prompts, expected.prompts);
        assert_eq!(round.teaching_decision, expected.teaching_decision);
    }

    // 既有 markdown 变体不受 A4 变体影响（untagged 顺序不得吞掉 markdown）。
    let markdown_version = ArtifactVersion {
        version: 1,
        payload: ArtifactPayload::Markdown {
            markdown: "# Artifact version\n".to_string(),
            diff: Some("diff".to_string()),
        },
        generated_by: ProviderName::Codex,
        reviewed_by: None,
        review_verdict: None,
        confirmed_by: None,
        is_current: true,
        created_at: "2026-11-13T00:00:00Z".to_string(),
        source_node_id: "node_sc_0001".to_string(),
    };
    let markdown_json = serde_json::to_value(&markdown_version).unwrap();
    assert_eq!(markdown_json["markdown"], "# Artifact version\n");
    let markdown_back: ArtifactVersion = serde_json::from_value(markdown_json).unwrap();
    assert_eq!(markdown_back, markdown_version);
    assert!(
        matches!(markdown_back.payload, ArtifactPayload::Markdown { .. }),
        "markdown 变体必须仍是 markdown 变体"
    );
}

/// A4 载荷 fixture：完整、合法（过真实包不变量校验）的失败修复包——两轮真实引用
/// （raw/source/教学提示都带真实 SHA-256 摘要与字节数）。
fn sc_failure_repair_package_fixture() -> ScFailureRepairPackage {
    fn sha256_hex(content: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hasher
            .finalize()
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect()
    }
    fn available(locator: &str, content: &str) -> ScEvidenceRef {
        ScEvidenceRef::Available {
            locator: locator.to_string(),
            sha256: sha256_hex(content),
            bytes: content.as_bytes().len() as u64,
        }
    }
    fn round_identity(ordinal: u32) -> ScRoundIdentity {
        ScRoundIdentity {
            chain_id: "chain-sc-0001".to_string(),
            round_id: format!("round-sc-{ordinal:04}"),
            node_id: format!("node-sc-{ordinal:04}"),
            round_ordinal: ordinal,
            provider: ProviderName::Codex,
            workspace_session_id: "session-sc-0001".to_string(),
            role_run_seq: u64::from(ordinal),
            run_ref: format!("run-sc-{ordinal:04}"),
        }
    }
    fn round_evidence(ordinal: u32) -> ScRoundEvidence {
        let raw_body = format!("provider raw body for round {ordinal}");
        let source_body = format!("normalized source body for round {ordinal}");
        let prompt_body = format!("teaching prompt body for round {ordinal}");
        let identity = round_identity(ordinal);
        ScRoundEvidence {
            identity,
            launch_state: ScRoundLaunchState::Started,
            failure_class: Some(ScFailureClass::Compile),
            blocking_reasons: vec!["E0308:3: mismatched types".to_string()],
            repair_instructions: vec!["先修依赖顺序再修类型".to_string()],
            raw: available(
                &format!("work-item-split-runs/run-sc-{ordinal:04}/run.json"),
                &raw_body,
            ),
            source: available(
                &format!("work-item-split-runs/run-sc-{ordinal:04}/source.txt"),
                &source_body,
            ),
            existing_failure_id: Some(format!("failure-{ordinal:04}")),
            attempt_key: Some(format!("attempt-{ordinal:04}")),
            prompts: vec![ScTeachingPromptEvidence {
                failed_round_id: format!("round-sc-{ordinal:04}"),
                target_round_ordinal: ordinal + 1,
                builder_id: "teaching-source-block".to_string(),
                prompt: available(
                    &format!("teaching-prompts/prompt-sc-{ordinal:04}.txt"),
                    &prompt_body,
                ),
                disposition: ScPromptDisposition::Sent,
            }],
            teaching_decision: ScTeachingDecision::NotRequested,
        }
    }

    ScFailureRepairPackage {
        schema_version: 1,
        package_id: "package-sc-0001".to_string(),
        chain_id: "chain-sc-0001".to_string(),
        session_id: "session-sc-0001".to_string(),
        project_id: "project-sc-0001".to_string(),
        issue_id: "issue-sc-0001".to_string(),
        plan_id: "plan-sc-0001".to_string(),
        terminal_class: "compile".to_string(),
        terminal_reason: "E0308:3: mismatched types".to_string(),
        completeness: ScPackageCompleteness::Complete,
        rounds: vec![round_evidence(1), round_evidence(2)],
        last_provider_output: available(
            "work-item-split-runs/run-sc-0002/run.json",
            "provider raw body for round 2",
        ),
        last_failure_source: ScFailureSourceEvidence {
            source_round_id: "round-sc-0002".to_string(),
            source: available(
                "work-item-split-runs/run-sc-0002/source.txt",
                "normalized source body for round 2",
            ),
        },
        teaching_prompts: vec![ScTeachingPromptEvidence {
            failed_round_id: "round-sc-0001".to_string(),
            target_round_ordinal: 2,
            builder_id: "teaching-source-block".to_string(),
            prompt: available(
                "teaching-prompts/prompt-sc-0001.txt",
                "teaching prompt body for round 1",
            ),
            disposition: ScPromptDisposition::Sent,
        }],
        evidence_log_ref: "sc-failure-evidence/chain-sc-0001".to_string(),
        evidence_digest: sha256_hex("chain-sc-0001 evidence prefix"),
        evidence_through_entry_key: "chain-sc-0001/terminal".to_string(),
        missing_evidence: Vec::new(),
    }
}
