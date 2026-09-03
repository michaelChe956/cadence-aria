// Task 5.1 —— coding driver `ARIA_AMENDMENT_SCRIPT` 扩展(方案 A,8.4a 形态)测试。
//
// 纯 Node：不启动服务、不发真实网络请求、不要求 credential。
// 依据 cheat-sheet：cadence/reports/workitem-conversational-gate-advance/evidence/amendment-wire-notes.md
// 三条 WS 线：① 原 plan session `human_gate_feedback`；② child repair session
// `confirm_plan_amendment`；③ coding WS 只收控制事件，resume 判定一律 durable 回读。
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  amendmentChildConfirmPlan,
  amendmentChildReconnectPlan,
  amendmentConfirmMessage,
  amendmentDiscoveryComplete,
  amendmentDiscoveryFromMessage,
  amendmentFeedbackCommandId,
  amendmentGateFeedbackMessage,
  amendmentGateFeedbackPlan,
  amendmentGateTurnOutcome,
  amendmentManifestFromUpdatedEvent,
  amendmentNoteUpdatedEvent,
  amendmentResumeJudgment,
  amendmentScriptFromEnv,
  codingControlMessagePlan,
  workspaceSessionWsUrl,
} from './coding_run_campaign.mjs';

const CAMPAIGN_DIR = path.dirname(fileURLToPath(import.meta.url));

// —— ① 脚本解析（复用 parseHumanScript 语法）——

test('amendment 脚本未设置时返回 null（不启用 amendment 模式）', () => {
  assert.equal(amendmentScriptFromEnv({}), null);
  assert.equal(amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: undefined }), null);
  assert.equal(amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: null }), null);
});

test('amendment 脚本复用 stage3 语法：request-change 文本保留冒号/分号外文本，confirm 无描述', () => {
  const actions = amendmentScriptFromEnv({
    ARIA_AMENDMENT_SCRIPT: 'request-change:移除对 config/hello.json 的依赖,问候文案固定为 hello;confirm',
  });
  assert.deepEqual(actions, [
    { decision: 'request-change', description: '移除对 config/hello.json 的依赖,问候文案固定为 hello' },
    { decision: 'confirm', description: null },
  ]);
  // 多轮修订脚本同样合法：每条 request-change 对应一个 amendment turn。
  assert.deepEqual(
    amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: 'request-change:第一轮:保留冒号;request-change:第二轮;confirm' }),
    [
      { decision: 'request-change', description: '第一轮:保留冒号' },
      { decision: 'request-change', description: '第二轮' },
      { decision: 'confirm', description: null },
    ],
  );
});

test('amendment 脚本 fail-closed：空串、abandon/advance、缺 request-change、缺 confirm、confirm 先于 request-change、语法错误', () => {
  assert.throws(() => amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: '' }), /ARIA_AMENDMENT_SCRIPT 不能为空/u);
  assert.throws(() => amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: '   ' }), /ARIA_AMENDMENT_SCRIPT 不能为空/u);
  assert.throws(() => amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: 'abandon' }), /abandon 在 amendment 链无对应动作/u);
  assert.throws(
    () => amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: 'request-change:甲;advance' }),
    /advance 在 amendment 链无对应动作/u,
  );
  assert.throws(() => amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: 'confirm' }), /至少一条 request-change/u);
  assert.throws(() => amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: 'confirm;request-change:甲' }), /必须以 request-change 开始/u);
  assert.throws(() => amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: 'request-change:甲' }), /至少一条 confirm/u);
  // parseHumanScript 的语法错误原样透传（复用语法，不放宽）。
  assert.throws(() => amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: 'request-change:' }), /必须提供冒号后的反馈文本/u);
  assert.throws(() => amendmentScriptFromEnv({ ARIA_AMENDMENT_SCRIPT: 'retry' }), /仅支持/u);
});

// —— ② 三条线消息构造（字段逐字对齐 cheat-sheet）——

test('线① 原 plan session 的 typed feedback wire 逐字对齐 human_gate_feedback', () => {
  const message = amendmentGateFeedbackMessage({ commandId: 'cmd-amend-0', feedback: '移除对 config/hello.json 的依赖' });
  assert.deepEqual(message, {
    type: 'human_gate_feedback',
    command_id: 'cmd-amend-0',
    feedback: '移除对 config/hello.json 的依赖',
  });
  assert.throws(() => amendmentGateFeedbackMessage({ commandId: '', feedback: '甲' }), /command_id/u);
  assert.throws(() => amendmentGateFeedbackMessage({ commandId: 'c', feedback: '' }), /feedback/u);
});

