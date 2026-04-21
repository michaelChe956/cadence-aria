import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';

import { afterEach, describe, expect, it } from 'vitest';

const REPO_ROOT = path.resolve(import.meta.dirname, '../../..');
const TASKS_ROOT = path.join(REPO_ROOT, 'cadence/cache/aria/tasks');
const SCRIPT_PATH = path.join(REPO_ROOT, 'scripts/verify-real-integration.sh');

const createdTaskIds: string[] = [];
const cleanupDirs: string[] = [];

async function createTaskFixture(input: { taskId: string; broken?: boolean; repoRoot?: string }): Promise<void> {
  const repoRoot = input.repoRoot ?? REPO_ROOT;
  const tasksRoot = path.join(repoRoot, 'cadence/cache/aria/tasks');
  const taskRoot = path.join(tasksRoot, input.taskId);
  const artifactsDir = path.join(taskRoot, 'artifacts');
  if (repoRoot === REPO_ROOT) {
    createdTaskIds.push(input.taskId);
  } else {
    cleanupDirs.push(repoRoot);
  }

  await fs.mkdir(artifactsDir, { recursive: true });
  await fs.writeFile(
    path.join(artifactsDir, 'spec-artifact.md'),
    [
      '# Spec',
      'producer: claude-code',
      input.broken ? 'source_capabilities: [OpenSpec]' : 'source_capabilities: [OpenSpec, superpowers]',
      'open_spec_evidence: provider=OpenSpec approved_refs=a,b evidence_type=approved-artifact-ref',
      'superpowers_evidence: provider=superpowers methods=brainstorming evidence_type=required-methods'
    ].join('\n'),
    'utf8'
  );
  await fs.writeFile(
    path.join(artifactsDir, 'plan-brief.md'),
    [
      'producer: claude-code',
      'source_capabilities: [OpenSpec, superpowers]',
      'open_spec_evidence: provider=OpenSpec approved_refs=a,b evidence_type=approved-artifact-ref',
      'superpowers_evidence: provider=superpowers methods=writing-plans evidence_type=required-methods'
    ].join('\n'),
    'utf8'
  );
  await fs.writeFile(
    path.join(artifactsDir, 'execution-context-bundle.yaml'),
    [
      'source_capabilities:',
      '  - OpenSpec',
      '  - superpowers',
      'required_methods:',
      '  - writing-plans',
      '  - test-driven-development',
      '  - verification-before-completion'
    ].join('\n'),
    'utf8'
  );
  await fs.writeFile(
    path.join(artifactsDir, 'dispatch-contract-exec-01.yaml'),
    [
      'worker_cli: codex',
      'required_methods:',
      '  - test-driven-development',
      '  - verification-before-completion'
    ].join('\n'),
    'utf8'
  );
  await fs.writeFile(
    path.join(artifactsDir, 'exec-result-exec-01.yaml'),
    [
      'capabilities_used:',
      '  - codex',
      'openspec_refs_consumed:',
      '  - artifacts/spec-artifact.md',
      'superpowers_refs_consumed:',
      '  - test-driven-development',
      '  - verification-before-completion'
    ].join('\n'),
    'utf8'
  );
  await fs.writeFile(
    path.join(artifactsDir, 'review-report.yaml'),
    [
      'producer: claude-code',
      'source_capabilities:',
      '  - OpenSpec',
      '  - superpowers',
      'verdict: passed'
    ].join('\n'),
    'utf8'
  );
  await fs.writeFile(
    path.join(artifactsDir, 'test-report.yaml'),
    [
      'producer: claude-code',
      'source_capabilities:',
      '  - OpenSpec',
      '  - superpowers',
      'verdict: passed'
    ].join('\n'),
    'utf8'
  );

  if (input.broken) {
    await fs.rm(path.join(artifactsDir, 'spec-artifact.md'), { force: true });
  }
}

async function runScript(input: {
  taskId?: string;
  repoRoot?: string;
}): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const args: string[] = [];
    if (input.taskId) {
      args.push('--task-id', input.taskId);
    }
    if (input.repoRoot) {
      args.push('--repo-root', input.repoRoot);
    }

    const child = spawn(SCRIPT_PATH, args, {
      cwd: REPO_ROOT
    });

    let stdout = '';
    let stderr = '';

    child.stdout.on('data', chunk => {
      stdout += chunk.toString();
    });

    child.stderr.on('data', chunk => {
      stderr += chunk.toString();
    });

    child.on('error', reject);
    child.on('close', code => {
      resolve({ code, stdout, stderr });
    });
  });
}

async function runScriptWithoutTaskId(): Promise<{ code: number | null; stdout: string; stderr: string }> {
  return new Promise((resolve, reject) => {
    const child = spawn(SCRIPT_PATH, [], {
      cwd: REPO_ROOT
    });

    let stdout = '';
    let stderr = '';

    child.stdout.on('data', chunk => {
      stdout += chunk.toString();
    });

    child.stderr.on('data', chunk => {
      stderr += chunk.toString();
    });

    child.on('error', reject);
    child.on('close', code => {
      resolve({ code, stdout, stderr });
    });
  });
}

afterEach(async () => {
  for (const taskId of createdTaskIds.splice(0)) {
    await fs.rm(path.join(TASKS_ROOT, taskId), { recursive: true, force: true });
  }

  for (const dir of cleanupDirs.splice(0)) {
    await fs.rm(dir, { recursive: true, force: true });
  }
});

describe('verify-real-integration script', () => {
  it('对完整任务输出 PASS', async () => {
    const taskId = `aria-${new Date().toISOString().slice(0, 10).replaceAll('-', '')}-910`;
    await createTaskFixture({ taskId });

    const result = await runScript({ taskId });

    expect(result.code).toBe(0);
    expect(result.stdout).toContain('PASS');
    expect(result.stdout).toContain(taskId);
  });

  it('对缺字段任务输出 FAIL 且退出非零', async () => {
    const taskId = `aria-${new Date().toISOString().slice(0, 10).replaceAll('-', '')}-911`;
    await createTaskFixture({ taskId, broken: true });

    const result = await runScript({ taskId });

    expect(result.code).not.toBe(0);
    expect(result.stdout).toContain('FAIL');
    expect(result.stdout).toContain('缺少文件: spec-artifact.md');
    expect(result.stdout.match(/缺少文件: spec-artifact\.md/g)?.length ?? 0).toBe(1);
    expect(result.stdout).not.toContain('文件缺少字段: spec-artifact.md');
  });

  it('未传 --task-id 时自动选择最新任务', async () => {
    const datePrefix = new Date().toISOString().slice(0, 10).replaceAll('-', '');
    const olderTaskId = `aria-${datePrefix}-920`;
    const latestTaskId = `aria-${datePrefix}-921`;
    await createTaskFixture({ taskId: olderTaskId });
    await createTaskFixture({ taskId: latestTaskId });

    const result = await runScriptWithoutTaskId();

    expect(result.code).toBe(0);
    expect(result.stdout).toContain('PASS');
    expect(result.stdout).toContain(`task_id: ${latestTaskId}`);
  });

  it('支持通过 --repo-root 校验外部仓库中的任务', async () => {
    const externalRepoRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-verify-external-root-'));
    const taskId = `aria-${new Date().toISOString().slice(0, 10).replaceAll('-', '')}-922`;
    await createTaskFixture({ taskId, repoRoot: externalRepoRoot });

    const result = await runScript({
      taskId,
      repoRoot: externalRepoRoot
    });

    expect(result.code).toBe(0);
    expect(result.stdout).toContain('PASS');
    expect(result.stdout).toContain(taskId);
  });
});
