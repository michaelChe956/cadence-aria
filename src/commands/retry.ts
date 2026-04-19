import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { nowIso } from '../utils/time.js';

export async function retryCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);
  if (!state.retryable) {
    throw new Error(`任务不可重试: ${taskId}`);
  }

  await writeState({
    ...state,
    status: 'executing',
    block_reason_code: null,
    blocking_stage: null,
    required_action: null,
    updated_at: nowIso()
  });

  return [`[Aria]`, `- status: executing`, `- task_id: ${taskId}`].join('\n');
}
