//! 批2 Task 23(b45):四家一致性矩阵与 Codex 双审批真机探针。
//!
//! - `b45_live_four_provider_roles`:四家×六职责(M1–M5/M7)×Direct/LC 双入口
//!   ×fresh/resume 矩阵编排——聚合既有 `run_provider_matrix`(五阶段:
//!   M1 Author=story/design/plan×2、M2 Coder=coding、M3 Reviewer=review、
//!   M7 生命周期=fresh/resume 与 spawn 计数)与 `run_boundary_and_failure_
//!   matrix`(M4 group/repair=coding 正向/受控写/D4、M5 失败关闭=17 固定
//!   负向场景零 spawn),逐家断言六职责域覆盖并汇总 roles-matrix.json
//!   (每格 CLI exact version/native ID/argv 或 wire/policy digest/cwd/
//!   target/spawn 计数全要素,cell.json 由 harness 落盘)。M6 Codex 岗位
//!   由 `b45_live_codex_danger_apply_patch` 承担。
//! - `b45_live_codex_danger_apply_patch`:固定实际 CLI 版本,danger-full-
//!   access 下 on-request(岗位纪律档)与 never(Coder 原生档)两个隔离
//!   fixture 真实执行受控 apply_patch;保存审批帧出现/未出现、fileChange
//!   事实、实际 diff、事件后中断 turn、durable 审计与最终 Failed/不可采信。
//!   事件不可观测或不能可靠中断=该版本机制未通过,不写 Decline 预防、
//!   不退 read-only、不清岗位规则(计划 Task 23 Step 4 红线)。
//!
//! 分批收证:`B45_T23_PROVIDERS`(逗号分隔 claude/codex/pi/kimi 子集,
//! 缺省四家全跑);单家现场命令沿用 `LIVE_MATRIX_RUN_MODE` 快照身份合同。

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cadence_aria::cross_cutting::codex_provider::CodexProvider;
use cadence_aria::cross_cutting::streaming_provider::{
    ProviderEvent, ProviderPermissionMode, ProviderToolPolicy, StreamingProviderAdapter,
    StreamingProviderInput,
};
use cadence_aria::cross_cutting::tool_policy_audit::{
    AUTHOR_FILE_CHANGE_DETECTED, RoleRunBoundAuditSink, TOOL_POLICY_RUN_AUDIT_PARTITION,
};
use cadence_aria::product::app_paths::ProductAppPaths;
use cadence_aria::product::lifecycle_store::LifecycleStore;
use cadence_aria::product::models::ProviderName;
use cadence_aria::protocol::contracts::{AdapterRole, ProviderType};
use serde_json::{Value, json};
use tokio_util::sync::CancellationToken;

use super::harness::{
    ENTRYPOINT_SPLIT_SYNC, ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT, EvidenceCell,
    LiveLcGatewayHarness, LiveMatrixEvidence,
};
use super::live_matrix::require_lc_gateway_e2e_switch;

/// 矩阵证据根(计划 Files:cadence/reports/lc-gateway-multi-provider/
/// <provider>/b45-t23-roles/)。
fn b45_t23_evidence_root(provider: &ProviderName, facet: &str) -> PathBuf {
    let family = match provider {
        ProviderName::ClaudeCode => "claude-code",
        ProviderName::Codex => "codex",
        ProviderName::Pi => "pi",
        ProviderName::KimiCode => "kimi-code",
        ProviderName::Fake => "fake",
    };
    PathBuf::from("cadence/reports/lc-gateway-multi-provider")
        .join(family)
        .join("b45-t23-roles")
        .join(facet)
}

