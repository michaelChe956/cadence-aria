import type { DispatchContract, ExecutionContextBundle } from '../../schemas/runtime-artifact-schema.js';
import { dispatchContractSchema, executionContextBundleSchema } from '../../schemas/runtime-artifact-schema.js';

export function validateHandoffFields(input: {
  approved_spec_ref: string | null;
  approved_plan_ref: string | null;
}): void {
  if (!input.approved_spec_ref || !input.approved_plan_ref) {
    throw new Error('missing frozen refs');
  }
}

export function validateExecutionContextBundle(input: unknown): ExecutionContextBundle {
  return executionContextBundleSchema.parse(input);
}

export function validateDispatchContract(input: unknown): DispatchContract {
  return dispatchContractSchema.parse(input);
}
