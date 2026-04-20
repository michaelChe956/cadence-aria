import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { nowIso } from '../utils/time.js';

export async function retryCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);
  if (!state.retryable) {
    throw new Error(`任务不可重试: ${taskId}`);
  }

  const execUnit = state.exec_units['exec-01'];
  if (!execUnit) {
    throw new Error(`缺少可重试执行单元: ${taskId}`);
  }

  await writeState({
    ...state,
    status: 'dispatched',
    active_exec_units: ['exec-01'],
    block_reason_code: null,
    blocking_stage: null,
    retryable: false,
    required_action: null,
    exec_units: {
      ...state.exec_units,
      'exec-01': {
        ...execUnit,
        status: 'pending',
        exit_code: null,
        started_at: undefined,
        finished_at: undefined
      }
    },
    updated_at: nowIso()
  });

  return [`[Aria]`, `- status: dispatched`, `- task_id: ${taskId}`].join('\n');
}
