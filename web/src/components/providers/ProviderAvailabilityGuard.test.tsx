import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ProviderHealthEntry, ProviderHealthResponse } from "../../api/types";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { ProviderAvailabilityGuard } from "./ProviderAvailabilityGuard";

const originalActions = {
  load: useProviderAvailabilityStore.getState().load,
  recheck: useProviderAvailabilityStore.getState().recheck,
  reset: useProviderAvailabilityStore.getState().reset,
};

function entry(
  provider: "claude_code" | "codex",
  available: boolean,
  overrides: Partial<ProviderHealthEntry> = {},
): ProviderHealthEntry {
  return {
    provider,
    display_name: provider === "claude_code" ? "Claude Code" : "Codex",
    available,
    version: available ? "1.0.0" : null,
    reason_code: available ? null : "command_missing",
    reason: available ? null : `${provider} command not found`,
    checked_at: "2026-07-14T00:00:00Z",
    install_hint: `Install ${provider}`,
    ...overrides,
  };
}

function setSnapshot(health: ProviderHealthResponse) {
  useProviderAvailabilityStore.setState({
    snapshot: health,
    loadStatus: "loaded",
    generation: health.generation,
    stateStatus: health.state_status,
    stateError: health.state_error,
    realWorkflowBlocked: health.real_workflow_blocked,
    testProviderEnabled: health.test_provider_enabled,
  });
}

function snapshot(
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
    providers: [entry("claude_code", true), entry("codex", false)],
    ...overrides,
  };
}

function renderGuard() {
  return render(
    <ProviderAvailabilityGuard>
      <button type="button">业务操作</button>
    </ProviderAvailabilityGuard>,
  );
}

