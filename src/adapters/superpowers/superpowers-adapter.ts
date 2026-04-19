import type { RequiredMethodsEvidence } from '../../schemas/runtime-artifact-schema.js';

export function getSuperpowersCapability() {
  return {
    available: true,
    source: 'installed-skill'
  };
}

export function buildSuperpowersEvidence(input: { stage: 'clarification' | 'spec' | 'plan' | 'review' | 'test' }): RequiredMethodsEvidence {
  const methods = {
    clarification: ['brainstorming'],
    spec: ['brainstorming'],
    plan: ['writing-plans'],
    review: ['verification-before-completion'],
    test: ['test-driven-development', 'verification-before-completion']
  } satisfies Record<string, string[]>;

  return {
    provider: 'superpowers',
    methods: methods[input.stage],
    evidence_type: 'required-methods'
  };
}
