export function buildIntakeCard(input: { task_id: string; title: string }): string {
  return [
    `task_id: ${input.task_id}`,
    'source: aria-native',
    'flow_type_suggestion: formal',
    'risk_level: medium',
    `scope_summary: ${input.title}`,
  ].join('\n');
}
