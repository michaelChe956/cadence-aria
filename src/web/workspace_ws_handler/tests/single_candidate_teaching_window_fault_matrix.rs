//! bound-sc-generation-teaching（序 3）Task 9：接管/旧数据兼容/三组序列
//! 的故障重放验收矩阵（REQ-WSC-28B/28C/28E，tasks 6.2/6.3/6.4/8.1）。
//!
//! 物理启动的权威证据 = durable `provider_start_ledger` 的
//! `single_candidate_author:` 前缀条目数（Task 7 确立口径：ledger 与窗口
//! attempt 是两条互不镜像的写路径）；**不**把尝试流水行的
//! `provider_attempt` 字段当「零物理启动」证据（Task 8 评审 Important-2
//! 修复后：未建窗路径两侧都写 `None`，`NoProviderStart` 零生产写点、
//! 登记为计划义务缺口），亦不断言流水行 stage 取值（与流水分类演化
//! 解耦：本矩阵的验收面是物理启动账与窗口耗尽，不镜像流水分类）。
//!
//! - `takeover_does_not_mint_new_chance`（6.2）：接管后新 session 上的教学
//!   请求 ⇒ 零额外调用、无新（可claim的）窗口；原 session ledger 保持权威。
//! - `none_fields_and_empty_ledger_mean_zero_not_two`（6.3）：窗口字段全
//!   缺省 + 空 ledger ⇒ 机会数为 0 而非 2（e2e：教学请求零额外 + 证据
//!   缺失说明）。
//! - `legacy_record_teaching_request_zero_extra`（6.4）：旧格式 durable 记录
//!   （无窗口键、旧式账本条目）被新代码读取 ⇒ 数值不被改写、教学请求零
//!   额外、不倒填。
//! - `sequence_matrix_three_groups_le_two_starts`（8.1 收口）：parse→
//!   structure、IR→structure、structure→parse 三组序列各自实际启动 ≤2、
//!   第二次失败后第三次为 0、durable 无结构槽痕迹。

use super::single_candidate_provider_run::{
    ProviderRunFixture, SequenceOutputProvider, next_error_message, next_provider_input,
    no_more_provider_inputs, single_candidate_context, single_candidate_markdown,
    wait_for_single_candidate_phase,
};
use super::*;
use crate::product::lifecycle_store::sc_failure_evidence::{
    ScFailureEvidenceEvent, sc_failure_chain_id, sc_failure_split_locator,
};
use crate::product::models::{SingleCandidatePhase, WorkspaceSessionRecord};
use crate::product::work_item_plan_policy::{
    ProviderStartLedgerEntry, RunPolicy, ScTeachingWindowRequest, ScTeachingWindowStatus,
    WorkItemPlanFlowKind,
};
use crate::web::workspace_ws_types::{
    ScEvidenceRef, ScFailureChainIdentity, ScFailureScope, ScPromptDisposition, ScRoundIdentity,
};
use sha2::Digest as _;
use std::sync::atomic::AtomicUsize;

/// durable `provider_start_ledger` 中 `single_candidate_author:{id}:` 前缀
/// 条目数（物理启动账，Task 7 口径）。
fn prefix_count(lifecycle: &crate::product::lifecycle_store::LifecycleStore, id: &str) -> usize {
    lifecycle
        .get_workspace_session(id)
        .expect("reload session for ledger")
        .provider_start_ledger
        .iter()
        .filter(|entry| {
            entry
                .provider_start_idempotency_key
                .starts_with(&format!("single_candidate_author:{id}:"))
        })
        .count()
}

fn record_of(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    id: &str,
) -> WorkspaceSessionRecord {
    lifecycle.get_workspace_session(id).expect("reload session")
}

fn has_policy_diagnostic(
    lifecycle: &crate::product::lifecycle_store::LifecycleStore,
    id: &str,
    code: &str,
) -> bool {
    record_of(lifecycle, id)
        .policy_diagnostics
        .iter()
        .any(|diagnostic| diagnostic.code == code)
}

