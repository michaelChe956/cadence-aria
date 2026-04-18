import type { State } from '../../schemas/state-schema.js';

type PlanApprovedGuardState = Pick<State, 'approved_plan_ref' | 'confirmation_event_path'>;
type DispatchedGuardState = Pick<
  State,
  'approved_plan_ref' | 'approved_spec_ref' | 'dispatch_contract_ref' | 'context_bundle_ref'
>;

export function guardPlanApproved(input: PlanApprovedGuardState) {
  if (!input.approved_plan_ref || !input.confirmation_event_path) {
    return { allowed: false as const, reason: 'missing_confirmation_event' };
  }

  return { allowed: true as const };
}

export function guardDispatched(input: DispatchedGuardState) {
  if (!input.approved_plan_ref || !input.approved_spec_ref) {
    return { allowed: false as const, reason: 'missing_frozen_refs' };
  }

  if (!input.dispatch_contract_ref || !input.context_bundle_ref) {
    return { allowed: false as const, reason: 'handoff_incomplete' };
  }

  return { allowed: true as const };
}
