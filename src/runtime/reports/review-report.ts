import { nowIso } from '../../utils/time.js';

export function buildReviewReport(taskId: string, resultSetId: string) {
  return {
    task_id: taskId,
    result_set_id: resultSetId,
    exec_units_reviewed: ['exec-01'],
    baseline_refs: ['artifacts/spec-artifact.md', 'artifacts/plan-brief.md'],
    method_refs: ['verification-before-completion'],
    blockers: [],
    suggestions: [],
    verdict: 'passed',
    producer: 'claude-code',
    source_capabilities: ['OpenSpec', 'superpowers'],
    generated_at: nowIso()
  };
}
