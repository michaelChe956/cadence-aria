import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import { intakeCommand } from '../../../src/commands/intake.js';
import { confirmPlanCommand } from '../../../src/commands/confirm-plan.js';
import { confirmSpecCommand } from '../../../src/commands/confirm-spec.js';
import { startCommand } from '../../../src/commands/start.js';
import { createDispatchArtifacts } from '../../../src/runtime/contracts/dispatch-contract.js';
import { getTaskArtifactsDir } from '../../../src/runtime/persistence/paths.js';
import { readState, writeState } from '../../../src/runtime/persistence/state-repository.js';

const ORIGINAL_CWD = process.cwd();

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-handoff-'));
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

describe('handoff checkpoint', () => {
  it('基于冻结引用生成 bundle 与 dispatch contract', async () => {
    const artifactsDir = getTaskArtifactsDir('aria-20260418-001');
    const specRef = path.join(artifactsDir, 'spec-artifact.md');
    const planRef = path.join(artifactsDir, 'plan-brief.md');

    await fs.mkdir(artifactsDir, { recursive: true });
    await fs.writeFile(specRef, '# spec', 'utf8');
    await fs.writeFile(planRef, '# plan', 'utf8');

    const result = await createDispatchArtifacts({
      task_id: 'aria-20260418-001',
      approved_spec_ref: specRef,
      approved_plan_ref: planRef
    });

    expect(result.context_bundle_ref).toContain('execution-context-bundle');
    expect(result.dispatch_contract_ref).toContain('dispatch-contract-exec-01');
  });

  it('confirm-spec 与 confirm-plan 会冻结引用并推进到 dispatched', async () => {
    const intake = await intakeCommand('为 Aria 增加 capability report 结构化输出');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';
    const artifactsDir = getTaskArtifactsDir(taskId);
    const specPath = path.join(artifactsDir, 'spec-artifact.md');
    const planPath = path.join(artifactsDir, 'plan-brief.md');

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    const afterSpec = await readState(taskId);
    expect(afterSpec.status).toBe('plan-review');
    expect(afterSpec.approved_spec_ref).toBe(specPath);
    expect(afterSpec.confirmation_pending).toBe('plan');
    expect(afterSpec.confirmation_artifact_path).toBe(planPath);

    const result = await confirmPlanCommand(taskId);
    expect(result).toContain('status: dispatched');

    const afterPlan = await readState(taskId);
    expect(afterPlan.status).toBe('dispatched');
    expect(afterPlan.approved_plan_ref).toBe(planPath);
    expect(afterPlan.active_exec_units).toEqual(['exec-01']);
    expect(afterPlan.context_bundle_ref).toContain('execution-context-bundle');
    expect(afterPlan.dispatch_contract_ref).toContain('dispatch-contract-exec-01');
    expect(afterPlan.exec_units['exec-01'].contract_path).toBe(afterPlan.dispatch_contract_ref);
    expect(afterPlan.confirmation_pending).toBe('none');
    expect(afterPlan.confirmation_artifact_path).toBeNull();

    const bundlePath = afterPlan.context_bundle_ref ?? '';
    const contractPath = afterPlan.dispatch_contract_ref ?? '';
    await expect(fs.access(bundlePath)).resolves.toBeUndefined();
    await expect(fs.access(contractPath)).resolves.toBeUndefined();

    const contractContent = await fs.readFile(contractPath, 'utf8');
    expect(contractContent).toContain(specPath);
    expect(contractContent).toContain(planPath);
    expect(contractContent).toContain(bundlePath);
  });

  it('缺少 confirmation_artifact_path 时 confirmPlanCommand 会报错', async () => {
    const intake = await intakeCommand('为 Aria 增加 capability report 结构化输出');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);

    const state = await readState(taskId);
    await writeState({
      ...state,
      confirmation_artifact_path: null
    });

    await expect(confirmPlanCommand(taskId)).rejects.toThrow('缺少待确认 plan 工件');
  });
});
