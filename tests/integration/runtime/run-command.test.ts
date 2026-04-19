import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { confirmPlanCommand } from '../../../src/commands/confirm-plan.js';
import { confirmSpecCommand } from '../../../src/commands/confirm-spec.js';
import { intakeCommand } from '../../../src/commands/intake.js';
import { runCommand } from '../../../src/commands/run.js';
import { startCommand } from '../../../src/commands/start.js';
import { getTaskArtifactsDir } from '../../../src/runtime/persistence/paths.js';
import { readState } from '../../../src/runtime/persistence/state-repository.js';

const ORIGINAL_CWD = process.cwd();

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-run-command-'));
  process.chdir(tempDir);
}

async function restoreWorkspace(): Promise<void> {
  process.chdir(ORIGINAL_CWD);
  if (tempDir) {
    await fs.rm(tempDir, { recursive: true, force: true });
  }
}

beforeEach(async () => {
  await setTempWorkspace();
});

afterEach(async () => {
  await restoreWorkspace();
});

describe('runCommand', () => {
  it('将 dispatched 任务推进到 reviewing/testing 并写入 exec result', async () => {
    const intake = await intakeCommand('为 Aria 增加 capability report 结构化输出');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    const output = await runCommand(taskId);
    expect(output).toContain('status: reviewing/testing');
    expect(output).toContain('review_status: pending');

    const resultPath = path.join(getTaskArtifactsDir(taskId), 'exec-result-exec-01.yaml');
    await expect(fs.access(resultPath)).resolves.toBeUndefined();

    const state = await readState(taskId);
    expect(state.status).toBe('reviewing/testing');
    expect(state.test_status).toBe('pending');
    expect(state.active_result_set_id).toBe(`result-set-${taskId}-01`);
    expect(state.exec_units['exec-01']?.result_path).toBe(resultPath);
    expect(state.exec_units['exec-01']?.status).toBe('succeeded');
  });
});
