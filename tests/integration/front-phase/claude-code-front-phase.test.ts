import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { confirmPlanCommand } from '../../../src/commands/confirm-plan.js';
import { confirmSpecCommand } from '../../../src/commands/confirm-spec.js';
import { intakeCommand } from '../../../src/commands/intake.js';
import { startCommand } from '../../../src/commands/start.js';
import { intakeFormalTask, startFormalTask } from '../../../src/runtime/orchestrator/task-orchestrator.js';

const ORIGINAL_CWD = process.cwd();
let tempDir = '';

beforeEach(async () => {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-front-phase-'));
  process.chdir(tempDir);
});

afterEach(async () => {
  process.chdir(ORIGINAL_CWD);
  await fs.rm(tempDir, { recursive: true, force: true });
});

describe('front phase with claude code', () => {
  it('生成的 spec 和 plan 带有 OpenSpec 与 superpowers 来源字段', async () => {
    const intake = await intakeCommand('为 Aria 增加真实前段编排');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);

    const spec = await fs.readFile(`cadence/cache/aria/tasks/${taskId}/artifacts/spec-artifact.md`, 'utf8');
    const plan = await fs.readFile(`cadence/cache/aria/tasks/${taskId}/artifacts/plan-brief.md`, 'utf8');

    expect(spec).toContain('producer: claude-code');
    expect(spec).toContain('source_capabilities: [OpenSpec, superpowers]');
    expect(plan).toContain('producer: claude-code');
    expect(plan).toContain('source_capabilities: [OpenSpec, superpowers]');
  });

  it('兼容入口 task-orchestrator 也会生成带来源证明的工件', async () => {
    const taskId = await intakeFormalTask('为 Aria 增加真实前段编排');

    await startFormalTask(taskId);

    const spec = await fs.readFile(`cadence/cache/aria/tasks/${taskId}/artifacts/spec-artifact.md`, 'utf8');
    const plan = await fs.readFile(`cadence/cache/aria/tasks/${taskId}/artifacts/plan-brief.md`, 'utf8');

    expect(spec).toContain('producer: claude-code');
    expect(plan).toContain('producer: claude-code');
    expect(spec).toContain('open_spec_evidence: provider=OpenSpec');
    expect(plan).toContain('superpowers_evidence: provider=superpowers');
  });

  it('篡改 spec 来源字段后 confirm-spec 会拒绝推进', async () => {
    const intake = await intakeCommand('为 Aria 增加真实前段编排');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);

    const specPath = `cadence/cache/aria/tasks/${taskId}/artifacts/spec-artifact.md`;
    const spec = await fs.readFile(specPath, 'utf8');
    await fs.writeFile(
      specPath,
      spec
        .replace('producer: claude-code', 'producer: fake-agent')
        .replace('source_capabilities: [OpenSpec, superpowers]', 'source_capabilities: [OpenSpec]'),
      'utf8'
    );

    await expect(confirmSpecCommand(taskId)).rejects.toThrow('缺少合法 spec 来源证明');
  });

  it('篡改 plan 来源字段后 confirm-plan 会拒绝推进', async () => {
    const intake = await intakeCommand('为 Aria 增加真实前段编排');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);

    const planPath = `cadence/cache/aria/tasks/${taskId}/artifacts/plan-brief.md`;
    const plan = await fs.readFile(planPath, 'utf8');
    await fs.writeFile(
      planPath,
      plan
        .replace('producer: claude-code', 'producer: fake-agent')
        .replace('source_capabilities: [OpenSpec, superpowers]', 'source_capabilities: [OpenSpec]'),
      'utf8'
    );

    await expect(confirmPlanCommand(taskId)).rejects.toThrow('缺少合法 plan 来源证明');
  });
});
