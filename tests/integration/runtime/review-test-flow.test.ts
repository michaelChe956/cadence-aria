import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { confirmPlanCommand } from '../../../src/commands/confirm-plan.js';
import { confirmSpecCommand } from '../../../src/commands/confirm-spec.js';
import { intakeCommand } from '../../../src/commands/intake.js';
import { runCommand } from '../../../src/commands/run.js';
import { startCommand } from '../../../src/commands/start.js';
import { readState } from '../../../src/runtime/persistence/state-repository.js';
import { parseYaml } from '../../../src/utils/yaml.js';

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';
let fakeBinDir = '';

async function createFakeBinaries(): Promise<void> {
  fakeBinDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-fake-bins-'));

  const codexPath = path.join(fakeBinDir, 'codex');
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

const yaml = [
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
].join('\n');

fs.writeFileSync(outputPath, yaml, 'utf8');
process.exit(0);
`;

  const claudePath = path.join(fakeBinDir, 'claude');
  const claudeScript = String.raw`#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');

const args = process.argv.slice(2);
const prompt = args.join(' ');
const taskId = (prompt.match(/task_id: (.+?)(?:\n|$)/m) ?? [])[1] ?? 'unknown';
const resultSetId = (prompt.match(/result_set_id: (.+?)(?:\n|$)/m) ?? [])[1] ?? 'result-set-unknown';
fs.appendFileSync(path.join(process.cwd(), 'claude-invocation.log'), prompt + '\n', 'utf8');

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

beforeEach(async () => {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-review-test-'));
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

describe('review and test flow', () => {
  it('run 之后生成 review/test report，并使任务进入 verified、patching 或 blocked', async () => {
    const intake = await intakeCommand('验证 review test 闭环');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    const output = await runCommand(taskId);

    const reviewPath = `cadence/cache/aria/tasks/${taskId}/artifacts/review-report.yaml`;
    const testPath = `cadence/cache/aria/tasks/${taskId}/artifacts/test-report.yaml`;

    await expect(fs.access(reviewPath)).resolves.toBeUndefined();
    await expect(fs.access(testPath)).resolves.toBeUndefined();

    const reviewReport = parseYaml(await fs.readFile(reviewPath, 'utf8')) as {
      producer: string;
      source_capabilities: string[];
      verdict: string;
    };
    const testReport = parseYaml(await fs.readFile(testPath, 'utf8')) as {
      producer: string;
      source_capabilities: string[];
      verdict: string;
    };

    expect(reviewReport.producer).toBe('claude-code');
    expect(reviewReport.source_capabilities).toEqual(['OpenSpec', 'superpowers']);
    expect(reviewReport.verdict).toBe('passed');
    expect(testReport.producer).toBe('claude-code');
    expect(testReport.source_capabilities).toEqual(['OpenSpec', 'superpowers']);
    expect(testReport.verdict).toBe('passed');

    const state = await readState(taskId);
    expect(['verified', 'patching', 'blocked']).toContain(state.status);
    expect(output).toContain(`status: ${state.status}`);

    const invocationLog = await fs.readFile(path.join(process.cwd(), 'claude-invocation.log'), 'utf8');
    expect(invocationLog).toContain('Claude Code Review Prompt');
    expect(invocationLog).toContain('Claude Code Test Prompt');
    expect(invocationLog).toContain('运行必要的检查命令');
    expect(invocationLog).toContain('只输出合法 YAML');
  }, 15000);
});
