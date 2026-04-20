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
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';
let fakeBinDir = '';

async function createFakeBinaries(): Promise<void> {
  fakeBinDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-run-bins-'));

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

if (process.env.ARIA_FAKE_CODEX_FAIL === '1') {
  process.stderr.write('fake codex failed\n');
  process.exit(1);
}

fs.writeFileSync(outputPath, '已读取规则并准备执行任务：' + taskId + '\n', 'utf8');
process.exit(0);
`;

  await fs.writeFile(codexPath, codexScript, 'utf8');
  const claudeScript = String.raw`#!/usr/bin/env node
const mode = process.env.ARIA_FAKE_CLAUDE_MODE ?? 'pass';
const prompt = process.argv.slice(2).join(' ');
const taskId = (prompt.match(/task_id: (.+?)(?:\\n|$)/m) ?? [])[1] ?? 'unknown';
const resultSetId = (prompt.match(/result_set_id: (.+?)(?:\\n|$)/m) ?? [])[1] ?? 'result-set-unknown';

if (mode === 'invalid') {
  process.stdout.write('not-yaml\\n');
  process.exit(0);
}

const reportTaskId = mode === 'mismatch-task' ? 'aria-19990101-001' : taskId;
const reportResultSetId = mode === 'mismatch-result-set' ? 'result-set-mismatch-01' : resultSetId;

const isReview = prompt.includes('Claude Code Review Prompt');
const yaml = isReview
  ? [
      'task_id: ' + reportTaskId,
      'result_set_id: ' + reportResultSetId,
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
    ].join('\n')
  : [
      'task_id: ' + reportTaskId,
      'result_set_id: ' + reportResultSetId,
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
    ].join('\n');

process.stdout.write(yaml);
process.exit(0);
`;

  await fs.writeFile(claudePath, claudeScript, 'utf8');
  await fs.chmod(codexPath, 0o755);
  await fs.chmod(claudePath, 0o755);

  process.env.PATH = `${fakeBinDir}${path.delimiter}${ORIGINAL_PATH}`;
}

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-run-command-'));
  process.chdir(tempDir);
  await createFakeBinaries();
}

async function restoreWorkspace(): Promise<void> {
  process.chdir(ORIGINAL_CWD);
  process.env.PATH = ORIGINAL_PATH;
  delete process.env.ARIA_FAKE_CODEX_FAIL;
  delete process.env.ARIA_FAKE_CLAUDE_MODE;
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

describe('runCommand', () => {
  it('将 dispatched 任务推进到最终 review/test 仲裁状态并写入正式报告', async () => {
    const intake = await intakeCommand('为 Aria 增加 capability report 结构化输出');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    const output = await runCommand(taskId);
    expect(output).toContain('status: verified');
    expect(output).toContain('review_status: passed');
    expect(output).toContain('test_status: passed');

    const resultPath = path.join(getTaskArtifactsDir(taskId), 'exec-result-exec-01.yaml');
    const reviewPath = path.join(getTaskArtifactsDir(taskId), 'review-report.yaml');
    const testPath = path.join(getTaskArtifactsDir(taskId), 'test-report.yaml');
    await expect(fs.access(resultPath)).resolves.toBeUndefined();
    await expect(fs.access(reviewPath)).resolves.toBeUndefined();
    await expect(fs.access(testPath)).resolves.toBeUndefined();

    const state = await readState(taskId);
    expect(state.status).toBe('verified');
    expect(state.review_status).toBe('passed');
    expect(state.test_status).toBe('passed');
    expect(state.active_result_set_id).toBe(`result-set-${taskId}-01`);
    expect(state.exec_units['exec-01']?.result_path).toBe(resultPath);
    expect(state.exec_units['exec-01']?.status).toBe('succeeded');
    expect(state.review_report_ref).toBe(reviewPath);
    expect(state.test_report_ref).toBe(testPath);
  }, 15000);

  it('当 codex 执行失败时收尾为 blocked，且不遗留 running exec unit', async () => {
    process.env.ARIA_FAKE_CODEX_FAIL = '1';
    const intake = await intakeCommand('验证 exec 失败收尾');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    await expect(runCommand(taskId)).rejects.toThrow(/codex_exec_failed/);

    const state = await readState(taskId);
    expect(state.status).toBe('blocked');
    expect(state.block_reason_code).toBe('execution_blocked');
    expect(state.blocking_stage).toBe('executing');
    expect(state.retryable).toBe(true);
    expect(state.exec_units['exec-01']?.status).toBe('failed');
    expect(state.exec_units['exec-01']?.exit_code).toBe(1);
    expect(state.exec_units['exec-01']?.finished_at).toBeTruthy();
  });

  it('当 claude 未输出合法 review/test 报告时拒绝推进 verified', async () => {
    process.env.ARIA_FAKE_CLAUDE_MODE = 'invalid';
    const intake = await intakeCommand('验证 review/test 报告必须来自实际输出');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    await expect(runCommand(taskId)).rejects.toThrow(/report/i);

    const state = await readState(taskId);
    expect(state.status).toBe('blocked');
    expect(state.block_reason_code).toBe('review_report_invalid');
    expect(state.blocking_stage).toBe('reviewing/testing');
    expect(state.retryable).toBe(false);
    expect(state.required_action).toBe('人工处理并补齐合法工件');
  });

  it('当 claude 输出的报告 task_id 不匹配时阻止推进 verified', async () => {
    process.env.ARIA_FAKE_CLAUDE_MODE = 'mismatch-task';
    const intake = await intakeCommand('验证 review/test 报告绑定到当前任务');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    await expect(runCommand(taskId)).rejects.toThrow(/review_report_task_mismatch/i);

    const state = await readState(taskId);
    expect(state.status).toBe('blocked');
    expect(state.block_reason_code).toBe('review_report_task_mismatch');
    expect(state.blocking_stage).toBe('reviewing/testing');
    expect(state.retryable).toBe(false);
  });
});
