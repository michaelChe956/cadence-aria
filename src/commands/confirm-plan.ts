import path from 'node:path';

import { appendConfirmationEvent } from '../runtime/persistence/confirmation-event-repository.js';
import { getTaskArtifactsDir } from '../runtime/persistence/paths.js';
import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { nowIso } from '../utils/time.js';

import { createDispatchArtifacts } from '../runtime/contracts/dispatch-contract.js';

export async function confirmPlanCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);
  if (state.status !== 'plan-review') {
    throw new Error(`任务不在可确认 plan 的状态: ${taskId}`);
  }

  const approved_spec_ref = state.approved_spec_ref;
  const approved_plan_ref = state.confirmation_artifact_path ?? path.posix.join(getTaskArtifactsDir(taskId), 'plan-brief.md');

  if (!approved_spec_ref || !approved_plan_ref) {
    throw new Error(`缺少冻结引用: ${taskId}`);
  }

  const confirmation_event_path = await appendConfirmationEvent(taskId, {
    task_id: taskId,
    confirmation_type: 'plan',
    artifact_ref: approved_plan_ref,
    decision: 'approved',
    actor: 'user',
    timestamp: nowIso(),
    note: 'plan approved'
  });

  const handoff = await createDispatchArtifacts({
    task_id: taskId,
    approved_spec_ref,
    approved_plan_ref
  });

  const result_path = path.posix.join(getTaskArtifactsDir(taskId), 'exec-result-exec-01.yaml');

  await writeState({
    ...state,
    approved_plan_ref,
    status: 'dispatched',
    confirmation_pending: 'none',
    confirmation_artifact_path: null,
    confirmation_event_path,
    context_bundle_ref: handoff.context_bundle_ref,
    dispatch_contract_ref: handoff.dispatch_contract_ref,
    active_exec_units: ['exec-01'],
    exec_units: {
      'exec-01': {
        status: 'pending',
        contract_path: handoff.dispatch_contract_ref,
        attempt: 0,
        exit_code: null,
        result_path,
        blocked_by: []
      }
    },
    updated_at: nowIso()
  });

  return ['[Aria]', '- status: dispatched'].join('\n');
}
