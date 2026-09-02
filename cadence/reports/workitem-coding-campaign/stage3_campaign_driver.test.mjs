// 阶段 3 Task 8.1 —— campaign driver 的 typed 人工门与 advance 模拟测试。
//
// 纯 Node：不启动服务、不发真实网络请求、不要求 credential；
// 通过 fixture transcript 驱动 driver 的 stage-3 gate controller。
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import {
  advanceSimulationAction,
  campaignCommandId,
  createStage3GateController,
  parseHumanScript,
  resultTemplate,
  shouldConsumeHumanAction,
  singleCandidateOutboundAllowed,
  stage3AdvanceFinishPlan,
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
  stage3AdvanceRejected,
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
  // REQ-CG-02：SC HumanConfirm stage 的服务端准入表只收 HumanGateFeedback|Confirm|HumanConfirm{Terminate}；
  // confirm 必须编码为裸 typed 消息（{"type":"confirm"}），不得再发 human_confirm{decision:"confirm"}。
  assert.deepEqual(stage3HumanMessage({ decision: 'confirm', description: null }, { commandId: null }), {
    type: 'confirm',
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
  assert.equal(singleCandidateOutboundAllowed({ type: 'confirm' }, 'interactive'), true,
    '裸 confirm wire（REQ-CG-02 SC 准入表）只对 interactive 放行');
  assert.equal(singleCandidateOutboundAllowed({ type: 'confirm' }, 'auto_if_valid'), false);
  // hello/pong 是连接层心跳：初连与 8.2c 重连（此时 flow_kind 已知）都必须放行。
  assert.equal(singleCandidateOutboundAllowed({ type: 'hello' }, 'interactive'), true);
  assert.equal(singleCandidateOutboundAllowed({ type: 'hello' }, 'auto_if_valid'), true);
  assert.equal(singleCandidateOutboundAllowed({ type: 'pong' }, 'auto_if_valid'), true);
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
  assert.equal(confirmSend.message.type, 'confirm');
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
  assert.equal(afterReconnect.outbound.message.type, 'confirm', '回合终态后门回到 Waiting，才发下一脚本动作');
  restarted.onInbound(stage3GateClosed('confirm'));
  const fields = restarted.resultFields();
  assert.equal(fields.human_gate_turns[0].turn_id, null);
  assert.equal(fields.human_gate_turns[0].status, 'completed');
  assert.ok(fields.durable_recovery_checks.some((entry) => entry.check === 'durable_command_replay_consumed'));
});

test('campaign_stage3_advance_rejected_records_audit_and_finishes_without_consuming', () => {
  // ws 在线时的完整路径：confirm 消费 → Confirmed 回读 → 发 advance → 服务端拒绝（前置不满足零副作用拒绝）。
  const { controller } = newController({ actions: parseHumanScript('confirm;advance') });
  const confirmSend = controller.onGateWaiting();
  assert.equal(confirmSend.message.type, 'confirm');
  controller.onInbound(stage3GateClosed('confirm'));
  const advance = controller.onConfirmedDurable({ providerStartLedger: stage3ProviderStartLedgerFixture() });
  assert.equal(advance.decision, 'send');
  assert.equal(advance.outbound.type, 'advance');

  // rejected：记录 advance_actions[].status='rejected' 审计，但不消费脚本动作（只有 completed/replay 才消费）。
  const rejected = controller.onInbound(stage3AdvanceRejected(advance.commandId));
  assert.deepEqual(rejected.consumed, [], 'advance_rejected 不消费脚本动作');
  assert.equal(controller.currentAction().decision, 'advance', '脚本指针仍停在 advance，未被 rejected 推进');
  const fields = controller.resultFields();
  assert.equal(fields.advance_actions.length, 1);
  assert.equal(fields.advance_actions[0].command_id, advance.commandId);
  assert.equal(fields.advance_actions[0].status, 'rejected');
  assert.equal(fields.advance_actions[0].code, 'ADVANCE_PRECONDITION_FAILED');
  // 镜像 driver 收尾分支的 noteProviderLedgerAfter 调用后再比对前后快照。
  controller.noteProviderLedgerAfter(stage3ProviderStartLedgerFixture());
  const fieldsAfterNote = controller.resultFields();
  assert.deepEqual(
    fieldsAfterNote.provider_start_ledger_before_after.before,
    fieldsAfterNote.provider_start_ledger_before_after.after,
    'rejected 零副作用：provider-start ledger 前后不变',
  );

  // 收尾判定：rejected 无条件清 advanceFinishPending 并按既定策略收尾（不悬挂至 hard timeout）；
  // completed 仍要求确认消费后才收尾。
  assert.deepEqual(
    stage3AdvanceFinishPlan({
      finishPending: true,
      message: stage3AdvanceRejected(advance.commandId),
      consumedKinds: [],
    }),
    {
      event: 'stage3_advance_rejected',
      command_id: advance.commandId,
      code: 'ADVANCE_PRECONDITION_FAILED',
    },
    'ws 在线时 advance 被拒必须立即收尾（rejected 不消费脚本动作也不得悬挂）',
  );
  assert.equal(
    stage3AdvanceFinishPlan({
      finishPending: true,
      message: stage3AdvanceCompleted(advance.commandId),
      consumedKinds: [],
    }),
    null,
    'completed 未确认消费（幂等命中/replay 未定）时不收尾',
  );
  assert.deepEqual(
    stage3AdvanceFinishPlan({
      finishPending: true,
      message: stage3AdvanceCompleted(advance.commandId),
      consumedKinds: ['advance'],
    }),
    {
      event: 'stage3_advance_simulation',
      decision: 'advance_completed',
      command_id: advance.commandId,
      attempt_id: 'attempt_fixture_0001',
      code: null,
    },
  );
  assert.equal(
    stage3AdvanceFinishPlan({
      finishPending: false,
      message: stage3AdvanceRejected(advance.commandId),
      consumedKinds: [],
    }),
    null,
    '非 advance 等待期收到 rejected 不触发收尾',
  );
});

test('campaign_stage3_takeover_transcript_switches_child_ws_and_keeps_parent_evidence', async () => {
  const { stage3TakeoverResponse, stage3TakeoverChildTranscript } = await import('./stage3_campaign_fixtures.mjs');
  const { stage3TakeoverDecision } = await import('./workitem_run_campaign.mjs');

  // takeover 门控：仅 SC+interactive 且未 takeover 过的 stopped_needs_human 放行。
  assert.equal(
    stage3TakeoverDecision({ failureClass: 'stopped_needs_human', flowKind: 'single_candidate', runPolicy: 'interactive', alreadyTakenOver: false }),
    'takeover',
  );
  assert.equal(
    stage3TakeoverDecision({ failureClass: 'stopped_needs_human', flowKind: 'single_candidate', runPolicy: 'auto_if_valid', alreadyTakenOver: false }),
    'finish',
    'auto 策略不走 takeover',
  );
  assert.equal(
    stage3TakeoverDecision({ failureClass: 'stopped_needs_human', flowKind: 'legacy', runPolicy: 'interactive', alreadyTakenOver: false }),
    'finish',
    'legacy flow 不走 takeover',
  );
  assert.equal(
    stage3TakeoverDecision({ failureClass: 'stopped_needs_human', flowKind: 'single_candidate', runPolicy: 'interactive', alreadyTakenOver: true }),
    'finish',
    '重复 takeover 必须幂等收尾',
  );
  assert.equal(
    stage3TakeoverDecision({ failureClass: 'confirmed', flowKind: 'single_candidate', runPolicy: 'interactive', alreadyTakenOver: false }),
    'finish',
    '非 stopped_needs_human 终态不触发 takeover',
  );

  // transcript：parent 侧一轮 turn 审计先落账 → takeover → child ws 事件继续消费。
  const { controller } = newController({
    actions: parseHumanScript('request-change:合成反馈 1 号;request-change:合成反馈 2 号;confirm'),
  });
  const parentSend = controller.onGateWaiting();
  controller.onInbound(stage3TurnOpen(parentSend.commandId, 'parent-turn-1', 2));
  // 回合终态后门回到 Waiting，driver 自动发送第二个 request-change。
  const parentComplete = controller.onInbound(stage3TurnCompleted('parent-turn-1'));
  assert.equal(parentComplete.outbound.message.type, 'human_gate_feedback');
  const secondCommandId = parentComplete.outbound.commandId;
  const parentLedger = stage3ProviderStartLedgerFixture();
  controller.noteProviderLedgerAfter(parentLedger);

  // durable terminal(stopped_needs_human) 在第二个动作受理前到达 → takeover。
  const takeover = stage3TakeoverResponse('ws_fixture_parent_0001');
  controller.noteTakeoverAction({
    parent_session_id: takeover.parent_session_id,
    child_session_id: takeover.workspace_session.workspace_session_id,
    takeover_event_id: takeover.takeover_event_id,
    trigger: 'durable_session_terminal',
    at: '2026-08-31T00:00:00Z',
  });

  // child ws 重连：门 Waiting，重发复用 checkpoint 里的同 command id。
  const childSend = controller.onGateWaiting({ reconnect: true });
  assert.equal(childSend.message.type, 'human_gate_feedback');
  assert.equal(childSend.commandId, secondCommandId, 'child 侧重连重发必须复用 checkpoint command id');
  assert.notEqual(childSend.commandId, parentSend.commandId);
  for (const event of stage3TakeoverChildTranscript(childSend.commandId)) {
    controller.onInbound(event);
  }

  const fields = controller.resultFields();
  // parent 证据保留：parent-era turn 审计仍在，且 takeover 审计就位。
  assert.equal(fields.human_gate_turns.length, 2, 'parent 与 child 的 turn 审计并存');
  assert.equal(fields.human_gate_turns[0].turn_id, 'parent-turn-1');
  assert.equal(fields.human_gate_turns[1].turn_id, 'child-turn-1');
  assert.equal(fields.takeover_actions.length, 1);
  assert.equal(fields.takeover_actions[0].parent_session_id, 'ws_fixture_parent_0001');
  assert.equal(fields.takeover_actions[0].child_session_id, 'ws_fixture_child_0001');
  assert.equal(fields.takeover_actions[0].takeover_event_id, 'takeover_event_fixture_0001');
  // takeover 切换不触碰 provider-start ledger（after 镜像快照，before 未经 advance 置位）。
  assert.equal(fields.provider_start_ledger_before_after.before, null);
  assert.deepEqual(fields.provider_start_ledger_before_after.after, parentLedger);
  assert.doesNotMatch(JSON.stringify(fields), /合成反馈/u, '审计只留 digest/长度，不含反馈全文');
});

test('campaign_stage3_ws_close_wires_durable_replay_to_on_durable_state', async () => {
  const { stage3DurableReplayState } = await import('./workitem_run_campaign.mjs');

  // 用真实 .aria 布局铺 durable 恢复面：session JSON + 终态 turn + advance 记录。
  const ariaRoot = fs.mkdtempSync(path.join(os.tmpdir(), 'aria-stage3-durable-'));
  const sessionDir = path.join(ariaRoot, 'projects', 'project_0001', 'issues', 'issue_0001', 'workspace-sessions');
  fs.mkdirSync(path.join(sessionDir, 'ws_fixture_child_0001', 'human-gate-turns'), { recursive: true });
  fs.mkdirSync(path.join(ariaRoot, 'projects', 'project_0001', 'issues', 'issue_0001', 'advance-records'), { recursive: true });
  fs.writeFileSync(path.join(sessionDir, 'ws_fixture_child_0001.json'), JSON.stringify({
    id: 'ws_fixture_child_0001',
    status: 'waiting_for_human',
    single_candidate_phase: 'approval',
  }));
  const { controller } = newController({
    actions: parseHumanScript('request-change:合成反馈 1 号;confirm;advance'),
  });
  const first = controller.onGateWaiting();
  fs.writeFileSync(path.join(sessionDir, 'ws_fixture_child_0001', 'human-gate-turns', 'turn-1.json'), JSON.stringify({
    turn_id: 'durable-turn-1',
    command_id: first.commandId,
    status: 'completed',
    result_artifact_ref: 'artifact_version_002',
  }));
  const advanceCommandId = controller.commandIdFor(2, 'advance');
  fs.writeFileSync(path.join(ariaRoot, 'projects', 'project_0001', 'issues', 'issue_0001', 'advance-records', 'advance_0001.json'), JSON.stringify({
    id: 'advance_0001',
    command_id: advanceCommandId,
    attempt_id: 'attempt_fixture_0001',
    status: 'ready',
  }));

  // durable 读取：坏 JSON/缺目录按无记录处理，不抛出。
  const state = stage3DurableReplayState({
    ariaRoot,
    projectId: 'project_0001',
    issueId: 'issue_0001',
    sessionId: 'ws_fixture_child_0001',
  });
  assert.equal(state.confirmedPlan, false);
  assert.deepEqual(state.replayedCommandIds, [first.commandId]);
  assert.deepEqual(state.replayedTurns, [{
    command_id: first.commandId,
    turn_id: 'durable-turn-1',
    status: 'completed',
    artifact_ref: 'artifact_version_002',
    failure_class: null,
  }]);
  assert.deepEqual(state.advanceRecords, [{ command_id: advanceCommandId, attempt_id: 'attempt_fixture_0001' }]);
  const missingDirs = stage3DurableReplayState({
    ariaRoot,
    projectId: 'project_missing',
    issueId: 'issue_missing',
    sessionId: 'ws_none',
  });
  assert.deepEqual(missingDirs, { confirmedPlan: false, replayedCommandIds: [], replayedTurns: [], advanceRecords: [] });

  // 8.2c 销项（8.1 Observation-1）：ws 关闭后驱动把 durable 记录作为真实
  // replay 消息喂给 onDurableState —— 终态 turn 收敛回合并放行下一动作。
  const replay = controller.onDurableState(state);
  assert.deepEqual(replay.consumed, [{ actionIndex: 0, kind: 'human_gate_feedback', via: 'durable_replay' }]);
  const turn = controller.resultFields().human_gate_turns[0];
  assert.equal(turn.status, 'completed', 'durable 终态 turn 直接收敛回合状态');
  assert.equal(turn.artifact_ref, 'artifact_version_002');
  const next = controller.onGateWaiting();
  assert.equal(next.message.type, 'confirm', '终态回放释放单飞后门回到 Waiting');
  // confirm 由 confirmedPlan 的 durable 回读消费。
  const confirmReplay = controller.onDurableState({ confirmedPlan: true });
  assert.deepEqual(confirmReplay.consumed, [{ actionIndex: 1, kind: 'confirm', via: 'durable_confirmed_readback' }]);
  // advance 由 durable advance 记录回放消费（幂等命中）。
  const advanceReplay = controller.onDurableState(state);
  assert.deepEqual(advanceReplay.consumed, [{ actionIndex: 2, kind: 'advance', via: 'durable_replay' }]);
  const fields = controller.resultFields();
  assert.ok(fields.durable_recovery_checks.some((entry) => entry.check === 'durable_command_replay_consumed' && entry.turn_status === 'completed'));
  assert.ok(fields.durable_recovery_checks.some((entry) => entry.check === 'durable_advance_record_replayed'));
  assert.equal(fields.advance_actions[0].status, 'completed');
  assert.equal(fields.advance_actions[0].attempt_id, 'attempt_fixture_0001');
  fs.rmSync(ariaRoot, { recursive: true, force: true });
});

test('campaign_stage3_confirm_conflict_waits_for_gate_reopen', () => {
  const { controller } = newController({ actions: parseHumanScript('request-change:合成反馈 1 号;confirm') });
  const send = controller.onGateWaiting();
  controller.onInbound(stage3TurnOpen(send.commandId, 'turn-1', 1));
  const afterTurn = controller.onInbound(stage3TurnCompleted('turn-1'));
  assert.equal(afterTurn.outbound.message.type, 'confirm', '回合终态后立即给出下一动作（既有契约不变）');

  // 真实链路：服务端复评期间门未回 approval → confirm 撞 CAS conflict。
  const swallowed = controller.noteGateCloseConflict('product_store_conflict: human_gate_close ws_0001');
  assert.equal(swallowed, true);
  // 非本 conflict 形态的错误不消化；重复通报不再消化。
  assert.equal(controller.noteGateCloseConflict('other error'), false);
  assert.equal(controller.noteGateCloseConflict('product_store_conflict: human_gate_close ws_0001'), false, '重复通报不再消化');

  // 门重新 Waiting（复评完成 stage_change human_confirm）→ 下一次 onGateWaiting
  // 重发同一 confirm；驱动只在门 Waiting 信号时调用，不会自发重发。
  const resend = controller.onGateWaiting();
  assert.equal(resend.message.type, 'confirm');
  assert.equal(resend.actionIndex, 1, '脚本指针仍停在 confirm');
  const closed = controller.onInbound(stage3GateClosed('confirm'));
  assert.deepEqual(closed.consumed, [{ actionIndex: 1, kind: 'confirm', via: 'human_gate_closed' }]);
  assert.ok(controller.resultFields().durable_recovery_checks.some((entry) => entry.check === 'confirm_conflict_awaiting_gate'));
});

// —— 阶段 3 Task 8.3 —— advance 完成后的 group snapshot readback ——
test('campaign_stage3_group_snapshot_readback_after_advance_completed', async () => {
  const {
    stage3GroupAttemptSnapshotFixture,
    stage3AdvanceCompleted,
  } = await import('./stage3_campaign_fixtures.mjs');
  const {
    stage3GroupSnapshotUrl,
    stage3GroupSnapshotEvidence,
  } = await import('./workitem_run_campaign.mjs');

  // workspace entry → HTTP GET URL:advance_completed 的 attempt_id 可直接读 group snapshot。
  const url = stage3GroupSnapshotUrl({
    base: 'http://127.0.0.1:4317',
    projectId: 'project_0001',
    issueId: 'issue_0001',
    attemptId: 'attempt_fixture_0001',
  });
  assert.equal(url, 'http://127.0.0.1:4317/api/projects/project_0001/issues/issue_0001/coding-attempts/attempt_fixture_0001');
  assert.throws(() => stage3GroupSnapshotUrl({ base: 'http://x', projectId: '', issueId: 'i', attemptId: 'a' }), /projectId/u);
  assert.throws(() => stage3GroupSnapshotUrl({ base: 'http://x', projectId: 'p', issueId: '', attemptId: 'a' }), /issueId/u);
  assert.throws(() => stage3GroupSnapshotUrl({ base: 'http://x', projectId: 'p', issueId: 'i', attemptId: '' }), /attemptId/u);

  // 脱敏 evidence:只含 attempt_id、unit logical IDs/status、binding revision digest、
  // lock ID、advance command/record、provider-start counts;不含候选文本/反馈全文。
  const snapshot = stage3GroupAttemptSnapshotFixture();
  const completed = stage3AdvanceCompleted('cmd-adv-0', 'attempt_fixture_0001');
  const evidence = stage3GroupSnapshotEvidence({
    snapshot,
    advanceMessage: completed,
    providerStartLedgerBefore: stage3ProviderStartLedgerFixture(),
    providerStartLedgerAfter: stage3ProviderStartLedgerFixture(),
  });
  assert.equal(evidence.attempt_id, 'attempt_fixture_0001');
  assert.deepEqual(evidence.units.map((unit) => [unit.logical_work_item_id, unit.status]), [
    ['wi_a', 'running'],
    ['wi_b', 'pending'],
  ]);
  assert.match(evidence.binding_revision_digest, /^[0-9a-f]{64}$/u, 'binding revision 只留 digest');
  assert.equal(evidence.lock_id, 'attempt_fixture_0001');
  assert.deepEqual(evidence.advance_record, {
    command_id: 'cmd-adv-0',
    record_id: 'advance_fixture_0001',
    status: 'ready',
    attempt_id: 'attempt_fixture_0001',
  });
  assert.deepEqual(evidence.provider_start_counts, { before: 2, after: 2 },
    'advance 不启动 provider:前后计数相等');
  const serialized = JSON.stringify(evidence);
  assert.doesNotMatch(serialized, /合成/u, 'evidence 不含反馈全文');
  assert.doesNotMatch(serialized, /markdown/u, 'evidence 不含候选文本载荷');
  assert.doesNotMatch(serialized, /plan_revision_fixture_0001/u, 'binding revision 原文不落 evidence');
  assert.equal(evidence.source, 'group_attempt_snapshot_readback');
});

test('campaign_stage3_group_snapshot_evidence_fails_closed_on_mismatched_attempt', async () => {
  const { stage3GroupAttemptSnapshotFixture } = await import('./stage3_campaign_fixtures.mjs');
  const { stage3GroupSnapshotEvidence } = await import('./workitem_run_campaign.mjs');
  const snapshot = stage3GroupAttemptSnapshotFixture();
  // advance 指向另一 attempt:readback 必须拒绝生成 evidence(而非静默错配)。
  const mismatched = stage3GroupSnapshotEvidence({
    snapshot,
    advanceMessage: { type: 'advance_completed', command_id: 'cmd-adv-x', attempt_id: 'attempt_other', workspace_entry: '/x' },
    providerStartLedgerBefore: [],
    providerStartLedgerAfter: [],
  });
  assert.equal(mismatched, null, 'attempt 不匹配时 readback fail-closed');
  // provider-start 计数不等:advance 期间出现 provider 启动,必须以 violation 标出。
  const violated = stage3GroupSnapshotEvidence({
    snapshot,
    advanceMessage: stage3AdvanceCompletedForEvidence(),
    providerStartLedgerBefore: [{ provider_start_idempotency_key: 'k1', started: true }],
    providerStartLedgerAfter: [
      { provider_start_idempotency_key: 'k1', started: true },
      { provider_start_idempotency_key: 'k2', started: true },
    ],
  });
  assert.equal(violated.provider_start_counts.before, 1);
  assert.equal(violated.provider_start_counts.after, 2);
  assert.equal(violated.provider_start_violation, true, 'ledger 计数变化必须显式标注违规');
});

function stage3AdvanceCompletedForEvidence() {
  return { type: 'advance_completed', command_id: 'cmd-adv-0', attempt_id: 'attempt_fixture_0001', workspace_entry: '/w' };
}
