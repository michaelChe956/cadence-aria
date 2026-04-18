import { z } from 'zod';

const execUnitSchema = z.object({
  status: z.enum(['pending', 'running', 'succeeded', 'failed', 'timeout', 'cancelled', 'blocked']),
  contract_path: z.string(),
  worktree_ref: z.string().optional(),
  attempt: z.number().int().min(0),
  exit_code: z.number().int().nullable(),
  result_path: z.string(),
  started_at: z.string().optional(),
  finished_at: z.string().optional(),
  blocked_by: z.array(z.string())
});

const patchUnitSchema = z.object({
  status: z.enum(['pending', 'running', 'succeeded', 'failed', 'cancelled']),
  based_on_exec_unit: z.string(),
  contract_path: z.string(),
  attempt: z.number().int().min(0),
  started_at: z.string().optional(),
  finished_at: z.string().optional()
});

export const stateSchema = z.object({
  task_id: z.string(),
  source: z.enum(['vk', 'native', 'aria-native']),
  flow_type: z.enum(['formal', 'fast-lane']),
  risk_level: z.enum(['low', 'medium', 'high']),
  status: z.enum([
    'intake',
    'clarification',
    'spec-drafting',
    'spec-review',
    'spec-approved',
    'planning',
    'plan-review',
    'plan-approved',
    'dispatched',
    'executing',
    'reviewing/testing',
    'patching',
    'verified',
    'done',
    'cancelled'
  ]),
  current_round: z.number().int().min(1),
  approved_spec_ref: z.string().nullable(),
  approved_plan_ref: z.string().nullable(),
  active_result_set_id: z.string().nullable(),
  active_exec_units: z.array(z.string()),
  confirmation_pending: z.enum(['none', 'spec', 'plan']),
  confirmation_mode: z.enum(['manual', 'auto-policy']),
  confirmation_artifact_path: z.string().nullable(),
  review_status: z.enum(['pending', 'passed', 'failed']),
  test_status: z.enum(['pending', 'passed', 'failed']),
  patch_required_by: z.enum(['none', 'review', 'test', 'both']),
  patch_round: z.number().int().min(0),
  block_reason_code: z.string().nullable().optional(),
  blocking_stage: z.string().nullable().optional(),
  retryable: z.boolean().optional(),
  required_action: z.string().nullable().optional(),
  exec_units: z.record(execUnitSchema),
  patch_units: z.record(patchUnitSchema).optional(),
  created_at: z.string(),
  updated_at: z.string()
});

export type State = z.infer<typeof stateSchema>;

export function parseState(input: unknown): State {
  return stateSchema.parse(input);
}

export const execUnitStateSchema = execUnitSchema;
export const patchUnitStateSchema = patchUnitSchema;
