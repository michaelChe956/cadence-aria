import { describe, expect, it } from 'vitest';
import { runCli } from '../../../src/commands/run-cli.js';

describe('runCli', () => {
  it('当未提供子命令时返回帮助文案', async () => {
    const output = await runCli([]);
    expect(output).toBe('aria:intake\naria:start\naria:run\naria:status\naria:result');
  });
});
