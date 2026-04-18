import { intakeFormalTask } from '../runtime/orchestrator/task-orchestrator.js';

export async function intakeCommand(title: string): Promise<string> {
  const taskId = await intakeFormalTask(title);

  return [
    '[Aria]',
    `- task_id: ${taskId}`,
    '- source: aria-native',
    '- flow_type_suggestion: formal',
    '- risk_level: medium',
    `- next: aria:start --task-id ${taskId}`,
  ].join('\n');
}