test('线② child repair session 的出版确认 wire 逐字对齐 confirm_plan_amendment', () => {
  assert.deepEqual(amendmentConfirmMessage({ amendmentId: 'plan_amendment_0001' }), {
    type: 'confirm_plan_amendment',
    amendment_id: 'plan_amendment_0001',
  });
  assert.throws(() => amendmentConfirmMessage({ amendmentId: '' }), /amendment_id/u);
});

test('workspace session WS 端点按 /api/workspace-sessions/{id}/ws 拼接并编码 sessionId', () => {
  assert.equal(
    workspaceSessionWsUrl({ wsBase: 'ws://127.0.0.1:4317', sessionId: 'session_0001' }),
    'ws://127.0.0.1:4317/api/workspace-sessions/session_0001/ws',
  );
  assert.equal(
    workspaceSessionWsUrl({ wsBase: 'ws://host:1/', sessionId: 'a b/c' }),
    'ws://host:1/api/workspace-sessions/a%20b%2Fc/ws',
    'wsBase 去尾斜杠、sessionId 走 encodeURIComponent',
  );
  assert.throws(() => workspaceSessionWsUrl({ wsBase: 'ws://h', sessionId: '' }), /sessionId/u);
});

// —— ③ command_id 幂等与重连重发判定 ——

test('amendment command_id 由 (attemptId, actionIndex) 确定性生成：重连/重启复用同值，非空 ≤256 bytes', () => {
  const first = amendmentFeedbackCommandId({ attemptId: 'attempt_0001', actionIndex: 0 });
  assert.equal(first, amendmentFeedbackCommandId({ attemptId: 'attempt_0001', actionIndex: 0 }));
  assert.notEqual(first, amendmentFeedbackCommandId({ attemptId: 'attempt_0001', actionIndex: 1 }));
  assert.notEqual(first, amendmentFeedbackCommandId({ attemptId: 'attempt_0002', actionIndex: 0 }));
  assert.match(first, /^cmd-human_gate_feedback-/u);
  assert.ok(first.length > 0 && first.length <= 256, `command_id 必须非空且 ≤256 bytes: ${first}`);
  assert.throws(() => amendmentFeedbackCommandId({ attemptId: '', actionIndex: 0 }), /attemptId/u);
  assert.throws(() => amendmentFeedbackCommandId({ attemptId: 'a', actionIndex: -1 }), /actionIndex/u);
});

test('线① gate 发送计划：未发→send；已发未完结→重连 resend 同 command；turn 完结→wait', () => {
  const actions = [
    { decision: 'request-change', description: '第一轮' },
    { decision: 'request-change', description: '第二轮' },
    { decision: 'confirm', description: null },
  ];
  // 初始：游标 0 的 request-change 待发。
  assert.deepEqual(
    amendmentGateFeedbackPlan({ actions, cursor: 0, inFlight: null, turnStatus: null }),
    { kind: 'send', actionIndex: 0, feedback: '第一轮' },
  );
  // 已发送但未观察 turn_open（断线重连）：同 command 重发（服务端 durable 查重，不启 provider）。
  const inFlight = { actionIndex: 0, commandId: 'cmd-fixed-0' };
  assert.deepEqual(
    amendmentGateFeedbackPlan({ actions, cursor: 0, inFlight, turnStatus: null }),
    { kind: 'resend', actionIndex: 0, commandId: 'cmd-fixed-0', reason: 'unacked_reconnect' },
  );
  assert.deepEqual(
    amendmentGateFeedbackPlan({ actions, cursor: 0, inFlight, turnStatus: 'open' }),
    { kind: 'resend', actionIndex: 0, commandId: 'cmd-fixed-0', reason: 'turn_open_reconnect' },
  );
  // turn 完结后游标已推进：下一轮 request-change 正常 send（多轮修订脚本）；confirm 落位则等待线②。
  assert.deepEqual(
    amendmentGateFeedbackPlan({ actions, cursor: 1, inFlight: null, turnStatus: 'completed' }),
    { kind: 'send', actionIndex: 1, feedback: '第二轮' },
    'turn 完结后 inFlight 已清空、游标推进，下一轮 request-change 照常发送',
  );
  // 游标落在 confirm 上时 gate 线不发送。
  assert.deepEqual(
    amendmentGateFeedbackPlan({ actions, cursor: 2, inFlight: null, turnStatus: null }),
    { kind: 'wait' },
  );
});

