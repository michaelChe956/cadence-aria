import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  CodingRoleProviderConfigSnapshot,
  ProviderHealthEntry,
  ProviderHealthResponse,
  RealProviderName,
} from "../../api/types";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { CodingProviderConfigPanel } from "./CodingProviderConfigPanel";

function providerEntry(
  provider: RealProviderName,
  available: boolean,
): ProviderHealthEntry {
  return {
    provider,
    display_name: provider === "claude_code" ? "Claude Code CLI" : "Codex CLI",
    available,
    version: available ? "1.0.0" : null,
    reason_code: available ? null : "command_missing",
    reason: available ? null : `${provider} 未安装`,
    checked_at: "2026-07-14T00:00:00Z",
    install_hint: `安装 ${provider}`,
  };
}

function setProviderHealth(
  claudeAvailable: boolean,
  codexAvailable: boolean,
  overrides: Partial<ProviderHealthResponse> = {},
) {
  const snapshot: ProviderHealthResponse = {
    schema_version: 1,
    generation: 1,
    checked_at: "2026-07-14T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: !claudeAvailable && !codexAvailable,
    test_provider_enabled: false,
    providers: [
      providerEntry("claude_code", claudeAvailable),
      providerEntry("codex", codexAvailable),
    ],
    ...overrides,
  };
  useProviderAvailabilityStore.setState({
    snapshot,
    loadStatus: "loaded",
    realWorkflowBlocked: snapshot.real_workflow_blocked,
    testProviderEnabled: snapshot.test_provider_enabled,
  });
}

function roleSnapshot(
  overrides: Partial<CodingRoleProviderConfigSnapshot> = {},
): CodingRoleProviderConfigSnapshot {
  return {
    coder: "claude_code",
    code_reviewer: "codex",
    internal_reviewer: "claude_code",
    review_rounds: 1,
    permission_modes: {
      coder: "supervised",
      code_reviewer: "supervised",
      internal_reviewer: "supervised",
    },
    ...overrides,
  };
}

function renderPanel({
  snapshot = roleSnapshot(),
  attemptScope = "work_item",
  lockedRole = null,
  configLocked = false,
  onSelect = vi.fn(),
}: {
  snapshot?: CodingRoleProviderConfigSnapshot;
  attemptScope?: "work_item" | "work_item_group";
  lockedRole?: "coder" | "code_reviewer" | "internal_reviewer" | null;
  configLocked?: boolean;
  onSelect?: ReturnType<typeof vi.fn>;
} = {}) {
  render(
    <CodingProviderConfigPanel
      snapshot={snapshot}
      attemptScope={attemptScope}
      lockedRole={lockedRole}
      configLocked={configLocked}
      maxAutoRework={2}
      onSelect={onSelect}
      onPermissionModeSelect={vi.fn()}
      onMaxAutoReworkSelect={vi.fn()}
    />,
  );
  return { onSelect };
}

afterEach(() => {
  useProviderAvailabilityStore.getState().reset();
});

describe("CodingProviderConfigPanel", () => {
  it("uses catalog labels and disabled guidance for ordinary roles", async () => {
    setProviderHealth(false, true);
    const { onSelect } = renderPanel();
    const coder = screen.getByRole("group", { name: "Coder Provider 配置" });

    expect(
      within(coder).getByRole("button", { name: "将 Coder 切换为 Claude Code CLI" }),
    ).toBeDisabled();
    expect(within(coder).getByText("claude_code 未安装")).toBeInTheDocument();
    expect(within(coder).getByText("安装 claude_code")).toBeInTheDocument();

    await userEvent.click(
      within(coder).getByRole("button", { name: "将 Coder 切换为 Codex CLI" }),
    );
    expect(onSelect).toHaveBeenCalledWith("coder", "codex");
  });

  it("uses the same catalog for the work item group final reviewer", () => {
    setProviderHealth(false, true);
    renderPanel({ attemptScope: "work_item_group" });
    const groupReviewer = screen.getByRole("group", {
      name: "GroupFinalReview Provider 配置",
    });

    expect(
      within(groupReviewer).getByRole("button", {
        name: "将 GroupFinalReview 切换为 Claude Code CLI",
      }),
    ).toBeDisabled();
    expect(
      within(groupReviewer).getByRole("button", {
        name: "将 GroupFinalReview 切换为 Codex CLI",
      }),
    ).toBeEnabled();
  });

  it("keeps an unavailable saved configuration visible without fallback", () => {
    setProviderHealth(false, true);
    renderPanel({ configLocked: true });
    const coder = screen.getByRole("group", { name: "Coder Provider 配置" });

    expect(coder).toHaveTextContent("claude_code");
    expect(
      within(coder).getByRole("button", { name: "将 Coder 切换为 Claude Code CLI" }),
    ).toHaveAttribute("aria-pressed", "true");
    expect(
      within(coder).getByRole("button", { name: "将 Coder 切换为 Codex CLI" }),
    ).toBeDisabled();
  });

  it("hides Fake in product mode except for a disabled current-value placeholder", () => {
    setProviderHealth(true, true);
    renderPanel({ snapshot: roleSnapshot({ coder: "fake" }) });

    const coder = screen.getByRole("group", { name: "Coder Provider 配置" });
    const reviewer = screen.getByRole("group", { name: "Code Reviewer Provider 配置" });
    expect(within(coder).getByRole("button", { name: "将 Coder 切换为 Fake" })).toBeDisabled();
    expect(within(reviewer).queryByRole("button", { name: /Fake/u })).not.toBeInTheDocument();
  });

  it("shows Fake as an available option only when test mode is enabled", () => {
    setProviderHealth(true, true, { test_provider_enabled: true });
    renderPanel();
    const reviewer = screen.getByRole("group", { name: "Code Reviewer Provider 配置" });

    expect(
      within(reviewer).getByRole("button", { name: "将 Code Reviewer 切换为 Fake" }),
    ).toBeEnabled();
  });
});
