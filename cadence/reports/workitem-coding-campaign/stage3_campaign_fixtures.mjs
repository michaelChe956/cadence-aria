// 阶段 3 campaign driver 的纯 transcript / fault fixture。
//
// 约束（task-8.1 brief）：本文件只承载合成事件序列与脚本样本，
// 不得包含 credential、真实 prompt 或任何真实运行数据；反馈文本一律使用
// 合成占位（“合成反馈 N 号”），仅用于验证 driver 侧编码/消费逻辑。
//
// 服务端事件 wire 形态对齐 src/web/workspace_ws_types/out.rs：
//   human_gate_turn_open { turn_id, command_id, remaining_budget }
//   human_gate_turn_completed { turn_id, artifact_ref }
//   human_gate_turn_failed { turn_id, failure_class, message }
//   human_gate_busy { turn_id }
//   human_gate_closed { decision, stage }
//   advance_completed { command_id, attempt_id, workspace_entry }
//   advance_rejected { command_id, code, reason }

export const STAGE3_SCRIPT_GRAMMAR_VERSION = '3';

export const STAGE3_SAMPLE_SCRIPT = 'request-change:合成反馈 1 号;request-change:合成反馈 2 号;confirm;advance';

export const STAGE3_SAMPLE_ACTIONS = [
  { decision: 'request-change', description: '合成反馈 1 号' },
  { decision: 'request-change', description: '合成反馈 2 号' },
  { decision: 'confirm', description: null },
  { decision: 'advance', description: null },
];

// 基准 campaign 身份（command id 确定性断言用）。
export const STAGE3_SAMPLE_CAMPAIGN_RUN_ID = 'workitem:codex:rep1:issue_fixture_0001';

export function stage3TurnOpen(commandId, turnId = 'turn-1', remainingBudget = 2) {
  return { type: 'human_gate_turn_open', command_id: commandId, turn_id: turnId, remaining_budget: remainingBudget };
}

export function stage3TurnCompleted(turnId = 'turn-1', artifactRef = 'artifact://fixture/candidate-v2') {
  return { type: 'human_gate_turn_completed', turn_id: turnId, artifact_ref: artifactRef };
}

export function stage3TurnFailed(turnId = 'turn-1', failureClass = 'provider_err', message = '合成失败原因') {
  return { type: 'human_gate_turn_failed', turn_id: turnId, failure_class: failureClass, message };
}

export function stage3GateBusy(turnId = 'turn-1') {
  return { type: 'human_gate_busy', turn_id: turnId };
}

export function stage3GateClosed(decision = 'confirm', stage = 'human_confirm') {
  return { type: 'human_gate_closed', decision, stage };
}

export function stage3AdvanceCompleted(commandId, attemptId = 'attempt_fixture_0001') {
  return {
    type: 'advance_completed',
    command_id: commandId,
    attempt_id: attemptId,
    workspace_entry: '/api/workspace-sessions/fixture-session-0001/ws',
  };
}

export function stage3AdvanceRejected(commandId, code = 'ADVANCE_PRECONDITION_FAILED', reason = '合成拒绝原因') {
  return { type: 'advance_rejected', command_id: commandId, code, reason };
}

// 完整多轮 happy-path transcript：两轮 request-change → confirm → Confirmed → advance。
// `commandIds` 依次对应 controller 为 script 动作 materialize 的 command id。
export function stage3HappyPathTranscript(commandIds) {
  const [feedback1, feedback2, , advance] = commandIds;
  return [
    stage3TurnOpen(feedback1, 'turn-1', 2),
    stage3TurnCompleted('turn-1'),
    stage3TurnOpen(feedback2, 'turn-2', 1),
    stage3TurnCompleted('turn-2'),
    stage3GateClosed('confirm'),
    stage3AdvanceCompleted(advance),
  ];
}

// 重连故障 transcript：turn_open 丢失 → ws close → 重连后同 command_id 的
// durable replay（session_state 恢复面）→ turn 完成。
export function stage3ReconnectFaultTranscript({ commandId, replayedCommandIds }) {
  return {
    beforeClose: [
      stage3GateBusy('turn-unknown'),
    ],
    afterReconnect: {
      durableState: { replayedCommandIds },
      events: [
        stage3TurnOpen(commandId, 'turn-1', 2),
        stage3TurnCompleted('turn-1'),
      ],
    },
  };
}

// 8.2a takeover fixture：幂等 takeover 端点响应 + child 侧门事件 transcript。
// 两次调用返回同一 child/takeover_event（幂等语义），parent 证据不动。
export function stage3TakeoverResponse(parentSessionId = 'ws_fixture_parent_0001') {
  return {
    workspace_session: {
      workspace_session_id: 'ws_fixture_child_0001',
      session_id: 'ws_fixture_child_0001',
      run_policy: 'interactive',
      flow_kind: 'single_candidate',
      session_status: 'waiting_for_human',
    },
    parent_session_id: parentSessionId,
    takeover_event_id: 'takeover_event_fixture_0001',
  };
}

// takeover 之后的 child WS transcript：parent 侧已发生的 turn 审计保留，
// child 门重新 Waiting，typed feedback 以同 campaign 身份继续。
export function stage3TakeoverChildTranscript(commandId) {
  return [
    { type: 'human_gate_turn_open', turn_id: 'child-turn-1', command_id: commandId, remaining_budget: 1 },
    { type: 'human_gate_turn_completed', turn_id: 'child-turn-1', artifact_ref: 'artifact://fixture/candidate-child-v2' },
    stage3GateClosed('confirm'),
  ];
}

// provider-start ledger fixture（advance 前后必须 byte 级不变）。
export function stage3ProviderStartLedgerFixture() {
  return [
    { provider_start_idempotency_key: 'fixture-provider-start-1', started: true },
    { provider_start_idempotency_key: 'fixture-provider-start-2', started: true },
  ];
}

// 8.3 advance 完成后的 group attempt snapshot fixture(GET coding-attempts/{id} 形态,
// 对齐 CodingAttemptSnapshotResponse 的脱敏子集:attempt/units/group_coding_progress/
// group_progress)。纯合成数据,不含真实运行载荷。
export function stage3GroupAttemptSnapshotFixture() {
  const attemptId = 'attempt_fixture_0001';
  const units = [
    { logical_work_item_id: 'wi_a', status: 'running' },
    { logical_work_item_id: 'wi_b', status: 'pending' },
  ];
  return {
    attempt: {
      attempt_id: attemptId,
      attempt_scope: 'work_item_group',
      status: 'created',
      stage: 'coding',
      work_item_group_id: 'work_item_plan_0001',
      current_work_item_id: 'wi_a',
    },
    units: units.map((unit) => ({
      unit_id: `coding_unit_${unit.logical_work_item_id}`,
      logical_work_item_id: unit.logical_work_item_id,
      status: unit.status,
      completion_commit: null,
      latest_handoff_revision_id: null,
    })),
    group_coding_progress: units.map((unit) => ({
      logical_work_item_id: unit.logical_work_item_id,
      status: unit.status,
      plan_revision_id: 'plan_revision_fixture_0001',
    })),
    group_progress: { total: 2, pending: 1, active: 1, completed: 0, failed_or_blocked: 0 },
    // advance record fixture:readback evidence 的 record 事实来源。
    advance_record: { id: 'advance_fixture_0001', status: 'ready' },
    // issue shared worktree lock fixture(lock owner = attempt)。
    issue_shared_worktree: {
      current_active_work_item_id: 'wi_a',
      current_lock_owner_id: attemptId,
    },
  };
}
