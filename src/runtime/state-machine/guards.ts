import type { State } from '../../schemas/state-schema.js';

type SpecReviewGuardState = Pick<State, 'confirmation_artifact_path'>;
type PlanApprovedGuardState = Pick<State, 'approved_plan_ref' | 'confirmation_event_path'>;
type DispatchedGuardState = Pick<
  State,
  'approved_plan_ref' | 'approved_spec_ref' | 'dispatch_contract_ref' | 'context_bundle_ref'
>;
type PlanReadyForDispatchGuardState = Pick<
  State,
  'approved_plan_ref' | 'approved_spec_ref' | 'confirmation_artifact_path' | 'confirmation_event_path' | 'dispatch_contract_ref' | 'context_bundle_ref'
>;
type VerifiedGuardState = Pick<
  State,
  'review_status' | 'test_status' | 'review_report_ref' | 'test_report_ref'
>;
type PatchingGuardState = Pick<State, 'patch_units'>;

export function guardSpecReviewed(input: SpecReviewGuardState) {
  if (!input.confirmation_artifact_path) {
    return { allowed: false as const, reason: 'missing_confirmation_artifact' };
  }

  return { allowed: true as const };
}

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

export function guardPlanReadyForDispatch(input: PlanReadyForDispatchGuardState) {
  if (!input.approved_plan_ref || !input.approved_spec_ref) {
    return { allowed: false as const, reason: 'missing_frozen_refs' };
  }

  if (!input.confirmation_event_path || !input.confirmation_artifact_path) {
    return { allowed: false as const, reason: 'missing_confirmation_artifact' };
  }

  if (!input.dispatch_contract_ref || !input.context_bundle_ref) {
    return { allowed: false as const, reason: 'handoff_incomplete' };
  }

  return { allowed: true as const };
}

export function guardVerified(input: VerifiedGuardState) {
  if (!input.review_report_ref || !input.test_report_ref) {
    return { allowed: false as const, reason: 'missing_review_test_reports' };
  }

  if (input.review_status !== 'passed' || input.test_status !== 'passed') {
    return { allowed: false as const, reason: 'review_or_test_not_passed' };
  }

  return { allowed: true as const };
}

export function guardPatchingReady(input: PatchingGuardState) {
  if (!input.patch_units || Object.keys(input.patch_units).length === 0) {
    return { allowed: false as const, reason: 'missing_patch_contract' };
  }

  return { allowed: true as const };
}
