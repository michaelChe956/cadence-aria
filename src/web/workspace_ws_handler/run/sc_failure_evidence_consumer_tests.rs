//! P09 runtime consumer 回归（真实 store 边界）。
//!
//! 独立子文件（模块本体 ≤1100 行 cap；测试整体外迁，不压行、不删边界）：
//! 声明于 `run/sc_failure_evidence.rs` 的
//! `#[cfg(test)] #[path = "sc_failure_evidence_consumer_tests.rs"] mod consumer_tests;`
//! 因此本文件内的 `super::` 仍是 `sc_failure_evidence`，`super::super::` 仍是
//! `single_candidate`——与内联时逐字等价。
//!
//! 边界：本模块只证明真实 store 的未截断读回与「写失败⇒零 FailureObserved 行」；
//! 端到端 gate 顺序（FailureObserved 先于 claim、判定先于 provider start）由
//! P27 真实 adapter 集成验收，不用恒定零计数冒充。
use super::super::sc_failure_transport::{ScBoundProviderRound, begin_sc_failure_round};
use crate::product::app_paths::ProductAppPaths;
use crate::product::lifecycle_store::LifecycleStore;
use crate::product::lifecycle_store::inputs::{
    CreateWorkspaceSessionInput, WorkItemPlanSessionOptions,
};
use crate::product::lifecycle_store::sc_failure_evidence::ScFailureEvidenceEvent;
use crate::product::models::{ProviderName, WorkspaceType};
use crate::product::work_item_plan_compiler::CompilerDiagnostic;
use crate::product::work_item_plan_policy::WorkItemPlanFlowKind;
use crate::web::workspace_ws_types::{ScEvidenceRef, ScFailureClass, ScFailureScope};

const PROJECT_ID: &str = "project_0001";
const ISSUE_ID: &str = "issue_0001";
const PLAN_ID: &str = "plan_0001";
const NODE_ID: &str = "node_sc_0001";
const BUSINESS_KEY: &str = "workspace_author:session:0";

/// SC single-candidate WorkItemPlan session（store 准入要求
/// flow_kind == SingleCandidate 且 scope.plan_id == session.entity_id）。
fn fixture() -> (tempfile::TempDir, LifecycleStore, ScFailureScope) {
    let tmp = tempfile::tempdir().expect("temp root");
    let lifecycle = LifecycleStore::new(ProductAppPaths::new(tmp.path().join(".aria")));
    let record = lifecycle
        .create_workspace_session(CreateWorkspaceSessionInput {
            project_id: PROJECT_ID.into(),
            issue_id: ISSUE_ID.into(),
            entity_id: PLAN_ID.into(),
            workspace_type: WorkspaceType::WorkItemPlan,
            author_provider: ProviderName::Fake,
            reviewer_provider: Some(ProviderName::Fake),
            review_rounds: 1,
            superpowers_enabled: false,
            openspec_enabled: false,
            work_item_plan_options: Some(WorkItemPlanSessionOptions {
                flow_kind: WorkItemPlanFlowKind::SingleCandidate,
                ..Default::default()
            }),
        })
        .expect("create SC session");
    let scope = ScFailureScope {
        session_id: record.id,
        project_id: PROJECT_ID.into(),
        issue_id: ISSUE_ID.into(),
        plan_id: PLAN_ID.into(),
    };
    (tmp, lifecycle, scope)
}

fn bind_round(lifecycle: &LifecycleStore, scope: &ScFailureScope) -> ScBoundProviderRound {
    begin_sc_failure_round(
        lifecycle,
        scope,
        BUSINESS_KEY,
        None,
        NODE_ID,
        &ProviderName::Fake,
        0,
    )
    .expect("round binds a real handle")
}

/// 独立摘要（绝不复用生产 `sc_sha256` 作期望值，避免同错自证）。
fn independent_sha256(text: &str) -> String {
    use sha2::Digest as _;
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}

fn diagnostic(code: &str, line: usize, message: &str) -> CompilerDiagnostic {
    CompilerDiagnostic {
        code: code.to_string(),
        line,
        field: format!("{code}.field"),
        message: message.to_string(),
        repair_example: format!("{code}=example"),
        kind: None,
        lowering_fault: None,
        ir_fault: None,
    }
}

