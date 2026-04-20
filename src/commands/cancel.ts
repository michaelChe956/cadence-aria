import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { canTransition } from '../runtime/state-machine/state-machine.js';
import { nowIso } from '../utils/time.js';

export async function cancelCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);

  const transition = canTransition(state, 'cancelled');
  if (!transition.allowed) {
    throw new Error(`任务 ${taskId} 当前状态 ${state.status} 不允许取消: ${transition.reason}`);
  }

  await writeState({
    ...state,
    status: 'cancelled',
    active_exec_units: [],
    updated_at: nowIso()
  });

  return [`[Aria]`, `- status: cancelled`, `- task_id: ${taskId}`].join('\n');
}
