import type { ApprovedArtifactEvidence } from '../../schemas/runtime-artifact-schema.js';

export function getOpenSpecCapability() {
  return {
    available: true,
    source: 'configured-ref'
  };
}

export function buildOpenSpecEvidence(input: { specRef: string; planRef: string }): ApprovedArtifactEvidence {
  return {
    provider: 'OpenSpec',
    approved_refs: [input.specRef, input.planRef],
    evidence_type: 'approved-artifact-ref'
  };
}
