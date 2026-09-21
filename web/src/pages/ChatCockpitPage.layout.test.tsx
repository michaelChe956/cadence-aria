import { fireEvent, screen, within } from "@testing-library/react";
import { describe, expect, it, vi } from "vitest";
import type * as ApiClient from "../api/client";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import { readCockpitSettings } from "../state/cockpit-settings";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { ChatCockpitPage } from "./ChatCockpitPage";
import {
  cockpitInbox,
  cockpitObservedRecords,
  gateItem,
  installCockpitPageTestHooks,
  renderCockpit,
  watchSession,
} from "./ChatCockpitPage.test-utils";

vi.mock("../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
  useWorkspaceWs: vi.fn(),
}));
vi.mock("../hooks/useUnloadGuard", () => ({ useUnloadGuard: vi.fn() }));
vi.mock("../api/client", async (importOriginal) => ({
  ...(await importOriginal<typeof ApiClient>()),
  takeoverWorkspaceSession: vi.fn(),
}));
vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceArtifactVersion: vi.fn(),
  fetchWorkspaceEventOutput: vi.fn(),
  fetchWorkspaceNodeDetail: vi.fn(),
  fetchWorkspacePrompt: vi.fn(),
}));
vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string }) => <div data-testid="monaco-viewer">{value}</div>,
}));
vi.mock("../components/cockpit/CockpitShell", () => ({
  useCockpitShellInbox: () => cockpitInbox,
  useCockpitInboxPulse: () => false,
  useCockpitSessionWatch: () => watchSession,
  useCockpitSettings: () => readCockpitSettings(),
  useCockpitObservedRecords: () => cockpitObservedRecords,
  useCockpitSettingsSlotRef: () => () => undefined,
}));

describe("ChatCockpitPage 布局（UI-A）", () => {
  installCockpitPageTestHooks();

  it("待处理默认收起为抽屉：只剩入口与计数提示，展开才是收件箱", () => {
    cockpitInbox.push(gateItem("session_001", "gate_1"));

    renderCockpit();

    const drawer = screen.getByTestId("cockpit-inbox-drawer");
    expect(drawer).toHaveAttribute("data-state", "closed");
    expect(drawer).toHaveClass("hidden");
    expect(screen.queryByTestId("cockpit-inbox-drawer-backdrop")).toBeNull();
    expect(screen.getByTestId("cockpit-inbox-drawer-count")).toHaveTextContent("1");

    fireEvent.click(screen.getByTestId("cockpit-inbox-drawer-trigger"));

    expect(drawer).toHaveAttribute("data-state", "open");
    expect(drawer).toHaveClass("flex");
    expect(drawer).toContainElement(screen.getByTestId("cockpit-inbox"));
    expect(screen.getByTestId("cockpit-inbox-drawer-backdrop")).toBeInTheDocument();
  });

  it("收起抽屉不丢待处理状态，关闭钮与遮罩都能收起", () => {
    cockpitInbox.push(gateItem("session_001", "gate_1"));

    renderCockpit();

    const trigger = screen.getByTestId("cockpit-inbox-drawer-trigger");
    fireEvent.click(trigger);
    fireEvent.click(screen.getByLabelText("选择 门禁等待"));
    expect(screen.getByLabelText("选择 门禁等待")).toBeChecked();

    fireEvent.click(screen.getByRole("button", { name: "收起待处理抽屉" }));
    expect(screen.getByTestId("cockpit-inbox-drawer")).toHaveAttribute("data-state", "closed");
    expect(screen.getByLabelText("选择 门禁等待")).toBeChecked();

    fireEvent.click(trigger);
    expect(screen.getByLabelText("选择 门禁等待")).toBeChecked();
    fireEvent.click(screen.getByTestId("cockpit-inbox-drawer-backdrop"));
    expect(screen.getByTestId("cockpit-inbox-drawer")).toHaveAttribute("data-state", "closed");
  });

  it("待处理退出主区，对话流成为主区域（权重高于执行流）", () => {
    cockpitInbox.push(gateItem("session_001", "gate_1"));

    renderCockpit();

    const main = screen.getByTestId("cockpit-main-region");
    const conversation = screen.getByTestId("cockpit-conversation-flow");
    const execution = screen.getByTestId("cockpit-execution-flow");
    expect(within(main).getByTestId("cockpit-conversation-flow")).toBeInTheDocument();
    expect(within(main).getByTestId("cockpit-execution-flow")).toBeInTheDocument();
    expect(within(main).queryByTestId("cockpit-inbox")).toBeNull();
    expect(main.className).toContain("flex-col");
    expect(conversation.className).toContain("flex-[2]");
    expect(execution.className).toContain("flex-1");
  });

  it("设置入口让位到页头，spec（产物审核）面板收起钮不被遮挡", () => {
    useWorkspaceStore.setState({ workspaceType: "story" });

    renderCockpit();

    expect(screen.getByTestId("cockpit-settings-slot").closest("header")).not.toBeNull();

    fireEvent.click(screen.getByTestId("cockpit-artifact-review-tab"));
    expect(screen.getByTestId("artifact-review-panel")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "收起面板" }));
    expect(screen.queryByTestId("artifact-review-panel")).toBeNull();
  });

  it("无待处理项时入口不显示计数徽标", () => {
    renderCockpit();

    expect(screen.getByTestId("cockpit-inbox-drawer-trigger")).toBeInTheDocument();
    expect(screen.queryByTestId("cockpit-inbox-drawer-count")).toBeNull();
  });
});
