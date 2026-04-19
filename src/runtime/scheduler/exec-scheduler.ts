import fs from 'node:fs/promises';
import path from 'node:path';

import { runCodexExec } from '../../adapters/codex/codex-adapter.js';
import { detectCapabilities } from '../../adapters/capability-detector.js';
import { getTaskArtifactsDir } from '../persistence/paths.js';
import { readState, writeState } from '../persistence/state-repository.js';
import { stringifyYaml } from '../../utils/yaml.js';
import { nowIso } from '../../utils/time.js';

export async function runSingleExecUnit(taskId: string): Promise<void> {
  const capabilities = detectCapabilities();
  if (!capabilities.openspec.available || !capabilities.superpowers.available || !capabilities.codex.available) {
    throw new Error('capability_blocked');
  }

  const state = await readState(taskId);
  const execUnit = state.exec_units['exec-01'];
  if (state.status !== 'dispatched' || !execUnit) {
    throw new Error(`任务不在可执行状态: ${taskId}`);
  }

  const startedAt = nowIso();
  const runningState = {
    ...state,
    status: 'executing' as const,
    active_exec_units: ['exec-01'],
    exec_units: {
      ...state.exec_units,
      'exec-01': {
        ...execUnit,
        status: 'running' as const,
        started_at: startedAt
      }
    },
    updated_at: startedAt
  };
  await writeState(runningState);

  const result = await runCodexExec({
    task_id: taskId,
    unit_id: 'exec-01'
  });

  const resultPath = path.join(getTaskArtifactsDir(taskId), 'exec-result-exec-01.yaml');
  await fs.mkdir(path.dirname(resultPath), { recursive: true });
  await fs.writeFile(resultPath, stringifyYaml(result), 'utf8');

  await writeState({
    ...runningState,
    status: 'reviewing/testing',
    review_status: 'pending',
    test_status: 'pending',
    active_result_set_id: `result-set-${taskId}-01`,
    active_exec_units: [],
    exec_units: {
      ...runningState.exec_units,
      'exec-01': {
        ...runningState.exec_units['exec-01'],
        status: 'succeeded',
        attempt: runningState.exec_units['exec-01'].attempt + 1,
        exit_code: 0,
        result_path: resultPath,
        finished_at: result.finished_at
      }
    },
    updated_at: result.finished_at
  });
}
