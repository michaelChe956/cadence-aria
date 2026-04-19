import { nowIso } from '../../utils/time.js';

export type CodexExecResult = {
  task_id: string;
  exec_unit_id: string;
  status: 'succeeded';
  changed_files: string[];
  summary: string;
  capabilities_used: string[];
  openspec_refs_consumed: string[];
  superpowers_refs_consumed: string[];
  degraded: boolean;
  degradation_reason: string | null;
  started_at: string;
  finished_at: string;
};

export async function runCodexExec(input: {
  task_id: string;
  unit_id: string;
}): Promise<CodexExecResult> {
  const startedAt = nowIso();
  const finishedAt = nowIso();

  return {
    task_id: input.task_id,
    exec_unit_id: input.unit_id,
    status: 'succeeded',
    changed_files: ['src/index.ts'],
    summary: '执行最小骨架生成',
    capabilities_used: ['codex'],
    openspec_refs_consumed: ['artifacts/spec-artifact.md'],
    superpowers_refs_consumed: ['test-driven-development', 'verification-before-completion'],
    degraded: false,
    degradation_reason: null,
    started_at: startedAt,
    finished_at: finishedAt
  };
}
