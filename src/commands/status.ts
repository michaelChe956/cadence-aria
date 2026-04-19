import { readState } from '../runtime/persistence/state-repository.js';

export async function statusCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);

  return [`[Aria]`, `- task_id: ${taskId}`, `- status: ${state.status}`].join('\n');
}
