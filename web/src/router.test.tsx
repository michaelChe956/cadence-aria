import { describe, expect, it } from "vitest";
import { router } from "./router";

describe("router", () => {
  it("registers the coding workspace route", () => {
    expect(router.routesByPath["/workbench/coding/$attemptId"]).toBeDefined();
  });
});
