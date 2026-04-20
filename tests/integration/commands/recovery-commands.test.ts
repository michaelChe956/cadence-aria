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

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';
let fakeBinDir = '';

async function createFakeBinaries(): Promise<void> {
  fakeBinDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-recovery-bins-'));

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

  const claudeScript = String.raw`#!/usr/bin/env node
const prompt = process.argv.slice(2).join(' ');
const taskId = (prompt.match(/task_id: (.+?)(?:\n|$)/m) ?? [])[1] ?? 'unknown';
const resultSetId = (prompt.match(/result_set_id: (.+?)(?:\n|$)/m) ?? [])[1] ?? 'result-set-unknown';

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
    'verdict: passed',
    'producer: claude-code',
    'source_capabilities:',
    '  - OpenSpec',
    '  - superpowers',
    'generated_at: 2026-04-19T00:00:02.000Z',
    ''
  ].join('\n'));
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
    'verdict: passed',
    'producer: claude-code',
    'source_capabilities:',
    '  - OpenSpec',
    '  - superpowers',
    'generated_at: 2026-04-19T00:00:03.000Z',
    ''
  ].join('\n'));
}
process.exit(0);
`;

  await fs.writeFile(codexPath, codexScript, 'utf8');
  await fs.writeFile(claudePath, claudeScript, 'utf8');
  await fs.chmod(codexPath, 0o755);
  await fs.chmod(claudePath, 0o755);

  process.env.PATH = `${fakeBinDir}${path.delimiter}${ORIGINAL_PATH}`;
}

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-recovery-commands-'));
  process.chdir(tempDir);
  await createFakeBinaries();
}

async function restoreWorkspace(): Promise<void> {
  process.chdir(ORIGINAL_CWD);
  process.env.PATH = ORIGINAL_PATH;
  if (fakeBinDir) {
    await fs.rm(fakeBinDir, { recursive: true, force: true });
    fakeBinDir = '';
  }
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
    await runCommand(taskId);

    const status = await statusCommand(taskId);
    expect(status).toContain(`task_id: ${taskId}`);
    expect(status).toContain('status: verified');

    const cancelled = await cancelCommand(taskId);
    expect(cancelled).toContain('status: cancelled');

    const afterCancel = await readState(taskId);
    await writeState({
      ...afterCancel,
      status: 'blocked',
      block_reason_code: 'execution_blocked',
      blocking_stage: 'reviewing/testing',
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
