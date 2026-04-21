import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';

import type { CliExecutionResult } from '../claude-code/claude-code-adapter.js';

export type CodexCommandInput = {
  cwd: string;
  promptPath: string;
  outputPath: string;
  promptContent?: string;
  timeoutMs?: number;
};

export function buildCodexCommand(input: CodexCommandInput): string[] {
  return [
    process.env.CADENCE_CODEX_BIN ?? 'codex',
    'exec',
    '--full-auto',
    '--ephemeral',
    '-C',
    input.cwd,
    '--output-last-message',
    input.outputPath,
    input.promptContent ?? input.promptPath,
  ];
}

const DEFAULT_TIMEOUT_MS = 30 * 60 * 1000;

export async function runCodexCli(input: CodexCommandInput): Promise<CliExecutionResult> {
  const promptContent = input.promptContent ?? await fs.readFile(input.promptPath, 'utf8');
  const args = buildCodexCommand({
    ...input,
    promptContent
  });

  if (args.length === 0) {
    throw new Error('spawn args cannot be empty');
  }

  return new Promise((resolve, reject) => {
    const child = spawn(args[0], args.slice(1), { cwd: input.cwd });
    let stdout = '';
    let stderr = '';

    const timeout = setTimeout(() => {
      child.kill('SIGTERM');
      reject(new Error('codex_exec_timed_out'));
    }, input.timeoutMs ?? DEFAULT_TIMEOUT_MS);

    child.stdin?.end();

    child.stdout?.on('data', chunk => {
      stdout += chunk.toString();
    });

    child.stderr?.on('data', chunk => {
      stderr += chunk.toString();
    });

    child.on('error', err => {
      clearTimeout(timeout);
      reject(err);
    });
    child.on('close', exitCode => {
      clearTimeout(timeout);
      resolve({
        exitCode: exitCode ?? 1,
        stdout,
        stderr,
      });
    });
  });
}
