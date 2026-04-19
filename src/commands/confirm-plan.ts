import fs from 'node:fs/promises';
import path from 'node:path';

import { appendConfirmationEvent } from '../runtime/persistence/confirmation-event-repository.js';
import { getTaskArtifactsDir } from '../runtime/persistence/paths.js';
import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { canTransition } from '../runtime/state-machine/state-machine.js';
import { parseYaml } from '../utils/yaml.js';
import { nowIso } from '../utils/time.js';

import { createDispatchArtifacts } from '../runtime/contracts/dispatch-contract.js';
import {
  validateDispatchContract,
  validateExecutionContextBundle
} from '../runtime/contracts/contract-validator.js';

function validateFrontPhaseArtifact(input: {
  content: string;
  artifactType: 'spec' | 'plan';
  expectedSpecRef: string;
  expectedPlanRef: string;
}): void {
  const producer = input.content.match(/^producer: (.+)$/m)?.[1]?.trim();
  if (producer !== 'claude-code') {
    throw new Error(`缺少合法 ${input.artifactType} 来源证明: producer`);
  }

  const sourceCapabilities = input.content.match(/^source_capabilities: \[(.+)\]$/m)?.[1]?.trim();
  if (sourceCapabilities !== 'OpenSpec, superpowers') {
    throw new Error(`缺少合法 ${input.artifactType} 来源证明: source_capabilities`);
  }

  const openSpecEvidence = input.content.match(/^open_spec_evidence: (.+)$/m)?.[1]?.trim();
  const expectedOpenSpecEvidence = `provider=OpenSpec approved_refs=${input.expectedSpecRef},${input.expectedPlanRef} evidence_type=approved-artifact-ref`;
  if (openSpecEvidence !== expectedOpenSpecEvidence) {
    throw new Error(`缺少合法 ${input.artifactType} 来源证明: open_spec_evidence`);
  }

  const superpowersEvidence = input.content.match(/^superpowers_evidence: (.+)$/m)?.[1]?.trim();
  const expectedMethods = input.artifactType === 'spec' ? 'methods=brainstorming' : 'methods=writing-plans';
  const expectedSuperpowersEvidence = `provider=superpowers ${expectedMethods} evidence_type=required-methods`;
  if (superpowersEvidence !== expectedSuperpowersEvidence) {
    throw new Error(`缺少合法 ${input.artifactType} 来源证明: superpowers_evidence`);
  }
}

export async function confirmPlanCommand(taskId: string): Promise<string> {
  const state = await readState(taskId);
  if (state.status !== 'plan-review') {
    throw new Error(`任务不在可确认 plan 的状态: ${taskId}`);
  }

  const approved_spec_ref = state.approved_spec_ref;
  const approved_plan_ref = state.confirmation_artifact_path;

  if (!approved_spec_ref || !approved_plan_ref) {
    throw new Error(`缺少待确认 plan 工件或冻结引用: ${taskId}`);
  }

  const planContent = await fs.readFile(approved_plan_ref, 'utf8');
  validateFrontPhaseArtifact({
    content: planContent,
    artifactType: 'plan',
    expectedSpecRef: approved_spec_ref,
    expectedPlanRef: approved_plan_ref
  });

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
  const bundle = validateExecutionContextBundle(
    parseYaml(await fs.readFile(handoff.context_bundle_ref, 'utf8'))
  );
  const contract = validateDispatchContract(
    parseYaml(await fs.readFile(handoff.dispatch_contract_ref, 'utf8'))
  );

  if (bundle.spec_ref !== approved_spec_ref || bundle.plan_ref !== approved_plan_ref) {
    throw new Error('execution context bundle 与冻结引用不一致');
  }

  if (
    contract.based_on_spec_ref !== approved_spec_ref ||
    contract.based_on_plan_ref !== approved_plan_ref ||
    contract.context_bundle_ref !== handoff.context_bundle_ref
  ) {
    throw new Error('dispatch contract 与冻结引用或 bundle 不一致');
  }

  const transition = canTransition({
    ...state,
    approved_plan_ref,
    confirmation_artifact_path: approved_plan_ref,
    context_bundle_ref: handoff.context_bundle_ref,
    dispatch_contract_ref: handoff.dispatch_contract_ref
  }, 'dispatched');
  if (!transition.allowed) {
    throw new Error(`无法推进到 dispatched: ${transition.reason}`);
  }

  const nextState = {
    ...state,
    approved_plan_ref,
    status: 'dispatched' as const,
    confirmation_pending: 'none' as const,
    confirmation_artifact_path: null,
    confirmation_event_path,
    context_bundle_ref: handoff.context_bundle_ref,
    dispatch_contract_ref: handoff.dispatch_contract_ref,
    active_exec_units: ['exec-01'],
    exec_units: {
      'exec-01': {
        status: 'pending' as const,
        contract_path: handoff.dispatch_contract_ref,
        attempt: 0,
        exit_code: null,
        result_path,
        blocked_by: []
      }
    },
    updated_at: nowIso()
  };

  await writeState(nextState);

  return ['[Aria]', '- status: dispatched'].join('\n');
}
