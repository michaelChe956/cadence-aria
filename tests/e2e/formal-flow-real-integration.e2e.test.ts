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

type FakeBinaryOptions = {
  codexExitCode?: number;
  reviewVerdict?: 'passed' | 'failed';
  testVerdict?: 'passed' | 'failed';
};

async function createFakeBinaries(options: FakeBinaryOptions = {}): Promise<void> {
  const { codexExitCode = 0, reviewVerdict = 'passed', testVerdict = 'passed' } = options;
  fakeBinDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-formal-flow-bins-'));

  const codexPath = path.join(fakeBinDir, 'codex');
  const claudePath = path.join(fakeBinDir, 'claude');

  const codexScript = `#!/usr/bin/env node
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
].join('\\n'), 'utf8');
process.exit(${codexExitCode});
`;

  const claudeScript = `#!/usr/bin/env node
const prompt = process.argv.slice(2).join(' ');
const taskId = (prompt.match(/task_id: (.+?)(?:\\n|$)/m) ?? [])[1] ?? 'unknown';
const resultSetId = (prompt.match(/result_set_id: (.+?)(?:\\n|$)/m) ?? [])[1] ?? 'result-set-unknown';

if (prompt.includes('Claude Code Review Prompt')) {
  process.stdout.write([
    'task_id: ' + taskId,
    'result_set_id: ' + resultSetId,
    'exec_units_reviewed:',
    '  - exec-01',
    'baseline_refs:',
    '  - artifacts/spec-artifact.md',
    '  - artifacts/plan-brief.md',
    'method_refs:',
    '  - verification-before-completion',
    'blockers: []',
    'suggestions: []',
    'verdict: ${reviewVerdict}',
    'producer: claude-code',
    'source_capabilities:',
    '  - OpenSpec',
    '  - superpowers',
    'generated_at: 2026-04-19T00:00:02.000Z',
    ''
  ].join('\\n'));
} else {
  process.stdout.write([
    'task_id: ' + taskId,
    'result_set_id: ' + resultSetId,
    'exec_units_tested:',
    '  - exec-01',
    'baseline_refs:',
    '  - artifacts/spec-artifact.md',
    '  - artifacts/plan-brief.md',
    'method_refs:',
    '  - test-driven-development',
    '  - verification-before-completion',
    'commands_run:',
    '  - pnpm check',
    '  - pnpm test',
    'failures: []',
    'passed_count: 2',
    'failed_count: 0',
    'verdict: ${testVerdict}',
    'producer: claude-code',
    'source_capabilities:',
    '  - OpenSpec',
    '  - superpowers',
    'generated_at: 2026-04-19T00:00:03.000Z',
    ''
  ].join('\\n'));
}
process.exit(0);
`;

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
  }, 15000);

  it('exec 失败后任务状态进入 blocked', async () => {
    const intake = await intakeCommand('exec 失败 E2E');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    expect(await startCommand(taskId)).toContain('spec-review');
    expect(await confirmSpecCommand(taskId)).toContain('plan-review');
    expect(await confirmPlanCommand(taskId)).toContain('dispatched');

    if (fakeBinDir) {
      await fs.rm(fakeBinDir, { recursive: true, force: true });
    }
    await createFakeBinaries({ codexExitCode: 1 });

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

    if (fakeBinDir) {
      await fs.rm(fakeBinDir, { recursive: true, force: true });
    }
    await createFakeBinaries({ reviewVerdict: 'failed', testVerdict: 'passed' });

    const runOutput = await runCommand(taskId);
    const state = await readState(taskId);

    expect(runOutput).toContain('status: patching');
    expect(state.status).toBe('patching');
    expect(state.review_status).toBe('failed');
    expect(state.test_status).toBe('passed');
    expect(state.patch_required_by).toBe('review');
  }, 15000);
});
