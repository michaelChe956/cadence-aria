// B-③（3.6 矩阵族③根因修复）：coder 完成报告 plan_defect_findings 解析失败
// 后的「恰一次教学重驱」行为面测试。族③现场（issue_0149）coder 仅把
// defect_class 写成非法枚举值 plan_defect，其余字段全部合法——本轮应在挂
// coding_output_human_triage 人工门前，先复用 resume 会话做一次错误原文回灌
// 的教学续跑；重驱输出合法则正常流转，仍不合法才落既有 blocked 门，且同一
// attempt 绝不重驱第二次（timeline marker 为持久恰一次闸门，防重入/崩溃后
// 二次消耗预算）。
use super::*;
use crate::cross_cutting::streaming_provider::{
    ProviderCompletion, ProviderEvent, ProviderSession,
};
use std::collections::VecDeque;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const TEACHING_REREDRIVE_NODE_TITLE: &str = "Coding 输出结构化教学重驱";

/// 逐次返回预置输出的 scripted provider：第 N 次 start 弹出第 N 条输出，
/// 并记录每次收到的 StreamingProviderInput 供断言（教学 prompt、resume
/// session 续接等）。
struct ScriptedOutputsProvider {
    outputs: Mutex<VecDeque<String>>,
    starts: AtomicUsize,
    inputs: Mutex<Vec<StreamingProviderInput>>,
}

impl ScriptedOutputsProvider {
    fn new(outputs: Vec<&str>) -> Self {
        Self {
            outputs: Mutex::new(outputs.into_iter().map(str::to_string).collect()),
            starts: AtomicUsize::new(0),
            inputs: Mutex::new(Vec::new()),
        }
    }

    fn recorded_inputs(&self) -> Vec<StreamingProviderInput> {
        self.inputs.lock().expect("provider inputs").clone()
    }
}

#[async_trait::async_trait]
impl StreamingProviderAdapter for ScriptedOutputsProvider {
    async fn start(
        &self,
        input: StreamingProviderInput,
        _cancel: tokio_util::sync::CancellationToken,
    ) -> Result<ProviderSession, ProviderAdapterError> {
        let start_no = self.starts.fetch_add(1, Ordering::SeqCst) + 1;
        let output = self
            .outputs
            .lock()
            .expect("provider outputs")
            .pop_front()
            .unwrap_or_else(|| "{\"plan_defect_findings\": []}".to_string());
        self.inputs
            .lock()
            .expect("provider inputs")
            .push(input.clone());
        let (event_tx, event_rx) = mpsc::channel(2);
        let (command_tx, _command_rx) = mpsc::channel(2);
        tokio::spawn(async move {
            let event = ProviderEvent::Completed(ProviderCompletion::from_output(
                output,
                input.structured_output_contract.as_ref(),
                Some(format!("provider-session-{start_no}")),
            ));
            let _ = event_tx.send(event).await;
        });
        Ok(ProviderSession {
            native_session_id: None,
            events: event_rx,
            commands: command_tx,
        })
    }
}

/// 族③现场同款非法输出：defect_class 使用未知变体 plan_defect，其余字段合法。
fn invalid_plan_defect_output() -> &'static str {
    "{\"plan_defect_findings\": [{\"finding_id\": \"coder_plan_defect_0001\", \
     \"severity\": \"error\", \"defect_class\": \"plan_defect\", \
     \"reason_code\": \"upstream_plan_invalid\", \
     \"message\": \"上游计划契约无法满足\", \"evidence\": [], \"contract_refs\": [], \
     \"capability_refs\": [], \"repair_target\": null, \
     \"recommended_route\": \"plan_repair\", \"confidence\": \"low\"}]}"
}

fn teaching_reredrive_node_count(
    store: &CodingAttemptStore,
    attempt: &CodingExecutionAttempt,
) -> usize {
    store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes")
        .into_iter()
        .filter(|node| node.title == TEACHING_REREDRIVE_NODE_TITLE)
        .count()
}