describe("ProviderAvailabilityGuard", () => {
  beforeEach(() => {
    originalActions.reset();
    useProviderAvailabilityStore.setState(originalActions);
  });

  it("loads provider availability once when the initial status is idle", async () => {
    const load = vi.fn().mockImplementation(() => {
      useProviderAvailabilityStore.setState({ loadStatus: "loading" });
      return Promise.resolve();
    });
    useProviderAvailabilityStore.setState({ load });

    const { rerender } = renderGuard();
    rerender(
      <ProviderAvailabilityGuard>
        <button type="button">业务操作</button>
      </ProviderAvailabilityGuard>,
    );

    await waitFor(() => expect(load).toHaveBeenCalledTimes(1));
  });

  it.each([
    {
      name: "load status is already loaded",
      prepare: () => useProviderAvailabilityStore.setState({ loadStatus: "loaded" }),
    },
    {
      name: "load status is already error",
      prepare: () =>
        useProviderAvailabilityStore.setState({
          loadStatus: "error",
          error: "network offline",
        }),
    },
    {
      name: "a snapshot already exists",
      prepare: () => {
        setSnapshot(snapshot());
        useProviderAvailabilityStore.setState({ loadStatus: "idle" });
      },
    },
  ])("does not load again when $name", ({ prepare }) => {
    const load = vi.fn().mockResolvedValue(undefined);
    prepare();
    useProviderAvailabilityStore.setState({ load });

    const { rerender } = renderGuard();
    rerender(
      <ProviderAvailabilityGuard>
        <button type="button">业务操作</button>
      </ProviderAvailabilityGuard>,
    );

    expect(load).not.toHaveBeenCalled();
  });

  it("shows a non-interactive detection state and does not mount children while loading", () => {
    useProviderAvailabilityStore.setState({ loadStatus: "loading" });

    renderGuard();

    expect(screen.getByRole("status")).toHaveTextContent(
      "正在检测 Claude Code 与 Codex",
    );
    expect(screen.queryByRole("button", { name: "业务操作" })).not.toBeInTheDocument();
  });

  it("renders children when at least one real provider is available", () => {
    const health = snapshot();
    setSnapshot(health);

    renderGuard();

    expect(screen.getByRole("button", { name: "业务操作" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("blocks the app with both real provider reasons and install hints", () => {
    setSnapshot(
      snapshot({
        real_workflow_blocked: true,
        providers: [
          entry("claude_code", false, {
            reason: "Claude command missing",
            install_hint: "Install Claude Code from the official package",
          }),
          entry("codex", false, {
            reason: "Codex command missing",
            install_hint: "Install Codex from the official package",
          }),
        ],
      }),
    );

    renderGuard();

    const dialog = screen.getByRole("dialog", { name: "需要安装或修复 Provider" });
    expect(dialog).toHaveAttribute("aria-modal", "true");
    expect(dialog).toHaveTextContent("Claude Code");
    expect(dialog).toHaveTextContent("Claude command missing");
    expect(dialog).toHaveTextContent("Install Claude Code from the official package");
    expect(dialog).toHaveTextContent("Codex command missing");
    expect(dialog).toHaveTextContent("Install Codex from the official package");
    expect(screen.queryByRole("button", { name: "业务操作" })).not.toBeInTheDocument();
  });

  it("does not expose close or bypass actions and Escape cannot dismiss the dialog", () => {
    setSnapshot(
      snapshot({
        real_workflow_blocked: true,
        providers: [entry("claude_code", false), entry("codex", false)],
      }),
    );
    renderGuard();

    fireEvent.keyDown(document, { key: "Escape" });

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /关闭|取消|跳过|稍后|绕过/ }),
    ).not.toBeInTheDocument();
  });

  it("moves focus to the recheck action when the blocking dialog appears", () => {
    setSnapshot(
      snapshot({
        real_workflow_blocked: true,
        providers: [entry("claude_code", false), entry("codex", false)],
      }),
    );

    renderGuard();

    expect(screen.getByRole("button", { name: "重新检测" })).toHaveFocus();
  });

  it("stays blocked for degraded state even when old provider entries are available", () => {
    setSnapshot(
      snapshot({
        state_status: "degraded",
        state_error: "failed to persist provider health snapshot",
        real_workflow_blocked: true,
        providers: [entry("claude_code", true), entry("codex", true)],
      }),
    );

    renderGuard();

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "业务操作" })).not.toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent(
      "状态存储降级：failed to persist provider health snapshot",
    );
  });

  it("fails closed after the initial load error and lets the user recheck", () => {
    const recheck = vi.fn().mockResolvedValue(undefined);
    useProviderAvailabilityStore.setState({
      loadStatus: "error",
      error: "network offline",
      recheck,
    });

    renderGuard();

    expect(
      screen.getByRole("dialog", { name: "Provider 状态读取失败" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("network offline");
    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));
    expect(recheck).toHaveBeenCalledTimes(1);
  });

  it("disables the recheck button while a request is running", () => {
    setSnapshot(
      snapshot({
        real_workflow_blocked: true,
        providers: [entry("claude_code", false), entry("codex", false)],
      }),
    );
    useProviderAvailabilityStore.setState({ recheckStatus: "rechecking" });

    renderGuard();

    expect(screen.getByRole("button", { name: "正在重新检测" })).toBeDisabled();
  });

  it("immediately renders children when recheck returns an unblocked generation", () => {
    setSnapshot(
      snapshot({
        real_workflow_blocked: true,
        providers: [entry("claude_code", false), entry("codex", false)],
      }),
    );
    const recheck = vi.fn().mockImplementation(() => {
      setSnapshot(snapshot({ generation: 2 }));
      return Promise.resolve();
    });
    useProviderAvailabilityStore.setState({ recheck });
    renderGuard();

    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));

    expect(screen.getByRole("button", { name: "业务操作" })).toBeInTheDocument();
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("keeps blocking and refreshes diagnostics when recheck is still blocked", () => {
    setSnapshot(
      snapshot({
        real_workflow_blocked: true,
        providers: [entry("claude_code", false), entry("codex", false)],
      }),
    );
    const recheck = vi.fn().mockImplementation(() => {
      setSnapshot(
        snapshot({
          generation: 2,
          checked_at: "2026-07-14T00:00:02Z",
          real_workflow_blocked: true,
          providers: [
            entry("claude_code", false, {
              reason: "still missing Claude",
              version: "2.0.0",
            }),
            entry("codex", false, { reason: "still missing Codex" }),
          ],
        }),
      );
      return Promise.resolve();
    });
    useProviderAvailabilityStore.setState({ recheck });
    renderGuard();

    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));

    expect(screen.getByRole("dialog")).toHaveTextContent("still missing Claude");
    expect(screen.getByRole("dialog")).toHaveTextContent("版本：2.0.0");
    expect(screen.getByRole("dialog")).toHaveTextContent("generation 2");
  });

  it("preserves the blocked snapshot and reports a recheck network error", () => {
    setSnapshot(
      snapshot({
        real_workflow_blocked: true,
        providers: [entry("claude_code", false), entry("codex", false)],
      }),
    );
    const recheck = vi.fn().mockImplementation(() => {
      useProviderAvailabilityStore.setState({
        recheckStatus: "error",
        error: "temporary network failure",
      });
      return Promise.resolve();
    });
    useProviderAvailabilityStore.setState({ recheck });
    renderGuard();

    fireEvent.click(screen.getByRole("button", { name: "重新检测" }));

    expect(screen.getByRole("dialog")).toHaveTextContent("generation 1");
    expect(screen.getByRole("alert")).toHaveTextContent("temporary network failure");
  });

  it("shows a non-destructive recheck error over unblocked children", () => {
    setSnapshot(snapshot());
    useProviderAvailabilityStore.setState({
      recheckStatus: "error",
      error: "temporary network failure",
    });

    renderGuard();

    expect(screen.getByRole("button", { name: "业务操作" })).toBeInTheDocument();
    expect(screen.getByRole("alert")).toHaveTextContent("temporary network failure");
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });

  it("does not let an enabled Fake provider unblock or appear in the install list", () => {
    setSnapshot(
      snapshot({
        real_workflow_blocked: true,
        test_provider_enabled: true,
        providers: [entry("claude_code", false), entry("codex", false)],
      }),
    );

    renderGuard();

    expect(screen.getByRole("dialog")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "业务操作" })).not.toBeInTheDocument();
    expect(screen.queryByText("Fake")).not.toBeInTheDocument();
  });
});
