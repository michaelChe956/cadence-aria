import { z } from 'zod';

const datetimeString = z.string();

const inputArtifactsSchema = z.record(z.union([z.string(), z.array(z.string())]));

const workspaceContextSchema = z.object({
  repo_path: z.string(),
  worktree_ref: z.string(),
  base_revision: z.string()
});

export const dispatchContractSchema = z.object({
  contract_version: z.literal('1.0'),
  generated_at: datetimeString,
  base_revision: z.string(),
  input_artifacts: inputArtifactsSchema,
  generated_from_plan: z.string(),
  source_task_refs: z.array(z.string()).min(1),
  task_id: z.string(),
  timeout_minutes: z.number().int().positive(),
  based_on_spec_ref: z.string(),
  based_on_plan_ref: z.string(),
  context_bundle_ref: z.string(),
  output_schema_ref: z.string(),
  exec_unit_id: z.string(),
  contract_type: z.literal('dispatch'),
  parent_task: z.string(),
  mode: z.literal('exec'),
  scope: z.object({
    files_allowed: z.array(z.string()).min(1),
    files_blocked: z.array(z.string()).optional()
  }),
  goal: z.string().min(1),
  acceptance: z.array(z.string()).min(1),
  dependencies: z.array(z.string()),
  worktree_ref: z.string().optional(),
  result_path: z.string(),
  retry_allowed: z.boolean()
});

export const executionContextBundleSchema = z.object({
  bundle_id: z.string(),
  spec_ref: z.string(),
  plan_ref: z.string(),
  scope_constraints_ref: z.string(),
  required_methods: z.array(z.string()).min(1),
  workspace_context: workspaceContextSchema,
  verification_requirements: z.array(z.string()).min(1),
  prompt_template_ref: z.string()
});

export type DispatchContract = z.infer<typeof dispatchContractSchema>;
export type ExecutionContextBundle = z.infer<typeof executionContextBundleSchema>;
