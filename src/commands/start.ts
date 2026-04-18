import { startFormalTask } from '../runtime/orchestrator/task-orchestrator.js';

export async function startCommand(taskId: string): Promise<string> {
  await startFormalTask(taskId);

  return ['[Aria]', '- status: spec-review', '- clarification_required: false', '- next: confirm-spec'].join('\n');
}
