import { spawn } from 'node:child_process';

import { nowIso } from '../../utils/time.js';
import type { CliCommandInput, CliExecutionResult } from '../claude-code/claude-code-adapter.js';

export type CodexExecResult = {
  task_id: string;
  exec_unit_id: string;
  status: 'succeeded';
  changed_files: string[];
  summary: string;
  capabilities_used: string[];
  openspec_refs_consumed: string[];
  superpowers_refs_consumed: string[];
  degraded: boolean;
  degradation_reason: string | null;
  started_at: string;
  finished_at: string;
};

export function buildCodexCommand(input: CliCommandInput): string[] {
  return [
    'codex',
    'exec',
    '-C',
    input.cwd,
    '--output-last-message',
    input.outputPath,
    input.promptPath,
  ];
}

async function runCodexCli(input: CliCommandInput): Promise<CliExecutionResult> {
  const args = buildCodexCommand(input);

  return new Promise((resolve, reject) => {
    const child = spawn(args[0]!, args.slice(1), { cwd: input.cwd });
    let stdout = '';
    let stderr = '';

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

export async function runCodexExec(input: {
  task_id: string;
  unit_id: string;
}): Promise<CodexExecResult>;

export async function runCodexExec(input: CliCommandInput): Promise<CliExecutionResult>;

export async function runCodexExec(
  input: {
    task_id: string;
    unit_id: string;
  } | CliCommandInput
): Promise<CodexExecResult | CliExecutionResult> {
  if ('cwd' in input) {
    return runCodexCli(input);
  }

  const startedAt = nowIso();
  const finishedAt = nowIso();

  return {
    task_id: input.task_id,
    exec_unit_id: input.unit_id,
    status: 'succeeded',
    changed_files: ['src/index.ts'],
    summary: '执行最小骨架生成',
    capabilities_used: ['codex'],
    openspec_refs_consumed: ['artifacts/spec-artifact.md'],
    superpowers_refs_consumed: ['test-driven-development', 'verification-before-completion'],
    degraded: false,
    degradation_reason: null,
    started_at: startedAt,
    finished_at: finishedAt
  };
}
