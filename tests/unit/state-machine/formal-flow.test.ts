import { describe, expect, it } from 'vitest';

import type { State } from '../../../src/schemas/state-schema.js';
import { canTransition } from '../../../src/runtime/state-machine/state-machine.js';

function createState(overrides: Partial<State> = {}): State {
  return {
    task_id: 'aria-20260418-001',
    task_title: '为 Aria 增加 capability report 结构化输出',
    source: 'aria-native',
    flow_type: 'formal',
    risk_level: 'medium',
    status: 'intake',
    current_round: 1,
    approved_spec_ref: null,
    approved_plan_ref: null,
    active_result_set_id: null,
    active_exec_units: [],
    confirmation_pending: 'none',
    confirmation_mode: 'manual',
    confirmation_artifact_path: null,
    confirmation_event_path: null,
    dispatch_contract_ref: null,
    context_bundle_ref: null,
    review_report_ref: null,
    test_report_ref: null,
    review_status: 'pending',
    test_status: 'pending',
    patch_required_by: 'none',
    patch_round: 0,
    exec_units: {},
    created_at: '2026-04-18T00:00:00.000Z',
    updated_at: '2026-04-18T00:00:00.000Z',
    ...overrides
  };
}

describe('canTransition', () => {
  it('plan-review 进入 plan-approved 时缺少确认工件会失败', () => {
    expect(
      canTransition(
        createState({
          status: 'plan-review',
          approved_spec_ref: 'artifacts/spec.md',
          approved_plan_ref: 'artifacts/plan.md',
          confirmation_event_path: null
        }),
        'plan-approved'
      )
    ).toEqual({
      allowed: false,
      reason: 'missing_confirmation_event'
    });
  });

  it('plan-review 进入 plan-approved 时具备确认工件会通过', () => {
    expect(
      canTransition(
        createState({
          status: 'plan-review',
          approved_spec_ref: 'artifacts/spec.md',
          approved_plan_ref: 'artifacts/plan.md',
          confirmation_event_path: 'cadence/cache/aria/tasks/aria-20260418-001/confirmation-event.yaml'
        }),
        'plan-approved'
      )
    ).toEqual({
      allowed: true
    });
  });

  it('spec-review 进入 plan-review 时具备待确认 spec 工件会通过', () => {
    expect(
      canTransition(
        createState({
          status: 'spec-review',
          approved_spec_ref: null,
          approved_plan_ref: null,
          confirmation_artifact_path: 'cadence/cache/aria/tasks/aria-20260418-001/artifacts/spec-artifact.md'
        }),
        'plan-review'
      )
    ).toEqual({
      allowed: true
    });
  });

  it('plan-approved 进入 dispatched 时缺少 frozen refs 会失败', () => {
    expect(
      canTransition(
        createState({
          status: 'plan-approved',
          approved_spec_ref: null,
          approved_plan_ref: 'artifacts/plan.md',
          confirmation_event_path: 'cadence/cache/aria/tasks/aria-20260418-001/confirmation-event.yaml',
          dispatch_contract_ref: 'cadence/cache/aria/tasks/aria-20260418-001/dispatch-contract.yaml',
          context_bundle_ref: 'cadence/cache/aria/tasks/aria-20260418-001/context-bundle.yaml'
        }),
        'dispatched'
      )
    ).toEqual({
      allowed: false,
      reason: 'missing_frozen_refs'
    });
  });

  it('plan-approved 进入 dispatched 时缺少 handoff 工件会失败', () => {
    expect(
      canTransition(
        createState({
          status: 'plan-approved',
          approved_spec_ref: 'artifacts/spec.md',
          approved_plan_ref: 'artifacts/plan.md',
          confirmation_event_path: 'cadence/cache/aria/tasks/aria-20260418-001/confirmation-event.yaml'
        }),
        'dispatched'
      )
    ).toEqual({
      allowed: false,
      reason: 'handoff_incomplete'
    });
  });

  it('plan-review 进入 dispatched 时缺少 handoff 关键字段会失败', () => {
    expect(
      canTransition(
        createState({
          status: 'plan-review',
          approved_spec_ref: 'artifacts/spec.md',
          approved_plan_ref: 'artifacts/plan.md',
          confirmation_event_path: 'cadence/cache/aria/tasks/aria-20260418-001/confirmation-event.yaml',
          confirmation_artifact_path: 'cadence/cache/aria/tasks/aria-20260418-001/artifacts/plan-brief.md'
        }),
        'dispatched'
      )
    ).toEqual({
      allowed: false,
      reason: 'handoff_incomplete'
    });
  });

  it('plan-approved 进入 dispatched 时具备 frozen refs 和 handoff 工件会通过', () => {
    expect(
      canTransition(
        createState({
          status: 'plan-approved',
          approved_spec_ref: 'artifacts/spec.md',
          approved_plan_ref: 'artifacts/plan.md',
          confirmation_event_path: 'cadence/cache/aria/tasks/aria-20260418-001/confirmation-event.yaml',
          dispatch_contract_ref: 'cadence/cache/aria/tasks/aria-20260418-001/dispatch-contract.yaml',
          context_bundle_ref: 'cadence/cache/aria/tasks/aria-20260418-001/context-bundle.yaml'
        }),
        'dispatched'
      )
    ).toEqual({
      allowed: true
    });
  });

  it('plan-review 进入 dispatched 时具备 frozen refs 和 handoff 工件会通过', () => {
    expect(
      canTransition(
        createState({
          status: 'plan-review',
          approved_spec_ref: 'artifacts/spec.md',
          approved_plan_ref: 'artifacts/plan.md',
          confirmation_event_path: 'cadence/cache/aria/tasks/aria-20260418-001/confirmation-event.yaml',
          confirmation_artifact_path: 'cadence/cache/aria/tasks/aria-20260418-001/artifacts/plan-brief.md',
          dispatch_contract_ref: 'cadence/cache/aria/tasks/aria-20260418-001/dispatch-contract.yaml',
          context_bundle_ref: 'cadence/cache/aria/tasks/aria-20260418-001/context-bundle.yaml'
        }),
        'dispatched'
      )
    ).toEqual({
      allowed: true
    });
  });

  it('明显非法跳转返回 invalid_transition', () => {
    expect(canTransition(createState({ status: 'intake' }), 'done')).toEqual({
      allowed: false,
      reason: 'invalid_transition'
    });
  });

  it('reviewing/testing 进入 verified 时缺少 review/test 报告会失败', () => {
    expect(
      canTransition(
        createState({
          status: 'reviewing/testing',
          review_status: 'passed',
          test_status: 'passed',
          review_report_ref: null,
          test_report_ref: null
        }),
        'verified'
      )
    ).toEqual({
      allowed: false,
      reason: 'missing_review_test_reports'
    });
  });

  it('reviewing/testing 进入 verified 时具备报告且结论通过会成功', () => {
    expect(
      canTransition(
        createState({
          status: 'reviewing/testing',
          review_status: 'passed',
          test_status: 'passed',
          review_report_ref: 'cadence/cache/aria/tasks/aria-20260418-001/artifacts/review-report.yaml',
          test_report_ref: 'cadence/cache/aria/tasks/aria-20260418-001/artifacts/test-report.yaml'
        }),
        'verified'
      )
    ).toEqual({
      allowed: true
    });
  });

  it('reviewing/testing 进入 verified 时任一结论失败会被阻止', () => {
    expect(
      canTransition(
        createState({
          status: 'reviewing/testing',
          review_status: 'failed',
          test_status: 'passed',
          review_report_ref: 'cadence/cache/aria/tasks/aria-20260418-001/artifacts/review-report.yaml',
          test_report_ref: 'cadence/cache/aria/tasks/aria-20260418-001/artifacts/test-report.yaml'
        }),
        'verified'
      )
    ).toEqual({
      allowed: false,
      reason: 'review_or_test_not_passed'
    });
  });
});