/// ① 真实 store 完整读回：FailureObserved 一轮写读即返回**未截断**事实
/// （诊断原文顺序、6 条指令逐字、source/raw 各自 ref 且同 locator），
/// 且 RoundBound 与本轮失败事实同链保留、读回行逐字等于返回行。
#[test]
fn observe_sc_failure_persists_full_untruncated_facts_in_real_store() {
    let (_tmp, lifecycle, scope) = fixture();
    let session_id = scope.session_id.clone();
    let bound = bind_round(&lifecycle, &scope);

    let raw_body = "raw provider output (verbatim)";
    let raw = ScEvidenceRef::Available {
        locator: format!("sc-split-output/v1/{session_id}/0"),
        sha256: independent_sha256(raw_body),
        bytes: raw_body.len() as u64,
    };
    let compile_source = "# plan\nnormalized compile source (verbatim)\n";
    let diagnostics = vec![
        diagnostic("missing_section", 0, "first reason"),
        diagnostic("unknown_structured_key", 12, "second reason"),
    ];
    let instructions: Vec<String> = (1..=6)
        .map(|n| format!("instruction-{n}: keep verbatim"))
        .collect();

    let facts = super::compile_failure_facts(
        &diagnostics,
        instructions.clone(),
        &raw,
        compile_source,
        &bound.round,
    );
    let observed = super::observe_sc_failure(&lifecycle, &bound.chain, &bound.round, facts)
        .expect("FailureObserved appends and strictly reads back");

    let ScFailureEvidenceEvent::FailureObserved {
        round_id,
        failure_class,
        blocking_reasons,
        repair_instructions,
        raw: read_raw,
        source: read_source,
        existing_failure_id,
        attempt_key,
    } = &observed.entry.event
    else {
        unreachable!("observe_sc_failure only appends FailureObserved rows");
    };
    assert_eq!(round_id.as_str(), bound.round.round_id.as_str());
    assert_eq!(failure_class.as_ref(), Some(&ScFailureClass::Compile));
    assert_eq!(blocking_reasons.len(), diagnostics.len() + instructions.len());
    assert_eq!(blocking_reasons[0], "missing_section:0:first reason");
    assert_eq!(blocking_reasons[1], "unknown_structured_key:12:second reason");
    assert_eq!(
        &blocking_reasons[diagnostics.len()..],
        instructions.as_slice(),
        "指令全集必须逐字、按序、不截断"
    );
    assert_eq!(repair_instructions.as_slice(), instructions.as_slice());
    assert_eq!(read_raw.clone(), raw.clone(), "raw 必须是本轮真实引用");
    let expected_source = ScEvidenceRef::Available {
        locator: format!("sc-split-output/v1/{session_id}/0"),
        sha256: independent_sha256(compile_source),
        bytes: compile_source.len() as u64,
    };
    assert_eq!(
        read_source.clone(),
        expected_source,
        "source 与 raw 同 locator、摘要/字节数取归一化正文真值"
    );
    assert!(existing_failure_id.is_none());
    assert!(attempt_key.is_none());

    let read = lifecycle
        .read_sc_failure_evidence(&bound.chain)
        .expect("strict evidence log readable");
    assert!(
        read.entries.iter().any(|entry| {
            matches!(&entry.event, ScFailureEvidenceEvent::RoundBound { round } if round == &bound.round)
        }),
        "本轮绑定事实必须与失败事实同链保留"
    );
    let stored = read
        .entries
        .iter()
        .find(|entry| entry.entry_key == super::failure_observed_key(&bound.round.round_id))
        .expect("FailureObserved row is durable");
    assert_eq!(stored.event, observed.entry.event);
}

/// ② 真实写准入失败（chain 身份与四元组不一致）：稳定码/step 上浮，
/// 且真实日志零 FailureObserved（不留半截失败事实）。
#[test]
fn evidence_write_failure_leaves_no_failure_observed_rows() {
    let (_tmp, lifecycle, scope) = fixture();
    let session_id = scope.session_id.clone();
    let bound = bind_round(&lifecycle, &scope);

    // 合法 64 位小写 hex，但与本轮身份四元组不一致 ⇒ 真实 store 写准入失败。
    let mut tampered = bound.chain.clone();
    tampered.chain_id = format!("chain-v1-{}", "0".repeat(64));

    let raw = ScEvidenceRef::Available {
        locator: format!("sc-split-output/v1/{session_id}/0"),
        sha256: independent_sha256("raw body"),
        bytes: "raw body".len() as u64,
    };
    let facts = super::compile_failure_facts(
        &[diagnostic("missing_section", 0, "first reason")],
        vec!["instruction-1: keep verbatim".to_string()],
        &raw,
        "compile source",
        &bound.round,
    );
    let error = super::observe_sc_failure(&lifecycle, &tampered, &bound.round, facts)
        .expect_err("chain 身份不一致时证据写准入必须失败");
    assert_eq!(error.code, "sc_failure_chain_invalid");
    assert_eq!(error.step, "append_sc_failure_evidence");

    let read = lifecycle
        .read_sc_failure_evidence(&bound.chain)
        .expect("strict evidence log readable");
    assert!(
        !read.entries.iter().any(|entry| matches!(
            &entry.event,
            ScFailureEvidenceEvent::FailureObserved { .. }
        )),
        "写失败不得留下半截失败事实"
    );
}