fn session_json_bytes(fixture: &ProviderRunFixture, id: &str) -> String {
    let record = record_of(&fixture.lifecycle, id);
    let path = fixture
        .lifecycle
        .app_paths()
        .issue_lifecycle_root(&record.project_id, &record.issue_id)
        .join("workspace-sessions")
        .join(format!("{id}.json"));
    std::fs::read_to_string(&path).expect("read session json")
}

/// 种子 chain 身份（chain_id 由 store 的长度前缀哈希口径算出，不自造）。
const SEED_BUSINESS_KEY: &str = "business_key_fault_matrix_seed";
const SEED_NODE_ID: &str = "node_fault_matrix_seed";
/// 种子 source 正文（真实写入 split run 产物 ⇒ 可读回）。
const SEED_SOURCE_TEXT: &str = "# Work Item Plan\nseed source body\n";
/// 种子教学提示正文（真实 put 全文 ⇒ 可读回、摘要自洽）。
const SEED_PROMPT_TEXT: &str = "教学提示种子正文（真实 put 后读回）\n";
/// 审计用构造器名（只进审计字段，不进 locator）。
const SEED_BUILDER_ID: &str = "seed_teaching_prompt";

/// 全文摘要（与证据库同一口径：sha256 小写 hex）。
fn sha256_hex(text: &str) -> String {
    hex::encode(sha2::Sha256::digest(text.as_bytes()))
}

/// 「完全合格」教学请求（四项判据的事实面完整；用于在播种会话上驱动
/// establish 的真实 store 面）。
///
/// 来源面全部来自真实 store（P26）：prompt locator/摘要出自真实
/// `put_sc_teaching_prompt` 并读回校验；failure source 指向真实 split run
/// 产物（读回后实算摘要）。旧的伪 ref / builder 名不再冒充「生产可读
/// 来源」。
fn seed_teaching_request(
    fixture: &ProviderRunFixture,
    session_id: &str,
) -> ScTeachingWindowRequest {
    let record = record_of(&fixture.lifecycle, session_id);
    // 真实 split run：source 正文与 locator 均来自真实写入。
    let handle = fixture
        .lifecycle
        .begin_work_item_split_provider_run(
            &record.project_id,
            &record.issue_id,
            &crate::product::models::ProviderName::Codex,
            session_id,
        )
        .expect("begin seed split run");
    fixture
        .lifecycle
        .complete_work_item_split_provider_run(
            &handle,
            "seed split prompt",
            &serde_json::Value::String(SEED_SOURCE_TEXT.to_string()),
        )
        .expect("complete seed split run");
    let scope = ScFailureScope {
        session_id: session_id.to_string(),
        project_id: record.project_id.clone(),
        issue_id: record.issue_id.clone(),
        plan_id: record.entity_id.clone(),
    };
    let chain = ScFailureChainIdentity {
        scope: scope.clone(),
        chain_id: sc_failure_chain_id(
            session_id,
            SEED_BUSINESS_KEY,
            SEED_NODE_ID,
            handle.role_run_seq,
        )
        .expect("seed chain id"),
        business_key: SEED_BUSINESS_KEY.to_string(),
        original_node_id: SEED_NODE_ID.to_string(),
        original_role_run_seq: handle.role_run_seq,
    };
    let round = ScRoundIdentity {
        chain_id: chain.chain_id.clone(),
        round_id: handle.run_ref.clone(),
        node_id: SEED_NODE_ID.to_string(),
        round_ordinal: 1,
        provider: crate::product::models::ProviderName::Codex,
        workspace_session_id: handle.workspace_session_id.clone(),
        role_run_seq: handle.role_run_seq,
        run_ref: handle.run_ref.clone(),
    };
    fixture
        .lifecycle
        .append_sc_failure_evidence(
            &chain,
            &ScFailureEvidenceEvent::RoundBound {
                round: round.clone(),
            },
        )
        .expect("bind seed round");
    // 真实 put：locator 与摘要来自 store（不是 builder 名）。
    let evidence = fixture
        .lifecycle
        .put_sc_teaching_prompt(
            &chain,
            &round.round_id,
            2,
            SEED_BUILDER_ID,
            SEED_PROMPT_TEXT,
            ScPromptDisposition::Prepared,
        )
        .expect("put seed teaching prompt");
    let ScEvidenceRef::Available {
        locator: prompt_ref,
        sha256: prompt_digest,
        bytes: prompt_bytes,
    } = evidence.prompt
    else {
        panic!("真实 put 必须产出可用的提示事实");
    };
    // locator 必须可读回且正文逐字一致（伪来源不可读）。
    let read_back = fixture
        .lifecycle
        .read_sc_teaching_prompt(&chain, &prompt_ref)
        .expect("seed prompt locator 必须可读回");
    assert_eq!(read_back, SEED_PROMPT_TEXT, "真实 put 的正文必须逐字可读回");
    assert_eq!(prompt_bytes, SEED_PROMPT_TEXT.len() as u64);
    // source：真实 split 产物（读回后实算摘要）。
    let source_body = fixture
        .lifecycle
        .read_sc_split_output(&scope, &round)
        .expect("seed source 必须可读回");
    let failure_source_ref = sc_failure_split_locator(session_id, handle.role_run_seq);
    let failure_source_sha256 = sha256_hex(&source_body);
    let digest64 = |seed: &str| format!("{seed:0<64}");
    ScTeachingWindowRequest {
        provider_path_admissible: true,
        failure_source_ref: Some(failure_source_ref),
        failure_source_sha256,
        failure_typed_kinds: vec![
            crate::product::planning_failure::PlanningFailureKind::Unknown {
                code: "t9_seed".to_string(),
            },
        ],
        business_request_digest: digest64("t9b"),
        material_digests: vec![digest64("t9c")],
        context_digest: digest64("t9d"),
        allowed_edit_closure: "seed closure".to_string(),
        instruction_digest: digest64("t9e"),
        prompt_ref,
        prompt_digest,
    }
}

