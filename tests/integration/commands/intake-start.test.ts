import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { intakeCommand } from '../../../src/commands/intake.js';
import { startCommand } from '../../../src/commands/start.js';
import { readState, writeState } from '../../../src/runtime/persistence/state-repository.js';
import { getTaskArtifactsDir } from '../../../src/runtime/persistence/paths.js';

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
    expect(intake).toContain('source: aria-native');
    expect(intake).toContain('flow_type_suggestion: formal');
    expect(intake).toContain('risk_level: medium');

    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';
    const artifactsDir = getTaskArtifactsDir(taskId);
    const intakeCardPath = path.join(artifactsDir, 'task-intake-card.md');
    const specPath = path.join(artifactsDir, 'spec-artifact.md');
    const planPath = path.join(artifactsDir, 'plan-brief.md');

    await fs.rm(intakeCardPath, { force: true });

    const start = await startCommand(taskId);
    expect(start).toContain('status: spec-review');
    expect(start).toContain('clarification_required: false');
    expect(start).toContain('next: confirm-spec');

    await expect(fs.access(specPath)).resolves.toBeUndefined();
    await expect(fs.access(planPath)).resolves.toBeUndefined();

    const state = await readState(taskId);
    expect(state.task_title).toBe('为 Aria 增加 capability report 结构化输出');
    expect(state.status).toBe('spec-review');
    expect(state.confirmation_pending).toBe('spec');
    expect(state.confirmation_artifact_path).toBe(specPath);

    const specContent = await fs.readFile(specPath, 'utf8');
    expect(specContent).toContain(state.task_title);

    await expect(startCommand(taskId)).rejects.toThrow('任务不在可启动状态');
  });

  it('旧状态缺少 task_title 时仍可通过 intake card 兼容启动并回填标题', async () => {
    const intake = await intakeCommand('为 Aria 增加 capability report 结构化输出');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';
    const artifactsDir = getTaskArtifactsDir(taskId);
    const intakeCardPath = path.join(artifactsDir, 'task-intake-card.md');
    const specPath = path.join(artifactsDir, 'spec-artifact.md');

    const legacyState = await readState(taskId);
    delete legacyState.task_title;
    await writeState(legacyState);

    const start = await startCommand(taskId);
    expect(start).toContain('status: spec-review');

    const state = await readState(taskId);
    expect(state.task_title).toBe('为 Aria 增加 capability report 结构化输出');
    expect(state.confirmation_artifact_path).toBe(specPath);
    await expect(fs.access(intakeCardPath)).resolves.toBeUndefined();
    await expect(fs.access(specPath)).resolves.toBeUndefined();
  });
});
