import fs from 'node:fs/promises';

import type { State } from '../schemas/state-schema.js';
import { reviewReportSchema, testReportSchema } from '../schemas/runtime-artifact-schema.js';
import { runSingleExecUnit } from '../runtime/scheduler/exec-scheduler.js';
import { arbitrateReviewAndTest } from '../runtime/arbitrator/review-test-arbitrator.js';
import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { runReviewAndTest } from '../runtime/orchestrator/review-test-orchestrator.js';
import { parseYaml } from '../utils/yaml.js';
import { nowIso } from '../utils/time.js';
import { canTransition } from '../runtime/state-machine/state-machine.js';
import { resolveRetryableBlock } from '../runtime/state-machine/recovery-rules.js';

function resolveReviewTestFailureReason(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);

  if (message.startsWith('review_report_task_mismatch:')) {
    return 'review_report_task_mismatch';
  }

  if (message.startsWith('review_report_result_set_mismatch:')) {
    return 'review_report_result_set_mismatch';
  }

  if (message.startsWith('review_report_invalid:')) {
    if (message.includes('review_report_task_mismatch:')) {
      return 'review_report_task_mismatch';
    }
    if (message.includes('review_report_result_set_mismatch:')) {
      return 'review_report_result_set_mismatch';
    }
    return 'review_report_invalid';
  }

  if (message.startsWith('test_report_task_mismatch:')) {
    return 'test_report_task_mismatch';
  }

  if (message.startsWith('test_report_result_set_mismatch:')) {
    return 'test_report_result_set_mismatch';
  }

  if (message.startsWith('test_report_invalid:')) {
    if (message.includes('test_report_task_mismatch:')) {
      return 'test_report_task_mismatch';
    }
    if (message.includes('test_report_result_set_mismatch:')) {
      return 'test_report_result_set_mismatch';
    }
    return 'test_report_invalid';
  }

  if (message.startsWith('claude_review_failed:')) {
    return 'claude_review_failed';
  }

  if (message.startsWith('claude_test_failed:')) {
    return 'claude_test_failed';
  }

  return 'review_test_failed';
}

function resolveExecutionFailureReason(error: unknown): string {
  const message = error instanceof Error ? error.message : String(error);

  if (message === 'capability_blocked') {
    return 'capability_blocked';
  }

  if (message.startsWith('codex_exec_failed:')) {
    return 'execution_blocked';
  }

  if (message.includes('冻结引用') || message.includes('execution context bundle') || message.includes('dispatch contract')) {
    return 'execution_prerequisites_invalid';
  }

  return 'execution_blocked';
}

function resolvePatchRequiredBy(reviewVerdict: string, testVerdict: string): State['patch_required_by'] {
  if (reviewVerdict === 'passed' && testVerdict === 'passed') return 'none';
  if (reviewVerdict !== 'passed' && testVerdict !== 'passed') return 'both';
  if (reviewVerdict !== 'passed') return 'review';
  return 'test';
}

export async function runCommand(taskId: string): Promise<string> {
  try {
    await runSingleExecUnit(taskId);
    const { reviewReportPath, testReportPath } = await runReviewAndTest(taskId);
    const state = await readState(taskId);
    const review = reviewReportSchema.parse(parseYaml(await fs.readFile(reviewReportPath, 'utf8')));
    const test = testReportSchema.parse(parseYaml(await fs.readFile(testReportPath, 'utf8')));
    const arbitration = arbitrateReviewAndTest({ review, test });
    const arbitrationResolution =
      arbitration.next_status === 'blocked' && arbitration.reason
        ? resolveRetryableBlock(arbitration.reason)
        : null;
    const patchRequiredBy = resolvePatchRequiredBy(review.verdict, test.verdict);

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
      retryable: arbitration.next_status === 'blocked' ? arbitrationResolution?.retryable : undefined,
      required_action: arbitration.next_status === 'blocked' ? arbitrationResolution?.required_action ?? null : null,
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
    } else if (arbitration.next_status === 'blocked') {
      // blocked 是合法的安全降级路径，不通过 Guard 检查
      // 但确认当前状态是 reviewing/testing
      if (state.status !== 'reviewing/testing') {
        throw new Error(`无法从 ${state.status} 进入 blocked`);
      }
    }

    await writeState(nextState);

    return [
      '[Aria]',
      `- status: ${nextState.status}`,
      `- review_status: ${nextState.review_status}`,
      `- test_status: ${nextState.test_status}`
    ].join('\n');
  } catch (error) {
    const latestState = await readState(taskId);
    if (latestState.status === 'dispatched' || latestState.status === 'executing') {
      const reasonCode = resolveExecutionFailureReason(error);
      const retryResolution = resolveRetryableBlock(reasonCode);
      await writeState({
        ...latestState,
        status: 'blocked',
        active_exec_units: [],
        block_reason_code: reasonCode,
        blocking_stage: 'executing',
        retryable: retryResolution.retryable,
        required_action: retryResolution.required_action,
        updated_at: nowIso()
      });
    } else if (latestState.status === 'reviewing/testing') {
      const reasonCode = resolveReviewTestFailureReason(error);
      const retryResolution = resolveRetryableBlock(reasonCode);
      await writeState({
        ...latestState,
        status: 'blocked',
        active_exec_units: [],
        block_reason_code: reasonCode,
        blocking_stage: 'reviewing/testing',
        retryable: retryResolution.retryable,
        required_action: retryResolution.required_action,
        updated_at: nowIso()
      });
    }

    throw error;
  }
}
