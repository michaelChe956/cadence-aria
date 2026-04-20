import fs from 'node:fs/promises';
import path from 'node:path';

import { canTransition } from '../runtime/state-machine/state-machine.js';
import { appendConfirmationEvent } from '../runtime/persistence/confirmation-event-repository.js';
import { getTaskArtifactsDir } from '../runtime/persistence/paths.js';
import { readState, writeState } from '../runtime/persistence/state-repository.js';
import { parseYaml } from '../utils/yaml.js';
import { nowIso } from '../utils/time.js';

function validateFrontPhaseArtifact(input: {
  content: string;
  artifactType: 'spec' | 'plan';
  expectedSpecRef: string;
  expectedPlanRef: string;
}): void {
  const parsed = parseYaml(input.content) as Record<string, unknown>;

  if (parsed.producer !== 'claude-code') {
    throw new Error(`缺少合法 ${input.artifactType} 来源证明: producer`);
  }

  const sourceCapabilities = Array.isArray(parsed.source_capabilities)
    ? parsed.source_capabilities
    : [];
  if (!sourceCapabilities.includes('OpenSpec') || !sourceCapabilities.includes('superpowers')) {
    throw new Error(`缺少合法 ${input.artifactType} 来源证明: source_capabilities`);
  }

  const openSpecEvidence = String(parsed.open_spec_evidence ?? '');
  const expectedOpenSpecEvidence = `provider=OpenSpec approved_refs=${input.expectedSpecRef},${input.expectedPlanRef} evidence_type=approved-artifact-ref`;
  if (openSpecEvidence !== expectedOpenSpecEvidence) {
    throw new Error(`缺少合法 ${input.artifactType} 来源证明: open_spec_evidence`);
  }

  const superpowersEvidence = String(parsed.superpowers_evidence ?? '');
  const expectedMethods = input.artifactType === 'spec' ? 'methods=brainstorming' : 'methods=writing-plans';
  const expectedSuperpowersEvidence = `provider=superpowers ${expectedMethods} evidence_type=required-methods`;
  if (superpowersEvidence !== expectedSuperpowersEvidence) {
    throw new Error(`缺少合法 ${input.artifactType} 来源证明: superpowers_evidence`);
  }
}

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
    expectedPlanRef: path.join(getTaskArtifactsDir(taskId), 'plan-brief.md')
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
