import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { WorkspacePage } from "./WorkspacePage";

vi.mock("../hooks/useWorkspaceWs", () => ({
  useWorkspaceWs: vi.fn(),
}));

type WorkspaceWsApi = ReturnType<typeof useWorkspaceWs>;

function mockWorkspaceWs(overrides: Partial<WorkspaceWsApi> = {}) {
  const api: WorkspaceWsApi = {
    sendMessage: vi.fn(),
    rollback: vi.fn(),
    confirm: vi.fn(),
    abort: vi.fn(),
    selectProvider: vi.fn(),
    respondPermission: vi.fn(),
    connectionStatus: "connected",
    ...overrides,
  };
  vi.mocked(useWorkspaceWs).mockReturnValue(api);
  return api;
}

describe("WorkspacePage", () => {
  beforeEach(() => {
    useWorkspaceStore.getState().reset();
    vi.clearAllMocks();
  });

  it("renders permission request card and sends approval", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      pendingPermissions: [
        {
          id: "perm_001",
          tool_name: "bash",
          description: "Run cargo test",
          risk_level: "medium",
        },
      ],
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByText("bash")).toBeInTheDocument();
    expect(screen.getByText("Run cargo test")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "允许" }));

    expect(api.respondPermission).toHaveBeenCalledWith("perm_001", true, undefined);
  });
});
