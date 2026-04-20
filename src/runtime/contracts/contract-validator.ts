import fs from 'node:fs/promises';
import path from 'node:path';

import type { DispatchContract, ExecutionContextBundle, ExecResultArtifact } from '../../schemas/runtime-artifact-schema.js';
import { dispatchContractSchema, executionContextBundleSchema, execResultSchema } from '../../schemas/runtime-artifact-schema.js';

export const EXPECTED_SOURCE_CAPABILITIES = ['OpenSpec', 'superpowers'] as const;
export const EXPECTED_BUNDLE_METHODS = ['writing-plans', 'test-driven-development', 'verification-before-completion'] as const;
export const EXPECTED_CONTRACT_METHODS = ['test-driven-development', 'verification-before-completion'] as const;
export const EXPECTED_VERIFICATION_REQUIREMENTS = ['pnpm check', 'pnpm test'] as const;
export const EXPECTED_WORKER_CLI = 'codex' as const;

function sameMembers(actual: string[], expected: readonly string[]): boolean {
  if (actual.length !== expected.length) {
    return false;
  }

  return expected.every(value => actual.includes(value));
}

function containsAll(actual: string[], expected: readonly string[]): boolean {
  return expected.every(value => actual.includes(value));
}

function toConsumedSpecRef(specRef: string): string {
  return path.posix.join('artifacts', path.posix.basename(specRef));
}

export async function validateHandoffFields(input: {
  approved_spec_ref: string | null;
  approved_plan_ref: string | null;
}): Promise<void> {
  if (!input.approved_spec_ref || !input.approved_plan_ref) {
    throw new Error('missing frozen refs');
  }

  await Promise.all([
    fs.access(input.approved_spec_ref),
    fs.access(input.approved_plan_ref)
  ]);
}

export function validateExecutionContextBundle(input: unknown): ExecutionContextBundle {
  const bundle = executionContextBundleSchema.parse(input);

  if (!sameMembers(bundle.source_capabilities, EXPECTED_SOURCE_CAPABILITIES)) {
    throw new Error('缺少合法 execution context bundle 来源能力');
  }

  if (!sameMembers(bundle.required_methods, EXPECTED_BUNDLE_METHODS)) {
    throw new Error('缺少合法 execution context bundle required_methods');
  }

  if (!sameMembers(bundle.verification_requirements, EXPECTED_VERIFICATION_REQUIREMENTS)) {
    throw new Error('缺少合法 execution context bundle verification_requirements');
  }

  return bundle;
}

export function validateDispatchContract(input: unknown): DispatchContract {
  const contract = dispatchContractSchema.parse(input);

  if (contract.worker_cli !== EXPECTED_WORKER_CLI) {
    throw new Error('缺少合法 dispatch contract worker_cli');
  }

  if (!sameMembers(contract.required_methods, EXPECTED_CONTRACT_METHODS)) {
    throw new Error('缺少合法 dispatch contract required_methods');
  }

  if (!sameMembers(contract.verification_requirements, EXPECTED_VERIFICATION_REQUIREMENTS)) {
    throw new Error('缺少合法 dispatch contract 运行要求');
  }

  return contract;
}

export function validateExecResult(
  input: unknown,
  expected?: {
    task_id: string;
    exec_unit_id: string;
    worker_cli: 'codex';
    spec_ref: string;
    required_methods: string[];
  }
): ExecResultArtifact {
  const result = execResultSchema.parse(input);

  if (expected && result.task_id !== expected.task_id) {
    throw new Error(`exec result task_id 不一致: expected=${expected.task_id} actual=${result.task_id}`);
  }

  if (expected && result.exec_unit_id !== expected.exec_unit_id) {
    throw new Error(`exec result exec_unit_id 不一致: expected=${expected.exec_unit_id} actual=${result.exec_unit_id}`);
  }

  if (expected && !result.capabilities_used.includes(expected.worker_cli)) {
    throw new Error(`exec result 未包含预期能力: ${expected.worker_cli}`);
  }

  if (expected && !result.openspec_refs_consumed.includes(toConsumedSpecRef(expected.spec_ref))) {
    throw new Error(`exec result 未包含预期 OpenSpec 消耗引用: ${toConsumedSpecRef(expected.spec_ref)}`);
  }

  if (expected && !containsAll(result.superpowers_refs_consumed, expected.required_methods)) {
    throw new Error('exec result 未覆盖 contract 要求的方法集合');
  }

  return result;
}
