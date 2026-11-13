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

/// 本轮 split 产物的真实 locator——直接复用 store 的同一构造，不复制格式串
/// （store 校验 FailureObserved 的 raw/source locator 必须与之逐字相等）。
fn split_locator(bound: &ScBoundProviderRound) -> String {
    crate::product::lifecycle_store::sc_failure_evidence::sc_failure_split_locator(
        &bound.round.workspace_session_id,
        bound.round.role_run_seq,
    )
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
    let bound = bind_round(&lifecycle, &scope);

    let raw_body = "raw provider output (verbatim)";
    let raw = ScEvidenceRef::Available {
        locator: split_locator(&bound),
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
        locator: split_locator(&bound),
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
    let bound = bind_round(&lifecycle, &scope);

    // 合法 64 位小写 hex，但与本轮身份四元组不一致 ⇒ 真实 store 写准入失败。
    let mut tampered = bound.chain.clone();
    tampered.chain_id = format!("chain-v1-{}", "0".repeat(64));

    let raw = ScEvidenceRef::Available {
        locator: split_locator(&bound),
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


/// P10 终局 RED：`finalize_sc_failure_chain` / `ScFailureTerminalOutcome` /
/// `terminal_exit_error` 尚未实现 ⇒ 编译红。真实 store 边界，无 engine、无 provider。
///
/// 断言的是终局事实本身（class/身份/缺项/坏日志零 synthetic、Unknown 零字节），
/// 不是编译误差：每个用例都从真实 RoundBound + 生产 producer 的 FailureObserved 出发。
mod terminal {
    use super::super::super::sc_failure_transport::{ScBoundProviderRound, begin_sc_failure_round};
    use super::super::{
        SC_TERMINAL_CLASS_COMPILE, SC_TERMINAL_CLASS_IR_PREFLIGHT, SC_TERMINAL_CLASS_STRUCTURE_GATE,
        SC_TERMINAL_CLASS_TEACHING_INTERRUPTED, ScFailureTerminalOutcome, finalize_sc_failure_chain,
    };
    use crate::product::lifecycle_store::LifecycleStore;
    use crate::product::lifecycle_store::sc_failure_evidence::ScFailureEvidenceEvent;
    use crate::product::models::ProviderName;
    use crate::web::workspace_ws_types::{
        ScEvidenceRef, ScFailureScope, ScPackageCompleteness, ScRoundLaunchState,
    };

    const RAW_BODY: &str = "# Work Item Plan\n";
    const COMPILE_SOURCE: &str = "# plan\nnormalized compile source\n";

    /// 一轮真实物理绑定（教学轮沿用原链身份）。
    fn bind(
        lifecycle: &LifecycleStore,
        scope: &ScFailureScope,
        original: Option<&ScBoundProviderRound>,
        ordinal: u32,
    ) -> ScBoundProviderRound {
        begin_sc_failure_round(
            lifecycle,
            scope,
            super::BUSINESS_KEY,
            original.map(|bound| &bound.chain),
            super::NODE_ID,
            &ProviderName::Fake,
            ordinal,
        )
        .expect("bind round")
    }

    /// 本轮真实 raw 引用（locator 与 store 校验逐字相等）。
    fn available(bound: &ScBoundProviderRound) -> ScEvidenceRef {
        ScEvidenceRef::Available {
            locator: super::split_locator(bound),
            sha256: super::independent_sha256(RAW_BODY),
            bytes: RAW_BODY.len() as u64,
        }
    }

    /// 走生产 producer 落本轮真实机械失败（不手写事件）。
    fn observe_compile(lifecycle: &LifecycleStore, bound: &ScBoundProviderRound) {
        let facts = super::super::compile_failure_facts(
            &[super::diagnostic("missing_section", 0, "first reason")],
            Vec::new(),
            &available(bound),
            COMPILE_SOURCE,
            &bound.round,
        );
        super::super::observe_sc_failure(lifecycle, &bound.chain, &bound.round, facts)
            .expect("FailureObserved appends");
    }

    fn log_path(root: &std::path::Path) -> std::path::PathBuf {
        fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, out);
                } else if path.file_name().is_some_and(|name| name == "rounds.jsonl") {
                    out.push(path);
                }
            }
        }
        let mut found = Vec::new();
        walk(root, &mut found);
        assert_eq!(found.len(), 1, "one chain log expected");
        found.remove(0)
    }

    /// 案例 T1（6 组）：三类失败 × 首次/第二次失败——terminal 覆盖全部真实轮，
    /// class/through key/身份精确，正常机械失败必得 Durable。
    #[test]
    fn finalize_covers_three_classes_and_both_first_and_second_failures() {
        for class in [
            SC_TERMINAL_CLASS_COMPILE,
            SC_TERMINAL_CLASS_IR_PREFLIGHT,
            SC_TERMINAL_CLASS_STRUCTURE_GATE,
        ] {
            for rounds in [1u32, 2u32] {
                let (_tmp, lifecycle, scope) = super::fixture();
                let first = bind(&lifecycle, &scope, None, 0);
                observe_compile(&lifecycle, &first);
                let mut round_ids = vec![first.round.round_id.clone()];
                if rounds == 2 {
                    let second = bind(&lifecycle, &scope, Some(&first), 1);
                    observe_compile(&lifecycle, &second);
                    round_ids.push(second.round.round_id.clone());
                }
                let last_round_id = round_ids.last().cloned().expect("round bound");
                let outcome = finalize_sc_failure_chain(
                    &lifecycle,
                    &first.chain,
                    class,
                    "original failure",
                    Vec::new(),
                )
                .expect("finalize");
                let ScFailureTerminalOutcome::Durable(read) = outcome else {
                    panic!("clean log must be durable");
                };
                let terminal = read.entries.last().expect("terminal entry");
                let ScFailureEvidenceEvent::ChainTerminal {
                    last_failure_round_id,
                    last_physical_round_id,
                    terminal_class,
                    completeness,
                    ..
                } = &terminal.event
                else {
                    panic!("last entry must be ChainTerminal")
                };
                assert_eq!(terminal_class, class);
                assert_eq!(last_failure_round_id.as_str(), last_round_id.as_str());
                assert_eq!(
                    last_physical_round_id.as_deref(),
                    Some(last_round_id.as_str())
                );
                assert!(matches!(completeness, ScPackageCompleteness::Complete));
                assert_eq!(read.evidence_through_entry_key, terminal.entry_key);
                for round_id in &round_ids {
                    assert!(
                        read.entries.iter().any(|entry| matches!(
                            &entry.event,
                            ScFailureEvidenceEvent::RoundBound { round } if &round.round_id == round_id
                        )),
                        "terminal 必须覆盖该轮的真实绑定"
                    );
                }
            }
        }
    }

    /// 案例 T2：Started 未交付 ⇒ 计入 last_physical、缺项带真实 cause，
    /// last_failure 仍指前一失败轮。
    #[test]
    fn started_without_delivery_is_last_physical_with_missing_output() {
        let (_tmp, lifecycle, scope) = super::fixture();
        let first = bind(&lifecycle, &scope, None, 0);
        observe_compile(&lifecycle, &first);
        let interrupted = bind(&lifecycle, &scope, Some(&first), 1);
        let outcome = finalize_sc_failure_chain(
            &lifecycle,
            &first.chain,
            SC_TERMINAL_CLASS_TEACHING_INTERRUPTED,
            "provider session drive failed: cancelled",
            vec![(
                interrupted.round.round_id.clone(),
                ScRoundLaunchState::Started,
            )],
        )
        .expect("finalize");
        let ScFailureTerminalOutcome::Durable(read) = outcome else {
            panic!("Started 观察必须 durable")
        };
        let ScFailureEvidenceEvent::ChainTerminal {
            last_failure_round_id,
            last_physical_round_id,
            completeness,
            missing_evidence,
            ..
        } = &read.entries.last().expect("terminal").event
        else {
            panic!("terminal")
        };
        assert_eq!(last_failure_round_id.as_str(), first.round.round_id.as_str());
        assert_eq!(
            last_physical_round_id.as_deref(),
            Some(interrupted.round.round_id.as_str())
        );
        assert!(matches!(completeness, ScPackageCompleteness::Incomplete));
        let missing = missing_evidence
            .iter()
            .find(|item| item.code == "sc_failure_output_not_delivered")
            .expect("Started 未交付必须计入缺项");
        assert_eq!(missing.round_id, interrupted.round.round_id);
        assert!(missing.detail.contains("cancelled"), "真实 cause 必须保留");
    }

    /// 案例 T3：无观察且无 FailureObserved 的 bound 轮 ⇒ Unknown，拒绝终局、零新增字节。
    #[test]
    fn unobserved_bound_round_is_unknown_and_appends_nothing() {
        let (tmp, lifecycle, scope) = super::fixture();
        let first = bind(&lifecycle, &scope, None, 0);
        observe_compile(&lifecycle, &first);
        let _ghost = bind(&lifecycle, &scope, Some(&first), 1);
        let path = log_path(tmp.path());
        let before = std::fs::read(&path).expect("log");
        let error = match finalize_sc_failure_chain(
            &lifecycle,
            &first.chain,
            SC_TERMINAL_CLASS_COMPILE,
            "original failure",
            Vec::new(),
        ) {
            Ok(_) => panic!("unknown observation must not finalize"),
            Err(error) => error,
        };
        assert_eq!(error.code, "sc_failure_launch_observation_unknown");
        assert_eq!(
            std::fs::read(&path).expect("log"),
            before,
            "未知观察不得 append/覆盖"
        );
    }

    /// 案例 T4：真实 NotStarted 观察不计 last_physical（保留前失败轮），也不产生缺项。
    #[test]
    fn supplied_not_started_is_never_last_physical() {
        let (_tmp, lifecycle, scope) = super::fixture();
        let first = bind(&lifecycle, &scope, None, 0);
        observe_compile(&lifecycle, &first);
        let refused = bind(&lifecycle, &scope, Some(&first), 1);
        let outcome = finalize_sc_failure_chain(
            &lifecycle,
            &first.chain,
            SC_TERMINAL_CLASS_TEACHING_INTERRUPTED,
            "refused",
            vec![(
                refused.round.round_id.clone(),
                ScRoundLaunchState::NotStarted,
            )],
        )
        .expect("finalize");
        let ScFailureTerminalOutcome::Durable(read) = outcome else {
            panic!("durable")
        };
        let ScFailureEvidenceEvent::ChainTerminal {
            last_physical_round_id,
            completeness,
            ..
        } = &read.entries.last().expect("terminal").event
        else {
            panic!("terminal")
        };
        assert_eq!(
            last_physical_round_id.as_deref(),
            Some(first.round.round_id.as_str())
        );
        assert!(
            matches!(completeness, ScPackageCompleteness::Complete),
            "NotStarted 不产生缺项"
        );
    }

    /// 案例 T5：坏日志 ⇒ Prefix（真实前缀 + 本地终局事实），不 append、永不 Complete，
    /// 缺项沿用 store 原 code/detail。
    #[test]
    fn corrupt_log_yields_prefix_outcome_without_fabricated_entries() {
        let (tmp, lifecycle, scope) = super::fixture();
        let first = bind(&lifecycle, &scope, None, 0);
        observe_compile(&lifecycle, &first);
        let path = log_path(tmp.path());
        let mut bytes = std::fs::read(&path).expect("log");
        bytes.extend_from_slice(b"{\"schema_version\":1,\"broken\"\n");
        std::fs::write(&path, &bytes).expect("corrupt");
        let before = std::fs::read(&path).expect("log");
        let outcome = finalize_sc_failure_chain(
            &lifecycle,
            &first.chain,
            SC_TERMINAL_CLASS_COMPILE,
            "original failure",
            Vec::new(),
        )
        .expect("corrupt log must still yield a prefix outcome");
        let ScFailureTerminalOutcome::Prefix { read, terminal } = outcome else {
            panic!("corrupt log must not claim durable terminal")
        };
        assert!(!read.entries.is_empty(), "真实前缀不得为空");
        assert!(
            read.entries
                .iter()
                .all(|entry| !matches!(&entry.event, ScFailureEvidenceEvent::ChainTerminal { .. })),
            "坏日志不得伪造 terminal entry"
        );
        assert_eq!(
            read.evidence_through_entry_key,
            read.entries.last().unwrap().entry_key
        );
        assert_eq!(terminal.completeness, ScPackageCompleteness::Incomplete);
        assert!(
            terminal
                .missing_evidence
                .iter()
                .any(|item| item.code == "sc_failure_log_corrupt"),
            "缺项必须沿用 store 原 code，不 hardcode 新码"
        );
        assert_eq!(
            std::fs::read(&path).expect("log"),
            before,
            "不得 append/改坏日志"
        );
    }
}
