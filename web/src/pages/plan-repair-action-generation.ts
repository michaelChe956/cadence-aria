import type { PlanRepairSessionState } from "../state/plan-repair-session";

export function planRepairActionGenerationKey(repair: PlanRepairSessionState) {
  return canonicalStringify({
    childSessionId: repair.childSessionId,
    request: {
      id: repair.request.id,
      status: repair.request.status,
      updatedAt: repair.request.updated_at,
      fingerprint: repair.request.fingerprint,
    },
    stage: repair.stage,
    amendment: repair.amendment
      ? {
          id: repair.amendment.id,
          newPlanRevisionId: repair.amendment.new_plan_revision_id,
          createdAt: repair.amendment.created_at,
        }
      : null,
    projectionId: repair.projection?.id ?? null,
    validation: repair.validation
      ? {
          id: repair.validation.id,
          planRevisionId: repair.validation.plan_revision_id,
          createdAt: repair.validation.created_at,
        }
      : null,
    packageIdentity: repair.package_identity
      ? {
          candidatePackageFingerprint:
            repair.package_identity.candidate_package_fingerprint,
          reviewGenerationRoundId:
            repair.package_identity.review_generation_round_id,
          nextPlanRevisionId: repair.package_identity.next_plan_revision_id,
        }
      : null,
    candidatePackageArtifactId: repair.candidate_package_artifact_id,
    planReview: repair.plan_review
      ? {
          generationRoundId: repair.plan_review.generation_round_id,
          verdict: repair.plan_review.verdict,
          reviewAction: repair.plan_review.review_action,
        }
      : null,
    impact: repair.impact,
    impactScopeReview: repair.impact_scope_review
      ? {
          candidatePackageFingerprint:
            repair.impact_scope_review.candidate_package_fingerprint,
          reviewGenerationRoundId:
            repair.impact_scope_review.review_generation_round_id,
        }
      : null,
    error: repair.error,
  });
}

function canonicalStringify(value: unknown) {
  return JSON.stringify(canonicalValue(value));
}

function canonicalValue(value: unknown): unknown {
  if (Array.isArray(value)) {
    return value.map(canonicalValue);
  }
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value)
        .sort(([left], [right]) => left.localeCompare(right))
        .map(([key, entry]) => [key, canonicalValue(entry)]),
    );
  }
  return value;
}
