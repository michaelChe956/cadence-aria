import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { cancelCommand } from '../../../src/commands/cancel.js';
import { confirmPlanCommand } from '../../../src/commands/confirm-plan.js';
import { confirmSpecCommand } from '../../../src/commands/confirm-spec.js';
import { doctorCommand } from '../../../src/commands/doctor.js';
import { intakeCommand } from '../../../src/commands/intake.js';
import { resultCommand } from '../../../src/commands/result.js';
import { retryCommand } from '../../../src/commands/retry.js';
import { runCommand } from '../../../src/commands/run.js';
import { startCommand } from '../../../src/commands/start.js';
import { statusCommand } from '../../../src/commands/status.js';
import { readState, writeState } from '../../../src/runtime/persistence/state-repository.js';
import { createFakeBinaries, cleanupFakeBinaries } from '../../fixtures/fake-binaries.js';

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-recovery-commands-'));
  process.chdir(tempDir);
  await createFakeBinaries({ writeArtifactToDisk: true }, ORIGINAL_PATH);
}

async function restoreWorkspace(): Promise<void> {
  process.chdir(ORIGINAL_CWD);
  process.env.PATH = ORIGINAL_PATH;
  await cleanupFakeBinaries();
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

describe('recovery commands', () => {
  it('doctor 输出 capability 探测结果', async () => {
    const output = await doctorCommand();
    expect(output).toContain('claude_code');
    expect(output).toContain('codex');
    expect(output).toContain('OpenSpec');
    expect(output).toContain('superpowers');
  });

  it('status/result/cancel/retry 基于 state.yaml 输出或推进状态', async () => {
    const intake = await intakeCommand('为 Aria 增加 capability report 结构化输出');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    // cancel from dispatched state (allowed)
    const afterDispatch = await readState(taskId);
    await writeState({
      ...afterDispatch,
      status: 'blocked',
      block_reason_code: 'execution_blocked',
      blocking_stage: 'executing',
      retryable: true,
      required_action: 'rerun-exec'
    });

    const cancelled = await cancelCommand(taskId);
    expect(cancelled).toContain('status: cancelled');

    // set to blocked with retryable for retry test
    const afterCancel = await readState(taskId);
    await writeState({
      ...afterCancel,
      status: 'blocked',
      block_reason_code: 'execution_blocked',
      blocking_stage: 'executing',
      retryable: true,
      required_action: 'rerun-exec'
    });

    const retried = await retryCommand(taskId);
    expect(retried).toContain('status: dispatched');

    const afterRetry = await readState(taskId);
    expect(afterRetry.status).toBe('dispatched');
    expect(afterRetry.active_exec_units).toEqual(['exec-01']);
    expect(afterRetry.exec_units['exec-01']?.status).toBe('pending');

    const result = await resultCommand(taskId);
    expect(result).toContain(`task_id: ${taskId}`);
    expect(result).toContain('final_status: dispatched');
  }, 15000);
});
