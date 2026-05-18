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
    startGeneration: vi.fn(),
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

  it("shows provider command progress in the execution tab", async () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      providerStatus: "running",
      executionEvents: [
        {
          event_id: "command_cmd_001",
          kind: "command",
          status: "completed",
          title: "Command completed",
          detail: "exit code 0",
          command: "pwd",
          cwd: "/tmp/repo",
          output: "/tmp/repo\n",
          exit_code: 0,
        },
      ],
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);
    await userEvent.click(screen.getByRole("button", { name: "执行" }));

    expect(screen.getByText("运行中")).toBeInTheDocument();
    expect(screen.getByText("pwd")).toBeInTheDocument();
    expect(screen.getByText("/tmp/repo")).toBeInTheDocument();
    expect(screen.getByText(/exit code 0/)).toBeInTheDocument();
  });

  it("starts generation from a prepared workspace without requiring typed input", async () => {
    const api = mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "prepare_context",
      messages: [
        {
          id: "msg_001",
          role: "system",
          content: "Workspace 生成任务已准备",
          checkpoint_id: null,
          created_at: "2026-05-18T00:00:00Z",
        },
      ],
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    await userEvent.click(screen.getByRole("button", { name: "开始生成" }));

    expect(api.startGeneration).toHaveBeenCalledTimes(1);
    expect(api.sendMessage).not.toHaveBeenCalled();
  });

  it("marks fast intermediate stages as visited in the flow rail", () => {
    mockWorkspaceWs();
    useWorkspaceStore.setState({
      stage: "human_confirm",
      visitedStages: ["prepare_context", "running", "cross_review", "human_confirm"],
    });

    render(<WorkspacePage sessionId="workspace_session_0001" onBack={vi.fn()} />);

    expect(screen.getByLabelText("运行中 已经过")).toBeInTheDocument();
    expect(screen.getByLabelText("交叉审查 已经过")).toBeInTheDocument();
    expect(screen.getByLabelText("人工确认 当前阶段")).toBeInTheDocument();
  });
});
