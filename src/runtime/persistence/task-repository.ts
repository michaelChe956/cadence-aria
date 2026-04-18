import fs from 'node:fs/promises';
import path from 'node:path';

import { getTaskArtifactsDir, getTaskRoot } from './paths.js';
import { writeState } from './state-repository.js';
import { createTaskId } from '../../utils/id.js';
import { nowIso } from '../../utils/time.js';
import type { State } from '../../schemas/state-schema.js';

export async function createTask(input: { title: string }): Promise<State> {
  const now = new Date();
  let task_id = await createTaskId(now);
  let taskRoot = getTaskRoot(task_id);

  await fs.mkdir(path.dirname(taskRoot), { recursive: true });

  while (true) {
    try {
      await fs.mkdir(taskRoot);
      break;
    } catch (error) {
      const code = (error as NodeJS.ErrnoException).code;
      if (code !== 'EEXIST') {
        throw error;
      }

      const suffix = Number(task_id.slice(-3));
      task_id = await createTaskId(now, suffix + 1);
      taskRoot = getTaskRoot(task_id);
    }
  }

  const artifactsDir = getTaskArtifactsDir(task_id);

  await fs.mkdir(artifactsDir, { recursive: true });

  const timestamp = nowIso();
  const state: State = {
    task_id,
    source: 'aria-native',
    flow_type: 'formal',
    risk_level: 'medium',
    status: 'intake',
    current_round: 1,
    approved_spec_ref: null,
    approved_plan_ref: null,
    active_result_set_id: null,
    active_exec_units: [],
    confirmation_pending: 'none',
    confirmation_mode: 'manual',
    confirmation_artifact_path: null,
    confirmation_event_path: null,
    dispatch_contract_ref: null,
    context_bundle_ref: null,
    review_status: 'pending',
    test_status: 'pending',
    patch_required_by: 'none',
    patch_round: 0,
    exec_units: {},
    created_at: timestamp,
    updated_at: timestamp,
  };

  await writeState(state);
  await fs.writeFile(path.join(artifactsDir, 'task-intake-card.md'), `# ${input.title}\n`, 'utf8');

  return state;
}
