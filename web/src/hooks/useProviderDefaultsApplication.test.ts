import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { WsProviderConfig } from "../api/types";
import {
  WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY,
  writeWorkspaceProviderDefaults,
} from "../state/workspace-provider-defaults";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import {
  useProviderDefaultsApplication,
  type ProviderDefaultsApplicationOptions,
} from "./useProviderDefaultsApplication";

const SESSION_ID = "session_001";

describe("useProviderDefaultsApplication (REQ-PPS-02)", () => {
  const selectProvider = vi.fn();

  beforeEach(() => {
    window.localStorage.clear();
    selectProvider.mockClear();
    useWorkspaceStore.getState().reset();
  });

  function renderApplicationHook(
    overrides: Partial<ProviderDefaultsApplicationOptions> = {},
  ) {
    useWorkspaceStore.setState({ sessionId: SESSION_ID });
    return renderHook(
      (options: ProviderDefaultsApplicationOptions) =>
        useProviderDefaultsApplication(options),
      {
        initialProps: {
          ws: { selectProvider },
          sessionId: SESSION_ID,
          connected: true,
          providerEditable: true,
          providers: { author: "claude_code", reviewer: "codex" },
          isTakeover: false,
          ...overrides,
        },
      },
    );
  }

  it("applies the stored defaults for the differences once per session", () => {
    writeWorkspaceProviderDefaults({
      author: "pi",
      reviewer: "kimi_code",
      reviewerEnabled: true,
    });

    const { rerender } = renderApplicationHook();

    expect(selectProvider).toHaveBeenCalledTimes(2);
    expect(selectProvider).toHaveBeenCalledWith("author", "pi");
    expect(selectProvider).toHaveBeenCalledWith("reviewer", "kimi_code");
    expect(useWorkspaceStore.getState().reviewerEnabled).toBe(true);

    // providers 后续变化（会话快照刷新/乐观回写）不重复补发。
    rerender({
      ws: { selectProvider },
      sessionId: SESSION_ID,
      connected: true,
      providerEditable: true,
      providers: { author: "codex", reviewer: "codex" },
      isTakeover: false,
    });

    expect(selectProvider).toHaveBeenCalledTimes(2);
  });

  it("sends nothing when the session already matches the stored defaults", () => {
    writeWorkspaceProviderDefaults({
      author: "claude_code",
      reviewer: "codex",
      reviewerEnabled: false,
    });

    renderApplicationHook();

    expect(selectProvider).not.toHaveBeenCalled();
  });

  it("sends nothing without stored defaults", () => {
    renderApplicationHook();

    expect(selectProvider).not.toHaveBeenCalled();
  });

  it.each([
    ["a locked provider window", { providerEditable: false }],
    ["an observed takeover session", { isTakeover: true }],
    ["a disconnected socket", { connected: false }],
    ["a session the store has not loaded yet", { sessionId: "session_002" }],
  ] as const)("sends nothing for %s", (_label, overrides) => {
    writeWorkspaceProviderDefaults({
      author: "pi",
      reviewer: "kimi_code",
      reviewerEnabled: true,
    });

    renderApplicationHook(overrides);

    expect(selectProvider).not.toHaveBeenCalled();
  });

  it("waits for the provider snapshot and applies it after it arrives", () => {
    writeWorkspaceProviderDefaults({
      author: "pi",
      reviewer: "codex",
      reviewerEnabled: false,
    });

    const { rerender } = renderApplicationHook({ providers: null });
    expect(selectProvider).not.toHaveBeenCalled();

    rerender({
      ws: { selectProvider },
      sessionId: SESSION_ID,
      connected: true,
      providerEditable: true,
      providers: { author: "claude_code", reviewer: "codex" } as WsProviderConfig,
      isTakeover: false,
    });

    expect(selectProvider).toHaveBeenCalledWith("author", "pi");
  });

  it("does not push a reviewer selection when cross review is disabled", () => {
    writeWorkspaceProviderDefaults({
      author: "claude_code",
      reviewer: "kimi_code",
      reviewerEnabled: false,
    });

    renderApplicationHook();

    expect(selectProvider).not.toHaveBeenCalled();
  });

  it("re-applies for the next session when the page switches sessions", () => {
    window.localStorage.setItem(
      WORKSPACE_PROVIDER_DEFAULTS_STORAGE_KEY,
      JSON.stringify({ author: "pi", reviewer: "codex", reviewerEnabled: false }),
    );
    const { rerender } = renderApplicationHook();

    expect(selectProvider).toHaveBeenCalledWith("author", "pi");

    act(() => {
      useWorkspaceStore.setState({ sessionId: "session_002" });
    });
    rerender({
      ws: { selectProvider },
      sessionId: "session_002",
      connected: true,
      providerEditable: true,
      providers: { author: "claude_code", reviewer: "codex" },
      isTakeover: false,
    });

    expect(selectProvider).toHaveBeenCalledTimes(2);
  });
});
