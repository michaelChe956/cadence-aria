import { readState } from '../runtime/persistence/state-repository.js';

export async function resultCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);

  return [`[Aria]`, `- final_status: ${state.status}`, `- task_id: ${taskId}`].join('\n');
}
