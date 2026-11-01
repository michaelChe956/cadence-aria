import { beforeEach, describe, expect, it } from "vitest";
import {
  WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY,
  readWorkspaceProviderDefaults,
  readWorkspaceProviderDefaultsSnapshot,
  writeWorkspaceProviderDefaults,
} from "./workspace-provider-defaults";

describe("workspace provider defaults creation snapshot (REQ-PPS-01)", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("narrows stored defaults into creation request provider fields", () => {
    writeWorkspaceProviderDefaults({
      author: "pi",
      reviewer: "kimi_code",
      reviewerEnabled: true,
    });

    expect(readWorkspaceProviderDefaultsSnapshot()).toEqual({
      author_provider: "pi",
      reviewer_provider: "kimi_code",
    });
  });

  it("drops unknown provider names and stays empty without stored defaults", () => {
    expect(readWorkspaceProviderDefaultsSnapshot()).toEqual({});

    writeWorkspaceProviderDefaults({
      author: "gpt-9",
      reviewer: "codex",
      reviewerEnabled: false,
    });

    expect(readWorkspaceProviderDefaultsSnapshot()).toEqual({
      reviewer_provider: "codex",
    });
  });
});

describe("workspace provider defaults", () => {
  beforeEach(() => {
    window.localStorage.clear();
  });

  it("round-trips provider defaults through localStorage", () => {
    const defaults = {
      author: "pi",
      reviewer: "kimi_code",
      reviewerEnabled: true,
    } as const;

    writeWorkspaceProviderDefaults(defaults);

    expect(readWorkspaceProviderDefaults()).toEqual(defaults);
  });

  it("returns null for missing or malformed payloads", () => {
    expect(readWorkspaceProviderDefaults()).toBeNull();

    window.localStorage.setItem(WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY, "not-json");
    expect(readWorkspaceProviderDefaults()).toBeNull();

    window.localStorage.setItem(
      WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY,
      JSON.stringify({ author: "pi", reviewer: "kimi_code", reviewerEnabled: "true" }),
    );
    expect(readWorkspaceProviderDefaults()).toBeNull();
  });

  it("silently degrades when storage access fails", () => {
    const originalGetItem = window.localStorage.getItem;
    const originalSetItem = window.localStorage.setItem;
    Object.defineProperty(window.localStorage, "getItem", {
      configurable: true,
      value: () => {
        throw new Error("storage disabled");
      },
    });
    Object.defineProperty(window.localStorage, "setItem", {
      configurable: true,
      value: () => {
        throw new Error("storage disabled");
      },
    });

    try {
      expect(readWorkspaceProviderDefaults()).toBeNull();
      expect(() =>
        writeWorkspaceProviderDefaults({
          author: "pi",
          reviewer: "kimi_code",
          reviewerEnabled: true,
        }),
      ).not.toThrow();
    } finally {
      Object.defineProperty(window.localStorage, "getItem", {
        configurable: true,
        value: originalGetItem,
      });
      Object.defineProperty(window.localStorage, "setItem", {
        configurable: true,
        value: originalSetItem,
      });
    }
  });
});
