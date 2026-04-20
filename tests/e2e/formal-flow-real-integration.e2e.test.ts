import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { confirmPlanCommand } from '../../src/commands/confirm-plan.js';
import { confirmSpecCommand } from '../../src/commands/confirm-spec.js';
import { intakeCommand } from '../../src/commands/intake.js';
import { runCommand } from '../../src/commands/run.js';
import { startCommand } from '../../src/commands/start.js';
import { readState } from '../../src/runtime/persistence/state-repository.js';
import { createFakeBinaries, cleanupFakeBinaries } from '../fixtures/fake-binaries.js';

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-formal-flow-e2e-'));
  process.chdir(tempDir);
  await createFakeBinaries({}, ORIGINAL_PATH);
}

async function restoreWorkspace(): Promise<void> {
  process.chdir(ORIGINAL_CWD);
  process.env.PATH = ORIGINAL_PATH;
  await cleanupFakeBinaries();
  if (tempDir) {
    await fs.rm(tempDir, { recursive: true, force: true });
    tempDir = '';
  }
}

beforeEach(async () => {
  await setTempWorkspace();
});

afterEach(async () => {
  await restoreWorkspace();
});

describe('formal flow real integration e2e', () => {
  it('跑通 intake -> start -> confirm-spec -> confirm-plan -> run，并产出正式闭环工件', async () => {
    const intake = await intakeCommand('一期真实闭环 E2E');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    expect(await startCommand(taskId)).toContain('spec-review');
    expect(await confirmSpecCommand(taskId)).toContain('plan-review');
    expect(await confirmPlanCommand(taskId)).toContain('dispatched');

    const runOutput = await runCommand(taskId);
    const state = await readState(taskId);

    expect(runOutput).toContain(`status: ${state.status}`);
    expect(['verified', 'patching', 'blocked']).toContain(state.status);

    await expect(fs.access(`cadence/cache/aria/tasks/${taskId}/artifacts/spec-artifact.md`)).resolves.toBeUndefined();
    await expect(fs.access(`cadence/cache/aria/tasks/${taskId}/artifacts/plan-brief.md`)).resolves.toBeUndefined();
    await expect(fs.access(`cadence/cache/aria/tasks/${taskId}/artifacts/exec-result-exec-01.yaml`)).resolves.toBeUndefined();
    await expect(fs.access(`cadence/cache/aria/tasks/${taskId}/artifacts/review-report.yaml`)).resolves.toBeUndefined();
    await expect(fs.access(`cadence/cache/aria/tasks/${taskId}/artifacts/test-report.yaml`)).resolves.toBeUndefined();
  }, 15000);

  it('exec 失败后任务状态进入 blocked', async () => {
    const intake = await intakeCommand('exec 失败 E2E');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    expect(await startCommand(taskId)).toContain('spec-review');
    expect(await confirmSpecCommand(taskId)).toContain('plan-review');
    expect(await confirmPlanCommand(taskId)).toContain('dispatched');

    await cleanupFakeBinaries();
    await createFakeBinaries({ codexExitCode: 1 }, ORIGINAL_PATH);

    await expect(runCommand(taskId)).rejects.toThrow();
    const state = await readState(taskId);
    expect(state.status).toBe('blocked');
    expect(state.blocking_stage).toBe('executing');
    expect(state.exec_units['exec-01'].status).toBe('failed');
  }, 15000);

  it('review 失败后任务状态进入 patching', async () => {
    const intake = await intakeCommand('review 失败 E2E');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    expect(await startCommand(taskId)).toContain('spec-review');
    expect(await confirmSpecCommand(taskId)).toContain('plan-review');
    expect(await confirmPlanCommand(taskId)).toContain('dispatched');

    await cleanupFakeBinaries();
    await createFakeBinaries({ reviewVerdict: 'failed', testVerdict: 'passed' }, ORIGINAL_PATH);

    const runOutput = await runCommand(taskId);
    const state = await readState(taskId);

    expect(runOutput).toContain('status: patching');
    expect(state.status).toBe('patching');
    expect(state.review_status).toBe('failed');
    expect(state.test_status).toBe('passed');
    expect(state.patch_required_by).toBe('review');
  }, 15000);
});