/// 经 policy route CAS 播种 durable（status/phase/gate/ledger 均显式给定；
/// 教学窗口字段不在 persist 清单内 ⇒ 既有窗口原样保留）。
fn seed_policy_route(
    fixture: &ProviderRunFixture,
    status: crate::product::models::WorkspaceSessionStatus,
    phase: Option<SingleCandidatePhase>,
    gate: Option<crate::product::work_item_plan_policy::HumanGateSnapshot>,
    ledger: Vec<ProviderStartLedgerEntry>,
) -> WorkspaceSessionRecord {
    let record = record_of(&fixture.lifecycle, &fixture.record.id);
    let persist = crate::product::lifecycle_store::PolicyRoutePersist {
        status,
        single_candidate_phase: phase,
        run_history: record.run_history.clone(),
        scope: record.review_invocation_scope.clone(),
        gate,
        diagnostics: record.policy_diagnostics.clone(),
        repair_reservation: None,
        provider_start_ledger: ledger,
        author_rewrite_budget: record.author_rewrite_budget.clone(),
    };
    fixture
        .lifecycle
        .compare_and_save_policy_route(&record, persist)
        .expect("seed durable state via policy route CAS")
}

/// 播种后同步 engine 的 runtime 镜像（避免 run 启动路径用旧镜像判相位）。
async fn reload_engine_session(fixture: &ProviderRunFixture, record: &WorkspaceSessionRecord) {
    fixture
        .engine
        .lock()
        .await
        .reload_session_from_record(record.clone());
}

async fn start_generation(context: WorkspaceInboundContext) {
    handle_workspace_inbound_message(
        context,
        WsInMessage::StartGeneration {
            provider_config: provider_config(),
            reviewer_enabled: false,
            resume_recovery: None,
        },
    )
    .await;
}

/// 可恢复人工门夹具（体例对照 lifecycle_store 测试的 `resumable_gate`）。
fn resumable_gate() -> crate::product::work_item_plan_policy::HumanGateSnapshot {
    use crate::product::work_item_plan_policy as policy;
    let class = policy::FindingClass::HumanRequired;
    policy::HumanGateSnapshot {
        candidate_recovery: None,
        findings: vec![policy::ClassifiedFinding {
            class,
            fingerprint: policy::FindingFingerprint::for_finding(
                None,
                class,
                "需要人工决定范围",
                Some("scope"),
            ),
            category: Some(policy::ReviewFindingCategory::ScopeConflict),
            severity: "blocking".to_string(),
            message: "需要人工决定范围".to_string(),
            evidence: Some("冲突证据".to_string()),
            required_action: Some("确认范围".to_string()),
            contract_field: Some("scope".to_string()),
            identity_unstable: false,
        }],
        repeated_fingerprints: Vec::new(),
        attempts_used: 2,
        manual_repairs_remaining: 1,
        trigger: policy::HumanReason::NativeHumanRequired,
        resumable: true,
        accepted_feedback_turns: None,
    }
}