test('线① turn 完结推进游标，turn 失败仅当下一条仍是 request-change 才续发，否则 fail-closed', () => {
  const twoRounds = [
    { decision: 'request-change', description: '第一轮' },
    { decision: 'request-change', description: '第二轮' },
    { decision: 'confirm', description: null },
  ];
  assert.deepEqual(
    amendmentGateTurnOutcome({ actions: twoRounds, cursor: 0, turnStatus: 'completed' }),
    { kind: 'advanced', cursor: 1 },
  );
  assert.deepEqual(
    amendmentGateTurnOutcome({ actions: twoRounds, cursor: 0, turnStatus: 'failed' }),
    { kind: 'advanced', cursor: 1 },
    '失败后仍有 request-change 轮次可续发',
  );
  const singleRound = [
    { decision: 'request-change', description: '唯一一轮' },
    { decision: 'confirm', description: null },
  ];
  assert.deepEqual(
    amendmentGateTurnOutcome({ actions: singleRound, cursor: 0, turnStatus: 'completed' }),
    { kind: 'advanced', cursor: 1 },
  );
  assert.deepEqual(
    amendmentGateTurnOutcome({ actions: singleRound, cursor: 0, turnStatus: 'failed' }),
    { kind: 'fail', failureClass: 'amendment_turn_failed' },
    'turn 失败且脚本无下一轮 request-change：不猜测恢复动作',
  );
});

// —— ④ 线② child 确认计划与重连重发 ——

test('线② 仅在 child stage=human_confirm 且游标落在 confirm 时发送；amendment id 缺失 fail-closed', () => {
  const actions = [
    { decision: 'request-change', description: '唯一一轮' },
    { decision: 'confirm', description: null },
  ];
  assert.deepEqual(
    amendmentChildConfirmPlan({
      actions, cursor: 1, childStage: 'human_confirm', amendmentId: 'plan_amendment_0001', codingEventAmendmentIds: new Set(),
    }),
    { kind: 'send', amendment_id: 'plan_amendment_0001', consume_cursor: 1 },
  );
  // 非 human_confirm（triaging/authoring 等更早阶段）继续等待。
  assert.deepEqual(
    amendmentChildConfirmPlan({
      actions, cursor: 1, childStage: 'authoring_revision', amendmentId: 'plan_amendment_0001', codingEventAmendmentIds: new Set(),
    }),
    { kind: 'wait', reason: 'child_stage_not_confirmable' },
  );
  // gate 轮次未完成（游标仍在 request-change）不得提前 confirm。
  assert.deepEqual(
    amendmentChildConfirmPlan({
      actions, cursor: 0, childStage: 'human_confirm', amendmentId: 'plan_amendment_0001', codingEventAmendmentIds: new Set(),
    }),
    { kind: 'wait', reason: 'gate_turn_pending' },
  );
  // coding 侧已收到该 amendment 的 plan_amendment_updated：不再发送（去重由 durable event 语义保证）。
  assert.deepEqual(
    amendmentChildConfirmPlan({
      actions, cursor: 1, childStage: 'human_confirm', amendmentId: 'plan_amendment_0001',
      codingEventAmendmentIds: new Set(['plan_amendment_0001']),
    }),
    { kind: 'wait', reason: 'coding_event_received' },
  );
  // amendment id 双源均缺：fail-closed，不得发送空 amendment_id。
  assert.deepEqual(
    amendmentChildConfirmPlan({
      actions, cursor: 1, childStage: 'human_confirm', amendmentId: null, codingEventAmendmentIds: new Set(),
    }),
    { kind: 'fail', failureClass: 'amendment_id_unresolved' },
  );
});

