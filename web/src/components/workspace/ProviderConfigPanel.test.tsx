import { fireEvent, render, screen, within } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import type {
  ProviderHealthEntry,
  ProviderHealthResponse,
  RealProviderName,
} from "../../api/types";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { ProviderConfigPanel } from "./ProviderConfigPanel";

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

function piEntry(available: boolean): ProviderHealthEntry {
  return {
    provider: "pi",
    display_name: "Pi",
    available,
    version: available ? "0.83.0" : null,
    reason_code: available ? null : "command_missing",
    reason: available ? null : "pi 未安装",
    checked_at: "2026-07-31T00:00:00Z",
    install_hint: "安装 pi",
  };
}

function setHealthWithPi(piAvailable: boolean) {
  useProviderAvailabilityStore.setState({
    snapshot: {
      schema_version: 1,
      generation: 1,
      checked_at: "2026-07-31T00:00:00Z",
      state_status: "ready",
      state_error: null,
      real_workflow_blocked: false,
      test_provider_enabled: false,
      providers: [
        providerEntry("claude_code", true),
        providerEntry("codex", true),
        piEntry(piAvailable),
      ],
    },
  });
}

afterEach(() => {
  useProviderAvailabilityStore.getState().reset();
});

describe("ProviderConfigPanel", () => {
  it("author 可选 Pi，且 Pi 角色只提供 Auto", () => {
    setHealthWithPi(true);
    const onPermissionModeSelect = vi.fn();
    render(
      <ProviderConfigPanel
        providers={{ author: "pi", reviewer: "codex" }}
        editable
        onSelectProvider={() => {}}
        reviewerEnabled
        onToggleReviewer={() => {}}
        permissionModes={{ author: "auto", reviewer: "auto" }}
        onPermissionModeSelect={onPermissionModeSelect}
      />,
    );

    const authorSelect = screen.getByLabelText("Author");
    expect(within(authorSelect).getByRole("option", { name: /Pi/ })).toBeTruthy();

    const authorModes = screen.getByTestId("author-permission-mode");
    expect(within(authorModes).getByRole("button", { name: "Auto" })).toBeTruthy();
    expect(
      within(authorModes).queryByRole("button", { name: "Supervised" }),
    ).toBeNull();

    const reviewerModes = screen.getByTestId("reviewer-permission-mode");
    expect(
      within(reviewerModes).getByRole("button", { name: "Supervised" }),
    ).toBeTruthy();
  });

  it("选择 Pi 时将权限模式重置为 Auto", () => {
    setHealthWithPi(true);
    const onSelectProvider = vi.fn();
    const onPermissionModeSelect = vi.fn();
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable
        onSelectProvider={onSelectProvider}
        reviewerEnabled
        onToggleReviewer={() => {}}
        permissionModes={{ author: "supervised", reviewer: "supervised" }}
        onPermissionModeSelect={onPermissionModeSelect}
      />,
    );

    fireEvent.change(screen.getByLabelText("Author"), { target: { value: "pi" } });

    expect(onSelectProvider).toHaveBeenCalledWith("author", "pi");
    expect(onPermissionModeSelect).toHaveBeenCalledWith("author", "auto");
  });

  it("Pi 不可用时选项禁用且显示原因", () => {
    setHealthWithPi(false);
    render(
      <ProviderConfigPanel
        providers={{ author: "pi", reviewer: "codex" }}
        editable
        onSelectProvider={() => {}}
        reviewerEnabled
        onToggleReviewer={() => {}}
        permissionModes={{ author: "auto", reviewer: "auto" }}
        onPermissionModeSelect={() => {}}
      />,
    );

    expect(screen.getByText(/pi 未安装/)).toBeTruthy();
    const authorSelect = screen.getByLabelText("Author");
    const piOption = within(authorSelect).getByRole("option", { name: /Pi/ });
    expect(piOption).toBeDisabled();
  });

  it("renders author and reviewer selects with reviewer enabled", () => {
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={true}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Author")).toBeInTheDocument();
    expect(screen.getByLabelText("Reviewer")).toBeInTheDocument();
    expect(screen.getByLabelText("启用交叉审核")).toBeChecked();
    expect(screen.getByText("可编辑")).toBeInTheDocument();
  });

  it("shows a quality warning when reviewer is disabled", () => {
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={false}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    expect(screen.getByText("未启用交叉审核可能降低 artifact 质量")).toBeInTheDocument();
    expect(screen.queryByLabelText("Reviewer")).not.toBeInTheDocument();
  });

  it("disables provider controls when not editable", () => {
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={false}
        reviewerEnabled={true}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    expect(screen.getByLabelText("Author")).toBeDisabled();
    expect(screen.getByLabelText("Reviewer")).toBeDisabled();
    expect(screen.getByLabelText("启用交叉审核")).toBeDisabled();
    expect(screen.getByText("已锁定")).toBeInTheDocument();
  });

  it("emits provider, reviewer toggle, and review round changes", () => {
    setProviderHealth(true, true, { test_provider_enabled: true });
    const onSelectProvider = vi.fn();
    const onToggleReviewer = vi.fn();
    const onChangeRounds = vi.fn();
    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={true}
        rounds={1}
        onSelectProvider={onSelectProvider}
        onToggleReviewer={onToggleReviewer}
        onChangeRounds={onChangeRounds}
      />,
    );

    fireEvent.change(screen.getByLabelText("Author"), { target: { value: "fake" } });
    fireEvent.click(screen.getByLabelText("启用交叉审核"));
    fireEvent.click(screen.getByRole("button", { name: "高级配置" }));
    fireEvent.change(screen.getByLabelText("审核轮次"), { target: { value: "2" } });

    expect(onSelectProvider).toHaveBeenCalledWith("author", "fake");
    expect(onToggleReviewer).toHaveBeenCalledWith(false);
    expect(onChangeRounds).toHaveBeenCalledWith(2);
  });

  describe.each(["story", "design", "work_item_plan"] as const)(
    "%s workspace Provider catalog",
    (workspaceType) => {
      it("keeps unavailable real providers visible and disabled with guidance", () => {
        setProviderHealth(false, true);

        render(
          <ProviderConfigPanel
            providers={{ author: "codex", reviewer: "codex" }}
            editable={true}
            reviewerEnabled={true}
            onSelectProvider={vi.fn()}
            onToggleReviewer={vi.fn()}
          />,
        );

        const author = screen.getByLabelText<HTMLSelectElement>("Author");
        expect(workspaceType).toMatch(/story|design|work_item_plan/u);
        expect(within(author).getByRole("option", { name: "Claude Code CLI" })).toBeDisabled();
        expect(within(author).getByRole("option", { name: "Codex CLI" })).toBeEnabled();
        expect(screen.getAllByText("claude_code 未安装").length).toBeGreaterThan(0);
        expect(screen.getAllByText("安装 claude_code").length).toBeGreaterThan(0);
      });
    },
  );

  it("shows all real providers disabled when the real workflow is blocked", () => {
    setProviderHealth(false, false);

    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={true}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    const author = screen.getByLabelText<HTMLSelectElement>("Author");
    expect(within(author).getByRole("option", { name: "Claude Code CLI" })).toBeDisabled();
    expect(within(author).getByRole("option", { name: "Codex CLI" })).toBeDisabled();
  });

  it("allows selecting an available Provider from the shared catalog", () => {
    setProviderHealth(false, true);
    const onSelectProvider = vi.fn();

    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={true}
        onSelectProvider={onSelectProvider}
        onToggleReviewer={vi.fn()}
      />,
    );

    fireEvent.change(screen.getByLabelText("Author"), { target: { value: "codex" } });
    expect(onSelectProvider).toHaveBeenCalledWith("author", "codex");
  });

  it("keeps a locked Provider visible after it becomes unavailable", () => {
    setProviderHealth(false, true);

    render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={false}
        reviewerEnabled={true}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    expect(screen.getByLabelText<HTMLSelectElement>("Author")).toHaveValue("claude_code");
    expect(screen.getByLabelText("Author")).toBeDisabled();
  });

  it("hides Fake in product mode and shows it only in test mode", () => {
    setProviderHealth(true, true);
    const { rerender } = render(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={true}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    expect(within(screen.getByLabelText("Author")).queryByRole("option", { name: "Fake" })).not.toBeInTheDocument();

    setProviderHealth(true, true, { test_provider_enabled: true });
    rerender(
      <ProviderConfigPanel
        providers={{ author: "claude_code", reviewer: "codex" }}
        editable={true}
        reviewerEnabled={true}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    expect(within(screen.getByLabelText("Author")).getByRole("option", { name: "Fake" })).toBeEnabled();
  });

  it("shows a locked Fake only as the current product-mode placeholder", () => {
    setProviderHealth(true, true);

    render(
      <ProviderConfigPanel
        providers={{ author: "fake", reviewer: "codex" }}
        editable={false}
        reviewerEnabled={true}
        onSelectProvider={vi.fn()}
        onToggleReviewer={vi.fn()}
      />,
    );

    const author = screen.getByLabelText<HTMLSelectElement>("Author");
    const reviewer = screen.getByLabelText<HTMLSelectElement>("Reviewer");
    expect(author).toHaveValue("fake");
    expect(within(author).getByRole("option", { name: "Fake" })).toBeDisabled();
    expect(within(reviewer).queryByRole("option", { name: "Fake" })).not.toBeInTheDocument();
  });
});
