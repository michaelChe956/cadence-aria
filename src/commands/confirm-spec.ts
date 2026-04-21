import fs from 'node:fs/promises';
import path from 'node:path';

import { canTransition } from '../runtime/state-machine/state-machine.js';
import { appendConfirmationEvent } from '../runtime/persistence/confirmation-event-repository.js';
import { getTaskArtifactsDir } from '../runtime/persistence/paths.js';
import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { nowIso } from '../utils/time.js';

import { validateFrontPhaseArtifact } from './shared/front-phase-validation.js';

export async function confirmSpecCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);
  if (state.status !== 'spec-review') {
    throw new Error(`任务不在可确认 spec 的状态: ${taskId}`);
  }
  if (!state.confirmation_artifact_path) {
    throw new Error(`缺少待确认 spec 工件: ${taskId}`);
  }

  const specContent = await fs.readFile(state.confirmation_artifact_path, 'utf8');
  validateFrontPhaseArtifact({
    content: specContent,
    artifactType: 'spec',
    expectedSpecRef: state.confirmation_artifact_path,
    expectedPlanRef: path.posix.join(getTaskArtifactsDir(taskId), 'plan-brief.md')
  });

  const transition = canTransition(state, 'plan-review');
  if (!transition.allowed) {
    throw new Error(`无法推进到 plan-review: ${transition.reason}`);
  }

  const confirmation_event_path = await appendConfirmationEvent(taskId, {
    task_id: taskId,
    confirmation_type: 'spec',
    artifact_ref: state.confirmation_artifact_path,
    decision: 'approved',
    actor: 'user',
    timestamp: nowIso(),
    note: 'spec approved'
  });

  await writeState({
    ...state,
    approved_spec_ref: state.confirmation_artifact_path,
    status: 'plan-review',
    confirmation_pending: 'plan',
    confirmation_artifact_path: path.posix.join(getTaskArtifactsDir(taskId), 'plan-brief.md'),
    confirmation_event_path,
    updated_at: nowIso()
  });

  return ['[Aria]', '- status: plan-review'].join('\n');
}
