// 阶段 3 Task 8.1 —— campaign driver 的 typed 人工门与 advance 模拟测试。
//
// 纯 Node：不启动服务、不发真实网络请求、不要求 credential；
// 通过 fixture transcript 驱动 driver 的 stage-3 gate controller。
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import test from 'node:test';

import {
  advanceSimulationAction,
  campaignCommandId,
  createStage3GateController,
  parseHumanScript,
  resultTemplate,
  shouldConsumeHumanAction,
  singleCandidateOutboundAllowed,
  stage3FeedbackDigest,
  stage3HumanMessage,
  stage3OutboundLogEntry,
  stage3TypedFlowActive,
} from './workitem_run_campaign.mjs';
import {
  STAGE3_SAMPLE_ACTIONS,
  STAGE3_SAMPLE_CAMPAIGN_RUN_ID,
  STAGE3_SAMPLE_SCRIPT,
  stage3AdvanceCompleted,
  stage3GateBusy,
  stage3GateClosed,
  stage3HappyPathTranscript,
  stage3ProviderStartLedgerFixture,
  stage3TurnCompleted,
  stage3TurnFailed,
  stage3TurnOpen,
} from './stage3_campaign_fixtures.mjs';

const sha256 = (text) => createHash('sha256').update(text, 'utf8').digest('hex');

function newController({ actions, checkpoint = null } = {}) {
  const persisted = [];
  const controller = createStage3GateController({
    actions: actions ?? parseHumanScript(STAGE3_SAMPLE_SCRIPT),
    campaignRunId: STAGE3_SAMPLE_CAMPAIGN_RUN_ID,
    checkpoint,
    persistCheckpoint: (next) => persisted.push(structuredClone(next)),
  });
  return { controller, persisted };
}

test('campaign_stage3_feedback_uses_typed_message_and_stable_command_id', () => {
  assert.equal(stage3TypedFlowActive('single_candidate', 'interactive'), true);
  assert.equal(stage3TypedFlowActive('legacy', 'interactive'), false);
  assert.equal(stage3TypedFlowActive('single_candidate', 'auto_if_valid'), false);

  const feedbackAction = STAGE3_SAMPLE_ACTIONS[0];
  const typed = stage3HumanMessage(feedbackAction, { commandId: 'cmd-fixture-0' });
  assert.deepEqual(typed, {
    type: 'human_gate_feedback',
    command_id: 'cmd-fixture-0',
    feedback: '合成反馈 1 号',
  });
  assert.notEqual(typed.decision, 'request-change', 'SC typed flow 绝不再编码 HumanConfirmDecision::RequestChange');
  assert.deepEqual(stage3HumanMessage({ decision: 'confirm', description: null }, { commandId: null }), {
    type: 'human_confirm',
    decision: 'confirm',
    payload: null,
  });
  assert.deepEqual(stage3HumanMessage({ decision: 'abandon', description: null }, { commandId: null }), {
    type: 'human_confirm',
    decision: 'terminate',
    payload: null,
  });
  assert.deepEqual(stage3HumanMessage({ decision: 'advance', description: null }, { commandId: 'cmd-adv-0' }), {
    type: 'advance',
    command_id: 'cmd-adv-0',
  });
  assert.throws(() => stage3HumanMessage(feedbackAction, { commandId: null }), /commandId/u);
  assert.throws(() => stage3HumanMessage({ decision: 'advance', description: null }, { commandId: null }), /commandId/u);

  const base = { campaignRunId: STAGE3_SAMPLE_CAMPAIGN_RUN_ID, actionIndex: 0, kind: 'human_gate_feedback' };
  const first = campaignCommandId(base);
  assert.equal(first, campaignCommandId({ ...base }), '同一 run 身份 + index + kind 必须生成稳定 command id');
  assert.notEqual(first, campaignCommandId({ ...base, actionIndex: 1 }));
  assert.notEqual(first, campaignCommandId({ ...base, kind: 'advance' }));
  assert.notEqual(first, campaignCommandId({ ...base, campaignRunId: 'workitem:codex:rep1:issue_other' }));
  assert.throws(() => campaignCommandId({ ...base, campaignRunId: '' }), /campaignRunId/u);
  assert.throws(() => campaignCommandId({ ...base, actionIndex: -1 }), /actionIndex/u);
  assert.throws(() => campaignCommandId({ ...base, actionIndex: 1.5 }), /actionIndex/u);
  assert.throws(() => campaignCommandId({ ...base, kind: 'human_confirm' }), /kind/u);

  assert.equal(singleCandidateOutboundAllowed({ type: 'human_gate_feedback' }, 'interactive'), true);
  assert.equal(singleCandidateOutboundAllowed({ type: 'advance' }, 'interactive'), true);
  assert.equal(singleCandidateOutboundAllowed({ type: 'human_gate_feedback' }, 'auto_if_valid'), false);
  assert.equal(singleCandidateOutboundAllowed({ type: 'advance' }, 'auto_if_valid'), false);
  assert.equal(singleCandidateOutboundAllowed({ type: 'human_confirm' }, 'interactive'), true);
  assert.equal(singleCandidateOutboundAllowed({ type: 'start_generation' }, 'auto_if_valid'), true);
  assert.equal(singleCandidateOutboundAllowed({ type: 'review_decision_response' }, 'interactive'), false,
    'legacy SC 决策族依旧不允许出站');
});

