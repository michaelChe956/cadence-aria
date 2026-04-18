export function guardPlanApproved(input: {
  approved_plan_ref: string | null;
  confirmation_event_path?: string | null;
}) {
  if (!input.approved_plan_ref || !input.confirmation_event_path) {
    return { allowed: false as const, reason: 'missing_confirmation_event' };
  }

  return { allowed: true as const };
}

export function guardDispatched(input: {
  approved_plan_ref: string | null;
  approved_spec_ref: string | null;
  dispatch_contract_ref?: string | null;
  context_bundle_ref?: string | null;
}) {
  if (!input.approved_plan_ref || !input.approved_spec_ref) {
    return { allowed: false as const, reason: 'missing_frozen_refs' };
  }

  if (!input.dispatch_contract_ref || !input.context_bundle_ref) {
    return { allowed: false as const, reason: 'handoff_incomplete' };
  }

  return { allowed: true as const };
}