/// 分批 provider 清集(env 缺省=四家全跑)。
fn b45_t23_providers_from_env() -> Vec<ProviderName> {
    let all = [
        ProviderName::ClaudeCode,
        ProviderName::Codex,
        ProviderName::Pi,
        ProviderName::KimiCode,
    ];
    match std::env::var("B45_T23_PROVIDERS") {
        Ok(raw) if !raw.trim().is_empty() => raw
            .split(',')
            .filter_map(|token| match token.trim() {
                "claude" => Some(ProviderName::ClaudeCode),
                "codex" => Some(ProviderName::Codex),
                "pi" => Some(ProviderName::Pi),
                "kimi" => Some(ProviderName::KimiCode),
                other => panic!(
                    "B45_T23_PROVIDERS 未知 provider:{other:?}(合法:claude/codex/pi/kimi)"
                ),
            })
            .collect(),
        _ => all.to_vec(),
    }
}

/// 单格摘要(M1–M7 每格要素:CLI exact version/native ID/argv 或 wire/
/// policy digest/cwd/target/spawn 计数;全量形态见 cell.json)。
fn b45_t23_cell_summary(cell: &EvidenceCell) -> Value {
    json!({
        "stage": cell.stage,
        "entrypoint": cell.entrypoint,
        "fresh_or_resume": cell.fresh_or_resume,
        "capability_state": cell.capability_state,
        "exact_version": cell.exact_version,
        "native_session_id": cell.native_session_id,
        "argv_or_wire_capture_exists": cell.argv_or_wire_capture_exists,
        "approval_and_tool_events_exist": cell.approval_and_tool_events_exist,
        "audit_projection_digest": cell.audit_projection_digest,
        "frozen_projection_digest": cell.frozen_projection_digest,
        "process_cwd": cell.process_cwd,
        "target": cell.target,
        "provider_spawn_count": cell.provider_spawn_count,
        "native_resume_confirmed_id": cell.native_resume_confirmed_id,
        "requested_resume_id": cell.requested_resume_id,
        "execution_origin": cell.execution_origin,
        "denied_reason": cell.denied_reason,
    })
}

