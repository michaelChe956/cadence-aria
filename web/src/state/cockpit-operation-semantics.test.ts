import { describe, expect, it } from "vitest";
import {
  BULK_OPERATION_WHITELIST,
  CODING_WORKSPACE_HOTKEYS,
  COCKPIT_HOTKEYS,
  DANGEROUS_CONFIRM_TIMEOUT_MS,
  OPERATOR_LABEL,
  canBulkApply,
  isTextEditingTarget,
} from "./cockpit-operation-semantics";

describe("cockpit operation semantics", () => {
  it("owns the shared constants and permits only idempotent bulk confirmation", () => {
    expect(OPERATOR_LABEL).toBe("本地浏览器");
    expect(DANGEROUS_CONFIRM_TIMEOUT_MS).toBe(10_000);
    expect(COCKPIT_HOTKEYS).toEqual({
      confirm: { code: "Enter", ctrlOrMeta: true, shift: false },
      feedback: { code: "KeyF", ctrlOrMeta: true, shift: false },
      takeover: { code: "KeyT", ctrlOrMeta: true, shift: true },
      advance: { code: "KeyA", ctrlOrMeta: true, shift: false },
    });
    expect(BULK_OPERATION_WHITELIST).toEqual(new Set(["confirm"]));
    expect(canBulkApply("confirm")).toBe(true);
    expect(canBulkApply("terminate")).toBe(false);
  });

  it("exports the same mapping object for both cockpit pages", () => {
    expect(CODING_WORKSPACE_HOTKEYS).toBe(COCKPIT_HOTKEYS);
  });

  it("recognizes editable targets before hotkeys can act", () => {
    expect(isTextEditingTarget(document.createElement("input"))).toBe(true);
    expect(isTextEditingTarget(document.createElement("textarea"))).toBe(true);
    expect(isTextEditingTarget(document.createElement("select"))).toBe(true);
    expect(isTextEditingTarget(document.createElement("button"))).toBe(false);
  });
});
