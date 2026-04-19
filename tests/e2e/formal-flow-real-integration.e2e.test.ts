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

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';
let fakeBinDir = '';

async function createFakeBinaries(): Promise<void> {
  fakeBinDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-formal-flow-bins-'));

  const codexPath = path.join(fakeBinDir, 'codex');
  const claudePath = path.join(fakeBinDir, 'claude');

  const codexScript = String.raw`#!/usr/bin/env node
const fs = require('node:fs');

const args = process.argv.slice(2);
let outputPath = '';
let promptContent = '';

for (let index = 0; index < args.length; index += 1) {
  const arg = args[index];
  if (arg === 'exec' || arg === '--full-auto') continue;
  if (arg === '-C') {
    index += 1;
    continue;
  }
  if (arg === '--output-last-message') {
    outputPath = args[index + 1] ?? '';
    index += 1;
    continue;
  }
  promptContent = arg;
}

const prompt = promptContent;
const taskId = (prompt.match(/^task_id: (.+)$/m) ?? [])[1] ?? 'unknown';

fs.writeFileSync(outputPath, [
  'task_id: ' + taskId,
  'exec_unit_id: exec-01',
  'status: succeeded',
  'changed_files:',
  '  - src/index.ts',
  'summary: fake codex exec',
  'capabilities_used:',
  '  - codex',
  'openspec_refs_consumed:',
  '  - artifacts/spec-artifact.md',
  'superpowers_refs_consumed:',
  '  - test-driven-development',
  '  - verification-before-completion',
  'degraded: false',
  'degradation_reason: null',
  'started_at: 2026-04-19T00:00:00.000Z',
  'finished_at: 2026-04-19T00:00:01.000Z',
  ''
].join('\n'), 'utf8');
process.exit(0);
`;

  const claudeScript = '#!/bin/sh\nexit 0\n';

  await fs.writeFile(codexPath, codexScript, 'utf8');
  await fs.writeFile(claudePath, claudeScript, 'utf8');
  await fs.chmod(codexPath, 0o755);
  await fs.chmod(claudePath, 0o755);

  process.env.PATH = `${fakeBinDir}${path.delimiter}${ORIGINAL_PATH}`;
}

beforeEach(async () => {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-formal-flow-e2e-'));
  process.chdir(tempDir);
  await createFakeBinaries();
});

afterEach(async () => {
  process.chdir(ORIGINAL_CWD);
  process.env.PATH = ORIGINAL_PATH;
  if (fakeBinDir) {
    await fs.rm(fakeBinDir, { recursive: true, force: true });
    fakeBinDir = '';
  }
  if (tempDir) {
    await fs.rm(tempDir, { recursive: true, force: true });
    tempDir = '';
  }
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
  });
});
