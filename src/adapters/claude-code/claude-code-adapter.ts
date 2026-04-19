import { spawn } from 'node:child_process';
import fs from 'node:fs/promises';

export type ClaudeCodeCommandInput = {
  cwd: string;
  promptPath: string;
  promptContent?: string;
};

export type CliExecutionResult = {
  exitCode: number;
  stdout: string;
  stderr: string;
};

export function buildClaudeCodeCommand(input: ClaudeCodeCommandInput): string[] {
  return [
    'claude',
    '--no-session-persistence',
    '-p',
    input.promptContent ?? input.promptPath,
  ];
}

async function runCliCommand(args: string[], cwd: string): Promise<CliExecutionResult> {
  return new Promise((resolve, reject) => {
    const child = spawn(args[0]!, args.slice(1), { cwd });
    let stdout = '';
    let stderr = '';

    child.stdin?.end();

    child.stdout?.on('data', chunk => {
      stdout += chunk.toString();
    });

    child.stderr?.on('data', chunk => {
      stderr += chunk.toString();
    });

    child.on('error', reject);
    child.on('close', exitCode => {
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
  }), input.cwd);
}
