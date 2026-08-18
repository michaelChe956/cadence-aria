import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { GroupChatSession, RoleInstance } from "../../api/groupChat";
import type { ProviderHealthResponse } from "../../api/types";
import { useProviderAvailabilityStore } from "../../state/provider-availability-store";
import { RoleBar } from "./RoleBar";

vi.mock("../../api/groupChat", async (importOriginal) => {
  const actual = await importOriginal<typeof import("../../api/groupChat")>();
  return { ...actual, addGroupChatRole: vi.fn() };
});

import { addGroupChatRole } from "../../api/groupChat";

const addRole = vi.mocked(addGroupChatRole);

const roles: RoleInstance[] = [
  {
    id: "author-1",
    role_key: "author",
    provider: "claude_code",
    display_name: "需求作者",
    permission_mode: "auto",
    seen_cursor: 0,
    injection_watermark: 0,
  },
  {
    id: "reviewer-1",
    role_key: "reviewer",
    provider: "codex",
    display_name: "审核员",
    permission_mode: "supervised",
    seen_cursor: 0,
    injection_watermark: 0,
  },
  {
    id: "researcher-1",
    role_key: "researcher",
    provider: "pi",
    display_name: "资料研究员",
    permission_mode: "supervised",
    seen_cursor: 0,
    injection_watermark: 0,
  },
];

const session = {
  id: "session-1",
  project_id: "project-1",
  issue_id: "issue-1",
  status: "active",
  roles,
  artifact_lines: [],
  created_at: "2026-08-18T00:00:00Z",
  updated_at: "2026-08-18T00:00:00Z",
} satisfies GroupChatSession;

function setProviderHealth(): void {
  const snapshot: ProviderHealthResponse = {
    schema_version: 1,
    generation: 1,
    checked_at: "2026-08-18T00:00:00Z",
    state_status: "ready",
    state_error: null,
    real_workflow_blocked: false,
    test_provider_enabled: false,
    providers: [
      {
        provider: "claude_code",
        display_name: "Claude Code CLI",
        available: true,
        version: "1.0.0",
        reason_code: null,
        reason: null,
        checked_at: "2026-08-18T00:00:00Z",
        install_hint: "",
      },
      {
        provider: "codex",
        display_name: "Codex CLI",
        available: true,
        version: "1.0.0",
        reason_code: null,
        reason: null,
        checked_at: "2026-08-18T00:00:00Z",
        install_hint: "",
      },
      {
        provider: "pi",
        display_name: "Pi CLI",
        available: true,
        version: "1.0.0",
        reason_code: null,
        reason: null,
        checked_at: "2026-08-18T00:00:00Z",
        install_hint: "",
      },
    ],
  };
  useProviderAvailabilityStore.setState({
    snapshot,
    loadStatus: "loaded",
    realWorkflowBlocked: false,
    testProviderEnabled: false,
  });
}

afterEach(() => {
  addRole.mockReset();
  useProviderAvailabilityStore.getState().reset();
});

describe("RoleBar", () => {
  it("渲染默认阵容、provider 绑定和只读提示", () => {
    setProviderHealth();
    render(<RoleBar sessionId="session-1" roles={roles} onSessionUpdated={vi.fn()} />);

    expect(screen.getByRole("region", { name: "群聊角色" })).toBeInTheDocument();
    expect(screen.getAllByText("需求作者").length).toBeGreaterThan(0);
    expect(screen.getAllByText("审核员").length).toBeGreaterThan(0);
    expect(screen.getAllByText("资料研究员").length).toBeGreaterThan(0);
    expect(screen.getByRole("region", { name: "群聊角色" })).toHaveTextContent(
      "Claude Code CLI",
    );
    expect(screen.getByRole("region", { name: "群聊角色" })).toHaveTextContent(
      "Codex CLI",
    );
    expect(screen.getByRole("region", { name: "群聊角色" })).toHaveTextContent(
      "Pi CLI",
    );
    expect(screen.getAllByText("只读")).toHaveLength(2);
    expect(screen.getAllByText("只读角色不可写入产物")).toHaveLength(2);
  });

  it("支持添加同角色多个实例，并展示新实例的 provider 绑定", async () => {
    setProviderHealth();
    const user = userEvent.setup();
    const addedRole: RoleInstance = {
      ...roles[0],
      id: "author-2",
      provider: "codex",
      display_name: "另一位作者",
    };
    const updatedSession = { ...session, roles: [...roles, addedRole] };
    addRole.mockResolvedValue(updatedSession);
    const onSessionUpdated = vi.fn();

    const { rerender } = render(
      <RoleBar sessionId="session-1" roles={roles} onSessionUpdated={onSessionUpdated} />,
    );
    await user.click(screen.getByRole("button", { name: "添加角色" }));
    const dialog = screen.getByRole("dialog", { name: "添加角色" });
    await user.selectOptions(within(dialog).getByLabelText("角色"), "author");
    await user.selectOptions(within(dialog).getByLabelText("Provider"), "codex");
    await user.type(within(dialog).getByLabelText("显示名（可选）"), "另一位作者");
    await user.click(within(dialog).getByRole("button", { name: "添加" }));

    await waitFor(() => {
      expect(addRole).toHaveBeenCalledWith("session-1", {
        role_key: "author",
        provider: "codex",
        display_name: "另一位作者",
      });
    });
    expect(onSessionUpdated).toHaveBeenCalledWith(updatedSession);
    rerender(
      <RoleBar
        sessionId="session-1"
        roles={updatedSession.roles}
        onSessionUpdated={onSessionUpdated}
      />,
    );
    expect(screen.getByText("另一位作者")).toBeInTheDocument();
    const authorCards = screen.getAllByTestId(/^role-card-author-/u);
    expect(authorCards).toHaveLength(2);
    expect(authorCards[1]).toHaveTextContent("Codex CLI");
  });

  it("提交失败时保留对话框并显示错误", async () => {
    setProviderHealth();
    addRole.mockRejectedValue(new Error("角色添加失败"));
    const user = userEvent.setup();
    render(<RoleBar sessionId="session-1" roles={roles} onSessionUpdated={vi.fn()} />);

    await user.click(screen.getByRole("button", { name: "添加角色" }));
    const dialog = screen.getByRole("dialog", { name: "添加角色" });
    await user.click(within(dialog).getByRole("button", { name: "添加" }));

    expect(await screen.findByRole("alert")).toHaveTextContent("角色添加失败");
    expect(screen.getByRole("dialog", { name: "添加角色" })).toBeInTheDocument();
  });
});
