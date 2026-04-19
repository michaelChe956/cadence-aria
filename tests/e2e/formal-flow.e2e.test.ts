import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { confirmPlanCommand } from '../../src/commands/confirm-plan.js';
import { confirmSpecCommand } from '../../src/commands/confirm-spec.js';
import { intakeCommand } from '../../src/commands/intake.js';
import { runCommand } from '../../src/commands/run.js';
import { startCommand } from '../../src/commands/start.js';

const ORIGINAL_CWD = process.cwd();

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-formal-flow-e2e-'));
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

describe('formal flow e2e', () => {
  it('跑通 intake -> start -> confirm-spec -> confirm-plan -> run', async () => {
    const intake = await intakeCommand('一期闭环 E2E');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    expect(await startCommand(taskId)).toContain('spec-review');
    expect(await confirmSpecCommand(taskId)).toContain('plan-review');
    expect(await confirmPlanCommand(taskId)).toContain('dispatched');
    expect(await runCommand(taskId)).toContain('reviewing/testing');
  });
});
