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

import * as codingDriverModule from './coding_run_campaign.mjs';
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

// —— ⑨ liveness 修复轮(Finding 1/2)：跨 socket 竞态与断线窗口内 turn 已完结 ——
// 新符号经命名空间解构获取：修复落地前为 undefined，红测只落在新增场景用例上，
// 既有 21 用例不受命名导入错误牵连。
const {
  amendmentDurableGateTurns,
  amendmentReplayedTurnReconciliation,
  createAmendmentRuntime,
} = codingDriverModule;

// 假 workspace WS：只记录出站消息，由测试按序触发 onopen/onmessage。
function fakeWorkspaceSocketFactory(bucket) {
  return class FakeWorkspaceSocket {
    constructor(url) {
      this.url = url;
      this.sent = [];
      this.onopen = null;
      this.onmessage = null;
      this.onclose = null;
      bucket.push(this);
    }

    send(raw) {
      this.sent.push(JSON.parse(raw));
    }

    close() {}

    fireOpen() {
      this.onopen?.();
    }

    fireMessage(message) {
      this.onmessage?.({ data: JSON.stringify(message) });
    }
  };
}

function amendmentRuntimeHarness({ actions, ariaRoot, attemptId = 'attempt_0001' }) {
  const sockets = [];
  const hooks = {
    failures: [],
    logs: [],
    fail(failureClass, error) {
      hooks.failures.push({ failureClass, error: String(error?.message ?? error) });
    },
    log(entry) {
      hooks.logs.push(entry);
    },
    noteUsage() {},
    elapsedSec: () => 0,
  };
  const runtime = createAmendmentRuntime({
    attemptId,
    actions,
    projectId: 'project_0001',
    issueId: 'issue_0001',
    hooks,
    deps: {
      WebSocketCtor: fakeWorkspaceSocketFactory(sockets),
      wsBase: 'ws://driver-test',
      ariaRoot,
    },
  });
  const socketOf = (sessionId) => sockets.find(
    (socket) => socket.url === `ws://driver-test/api/workspace-sessions/${sessionId}/ws`,
  );
  return { runtime, hooks, sockets, socketOf };
}

function planRepairRequiredWire({ parentSessionId = 'session_plan_0001', childSessionId = 'session_child_0001' } = {}) {
  return {
    type: 'plan_repair_required',
    request: { id: 'repair_0001', amendment_id: null },
    session_link: {
      id: 'link_0001',
      relation: 'plan_repair',
      parent_session_id: parentSessionId,
      child_session_id: childSessionId,
      trigger: {
        attempt_id: 'attempt_0001',
        unit_run_id: 'run_0001',
        review_id: null,
        finding_id: 'finding_0001',
        repair_request_id: 'repair_0001',
        amendment_id: '',
        fingerprint: 'fp',
        base_plan_revision_id: 'rev_0001',
      },
      return_context: {
        original_attempt_id: 'attempt_0001',
        original_unit_run_id: 'run_0001',
        timeline_anchor_id: 'node_0001',
        original_route: '/workbench/projects/project_0001/issues/issue_0001/coding/attempt_0001',
      },
      created_at: '2026-09-03T00:00:00.000Z',
    },
  };
}

function childHumanConfirmState() {
  return {
    type: 'session_state',
    stage: 'human_confirm',
    plan_repair: {
      request: { id: 'repair_0001', amendment_id: 'plan_amendment_0001' },
      amendment: { id: 'plan_amendment_0001' },
    },
  };
}

const SINGLE_ROUND_ACTIONS = [
  { decision: 'request-change', description: '移除对 config/hello.json 的依赖,问候文案固定为 hello' },
  { decision: 'confirm', description: null },
];

function writeDurableTurn({ ariaRoot, turn }) {
  const directory = path.join(
    ariaRoot, 'projects', 'project_0001', 'issues', 'issue_0001',
    'workspace-sessions', turn.session_id, 'human-gate-turns',
  );
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(path.join(directory, `${turn.turn_id}.json`), JSON.stringify(turn), 'utf8');
}

function durableTurnOf({ attemptId, actionIndex, turnId, status, failureClass = null }) {
  return {
    turn_id: turnId,
    session_id: 'session_plan_0001',
    command_id: amendmentFeedbackCommandId({ attemptId, actionIndex }),
    feedback_text: '移除对 config/hello.json 的依赖,问候文案固定为 hello',
    status,
    attempt_no: 1,
    budget_reserved: 1,
    source_hash: 'a'.repeat(64),
    result_artifact_ref: status === 'completed' ? 'artifact:rev_0002' : null,
    failure_class: failureClass,
    created_at: '2026-09-03T00:00:00.000Z',
    updated_at: '2026-09-03T00:01:00.000Z',
  };
}

