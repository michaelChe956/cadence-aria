import { describe, expect, it } from 'vitest';
import { buildCodexCommand, runLegacyCodexExec, runCodexExec } from '../../../src/adapters/codex/codex-adapter.js';

describe('buildCodexCommand', () => {
  it('使用 promptContent 构建命令', () => {
    const args = buildCodexCommand({
      cwd: '/project',
      promptPath: '/project/prompt.txt',
      outputPath: '/project/output.md',
      promptContent: '实现功能'
    });
    expect(args[0]).toBe('codex');
    expect(args).toContain('exec');
    expect(args).toContain('--full-auto');
    expect(args).toContain('实现功能');
    expect(args).toContain('/project/output.md');
  });

  it('使用 promptPath 作为 fallback', () => {
    const args = buildCodexCommand({
      cwd: '/project',
      promptPath: '/project/prompt.txt',
      outputPath: '/project/output.md'
    });
    expect(args).toContain('/project/prompt.txt');
  });

  it('支持 CADENCE_CODEX_BIN 环境变量', () => {
    const originalBin = process.env.CADENCE_CODEX_BIN;
    process.env.CADENCE_CODEX_BIN = '/custom/codex';
    const args = buildCodexCommand({
      cwd: '/project',
      promptPath: '/project/prompt.txt',
      outputPath: '/project/output.md'
    });
    expect(args[0]).toBe('/custom/codex');
    process.env.CADENCE_CODEX_BIN = originalBin;
  });
});

describe('runLegacyCodexExec', () => {
  it('返回成功的执行结果', async () => {
    const result = await runLegacyCodexExec({
      task_id: 'task-001',
      unit_id: 'exec-01'
    });
    expect(result.task_id).toBe('task-001');
    expect(result.exec_unit_id).toBe('exec-01');
    expect(result.status).toBe('succeeded');
    expect(result.changed_files.length).toBeGreaterThan(0);
    expect(result.degraded).toBe(false);
    expect(result.degradation_reason).toBeNull();
  });

  it('包含 capabilities_used 和 refs', async () => {
    const result = await runLegacyCodexExec({
      task_id: 'task-001',
      unit_id: 'exec-01'
    });
    expect(result.capabilities_used).toContain('codex');
    expect(result.openspec_refs_consumed.length).toBeGreaterThan(0);
    expect(result.superpowers_refs_consumed.length).toBeGreaterThan(0);
  });
});

describe('runCodexExec', () => {
  it('委托给 runLegacyCodexExec', async () => {
    const result = await runCodexExec({
      task_id: 'task-002',
      unit_id: 'exec-01'
    });
    expect(result.task_id).toBe('task-002');
    expect(result.status).toBe('succeeded');
  });
});
