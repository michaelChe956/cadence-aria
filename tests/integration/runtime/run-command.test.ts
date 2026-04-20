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
import { parseYaml, stringifyYaml } from '../../../src/utils/yaml.js';
import { createFakeBinaries, cleanupFakeBinaries } from '../../fixtures/fake-binaries.js';

const ORIGINAL_CWD = process.cwd();
const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';

async function setTempWorkspace(): Promise<void> {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-run-command-'));
  process.chdir(tempDir);
  await createFakeBinaries({ writeArtifactToDisk: true }, ORIGINAL_PATH);
}

async function restoreWorkspace(): Promise<void> {
  process.chdir(ORIGINAL_CWD);
  process.env.PATH = ORIGINAL_PATH;
  delete process.env.CADENCE_CODEX_BIN;
  delete process.env.CADENCE_CLAUDE_BIN;
  delete process.env.ARIA_FAKE_ARTIFACT_ROOT;
  delete process.env.ARIA_FAKE_CODEX_FAIL;
  delete process.env.ARIA_FAKE_CODEX_CWD_LOG;
  delete process.env.ARIA_FAKE_CLAUDE_MODE;
  delete process.env.ARIA_FAKE_CLAUDE_CWD_LOG;
  delete process.env.ARIA_FAKE_CLAUDE_DEBUG_LOG;
  await cleanupFakeBinaries();
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

  it('当执行前 capability 检查失败时，也会收尾为 blocked 并标记可重试', async () => {
    const intake = await intakeCommand('验证执行前 capability 失败收尾');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    process.env.PATH = '';

    await expect(runCommand(taskId)).rejects.toThrow(/capability_blocked/);

    const state = await readState(taskId);
    expect(state.status).toBe('blocked');
    expect(state.block_reason_code).toBe('capability_blocked');
    expect(state.blocking_stage).toBe('executing');
    expect(state.retryable).toBe(true);
    expect(state.required_action).toContain('aria:retry');
    expect(state.active_exec_units).toEqual([]);
    expect(state.exec_units['exec-01']?.status).toBe('pending');
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

  it('exec 与 review/test 都使用 execution context bundle 中的 repo_path 作为工作目录', async () => {
    const intake = await intakeCommand('验证运行阶段使用 bundle repo_path');
    const taskId = intake.match(/task_id: (aria-\d{8}-\d{3})/)?.[1] ?? '';

    await startCommand(taskId);
    await confirmSpecCommand(taskId);
    await confirmPlanCommand(taskId);

    const alternateRepoPath = await fs.mkdtemp(path.join(tempDir, 'alternate-repo-'));
    const bundlePath = path.join(getTaskArtifactsDir(taskId), 'execution-context-bundle.yaml');
    const bundle = parseYaml(await fs.readFile(bundlePath, 'utf8')) as {
      workspace_context: {
        repo_path: string;
        worktree_ref: string;
        base_revision: string;
      };
    };
    bundle.workspace_context.repo_path = alternateRepoPath;
    await fs.writeFile(bundlePath, stringifyYaml(bundle), 'utf8');

    const codexCwdLog = path.join(tempDir, 'codex-cwd.log');
    const claudeCwdLog = path.join(tempDir, 'claude-cwd.log');
    process.env.ARIA_FAKE_CODEX_CWD_LOG = codexCwdLog;
    process.env.ARIA_FAKE_CLAUDE_CWD_LOG = claudeCwdLog;

    await runCommand(taskId);

    await expect(fs.readFile(codexCwdLog, 'utf8')).resolves.toBe(`${alternateRepoPath}\n`);
    await expect(fs.readFile(claudeCwdLog, 'utf8')).resolves.toBe(`${alternateRepoPath}\n${alternateRepoPath}\n`);
  });
});
