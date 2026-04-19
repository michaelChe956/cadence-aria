import fs from 'node:fs/promises';

import type { State } from '../schemas/state-schema.js';
import { runSingleExecUnit } from '../runtime/scheduler/exec-scheduler.js';
import { arbitrateReviewAndTest } from '../runtime/arbitrator/review-test-arbitrator.js';
import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { runReviewAndTest } from '../runtime/orchestrator/review-test-orchestrator.js';
import { parseYaml } from '../utils/yaml.js';
import { nowIso } from '../utils/time.js';
import { canTransition } from '../runtime/state-machine/state-machine.js';

export async function runCommand(taskId: string): Promise<string> {
  await runSingleExecUnit(taskId);
  const { reviewReportPath, testReportPath } = await runReviewAndTest(taskId);
  const state = await readState(taskId);
  const review = parseYaml(await fs.readFile(reviewReportPath, 'utf8')) as {
    verdict: 'passed' | 'failed';
    result_set_id: string;
  };
  const test = parseYaml(await fs.readFile(testReportPath, 'utf8')) as {
    verdict: 'passed' | 'failed';
    result_set_id: string;
  };
  const arbitration = arbitrateReviewAndTest({ review, test });
  const patchRequiredBy: State['patch_required_by'] =
    review.verdict === 'passed' && test.verdict === 'passed'
      ? 'none'
      : review.verdict !== 'passed' && test.verdict !== 'passed'
        ? 'both'
        : review.verdict !== 'passed'
          ? 'review'
          : 'test';

  const nextState = {
    ...state,
    status: arbitration.next_status,
    review_status: review.verdict,
    test_status: test.verdict,
    review_report_ref: reviewReportPath,
    test_report_ref: testReportPath,
    patch_required_by: patchRequiredBy,
    block_reason_code: arbitration.next_status === 'blocked' ? arbitration.reason : null,
    blocking_stage: arbitration.next_status === 'blocked' ? 'reviewing/testing' : null,
    retryable: arbitration.next_status === 'blocked' ? false : undefined,
    required_action: arbitration.next_status === 'blocked' ? 'resolve-review-test-mismatch' : null,
    updated_at: nowIso()
  };

  if (arbitration.next_status === 'verified') {
    const transition = canTransition({
      ...nextState,
      status: 'reviewing/testing'
    }, 'verified');
    if (!transition.allowed) {
      throw new Error(`无法推进到 verified: ${transition.reason}`);
    }
  }

  await writeState(nextState);

  return [
    '[Aria]',
    `- status: ${nextState.status}`,
    `- review_status: ${nextState.review_status}`,
    `- test_status: ${nextState.test_status}`
  ].join('\n');
}