test('Finding 1：child human_confirm 快照先于父线 turn 完结到达时，turn 完结后必须补发 confirm（不得停摆到硬超时）', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'aria-amendment-liveness1-'));
  const { runtime, hooks, socketOf } = amendmentRuntimeHarness({
    actions: SINGLE_ROUND_ACTIONS,
    ariaRoot: path.join(tmp, 'aria'),
  });
  runtime.onCodingControl(planRepairRequiredWire());
  const parent = socketOf('session_plan_0001');
  const child = socketOf('session_child_0001');
  const commandId = amendmentFeedbackCommandId({ attemptId: 'attempt_0001', actionIndex: 0 });

  parent.fireOpen();
  assert.ok(
    parent.sent.some((message) => message.type === 'human_gate_feedback' && message.command_id === commandId),
    '首发即发出游标 0 的 typed feedback（确定性 command_id）',
  );
  child.fireOpen();

  // 竞态：child 的 human_confirm 快照先到——此时游标仍在 request-change，只能 wait（gate_turn_pending）。
  child.fireMessage(childHumanConfirmState());
  assert.equal(
    child.sent.some((message) => message.type === 'confirm_plan_amendment'),
    false,
    '游标未落位前不得提前 confirm',
  );

  // 随后父线 turn 完结：游标推进到 confirm——之后没有任何新的 child session_state 再到达。
  parent.fireMessage({ type: 'human_gate_turn_open', command_id: commandId, turn_id: 'turn_0001', remaining_budget: 1 });
  parent.fireMessage({ type: 'human_gate_turn_completed', turn_id: 'turn_0001', artifact_ref: 'artifact:rev_0002' });

  assert.deepEqual(hooks.failures, []);
  assert.ok(
    child.sent.some((message) => message.type === 'confirm_plan_amendment' && message.amendment_id === 'plan_amendment_0001'),
    'turn 完结推进游标后必须用缓存的 childStage/childAmendmentId 重求值 child 确认计划并补发 confirm',
  );
  runtime.close();
});

test('Finding 2：replayed turn_open 撞上断线窗口内已 terminal 的 durable turn → 回读对账直接推进，不等不会再来的 turn_completed', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'aria-amendment-liveness2-'));
  const ariaRoot = path.join(tmp, 'aria');
  const commandId = amendmentFeedbackCommandId({ attemptId: 'attempt_0001', actionIndex: 0 });
  writeDurableTurn({ ariaRoot, turn: durableTurnOf({ attemptId: 'attempt_0001', actionIndex: 0, turnId: 'turn_0001', status: 'completed' }) });

  const { runtime, hooks, socketOf } = amendmentRuntimeHarness({ actions: SINGLE_ROUND_ACTIONS, ariaRoot });
  runtime.onCodingControl(planRepairRequiredWire());
  const parent = socketOf('session_plan_0001');
  const child = socketOf('session_child_0001');

  child.fireOpen();
  child.fireMessage(childHumanConfirmState());
  parent.fireOpen();
  assert.ok(
    parent.sent.some((message) => message.type === 'human_gate_feedback' && message.command_id === commandId),
    '确定性 command_id 重发/首发（进程重启后同值命中服务端 durable 查重）',
  );

  // 服务端对 Replayed 一律回 human_gate_turn_open，不区分 turn 是否已 Completed；
  // turn_completed 不会再来了——不回读 durable 记录就会永久等待。
  parent.fireMessage({ type: 'human_gate_turn_open', command_id: commandId, turn_id: 'turn_0001', remaining_budget: 0 });

  assert.deepEqual(hooks.failures, []);
  assert.ok(
    child.sent.some((message) => message.type === 'confirm_plan_amendment' && message.amendment_id === 'plan_amendment_0001'),
    '必须凭 durable terminal 记录推进游标并发出 confirm，而不是等待不会到来的 turn_completed',
  );
  assert.ok(
    hooks.logs.some((entry) => entry.event === 'amendment_turn_reconciled_from_durable'),
    '对账推进必须留下 amendment_turn_reconciled_from_durable 审计日志',
  );
  runtime.close();
});

