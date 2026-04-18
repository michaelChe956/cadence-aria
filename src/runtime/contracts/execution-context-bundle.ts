import path from 'node:path';

import type { ExecutionContextBundle } from '../../schemas/runtime-artifact-schema.js';

export function buildExecutionContextBundle(input: {
  task_id: string;
  spec_ref: string;
  plan_ref: string;
}): ExecutionContextBundle {
  return {
    bundle_id: `execution-context-bundle-${input.task_id}`,
    spec_ref: input.spec_ref,
    plan_ref: input.plan_ref,
    scope_constraints_ref: input.plan_ref,
    required_methods: ['writing-plans', 'test-driven-development', 'verification-before-completion'],
    workspace_context: {
      repo_path: process.cwd(),
      worktree_ref: process.env.CADENCE_WORKTREE_REF ?? path.basename(process.cwd()),
      base_revision: process.env.CADENCE_BASE_REVISION ?? 'unknown'
    },
    verification_requirements: ['pnpm check', 'pnpm test'],
    prompt_template_ref: 'codex/prompts/dispatch.md'
  };
}
