import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { canTransition } from '../runtime/state-machine/state-machine.js';
import { nowIso } from '../utils/time.js';

export async function retryCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);
  if (state.status !== 'blocked') {
    throw new Error(`任务不在可重试状态: ${taskId} (当前: ${state.status})`);
  }

  const execUnit = state.exec_units['exec-01'];
  if (!execUnit) {
    throw new Error(`缺少可重试执行单元: ${taskId}`);
  }

  if (execUnit.attempt >= 3) {
    throw new Error(`任务已达到最大重试次数 (3): ${taskId}`);
  }

  const nextState = {
    ...state,
    status: 'dispatched' as const,
    active_exec_units: ['exec-01'],
    block_reason_code: null as string | null,
    blocking_stage: null as string | null,
    retryable: false,
    required_action: null as string | null,
    exec_units: {
      ...state.exec_units,
      'exec-01': {
        ...execUnit,
        status: 'pending' as const,
        exit_code: null as number | null,
        started_at: undefined as string | undefined,
        finished_at: undefined as string | undefined
      }
    },
    updated_at: nowIso()
  };

  const transition = canTransition(state, 'dispatched');
  if (!transition.allowed) {
    throw new Error(`无法推进到 dispatched: ${transition.reason}`);
  }

  await writeState(nextState);

  return [`[Aria]`, `- status: dispatched`, `- task_id: ${taskId}`].join('\n');
}
