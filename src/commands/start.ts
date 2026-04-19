import { startFrontPhaseTask } from '../runtime/orchestrator/front-phase-orchestrator.js';

export async function startCommand(taskId: string): Promise<string> {
  await startFrontPhaseTask(taskId);

  return ['[Aria]', '- status: spec-review', '- clarification_required: false', '- next: confirm-spec'].join('\n');
}
