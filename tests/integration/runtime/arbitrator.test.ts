import { describe, expect, it } from 'vitest';

import { arbitrateReviewAndTest } from '../../../src/runtime/arbitrator/review-test-arbitrator.js';

describe('arbitrateReviewAndTest', () => {
  it('当 review/test 均通过时进入 verified', () => {
    const result = arbitrateReviewAndTest({
      review: { verdict: 'passed', result_set_id: 'result-set-1' },
      test: { verdict: 'passed', result_set_id: 'result-set-1' }
    });

    expect(result.next_status).toBe('verified');
  });

  it('当 review/test 结果集不一致时进入 blocked', () => {
    const result = arbitrateReviewAndTest({
      review: { verdict: 'passed', result_set_id: 'result-set-1' },
      test: { verdict: 'passed', result_set_id: 'result-set-2' }
    });

    expect(result).toEqual({
      next_status: 'blocked',
      reason: 'result_set_mismatch'
    });
  });

  it('当任一结论未通过时进入 blocked，避免进入不可恢复的 patching', () => {
    const result = arbitrateReviewAndTest({
      review: { verdict: 'failed', result_set_id: 'result-set-1' },
      test: { verdict: 'passed', result_set_id: 'result-set-1' }
    });

    expect(result).toEqual({
      next_status: 'blocked',
      reason: 'must_fix_detected'
    });
  });
});