test('campaign_stage3_reconnect_reuses_command_id_without_consuming_script_twice', () => {
  const { controller, persisted } = newController();

  // 门 Waiting：发出第一轮 typed feedback，command id 立即写 checkpoint。
  const firstSend = controller.onGateWaiting();
  const firstCommandId = firstSend.commandId;
  assert.equal(firstSend.message.type, 'human_gate_feedback');
  assert.equal(firstSend.message.feedback, '合成反馈 1 号');
  assert.ok(firstCommandId, 'typed feedback 必须携带 command id');
  assert.ok(persisted.length >= 1, 'command id materialize 时必须立即持久化 checkpoint');
  assert.ok(JSON.stringify(persisted[0]).includes(firstCommandId));

  // 非本 command id 的 turn_open（他端回放）不得消费当前动作。
  const foreignOpen = controller.onInbound(stage3TurnOpen('cmd-not-mine', 'turn-x', 2));
  assert.deepEqual(foreignOpen.consumed, []);
  assert.equal(foreignOpen.outbound, null);

  // turn 进行中的 busy 不消费、也不推进脚本。
  const busy = controller.onInbound(stage3GateBusy('turn-x'));
  assert.deepEqual(busy.consumed, []);

  // 模拟进程重启：同 checkpoint 载入新 controller，重发必须复用同一 command id，
  // 且脚本指针仍停留在第 0 条（未被重复消费）。
  const restarted = createStage3GateController({
    actions: parseHumanScript(STAGE3_SAMPLE_SCRIPT),
    campaignRunId: STAGE3_SAMPLE_CAMPAIGN_RUN_ID,
    checkpoint: persisted.at(-1),
    persistCheckpoint: () => {},
  });
  const resend = restarted.onGateWaiting({ reconnect: true });
  assert.equal(resend.message.type, 'human_gate_feedback');
  assert.equal(resend.commandId, firstCommandId, '重连重发必须复用原 command id');
  assert.deepEqual(restarted.resultFields().durable_recovery_checks.map((entry) => entry.check),
    ['checkpoint_command_id_reused']);

  // durable replay（同 command id 的恢复面）才允许消费 request-change。
  const replayConsumed = restarted.onDurableState({ replayedCommandIds: [firstCommandId] });
  assert.deepEqual(replayConsumed.consumed, [{ actionIndex: 0, kind: 'human_gate_feedback', via: 'durable_replay' }]);

  // 重复 turn_open / 重复 completed 不重复消费。
  const duplicate = restarted.onInbound(stage3TurnOpen(firstCommandId, 'turn-1', 2));
  assert.deepEqual(duplicate.consumed, [], '重复 turn_open 不重复消费');

  // 全链路 happy path：连续多轮 request-change → confirm，同一 gate 不再受单次应答限制。
  const { controller: full } = newController();
  const sent = [full.onGateWaiting()];
  const commandIds = sent.map((entry) => entry.commandId);
  commandIds.push(campaignCommandId({
    campaignRunId: STAGE3_SAMPLE_CAMPAIGN_RUN_ID,
    actionIndex: 1,
    kind: 'human_gate_feedback',
  }), null, campaignCommandId({
    campaignRunId: STAGE3_SAMPLE_CAMPAIGN_RUN_ID,
    actionIndex: 3,
    kind: 'advance',
  }));
  for (const event of stage3HappyPathTranscript(commandIds)) {
    full.onInbound(event);
  }
  const fields = full.resultFields();
  assert.equal(fields.human_gate_turns.length, 2, '两轮 request-change 各留一条 turn 审计');
  assert.equal(fields.advance_actions.length, 1);
  assert.equal(fields.advance_actions[0].status, 'completed');
  for (const turn of fields.human_gate_turns) {
    assert.match(turn.feedback_digest, /^[0-9a-f]{64}$/u);
    assert.equal(typeof turn.feedback_length, 'number');
  }
  assert.doesNotMatch(JSON.stringify(fields), /合成反馈/u, '审计只留 digest/长度，不留反馈全文');

  // 首个 controller 的独立事件流也必须保持一致的单飞约束：
  // in-flight 期间 onGateWaiting 不发下一动作。
  const { controller: singleFlight } = newController({ actions: parseHumanScript('request-change:合成反馈 1 号;request-change:合成反馈 2 号') });
  const send1 = singleFlight.onGateWaiting();
  singleFlight.onInbound(stage3TurnOpen(send1.commandId, 'turn-1', 1));
  assert.equal(singleFlight.onGateWaiting(), null, 'turn 未终态（in-flight）期间不得发送下一动作');
  const failedOutcome = singleFlight.onInbound(stage3TurnFailed('turn-1', 'provider_err', '合成失败原因'));
  assert.ok(failedOutcome.outbound, '回合终态（含失败）后门回到 Waiting，应给出下一脚本动作');
  const send2 = failedOutcome.outbound;
  assert.equal(send2.message.feedback, '合成反馈 2 号');
  assert.notEqual(send2.commandId, send1.commandId);
});

