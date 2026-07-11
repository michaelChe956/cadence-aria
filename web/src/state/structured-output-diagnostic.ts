import type { StructuredOutputDiagnostic } from "../api/types";

export function structuredOutputDiagnosticFromUnknown(
  value: unknown,
): StructuredOutputDiagnostic | undefined {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    return undefined;
  }

  const diagnostic = value as Record<string, unknown>;
  if (
    typeof diagnostic.code !== "string" ||
    typeof diagnostic.message !== "string" ||
    typeof diagnostic.repair_attempted !== "boolean" ||
    typeof diagnostic.repair_succeeded !== "boolean"
  ) {
    return undefined;
  }

  const rawOutputPreview = diagnostic.raw_output_preview;
  if (
    rawOutputPreview !== undefined &&
    rawOutputPreview !== null &&
    typeof rawOutputPreview !== "string"
  ) {
    return undefined;
  }

  return {
    code: diagnostic.code,
    message: diagnostic.message,
    repair_attempted: diagnostic.repair_attempted,
    repair_succeeded: diagnostic.repair_succeeded,
    ...(rawOutputPreview === undefined ? {} : { raw_output_preview: rawOutputPreview }),
  };
}