/// 测试夹具手术：整记录 JSON 的单键改写（takeover 前置的 run_policy 字段
/// 不在 policy route persist 清单内，须直接手术）。
fn patch_session_json_field(fixture: &ProviderRunFixture, id: &str, key: &str, value: &str) {
    let path = fixture
        .lifecycle
        .app_paths()
        .issue_lifecycle_root("project_0001", "issue_0001")
        .join("workspace-sessions")
        .join(format!("{id}.json"));
    let raw = std::fs::read_to_string(&path).expect("read session json");
    let mut document: serde_json::Value = serde_json::from_str(&raw).expect("parse session json");
    document[key] = serde_json::Value::String(value.to_string());
    std::fs::write(
        &path,
        serde_json::to_string(&document).expect("serialize session json"),
    )
    .expect("write session json");
}

/// 6.2（e2e）：接管后新 session 上的教学请求 ⇒ 零额外调用、无新窗口；
/// 原 session ledger 保持权威。接管清空子会话 ledger（design D7）⇒ 子会话
/// 上重放的教学请求无法证明原授权（判据① 前缀为空）⇒ `NoProvableAuthorization`
/// 未批准（max=1、无可 claim attempt）⇒ handler 消费面 `Denied` ⇒ 不重驱。
#[tokio::test]
async fn takeover_does_not_mint_new_chance() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // ① 播种父会话：Generate 相位 + 原调用 :0（父链可证明的原授权）。
    let entry = ProviderStartLedgerEntry {
        provider_start_idempotency_key: format!("single_candidate_author:{}:0", fixture.record.id),
        started: true,
        provider: None,
        started_at: None,
        provider_attempt_key: None,
    };
    let generate = seed_policy_route(
        &fixture,
        crate::product::models::WorkspaceSessionStatus::Running,
        Some(SingleCandidatePhase::Generate),
        None,
        vec![entry],
    );
    // ② 父会话经真实 establish 面建立已批准教学窗口（父链的 C 事实）。
    let established = match fixture
        .lifecycle
        .establish_sc_teaching_window(
            &generate,
            &seed_teaching_request(&fixture, &fixture.record.id),
        )
        .expect("seed establish teaching window on parent")
    {
        crate::product::lifecycle_store::ScTeachingEstablishOutcome::Established {
            record,
            approved,
            ..
        } => {
            assert!(approved, "前提：父窗口已批准");
            record
        }
        _ => panic!("seeding establish must create the parent window"),
    };
    let parent_window = established
        .sc_generation_teaching_windows
        .last()
        .expect("parent window")
        .clone();
    // ③ 手术父会话至 takeover 前置态（StoppedNeedsHuman + AutoIfValid +
    //    可恢复人工门；run_policy 不在 persist 清单 ⇒ JSON 手术）。
    let stopped = seed_policy_route(
        &fixture,
        crate::product::models::WorkspaceSessionStatus::StoppedNeedsHuman,
        Some(SingleCandidatePhase::Generate),
        Some(resumable_gate()),
        established.provider_start_ledger.clone(),
    );
    patch_session_json_field(&fixture, &fixture.record.id, "run_policy", "auto_if_valid");
    let parent = record_of(&fixture.lifecycle, &stopped.id);
    assert_eq!(parent.run_policy, RunPolicy::AutoIfValid);

    // ④ 真实接管 ⇒ 子会话。
    let child = fixture
        .lifecycle
        .takeover_stopped_needs_human(&parent.id)
        .expect("takeover stopped run");
    let child_id = child.id.clone();
    assert!(
        child.sc_generation_teaching_windows.is_empty(),
        "接管不得把窗口铸造/复制到新 session"
    );
    assert!(
        child.provider_start_ledger.is_empty(),
        "接管清空子会话 ledger（design D7）"
    );

    // ⑤ 接管后新 session 上的教学请求（与父链同一请求的重放）：子会话无
    //    可证明原授权 ⇒ 未批准 ⇒ 机会 0 而非 2 ⇒ 零额外调用。
    let request = seed_teaching_request(&fixture, &child_id);
    let child_seeded = record_of(&fixture.lifecycle, &child_id);
    match fixture
        .lifecycle
        .establish_sc_teaching_window(&child_seeded, &request)
        .expect("teaching request on takeover child resolves")
    {
        crate::product::lifecycle_store::ScTeachingEstablishOutcome::Established {
            record,
            approved,
            rejection,
        } => {
            assert!(!approved, "接管子会话不得凭新 session 身份获得批准");
            assert_eq!(
                rejection,
                Some(crate::product::work_item_plan_policy::ScTeachingApprovalRejection::NoProvableAuthorization),
                "证据缺失说明：空 ledger ⇒ 无可证明原授权"
            );
            let window = record
                .sc_generation_teaching_windows
                .last()
                .expect("window");
            assert_eq!(window.max_attempts, 1, "未批准 ⇒ max=1（不是 2）");
            assert_eq!(
                window.claimable_attempt_no(),
                None,
                "机会数为 0（无可 claim attempt）"
            );
            assert!(
                has_policy_diagnostic(
                    &fixture.lifecycle,
                    &child_id,
                    "sc_teaching_window_not_approved"
                ),
                "证据缺失必须留痕 durable 诊断"
            );
        }
        crate::product::lifecycle_store::ScTeachingEstablishOutcome::Existing { .. } => {
            panic!("子会话无窗口，必须走建立分支")
        }
    }
    // claim（handler 消费点）⇒ Denied(NotApproved)：零额外调用的执法面。
    assert_eq!(
        fixture
            .lifecycle
            .claim_sc_teaching_attempt(&record_of(&fixture.lifecycle, &child_id), 0)
            .expect("claim on child resolves"),
        crate::product::lifecycle_store::ScTeachingClaimOutcome::Denied(
            crate::product::lifecycle_store::ScTeachingClaimRejection::NotApproved
        )
    );
    // 零额外物理启动：子会话 ledger 无任何占位/启动（接管清空后零新增）。
    assert_eq!(prefix_count(&fixture.lifecycle, &child_id), 0);
    // ⑥ 原 session ledger 保持权威：父记录窗口与账本原样。
    let parent_after = record_of(&fixture.lifecycle, &parent.id);
    assert_eq!(
        parent_after
            .sc_generation_teaching_windows
            .last()
            .expect("parent window"),
        &parent_window,
        "父窗口事实原样（权威）"
    );
    assert_eq!(
        prefix_count(&fixture.lifecycle, &parent.id),
        established.provider_start_ledger.len(),
        "父 ledger 条目数原样（权威）"
    );
    assert_eq!(
        parent_after.provider_start_ledger,
        established.provider_start_ledger
    );
}

