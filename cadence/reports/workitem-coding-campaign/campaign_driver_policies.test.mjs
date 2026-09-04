import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

import {
  activeNodeTypeForActiveNode,
  authorConfirmAction,
  applyResultTiming,
  confirmedCountForPlanStatus,
  collectUsageByRole,
  corpusSelectionFromEnv,
  createStage3GateController,
  createStage3GateHoldQueue,
  fixtureSetFromEnv,
  generationModeSelectionForNode,
  legacySingleCandidateDecisionMessage,
  hasMustFixFindings,
  humanConfirmAction,
  humanConfirmMessage,
  humanConfirmRequestChangeMessage,
  humanConfirmScriptAction,
  humanGateActionAudit,
  isHumanGateStage,
  parseHumanScript,
  shouldClearPendingGateNode,
  shouldConsumeHumanAction,
  singleCandidateOutboundAllowed,
  stage3GateWaitingSignal,
  stage3HumanMessage,
  stage3TypedFlowActive,
  providerRolesForSelection,
  reviewCompleteAction,
  reviewDecisionAction,
  reviewRepairAction,
  reviewCycleId,
  prepareOptionsForProvider,
  prepareOptionsForSet,
  resultTemplate,
  sessionStateProtocol,
  skipOptionalFindingsOption,
  terminalSessionAction,
} from './workitem_run_campaign.mjs';

const CAMPAIGN_DIR = path.dirname(fileURLToPath(import.meta.url));
import { preflightFailureOutDir } from './coding_run_campaign.mjs';

const EXPECTED_FLOW_KIND = process.env.ARIA_EXPECTED_FLOW_KIND ?? 'legacy';

test('needs_human review verdict waits for the durable server terminal before closing in auto mode', () => {
  assert.deepEqual(
    reviewCompleteAction('needs_human', 'auto_if_valid'),
    { kind: 'await_terminal_session_state' },
  );
  assert.deepEqual(reviewCompleteAction('pass', 'auto_if_valid'), { kind: 'continue' });
  assert.deepEqual(reviewCompleteAction('revise', 'auto_if_valid'), { kind: 'continue' });
});

test('interactive needs_human enters the human gate without waiting for a terminal', () => {
  assert.deepEqual(
    reviewCompleteAction('needs_human', 'interactive'),
    { kind: 'human_confirm_request_change' },
  );
});

test('interactive human_confirm messages use the HumanConfirm payload contract and preserve feedback verbatim', () => {
  const feedback = '请修复 API 契约\n保留此行末尾空格  ';
  assert.deepEqual(humanConfirmRequestChangeMessage(feedback), {
    type: 'human_confirm',
    decision: 'request-change',
    payload: { description: feedback, source: 'human' },
  });
});

test('human script parses multiple gate responses, preserves colons in feedback, and defaults to confirm after exhaustion', () => {
  const script = parseHumanScript('request-change:请补全 CT-001:能力声明并保持中文输出;confirm');

  assert.deepEqual(script, [
    { decision: 'request-change', description: '请补全 CT-001:能力声明并保持中文输出' },
    { decision: 'confirm', description: null },
  ]);
  assert.deepEqual(humanConfirmScriptAction(script, 0, 'interactive'), {
    decision: 'request-change',
    description: '请补全 CT-001:能力声明并保持中文输出',
    source: 'human_script',
  });
  assert.deepEqual(humanConfirmScriptAction(script, 1, 'interactive'), {
    decision: 'confirm',
    description: null,
    source: 'human_script',
  });
  assert.deepEqual(humanConfirmScriptAction(script, 2, 'interactive'), {
    decision: 'confirm',
    description: null,
    source: 'human_script_exhausted_default',
  });
});

