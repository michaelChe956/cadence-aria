import type {
  ContractImpactReport,
  PlanAmendmentManifest,
} from "../../api/types";

export function classifyPlanRepairImpact(
  amendment: PlanAmendmentManifest,
  impact: ContractImpactReport,
) {
  const actualAffected = unique([
    ...Object.keys(amendment.revised_work_items),
    ...amendment.contract_deltas.map((delta) => delta.logical_work_item_id),
    ...amendment.stale_units,
    ...amendment.revalidation_required_units,
    ...Object.keys(amendment.replacement_units),
    ...Object.values(amendment.replacement_units).flat(),
    ...impact.direct_stale,
    ...impact.direct_revalidation,
    amendment.resume_target.logical_work_item_id,
  ]);
  const actualSet = new Set(actualAffected);
  const conditionalOnly = unique(impact.conditional_downstream).filter(
    (logicalId) => !actualSet.has(logicalId),
  );
  const conditionalSet = new Set(impact.conditional_downstream);
  const unaffected = unique(impact.unaffected).filter(
    (logicalId) => !actualSet.has(logicalId) && !conditionalSet.has(logicalId),
  );
  return { actualAffected, conditionalOnly, unaffected };
}

function unique(values: string[]) {
  return [...new Set(values.filter(Boolean))];
}