/// 6.3（e2e）：窗口字段全缺省 + 空 ledger ⇒ 机会数为 0 而非 2；教学请求
/// 零额外 + 证据缺失说明（`sc_teaching_window_not_approved` 诊断）。
#[tokio::test]
async fn none_fields_and_empty_ledger_mean_zero_not_two() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 窗口字段全缺省（fresh 记录无窗口键）+ 空 ledger + Generate 相位
    //（生成中形态：教学请求可发生，但原授权不可证明）。
    let seeded = seed_policy_route(
        &fixture,
        crate::product::models::WorkspaceSessionStatus::Running,
        Some(SingleCandidatePhase::Generate),
        None,
        Vec::new(),
    );
    reload_engine_session(&fixture, &seeded).await;
    assert!(
        !session_json_bytes(&fixture, &fixture.record.id)
            .contains("sc_generation_teaching_windows"),
        "前提：窗口字段全缺省（无窗口键）"
    );
    assert!(
        seeded.provider_start_ledger.is_empty(),
        "前提：空 ledger（无可证明原授权）"
    );

    // 教学请求（事实完整）⇒ 判据① 失败 ⇒ 未批准 ⇒ 机会数 0 而非 2。
    let request = seed_teaching_request(&fixture, &fixture.record.id);
    match fixture
        .lifecycle
        .establish_sc_teaching_window(&seeded, &request)
        .expect("teaching request resolves")
    {
        crate::product::lifecycle_store::ScTeachingEstablishOutcome::Established {
            record,
            approved,
            rejection,
        } => {
            assert!(!approved);
            assert_eq!(
                rejection,
                Some(crate::product::work_item_plan_policy::ScTeachingApprovalRejection::NoProvableAuthorization),
                "不从「字段缺省 + 空 ledger」推断机会（证据缺失说明）"
            );
            let window = record
                .sc_generation_teaching_windows
                .last()
                .expect("window");
            assert_eq!(window.max_attempts, 1, "缺省 ⇒ max=1（不是 2）");
            assert_eq!(window.attempts.len(), 1, "attempt1 已固化（原调用占位）");
            assert_eq!(window.claimable_attempt_no(), None, "机会数 = 0 而非 2");
        }
        crate::product::lifecycle_store::ScTeachingEstablishOutcome::Existing { .. } => {
            panic!("无窗口记录必须走建立分支")
        }
    }
    assert!(
        has_policy_diagnostic(
            &fixture.lifecycle,
            &fixture.record.id,
            "sc_teaching_window_not_approved"
        ),
        "证据缺失必须留痕 durable 诊断（显式说明，不静默）"
    );
    // 教学请求零额外：claim ⇒ Denied(NotApproved)（handler 消费面：Denied ⇒
    // 不重驱）；ledger 零占位 ⇒ 零物理启动面。
    assert_eq!(
        fixture
            .lifecycle
            .claim_sc_teaching_attempt(&record_of(&fixture.lifecycle, &fixture.record.id), 0)
            .expect("claim resolves"),
        crate::product::lifecycle_store::ScTeachingClaimOutcome::Denied(
            crate::product::lifecycle_store::ScTeachingClaimRejection::NotApproved
        )
    );
    assert_eq!(
        prefix_count(&fixture.lifecycle, &fixture.record.id),
        0,
        "零额外物理启动（无占位、无启动）"
    );
}

