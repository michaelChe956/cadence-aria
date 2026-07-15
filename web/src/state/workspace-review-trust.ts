export function trustedReviewComments(
  metadata: Record<string, unknown> | undefined,
): string {
  const diagnostic = metadata?.structured_output_diagnostic;
  const failedDiagnostic =
    isRecord(diagnostic) && diagnostic.repair_succeeded === false;
  if (failedDiagnostic) {
    return "";
  }
  return typeof metadata?.comments === "string" ? metadata.comments.trim() : "";
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}
