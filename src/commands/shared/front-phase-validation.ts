import { parseYaml } from '../../utils/yaml.js';

export function validateFrontPhaseArtifact(input: {
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
