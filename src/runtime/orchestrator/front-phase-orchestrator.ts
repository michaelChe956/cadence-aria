import fs from 'node:fs/promises';
import path from 'node:path';

import { buildOpenSpecEvidence } from '../../adapters/openspec/openspec-adapter.js';
import { buildSuperpowersEvidence } from '../../adapters/superpowers/superpowers-adapter.js';
import { buildIntakeCard } from '../reports/intake-card.js';
import { buildPlanBrief } from '../reports/plan-brief.js';
import { buildSpecArtifact } from '../reports/spec-artifact.js';
import { createTask } from '../persistence/task-repository.js';
import { getTaskArtifactsDir } from '../persistence/paths.js';
import { readState, writeState } from '../persistence/state-repository.js';
import { nowIso } from '../../utils/time.js';

async function readLegacyTaskTitle(taskId: string): Promise<string> {
  const intakeCardPath = path.join(getTaskArtifactsDir(taskId), 'task-intake-card.md');
  const content = await fs.readFile(intakeCardPath, 'utf8');
  const match = content.match(/^scope_summary: (.+)$/m);

  if (!match) {
    throw new Error(`无法从 intake card 读取标题: ${taskId}`);
  }

  return match[1];
}

export async function intakeFrontPhaseTask(title: string): Promise<string> {
  const state = await createTask({ title });
  const artifactsDir = getTaskArtifactsDir(state.task_id);
  const intakeCardPath = path.join(artifactsDir, 'task-intake-card.md');

  await fs.writeFile(intakeCardPath, buildIntakeCard({ task_id: state.task_id, title }), 'utf8');

  return state.task_id;
}

export async function startFrontPhaseTask(taskId: string): Promise<void> {
  const state = await readState(taskId);
  if (state.status !== 'intake') {
    throw new Error(`任务不在可启动状态: ${taskId}`);
  }

  const artifactsDir = getTaskArtifactsDir(taskId);
  const specPath = path.join(artifactsDir, 'spec-artifact.md');
  const planPath = path.join(artifactsDir, 'plan-brief.md');
  const taskTitle = state.task_title ?? await readLegacyTaskTitle(taskId);

  const openSpecEvidence = buildOpenSpecEvidence({ specRef: specPath, planRef: planPath });
  const specSuperpowersEvidence = buildSuperpowersEvidence({ stage: 'spec' });
  const planSuperpowersEvidence = buildSuperpowersEvidence({ stage: 'plan' });

  await fs.mkdir(artifactsDir, { recursive: true });
  await fs.writeFile(
    specPath,
    buildSpecArtifact({
      title: taskTitle,
      openSpecEvidence,
      superpowersEvidence: specSuperpowersEvidence
    }),
    'utf8'
  );
  await fs.writeFile(
    planPath,
    buildPlanBrief({
      taskId,
      openSpecEvidence,
      superpowersEvidence: planSuperpowersEvidence
    }),
    'utf8'
  );

  await writeState({
    ...state,
    task_title: taskTitle,
    status: 'spec-review',
    confirmation_pending: 'spec',
    confirmation_artifact_path: specPath,
    updated_at: nowIso()
  });
}
