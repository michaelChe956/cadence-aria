import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { intakeCommand } from '../../../src/commands/intake.js';
import { startCommand } from '../../../src/commands/start.js';

const ORIGINAL_CWD = process.cwd();

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-intake-start-'));
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

describe('intake/start', () => {
  it('从 intake 创建任务并在 start 后进入 spec-review', async () => {
    const intake = await intakeCommand('为 Aria 增加 capability report 结构化输出');
    expect(intake).toContain('task_id: aria-');

    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';
    const start = await startCommand(taskId);
    expect(start).toContain('status: spec-review');
    expect(start).toContain('next: confirm-spec');
  });
});
