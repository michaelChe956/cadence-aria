import assert from 'node:assert/strict';
import test from 'node:test';

import {
  authorConfirmAction,
  collectUsageByRole,
  generationModeSelectionForNode,
  hasMustFixFindings,
  humanConfirmAction,
  humanConfirmRequestChangeMessage,
  providerRolesForSelection,
  reviewCompleteAction,
  reviewDecisionAction,
  reviewRepairAction,
  reviewCycleId,
  prepareOptionsForProvider,
  sessionStateProtocol,
  skipOptionalFindingsOption,
  terminalSessionAction,
} from './workitem_run_campaign.mjs';
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
    terminalSessionAction(sessionState({ session_status: 'waiting_for_human' }), false, true),
    { kind: 'terminal', failureClass: 'awaiting_human' },
    'a completed stage must finish after durable readback records a human gate',
  );
  assert.deepEqual(
    terminalSessionAction(sessionState({ session_status: 'waiting_for_human' }), true),
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
