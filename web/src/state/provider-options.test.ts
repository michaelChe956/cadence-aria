import { describe, expect, it } from "vitest";
import type {
  ProviderHealthEntry,
  ProviderHealthResponse,
  RealProviderName,
} from "../api/types";
import { getProviderOption, getProviderOptions } from "./provider-options";

function entry(
  provider: RealProviderName,
  available: boolean,
  overrides: Partial<ProviderHealthEntry> = {},
): ProviderHealthEntry {
  return {
    provider,
    display_name: provider === "claude_code" ? "Claude Code API" : "Codex API",
    available,
    version: available ? "1.0.0" : null,
    reason_code: available ? null : "command_missing",
    reason: available ? null : `${provider} unavailable`,
    checked_at: "2026-07-14T00:00:00Z",
    install_hint: `Install ${provider}`,
    ...overrides,
  };
}

function snapshot(
  providers: ProviderHealthEntry[],
  overrides: Partial<ProviderHealthResponse> = {},
): ProviderHealthResponse {
  return {
    schema_version: 1,
    generation: 1,
    checked_at: "2026-07-14T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: false,
    test_provider_enabled: false,
    providers,
    ...overrides,
  };
}

describe("provider options", () => {
  it.each([
    {
      name: "Claude available and Codex unavailable",
      providers: [entry("claude_code", true), entry("codex", false)],
      available: { claude_code: true, codex: false },
    },
    {
      name: "Codex available and Claude unavailable",
      providers: [entry("claude_code", false), entry("codex", true)],
      available: { claude_code: false, codex: true },
    },
    {
      name: "all real providers unavailable",
      providers: [entry("claude_code", false), entry("codex", false)],
      available: { claude_code: false, codex: false },
    },
  ])("maps $name with stable order and entry reasons", ({ providers, available }) => {
    const options = getProviderOptions(
      snapshot(providers, {
        real_workflow_blocked: !Object.values(available).some(Boolean),
      }),
    );

    expect(options.map((option) => option.value)).toEqual([
      "claude_code",
      "codex",
      "pi",
      "fake",
    ]);
    expect(options[0]).toMatchObject({
      label: "Claude Code API",
      visible: true,
      real: true,
      available: available.claude_code,
      disabled: !available.claude_code,
      installHint: "Install claude_code",
    });
    expect(options[1]).toMatchObject({
      label: "Codex API",
      visible: true,
      real: true,
      available: available.codex,
      disabled: !available.codex,
      installHint: "Install codex",
    });
    expect(options[2]).toMatchObject({
      value: "pi",
      label: "Pi",
      visible: true,
      available: false,
      disabled: true,
      reason: "Provider 状态尚未确认",
      installHint: null,
    });
    if (!available.claude_code) {
      expect(options[0].reason).toBe("claude_code unavailable");
    }
    if (!available.codex) {
      expect(options[1].reason).toBe("codex unavailable");
    }
  });

  it("fails closed for a degraded snapshot even when old entries remain available", () => {
    const options = getProviderOptions(
      snapshot([entry("claude_code", true), entry("codex", true)], {
        state_status: "degraded",
        state_error: "provider health persistence failed",
        real_workflow_blocked: true,
      }),
    );

    expect(options.slice(0, 2)).toEqual(
      expect.arrayContaining([
        expect.objectContaining({
          value: "claude_code",
          available: false,
          disabled: true,
          reason: "provider health persistence failed",
        }),
        expect.objectContaining({
          value: "codex",
          available: false,
          disabled: true,
          reason: "provider health persistence failed",
        }),
      ]),
    );
  });

  it("prefers an entry reason over the degraded state reason", () => {
    const option = getProviderOption(
      snapshot(
        [entry("claude_code", false), entry("codex", true)],
        {
          state_status: "degraded",
          state_error: "provider health persistence failed",
          real_workflow_blocked: true,
        },
      ),
      "claude_code",
    );

    expect(option.reason).toBe("claude_code unavailable");
  });

  it("uses fallback labels and fails closed for missing real entries", () => {
    const options = getProviderOptions(
      snapshot([
        entry("claude_code", true, {
          display_name: "",
        }),
      ]),
    );

    expect(options[0].label).toBe("Claude Code");
    expect(options[1]).toMatchObject({
      value: "codex",
      label: "Codex",
      visible: true,
      available: false,
      disabled: true,
      reason: "Provider 状态尚未确认",
      installHint: null,
    });
  });

  it("fails closed before any snapshot is available", () => {
    const options = getProviderOptions(null);

    expect(options.slice(0, 2).every((option) => option.disabled)).toBe(true);
    expect(options.slice(0, 2).every((option) => !option.available)).toBe(true);
    expect(options.slice(0, 2).map((option) => option.reason)).toEqual([
      "Provider 状态尚未确认",
      "Provider 状态尚未确认",
    ]);
  });

  it("hides Fake in product mode and enables it only from the API test flag", () => {
    const productFake = getProviderOption(
      snapshot([entry("claude_code", true), entry("codex", true)]),
      "fake",
    );
    const testFake = getProviderOption(
      snapshot([entry("claude_code", false), entry("codex", false)], {
        real_workflow_blocked: true,
        test_provider_enabled: true,
      }),
      "fake",
    );

    expect(productFake).toMatchObject({
      label: "Fake",
      visible: false,
      available: false,
      disabled: true,
      real: false,
    });
    expect(testFake).toMatchObject({
      label: "Fake",
      visible: true,
      available: true,
      disabled: false,
      reason: null,
      installHint: null,
      real: false,
    });
  });

  it("returns the same catalog rule through the by-name selector", () => {
    const health = snapshot([entry("claude_code", true), entry("codex", false)]);

    expect(getProviderOption(health, "codex")).toEqual(
      getProviderOptions(health).find((option) => option.value === "codex"),
    );
  });
  it("pi available 时出现在 provider 选项中", () => {
    const snapshot = {
      real_workflow_blocked: false,
      state_error: null,
      state_status: "ready" as const,
      test_provider_enabled: false,
      providers: [
        {
          provider: "pi",
          display_name: "Pi",
          available: true,
          version: "0.83.0",
          reason_code: null,
          reason: null,
          checked_at: "",
          install_hint: "",
        },
      ],
    };
    const options = getProviderOptions(snapshot as any);
    const pi = options.find((option) => option.value === "pi");
    expect(pi).toBeDefined();
    expect(pi?.available).toBe(true);
    expect(pi?.real).toBe(true);
  });
});