test('campaign_stage3_advance_waits_for_durable_confirmed', () => {
  const advanceAction = { decision: 'advance', description: null };
  assert.equal(advanceSimulationAction({ confirmedPlan: false, existingAdvance: null, action: advanceAction }), 'wait');
  assert.equal(advanceSimulationAction({ confirmedPlan: false, existingAdvance: null, action: null }), 'wait');
  assert.equal(advanceSimulationAction({ confirmedPlan: true, existingAdvance: null, action: null }), 'wait');
  assert.equal(advanceSimulationAction({ confirmedPlan: true, existingAdvance: null, action: advanceAction }), 'send');
  const existing = { command_id: 'cmd-adv-existing', attempt_id: 'attempt_fixture_0001' };
  assert.equal(advanceSimulationAction({ confirmedPlan: true, existingAdvance: existing, action: advanceAction }), 'complete');
  assert.equal(advanceSimulationAction({ confirmedPlan: false, existingAdvance: existing, action: advanceAction }), 'complete',
    '已有 durable advance 记录（幂等命中）优先于前置校验');

  // Confirmed 回读前，advance 动作不得发送。
  const { controller } = newController({ actions: parseHumanScript('confirm;advance') });
  const confirmSend = controller.onGateWaiting();
  assert.equal(confirmSend.message.decision, 'confirm');
  assert.equal(controller.onConfirmedDurable({ providerStartLedger: stage3ProviderStartLedgerFixture() }).decision, null,
    'confirm 尚未被 human_gate_closed 消费时不应推进到 advance');
  controller.onInbound(stage3GateClosed('confirm'));
  const premature = controller.onConfirmedDurable({ providerStartLedger: stage3ProviderStartLedgerFixture() });
  assert.equal(premature.decision, 'send');
  assert.equal(premature.outbound.type, 'advance');
  assert.ok(premature.commandId);

  // advance_completed 消费动作；provider ledger 前后不变（advance 不启动 provider）。
  const outcome = controller.onInbound(stage3AdvanceCompleted(premature.commandId));
  assert.deepEqual(outcome.consumed, [{ actionIndex: 1, kind: 'advance', via: 'advance_completed' }]);
  controller.noteProviderLedgerAfter(stage3ProviderStartLedgerFixture());
  const fields = controller.resultFields();
  assert.deepEqual(
    fields.provider_start_ledger_before_after.before,
    fields.provider_start_ledger_before_after.after,
    'advance 模拟不得改变 provider-start ledger',
  );
  assert.equal(fields.advance_actions[0].attempt_id, 'attempt_fixture_0001');

  // 无 advance 脚本：Confirmed 后保持阶段 2 handoff finish 行为。
  const { controller: noAdvance } = newController({ actions: parseHumanScript('confirm') });
  noAdvance.onGateWaiting();
  noAdvance.onInbound(stage3GateClosed('confirm'));
  assert.equal(noAdvance.onConfirmedDurable({ providerStartLedger: stage3ProviderStartLedgerFixture() }).decision, null);

  // 消费判定 helper：advance 只有 advance_completed/同 record replay 才消费。
  const advanceWithCommand = { decision: 'advance', commandId: 'cmd-adv-0' };
  assert.equal(shouldConsumeHumanAction(stage3AdvanceCompleted('cmd-adv-0'), {}, advanceWithCommand), true);
  assert.equal(
    shouldConsumeHumanAction(stage3AdvanceCompleted('cmd-other'), {}, advanceWithCommand),
    false,
  );
  assert.equal(
    shouldConsumeHumanAction({ type: 'advance_rejected', command_id: 'cmd-adv-0', code: 'X', reason: 'r' }, {}, advanceWithCommand),
    false,
    'advance_rejected 不消费脚本动作',
  );
  assert.equal(
    shouldConsumeHumanAction(
      { type: 'session_state' },
      { advanceRecords: [{ command_id: 'cmd-adv-0' }] },
      advanceWithCommand,
    ),
    true,
    '同 record 的 durable replay 才消费 advance',
  );
});