test('线② 断线重连：confirm 已发但 coding 侧未见事件 → resend 同 amendment_id 触发投递重试', () => {
  assert.deepEqual(
    amendmentChildReconnectPlan({
      childStage: 'human_confirm', amendmentId: 'plan_amendment_0001',
      codingEventAmendmentIds: new Set(), confirmSentFor: 'plan_amendment_0001',
    }),
    { kind: 'resend', amendment_id: 'plan_amendment_0001' },
  );
  assert.deepEqual(
    amendmentChildReconnectPlan({
      childStage: 'human_confirm', amendmentId: 'plan_amendment_0001',
      codingEventAmendmentIds: new Set(['plan_amendment_0001']), confirmSentFor: 'plan_amendment_0001',
    }),
    { kind: 'wait', reason: 'coding_event_received' },
  );
  assert.deepEqual(
    amendmentChildReconnectPlan({
      childStage: 'awaiting_confirmation_other', amendmentId: 'plan_amendment_0001',
      codingEventAmendmentIds: new Set(), confirmSentFor: 'plan_amendment_0001',
    }),
    { kind: 'wait', reason: 'child_stage_not_confirmable' },
  );
  assert.deepEqual(
    amendmentChildReconnectPlan({
      childStage: 'human_confirm', amendmentId: null, codingEventAmendmentIds: new Set(), confirmSentFor: null,
    }),
    { kind: 'wait', reason: 'nothing_in_flight' },
  );
});

// —— ⑤ 发现路径双源（plan_repair_required.session_link + coding_session_state.linked_plan_repair）——

test('发现双源：session_link 是完整对象（非 URL），linked_plan_repair 同样可发现', () => {
  const fromEvent = amendmentDiscoveryFromMessage({
    type: 'plan_repair_required',
    request: { id: 'repair_0001', amendment_id: null },
    session_link: {
      id: 'link_0001',
      relation: 'plan_repair',
      parent_session_id: 'session_plan_0001',
      child_session_id: 'session_child_0001',
      trigger: { attempt_id: 'attempt_0001', unit_run_id: 'run_0001', review_id: null, finding_id: 'finding_0001', repair_request_id: 'repair_0001', amendment_id: '', fingerprint: 'fp', base_plan_revision_id: 'rev_0001' },
      return_context: { original_attempt_id: 'attempt_0001', original_unit_run_id: 'run_0001', timeline_anchor_id: 'node_0001', original_route: '/workbench/projects/project_0001/issues/issue_0001/coding/attempt_0001' },
      created_at: '2026-09-03T00:00:00.000Z',
    },
  });
  assert.deepEqual(fromEvent, {
    source: 'plan_repair_required',
    parent_session_id: 'session_plan_0001',
    child_session_id: 'session_child_0001',
    amendment_id: null,
    repair_request_id: 'repair_0001',
  });
  assert.equal(amendmentDiscoveryComplete(fromEvent), true);

  // coding_session_state.linked_plan_repair（durable，重连恢复）是第二发现源；amendment.id 优先于 request.amendment_id。
  const fromState = amendmentDiscoveryFromMessage({
    type: 'coding_session_state',
    attempt_id: 'attempt_0001',
    status: 'awaiting_plan_amendment',
    stage: 'coding',
    units: [],
    linked_plan_repair: {
      request: { id: 'repair_0001', amendment_id: 'plan_amendment_0001' },
      link: { id: 'link_0001', parent_session_id: 'session_plan_0001', child_session_id: 'session_child_0001' },
      stage: 'awaiting_confirmation',
      amendment: { id: 'plan_amendment_0001', new_plan_revision_id: 'rev_0002' },
    },
  });
  assert.deepEqual(fromState, {
    source: 'coding_session_state',
    parent_session_id: 'session_plan_0001',
    child_session_id: 'session_child_0001',
    amendment_id: 'plan_amendment_0001',
    repair_request_id: 'repair_0001',
  });
  // 无 repair 信息的事件/消息返回 null；缺失 child 的发现不完整。
  assert.equal(amendmentDiscoveryFromMessage({ type: 'coding_session_state', linked_plan_repair: null }), null);
  assert.equal(amendmentDiscoveryFromMessage({ type: 'coding_stage_change' }), null);
  assert.equal(
    amendmentDiscoveryComplete({ source: 'plan_repair_required', parent_session_id: 'p', child_session_id: '', amendment_id: null, repair_request_id: null }),
    false,
  );
});

// —— ⑥ plan_amendment_updated 去重与 manifest 提取 ——

