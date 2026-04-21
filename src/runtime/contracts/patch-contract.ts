import fs from 'node:fs/promises';
import path from 'node:path';

import { nowIso } from '../../utils/time.js';
import { getTaskArtifactsDir } from '../persistence/paths.js';
import { stringifyYaml } from '../../utils/yaml.js';

export function buildPatchContract(input: {
  taskId: string;
  resultSetId: string;
  dispatchContractRef: string;
  contextBundleRef: string;
  specRef: string;
  planRef: string;
  patchRequiredBy: 'review' | 'test' | 'both';
}) {
  const reasons = {
    review: ['fix-review-blocker-01'],
    test: ['fix-test-failure-01'],
    both: ['fix-review-blocker-01', 'fix-test-failure-01']
  } as const;

  return {
    task_id: input.taskId,
    unit_id: 'patch-01',
    contract_type: 'patch',
    based_on_dispatch_contract: input.dispatchContractRef,
    based_on_exec_unit: 'exec-01',
    based_on_spec_ref: input.specRef,
    based_on_plan_ref: input.planRef,
    based_on_result_set_id: input.resultSetId,
    patch_required_by: input.patchRequiredBy,
    patch_reason: 'must-fix items found',
    must_fix_items: [...reasons[input.patchRequiredBy]],
    context_bundle_ref: input.contextBundleRef,
    output_schema_ref: 'src/schemas/runtime-artifact-schema.ts',
    generated_at: nowIso()
  };
}

export async function createPatchArtifacts(input: {
  taskId: string;
  resultSetId: string;
  dispatchContractRef: string;
  contextBundleRef: string;
  specRef: string;
  planRef: string;
  patchRequiredBy: 'review' | 'test' | 'both';
}): Promise<{ patchContractRef: string; patchUnitId: 'patch-01' }> {
  const patchContractRef = path.posix.join(getTaskArtifactsDir(input.taskId), 'patch-contract-patch-01.yaml');
  const contract = buildPatchContract(input);

  await fs.mkdir(path.posix.dirname(patchContractRef), { recursive: true });
  await fs.writeFile(patchContractRef, stringifyYaml(contract), 'utf8');

  return {
    patchContractRef,
    patchUnitId: 'patch-01'
  };
}