test('campaign_stage3_gate_events_never_consume_on_busy_or_rejected', () => {
  const feedbackAction = { decision: 'request-change', commandId: 'cmd-fb-0' };
  assert.equal(shouldConsumeHumanAction(stage3GateBusy('turn-1'), {}, feedbackAction), false);
  assert.equal(
    shouldConsumeHumanAction(stage3TurnFailed('turn-1', 'provider_err', 'x'), {}, feedbackAction),
    false,
    'turn_failed 是回合终态，但消费只认 turn_open/durable replay',
  );
  assert.equal(shouldConsumeHumanAction(stage3TurnOpen('cmd-fb-0', 'turn-1', 2), {}, feedbackAction), true);
  assert.equal(
    shouldConsumeHumanAction(stage3TurnOpen('cmd-other', 'turn-1', 2), {}, feedbackAction),
    false,
  );
  assert.equal(
    shouldConsumeHumanAction({ type: 'session_state' }, { replayedCommandIds: ['cmd-fb-0'] }, feedbackAction),
    true,
    '同 command id 的 durable replay 消费 request-change',
  );

  const confirmAction = { decision: 'confirm', commandId: null };
  assert.equal(shouldConsumeHumanAction(stage3GateClosed('confirm'), {}, confirmAction), true);
  assert.equal(shouldConsumeHumanAction(stage3GateClosed('approve'), {}, confirmAction), true);
  assert.equal(shouldConsumeHumanAction(stage3GateClosed('terminate'), {}, confirmAction), false);
  assert.equal(shouldConsumeHumanAction({ type: 'session_state' }, { confirmedPlan: true }, confirmAction), true);
  assert.equal(shouldConsumeHumanAction({ type: 'session_state' }, { confirmedPlan: false }, confirmAction), false);

  const abandonAction = { decision: 'abandon', commandId: null };
  assert.equal(shouldConsumeHumanAction(stage3GateClosed('terminate'), {}, abandonAction), true);
  assert.equal(shouldConsumeHumanAction(stage3GateClosed('confirm'), {}, abandonAction), false);
});