test('plan_amendment_updated manifest 提取：字段逐字对齐 wire（event_id durable、resume_target.mode）', () => {
  const manifest = amendmentManifestFromUpdatedEvent({
    type: 'plan_amendment_updated',
    event_id: 'coding_plan_amendment_updated_attempt_0001_plan_amendment_0001',
    amendment: {
      id: 'plan_amendment_0001',
      repair_request_id: 'repair_0001',
      previous_plan_revision_id: 'rev_0001',
      new_plan_revision_id: 'rev_0002',
      resume_target: { logical_work_item_id: 'wi_0001', mode: 'reexecute' },
    },
  });
  assert.deepEqual(manifest, {
    id: 'plan_amendment_0001',
    repair_request_id: 'repair_0001',
    previous_plan_revision_id: 'rev_0001',
    new_plan_revision_id: 'rev_0002',
    resume_target: { logical_work_item_id: 'wi_0001', mode: 'reexecute' },
  });
  assert.throws(() => amendmentManifestFromUpdatedEvent({ type: 'plan_amendment_updated' }), /event_id/u);
  assert.throws(
    () => amendmentManifestFromUpdatedEvent({ type: 'plan_amendment_updated', event_id: 'e1', amendment: { id: '' } }),
    /amendment\.id/u,
  );
});

test('plan_amendment_updated 按 event_id 幂等去重：重放不重复消费，不重发确认', () => {
  const seen = new Set();
  const event = {
    type: 'plan_amendment_updated',
    event_id: 'coding_plan_amendment_updated_attempt_0001_plan_amendment_0001',
    amendment: {
      id: 'plan_amendment_0001',
      repair_request_id: 'repair_0001',
      previous_plan_revision_id: 'rev_0001',
      new_plan_revision_id: 'rev_0002',
      resume_target: { logical_work_item_id: 'wi_0001', mode: 'reexecute' },
    },
  };
  const first = amendmentNoteUpdatedEvent(seen, event);
  assert.equal(first.duplicate, false);
  assert.equal(first.manifest.id, 'plan_amendment_0001');
  // 断线重连后允许重收相同 event_id：去重、不重发确认。
  const replay = amendmentNoteUpdatedEvent(seen, event);
  assert.equal(replay.duplicate, true);
  assert.equal(replay.manifest.id, 'plan_amendment_0001');
  // 不同 amendment（新 event_id）不去重。
  const second = amendmentNoteUpdatedEvent(seen, {
    ...event,
    event_id: 'coding_plan_amendment_updated_attempt_0001_plan_amendment_0002',
    amendment: { ...event.amendment, id: 'plan_amendment_0002' },
  });
  assert.equal(second.duplicate, false);
  assert.equal(seen.size, 2);
});

// —— ⑦ durable resume 判定（不得凭 plan_amendment_updated 判成功）——

function snapshotOf(overrides = {}) {
  return {
    attempt_id: 'attempt_0001',
    status: 'running',
    stage: 'coding',
    units: [{ unit_id: 'unit_0001', logical_work_item_id: 'wi_0001', status: 'running' }],
    linked_plan_repair: null,
    ...overrides,
  };
}

function manifestOf(overrides = {}) {
  return {
    id: 'plan_amendment_0001',
    repair_request_id: 'repair_0001',
    previous_plan_revision_id: 'rev_0001',
    new_plan_revision_id: 'rev_0002',
    resume_target: { logical_work_item_id: 'wi_0001', mode: 'reexecute' },
    ...overrides,
  };
}

test('manifest 未记录或快照未就绪 → pending，绝不凭单一事件判 resume', () => {
  assert.deepEqual(
    amendmentResumeJudgment({ expectedAttemptId: 'attempt_0001', manifest: null, snapshot: snapshotOf() }),
    { kind: 'pending', reason: 'amendment_manifest_missing' },
  );
  assert.deepEqual(
    amendmentResumeJudgment({ expectedAttemptId: 'attempt_0001', manifest: manifestOf(), snapshot: null }),
    { kind: 'pending', reason: 'snapshot_missing' },
  );
  // 收到事件但 attempt 仍处 amendment 暂停态：条件未满足，pending。
  assert.deepEqual(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf(),
      snapshot: snapshotOf({ status: 'awaiting_plan_amendment' }),
    }),
    { kind: 'pending', reason: 'conditions_not_met', mismatches: ['status:awaiting_plan_amendment!=running'] },
  );
  // resume_target 单元尚未出现在快照：pending（等待后续 durable 状态）。
  assert.deepEqual(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf(),
      snapshot: snapshotOf({ units: [] }),
    }),
    { kind: 'pending', reason: 'resume_target_unit_missing' },
  );
});

