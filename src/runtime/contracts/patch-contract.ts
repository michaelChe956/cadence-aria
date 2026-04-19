import { nowIso } from '../../utils/time.js';

export function buildPatchContract(taskId: string, resultSetId: string) {
  return {
    task_id: taskId,
    unit_id: 'patch-01',
    contract_type: 'patch',
    based_on_spec_ref: 'artifacts/spec-artifact.md',
    based_on_plan_ref: 'artifacts/plan-brief.md',
    based_on_result_set_id: resultSetId,
    patch_reason: 'must-fix items found',
    must_fix_items: ['fix-review-blocker-01'],
    context_bundle_ref: 'artifacts/execution-context-bundle.yaml',
    output_schema_ref: 'src/schemas/runtime-artifact-schema.ts',
    generated_at: nowIso()
  };
}
