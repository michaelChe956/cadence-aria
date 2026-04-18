import { describe, expect, it } from 'vitest';
import { dispatchContractSchema, executionContextBundleSchema } from '../../../src/schemas/runtime-artifact-schema.js';

describe('runtime-artifact-schema', () => {
  it('校验 dispatchContractSchema 的 happy path', () => {
    const contract = dispatchContractSchema.parse({
      contract_version: '1.0',
      generated_at: '2026-04-18T00:00:00.000Z',
      base_revision: 'abc123',
      input_artifacts: {
        'spec.md': 'cadence/cache/aria/tasks/aria-20260418-001/spec.md'
      },
      generated_from_plan: 'plan-aria-20260418-001',
      source_task_refs: ['aria-20260418-001'],
      task_id: 'aria-20260418-001',
      timeout_minutes: 30,
      based_on_spec_ref: 'spec-ref-1',
      based_on_plan_ref: 'plan-ref-1',
      context_bundle_ref: 'bundle-1',
      output_schema_ref: 'schema-ref-1',
      exec_unit_id: 'exec-01',
      contract_type: 'dispatch',
      parent_task: 'aria-20260418-001',
      mode: 'exec',
      scope: {
        files_allowed: ['src/index.ts'],
        files_blocked: ['src/legacy.ts']
      },
      goal: '更新索引逻辑',
      acceptance_checks: ['测试通过', '类型检查通过'],
      dependencies: [],
      worktree_ref: 'wt-exec-01',
      result_path: 'cadence/cache/aria/tasks/aria-20260418-001/exec-01-result.md',
      retry_allowed: true
    });

    expect(contract.contract_type).toBe('dispatch');
  });

  it('校验 executionContextBundleSchema 的 happy path', () => {
    const bundle = executionContextBundleSchema.parse({
      bundle_id: 'bundle-1',
      spec_ref: 'spec-ref-1',
      plan_ref: 'plan-ref-1',
      scope_constraints_ref: 'scope-constraints-1',
      required_methods: ['tdd', 'verify'],
      workspace_context: {
        repo_path: '/home/michael/workspace/github/cadence-aria',
        worktree_ref: 'feature-aria-phase1-foundation',
        base_revision: 'abc123'
      },
      verification_requirements: ['pnpm check', 'pnpm vitest run tests/unit'],
      prompt_template_ref: 'template-1'
    });

    expect(bundle.bundle_id).toBe('bundle-1');
  });
});