/// 六职责域覆盖断言(M6 由 danger 探针承担,此处只记录归属)。
///
/// 返回该 provider 的职责域摘要(roles-matrix.json 的单家段)。
fn b45_t23_assert_role_coverage(
    provider: &ProviderName,
    matrix: &LiveMatrixEvidence,
    boundary: &LiveMatrixEvidence,
) -> Value {
    let run_mode = super::snapshot::RunMode::from_env()
        .expect("LIVE_MATRIX_RUN_MODE 已在 run_provider_matrix 入口校验");
    let resume_round = matches!(run_mode, super::snapshot::RunMode::ResumeFromPlanSnapshot);
    let confirmed = matrix.confirmed_cells();
    let stage_confirmed =
        |stage: &str| confirmed.iter().filter(|cell| cell.stage == stage).count();

    // M1 Author:story/design/plan 三阶段真实执行格(resume 轮为承继格,
    // 引用来源轮证据链,不重新计票)。
    for stage in ["story", "design", "plan"] {
        if resume_round {
            let carried = matrix
                .cells()
                .iter()
                .filter(|cell| cell.stage == stage && cell.capability_state == "carried")
                .count();
            assert!(
                carried > 0,
                "M1 Author({stage}):resume 轮必须有承继格(引用来源轮证据链)"
            );
        } else {
            assert!(
                stage_confirmed(stage) > 0,
                "M1 Author({stage}):全链轮必须有真实 confirmed 格"
            );
        }
    }
    // Plan 双入口分列(Direct/LC:streaming 主入口与 split_sync 同步桥)。
    for entrypoint in [
        ENTRYPOINT_WORKSPACE_STREAMING_PLAN_SPLIT,
        ENTRYPOINT_SPLIT_SYNC,
    ] {
        assert!(
            matrix.cells().iter().any(|cell| cell.entrypoint == entrypoint),
            "M1 Author(plan)缺 {entrypoint} 入口证据(双栈分列,不以单入口冒充)"
        );
    }
    // M2 Coder / M3 Reviewer:真实执行轮必须有 confirmed 格。
    assert!(
        stage_confirmed("coding") > 0,
        "M2 Coder:coding 必须有真实 confirmed 格"
    );
    assert!(
        stage_confirmed("review") > 0,
        "M3 Reviewer:review 必须有真实 confirmed 格"
    );
    // M7 生命周期:resume 格原生同 ID(快照续跑轮);fresh 轮断言格完备
    //(fresh_or_resume 标记与 spawn 计数在 confirmed 断言组已承载)。
    if resume_round {
        let resume_confirmed = confirmed
            .iter()
            .filter(|cell| cell.fresh_or_resume == "resume")
            .count();
        assert!(
            resume_confirmed > 0,
            "M7 生命周期:resume 轮必须有真实执行的 resume 格(coding/review)"
        );
    }
    // confirmed 格全要素(Step 1 断言组已在 validate_against 承担,此处
    // 抽查 M 矩阵要素:CLI exact version 与 argv/wire 捕获不得空壳)。
    for cell in confirmed {
        assert!(
            !cell.exact_version.trim().is_empty(),
            "confirmed 格缺 CLI exact version({}/{})",
            cell.stage,
            cell.entrypoint
        );
        assert!(
            cell.argv_or_wire_capture_exists,
            "confirmed 格缺 argv/wire 捕获({}/{})",
            cell.stage,
            cell.entrypoint
        );
    }

    // M4 group/repair:边界段 coding 正向(受控 commit+正向宿主写)、
    // D4 前后快照格。
    let boundary_refs: Vec<&str> = boundary
        .cells()
        .iter()
        .map(|cell| cell.run_ref.as_str())
        .collect();
    assert!(
        boundary
            .cells()
            .iter()
            .any(|cell| cell.stage == "coding" && cell.capability_state == "confirmed"),
        "M4 group/repair:边界矩阵缺 coding 正向 confirmed 格"
    );
    assert!(
        boundary_refs.iter().any(|r#ref| r#ref.contains("d4")),
        "M4 group/repair:缺 D4 前后快照格({:?})",
        boundary_refs
    );
    // M5 失败关闭:17 固定负向场景逐格(boundary_matrix_evidence_flags 的
    // failure_scenario;真实零 spawn 三面证明由 harness 场景体与现场断言
    // 承担,此处结构面要求场景格计数与固定场景清单一致)。
    let failure_cells = boundary
        .cells()
        .iter()
        .filter(|cell| {
            cell.boundary_matrix_evidence_flags()
                .is_some_and(|flags| flags.contains_key("failure_scenario"))
        })
        .count();
    let expected_failures = super::failure_matrix::FIXED_FAILURE_SCENARIOS.len();
    assert_eq!(
        failure_cells, expected_failures,
        "M5 失败关闭:负向场景格数({failure_cells})必须等于固定清单({expected_failures}),不删格"
    );

    json!({
        "provider": format!("{provider:?}"),
        "run_mode": format!("{run_mode:?}"),
        "matrix_cells": matrix.cells().iter().map(b45_t23_cell_summary).collect::<Vec<_>>(),
        "boundary_cells": boundary.cells().iter().map(b45_t23_cell_summary).collect::<Vec<_>>(),
        "matrix_rejections": matrix
                .rejections
                .iter()
                .map(|rejection| json!({
                    "stage": rejection.stage,
                    "entrypoint": rejection.entrypoint,
                    "fresh_or_resume": rejection.fresh_or_resume,
                    "reason": rejection.reason,
                }))
                .collect::<Vec<_>>(),
        "duties": {
            "m1_author": ["story", "design", "plan(streaming+split_sync)"],
            "m2_coder": ["coding"],
            "m3_reviewer": ["review"],
            "m4_group_repair": boundary_refs,
            "m5_fail_closed": format!("{failure_cells} scenarios"),
            "m6_codex_danger": "b45_live_codex_danger_apply_patch",
            "m7_lifecycle": if resume_round { "resume(same native id)" } else { "fresh(spawn counting)" },
        },
    })
}

/// Step 3 真实执行:四家×六职责×Direct/LC×fresh/resume/retry 矩阵。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 3 命令执行(四家×六职责;需要真实 CLI/trust/#8 政策就绪;B45_T23_PROVIDERS 可分批)"]
async fn b45_live_four_provider_roles() {
    require_lc_gateway_e2e_switch();
    let providers = b45_t23_providers_from_env();
    assert!(
        !providers.is_empty(),
        "B45_T23_PROVIDERS 至少一家(claude/codex/pi/kimi)"
    );
    let mut summary = BTreeMap::<String, Value>::new();
    for provider in providers {
        // (a) 五阶段矩阵:M1/M2/M3/M7(Author/Coder/Reviewer/生命周期)。
        let matrix_root = b45_t23_evidence_root(&provider, "matrix");
        let matrix = match LiveLcGatewayHarness::new()
            .run_provider_matrix(provider.clone(), &matrix_root)
            .await
        {
            Ok(matrix) => matrix,
            Err(failure) => panic!(
                "run_provider_matrix 现场执行失败 [{}]{}: {} (环境不可运行须报告 BLOCKED,不删格)",
                failure.reason_code,
                failure
                    .stage
                    .map(|stage| format!("/{stage}"))
                    .unwrap_or_default(),
                failure.message
            ),
        };
        // (b) 边界+失败关闭:M4/M5(group/repair 与 17 负向场景)。
        let boundary = match LiveLcGatewayHarness::new()
            .run_boundary_and_failure_matrix(provider.clone())
            .await
        {
            Ok(boundary) => boundary,
            Err(failure) => panic!(
                "run_boundary_and_failure_matrix 现场执行失败 [{}]{}: {} (环境不可运行须报告 BLOCKED,不删格)",
                failure.reason_code,
                failure
                    .stage
                    .map(|stage| format!("/{stage}"))
                    .unwrap_or_default(),
                failure.message
            ),
        };
        let provider_summary = b45_t23_assert_role_coverage(&provider, &matrix, &boundary);
        summary.insert(format!("{provider:?}"), provider_summary);
    }
    // 汇总落盘(roles-matrix.json;不覆盖 cell.json 证据链)。
    let aggregate_root = PathBuf::from("cadence/reports/lc-gateway-multi-provider");
    std::fs::create_dir_all(&aggregate_root).expect("b45 t23 汇总目录");
    std::fs::write(
        aggregate_root.join("b45-t23-roles-matrix.json"),
        serde_json::to_vec_pretty(&json!({
            "providers": summary,
            "m6_codex_danger": "b45_live_codex_danger_apply_patch",
        }))
        .expect("roles matrix json"),
    )
    .expect("roles-matrix.json 落盘");
}

// ---------------------------------------------------------------------------
// M6:codex danger-full-access 双审批真机探针(on-request/never)
// ---------------------------------------------------------------------------

/// codex danger 探针 fixture 产物根(计划:测试 fixture 在 /tmp/aria-bug45/t23/)。
const B45_T23_FIXTURE_ROOT: &str = "/tmp/aria-bug45/t23";

/// 受控 apply_patch 目标文件内容(前后标记,供 diff 观察)。
const PATCH_TARGET_BEFORE: &str = "before-b45-t23\n";
const PATCH_TARGET_AFTER: &str = "after-b45-t23\n";

/// 危险档位标签(固定 CLI 版本下分别执行)。
#[derive(Clone, Copy, PartialEq, Eq)]
enum B45DangerProfile {
    /// 岗位纪律档:deny 投影恒 on-request(T13 检测型纪律)。
    OnRequest,
    /// Coder 原生档:Auto 恒 never(REQ-LCG-04 改版)。
    Never,
}

impl B45DangerProfile {
    fn label(self) -> &'static str {
        match self {
            Self::OnRequest => "danger-onrequest",
            Self::Never => "danger-never",
        }
    }
    fn permission_mode(self) -> ProviderPermissionMode {
        match self {
            // 纪律档 approvalPolicy 恒 on-request(codex_launch_params);
            // permission_mode 取值不影响该档映射。never 档 Auto→never。
            Self::OnRequest | Self::Never => ProviderPermissionMode::Auto,
        }
    }
    fn tool_policy(self) -> Option<ProviderToolPolicy> {
        match self {
            Self::OnRequest => Some(ProviderToolPolicy::deny_file_write_builtins()),
            Self::Never => None,
        }
    }
}