#[tokio::test]
async fn coder_invalid_plan_defect_output_gets_single_teaching_reredrive_and_recovers() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ScriptedOutputsProvider::new(vec![
        invalid_plan_defect_output(),
        "{\"plan_defect_findings\": []}",
    ]);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let persisted = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect("教学重驱后输出合法，coding 正常完成");

    // 恰一次教学重驱：第一轮 + 重驱轮 = 2 次 provider 调用。
    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    assert_ne!(persisted.status, CodingAttemptStatus::Blocked);

    let inputs = provider.recorded_inputs();
    assert_eq!(inputs.len(), 2);
    // 第一轮是全新会话；重驱轮必须复用第一轮落地的 provider session 续会话。
    assert_eq!(inputs[0].resume_provider_session_id, None);
    assert_eq!(
        inputs[1].resume_provider_session_id.as_deref(),
        Some("provider-session-1")
    );
    // 重驱 prompt：错误原文逐字回灌 + 立即重新输出指令 + 共享单源契约
    //（含 defect_class 8 取值逐字枚举，A 件）。
    for required in [
        "上一轮已结束，但你没有输出合法的 plan_defect_findings",
        "plan_defect_finding_invalid: unknown variant `plan_defect`",
        "立即重新输出完整完成报告",
        "defect_class 只能逐字使用以下 8 个取值之一",
    ] {
        assert!(
            inputs[1].prompt.contains(required),
            "教学重驱 prompt 必须包含 {required}: {}",
            inputs[1].prompt
        );
    }

    // 无 blocked 门，attempt 未被阻塞。
    assert!(
        store
            .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("open gates")
            .is_empty()
    );
    assert_ne!(
        store
            .get_attempt(&attempt.project_id, &attempt.issue_id, &attempt.id)
            .expect("persisted attempt")
            .status,
        CodingAttemptStatus::Blocked
    );

    // 审计留痕：教学重驱 timeline node 恰一个且已完成。
    let reredrive_nodes = store
        .get_timeline_nodes(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("timeline nodes")
        .into_iter()
        .filter(|node| node.title == TEACHING_REREDRIVE_NODE_TITLE)
        .collect::<Vec<_>>();
    assert_eq!(reredrive_nodes.len(), 1);
    assert_eq!(
        reredrive_nodes[0].status,
        CodingTimelineNodeStatus::Completed
    );

    // 两轮 coder role run 均为 Completed，重驱轮 trigger=AutomaticRetry。
    let coder_runs = store
        .list_role_runs(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("role runs")
        .into_iter()
        .filter(|run| {
            run.stage == CodingExecutionStage::Coding && run.role == CodingProviderRole::Coder
        })
        .collect::<Vec<_>>();
    assert_eq!(coder_runs.len(), 2);
    assert_eq!(coder_runs[0].trigger, CodingRoleRunTrigger::Initial);
    assert_eq!(coder_runs[0].status, CodingRoleRunStatus::Completed);
    assert_eq!(coder_runs[1].trigger, CodingRoleRunTrigger::AutomaticRetry);
    assert_eq!(coder_runs[1].status, CodingRoleRunStatus::Completed);
}

#[tokio::test]
async fn coder_teaching_reredrive_still_invalid_lands_human_triage_gate_exactly_once() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ScriptedOutputsProvider::new(vec![
        invalid_plan_defect_output(),
        invalid_plan_defect_output(),
    ]);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let persisted = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect("重驱后仍非法走既有 blocked 门语义");

    // 恰一次：两轮输出均非法也只重驱一次，绝不第三次调用 provider。
    assert_eq!(provider.starts.load(Ordering::SeqCst), 2);
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);

    // 既有 blocked 门语义零变化：coding_output_human_triage，描述含最新解析错误。
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("coding_output_human_triage")
    );
    assert!(
        gates[0]
            .description
            .contains("plan_defect_finding_invalid: unknown variant `plan_defect`"),
        "gate 描述必须携带解析错误原文: {}",
        gates[0].description
    );
    assert_eq!(
        teaching_reredrive_node_count(&store, &attempt),
        1,
        "教学重驱 timeline node 恰一个"
    );
}

#[tokio::test]
async fn coder_teaching_reredrive_marker_prevents_second_reredrive_on_reentry() {
    let (_root, store, attempt) = running_attempt_with_worktree();
    // 预置教学重驱 marker 节点（模拟此前轮次已重驱/崩溃后重入），持久恰一次
    // 闸门必须直接走既有 blocked 门，不再触发任何教学重驱。
    store
        .save_timeline_node(
            &attempt,
            CodingTimelineNode {
                id: "coding_node_0001".to_string(),
                attempt_id: attempt.id.clone(),
                stage: CodingExecutionStage::Coding,
                title: TEACHING_REREDRIVE_NODE_TITLE.to_string(),
                status: CodingTimelineNodeStatus::Completed,
                agent_role: Some(CodingAgentRole::Author),
                summary: None,
                started_at: "2026-09-04T00:00:00Z".to_string(),
                completed_at: Some("2026-09-04T00:00:01Z".to_string()),
                artifact_refs: Vec::new(),
            },
        )
        .expect("seed reredrive marker node");

    let (tx, _rx) = mpsc::channel(16);
    let engine = CodingWorkspaceEngine::new(store.clone(), GitWorkspaceService::new(), tx);
    let provider = ScriptedOutputsProvider::new(vec![invalid_plan_defect_output()]);
    let (_command_tx, mut command_rx) = mpsc::channel(1);

    let persisted = engine
        .execute_coding_with_commands(
            &attempt,
            &provider,
            &CodingExecutionContext::default(),
            &mut command_rx,
        )
        .await
        .expect("marker 存在时直接落 blocked 门");

    assert_eq!(provider.starts.load(Ordering::SeqCst), 1);
    assert_eq!(persisted.status, CodingAttemptStatus::Blocked);
    let gates = store
        .list_open_blocked_gates(&attempt.project_id, &attempt.issue_id, &attempt.id)
        .expect("open gates");
    assert_eq!(gates.len(), 1);
    assert_eq!(
        gates[0].reason_code.as_deref(),
        Some("coding_output_human_triage")
    );
    assert_eq!(teaching_reredrive_node_count(&store, &attempt), 1);
}
