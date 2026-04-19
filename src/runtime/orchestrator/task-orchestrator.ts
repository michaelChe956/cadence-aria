import { intakeFrontPhaseTask, startFrontPhaseTask } from './front-phase-orchestrator.js';

export async function intakeFormalTask(title: string): Promise<string> {
  return intakeFrontPhaseTask(title);
}

export async function startFormalTask(taskId: string): Promise<void> {
  await startFrontPhaseTask(taskId);
}
