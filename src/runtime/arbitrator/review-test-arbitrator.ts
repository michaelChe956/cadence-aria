export function arbitrateReviewAndTest(input: {
  review: { verdict: string; result_set_id: string };
  test: { verdict: string; result_set_id: string };
}) {
  if (input.review.result_set_id !== input.test.result_set_id) {
    return { next_status: 'blocked' as const, reason: 'result_set_mismatch' as const };
  }

  if (input.review.verdict === 'passed' && input.test.verdict === 'passed') {
    return { next_status: 'verified' as const };
  }

  return { next_status: 'blocked' as const, reason: 'must_fix_detected' as const };
}
