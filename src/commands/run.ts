import { runSingleExecUnit } from '../runtime/scheduler/exec-scheduler.js';

export async function runCommand(taskId: string): Promise<string> {
  await runSingleExecUnit(taskId);

  return [
    '[Aria]',
    '- status: reviewing/testing',
    '- review_status: pending',
    '- test_status: pending'
  ].join('\n');
}