test('reexecute 模式 durable 判据：同 attempt + status running + stage coding + unit running', () => {
  const judgment = amendmentResumeJudgment({
    expectedAttemptId: 'attempt_0001',
    manifest: manifestOf(),
    snapshot: snapshotOf(),
  });
  assert.equal(judgment.kind, 'resumed');
  assert.deepEqual(judgment.evidence, {
    attempt_id: 'attempt_0001',
    status: 'running',
    stage: 'coding',
    unit_status: 'running',
    amendment_id: 'plan_amendment_0001',
    new_plan_revision_id: 'rev_0002',
    resume_mode: 'reexecute',
  });
  // stage 未回落到 coding：不判 resume。
  assert.equal(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf(),
      snapshot: snapshotOf({ stage: 'code_review' }),
    }).kind,
    'pending',
  );
});

test('revalidate / await_handoff 模式按 manifest.resume_target 判定', () => {
  assert.equal(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf({ resume_target: { logical_work_item_id: 'wi_0001', mode: 'revalidate' } }),
      snapshot: snapshotOf({ stage: 'code_review', units: [{ unit_id: 'unit_0001', logical_work_item_id: 'wi_0001', status: 'needs_revalidation' }] }),
    }).kind,
    'resumed',
  );
  assert.equal(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf({ resume_target: { logical_work_item_id: 'wi_0001', mode: 'revalidate' } }),
      snapshot: snapshotOf({ units: [{ unit_id: 'unit_0001', logical_work_item_id: 'wi_0001', status: 'needs_revalidation' }] }),
    }).kind,
    'pending',
    'revalidate 还要求 stage=code_review',
  );
  assert.equal(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf({ resume_target: { logical_work_item_id: 'wi_0001', mode: 'await_handoff' } }),
      snapshot: snapshotOf({ status: 'awaiting_plan_amendment', stage: 'coding', units: [{ unit_id: 'unit_0001', logical_work_item_id: 'wi_0001', status: 'awaiting_amendment' }] }),
    }).kind,
    'resumed',
    'await_handoff 只要求 resume target 单元 awaiting_amendment',
  );
});

test('未知 resume_target.mode 一律 fail-closed（不 pending、不猜测）', () => {
  assert.deepEqual(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf({ resume_target: { logical_work_item_id: 'wi_0001', mode: 'warp' } }),
      snapshot: snapshotOf(),
    }),
    { kind: 'fail', failureClass: 'amendment_resume_target_mode_unknown', mode: 'warp' },
  );
  assert.deepEqual(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf({ resume_target: null }),
      snapshot: snapshotOf(),
    }),
    { kind: 'fail', failureClass: 'amendment_resume_target_mode_unknown', mode: null },
    '缺失 resume_target 同样按未知模式 fail-closed',
  );
});

test('resume 身份判据 fail-closed：attempt 漂移与 binding 指向旧 revision', () => {
  assert.deepEqual(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf(),
      snapshot: snapshotOf({ attempt_id: 'attempt_9999' }),
    }),
    { kind: 'fail', failureClass: 'amendment_resume_attempt_changed', snapshot_attempt_id: 'attempt_9999' },
  );
  assert.deepEqual(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf(),
      snapshot: snapshotOf({
        linked_plan_repair: {
          amendment: { id: 'plan_amendment_0001', new_plan_revision_id: 'rev_0001' },
        },
      }),
    }),
    { kind: 'fail', failureClass: 'amendment_resume_binding_diverged', snapshot_new_plan_revision_id: 'rev_0001' },
    'linked_plan_repair 内联的 durable amendment 与事件 manifest 的 new_plan_revision_id 不一致 → binding 未更新',
  );
  // 内联 binding 一致时不阻塞判定。
  assert.equal(
    amendmentResumeJudgment({
      expectedAttemptId: 'attempt_0001',
      manifest: manifestOf(),
      snapshot: snapshotOf({
        linked_plan_repair: { amendment: { id: 'plan_amendment_0001', new_plan_revision_id: 'rev_0002' } },
      }),
    }).kind,
    'resumed',
  );
});

// —— ⑧ 回归：ARIA_AMENDMENT_SCRIPT 未设时零行为变化 ——

