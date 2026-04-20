import fs from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';

import { afterEach, beforeEach, describe, expect, it } from 'vitest';

import {
  validateDispatchContract,
  validateExecResult,
  validateExecutionContextBundle,
  validateHandoffFields,
  validateReviewReport,
  validateTestReport
} from '../../../../src/runtime/contracts/contract-validator.js';

let tempDir = '';

beforeEach(async () => {
  tempDir = await fs.mkdtemp(path.join(os.tmpdir(), 'aria-contract-validator-'));
});

afterEach(async () => {
  if (tempDir) {
    await fs.rm(tempDir, { recursive: true, force: true });
    tempDir = '';
  }
});

function createBundle(overrides: Record<string, unknown> = {}) {
  return {
    bundle_id: 'execution-context-bundle-aria-20260419-001',
    spec_ref: 'cadence/cache/aria/tasks/aria-20260419-001/artifacts/spec-artifact.md',
    plan_ref: 'cadence/cache/aria/tasks/aria-20260419-001/artifacts/plan-brief.md',
    scope_constraints_ref: 'cadence/cache/aria/tasks/aria-20260419-001/artifacts/plan-brief.md',
    required_methods: ['writing-plans', 'test-driven-development', 'verification-before-completion'],
    source_capabilities: ['OpenSpec', 'superpowers'],
    workspace_context: {
      repo_path: '/tmp/worktree',
      worktree_ref: 'feature-aria',
      base_revision: 'unknown'
    },
    verification_requirements: ['pnpm check', 'pnpm test'],
    prompt_template_ref: 'codex/prompts/dispatch.md',
    ...overrides
  };
}

function createDispatchContract(overrides: Record<string, unknown> = {}) {
  return {
    contract_version: '1.0',
    generated_at: '2026-04-19T00:00:00.000Z',
    base_revision: 'unknown',
    input_artifacts: {
      approved_spec_ref: 'artifacts/spec-artifact.md',
      approved_plan_ref: 'artifacts/plan-brief.md'
    },
    generated_from_plan: 'artifacts/plan-brief.md',
    source_task_refs: ['aria-20260419-001'],
    task_id: 'aria-20260419-001',
    timeout_minutes: 30,
    based_on_spec_ref: 'artifacts/spec-artifact.md',
    based_on_plan_ref: 'artifacts/plan-brief.md',
    context_bundle_ref: 'artifacts/execution-context-bundle.yaml',
    output_schema_ref: 'src/schemas/runtime-artifact-schema.ts',
    exec_unit_id: 'exec-01',
    worker_cli: 'codex',
    required_methods: ['test-driven-development', 'verification-before-completion'],
    verification_requirements: ['pnpm check', 'pnpm test'],
    contract_type: 'dispatch',
    parent_task: 'aria-20260419-001',
    mode: 'exec',
    scope: {
      files_allowed: ['src/**'],
      files_blocked: ['.claude/**']
    },
    goal: '实现一期 formal flow 最小闭环',
    acceptance_checks: ['pnpm check', 'pnpm test'],
    dependencies: [],
    worktree_ref: 'feature-aria',
    result_path: 'artifacts/exec-result-exec-01.yaml',
    retry_allowed: false,
    ...overrides
  };
}

function createExecResult(overrides: Record<string, unknown> = {}) {
  return {
    task_id: 'aria-20260419-001',
    exec_unit_id: 'exec-01',
    status: 'succeeded',
    changed_files: ['src/index.ts'],
    summary: 'fake codex exec',
    capabilities_used: ['codex'],
    openspec_refs_consumed: ['artifacts/spec-artifact.md'],
    superpowers_refs_consumed: ['test-driven-development', 'verification-before-completion'],
    degraded: false,
    degradation_reason: null,
    started_at: '2026-04-19T00:00:00.000Z',
    finished_at: '2026-04-19T00:00:01.000Z',
    ...overrides
  };
}

function createReviewReport(overrides: Record<string, unknown> = {}) {
  return {
    task_id: 'aria-20260419-001',
    result_set_id: 'result-set-aria-20260419-001-01',
    exec_units_reviewed: ['exec-01'],
    baseline_refs: ['artifacts/spec-artifact.md', 'artifacts/plan-brief.md'],
    method_refs: ['verification-before-completion'],
    blockers: [],
    suggestions: [],
    verdict: 'passed',
    producer: 'claude-code',
    source_capabilities: ['OpenSpec', 'superpowers'],
    generated_at: '2026-04-19T00:00:02.000Z',
    ...overrides
  };
}

function createTestReport(overrides: Record<string, unknown> = {}) {
  return {
    task_id: 'aria-20260419-001',
    result_set_id: 'result-set-aria-20260419-001-01',
    exec_units_tested: ['exec-01'],
    baseline_refs: ['artifacts/spec-artifact.md', 'artifacts/plan-brief.md'],
    method_refs: ['test-driven-development', 'verification-before-completion'],
    commands_run: ['pnpm check', 'pnpm test'],
    failures: [],
    passed_count: 2,
    failed_count: 0,
    verdict: 'passed',
    producer: 'claude-code',
    source_capabilities: ['OpenSpec', 'superpowers'],
    generated_at: '2026-04-19T00:00:03.000Z',
    ...overrides
  };
}

