import fs from 'node:fs/promises';
import path from 'node:path';

import { runCodexCli } from '../../adapters/codex/codex-adapter.js';
import { detectCapabilities } from '../../adapters/capability-detector.js';
import { getTaskArtifactsDir } from '../persistence/paths.js';
import { readState, writeState } from '../persistence/state-repository.js';
import { parseYaml } from '../../utils/yaml.js';
import { stringifyYaml } from '../../utils/yaml.js';
import { nowIso } from '../../utils/time.js';
import { validateDispatchContract, validateExecutionContextBundle, validateExecResult } from '../contracts/contract-validator.js';
import { resolveRetryableBlock } from '../state-machine/recovery-rules.js';

function toConsumedSpecRef(specRef: string): string {
  return path.posix.join('artifacts', path.posix.basename(specRef));
}

function normalizeExecSummary(input: string): string {
  const summary = input.replace(/\s+/g, ' ').trim();
  return summary || 'codex exec completed';
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
    '',
    '不要读取仓库规则与文件，不要执行命令，不要修改任何文件。',
    '不要输出 YAML，也不要解释流程。',
    `只输出一行中文摘要：已接收任务 ${input.taskId}，将按 dispatch contract 执行。`,
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
  await fs.mkdir(path.dirname(resultPath), { recursive: true });
  await fs.writeFile(promptPath, buildExecPrompt({
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

  try {
    const execOutput = await runCodexCli({
      cwd: process.cwd(),
      promptPath,
      outputPath: resultPath
    });
    if (execOutput.exitCode !== 0) {
      await finishExecAsBlocked({
        reasonCode: 'execution_blocked',
        execStatus: 'failed',
        exitCode: execOutput.exitCode
      });
      throw new Error(`codex_exec_failed: ${execOutput.stderr}`);
    }

    const finishedAt = nowIso();
    const execResult = {
      task_id: taskId,
      exec_unit_id: 'exec-01' as const,
      status: 'succeeded' as const,
      changed_files: [],
      summary: await readExecSummary(resultPath, execOutput.stdout, execOutput.stderr),
      capabilities_used: [contract.worker_cli],
      openspec_refs_consumed: [toConsumedSpecRef(bundle.spec_ref)],
      superpowers_refs_consumed: [...contract.required_methods],
      degraded: false,
      degradation_reason: null,
      started_at: startedAt,
      finished_at: finishedAt
    };
    await fs.writeFile(resultPath, stringifyYaml(execResult), 'utf8');

    const validatedResult = validateExecResult(parseYaml(await fs.readFile(resultPath, 'utf8')), {
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
    if (latestState.exec_units['exec-01']?.status === 'running') {
      await finishExecAsBlocked({
        reasonCode: 'execution_blocked',
        execStatus: 'blocked',
        exitCode: 1
      });
    }
    throw error;
  }
}