/// 6.4（e2e）：旧格式 durable 记录（无窗口键、旧式 `started` 布尔条目）
/// 被新代码读取 ⇒ 数值不被改写、教学请求零额外、不倒填。
#[tokio::test]
async fn legacy_record_teaching_request_zero_extra() {
    let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
    // 旧式账本条目：started 布尔 + 无 provider/started_at/attempt 键
    //（Task 2 之前的历史形态；序列化字节与旧记录一致）。
    let legacy_entry = |ordinal: usize| ProviderStartLedgerEntry {
        provider_start_idempotency_key: format!(
            "single_candidate_author:{}:{ordinal}",
            fixture.record.id
        ),
        started: true,
        provider: None,
        started_at: None,
        provider_attempt_key: None,
    };
    let original_phase = record_of(&fixture.lifecycle, &fixture.record.id).single_candidate_phase;
    let seeded = seed_policy_route(
        &fixture,
        record_of(&fixture.lifecycle, &fixture.record.id).status,
        original_phase,
        None,
        vec![legacy_entry(0), legacy_entry(1)],
    );
    reload_engine_session(&fixture, &seeded).await;
    let numbers_before = (
        seeded.run_history.repairs_used,
        seeded.run_history.manual_repairs_used,
    );
    assert!(
        !session_json_bytes(&fixture, &fixture.record.id)
            .contains("sc_generation_teaching_windows"),
        "前提：旧格式记录无窗口键"
    );

    // 新代码真实跑一条生成：原调用（:2）+ parse 可教学失败 ⇒ 教学请求。
    let (input_tx, mut input_rx) = mpsc::unbounded_channel();
    let provider = Arc::new(SequenceOutputProvider {
        outputs: vec![
            {
                let mut markdown = single_candidate_markdown(&fixture.story_id, &fixture.design_id);
                markdown = markdown
                    .split("\n\n")
                    .filter(|chunk| !chunk.starts_with("### Goal"))
                    .collect::<Vec<_>>()
                    .join("\n\n");
                markdown
            },
            single_candidate_markdown(&fixture.story_id, &fixture.design_id),
        ],
        inputs: input_tx,
        next: AtomicUsize::new(0),
    });
    let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
    start_generation(context).await;
    let _first_input = next_provider_input(&mut input_rx).await;
    let starts_after_original = prefix_count(&fixture.lifecycle, &fixture.record.id);
    // 教学请求零额外：旧两启动 + 原调用 = 3 > 1 ⇒ 无剩余机会 ⇒ 无重驱输入。
    no_more_provider_inputs(&mut input_rx).await;
    let message = next_error_message(&mut outbound_rx).await;
    assert!(
        message.contains("compile markdown source failed") && message.contains("missing_section"),
        "旧记录链按既有失败面回显真实原因: {message}"
    );
    assert!(
        !message.contains("after one teaching re-drive"),
        "零额外 ⇒ 单轮失败面: {message}"
    );
    wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;

    let after = record_of(&fixture.lifecycle, &fixture.record.id);
    // 数值不被改写。
    assert_eq!(
        (
            after.run_history.repairs_used,
            after.run_history.manual_repairs_used
        ),
        numbers_before,
        "旧记录的预算数值不被新代码改写"
    );
    // 不倒填：旧条目仍无 provider_attempt_key / provider / started_at。
    for ordinal in 0..2 {
        let entry = after
            .provider_start_ledger
            .iter()
            .find(|entry| {
                entry.provider_start_idempotency_key
                    == format!("single_candidate_author:{}:{ordinal}", fixture.record.id)
            })
            .expect("legacy entry retained");
        assert!(
            entry.provider_attempt_key.is_none()
                && entry.provider.is_none()
                && entry.started_at.is_none(),
            "旧条目不被倒填关联（ordinal {ordinal}）: {entry:?}"
        );
    }
    // 教学请求零额外：物理启动 = 2 旧 + 1 原调用，无第三条。
    assert_eq!(
        prefix_count(&fixture.lifecycle, &fixture.record.id),
        starts_after_original,
        "教学请求零额外物理启动"
    );
    assert_eq!(
        prefix_count(&fixture.lifecycle, &fixture.record.id),
        3,
        "旧账保持权威（2 旧启动 + 本轮原调用）"
    );
    // 窗口：未批准（判据③ 3 > 1 ⇒ 无剩余物理机会）⇒ 零教学 claim。
    assert!(
        has_policy_diagnostic(
            &fixture.lifecycle,
            &fixture.record.id,
            "sc_teaching_window_not_approved"
        ),
        "无剩余机会必须留痕 durable 诊断"
    );
    let window = after
        .sc_generation_teaching_windows
        .last()
        .expect("teaching window recorded");
    assert_eq!(
        (
            window.max_attempts,
            window.repair_once_approved,
            window.attempts.len()
        ),
        (1, false, 1),
        "未批准窗口不含任何教学 claim（零额外）"
    );
    assert!(
        matches!(
            window.status,
            ScTeachingWindowStatus::Closed {
                reason:
                    crate::product::work_item_plan_policy::ScTeachingWindowCloseReason::Exhausted
            }
        ),
        "denial 关窗收口（无 claim 可消费，重放不得再入）: {:?}",
        window.status
    );
}

