export function buildPlanBrief(taskId: string): string {
  return `plan_id: plan-${taskId}\nexec_unit_count: 1\nacceptance_strategy: all_units_pass\n`;
}
