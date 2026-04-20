import type { ApprovedArtifactEvidence, RequiredMethodsEvidence } from '../../schemas/runtime-artifact-schema.js';

type SpecArtifactInput = {
  title: string;
  openSpecEvidence: ApprovedArtifactEvidence;
  superpowersEvidence: RequiredMethodsEvidence;
};

export function buildSpecArtifact(input: SpecArtifactInput): string {
  return [
    '# Spec',
    'producer: claude-code',
    'source_capabilities:',
    '  - OpenSpec',
    '  - superpowers',
    `open_spec_evidence: provider=${input.openSpecEvidence.provider} approved_refs=${input.openSpecEvidence.approved_refs.join(',')} evidence_type=${input.openSpecEvidence.evidence_type}`,
    `superpowers_evidence: provider=${input.superpowersEvidence.provider} methods=${input.superpowersEvidence.methods.join(',')} evidence_type=${input.superpowersEvidence.evidence_type}`,
    `goal: ${input.title}`,
    'scope: 仅覆盖一期 formal flow 最小闭环'
  ].join('\n');
}
