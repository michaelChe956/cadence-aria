import fs from 'node:fs/promises';
import path from 'node:path';

import type { DispatchContract } from '../../schemas/runtime-artifact-schema.js';
import { stringifyYaml } from '../../utils/yaml.js';
import { getTaskArtifactsDir } from '../persistence/paths.js';
import { nowIso } from '../../utils/time.js';

import { buildExecutionContextBundle } from './execution-context-bundle.js';
import { validateDispatchContract, validateExecutionContextBundle, validateHandoffFields } from './contract-validator.js';

function buildDispatchContract(input: {
  task_id: string;
  approved_spec_ref: string;
  approved_plan_ref: string;
  context_bundle_ref: string;
  goal?: string;
}): DispatchContract {
  return {
    contract_version: '1.0',
    generated_at: nowIso(),
    base_revision: process.env.CADENCE_BASE_REVISION ?? 'unknown',
    input_artifacts: {
      approved_spec_ref: input.approved_spec_ref,
      approved_plan_ref: input.approved_plan_ref
    },
    generated_from_plan: input.approved_plan_ref,
    source_task_refs: [input.task_id],
    task_id: input.task_id,
    timeout_minutes: 30,
    based_on_spec_ref: input.approved_spec_ref,
    based_on_plan_ref: input.approved_plan_ref,
    context_bundle_ref: input.context_bundle_ref,
    output_schema_ref: 'src/schemas/runtime-artifact-schema.ts',
    exec_unit_id: 'exec-01',
    worker_cli: 'codex',
    required_methods: ['test-driven-development', 'verification-before-completion'],
    verification_requirements: ['pnpm check', 'pnpm test'],
    contract_type: 'dispatch',
    parent_task: input.task_id,
    mode: 'exec',
    scope: {
      files_allowed: ['src/**', 'tests/**', 'cadence/cache/aria/**'],
      files_blocked: ['cadence/designs/**', '.claude/**']
    },
    goal: input.goal ?? '按 dispatch contract 完成实现',
    acceptance_checks: ['pnpm check', 'pnpm test'],
    dependencies: [],
    result_path: path.posix.join(getTaskArtifactsDir(input.task_id), 'exec-result-exec-01.yaml'),
    retry_allowed: false,
    worktree_ref: process.env.CADENCE_WORKTREE_REF ?? path.basename(process.cwd())
  };
}

export async function createDispatchArtifacts(input: {
  task_id: string;
  approved_spec_ref: string;
  approved_plan_ref: string;
  goal?: string;
}): Promise<{ context_bundle_ref: string; dispatch_contract_ref: string }> {
  await validateHandoffFields({
    approved_spec_ref: input.approved_spec_ref,
    approved_plan_ref: input.approved_plan_ref
  });

  const artifactsDir = getTaskArtifactsDir(input.task_id);
  const context_bundle_ref = path.posix.join(artifactsDir, 'execution-context-bundle.yaml');
  const dispatch_contract_ref = path.posix.join(artifactsDir, 'dispatch-contract-exec-01.yaml');

  const bundle = validateExecutionContextBundle(
    buildExecutionContextBundle({
      task_id: input.task_id,
      spec_ref: input.approved_spec_ref,
      plan_ref: input.approved_plan_ref,
      scope_constraints_ref: input.approved_plan_ref
    })
  );
  const contract = validateDispatchContract(
    buildDispatchContract({
      task_id: input.task_id,
      approved_spec_ref: input.approved_spec_ref,
      approved_plan_ref: input.approved_plan_ref,
      context_bundle_ref,
      goal: input.goal
    })
  );

  await fs.mkdir(artifactsDir, { recursive: true });
  await fs.writeFile(context_bundle_ref, stringifyYaml(bundle), 'utf8');
  await fs.writeFile(dispatch_contract_ref, stringifyYaml(contract), 'utf8');

  return { context_bundle_ref, dispatch_contract_ref };
}