test('Finding 2 守恒：durable 记录非 terminal（running）时回读不推进，仍由 WS turn_completed 正常结算', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'aria-amendment-liveness3-'));
  const ariaRoot = path.join(tmp, 'aria');
  const commandId = amendmentFeedbackCommandId({ attemptId: 'attempt_0001', actionIndex: 0 });
  writeDurableTurn({ ariaRoot, turn: durableTurnOf({ attemptId: 'attempt_0001', actionIndex: 0, turnId: 'turn_0001', status: 'running' }) });

  const { runtime, hooks, socketOf } = amendmentRuntimeHarness({ actions: SINGLE_ROUND_ACTIONS, ariaRoot });
  runtime.onCodingControl(planRepairRequiredWire());
  const parent = socketOf('session_plan_0001');
  const child = socketOf('session_child_0001');
  child.fireOpen();
  child.fireMessage(childHumanConfirmState());
  parent.fireOpen();

  parent.fireMessage({ type: 'human_gate_turn_open', command_id: commandId, turn_id: 'turn_0001', remaining_budget: 1 });
  assert.equal(
    child.sent.some((message) => message.type === 'confirm_plan_amendment'),
    false,
    'turn 仍在跑（durable status=running）：不得提前推进',
  );

  parent.fireMessage({ type: 'human_gate_turn_completed', turn_id: 'turn_0001', artifact_ref: 'artifact:rev_0002' });
  assert.deepEqual(hooks.failures, []);
  assert.ok(
    child.sent.some((message) => message.type === 'confirm_plan_amendment' && message.amendment_id === 'plan_amendment_0001'),
    '正常在线流程不受对账逻辑影响：WS turn_completed 到达即推进',
  );
  runtime.close();
});

test('Finding 2 重启自愈：进程重启后 cursor 归零重发，durable terminal 记录逐轮对账直到 confirm 发出', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'aria-amendment-liveness4-'));
  const ariaRoot = path.join(tmp, 'aria');
  writeDurableTurn({ ariaRoot, turn: durableTurnOf({ attemptId: 'attempt_0001', actionIndex: 0, turnId: 'turn_0001', status: 'completed' }) });
  writeDurableTurn({ ariaRoot, turn: durableTurnOf({ attemptId: 'attempt_0001', actionIndex: 1, turnId: 'turn_0002', status: 'completed' }) });
  const twoRounds = [
    { decision: 'request-change', description: '第一轮' },
    { decision: 'request-change', description: '第二轮' },
    { decision: 'confirm', description: null },
  ];

  const { runtime, hooks, socketOf } = amendmentRuntimeHarness({ actions: twoRounds, ariaRoot });
  runtime.onCodingControl(planRepairRequiredWire());
  const parent = socketOf('session_plan_0001');
  const child = socketOf('session_child_0001');
  const commandZero = amendmentFeedbackCommandId({ attemptId: 'attempt_0001', actionIndex: 0 });
  const commandOne = amendmentFeedbackCommandId({ attemptId: 'attempt_0001', actionIndex: 1 });

  child.fireOpen();
  child.fireMessage(childHumanConfirmState());
  parent.fireOpen();
  assert.ok(parent.sent.some((message) => message.type === 'human_gate_feedback' && message.command_id === commandZero));

  // 第 0 轮 replayed turn_open → durable terminal → 推进并立即续发第 1 轮（无需任何 WS 完结事件）。
  parent.fireMessage({ type: 'human_gate_turn_open', command_id: commandZero, turn_id: 'turn_0001', remaining_budget: 0 });
  assert.ok(
    parent.sent.some((message) => message.type === 'human_gate_feedback' && message.command_id === commandOne),
    '第一轮对账推进后必须立即续发第二轮 typed feedback',
  );

  // 第 1 轮 replayed turn_open → durable terminal → 推进到 confirm → 补发确认。
  parent.fireMessage({ type: 'human_gate_turn_open', command_id: commandOne, turn_id: 'turn_0002', remaining_budget: 0 });
  assert.deepEqual(hooks.failures, []);
  assert.ok(
    child.sent.some((message) => message.type === 'confirm_plan_amendment' && message.amendment_id === 'plan_amendment_0001'),
    '两轮 durable 对账后游标落到 confirm，确认必须发出',
  );
  runtime.close();
});

