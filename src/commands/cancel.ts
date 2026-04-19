import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { nowIso } from '../utils/time.js';

export async function cancelCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);

  await writeState({
    ...state,
    status: 'cancelled',
    active_exec_units: [],
    updated_at: nowIso()
  });

  return [`[Aria]`, `- status: cancelled`, `- task_id: ${taskId}`].join('\n');
}
