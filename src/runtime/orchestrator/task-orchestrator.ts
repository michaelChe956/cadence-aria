import fs from 'node:fs/promises';
import path from 'node:path';

import { createTask } from '../persistence/task-repository.js';
import { getTaskArtifactsDir } from '../persistence/paths.js';
import { readState, writeState } from '../persistence/state-repository.js';
import { buildIntakeCard } from '../reports/intake-card.js';
import { buildPlanBrief } from '../reports/plan-brief.js';
import { buildSpecArtifact } from '../reports/spec-artifact.js';
import { nowIso } from '../../utils/time.js';

async function readTaskTitle(taskId: string): Promise<string> {
  const intakeCardPath = path.join(getTaskArtifactsDir(taskId), 'task-intake-card.md');
  const content = await fs.readFile(intakeCardPath, 'utf8');
  const match = content.match(/^scope_summary: (.+)$/m);

  if (!match) {
    throw new Error(`无法从 intake card 读取标题: ${taskId}`);
  }

  return match[1];
}

export async function intakeFormalTask(title: string): Promise<string> {
  const state = await createTask({ title });
  const artifactsDir = getTaskArtifactsDir(state.task_id);
  const intakeCardPath = path.join(artifactsDir, 'task-intake-card.md');

  await fs.writeFile(intakeCardPath, buildIntakeCard({ task_id: state.task_id, title }), 'utf8');

  return state.task_id;
}

export async function startFormalTask(taskId: string): Promise<void> {
  const state = await readState(taskId);
  const artifactsDir = getTaskArtifactsDir(taskId);
  const specPath = path.join(artifactsDir, 'spec-artifact.md');
  const planPath = path.join(artifactsDir, 'plan-brief.md');
  const title = await readTaskTitle(taskId);

  await fs.writeFile(specPath, buildSpecArtifact(title), 'utf8');
  await fs.writeFile(planPath, buildPlanBrief(taskId), 'utf8');

  await writeState({
    ...state,
    status: 'spec-review',
    confirmation_pending: 'spec',
    confirmation_artifact_path: specPath,
    updated_at: nowIso(),
  });
}
