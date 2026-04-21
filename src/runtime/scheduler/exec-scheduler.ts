import { execFile } from 'node:child_process';
import fs from 'node:fs/promises';
import path from 'node:path';
import { promisify } from 'node:util';

import { runCodexCli } from '../../adapters/codex/codex-adapter.js';
import { toConsumedSpecRef } from '../../utils/artifact-refs.js';
import { detectCapabilities } from '../../adapters/capability-detector.js';
import { getTaskArtifactsDir } from '../persistence/paths.js';
import { readState, writeState } from '../persistence/state-repository.js';
import { parseYaml } from '../../utils/yaml.js';
import { stringifyYaml } from '../../utils/yaml.js';
import { nowIso } from '../../utils/time.js';
import { validateDispatchContract, validateExecutionContextBundle, validateExecResult } from '../contracts/contract-validator.js';
import { resolveRetryableBlock } from '../state-machine/recovery-rules.js';
import { resolveWorkspacePath } from '../../utils/workspace.js';

const execFileAsync = promisify(execFile);

function normalizeExecSummary(input: string): string {
  const summary = input.replace(/\s+/g, ' ').trim();
  return summary || 'codex exec completed';
}

function normalizeChangedFile(filePath: string): string {
  return filePath.replaceAll(path.sep, '/').replace(/^\.\//, '');
}

function matchesScopePattern(filePath: string, pattern: string): boolean {
  const normalizedFile = normalizeChangedFile(filePath);
  const normalizedPattern = pattern.replaceAll(path.sep, '/');

  if (normalizedPattern.endsWith('/**')) {
    const prefix = normalizedPattern.slice(0, -3);
    return normalizedFile === prefix || normalizedFile.startsWith(`${prefix}/`);
  }

  return normalizedFile === normalizedPattern;
}

async function collectChangedFiles(cwd: string): Promise<string[]> {
  try {
    const { stdout } = await execFileAsync('git', ['status', '--short', '--untracked-files=all'], { cwd });
    return stdout
      .split('\n')
      .map(line => line.trim())
      .filter(Boolean)
      .map(line => normalizeChangedFile(line.slice(3).trim()))
      .filter(Boolean)
      .filter((value, index, items) => items.indexOf(value) === index);
  } catch {
    return [];
  }
}

function assertChangedFilesWithinScope(changedFiles: string[], scope?: { files_allowed: string[]; files_blocked?: string[] }): void {
  if (!scope) {
    return;
  }

  for (const filePath of changedFiles) {
    const allowed = scope.files_allowed.some(pattern => matchesScopePattern(filePath, pattern));
    if (!allowed) {
      throw new Error(`dispatch contract scope 不允许修改文件: ${filePath}`);
    }

    const blocked = scope.files_blocked?.some(pattern => matchesScopePattern(filePath, pattern)) ?? false;
    if (blocked) {
      throw new Error(`dispatch contract scope 明确禁止修改文件: ${filePath}`);
    }
  }
}

async function readExecSummary(resultPath: string, stdout: string, stderr: string): Promise<string> {
  try {
    const persisted = await fs.readFile(resultPath, 'utf8');
    if (persisted.trim()) {
      return normalizeExecSummary(persisted);
    }
  } catch {
    // ignore missing output-last-message and fallback to process streams
  }

  const streamed = stdout.trim() || stderr.trim();
  return normalizeExecSummary(streamed);
}

function buildExecPrompt(input: {
  taskId: string;
  bundle: {
    spec_ref: string;
    plan_ref: string;
    scope_constraints_ref: string;
    required_methods: string[];
    source_capabilities: string[];
    verification_requirements: string[];
  };
  contract: {
    worker_cli: 'codex';
    required_methods: string[];
    verification_requirements: string[];
    goal?: string;
    acceptance_checks?: string[];
    scope?: {
      files_allowed: string[];
      files_blocked?: string[];
    };
  };
}): string {
  return [
    '# Codex Exec Prompt',
    `task_id: ${input.taskId}`,
    `spec_ref: ${input.bundle.spec_ref}`,
    `plan_ref: ${input.bundle.plan_ref}`,
    `scope_constraints_ref: ${input.bundle.scope_constraints_ref}`,
    `source_capabilities: [${input.bundle.source_capabilities.join(', ')}]`,
    `required_methods: [${input.bundle.required_methods.join(', ')}]`,
    `verification_requirements: [${input.bundle.verification_requirements.join(', ')}]`,
    `worker_cli: ${input.contract.worker_cli}`,
    `contract_required_methods: [${input.contract.required_methods.join(', ')}]`,
    `contract_verification_requirements: [${input.contract.verification_requirements.join(', ')}]`,
    `goal: ${input.contract.goal ?? '按 dispatch contract 完成实现'}`,
    `files_allowed: [${input.contract.scope?.files_allowed.join(', ') ?? ''}]`,
    `files_blocked: [${input.contract.scope?.files_blocked?.join(', ') ?? ''}]`,
    `acceptance_checks: [${input.contract.acceptance_checks?.join(', ') ?? ''}]`,
    '',
    '请先读取仓库规则、spec、plan 与 dispatch contract，再开始执行。',
    '仅可在 files_allowed 范围内修改文件，必须避开 files_blocked。',
    '实现时必须遵循 contract_required_methods，并完成 verification_requirements 与 acceptance_checks 中要求的验证。',
    '可以在允许范围内修改文件、运行必要命令，并在完成实现后运行 verification_requirements。',
    '不要输出 YAML，也不要解释流程。',
    `只输出一行中文摘要：已接收任务 ${input.taskId}，已完成实现与验证。`,
  ].join('\n');
}

export async function runSingleExecUnit(taskId: string): Promise<void> {
  const capabilities = detectCapabilities();
  if (!capabilities.openspec.available || !capabilities.superpowers.available || !capabilities.codex.available) {
    throw new Error('capability_blocked');
  }

  const state = await readState(taskId);
  const execUnit = state.exec_units['exec-01'];
  if (state.status !== 'dispatched' || !execUnit) {
    throw new Error(`任务不在可执行状态: ${taskId}`);
  }

  if (!state.approved_spec_ref || !state.approved_plan_ref || !state.context_bundle_ref || !state.dispatch_contract_ref) {
    throw new Error(`任务缺少执行所需的冻结引用: ${taskId}`);
  }

  const bundle = validateExecutionContextBundle(
    parseYaml(await fs.readFile(state.context_bundle_ref, 'utf8'))
  );
  const contract = validateDispatchContract(
    parseYaml(await fs.readFile(state.dispatch_contract_ref, 'utf8'))
  );

  if (bundle.spec_ref !== state.approved_spec_ref || bundle.plan_ref !== state.approved_plan_ref) {
    throw new Error('execution context bundle 与冻结引用不一致');
  }

  if (
    contract.based_on_spec_ref !== state.approved_spec_ref ||
    contract.based_on_plan_ref !== state.approved_plan_ref ||
    contract.context_bundle_ref !== state.context_bundle_ref
  ) {
    throw new Error('dispatch contract 与冻结引用或 bundle 不一致');
  }

  const startedAt = nowIso();
  const executionCwd = bundle.workspace_context.repo_path;
  const runningState = {
    ...state,
    status: 'executing' as const,
    active_exec_units: ['exec-01'],
    exec_units: {
      ...state.exec_units,
      'exec-01': {
        ...execUnit,
        status: 'running' as const,
        started_at: startedAt
      }
    },
    updated_at: startedAt
  };
  await writeState(runningState);

  const resultPath = path.join(getTaskArtifactsDir(taskId), 'exec-result-exec-01.yaml');
  const promptPath = path.join(getTaskArtifactsDir(taskId), 'exec-prompt-exec-01.md');
  const resultOutputPath = resolveWorkspacePath(resultPath, executionCwd);
  const promptOutputPath = resolveWorkspacePath(promptPath, executionCwd);
  await fs.mkdir(path.dirname(resultOutputPath), { recursive: true });
  await fs.writeFile(promptOutputPath, buildExecPrompt({
    taskId,
    bundle,
    contract
  }), 'utf8');

  const finishExecAsBlocked = async (input: {
    reasonCode: string;
    execStatus: 'failed' | 'blocked';
    exitCode: number;
  }): Promise<void> => {
    const finishedAt = nowIso();
    const retryResolution = resolveRetryableBlock(input.reasonCode);

    await writeState({
      ...runningState,
      status: 'blocked',
      active_exec_units: [],
      block_reason_code: input.reasonCode,
      blocking_stage: 'executing',
      retryable: retryResolution.retryable,
      required_action: retryResolution.required_action,
      exec_units: {
        ...runningState.exec_units,
        'exec-01': {
          ...runningState.exec_units['exec-01'],
          status: input.execStatus,
          attempt: runningState.exec_units['exec-01'].attempt + 1,
          exit_code: input.exitCode,
          finished_at: finishedAt
        }
      },
      updated_at: finishedAt
    });
  };

  let execFailedHandled = false;

  try {
    const execOutput = await runCodexCli({
      cwd: executionCwd,
      promptPath: promptOutputPath,
      outputPath: resultOutputPath
    });
    if (execOutput.exitCode !== 0) {
      execFailedHandled = true;
      await finishExecAsBlocked({
        reasonCode: 'execution_blocked',
        execStatus: 'failed',
        exitCode: execOutput.exitCode
      });
      throw new Error(`codex_exec_failed: ${execOutput.stderr}`);
    }

    const finishedAt = nowIso();
    const changedFiles = await collectChangedFiles(executionCwd);
    assertChangedFilesWithinScope(changedFiles, contract.scope);
    const execResult = {
      task_id: taskId,
      exec_unit_id: 'exec-01' as const,
      status: 'succeeded' as const,
      changed_files: changedFiles,
      summary: await readExecSummary(resultOutputPath, execOutput.stdout, execOutput.stderr),
      capabilities_used: [contract.worker_cli],
      openspec_refs_consumed: [toConsumedSpecRef(bundle.spec_ref)],
      superpowers_refs_consumed: [...contract.required_methods],
      degraded: false,
      degradation_reason: null,
      started_at: startedAt,
      finished_at: finishedAt
    };
    await fs.writeFile(resultOutputPath, stringifyYaml(execResult), 'utf8');

    const validatedResult = validateExecResult(parseYaml(await fs.readFile(resultOutputPath, 'utf8')), {
      task_id: taskId,
      exec_unit_id: 'exec-01',
      worker_cli: contract.worker_cli,
      spec_ref: bundle.spec_ref,
      required_methods: contract.required_methods
    });

    await writeState({
      ...runningState,
      status: 'reviewing/testing',
      review_status: 'pending',
      test_status: 'pending',
      active_result_set_id: `result-set-${taskId}-01`,
      active_exec_units: [],
      exec_units: {
        ...runningState.exec_units,
        'exec-01': {
          ...runningState.exec_units['exec-01'],
          status: 'succeeded',
          attempt: runningState.exec_units['exec-01'].attempt + 1,
          exit_code: 0,
          result_path: resultPath,
          finished_at: validatedResult.finished_at
        }
      },
      updated_at: validatedResult.finished_at
    });
  } catch (error) {
    const latestState = await readState(taskId);
    if (!execFailedHandled && latestState.exec_units['exec-01']?.status === 'running') {
      await finishExecAsBlocked({
        reasonCode: 'execution_blocked',
        execStatus: 'blocked',
        exitCode: 1
      });
    }
    throw error;
  }
}
