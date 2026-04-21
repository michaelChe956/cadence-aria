import { describe, expect, it } from 'vitest';
import { buildCodexCommand } from '../../../src/adapters/codex/codex-adapter.js';

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
