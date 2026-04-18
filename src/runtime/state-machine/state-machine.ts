import { guardDispatched, guardPlanApproved } from './guards.js';

export function canTransition(
  state: {
    status: string;
    approved_plan_ref: string | null;
    approved_spec_ref: string | null;
    confirmation_event_path?: string | null;
    dispatch_contract_ref?: string | null;
    context_bundle_ref?: string | null;
  },
  target: string
) {
  if (state.status === 'plan-review' && target === 'plan-approved') {
    return guardPlanApproved(state);
  }

  if (state.status === 'plan-approved' && target === 'dispatched') {
    return guardDispatched(state);
  }

  return { allowed: true as const };
}
