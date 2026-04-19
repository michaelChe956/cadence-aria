import type { ApprovedArtifactEvidence, RequiredMethodsEvidence } from '../../schemas/runtime-artifact-schema.js';

type PlanBriefInput = {
  taskId: string;
  openSpecEvidence: ApprovedArtifactEvidence;
  superpowersEvidence: RequiredMethodsEvidence;
};

export function buildPlanBrief(input: PlanBriefInput): string {
  return [
    'producer: claude-code',
    'source_capabilities: [OpenSpec, superpowers]',
    `open_spec_evidence: provider=${input.openSpecEvidence.provider} approved_refs=${input.openSpecEvidence.approved_refs.join(',')} evidence_type=${input.openSpecEvidence.evidence_type}`,
    `superpowers_evidence: provider=${input.superpowersEvidence.provider} methods=${input.superpowersEvidence.methods.join(',')} evidence_type=${input.superpowersEvidence.evidence_type}`,
    `plan_id: plan-${input.taskId}`,
    'exec_unit_count: 1',
    'acceptance_strategy: all_units_pass'
  ].join('\n');
}
