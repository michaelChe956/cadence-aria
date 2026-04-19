import { nowIso } from '../../utils/time.js';

export function buildTestReport(taskId: string, resultSetId: string) {
  return {
    task_id: taskId,
    result_set_id: resultSetId,
    exec_units_tested: ['exec-01'],
    baseline_refs: ['artifacts/spec-artifact.md', 'artifacts/plan-brief.md'],
    method_refs: ['verification-before-completion'],
    commands_run: ['pnpm check', 'pnpm test'],
    failures: [],
    passed_count: 2,
    failed_count: 0,
    verdict: 'passed',
    producer: 'claude',
    source_capabilities: ['superpowers'],
    generated_at: nowIso()
  };
}