test('interactive needs_human and final-pass gates use human_confirm messages and audit actions across both human stages', () => {
  const needsHumanMessage = humanConfirmMessage(humanConfirmScriptAction(
    parseHumanScript('request-change:请补全 CT-001 的能力声明'), 0, 'interactive',
  ));
  assert.deepEqual(needsHumanMessage, {
    type: 'human_confirm',
    decision: 'request-change',
    payload: { description: '请补全 CT-001 的能力声明', source: 'human' },
  });
  assert.deepEqual(
    humanGateActionAudit({
      sequence: 1,
      enteredAt: '2026-08-30T00:00:00.000Z',
      elapsedSec: 12.5,
      stage: 'human_confirm',
      reviewVerdict: 'needs_human',
      activeNodeId: 'human-gate-needs-human',
      response: humanConfirmScriptAction(parseHumanScript('request-change:请补全 CT-001 的能力声明'), 0, 'interactive'),
      message: needsHumanMessage,
      sentAt: '2026-08-30T00:00:01.000Z',
    }),
    {
      sequence: 1,
      enteredAt: '2026-08-30T00:00:00.000Z',
      elapsedSec: 12.5,
      stage: 'human_confirm',
      reviewVerdict: 'needs_human',
      active_node_id: 'human-gate-needs-human',
      source: 'human_script',
      message: {
        decision: 'request-change',
        description_summary: '请补全 CT-001 的能力声明',
      },
      sentAt: '2026-08-30T00:00:01.000Z',
    },
  );

  const finalPassMessage = humanConfirmMessage(humanConfirmScriptAction(
    parseHumanScript('confirm'), 0, 'interactive',
  ));
  assert.deepEqual(finalPassMessage, { type: 'human_confirm', decision: 'confirm', payload: null });
  assert.deepEqual(
    humanGateActionAudit({
      sequence: 2,
      enteredAt: '2026-08-30T00:00:02.000Z',
      elapsedSec: 18,
      stage: 'waiting_for_human',
      reviewVerdict: 'pass',
      activeNodeId: 'human-gate-final-pass',
      response: humanConfirmScriptAction(parseHumanScript('confirm'), 0, 'interactive'),
      message: finalPassMessage,
      sentAt: '2026-08-30T00:00:03.000Z',
    }),
    {
      sequence: 2,
      enteredAt: '2026-08-30T00:00:02.000Z',
      elapsedSec: 18,
      stage: 'waiting_for_human',
      reviewVerdict: 'pass',
      active_node_id: 'human-gate-final-pass',
      source: 'human_script',
      message: { decision: 'confirm', description_summary: null },
      sentAt: '2026-08-30T00:00:03.000Z',
    },
  );

  assert.equal(isHumanGateStage('human_confirm'), true);
  assert.equal(isHumanGateStage('waiting_for_human'), true);
  assert.equal(isHumanGateStage('review_decision'), false);
  assert.equal(
    shouldClearPendingGateNode('human_confirm', 'stage_change', null),
    true,
    '没有 active node 的 stage_change 必须等待 node 创建',
  );
  assert.equal(
    shouldClearPendingGateNode('waiting_for_human', 'stage_change', 'human-gate-final-pass'),
    false,
    '携带 active human node 的 stage_change 必须保留节点并立即走统一应答',
  );
  assert.equal(
    shouldClearPendingGateNode('human_confirm', 'stage_change', null, 'human_confirm'),
    false,
    'node_created 若先到达，后续未携带 active_node_id 的 stage_change 仍须保留已识别的人工节点',
  );
  assert.equal(
    activeNodeTypeForActiveNode(new Map(), 'next-human-gate', 'previous-human-gate', 'human_confirm'),
    null,
    '新 active_node_id 尚未随事件携带 node 时，不得沿用旧 human_confirm 类型而向旧门发送应答',
  );
  assert.equal(
    activeNodeTypeForActiveNode(
      new Map([['next-human-gate', { node_type: 'human_confirm' }]]),
      'next-human-gate',
      'previous-human-gate',
      null,
    ),
    'human_confirm',
  );
  assert.equal(singleCandidateOutboundAllowed(needsHumanMessage, 'interactive'), true);
  assert.equal(singleCandidateOutboundAllowed(finalPassMessage, 'auto_if_valid'), false);
  assert.equal(singleCandidateOutboundAllowed({ type: 'stage_change', stage: 'human_confirm' }, 'interactive'), false);
  assert.equal(
    legacySingleCandidateDecisionMessage({ type: 'stage_change', stage: 'human_confirm' }, 'interactive'),
    null,
    'interactive SingleCandidate human gates must reach the unified human_confirm responder',
  );
  assert.equal(
    legacySingleCandidateDecisionMessage({ type: 'stage_change', stage: 'waiting_for_human' }, 'interactive'),
    null,
    'waiting_for_human is the same interactive human gate',
  );
  const autoHumanGate = { type: 'stage_change', stage: 'human_confirm' };
  assert.deepEqual(
    legacySingleCandidateDecisionMessage(autoHumanGate, 'auto_if_valid'),
    autoHumanGate,
    'auto 模式仍将 SingleCandidate 的旧人工门消息视为协议回归',
  );
  const driverSource = fs.readFileSync(path.join(CAMPAIGN_DIR, 'workitem_run_campaign.mjs'), 'utf8');
  assert.doesNotMatch(
    driverSource,
    /send\(\{\s*type:\s*['"]stage_change['"]/u,
    'interactive 人工门不得发送旧 stage_change 协议',
  );
});

test('campaign_stage3_script_parses_multiple_request_changes_confirm_abandon_and_advance', () => {
  const script = parseHumanScript(
    'request-change:合成反馈 1 号;request-change:合成反馈 2 号;confirm;abandon;advance',
  );
  assert.deepEqual(script, [
    { decision: 'request-change', description: '合成反馈 1 号' },
    { decision: 'request-change', description: '合成反馈 2 号' },
    { decision: 'confirm', description: null },
    { decision: 'abandon', description: null },
    { decision: 'advance', description: null },
  ]);
  assert.deepEqual(parseHumanScript('abandon;advance'), [
    { decision: 'abandon', description: null },
    { decision: 'advance', description: null },
  ]);
  assert.throws(() => parseHumanScript('abandon:理由'), /abandon 不接受描述/u);
  assert.throws(() => parseHumanScript('advance:参数'), /advance 不接受描述/u);
  assert.throws(() => parseHumanScript('confirm:描述'), /confirm 不接受描述/u);
  assert.throws(() => parseHumanScript('request-change:'), /必须提供冒号后的反馈文本/u);
  assert.throws(() => parseHumanScript('retry'), /仅支持 confirm、request-change:<文本>、abandon 或 advance/u);
  assert.deepEqual(parseHumanScript(undefined), []);

  // 阶段 2 的 gate-index 语义保持不变：abandon/advance 不进入 legacy 应答。
  assert.equal(humanConfirmScriptAction(script, 0, 'interactive').description, '合成反馈 1 号');
  assert.deepEqual(humanConfirmScriptAction(script, 5, 'interactive'), {
    decision: 'confirm',
    description: null,
    source: 'human_script_exhausted_default',
  });
  assert.throws(
    () => humanConfirmMessage({ decision: 'abandon', description: null, source: 'human_script' }),
    /不支持的人工确认决策/u,
    'legacy 消息编码继续 fail-closed，不悄悄扩展 legacy 枚举',
  );
});

test('campaign_stage3_legacy_request_change_wire_stays_unchanged', () => {
  // 阶段 2 既有 wire 拼写与 payload 逐字保留（legacy flow 行为不变）。
  assert.deepEqual(humanConfirmRequestChangeMessage('请修订甲'), {
    type: 'human_confirm',
    decision: 'request-change',
    payload: { description: '请修订甲', source: 'human' },
  });
  assert.deepEqual(humanConfirmMessage({ decision: 'confirm', description: null, source: 'human_script' }), {
    type: 'human_confirm',
    decision: 'confirm',
    payload: null,
  });

  // typed flow 只在 single_candidate+interactive 激活，且绝不回退到旧 request-change 编码。
  assert.equal(stage3TypedFlowActive('single_candidate', 'interactive'), true);
  assert.equal(stage3TypedFlowActive('legacy', 'interactive'), false);
  assert.equal(stage3TypedFlowActive('single_candidate', 'auto_if_valid'), false);
  const typed = stage3HumanMessage(
    { decision: 'request-change', description: '请修订甲' },
    { commandId: 'cmd-fixture' },
  );
  assert.equal(typed.type, 'human_gate_feedback');
  assert.notEqual(typed.decision, 'request-change');

  // 出站 allowlist：typed 命令（feedback/advance/裸 confirm）只对 interactive 放行，legacy 决策族维持拒绝。
  assert.equal(singleCandidateOutboundAllowed({ type: 'human_confirm' }, 'interactive'), true);
  assert.equal(singleCandidateOutboundAllowed({ type: 'human_gate_feedback' }, 'interactive'), true);
  assert.equal(singleCandidateOutboundAllowed({ type: 'advance' }, 'interactive'), true);
  assert.equal(singleCandidateOutboundAllowed({ type: 'confirm' }, 'interactive'), true,
    'SC typed 流的 confirm 按 REQ-CG-02 准入表编码为裸 typed 消息，只对 interactive 放行');
  assert.equal(singleCandidateOutboundAllowed({ type: 'human_gate_feedback' }, 'auto_if_valid'), false);
  assert.equal(singleCandidateOutboundAllowed({ type: 'advance' }, 'auto_if_valid'), false);
  assert.equal(singleCandidateOutboundAllowed({ type: 'confirm' }, 'auto_if_valid'), false);
  assert.equal(singleCandidateOutboundAllowed({ type: 'review_decision_response' }, 'interactive'), false);

  // legacy 门阶段的消费判定不因 stage-3 事件误触发（阶段 2 回归护栏）。
  assert.equal(
    shouldConsumeHumanAction({ type: 'human_gate_turn_open', command_id: 'c', turn_id: 't' }, {}, { decision: 'confirm', commandId: null }),
    false,
  );
});

test('human script is inert outside interactive policy', () => {
  const script = parseHumanScript('request-change:不应在 auto 模式发送');
  assert.equal(humanConfirmScriptAction(script, 0, 'auto_if_valid'), null);
  assert.equal(
    Object.hasOwn(resultTemplate('codex', 1, '/tmp/out', {}, 'digest', 'auto_if_valid'), 'humanGateActions'),
    false,
    'auto 模式的 result.json 模板不新增人工门字段',
  );
  assert.deepEqual(
    humanConfirmAction({
      activeNodeType: 'human_confirm',
      reviewVerdict: 'needs_human',
      reviewComplete: { findings: [] },
    }),
    { kind: 'request_change' },
  );
});

test('interactive result template reserves the human gate action audit trail', () => {
  assert.deepEqual(
    resultTemplate('codex', 1, '/tmp/out', {}, 'digest', 'interactive').humanGateActions,
    [],
  );
});

test('interactive human gates distinguish needs_human/manual revision from the final pass confirmation', () => {
  assert.equal(hasMustFixFindings({ findings: [{ severity: 'must_fix', message: '仍需修复' }] }), true);
  assert.deepEqual(
    humanConfirmAction({
      activeNodeType: 'human_confirm',
      reviewVerdict: 'needs_human',
      reviewComplete: { findings: [{ severity: 'must_fix', message: '修复' }] },
    }),
    { kind: 'request_change' },
  );
  assert.deepEqual(
    humanConfirmAction({
      activeNodeType: 'human_confirm',
      reviewVerdict: 'revise',
      reviewComplete: { findings: [] },
      manualRepairsUsed: 2,
    }),
    { kind: 'request_change' },
  );
  assert.deepEqual(
    humanConfirmAction({
      activeNodeType: 'human_confirm',
      reviewVerdict: 'needs_human',
      reviewComplete: { findings: [] },
      manualRepairsUsed: 3,
    }),
    { kind: 'fail', failureClass: 'manual_budget_exhausted' },
  );
  assert.deepEqual(
    humanConfirmAction({
      activeNodeType: 'human_confirm',
      reviewVerdict: 'pass',
      reviewComplete: { findings: [{ severity: 'suggestion', message: '可选建议' }] },
    }),
    { kind: 'confirm' },
    'a clean re-review reaches the final approval gate even without a prior manual request',
  );
  assert.deepEqual(
    humanConfirmAction({
      activeNodeType: 'human_confirm',
      reviewVerdict: 'pass',
      reviewComplete: { findings: [{ severity: 'must_fix', message: '历史门快照残留' }] },
      humanGateSnapshot: { findings: [{ severity: 'must_fix', message: '历史门快照残留' }] },
      manualRepairRequestsSent: 1,
    }),
    { kind: 'confirm' },
    'the latest pass verdict, not stale needs_human gate findings, selects final confirmation',
  );
  assert.deepEqual(
    humanConfirmAction({
      activeNodeType: 'human_confirm',
      reviewVerdict: null,
      reviewComplete: null,
    }),
    { kind: 'fail', failureClass: 'human_confirm_verdict_unknown' },
  );
});

test('usage collection recursively extracts execution_event usage by role', () => {
  const usageByRole = {};
  collectUsageByRole({
    type: 'execution_event',
    event: {
      kind: 'usage',
      output: JSON.stringify({
        role: 'reviewer',
        input_tokens: 101,
        output_tokens: 202,
        cache_read_tokens: 303,
        cache_creation_tokens: 404,
      }),
    },
  }, usageByRole);

  assert.deepEqual(usageByRole, {
    reviewer: {
      input_tokens: 101,
      output_tokens: 202,
      cache_read_tokens: 303,
      cache_creation_tokens: 404,
    },
  });
});

test('review decision deterministically skips optional findings and records the exact option', () => {
  const action = reviewDecisionAction({
    options: ['apply_optional_findings', 'skip_optional_findings'],
  }, 'pass');

  assert.deepEqual(action, {
    kind: 'respond',
    decision: 'skip_optional_findings',
    strategy: 'skip_optional_findings_keyword',
    extra_context: null,
  });
  assert.equal(skipOptionalFindingsOption(['apply_optional_findings', 'skip_optional_findings']), 'skip_optional_findings');
});

test('review decision fails closed when provided options do not include a skip-optional choice', () => {
  assert.deepEqual(
    reviewDecisionAction({ options: ['continue', 'human_intervene'] }, 'pass'),
    { kind: 'fail', strategy: 'unhandled_options' },
  );
});

test('review decision follows the standard revise options through review_decision_response', () => {
  assert.deepEqual(
    reviewDecisionAction(
      { options: ['continue', 'continue_with_context', 'human_intervene'] },
      'revise',
      { findings: [
        { required_action: '补充后端契约校验' },
        { required_action: '明确集成测试边界' },
      ] },
    ),
    {
      kind: 'respond',
      decision: 'continue_with_context',
      strategy: 'revise_continue_with_context',
      extra_context: '补充后端契约校验\n明确集成测试边界',
    },
  );
  assert.deepEqual(
    reviewDecisionAction({ options: ['continue'] }, 'revise'),
    { kind: 'respond', decision: 'continue', strategy: 'revise_continue', extra_context: null },
  );
  assert.deepEqual(
    reviewDecisionAction({ options: ['continue_with_context'] }, 'pass'),
    { kind: 'fail', strategy: 'unhandled_options' },
  );
});

test('continue_with_context uses findings required actions and a deterministic fallback', () => {
  assert.equal(
    reviewDecisionAction(
      { options: ['continue_with_context'] },
      'revise',
      { findings: [
        { required_action: '补充后端契约校验' },
        { required_action: '明确集成测试边界' },
      ] },
    ).extra_context,
    '补充后端契约校验\n明确集成测试边界',
  );
  assert.equal(
    reviewDecisionAction({ options: ['continue_with_context'] }, 'revise', { findings: [] }).extra_context,
    '请根据评审意见修订',
  );
  assert.equal(
    reviewDecisionAction({ options: ['apply_optional_findings', 'skip_optional_findings'] }, 'revise', {
      findings: [{ required_action: '不要发送给跳过可选建议' }],
    }).extra_context,
    null,
  );
});

test('review decision preserves the legacy revise-only path only when options are absent', () => {
  assert.deepEqual(
    reviewDecisionAction({}, 'revise'),
    { kind: 'revise', decision: 'revise-with-context', strategy: 'legacy_revise_without_options' },
  );
  assert.deepEqual(
    reviewDecisionAction({}, 'pass'),
    { kind: 'fail', strategy: 'unhandled_without_options' },
  );
  assert.deepEqual(
    reviewDecisionAction({ options: [] }, 'revise'),
    { kind: 'revise', decision: 'revise-with-context', strategy: 'legacy_revise_without_options' },
  );
});

test('同一会话的两个 generation-mode 节点各发送一次选择，重复事件不重复发送', () => {
  const respondedNodeIds = new Set();
  const expected = { type: 'select_work_item_generation_mode', mode: 'batch' };

  assert.deepEqual(generationModeSelectionForNode('generation-mode-1', respondedNodeIds), expected);
  assert.equal(generationModeSelectionForNode('generation-mode-1', respondedNodeIds), null);
  assert.deepEqual(generationModeSelectionForNode('generation-mode-2', respondedNodeIds), expected);
  assert.equal(generationModeSelectionForNode('generation-mode-2', respondedNodeIds), null);
});

test('author confirmation accepts the generic revision gate while failing closed for unknown nodes and missing Draft artifacts', () => {
  assert.deepEqual(
    authorConfirmAction('author_confirm', true),
    { kind: 'author_accept_with_review' },
  );
  assert.deepEqual(
    authorConfirmAction('unrecognized_author_confirm', true),
    { kind: 'fail', failureClass: 'unexpected_author_confirm' },
  );
  assert.deepEqual(
    authorConfirmAction('work_item_draft_confirm', false),
    { kind: 'fail', failureClass: 'draft_artifact_missing' },
  );
  assert.deepEqual(
    authorConfirmAction('work_item_draft_confirm', true),
    { kind: 'draft_decision' },
  );
});

test('provider selection limits responses to defaults or a single fail-closed author assumption', () => {
  assert.deepEqual(
    providerRolesForSelection({ defaults: { reviewer: 'codex' } }, 'prepare_context'),
    { roles: ['reviewer'], strategy: 'message_defaults', assumption: null },
  );
  assert.deepEqual(
    providerRolesForSelection({}, 'cross_review'),
    { roles: ['reviewer'], strategy: 'stage_reviewer', assumption: null },
  );
  assert.deepEqual(
    providerRolesForSelection({}, null),
    {
      roles: ['author'],
      strategy: 'author_fallback',
      assumption: 'provider_select_request 未提供可识别 defaults 或 stage；按保守假设仅选择 author。',
    },
  );
});

test('coding preflight failure directories include their supplied timestamp', () => {
  assert.equal(
    preflightFailureOutDir('/tmp/aria-campaign', 'codex', '20260826010203123'),
    '/tmp/aria-campaign/coding-codex-preflight-failed-20260826010203123',
  );
});

function sessionState(overrides = {}) {
  return {
    type: 'session_state',
    session_status: 'running',
    flow_kind: EXPECTED_FLOW_KIND,
    run_policy: 'auto_if_valid',
    run_history: {
      repairs_used: 2,
      manual_repairs_used: 1,
      transitions_used: 4,
      initial_review_count: 3,
      verification_review_count: 2,
      review_cycles: {
        'outline:outline_0001': {
          repairs_used: 1,
          initial_count: 1,
          verification_count: 1,
        },
        'draft:outline_0002': {
          repairs_used: 1,
          initial_count: 1,
          verification_count: 0,
        },
      },
    },
    provider_start_ledger: [
      { provider_start_idempotency_key: 'provider-start-1', started: true },
      { provider_start_idempotency_key: 'provider-start-1', started: true },
      { provider_start_idempotency_key: 'provider-start-2', started: true },
      { provider_start_idempotency_key: 'reserved-only', started: false },
    ],
    ...overrides,
  };
}

test('session state protocol locks the configured flow policy and reads durable counters', () => {
  const protocol = sessionStateProtocol(sessionState());

  assert.deepEqual(protocol, {
    session_status: 'running',
    flow_kind: EXPECTED_FLOW_KIND,
    run_policy: 'auto_if_valid',
    run_history: {
      repairs_used: 2,
      manual_repairs_used: 1,
      transitions_used: 4,
      initial_review_count: 3,
      verification_review_count: 2,
      review_cycles: {
        'outline:outline_0001': {
          repairs_used: 1,
          initial_count: 1,
          verification_count: 1,
        },
        'draft:outline_0002': {
          repairs_used: 1,
          initial_count: 1,
          verification_count: 0,
        },
      },
    },
    policy_diagnostics: [],
    human_gate_snapshot: null,
    provider_start_ledger: [
      { provider_start_idempotency_key: 'provider-start-1', started: true },
      { provider_start_idempotency_key: 'provider-start-2', started: true },
      { provider_start_idempotency_key: 'reserved-only', started: false },
    ],
    provider_start_count: 2,
  });
});

test('session state protocol rejects flow or policy drift rather than accepting a different flow', () => {
  const driftedFlowKind = EXPECTED_FLOW_KIND === 'legacy' ? 'single_candidate' : 'legacy';
  assert.throws(
    () => sessionStateProtocol(sessionState({ flow_kind: driftedFlowKind })),
    new RegExp(`flow_kind.*${EXPECTED_FLOW_KIND}`, 'i'),
  );
  assert.throws(
    () => sessionStateProtocol(sessionState({ run_policy: 'interactive' })),
    /run_policy.*auto_if_valid/i,
  );
  assert.throws(
    () => sessionStateProtocol(sessionState({ run_history: null })),
    /run_history/i,
  );
  assert.throws(
    () => sessionStateProtocol({ type: 'stage_change' }),
    /首条状态消息必须是 session_state/,
  );
});

test('session state protocol fails closed when a durable cycle exceeds one review phase', () => {
  const invalid = sessionState({
    run_history: {
      ...sessionState().run_history,
      review_cycles: {
        'batch:batch_0001': {
          repairs_used: 0,
          initial_count: 2,
          verification_count: 0,
        },
      },
    },
  });

  assert.throws(() => sessionStateProtocol(invalid), /initial_count.*至多为 1/i);
});

test('automatic revision guard uses the matching durable review cycle instead of cross_review stage entries', () => {
  const reviewComplete = {
    type: 'review_complete',
    work_item_plan_review: {
      review_scope: 'batch',
      batch_id: 'batch_0001',
    },
  };
  assert.equal(reviewCycleId(reviewComplete), 'batch:batch_0001');
  assert.deepEqual(
    reviewRepairAction(sessionStateProtocol(sessionState({
      run_history: {
        ...sessionState().run_history,
        review_cycles: {
          'batch:batch_0001': {
            repairs_used: 0,
            initial_count: 1,
            verification_count: 0,
          },
          'batch:exhausted_elsewhere': {
            repairs_used: 1,
            initial_count: 1,
            verification_count: 1,
          },
        },
      },
    })).run_history, reviewComplete),
    { kind: 'continue' },
  );
  assert.deepEqual(
    reviewRepairAction(sessionStateProtocol(sessionState({
      run_history: {
        ...sessionState().run_history,
        review_cycles: {
          'batch:batch_0001': {
            repairs_used: 1,
            initial_count: 1,
            verification_count: 1,
          },
        },
      },
    })).run_history, reviewComplete),
    { kind: 'fail', failureClass: 'review_revision_limit' },
  );
  assert.deepEqual(
    reviewRepairAction(null, reviewComplete),
    { kind: 'fail', failureClass: 'durable_run_history_missing' },
  );
  assert.deepEqual(
    reviewRepairAction(sessionStateProtocol(sessionState()).run_history, { type: 'review_complete' }),
    { kind: 'fail', failureClass: 'durable_review_cycle_unresolved' },
  );
  assert.deepEqual(
    reviewRepairAction(sessionStateProtocol(sessionState({
      run_history: {
        ...sessionState().run_history,
        review_cycles: {},
      },
    })).run_history, reviewComplete),
    { kind: 'fail', failureClass: 'durable_review_cycle_missing' },
  );
});

test('terminal session states classify durable terminal outcomes and retain structured policy diagnostics', () => {
  assert.deepEqual(
    terminalSessionAction(sessionState({ session_status: 'waiting_for_human' })),
    { kind: 'continue' },
    'a pre-review waiting state must not close the campaign',
  );
  assert.deepEqual(
    terminalSessionAction(sessionState({ session_status: 'waiting_for_human' }), false, true, 'legacy'),
    { kind: 'terminal', failureClass: 'awaiting_human' },
    'a completed stage must finish after durable readback records a human gate',
  );
  assert.deepEqual(
    terminalSessionAction(sessionState({ session_status: 'waiting_for_human' }), true, false, 'legacy'),
    { kind: 'terminal', failureClass: 'awaiting_human' },
    'a needs_human review must close only after the server records waiting_for_human',
  );
  const stoppedNeedsHuman = sessionState({
    session_status: 'stopped_needs_human',
    human_gate_snapshot: {
      trigger: 'review_needs_human',
      attempts_used: 1,
      resumable: true,
    },
  });
  assert.deepEqual(
    terminalSessionAction(stoppedNeedsHuman),
    { kind: 'terminal', failureClass: 'stopped_needs_human' },
  );
  assert.deepEqual(
    sessionStateProtocol(stoppedNeedsHuman).human_gate_snapshot,
    stoppedNeedsHuman.human_gate_snapshot,
    'durable gate snapshot must be available for result.json',
  );
  const policyFailed = sessionState({
    session_status: 'failed',
    policy_diagnostics: [{
      code: 'unknown_finding_category',
      message: 'unrecognized category',
      field: 'findings[0].category',
    }],
  });
  assert.deepEqual(
    terminalSessionAction(policyFailed),
    { kind: 'terminal', failureClass: 'policy_failed' },
  );
  assert.deepEqual(
    sessionStateProtocol(policyFailed).policy_diagnostics,
    policyFailed.policy_diagnostics,
    'durable policy diagnostics must be available for result.json',
  );
  assert.deepEqual(
    terminalSessionAction(sessionState({ session_status: 'failed' })),
    { kind: 'terminal', failureClass: 'policy_failed' },
  );
  assert.deepEqual(
    terminalSessionAction(sessionState({ session_status: 'confirmed' })),
    { kind: 'complete' },
  );
});

test('non-terminal policy diagnostics are parsed and fatal diagnostic codes fail before review', () => {
  const state = sessionState({
    policy_diagnostics: [{ code: 'unknown_finding_category', message: 'bad', field: null }],
  });

  assert.deepEqual(
    terminalSessionAction(state),
    { kind: 'fail', failureClass: 'unknown_finding_category' },
  );
  assert.deepEqual(
    sessionStateProtocol(state).policy_diagnostics,
    [{ code: 'unknown_finding_category', message: 'bad', field: null }],
  );
});

test('prepare request serializes the auto policy using the server snake_case field name', () => {
  assert.deepEqual(
    prepareOptionsForProvider('codex'),
    {
      story_spec_ids: ['story_spec_0001'],
      design_spec_ids: ['design_spec_0001'],
      author_provider: 'codex',
      reviewer_provider: 'codex',
      review_rounds: 1,
      superpowers_enabled: true,
      openspec_enabled: true,
      run_policy: 'auto_if_valid',
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
    },
  );
});

test('prepareOptionsForSet 按 fixture set 联动：levels 默认保持全栈形态不变', () => {
  assert.deepEqual(
    prepareOptionsForSet('levels'),
    {
      story_spec_ids: ['story_spec_0001'],
      design_spec_ids: ['design_spec_0001'],
      author_provider: null,
      reviewer_provider: null,
      review_rounds: 1,
      superpowers_enabled: true,
      openspec_enabled: true,
      run_policy: 'auto_if_valid',
      include_integration_tests: true,
      include_e2e_tests: false,
      force_frontend_backend_split: true,
      require_execution_plan_confirm: false,
    },
  );
});

test('prepareOptionsForSet minimal/defect 为单契约后端工作项关闭拆分与集成测试', () => {
  const minimal = prepareOptionsForSet('minimal');
  const defect = prepareOptionsForSet('defect');
  for (const options of [minimal, defect]) {
    assert.equal(options.force_frontend_backend_split, false);
    assert.equal(options.include_integration_tests, false);
    assert.equal(options.include_e2e_tests, false);
  }
  // 除联动三开关外，其余字段必须与 levels 逐字一致。
  const levels = prepareOptionsForSet('levels');
  for (const options of [minimal, defect]) {
    for (const [key, value] of Object.entries(levels)) {
      if (key === 'force_frontend_backend_split' || key === 'include_integration_tests') continue;
      assert.deepEqual(options[key], value, `字段 ${key} 必须与 levels 一致`);
    }
  }
});

test('prepareOptionsForSet 对未知 fixture set fail-closed', () => {
  assert.throws(() => prepareOptionsForSet('bogus'), /不支持的 fixture set/);
});

test('SingleCandidate 的 result 模板承载确认数、持续时间、完整去重账本与旧决策审计', () => {
  const result = resultTemplate('codex', 1, '/tmp/out', {}, 'digest', 'auto_if_valid');

  assert.equal(result.confirmed_count, 0);
  assert.equal(result.duration_ms, null);
  assert.deepEqual(result.provider_start_ledger, []);
  assert.deepEqual(result.legacy_decision_messages, []);
  const single = sessionState({ flow_kind: 'single_candidate' });
  assert.deepEqual(
    sessionStateProtocol(single, 'auto_if_valid', 'single_candidate').provider_start_ledger,
    [
      { provider_start_idempotency_key: 'provider-start-1', started: true },
      { provider_start_idempotency_key: 'provider-start-2', started: true },
      { provider_start_idempotency_key: 'reserved-only', started: false },
    ],
    'durable ledger 必须按 idempotency key 保留完整规范化条目，而不是只保留 count',
  );
  assert.equal(
    sessionStateProtocol(single, 'auto_if_valid', 'single_candidate').provider_start_count,
    2,
    'provider_start_count 仅是去重账本的派生观测字段',
  );
});

test('SingleCandidate 首个 durable session state 对 flow/policy/history 失败关闭', () => {
  const single = sessionState({ flow_kind: 'single_candidate' });
  assert.equal(
    sessionStateProtocol(single, 'auto_if_valid', 'single_candidate').flow_kind,
    'single_candidate',
  );
  assert.throws(
    () => sessionStateProtocol(sessionState({ flow_kind: 'legacy' }), 'auto_if_valid', 'single_candidate'),
    /flow_kind.*single_candidate/i,
  );
  assert.throws(
    () => sessionStateProtocol(single, 'interactive', 'single_candidate'),
    /run_policy.*interactive/i,
  );
  assert.throws(
    () => sessionStateProtocol(sessionState({ flow_kind: 'single_candidate', run_history: null }), 'auto_if_valid', 'single_candidate'),
    /run_history/i,
  );
});

test('SingleCandidate 只接受规定终态，并对旧决策请求记录协议回归而不自动应答', () => {
  assert.deepEqual(
    terminalSessionAction(sessionState({ flow_kind: 'single_candidate', session_status: 'confirmed' }), false, false, 'single_candidate'),
    { kind: 'complete' },
  );
  assert.deepEqual(
    terminalSessionAction(sessionState({ flow_kind: 'single_candidate', session_status: 'stopped_needs_human' }), false, false, 'single_candidate'),
    { kind: 'terminal', failureClass: 'stopped_needs_human' },
  );
  assert.deepEqual(
    terminalSessionAction(sessionState({ flow_kind: 'single_candidate', session_status: 'failed' }), false, false, 'single_candidate'),
    { kind: 'terminal', failureClass: 'policy_failed' },
  );
  assert.deepEqual(
    terminalSessionAction(sessionState({ flow_kind: 'single_candidate', session_status: 'waiting_for_human' }), true, true, 'single_candidate'),
    { kind: 'continue' },
    'waiting_for_human 不是 SingleCandidate 接受的终态',
  );
  for (const message of [
    { type: 'provider_select_request' },
    { type: 'review_decision_required' },
    { type: 'stage_change', stage: 'human_confirm' },
    { type: 'timeline_node_created', node: { node_type: 'human_confirm' } },
    { type: 'timeline_node_created', node: { node_type: 'work_item_generation_mode' } },
    { type: 'timeline_node_created', node: { node_type: 'work_item_plan_outline_confirm' } },
    { type: 'timeline_node_created', node: { node_type: 'work_item_draft_confirm' } },
    { type: 'timeline_node_created', node: { node_type: 'work_item_batch_confirm' } },
  ]) {
    assert.deepEqual(legacySingleCandidateDecisionMessage(message), message);
  }
  assert.equal(legacySingleCandidateDecisionMessage({ type: 'review_complete' }), null);
});

test('Confirmed lifecycle plan 仅在验证确认后将每案 confirmed_count 写为 1，并记录真实 duration_ms', () => {
  assert.equal(confirmedCountForPlanStatus('Confirmed'), 1);
  assert.equal(confirmedCountForPlanStatus('confirmed'), 1);
  assert.equal(confirmedCountForPlanStatus('failed'), 0);
  assert.equal(confirmedCountForPlanStatus(null), 0);
  const result = { finishedAt: null, elapsedSec: null, duration_ms: null };
  applyResultTiming(result, 1_234);
  assert.equal(result.duration_ms, 1_234);
  assert.equal(result.elapsedSec, 1.234);
  assert.match(result.finishedAt, /^\d{4}-\d{2}-\d{2}T/);
});

test('dry-run 忽略 ARIA_DATA_ROOT，固定报告当前 worktree 的 .aria 且不连接服务', () => {
  const driver = path.join(CAMPAIGN_DIR, 'workitem_run_campaign.mjs');
  const repoRoot = path.resolve(CAMPAIGN_DIR, '../../..');
  const run = spawnSync(
    process.execPath,
    [driver, 'codex', '1', '/tmp/aria-phase2-policy-test', '--dry-run'],
    {
      cwd: repoRoot,
      encoding: 'utf8',
      env: {
        ...process.env,
        ARIA_DATA_ROOT: '/tmp/must-not-be-used-by-workitem-driver',
        ARIA_EXPECTED_FLOW_KIND: 'single_candidate',
        ARIA_RUN_POLICY: 'auto_if_valid',
        ARIA_WORKITEM_HARD_TIMEOUT_MS: '720000',
      },
    },
  );
  assert.equal(run.status, 0, run.stderr);
  const output = JSON.parse(run.stdout);
  assert.equal(output.aria_data_root, path.join(repoRoot, '.aria'));
  assert.equal(output.no_http_or_websocket_requests, true);
  assert.equal(output.prepare_options.run_policy, 'auto_if_valid');
});

test('fixture set 映射默认与显式', () => {
  assert.deepEqual(fixtureSetFromEnv({}), { corpus: '07-fullstack-levels.md', set: 'levels' });
  assert.deepEqual(fixtureSetFromEnv({ ARIA_FIXTURE_SET: 'minimal' }), { corpus: '08-minimal-hello-api.md', set: 'minimal' });
  assert.deepEqual(fixtureSetFromEnv({ ARIA_FIXTURE_SET: 'defect' }), { corpus: '09-amendment-defect.md', set: 'defect' });
});

test('fixture set 未知值与未知语料均 fail-closed', () => {
  assert.throws(() => fixtureSetFromEnv({ ARIA_FIXTURE_SET: 'bogus' }), /ARIA_FIXTURE_SET/);
  assert.throws(() => fixtureSetFromEnv({ ARIA_ISSUE_CORPUS_FILE: '07-fullstack-levels.md', ARIA_FIXTURE_SET: 'minimal' }), /ARIA_/);
});

test('corpusSelectionFromEnv 与 fixtureSetFromEnv 同构，显式语料仅在与套映射一致时放行', () => {
  for (const env of [{}, { ARIA_FIXTURE_SET: 'minimal' }, { ARIA_FIXTURE_SET: 'defect' }]) {
    assert.deepEqual(corpusSelectionFromEnv(env), fixtureSetFromEnv(env));
  }
  assert.deepEqual(
    corpusSelectionFromEnv({ ARIA_FIXTURE_SET: 'minimal', ARIA_ISSUE_CORPUS_FILE: '08-minimal-hello-api.md' }),
    { corpus: '08-minimal-hello-api.md', set: 'minimal' },
  );
  assert.throws(
    () => corpusSelectionFromEnv({ ARIA_ISSUE_CORPUS_FILE: '06-multi-constraint.md' }),
    /ARIA_/,
  );
});

test('dry-run 对未知 fixture set fail-closed（非零退出且错误信息含 ARIA_）', () => {
  const driver = path.join(CAMPAIGN_DIR, 'workitem_run_campaign.mjs');
  const repoRoot = path.resolve(CAMPAIGN_DIR, '../../..');
  const run = spawnSync(
    process.execPath,
    [driver, 'codex', '1', '/tmp/aria-phase2-policy-test', '--dry-run'],
    {
      cwd: repoRoot,
      encoding: 'utf8',
      env: { ...process.env, ARIA_FIXTURE_SET: 'bogus' },
    },
  );
  assert.notEqual(run.status, 0);
  assert.match(run.stderr, /ARIA_FIXTURE_SET/);
});

test('dry-run 的 prepare_options 反映选中 fixture set 的联动值（minimal 关闭拆分与集成测试）', () => {
  const driver = path.join(CAMPAIGN_DIR, 'workitem_run_campaign.mjs');
  const repoRoot = path.resolve(CAMPAIGN_DIR, '../../..');
  for (const set of ['minimal', 'defect']) {
    const run = spawnSync(
      process.execPath,
      [driver, 'codex', '1', '/tmp/aria-phase2-policy-test', '--dry-run'],
      {
        cwd: repoRoot,
        encoding: 'utf8',
        env: { ...process.env, ARIA_FIXTURE_SET: set },
      },
    );
    assert.equal(run.status, 0, run.stderr);
    const output = JSON.parse(run.stdout);
    assert.equal(output.fixture_set, set);
    assert.equal(output.prepare_options.force_frontend_backend_split, false);
    assert.equal(output.prepare_options.include_integration_tests, false);
    assert.equal(output.prepare_options.include_e2e_tests, false);
  }
});

test('coding driver 只消费 Confirmed handoff，不读取 WorkItem SingleCandidate 协议也不发送其旧决策', () => {
  const source = fs.readFileSync(path.join(CAMPAIGN_DIR, 'coding_run_campaign.mjs'), 'utf8');
  assert.match(source, /只消费已确认的 Work Item Plan handoff/);
  for (const forbiddenProtocolField of [
    'flow_kind',
    'generation_mode',
    'review_scope',
    'run_history',
    'provider_start_ledger',
    'author_decision',
    'select_work_item_generation_mode',
    'work_item_draft_decision',
    'work_item_batch_decision',
    'review_decision_response',
  ]) {
    assert.doesNotMatch(source, new RegExp(forbiddenProtocolField));
  }
});

// —— 8.5 driver 时序修复：turn 终态后等门回位（human_confirm/approval）再投递下一脚本动作 ——
// 现场证据 /tmp/aria-stage35-pb/pi/pi/rep1/ws.jsonl：human_gate_turn_completed 后
// driver 立即外发 confirm，而服务端先进修订评审周期（stage_change cross_review →
// prepare_context → …）再回人工门 → confirm 撞 INVALID_MESSAGE_FOR_STAGE →
// protocol_error 终态。以下合成时序用例锁定修复后的策略。

test('stage3 门回位信号只认 human_confirm / waiting_for_human+approval 两种入站形态', () => {
  // 合法门回位信号
  assert.equal(stage3GateWaitingSignal({ type: 'stage_change', stage: 'human_confirm' }), true);
  assert.equal(stage3GateWaitingSignal({ type: 'stage_change', stage: 'waiting_for_human' }), true);
  assert.equal(
    stage3GateWaitingSignal({ type: 'session_state', stage: 'human_confirm', session_status: 'open' }),
    true,
    'session_state 携带 human gate stage 同样是门回位',
  );
  assert.equal(stage3GateWaitingSignal({
    type: 'session_state',
    session_status: 'waiting_for_human',
    single_candidate_phase: 'approval',
  }), true, 'session_status=waiting_for_human 且 phase=approval 是门回位');

  // 修订评审周期与流事件都不是门回位信号
  assert.equal(stage3GateWaitingSignal({ type: 'stage_change', stage: 'cross_review' }), false);
  assert.equal(stage3GateWaitingSignal({ type: 'stage_change', stage: 'prepare_context' }), false);
  assert.equal(stage3GateWaitingSignal({ type: 'stage_change', stage: 'running' }), false);
  assert.equal(
    stage3GateWaitingSignal({ type: 'session_state', session_status: 'waiting_for_human', single_candidate_phase: 'evaluate' }),
    false,
    '评审期 phase（evaluate）不是门回位',
  );
  assert.equal(
    stage3GateWaitingSignal({ type: 'session_state', session_status: 'waiting_for_human' }),
    false,
    '缺 single_candidate_phase 不猜测为门回位',
  );
  assert.equal(stage3GateWaitingSignal({ type: 'stream_chunk', node_id: 'n1', chunk: 'x' }), false);
  assert.equal(stage3GateWaitingSignal({ type: 'review_complete', verdict: 'needs_human' }), false);
  assert.equal(stage3GateWaitingSignal({ type: 'human_gate_turn_completed', turn_id: 'turn-1' }), false);
  assert.equal(stage3GateWaitingSignal(null), false);
});

test('stage3 turn 终态后下一动作必须等门回位才投递，评审周期事件不打断等位', () => {
  const controller = createStage3GateController({
    actions: parseHumanScript('request-change:合成反馈 1 号;confirm'),
    campaignRunId: 'workitem:codex:rep1:issue_gate_timing_0001',
    persistCheckpoint: () => {},
  });
  const queue = createStage3GateHoldQueue();
  const delivered = [];

  // 门 Waiting：首轮 request-change 正常外发（不受等位约束）。
  const first = controller.onGateWaiting();
  assert.equal(first.message.type, 'human_gate_feedback');
  delivered.push(first.message);
  controller.onInbound({ type: 'human_gate_turn_open', command_id: first.commandId, turn_id: 'turn-1', remaining_budget: 1 });

  // turn 完成：控制器契约不变（产出下一动作），但 driver 只登记等位、不外发。
  const terminal = controller.onInbound({
    type: 'human_gate_turn_completed',
    turn_id: 'turn-1',
    artifact_ref: 'artifact://fixture/candidate-v2',
  });
  assert.equal(terminal.outbound.message.type, 'confirm', '控制器契约不变：turn 终态仍产出下一脚本动作');
  const waitEntry = queue.holdFromTurnTerminal('human_gate_turn_completed', terminal.outbound);
  assert.equal(waitEntry.event, 'stage3_gate_return_wait');
  assert.equal(waitEntry.trigger, 'human_gate_turn_completed');
  assert.equal(waitEntry.actionIndex, 1);
  assert.equal(waitEntry.source, 'human_script');

  // 等位期间：修订评审周期事件流（cross_review/prepare_context/running、stream、
  // review_complete、busy）都不是门回位，不得投递、不得打断。
  const reviewCycle = [
    { type: 'stage_change', stage: 'cross_review' },
    { type: 'stage_change', stage: 'prepare_context' },
    { type: 'stream_chunk', node_id: 'n1', chunk: 'x' },
    { type: 'stage_change', stage: 'running' },
    { type: 'review_complete', verdict: 'needs_human', round: 1, findings: [] },
    { type: 'human_gate_busy', turn_id: 'turn-1' },
  ];
  for (const event of reviewCycle) {
    assert.equal(stage3GateWaitingSignal(event), false, `${event.type}/${String(event.stage ?? '')} 不得识别为门回位`);
    assert.equal(queue.offerInbound(event), null, '门未回位期间不得投递待发动作');
  }
  assert.deepEqual(delivered, [first.message], 'confirm 未外发（门未回位）');
  assert.equal(queue.pendingSubmission().message.type, 'confirm');

  // 门回位（stage_change human_confirm）→ 此刻才投递 confirm。
  const dispatch = queue.offerInbound({ type: 'stage_change', stage: 'human_confirm' });
  assert.equal(dispatch.submission.message.type, 'confirm');
  assert.equal(dispatch.via, 'stage_change');
  assert.equal(dispatch.stage, 'human_confirm');
  delivered.push(dispatch.submission.message);
  assert.deepEqual(delivered.map((message) => message.type), ['human_gate_feedback', 'confirm']);

  // 投递后链路零回归：gate closed 正常消费 confirm；无待发动作时门信号是 no-op。
  const closed = controller.onInbound({ type: 'human_gate_closed', decision: 'confirm', stage: 'human_confirm' });
  assert.deepEqual(closed.consumed, [{ actionIndex: 1, kind: 'confirm', via: 'human_gate_closed' }]);
  assert.equal(queue.pendingSubmission(), null);
  assert.equal(queue.offerInbound({ type: 'stage_change', stage: 'human_confirm' }), null, '重复门信号不重复投递');
});

test('stage3 门未回位时 confirm 保持有界等待不报错；INVALID_MESSAGE_FOR_STAGE 按「门未回位」继续等', () => {
  const controller = createStage3GateController({
    actions: parseHumanScript('request-change:合成反馈 1 号;confirm'),
    campaignRunId: 'workitem:codex:rep1:issue_gate_timing_0002',
    persistCheckpoint: () => {},
  });
  const queue = createStage3GateHoldQueue();
  const first = controller.onGateWaiting();
  controller.onInbound({ type: 'human_gate_turn_open', command_id: first.commandId, turn_id: 'turn-1', remaining_budget: 1 });
  const terminal = controller.onInbound({
    type: 'human_gate_turn_completed',
    turn_id: 'turn-1',
    artifact_ref: 'artifact://fixture/candidate-v2',
  });
  queue.holdFromTurnTerminal('human_gate_turn_completed', terminal.outbound);

  // 门迟迟未回位：等位是稳定状态——不抛错、不外发（有界等待由驱动硬超时兜底收口）。
  assert.doesNotThrow(() => {
    for (let index = 0; index < 5; index += 1) {
      assert.equal(queue.offerInbound({ type: 'stage_change', stage: 'prepare_context' }), null);
      assert.equal(queue.offerInbound({ type: 'stage_change', stage: 'cross_review' }), null);
    }
  });
  assert.equal(queue.pendingSubmission().message.type, 'confirm');

  // 阶段错误不再立即终态：INVALID_MESSAGE_FOR_STAGE 按「门未回位」消化，等门回位重发。
  const stageError = 'message confirm not allowed in stage prepare_context';
  assert.equal(controller.noteGateCloseConflict(stageError, 'INVALID_MESSAGE_FOR_STAGE'), true, '阶段错误按门未回位消化');
  assert.equal(
    controller.noteGateCloseConflict(stageError, 'INVALID_MESSAGE_FOR_STAGE'),
    false,
    '重复通报不再消化',
  );
  assert.equal(controller.noteGateCloseConflict('其他错误', null), false, '非阶段/CAS 冲突错误不消化');
  assert.ok(
    controller.resultFields().durable_recovery_checks.some((entry) => entry.check === 'confirm_invalid_stage_awaiting_gate'),
    '阶段错误消化必须落 durable_recovery_checks 审计',
  );

  // 消化后：held 副本作废（避免与重发路径双发），门回位信号本身不携带待发动作；
  // 由 onGateWaiting 在门回位时机重发同一 confirm（脚本指针不推进）。
  assert.deepEqual(queue.dropPending('INVALID_MESSAGE_FOR_STAGE'), {
    trigger: 'INVALID_MESSAGE_FOR_STAGE',
    actionIndex: 1,
  });
  assert.equal(queue.pendingSubmission(), null);
  assert.equal(queue.offerInbound({ type: 'stage_change', stage: 'human_confirm' }), null);
  const resend = controller.onGateWaiting();
  assert.equal(resend.message.type, 'confirm');
  assert.equal(resend.actionIndex, 1, '脚本指针仍停在 confirm，重发同一动作');
  const closed = controller.onInbound({ type: 'human_gate_closed', decision: 'confirm', stage: 'human_confirm' });
  assert.deepEqual(closed.consumed, [{ actionIndex: 1, kind: 'confirm', via: 'human_gate_closed' }]);

  // 仅凭错误文本（无 code）也要能识别同一形态；human_gate_close 旧形态保持原语义。
  const textOnly = createStage3GateController({
    actions: parseHumanScript('request-change:合成反馈 1 号;confirm'),
    campaignRunId: 'workitem:codex:rep1:issue_gate_timing_0003',
    persistCheckpoint: () => {},
  });
  const textFirst = textOnly.onGateWaiting();
  textOnly.onInbound({ type: 'human_gate_turn_open', command_id: textFirst.commandId, turn_id: 'turn-1', remaining_budget: 1 });
  const textTerminal = textOnly.onInbound({ type: 'human_gate_turn_completed', turn_id: 'turn-1', artifact_ref: 'a' });
  assert.ok(textTerminal.outbound, 'turn 终态产出待发 confirm');
  assert.equal(
    textOnly.noteGateCloseConflict('INVALID_MESSAGE_FOR_STAGE: message confirm not allowed in stage prepare_context'),
    true,
    '错误文本含阶段错误形态即可消化',
  );
  assert.equal(textOnly.noteGateCloseConflict('product_store_conflict: human_gate_close ws_0001'), false, '消化一次后不再消化');
});
