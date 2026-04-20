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
import { createFakeBinaries, cleanupFakeBinaries } from '../../fixtures/fake-binaries.js';

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-review-test-'));
  process.chdir(tempDir);
  await createFakeBinaries({ writeArtifactToDisk: true }, ORIGINAL_PATH);
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