/// 事件流观察记录(审批帧/fileChange/中断/durable 的原始事实)。
#[derive(Default)]
struct B45DangerObservation {
    permission_requests: Vec<String>,
    file_change_warnings: Vec<String>,
    tool_policy_decisions: Vec<String>,
    executions: usize,
    text_deltas: usize,
    terminal: Option<String>,
    event_log: Vec<Value>,
}

/// 建隔离 fixture:小 git 仓+受控 apply_patch 目标文件。
fn b45_danger_fixture(label: &str) -> PathBuf {
    let root = PathBuf::from(B45_T23_FIXTURE_ROOT).join(label);
    // 每次真机轮重建隔离 fixture(避免上次残留导致空提交失败)。
    if root.exists() {
        std::fs::remove_dir_all(&root).expect("b45 danger fixture 旧目录清理");
    }
    std::fs::create_dir_all(&root).expect("b45 danger fixture dir");
    let repo = root.join("repo");
    std::fs::create_dir_all(&repo).expect("b45 danger repo dir");
    std::fs::write(repo.join("patch_target.md"), PATCH_TARGET_BEFORE)
        .expect("patch target before");
    for arguments in [
        vec![
            "init".to_string(),
            "-q".to_string(),
            "--initial-branch=main".to_string(),
        ],
        vec!["add".to_string(), "-A".to_string()],
        vec![
            "commit".to_string(),
            "-q".to_string(),
            "-m".to_string(),
            "b45 danger fixture baseline".to_string(),
        ],
    ] {
        let status = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo)
            .args([
                "-c",
                "user.email=b45-t23@example.invalid",
                "-c",
                "user.name=b45 t23 fixture",
            ])
            .args(&arguments)
            .status()
            .expect("b45 danger fixture git spawns");
        assert!(
            status.success(),
            "b45 danger fixture git {arguments:?} must succeed"
        );
    }
    repo
}