test('amendmentDurableGateTurns：回读 human-gate-turns/*.json 的 command/turn/status；缺目录/坏文件/参数缺失按无记录处理且绝不抛出', () => {
  const tmp = fs.mkdtempSync(path.join(os.tmpdir(), 'aria-amendment-durable-'));
  const ariaRoot = path.join(tmp, 'aria');
  // 目录不存在（尚无 turn 记录）→ 空列表。
  assert.deepEqual(
    amendmentDurableGateTurns({ ariaRoot, projectId: 'p', issueId: 'i', sessionId: 's' }),
    [],
  );
  const directory = path.join(ariaRoot, 'projects', 'p', 'issues', 'i', 'workspace-sessions', 's', 'human-gate-turns');
  fs.mkdirSync(directory, { recursive: true });
  fs.writeFileSync(path.join(directory, 'turn_0002.json'), JSON.stringify({
    turn_id: 'turn_0002', session_id: 's', command_id: 'cmd-b', feedback_text: '乙',
    status: 'failed', attempt_no: 1, budget_reserved: 1, source_hash: '',
    result_artifact_ref: null, failure_class: 'provider_err', created_at: 't', updated_at: 't',
  }), 'utf8');
  fs.writeFileSync(path.join(directory, 'turn_0001.json'), JSON.stringify({
    turn_id: 'turn_0001', session_id: 's', command_id: 'cmd-a', feedback_text: '甲',
    status: 'completed', attempt_no: 1, budget_reserved: 1, source_hash: 'b'.repeat(64),
    result_artifact_ref: 'artifact:rev_0002', failure_class: null, created_at: 't', updated_at: 't',
  }), 'utf8');
  fs.writeFileSync(path.join(directory, 'broken.json'), '{oops', 'utf8');
  fs.writeFileSync(path.join(directory, 'readme.txt'), '不是 json', 'utf8');
  assert.deepEqual(
    amendmentDurableGateTurns({ ariaRoot, projectId: 'p', issueId: 'i', sessionId: 's' }),
    [
      {
        command_id: 'cmd-a', turn_id: 'turn_0001', status: 'completed', attempt_no: 1,
        budget_reserved: 1, result_artifact_ref: 'artifact:rev_0002', failure_class: null,
      },
      {
        command_id: 'cmd-b', turn_id: 'turn_0002', status: 'failed', attempt_no: 1,
        budget_reserved: 1, result_artifact_ref: null, failure_class: 'provider_err',
      },
    ],
    '按 command_id 排序稳定输出；坏文件与非 .json 一律跳过',
  );
  // 参数缺失（生产侧未提供 project/issue/session）→ 空列表，不抛出。
  assert.deepEqual(amendmentDurableGateTurns({ ariaRoot, projectId: null, issueId: 'i', sessionId: 's' }), []);
  assert.deepEqual(amendmentDurableGateTurns({}), []);
});

test('amendmentReplayedTurnReconciliation：no_record / open_live / terminal / conflict 四态判定', () => {
  const turns = [
    {
      command_id: 'cmd-a', turn_id: 'turn_0001', status: 'completed', attempt_no: 1,
      budget_reserved: 1, result_artifact_ref: 'artifact:rev_0002', failure_class: null,
    },
    {
      command_id: 'cmd-b', turn_id: 'turn_0002', status: 'reserved', attempt_no: 1,
      budget_reserved: 1, result_artifact_ref: null, failure_class: null,
    },
    {
      command_id: 'cmd-c', turn_id: 'turn_0003', status: 'failed', attempt_no: 2,
      budget_reserved: 1, result_artifact_ref: null, failure_class: 'timeout',
    },
  ];
  assert.deepEqual(
    amendmentReplayedTurnReconciliation({ commandId: 'cmd-zz', turnId: 'turn_0009', turns }),
    { kind: 'no_record' },
  );
  assert.deepEqual(
    amendmentReplayedTurnReconciliation({ commandId: 'cmd-b', turnId: 'turn_0002', turns }),
    { kind: 'open_live', turn_id: 'turn_0002' },
    'turn 仍 reserved/running：照常等 WS 事件，不得提前推进',
  );
  assert.deepEqual(
    amendmentReplayedTurnReconciliation({ commandId: 'cmd-a', turnId: 'turn_0001', turns }),
    {
      kind: 'terminal', turn_id: 'turn_0001', status: 'completed', source: 'durable_turn_record',
      result_artifact_ref: 'artifact:rev_0002', failure_class: null,
    },
  );
  assert.deepEqual(
    amendmentReplayedTurnReconciliation({ commandId: 'cmd-c', turnId: 'turn_0003', turns }),
    {
      kind: 'terminal', turn_id: 'turn_0003', status: 'failed', source: 'durable_turn_record',
      result_artifact_ref: null, failure_class: 'timeout',
    },
    'failed 同样是 terminal：按 durable 结果走 amendmentGateTurnOutcome 的失败分支',
  );
  assert.deepEqual(
    amendmentReplayedTurnReconciliation({ commandId: 'cmd-a', turnId: 'turn_0099', turns }),
    { kind: 'conflict', observed_turn_id: 'turn_0099', record_turn_id: 'turn_0001' },
    'command 命中但 turn_id 不一致：durable 记录不可信，保守回退等待 WS 事件',
  );
});
