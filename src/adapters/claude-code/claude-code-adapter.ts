import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';

export type ClaudeCodeCommandInput = {
  cwd: string;
  promptPath: string;
  promptContent?: string;
  timeoutMs?: number;
};

export type CliExecutionResult = {
  exitCode: number;
  stdout: string;
  stderr: string;
};

export function buildClaudeCodeCommand(input: ClaudeCodeCommandInput): string[] {
  return [
    process.env.CADENCE_CLAUDE_BIN ?? 'claude',
    '--no-session-persistence',
    '-p',
    input.promptContent ?? input.promptPath,
  ];
}

const DEFAULT_TIMEOUT_MS = 30 * 60 * 1000;

async function runCliCommand(args: string[], cwd: string, timeoutMs = DEFAULT_TIMEOUT_MS): Promise<CliExecutionResult> {
  if (args.length === 0) {
    throw new Error('spawn args cannot be empty');
  }
  return new Promise((resolve, reject) => {
    const child = spawn(args[0], args.slice(1), { cwd });
    let stdout = '';
    let stderr = '';

    const timeout = setTimeout(() => {
      child.kill('SIGTERM');
      reject(new Error('claude_code_timed_out'));
    }, timeoutMs);

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

export async function runClaudeCode(input: ClaudeCodeCommandInput): Promise<CliExecutionResult> {
  const promptContent = input.promptContent ?? await fs.readFile(input.promptPath, 'utf8');
  return runCliCommand(buildClaudeCodeCommand({
    ...input,
    promptContent
  }), input.cwd, input.timeoutMs);
}
