import { afterEach, describe, expect, it } from "vitest";
import { planRepairSnapshotFixture } from "./workspace-plan-repair-test-fixtures";
import { parentSessionIdFor } from "./parent-session-navigation";

describe("parentSessionIdFor", () => {
  afterEach(() => {
    sessionStorage.clear();
  });

  it("prefers the durable plan-repair parent for the current child", () => {
    sessionStorage.setItem("aria.takeover-parent:child_001", "takeover_parent");

    expect(
      parentSessionIdFor(planRepairSnapshotFixture("child_001"), {}, "child_001"),
    ).toBe("session_parent");
  });

  it("uses the in-memory takeover link before its browser-session mirror", () => {
    sessionStorage.setItem("aria.takeover-parent:child_002", "session_storage_parent");

    expect(
      parentSessionIdFor(
        null,
        {
          child_002: {
            parentSessionId: "audit_store_parent",
            takeoverEventId: "takeover_001",
          },
        },
        "child_002",
      ),
    ).toBe("audit_store_parent");
  });

  it("restores a takeover parent from session storage after store memory is lost", () => {
    sessionStorage.setItem("aria.takeover-parent:child_003", "parent_003");

    expect(parentSessionIdFor(null, {}, "child_003")).toBe("parent_003");
  });

  it("does not infer a parent for a session without either link", () => {
    expect(parentSessionIdFor(null, {}, "ordinary")).toBeNull();
  });
});
