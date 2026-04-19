import { describe, expect, it } from 'vitest';

import {
  buildClaudeCodeCommand,
  buildCodexCommand,
} from '../../../src/adapters/claude-code/claude-code-adapter.js';

describe('cli adapters', () => {
  it('为 claude code 构造带工作目录和输入文件的命令', () => {
    expect(buildClaudeCodeCommand({
      cwd: '/tmp/task-1',
      promptPath: 'cadence/cache/aria/tasks/task-1/artifacts/spec-prompt.md',
      outputPath: 'cadence/cache/aria/tasks/task-1/artifacts/spec-artifact.md',
    })).toEqual([
      'claude',
      'code',
      '--cwd',
      '/tmp/task-1',
      '--input',
      'cadence/cache/aria/tasks/task-1/artifacts/spec-prompt.md',
      '--output',
      'cadence/cache/aria/tasks/task-1/artifacts/spec-artifact.md',
    ]);
  });

  it('为 codex 构造带 contract prompt 的命令', () => {
    expect(buildCodexCommand({
      cwd: '/tmp/task-1',
      promptPath: 'cadence/cache/aria/tasks/task-1/artifacts/dispatch-prompt.md',
      outputPath: 'cadence/cache/aria/tasks/task-1/artifacts/exec-result-exec-01.yaml',
    })).toEqual([
      'codex',
      '--cwd',
      '/tmp/task-1',
      '--input',
      'cadence/cache/aria/tasks/task-1/artifacts/dispatch-prompt.md',
      '--output',
      'cadence/cache/aria/tasks/task-1/artifacts/exec-result-exec-01.yaml',
    ]);
  });
});
