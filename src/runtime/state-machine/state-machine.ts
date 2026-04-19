import type { State } from '../../schemas/state-schema.js';

import {
  guardDispatched,
  guardPlanApproved,
  guardPlanReadyForDispatch,
  guardSpecReviewed,
  guardVerified
} from './guards.js';

const allowedTransitions: Record<State['status'], readonly State['status'][]> = {
  intake: ['clarification', 'spec-drafting'],
  clarification: ['spec-drafting'],
  'spec-drafting': ['spec-review'],
  'spec-review': ['spec-approved', 'plan-review'],
  'spec-approved': ['planning'],
  planning: ['plan-review'],
  'plan-review': ['plan-approved', 'dispatched'],
  'plan-approved': ['dispatched'],
  dispatched: ['executing'],
  executing: ['reviewing/testing'],
  'reviewing/testing': ['patching', 'verified'],
  patching: ['executing'],
  blocked: [],
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

  if (allowedTransitions[state.status]?.includes(target)) {
    return { allowed: true as const };
  }

  return { allowed: false as const, reason: 'invalid_transition' };
}