test('未设脚本时 coding 控制消息仍走 unhandled_coding_control_message fail-closed', () => {
  for (const type of ['plan_repair_required', 'plan_amendment_updated']) {
    assert.deepEqual(
      codingControlMessagePlan({ amendmentActions: null, messageType: type }),
      { kind: 'fail', failureClass: 'unhandled_coding_control_message' },
      `${type} 在未启用 amendment 模式时必须保持既有 fail-closed`,
    );
  }
  assert.deepEqual(
    codingControlMessagePlan({ amendmentActions: [{ decision: 'request-change', description: '甲' }, { decision: 'confirm', description: null }], messageType: 'plan_repair_required' }),
    { kind: 'amendment' },
  );
  // driver 源码保留原 fail 分支路径：case 块经 codingControlMessagePlan 分派，
  // unhandled_coding_control_message 字面量由纯函数返回（未启用时同一路径）。
  const source = fs.readFileSync(path.join(CAMPAIGN_DIR, 'coding_run_campaign.mjs'), 'utf8');
  assert.match(source, /case 'plan_repair_required':[\s\S]{0,600}codingControlMessagePlan/u);
  assert.match(
    source,
    /function codingControlMessagePlan\(\{ amendmentActions \}\) \{[\s\S]{0,200}unhandled_coding_control_message/u,
    '未启用 amendment 模式时纯函数必须返回 unhandled_coding_control_message',
  );
});

test('dry-run 回归：未设 ARIA_AMENDMENT_SCRIPT 时 amendment 关闭、不发任何网络请求；设置时报告动作数', () => {
  const driver = path.join(CAMPAIGN_DIR, 'coding_run_campaign.mjs');
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'aria-amendment-policy-'));
  const handoffPath = path.join(tmp, 'handoff.json');
  fs.writeFileSync(handoffPath, JSON.stringify({
    project_id: 'project_0001',
    issue_id: 'issue_0001',
    plan_id: 'plan_0001',
    repository_id: 'repository_0001',
    provider: 'codex',
    work_item_ids: ['work_item_0001'],
  }), 'utf8');

  const unset = spawnSync(
    process.execPath,
    [driver, handoffPath, path.join(tmp, 'out'), '--dry-run'],
    { encoding: 'utf8', env: { ...process.env, ARIA_AMENDMENT_SCRIPT: '' } },
  );
  // 通过 delete 模拟未设置（空字符串视作未设置之外的显式空值：这里用真 unset）。
  const envUnset = { ...process.env };
  delete envUnset.ARIA_AMENDMENT_SCRIPT;
  const unsetRun = spawnSync(
    process.execPath,
    [driver, handoffPath, path.join(tmp, 'out'), '--dry-run'],
    { encoding: 'utf8', env: envUnset },
  );
  assert.equal(unsetRun.status, 0, unsetRun.stderr);
  const unsetOutput = JSON.parse(unsetRun.stdout);
  assert.deepEqual(unsetOutput.amendment_script, { enabled: false, actions: null });
  assert.equal(unsetOutput.no_http_or_websocket_requests, true);
  assert.equal(unset.status, 2, '显式空字符串启动校验失败关闭');
  assert.match(unset.stderr, /ARIA_AMENDMENT_SCRIPT 不能为空/u);

  const enabled = spawnSync(
    process.execPath,
    [driver, handoffPath, path.join(tmp, 'out'), '--dry-run'],
    {
      encoding: 'utf8',
      env: {
        ...envUnset,
        ARIA_AMENDMENT_SCRIPT: 'request-change:移除对 config/hello.json 的依赖,问候文案固定为 hello;confirm',
      },
    },
  );
  assert.equal(enabled.status, 0, enabled.stderr);
  const enabledOutput = JSON.parse(enabled.stdout);
  assert.deepEqual(enabledOutput.amendment_script, { enabled: true, actions: 2 });
  assert.equal(enabledOutput.no_http_or_websocket_requests, true, 'dry-run 不因 amendment 脚本发任何请求');

  const invalid = spawnSync(
    process.execPath,
    [driver, handoffPath, path.join(tmp, 'out'), '--dry-run'],
    { encoding: 'utf8', env: { ...envUnset, ARIA_AMENDMENT_SCRIPT: 'abandon' } },
  );
  assert.notEqual(invalid.status, 0);
  assert.match(invalid.stderr, /ARIA_AMENDMENT_SCRIPT/u);
});
