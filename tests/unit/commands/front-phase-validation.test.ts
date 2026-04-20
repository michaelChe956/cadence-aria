import { describe, expect, it } from 'vitest';
import { validateFrontPhaseArtifact } from '../../../src/commands/shared/front-phase-validation.js';

describe('validateFrontPhaseArtifact', () => {
  it('spec 类型验证通过', () => {
    expect(() =>
      validateFrontPhaseArtifact({
        content: [
          'producer: claude-code',
          'source_capabilities: [OpenSpec, superpowers]',
          'open_spec_evidence: "provider=OpenSpec approved_refs=spec.md,plan.md evidence_type=approved-artifact-ref"',
          'superpowers_evidence: "provider=superpowers methods=brainstorming evidence_type=required-methods"'
        ].join('\n'),
        artifactType: 'spec',
        expectedSpecRef: 'spec.md',
        expectedPlanRef: 'plan.md'
      })
    ).not.toThrow();
  });

  it('plan 类型验证通过', () => {
    expect(() =>
      validateFrontPhaseArtifact({
        content: [
          'producer: claude-code',
          'source_capabilities: [OpenSpec, superpowers]',
          'open_spec_evidence: "provider=OpenSpec approved_refs=spec.md,plan.md evidence_type=approved-artifact-ref"',
          'superpowers_evidence: "provider=superpowers methods=writing-plans evidence_type=required-methods"'
        ].join('\n'),
        artifactType: 'plan',
        expectedSpecRef: 'spec.md',
        expectedPlanRef: 'plan.md'
      })
    ).not.toThrow();
  });

  it('缺少 producer 时抛出错误', () => {
    expect(() =>
      validateFrontPhaseArtifact({
        content: 'producer: unknown\nsource_capabilities: [OpenSpec, superpowers]',
        artifactType: 'spec',
        expectedSpecRef: 'spec.md',
        expectedPlanRef: 'plan.md'
      })
    ).toThrow('来源证明: producer');
  });

  it('缺少 OpenSpec source_capabilities 时抛出错误', () => {
    expect(() =>
      validateFrontPhaseArtifact({
        content: 'producer: claude-code\nsource_capabilities: [superpowers]',
        artifactType: 'spec',
        expectedSpecRef: 'spec.md',
        expectedPlanRef: 'plan.md'
      })
    ).toThrow('来源证明: source_capabilities');
  });

  it('open_spec_evidence 不匹配时抛出错误', () => {
    expect(() =>
      validateFrontPhaseArtifact({
        content: [
          'producer: claude-code',
          'source_capabilities: [OpenSpec, superpowers]',
          'open_spec_evidence: "wrong evidence"',
          'superpowers_evidence: "provider=superpowers methods=brainstorming evidence_type=required-methods"'
        ].join('\n'),
        artifactType: 'spec',
        expectedSpecRef: 'spec.md',
        expectedPlanRef: 'plan.md'
      })
    ).toThrow('来源证明: open_spec_evidence');
  });

  it('superpowers_evidence 不匹配时抛出错误', () => {
    expect(() =>
      validateFrontPhaseArtifact({
        content: [
          'producer: claude-code',
          'source_capabilities: [OpenSpec, superpowers]',
          'open_spec_evidence: "provider=OpenSpec approved_refs=spec.md,plan.md evidence_type=approved-artifact-ref"',
          'superpowers_evidence: "wrong evidence"'
        ].join('\n'),
        artifactType: 'spec',
        expectedSpecRef: 'spec.md',
        expectedPlanRef: 'plan.md'
      })
    ).toThrow('来源证明: superpowers_evidence');
  });
});
