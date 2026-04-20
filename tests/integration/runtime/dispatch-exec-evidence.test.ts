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
import { parseYaml } from '../../../src/utils/yaml.js';
import { createFakeBinaries, cleanupFakeBinaries } from '../../fixtures/fake-binaries.js';

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-dispatch-exec-'));
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

describe('dispatch and exec evidence', () => {
  it('exec result 由 Aria 侧生成并通过证据校验，不依赖 codex 输出 YAML', async () => {
    const intake = await intakeCommand('验证 exec 真实证据');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    const bundle = parseYaml(await fs.readFile(`cadence/cache/aria/tasks/${taskId}/artifacts/execution-context-bundle.yaml`, 'utf8')) as {
      spec_ref: string;
      plan_ref: string;
      scope_constraints_ref: string;
      source_capabilities: string[];
      required_methods: string[];
      verification_requirements: string[];
    };
    expect(bundle.spec_ref).toBe(`cadence/cache/aria/tasks/${taskId}/artifacts/spec-artifact.md`);
    expect(bundle.plan_ref).toBe(`cadence/cache/aria/tasks/${taskId}/artifacts/plan-brief.md`);
    expect(bundle.scope_constraints_ref).toBe(`cadence/cache/aria/tasks/${taskId}/artifacts/plan-brief.md`);
    expect(bundle.source_capabilities).toEqual(['OpenSpec', 'superpowers']);
    expect(bundle.required_methods).toEqual(['writing-plans', 'test-driven-development', 'verification-before-completion']);
    expect(bundle.verification_requirements).toEqual(['pnpm check', 'pnpm test']);

    const contract = parseYaml(await fs.readFile(`cadence/cache/aria/tasks/${taskId}/artifacts/dispatch-contract-exec-01.yaml`, 'utf8')) as {
      worker_cli: string;
      required_methods: string[];
      verification_requirements: string[];
    };
    expect(contract.worker_cli).toBe('codex');
    expect(contract.required_methods).toEqual(['test-driven-development', 'verification-before-completion']);
    expect(contract.verification_requirements).toEqual(['pnpm check', 'pnpm test']);

    const promptPath = path.join(getTaskArtifactsDir(taskId), 'exec-prompt-exec-01.md');
    const resultPath = path.join(getTaskArtifactsDir(taskId), 'exec-result-exec-01.yaml');

    await runCommand(taskId);

    const prompt = await fs.readFile(promptPath, 'utf8');
    expect(prompt).toContain(taskId);
    expect(prompt).toContain(bundle.spec_ref);
    expect(prompt).toContain(contract.required_methods[0]);
    expect(prompt).toContain('请先读取仓库规则、spec、plan 与 dispatch contract');
    expect(prompt).toContain('可以在允许范围内修改文件');
    expect(prompt).toContain('完成实现后运行 verification_requirements');

    const invocationLog = await fs.readFile(path.join(process.cwd(), 'codex-invocation.log'), 'utf8');
    expect(invocationLog).toContain('--output-last-message');
    expect(invocationLog).toContain(resultPath);
    expect(invocationLog).toContain(taskId);

    const result = parseYaml(await fs.readFile(resultPath, 'utf8')) as {
      task_id: string;
      exec_unit_id: string;
      capabilities_used: string[];
      openspec_refs_consumed: string[];
      superpowers_refs_consumed: string[];
      summary: string;
    };

    expect(result.task_id).toBe(taskId);
    expect(result.exec_unit_id).toBe('exec-01');
    expect(result.capabilities_used).toEqual(['codex']);
    expect(result.openspec_refs_consumed).toEqual(expect.arrayContaining(['artifacts/spec-artifact.md']));
    expect(result.superpowers_refs_consumed).toEqual(expect.arrayContaining(contract.required_methods));
    expect(result.summary).toContain('fake codex exec');
  }, 15000);
});
