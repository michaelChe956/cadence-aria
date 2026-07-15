import { describe, expect, it } from "vitest";
import { structuredOutputDiagnosticFromUnknown } from "./structured-output-diagnostic";

describe("structuredOutputDiagnosticFromUnknown", () => {
  const validDiagnostic = {
    code: "invalid_json",
    message: "Reviewer 输出不是合法 JSON",
    repair_attempted: true,
    repair_succeeded: false,
  };

  it.each([
    null,
    undefined,
    "invalid",
    [],
    {},
    { ...validDiagnostic, code: 123 },
    { ...validDiagnostic, message: null },
    { ...validDiagnostic, repair_attempted: "true" },
    { ...validDiagnostic, repair_succeeded: 0 },
    { ...validDiagnostic, raw_output_preview: { untrusted: true } },
  ])("rejects invalid diagnostic %#", (value) => {
    expect(structuredOutputDiagnosticFromUnknown(value)).toBeUndefined();
  });

  it.each([
    [undefined, undefined],
    [null, null],
    ["未校验内容", "未校验内容"],
  ] as const)("accepts raw preview %s", (rawOutputPreview, expectedPreview) => {
    const input = {
      ...validDiagnostic,
      ...(rawOutputPreview === undefined ? {} : { raw_output_preview: rawOutputPreview }),
    };

    expect(structuredOutputDiagnosticFromUnknown(input)).toEqual({
      ...validDiagnostic,
      ...(rawOutputPreview === undefined ? {} : { raw_output_preview: expectedPreview }),
    });
  });
});
