import { describe, expect, it } from 'vitest';
import { canTransition } from '../../../src/runtime/state-machine/state-machine.js';

describe('canTransition', () => {
  it('无 approved_plan_ref 时不能从 plan-review 进入 plan-approved', () => {
    expect(
      canTransition(
        {
          status: 'plan-review',
          approved_plan_ref: null,
          approved_spec_ref: 'artifacts/spec.md'
        },
        'plan-approved'
      )
    ).toEqual({
      allowed: false,
      reason: 'missing_confirmation_event'
    });
  });
});