describe('contract-validator', () => {
  it('validateHandoffFields 在缺少冻结引用时失败', async () => {
    await expect(validateHandoffFields({
      approved_spec_ref: null,
      approved_plan_ref: 'artifacts/plan-brief.md'
    })).rejects.toThrow('missing frozen refs');
  });

  it('validateHandoffFields 在文件存在时通过', async () => {
    const specPath = path.join(tempDir, 'spec-artifact.md');
    const planPath = path.join(tempDir, 'plan-brief.md');
    await fs.writeFile(specPath, 'spec', 'utf8');
    await fs.writeFile(planPath, 'plan', 'utf8');

    await expect(validateHandoffFields({
      approved_spec_ref: specPath,
      approved_plan_ref: planPath
    })).resolves.toBeUndefined();
  });

  it('validateExecutionContextBundle 校验来源能力、方法与验证要求', () => {
    expect(() => validateExecutionContextBundle(createBundle({
      source_capabilities: ['OpenSpec']
    }))).toThrow('缺少合法 execution context bundle 来源能力');

    expect(() => validateExecutionContextBundle(createBundle({
      required_methods: ['writing-plans']
    }))).toThrow('缺少合法 execution context bundle required_methods');

    expect(() => validateExecutionContextBundle(createBundle({
      verification_requirements: ['pnpm test']
    }))).toThrow('缺少合法 execution context bundle verification_requirements');

    expect(validateExecutionContextBundle(createBundle()).bundle_id).toContain('execution-context-bundle');
  });

  it('validateDispatchContract 校验 worker_cli、required_methods 与 verification_requirements', () => {
    expect(() => validateDispatchContract(createDispatchContract({
      worker_cli: 'claude-code'
    }))).toThrow('Invalid literal value');

    expect(() => validateDispatchContract(createDispatchContract({
      required_methods: ['verification-before-completion']
    }))).toThrow('缺少合法 dispatch contract required_methods');

    expect(() => validateDispatchContract(createDispatchContract({
      verification_requirements: ['pnpm build']
    }))).toThrow('缺少合法 dispatch contract 运行要求');

    expect(validateDispatchContract(createDispatchContract()).worker_cli).toBe('codex');
  });

  it('validateExecResult 校验 task_id、exec_unit_id、能力和来源消费', () => {
    const expected = {
      task_id: 'aria-20260419-001',
      exec_unit_id: 'exec-01',
      worker_cli: 'codex' as const,
      spec_ref: 'cadence/cache/aria/tasks/aria-20260419-001/artifacts/spec-artifact.md',
      required_methods: ['test-driven-development', 'verification-before-completion']
    };

    expect(() => validateExecResult(createExecResult({
      task_id: 'aria-20260419-999'
    }), expected)).toThrow('exec result task_id 不一致');

    expect(() => validateExecResult(createExecResult({
      exec_unit_id: 'exec-02'
    }), expected)).toThrow('exec result exec_unit_id 不一致');

    expect(() => validateExecResult(createExecResult({
      capabilities_used: ['claude-code']
    }), expected)).toThrow('exec result 未包含预期能力');

    expect(() => validateExecResult(createExecResult({
      openspec_refs_consumed: ['artifacts/other-spec.md']
    }), expected)).toThrow('exec result 未包含预期 OpenSpec 消耗引用');

    expect(() => validateExecResult(createExecResult({
      superpowers_refs_consumed: ['verification-before-completion']
    }), expected)).toThrow('exec result 未覆盖 contract 要求的方法集合');

    expect(validateExecResult(createExecResult(), expected).status).toBe('succeeded');
  });

  it('validateReviewReport 校验当前任务绑定的基线、方法与执行单元', () => {
    const expected = {
      task_id: 'aria-20260419-001',
      result_set_id: 'result-set-aria-20260419-001-01',
      exec_unit_id: 'exec-01',
      spec_ref: 'cadence/cache/aria/tasks/aria-20260419-001/artifacts/spec-artifact.md',
      plan_ref: 'cadence/cache/aria/tasks/aria-20260419-001/artifacts/plan-brief.md',
      required_methods: ['verification-before-completion'],
      source_capabilities: ['OpenSpec', 'superpowers']
    };

    expect(() => validateReviewReport(createReviewReport({
      baseline_refs: ['artifacts/spec-artifact.md']
    }), expected)).toThrow('review report baseline_refs 不完整');

    expect(() => validateReviewReport(createReviewReport({
      method_refs: ['test-driven-development']
    }), expected)).toThrow('review report 未覆盖要求的方法集合');

    expect(() => validateReviewReport(createReviewReport({
      exec_units_reviewed: ['exec-02']
    }), expected)).toThrow('review report 未绑定预期 exec_unit');

    expect(validateReviewReport(createReviewReport(), expected).verdict).toBe('passed');
  });

  it('validateTestReport 校验当前任务绑定的基线、方法与执行单元', () => {
    const expected = {
      task_id: 'aria-20260419-001',
      result_set_id: 'result-set-aria-20260419-001-01',
      exec_unit_id: 'exec-01',
      spec_ref: 'cadence/cache/aria/tasks/aria-20260419-001/artifacts/spec-artifact.md',
      plan_ref: 'cadence/cache/aria/tasks/aria-20260419-001/artifacts/plan-brief.md',
      required_methods: ['test-driven-development', 'verification-before-completion'],
      source_capabilities: ['OpenSpec', 'superpowers']
    };

    expect(() => validateTestReport(createTestReport({
      baseline_refs: ['artifacts/other-plan.md']
    }), expected)).toThrow('test report baseline_refs 不完整');

    expect(() => validateTestReport(createTestReport({
      method_refs: ['verification-before-completion']
    }), expected)).toThrow('test report 未覆盖要求的方法集合');

    expect(() => validateTestReport(createTestReport({
      exec_units_tested: ['exec-02']
    }), expected)).toThrow('test report 未绑定预期 exec_unit');

    expect(validateTestReport(createTestReport(), expected).verdict).toBe('passed');
  });
});
