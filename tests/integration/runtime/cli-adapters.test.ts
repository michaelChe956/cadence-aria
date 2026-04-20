import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, describe, expect, it } from 'vitest';

import {
  buildClaudeCodeCommand,
  runClaudeCode,
} from '../../../src/adapters/claude-code/claude-code-adapter.js';
import { buildCodexCommand, runCodexCli } from '../../../src/adapters/codex/codex-adapter.js';
import { detectCapabilities } from '../../../src/adapters/capability-detector.js';

const ORIGINAL_PATH = process.env.PATH ?? '';
const ORIGINAL_CWD = process.cwd();

let tempDir = '';

afterEach(async () => {
  process.env.PATH = ORIGINAL_PATH;
  delete process.env.CLAUDE_OUTPUT_PATH;
  delete process.env.CADENCE_CODEX_BIN;
  delete process.env.CADENCE_CLAUDE_BIN;
  if (tempDir) {
    await fs.rm(tempDir, { recursive: true, force: true });
    tempDir = '';
  }
  process.chdir(ORIGINAL_CWD);
});

describe('cli adapters', () => {
  it('为 claude code 构造带工作目录和输入文件的命令', () => {
    expect(buildClaudeCodeCommand({
      cwd: '/tmp/task-1',
      promptPath: 'cadence/cache/aria/tasks/task-1/artifacts/spec-prompt.md',
      promptContent: '# Spec\nproducer: claude-code',
    })).toEqual([
      'claude',
      '--no-session-persistence',
      '-p',
      '# Spec\nproducer: claude-code',
    ]);
  });

  it('为 codex 构造带 contract prompt 的命令', () => {
    expect(buildCodexCommand({
      cwd: '/tmp/task-1',
      promptPath: 'cadence/cache/aria/tasks/task-1/artifacts/dispatch-prompt.md',
      outputPath: 'cadence/cache/aria/tasks/task-1/artifacts/exec-result-exec-01.yaml',
      promptContent: '# Dispatch Prompt\ntask_id: aria-20260419-001',
    })).toEqual([
      'codex',
      'exec',
      '--full-auto',
      '--ephemeral',
      '-C',
      '/tmp/task-1',
      '--output-last-message',
      'cadence/cache/aria/tasks/task-1/artifacts/exec-result-exec-01.yaml',
      '# Dispatch Prompt\ntask_id: aria-20260419-001',
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

  it('优先识别 CADENCE_CODEX_BIN 与 CADENCE_CLAUDE_BIN 指向的可执行文件', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-'));
    const codexPath = path.join(tempDir, 'custom-codex');
    const claudePath = path.join(tempDir, 'custom-claude');
    await fs.writeFile(codexPath, '#!/bin/sh\nexit 0\n', 'utf8');
    await fs.writeFile(claudePath, '#!/bin/sh\nexit 0\n', 'utf8');
    await fs.chmod(codexPath, 0o755);
    await fs.chmod(claudePath, 0o755);

    process.env.PATH = '';
    process.env.CADENCE_CODEX_BIN = codexPath;
    process.env.CADENCE_CLAUDE_BIN = claudePath;

    expect(detectCapabilities().codex).toEqual({
      available: true,
      source: codexPath
    });
    expect(detectCapabilities().claude_code).toEqual({
      available: true,
      source: claudePath
    });
  });

  it('当工作区存在同名 codex 文件但 PATH 未命中时仍判为不可用', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-'));
    const workspaceBinary = path.join(tempDir, 'codex');
    const isolatedPathDir = path.join(tempDir, 'bin');
    await fs.mkdir(isolatedPathDir);
    await fs.writeFile(workspaceBinary, '#!/bin/sh\nexit 0\n', 'utf8');
    await fs.chmod(workspaceBinary, 0o755);
    process.chdir(tempDir);
    process.env.PATH = isolatedPathDir;

    expect(detectCapabilities().codex).toEqual({
      available: false,
      source: 'codex'
    });
  });

  it('当 PATH 含空片段时允许在当前目录命中 codex', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-'));
    const workspaceBinary = path.join(tempDir, 'codex');
    await fs.writeFile(workspaceBinary, '#!/bin/sh\nexit 0\n', 'utf8');
    await fs.chmod(workspaceBinary, 0o755);
    process.chdir(tempDir);
    process.env.PATH = ':';

    expect(detectCapabilities().codex).toEqual({
      available: true,
      source: 'codex'
    });
  });

  it('只在 Windows 语义下把 codex.cmd 视为 launcher', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-'));
    const binaryPath = path.join(tempDir, 'codex.cmd');
    await fs.writeFile(binaryPath, '@echo off\r\nexit /b 0\r\n', 'utf8');
    await fs.chmod(binaryPath, 0o755);
    process.env.PATH = tempDir;

    if (process.platform === 'win32') {
      expect(detectCapabilities().codex).toEqual({
        available: true,
        source: binaryPath
      });
      return;
    }

    expect(detectCapabilities().codex).toEqual({
      available: false,
      source: 'codex'
    });
  });

  it('运行 codex CLI 时会主动关闭 stdin，避免真实进程等待额外输入', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-'));
    const binaryPath = path.join(tempDir, 'codex');
    const promptPath = path.join(tempDir, 'prompt.md');
    const outputPath = path.join(tempDir, 'output.txt');
    const script = String.raw`#!/usr/bin/env node
const fs = require('node:fs');

const args = process.argv.slice(2);
let outputPath = '';
for (let index = 0; index < args.length; index += 1) {
  if (args[index] === '--output-last-message') {
    outputPath = args[index + 1] ?? '';
    index += 1;
  }
}

process.stdin.resume();
process.stdin.on('end', () => {
  fs.writeFileSync(outputPath, 'stdin closed', 'utf8');
  process.exit(0);
});
`;
    await fs.writeFile(binaryPath, script, 'utf8');
    await fs.chmod(binaryPath, 0o755);
    await fs.writeFile(promptPath, 'prompt', 'utf8');
    process.env.PATH = `${tempDir}${path.delimiter}${ORIGINAL_PATH}`;

    const result = await runCodexCli({
      cwd: tempDir,
      promptPath,
      outputPath
    });

    expect(result.exitCode).toBe(0);
    await expect(fs.readFile(outputPath, 'utf8')).resolves.toBe('stdin closed');
  });

  it('运行 claude code 时会主动关闭 stdin，避免真实进程等待额外输入', async () => {
    tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'cadence-aria-cli-'));
    const binaryPath = path.join(tempDir, 'claude');
    const promptPath = path.join(tempDir, 'prompt.md');
    const outputPath = path.join(tempDir, 'claude-output.txt');
    const script = String.raw`#!/usr/bin/env node
const fs = require('node:fs');

process.stdin.resume();
process.stdin.on('end', () => {
  fs.writeFileSync(process.env.CLAUDE_OUTPUT_PATH, 'stdin closed', 'utf8');
  process.exit(0);
});
`;
    await fs.writeFile(binaryPath, script, 'utf8');
    await fs.chmod(binaryPath, 0o755);
    await fs.writeFile(promptPath, 'prompt', 'utf8');
    process.env.PATH = `${tempDir}${path.delimiter}${ORIGINAL_PATH}`;
    process.env.CLAUDE_OUTPUT_PATH = outputPath;

    const result = await runClaudeCode({
      cwd: tempDir,
      promptPath
    });

    expect(result.exitCode).toBe(0);
    await expect(fs.readFile(outputPath, 'utf8')).resolves.toBe('stdin closed');
  });
});