/// 解析 codex CLI 实际路径与 exact version(固定 CLI 版本记录)。
fn b45_codex_cli() -> (PathBuf, String) {
    let bin = PathBuf::from(
        std::env::var("B45_T23_CODEX_BIN").unwrap_or_else(|_| "codex".to_string()),
    );
    let output = std::process::Command::new(&bin)
        .arg("--version")
        .output()
        .expect("codex --version spawns(需 codex CLI 在 PATH 或 B45_T23_CODEX_BIN)");
    assert!(
        output.status.success(),
        "codex --version 失败:{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    assert!(!version.is_empty(), "codex --version 输出为空");
    (bin, version)
}

/// 单档真实执行:danger-full-access 下受控 apply_patch,收集审批有无/
/// fileChange 事实/中断/durable 的原始帧。
async fn b45_run_danger_profile(
    profile: B45DangerProfile,
    bin: &Path,
) -> (B45DangerObservation, PathBuf, PathBuf, Option<String>) {
    let repo = b45_danger_fixture(profile.label());
    let fixture_root = repo.parent().expect("fixture root").to_path_buf();
    // durable 审计落 fixture 内 .aria(不触碰真实 HOME/store)。
    let app_paths = ProductAppPaths::new(fixture_root.join(".aria"));
    let lifecycle = Arc::new(LifecycleStore::new(app_paths.clone()));
    let workspace_session_id = format!("b45t23{}", profile.label().trim_start_matches("danger-"));
    let input = StreamingProviderInput {
        provider_type: ProviderType::Codex,
        // 纪律档=Author 岗位承载角色(Orchestrator,deny 守卫唯一合法面
        // 之一);never 档=Coder 原生 Executor。
        role: match profile {
            B45DangerProfile::OnRequest => AdapterRole::Orchestrator,
            B45DangerProfile::Never => AdapterRole::Executor,
        },
        prompt: "Use the apply_patch tool to edit patch_target.md in the current \
                 directory: replace the line 'before-b45-t23' with 'after-b45-t23'. \
                 Then reply with exactly DONE."
            .to_string(),
        working_dir: repo.clone(),
        working_directory: None,
        workspace_session_id: Some(workspace_session_id.clone()),
        resume_provider_session_id: None,
        permission_mode: profile.permission_mode(),
        tool_policy: profile.tool_policy(),
        audit_sink: Some(
            RoleRunBoundAuditSink::new(lifecycle.clone(), workspace_session_id.clone(), 1)
                .into_sink(),
        ),
        structured_output_contract: None,
        // adapter PATH 冻结契约(批2 Task 3):不回退宿主 PATH——显式
        // 注入当前进程 PATH,使 codex CLI 可解析。
        env_vars: BTreeMap::from([(
            "PATH".to_string(),
            std::env::var("PATH").unwrap_or_default(),
        )]),
        timeout_secs: 300,
        baseline_tree: None,
    };
    let adapter = CodexProvider::new(bin.to_path_buf());
    let cancel = CancellationToken::new();
    let start_outcome = tokio::time::timeout(
        std::time::Duration::from_secs(360),
        adapter.start(input, cancel),
    )
    .await
    .expect("b45 danger start 不得悬置(360s 界)");

    let mut observation = B45DangerObservation::default();
    let audit_partition = app_paths
        .root()
        .join(TOOL_POLICY_RUN_AUDIT_PARTITION)
        .join(&workspace_session_id);
    let start_error: Option<String> = match start_outcome {
        // 检测型纪律:fileChange 事实 → 事件 → durable → 中断 turn → start Err。
        Err(error) => {
            observation.terminal = Some(format!("start_error: {error}"));
            Some(format!("{error}"))
        }
        Ok(mut session) => {
            let deadline = std::time::Duration::from_secs(360);
            loop {
                let event = match tokio::time::timeout(deadline, session.events.recv()).await {
                    Ok(Some(event)) => event,
                    Ok(None) => break,
                    Err(_) => break,
                };
                let record = match &event {
                    ProviderEvent::TextDelta { content } => {
                        observation.text_deltas += 1;
                        json!({"kind": "text_delta", "chars": content.len()})
                    }
                    ProviderEvent::PermissionRequest(request) => {
                        observation
                            .permission_requests
                            .push(request.tool_name.clone());
                        json!({"kind": "permission_request", "tool": request.tool_name})
                    }
                    ProviderEvent::Execution(_) => {
                        observation.executions += 1;
                        json!({"kind": "execution"})
                    }
                    ProviderEvent::ToolCall(call) => {
                        json!({"kind": "tool_call", "tool": call.tool_name})
                    }
                    ProviderEvent::ToolResult(result) => {
                        json!({"kind": "tool_result", "tool_id": result.tool_use_id})
                    }
                    ProviderEvent::ToolPolicyWarning(warning) => {
                        observation
                            .file_change_warnings
                            .push(warning.reason_code.clone());
                        json!({
                            "kind": "tool_policy_warning",
                            "reason_code": warning.reason_code,
                            "method": warning.method,
                        })
                    }
                    ProviderEvent::ToolPolicyDecision(decision) => {
                        observation
                            .tool_policy_decisions
                            .push(decision.decision.to_string());
                        json!({
                            "kind": "tool_policy_decision",
                            "category": decision.category,
                            "decision": decision.decision,
                        })
                    }
                    ProviderEvent::ToolPolicyTerminated(termination) => {
                        json!({
                            "kind": "tool_policy_terminated",
                            "reason_code": termination.reason_code,
                        })
                    }
                    ProviderEvent::Completed(_) => {
                        observation.terminal = Some("completed".to_string());
                        json!({"kind": "completed"})
                    }
                    ProviderEvent::Failed { message } => {
                        observation.terminal = Some(format!("failed: {message}"));
                        json!({"kind": "failed", "message": message})
                    }
                    ProviderEvent::StatusChanged(status) => {
                        json!({"kind": "status", "status": format!("{status:?}")})
                    }
                    ProviderEvent::ProtocolError { code, message, .. } => {
                        json!({"kind": "protocol_error", "code": code, "message": message})
                    }
                    ProviderEvent::PermissionTimeout { permission_id } => {
                        json!({"kind": "permission_timeout", "permission_id": permission_id})
                    }
                    ProviderEvent::UsageReport(_) => json!({"kind": "usage"}),
                    ProviderEvent::TerminalActivity(_) => json!({"kind": "terminal_activity"}),
                    ProviderEvent::ChoiceRequest(choice) => {
                        json!({"kind": "choice_request", "source": format!("{:?}", choice.source)})
                    }
                };
                observation.event_log.push(record);
                if matches!(
                    event,
                    ProviderEvent::Completed(_)
                        | ProviderEvent::Failed { .. }
                        | ProviderEvent::ToolPolicyTerminated(_)
                ) {
                    break;
                }
            }
            None
        }
    };
    (observation, repo, audit_partition, start_error)
}

/// 读取 durable 审计分区全部 jsonl 行(原始 durable 事实)。
fn b45_read_audit_lines(partition: &Path) -> Vec<String> {
    let mut lines = Vec::new();
    let Ok(entries) = std::fs::read_dir(partition) else {
        return lines;
    };
    for entry in entries.flatten() {
        if entry.path().extension().is_some_and(|ext| ext == "jsonl") {
            if let Ok(content) = std::fs::read_to_string(entry.path()) {
                lines.extend(content.lines().map(str::to_string));
            }
        }
    }
    lines
}

/// Step 4 真实执行:固定 CLI 版本,danger-full-access 下 on-request/never
/// 两个隔离 fixture 受控 apply_patch。
#[tokio::test]
#[ignore = "b45 真实现场:LC_GATEWAY_E2E=1 且 Step 4 命令执行(需要 codex CLI 登录态;固定实际 CLI 版本)"]
async fn b45_live_codex_danger_apply_patch() {
    require_lc_gateway_e2e_switch();
    let (bin, version) = b45_codex_cli();

    // ---- 场景 A:on-request(岗位纪律档)----
    let (onrequest, onrequest_repo, onrequest_audit, onrequest_start_error) =
        b45_run_danger_profile(B45DangerProfile::OnRequest, &bin).await;
    let onrequest_diff =
        std::fs::read_to_string(onrequest_repo.join("patch_target.md")).unwrap_or_default();
    let onrequest_audit_lines = b45_read_audit_lines(&onrequest_audit);

    // ---- 场景 B:never(Coder 原生档)----
    let (never, never_repo, never_audit, never_start_error) =
        b45_run_danger_profile(B45DangerProfile::Never, &bin).await;
    let never_diff =
        std::fs::read_to_string(never_repo.join("patch_target.md")).unwrap_or_default();
    let never_audit_lines = b45_read_audit_lines(&never_audit);

    // 证据落盘(审批帧/fileChange/diff/中断/durable 全量原始事实)。
    let evidence_root = PathBuf::from(B45_T23_FIXTURE_ROOT);
    std::fs::create_dir_all(&evidence_root).expect("b45 danger evidence dir");
    let evidence = json!({
        "cli": {"bin": bin.display().to_string(), "version": version},
        "sandbox": "danger-full-access",
        "on_request": {
            "approval_policy": "on-request(discipline)",
            "start_error": onrequest_start_error,
            "terminal": onrequest.terminal,
            "permission_requests": onrequest.permission_requests,
            "file_change_warnings": onrequest.file_change_warnings,
            "tool_policy_decisions": onrequest.tool_policy_decisions,
            "executions": onrequest.executions,
            "text_deltas": onrequest.text_deltas,
            "patch_target_after": onrequest_diff,
            "patch_applied": onrequest_diff == PATCH_TARGET_AFTER,
            "durable_audit_lines": onrequest_audit_lines,
            "events": onrequest.event_log,
        },
        "never": {
            "approval_policy": "never",
            "start_error": never_start_error,
            "terminal": never.terminal,
            "permission_requests": never.permission_requests,
            "file_change_warnings": never.file_change_warnings,
            "tool_policy_decisions": never.tool_policy_decisions,
            "executions": never.executions,
            "text_deltas": never.text_deltas,
            "patch_target_after": never_diff,
            "patch_applied": never_diff == PATCH_TARGET_AFTER,
            "durable_audit_lines": never_audit_lines,
            "events": never.event_log,
        },
    });
    std::fs::write(
        evidence_root.join("codex-danger-apply-patch.json"),
        serde_json::to_vec_pretty(&evidence).expect("danger evidence json"),
    )
    .expect("danger evidence 落盘");

    // ---- 判绿(红线:事件不可观测或不能可靠中断=机制未通过)----
    // 场景 A(on-request 纪律):fileChange 必须可观测(事件通道 warning 或
    // FileChange 审批请求至少其一;danger 下 apply_patch 可能无审批帧,
    // item 通知是检测型触发面)。
    let onrequest_file_change_observable = !onrequest.file_change_warnings.is_empty()
        || onrequest
            .tool_policy_decisions
            .iter()
            .any(|decision| decision == "decline")
        || !onrequest.permission_requests.is_empty();
    assert!(
        onrequest_file_change_observable,
        "on-request 档 fileChange 不可观测(无 warning/审批帧/决策事件)——\
         该 CLI 版本({version})apply_patch 机制未通过,不以「审批没来所以被拒\
         成功」交差"
    );
    // 场景 A:检测型纪律必须 durable(file_change_detected 落审计分区)。
    assert!(
        onrequest_audit_lines
            .iter()
            .any(|line| line.contains("file_change_detected")),
        "on-request 档 durable 审计缺 file_change_detected(事件→durable→中断\
         次序断裂)"
    );
    // 场景 A:turn 必须被中断(终态=start Err 携带 author_file_change_
    // detected,或事件流 ToolPolicyTerminated/Failed)——本轮不可采信。
    let onrequest_interrupted = onrequest_start_error
        .as_deref()
        .is_some_and(|message| message.contains(AUTHOR_FILE_CHANGE_DETECTED))
        || onrequest
            .event_log
            .iter()
            .any(|event| event.get("kind").and_then(Value::as_str) == Some("tool_policy_terminated"))
        || onrequest
            .terminal
            .as_deref()
            .is_some_and(|terminal| terminal.starts_with("failed:"));
    assert!(
        onrequest_interrupted,
        "on-request 档未可靠中断(turn 未失败)——检测型纪律红线;start_error=\
         {:?}, terminal={:?}",
        onrequest_start_error,
        onrequest.terminal
    );
    // 场景 B(never):审批必须缺席(never 档无审批请求;出现即映射失真)。
    assert!(
        never.permission_requests.is_empty(),
        "never 档出现审批请求({:?})——approvalPolicy=never 映射失真",
        never.permission_requests
    );
    // 场景 B:fileChange 可观测性——apply_patch 生效(diff 变更)或事件流
    // 有工具/执行活动;两者皆无=事件不可观测,该版本机制未通过。
    let never_observable = never_diff == PATCH_TARGET_AFTER
        || never.executions > 0
        || !never.event_log.is_empty();
    assert!(
        never_observable,
        "never 档 apply_patch 完全不可观测(无 diff、无事件)——该 CLI 版本\
         ({version})机制未通过"
    );
    // 场景 B:非纪律会话不中断(终态完成;若 Failed 如实记录,由判读面
    // 裁决,此处要求事件事实完整落盘——已由 evidence 落盘承载)。
}