test('campaign_stage3_logs_and_results_redact_feedback_content', () => {
  const logEntry = stage3OutboundLogEntry({
    type: 'human_gate_feedback',
    command_id: 'cmd-fb-0',
    feedback: '合成反馈 1 号',
  });
  assert.deepEqual(logEntry, {
    type: 'human_gate_feedback',
    command_id: 'cmd-fb-0',
    feedback_digest: sha256('合成反馈 1 号'),
    feedback_length: '合成反馈 1 号'.length,
  });
  const serialized = JSON.stringify(logEntry);
  assert.doesNotMatch(serialized, /合成反馈/u, 'ws.jsonl 出站条目只留 digest/长度，不留反馈全文');
  assert.equal(stage3FeedbackDigest('合成反馈 1 号'), sha256('合成反馈 1 号'));

  // 非 feedback 消息原样通过（advance/human_confirm 不含反馈文本）。
  const advanceMessage = { type: 'advance', command_id: 'cmd-adv-0' };
  assert.equal(stage3OutboundLogEntry(advanceMessage), advanceMessage);
  const confirmMessage = { type: 'human_confirm', decision: 'confirm', payload: null };
  assert.equal(stage3OutboundLogEntry(confirmMessage), confirmMessage);

  // result 模板为 interactive 预留 stage-3 审计字段，auto 策略保持惰性。
  const interactive = resultTemplate('codex', 1, '/tmp/out', {}, 'digest', 'interactive');
  assert.deepEqual(interactive.human_gate_turns, []);
  assert.deepEqual(interactive.advance_actions, []);
  assert.deepEqual(interactive.takeover_actions, []);
  assert.deepEqual(interactive.durable_recovery_checks, []);
  assert.equal(interactive.provider_start_ledger_before_after, null);
  const auto = resultTemplate('codex', 1, '/tmp/out', {}, 'digest', 'auto_if_valid');
  for (const reserved of ['humanGateActions', 'human_gate_turns', 'advance_actions', 'takeover_actions', 'durable_recovery_checks']) {
    assert.equal(Object.hasOwn(auto, reserved), false);
  }
});

test('campaign_stage3_multi_turn_flow_survives_reconnect_fault_transcript', () => {
  // 断线（turn_open 丢失）→ 重连 → durable replay 恢复 → 回合完成 → 下一动作。
  const { controller: first } = newController({
    actions: parseHumanScript('request-change:合成反馈 1 号;confirm'),
  });
  const send = first.onGateWaiting();
  for (const event of [stage3GateBusy('turn-unknown')]) {
    assert.deepEqual(first.onInbound(event).consumed, []);
  }

  const restarted = createStage3GateController({
    actions: parseHumanScript('request-change:合成反馈 1 号;confirm'),
    campaignRunId: STAGE3_SAMPLE_CAMPAIGN_RUN_ID,
    checkpoint: null,
    persistCheckpoint: () => {},
  });
  // 重连后按 checkpoint 语义复用同 id（此处从零 checkpoint 演示确定性重算）。
  const rederived = restarted.commandIdFor(0, 'human_gate_feedback');
  assert.equal(rederived, send.commandId, 'campaignCommandId 由稳定 run 身份确定性重算');

  const replay = restarted.onDurableState({ replayedCommandIds: [rederived] });
  assert.deepEqual(replay.consumed, [{ actionIndex: 0, kind: 'human_gate_feedback', via: 'durable_replay' }]);
  const afterReconnect = restarted.onInbound(stage3TurnCompleted('turn-1'));
  assert.equal(afterReconnect.outbound.message.decision, 'confirm', '回合终态后门回到 Waiting，才发下一脚本动作');
  restarted.onInbound(stage3GateClosed('confirm'));
  const fields = restarted.resultFields();
  assert.equal(fields.human_gate_turns[0].turn_id, null);
  assert.equal(fields.human_gate_turns[0].status, 'completed');
  assert.ok(fields.durable_recovery_checks.some((entry) => entry.check === 'durable_command_replay_consumed'));
});
