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

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';
let fakeCodexDir = '';

async function createFakeCodexBinary(): Promise<void> {
  fakeCodexDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-fake-codex-'));
  const scriptPath = path.join(fakeCodexDir, 'codex');
  const claudePath = path.join(fakeCodexDir, 'claude');
  const script = String.raw`#!/usr/bin/env node
const fs = require('node:fs');
const path = require('node:path');

const args = process.argv.slice(2);
fs.writeFileSync(path.join(process.cwd(), 'codex-invocation.log'), args.join('\n') + '\n', 'utf8');

let outputPath = '';
let promptContent = '';

for (let index = 0; index < args.length; index += 1) {
  const arg = args[index];
  if (arg === 'exec' || arg === '--full-auto') {
    continue;
  }
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

if (!outputPath || !promptContent) {
  process.stderr.write('missing prompt or output path\n');
  process.exit(1);
}

const prompt = promptContent;
const taskId = (prompt.match(/^task_id: (.+)$/m) ?? [])[1] ?? 'unknown';
fs.writeFileSync(outputPath, '已读取规则并准备执行任务：' + taskId + '\n', 'utf8');
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

  await fs.writeFile(scriptPath, script, 'utf8');
  await fs.writeFile(claudePath, claudeScript, 'utf8');
  await fs.chmod(scriptPath, 0o755);
  await fs.chmod(claudePath, 0o755);
  process.env.PATH = `${fakeCodexDir}${path.delimiter}${ORIGINAL_PATH}`;
}

beforeEach(async () => {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-dispatch-exec-'));
  process.chdir(tempDir);
  await createFakeCodexBinary();
});

afterEach(async () => {
  process.chdir(ORIGINAL_CWD);
  process.env.PATH = ORIGINAL_PATH;
  if (fakeCodexDir) {
    await fs.rm(fakeCodexDir, { recursive: true, force: true });
    fakeCodexDir = '';
  }
  if (tempDir) {
    await fs.rm(tempDir, { recursive: true, force: true });
    tempDir = '';
  }
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
    expect(prompt).toContain('不要读取仓库规则与文件');
    expect(prompt).toContain('只输出一行中文摘要');

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
    expect(result.summary).toContain('已读取规则并准备执行任务');
  }, 15000);
});
