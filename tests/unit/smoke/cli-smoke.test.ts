import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { readState, writeState } from '../../../src/runtime/persistence/state-repository.js';
import { runCli } from '../../../src/commands/run-cli.js';

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';
let tempDir = '';
let fakeBinDir = '';

async function createFakeBinaries(): Promise<void> {
  fakeBinDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-smoke-bins-'));

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

  await fs.writeFile(codexPath, codexScript, 'utf8');
  await fs.writeFile(claudePath, '#!/bin/sh\nexit 0\n', 'utf8');
  await fs.chmod(codexPath, 0o755);
  await fs.chmod(claudePath, 0o755);

  process.env.PATH = `${fakeBinDir}${path.delimiter}${ORIGINAL_PATH}`;
}

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-smoke-'));
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

describe('runCli', () => {
  it('当未提供子命令时返回帮助文案', async () => {
    const output = await runCli([]);
    expect(output).toBe('aria:intake\naria:start\nconfirm-spec\nconfirm-plan\naria:run\naria:status\naria:result');
  });

  it('支持 help 与 doctor 入口，并对未知命令报错', async () => {
    await expect(runCli(['--help'])).resolves.toContain('aria:intake');
    await expect(runCli(['-h'])).resolves.toContain('aria:run');
    await expect(runCli(['doctor'])).resolves.toContain('OpenSpec');
    await expect(runCli(['unknown-command'])).rejects.toThrow('未知命令');
  });

  it('参数缺失时返回明确错误', async () => {
    await expect(runCli(['aria:intake'])).rejects.toThrow('aria:intake 需要标题');
    await expect(runCli(['aria:start'])).rejects.toThrow('缺少参数: --task-id');
    await expect(runCli(['aria:start', '--task-id', '   '])).rejects.toThrow('缺少参数值: --task-id');
  });

  it('可通过真实入口完成 intake/start/confirm-spec/confirm-plan', async () => {
    const intake = await runCli(['aria:intake', '为 Aria 增加 capability report 结构化输出']);
    expect(intake).toContain('task_id: aria-');

    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';
    expect(taskId).toMatch(/^aria-\d{8}-\d{3}$/);

    const start = await runCli(['aria:start', '--task-id', taskId]);
    expect(start).toContain('status: spec-review');

    const spec = await runCli(['confirm-spec', '--task-id', taskId]);
    expect(spec).toContain('status: plan-review');

    const plan = await runCli(['confirm-plan', '--task-id', taskId]);
    expect(plan).toContain('status: dispatched');
  });

  it('支持别名入口完成 run/status/result/cancel/retry', async () => {
    const intake = await runCli(['intake', '为 Aria 增加 capability report 结构化输出']);
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await runCli(['start', '--task-id', taskId]);
    await runCli(['confirm-spec', '--task-id', taskId]);
    await runCli(['confirm-plan', '--task-id', taskId]);

    const run = await runCli(['run', '--task-id', taskId]);
    expect(run).toContain('status: verified');

    const status = await runCli(['status', '--task-id', taskId]);
    expect(status).toContain('status: verified');

    const cancel = await runCli(['cancel', '--task-id', taskId]);
    expect(cancel).toContain('status: cancelled');

    await expect(runCli(['retry', '--task-id', taskId])).rejects.toThrow('任务不可重试');

    const state = await readState(taskId);
    await writeState({
      ...state,
      status: 'blocked',
      retryable: true,
      block_reason_code: 'execution_blocked',
      blocking_stage: 'reviewing/testing',
      required_action: 'rerun-exec'
    });

    const retry = await runCli(['retry', '--task-id', taskId]);
    expect(retry).toContain('status: executing');

    const result = await runCli(['result', '--task-id', taskId]);
    expect(result).toContain('final_status: executing');
  });
});
