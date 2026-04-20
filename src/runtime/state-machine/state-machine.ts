import type { State } from '../../schemas/state-schema.js';

import {
  guardBlockedRetry,
  guardDispatched,
  guardPatchingReady,
  guardPlanApproved,
  guardPlanReadyForDispatch,
  guardSpecReviewed,
  guardVerified
} from './guards.js';

const allowedTransitions: Record<State['status'], readonly State['status'][]> = {
  intake: ['clarification', 'spec-drafting', 'cancelled'],
  clarification: ['spec-drafting', 'cancelled'],
  'spec-drafting': ['spec-review', 'cancelled'],
  'spec-review': ['spec-approved', 'plan-review', 'cancelled'],
  'spec-approved': ['planning', 'cancelled'],
  planning: ['plan-review', 'cancelled'],
  'plan-review': ['plan-approved', 'dispatched', 'cancelled'],
  'plan-approved': ['dispatched', 'cancelled'],
  dispatched: ['executing', 'cancelled'],
  executing: ['reviewing/testing', 'cancelled'],
  'reviewing/testing': ['patching', 'verified', 'cancelled'],
  patching: ['executing', 'cancelled'],
  blocked: ['dispatched', 'cancelled'],
  verified: ['done'],
  done: [],
  cancelled: []
};

export function canTransition(
  state: State,
  target: State['status']
) {
  if (state.status === 'spec-review' && target === 'plan-review') {
    return guardSpecReviewed(state);
  }

  if (state.status === 'plan-review' && target === 'plan-approved') {
    return guardPlanApproved(state);
  }

  if (state.status === 'plan-review' && target === 'dispatched') {
    return guardPlanReadyForDispatch(state);
  }

  if (state.status === 'plan-approved' && target === 'dispatched') {
    return guardDispatched(state);
  }

  if (state.status === 'reviewing/testing' && target === 'verified') {
    return guardVerified(state);
  }

  if (state.status === 'patching' && target === 'executing') {
    return guardPatchingReady(state);
  }

  if (state.status === 'blocked' && target === 'dispatched') {
    return guardBlockedRetry(state);
  }

  if (allowedTransitions[state.status]?.includes(target)) {
    return { allowed: true as const };
  }

  return { allowed: false as const, reason: 'invalid_transition' };
}