/// 8.1 收口：三组序列（parse→structure、IR→structure、structure→parse）
/// 各自断言实际启动 ≤2、第二次失败后第三次为 0、无新开结构槽（durable 无
/// 双布尔槽痕迹）。物理启动证据 = 前缀条目数；不断言流水行 stage 取值
///（与流水分类演化解耦：Task 8 修复后 stage 已四路穷尽，但本矩阵的验收
/// 面是物理启动账与窗口耗尽，不镜像流水分类）。
#[tokio::test]
async fn sequence_matrix_three_groups_le_two_starts() {
    enum Round {
        MissingSection(&'static str),
        UnknownRequirementRef,
        DependencySelfCycle,
    }
    let build_output = |round: &Round, story_id: &str, design_id: &str| {
        match round {
        Round::MissingSection(section) => single_candidate_markdown(story_id, design_id)
            .split("\n\n")
            .filter(|chunk| !chunk.starts_with(section))
            .collect::<Vec<_>>()
            .join("\n\n"),
        Round::UnknownRequirementRef => {
            super::single_candidate_ir_reredrive::markdown_with_unknown_requirement_ref(
                story_id, design_id,
            )
        }
        Round::DependencySelfCycle => {
            single_candidate_markdown(story_id, design_id).replacen(
                "### Inputs\n\n### Outputs",
                "### Inputs\n- contract_id: contract.backend-internal\n- provider_logical_work_item_id: WI-001\n- required_capabilities: api.backend.read\n- compatibility_policy: require_all\n\n### Outputs",
                1,
            )
        }
    }
    };
    let run_group = |name: &'static str,
                     first: Round,
                     second: Round,
                     second_round_marker: &'static str| {
        async move {
            let fixture = ProviderRunFixture::new(WorkItemPlanFlowKind::SingleCandidate);
            let (input_tx, mut input_rx) = mpsc::unbounded_channel();
            let provider = Arc::new(SequenceOutputProvider {
                outputs: vec![
                    build_output(&first, &fixture.story_id, &fixture.design_id),
                    build_output(&second, &fixture.story_id, &fixture.design_id),
                    single_candidate_markdown(&fixture.story_id, &fixture.design_id),
                ],
                inputs: input_tx,
                next: AtomicUsize::new(0),
            });
            let (context, mut outbound_rx) = single_candidate_context(&fixture, provider);
            start_generation(context).await;
            // 启动 #1（原调用）+ 启动 #2（教学重驱）。
            let _first_input = next_provider_input(&mut input_rx).await;
            let reredrive_input = next_provider_input(&mut input_rx).await;
            assert!(
                reredrive_input.prompt.contains(second_round_marker),
                "{name}: 第二轮必须是教学重驱: {}",
                reredrive_input.prompt
            );
            // 第二次失败后第三次为 0：重驱轮之后无新输入。
            no_more_provider_inputs(&mut input_rx).await;
            let _message = next_error_message(&mut outbound_rx).await;
            wait_for_single_candidate_phase(&fixture, SingleCandidatePhase::Failed).await;
            // 实际启动 ≤2 且恰 2（原调用 + 教学；物理启动账为权威证据）。
            let starts = prefix_count(&fixture.lifecycle, &fixture.record.id);
            assert!(starts <= 2, "{name}: 实际启动必须 ≤2（实测 {starts}）");
            assert_eq!(starts, 2, "{name}: 原调用 + 一次教学 = 恰 2 次物理启动");
            let durable = record_of(&fixture.lifecycle, &fixture.record.id);
            assert_eq!(
                durable.sc_generation_teaching_windows.len(),
                1,
                "{name}: 单窗口承载两类失败"
            );
            let window = durable
                .sc_generation_teaching_windows
                .last()
                .expect("window");
            assert_eq!(window.attempts.len(), 2, "{name}: 两 attempt 后耗尽");
            assert!(
                matches!(
                    window.status,
                    ScTeachingWindowStatus::Closed {
                        reason: crate::product::work_item_plan_policy::ScTeachingWindowCloseReason::Exhausted
                    }
                ),
                "{name}: 第二次失败必须关窗（无第三次）: {:?}",
                window.status
            );
            // 无新开结构槽：durable 无双布尔槽痕迹（Task 6 删除面）。
            let json = session_json_bytes(&fixture, &fixture.record.id);
            assert!(
                !json.contains("structure_rerun_used") && !json.contains("compile_ir_rerun_used"),
                "{name}: durable 不得残留结构槽痕迹"
            );
            assert!(
                durable.repair_reservation.is_none(),
                "{name}: 无 A 通道预领（不旁路）"
            );
        }
    };
    // parse→structure。
    run_group(
        "parse→structure",
        Round::MissingSection("### Goal"),
        Round::DependencySelfCycle,
        "missing_section",
    )
    .await;
    // IR→structure。
    run_group(
        "IR→structure",
        Round::UnknownRequirementRef,
        Round::DependencySelfCycle,
        "unknown_requirement_ref",
    )
    .await;
    // structure→parse。
    run_group(
        "structure→parse",
        Round::DependencySelfCycle,
        Round::MissingSection("### Tasks"),
        "依赖环",
    )
    .await;
}
