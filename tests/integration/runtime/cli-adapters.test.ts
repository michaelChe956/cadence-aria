import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import {
  buildClaudeCodeCommand,
} from '../../../src/adapters/claude-code/claude-code-adapter.js';
import { buildCodexCommand } from '../../../src/adapters/codex/codex-adapter.js';
import { detectCapabilities } from '../../../src/adapters/capability-detector.js';

const ORIGINAL_PATH = process.env.PATH ?? '';

let tempDir = '';

afterEach(async () => {
  process.env.PATH = ORIGINAL_PATH;
  if (tempDir) {
    await fs.rm(tempDir, { recursive: true, force: true });
    tempDir = '';
  }
});

describe('cli adapters', () => {
  it('为 claude code 构造带工作目录和输入文件的命令', () => {
    expect(buildClaudeCodeCommand({
      cwd: '/tmp/task-1',
      promptPath: 'cadence/cache/aria/tasks/task-1/artifacts/spec-prompt.md',
    })).toEqual([
      'claude',
      '-p',
      'cadence/cache/aria/tasks/task-1/artifacts/spec-prompt.md',
    ]);
  });

  it('为 codex 构造带 contract prompt 的命令', () => {
    expect(buildCodexCommand({
      cwd: '/tmp/task-1',
      promptPath: 'cadence/cache/aria/tasks/task-1/artifacts/dispatch-prompt.md',
      outputPath: 'cadence/cache/aria/tasks/task-1/artifacts/exec-result-exec-01.yaml',
    })).toEqual([
      'codex',
      'exec',
      '-C',
      '/tmp/task-1',
      '--output-last-message',
      'cadence/cache/aria/tasks/task-1/artifacts/exec-result-exec-01.yaml',
      'cadence/cache/aria/tasks/task-1/artifacts/dispatch-prompt.md',
    ]);
  });

  it('将存在但不可执行的 codex 二进制视为不可用', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-'));
    const binaryPath = path.join(tempDir, 'codex');
    await fs.writeFile(binaryPath, '#!/bin/sh\nexit 0\n', 'utf8');
    await fs.chmod(binaryPath, 0o644);
    process.env.PATH = tempDir;

    expect(detectCapabilities().codex).toEqual({
      available: false,
      source: 'codex'
    });
  });

  it('将 codex.cmd 视为可用 launcher', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-'));
    const binaryPath = path.join(tempDir, 'codex.cmd');
    await fs.writeFile(binaryPath, '@echo off\r\nexit /b 0\r\n', 'utf8');
    await fs.chmod(binaryPath, 0o755);
    process.env.PATH = tempDir;

    expect(detectCapabilities().codex).toEqual({
      available: true,
      source: binaryPath
    });
  });
});
