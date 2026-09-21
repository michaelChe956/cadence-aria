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
    const backdrop = screen.getByTestId("cockpit-inbox-drawer-backdrop");
    expect(drawer).toHaveAttribute("data-state", "closed");
    // 动效收起态：屏外 + 不可见（不可聚焦/不可交互），遮罩淡出并放行点击。
    expect(drawer).toHaveClass("invisible", "translate-x-full", "opacity-0");
    expect(backdrop).toHaveClass("opacity-0", "pointer-events-none");
    expect(screen.getByTestId("cockpit-inbox-drawer-count")).toHaveTextContent("1");

    fireEvent.click(screen.getByTestId("cockpit-inbox-drawer-trigger"));

    expect(drawer).toHaveAttribute("data-state", "open");
    expect(drawer).toHaveClass("visible", "translate-x-0", "opacity-100");
    expect(drawer).toContainElement(screen.getByTestId("cockpit-inbox"));
    expect(backdrop).toHaveClass("opacity-100");
  });

  it("抽屉开合走滑入/淡出过渡，减弱动效下静止", () => {
    cockpitInbox.push(gateItem("session_001", "gate_1"));

    renderCockpit();

    // 视觉契约（先例：ImageCreatePage 抽屉 / StageStepper）：
    // transform/opacity 过渡 200ms ease-out（150–250ms 档）+ motion-reduce 降级。
    const drawer = screen.getByTestId("cockpit-inbox-drawer");
    const backdrop = screen.getByTestId("cockpit-inbox-drawer-backdrop");
    expect(drawer).toHaveClass(
      "transition-[transform,opacity,visibility]",
      "duration-200",
      "ease-out",
      "motion-reduce:transition-none",
    );
    expect(backdrop).toHaveClass(
      "transition-opacity",
      "duration-200",
      "ease-out",
      "motion-reduce:transition-none",
    );
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

  it("执行流退居左侧窄栏，对话流占满余宽（自上而下的紧凑条目）", () => {
    cockpitInbox.push(gateItem("session_001", "gate_1"));

    renderCockpit();

    const main = screen.getByTestId("cockpit-main-region");
    const columns = screen.getByTestId("cockpit-main-columns");
    const conversation = screen.getByTestId("cockpit-conversation-flow");
    const execution = screen.getByTestId("cockpit-execution-flow");
    expect(within(main).getByTestId("cockpit-conversation-flow")).toBeInTheDocument();
    expect(within(main).getByTestId("cockpit-execution-flow")).toBeInTheDocument();
    expect(within(main).queryByTestId("cockpit-inbox")).toBeNull();
    // 左窄栏 + 右主区的横向双栏：执行流 ≤200px 纵向窄条，对话流吃掉剩余宽度。
    expect(columns.className).toContain("flex-row");
    expect(within(columns).getByTestId("cockpit-execution-flow")).toBeInTheDocument();
    expect(within(columns).getByTestId("cockpit-conversation-flow")).toBeInTheDocument();
    expect(columns.firstElementChild).toBe(execution);
    expect(execution.className).toContain("w-48");
    expect(execution.className).toContain("shrink-0");
    expect(conversation.className).toContain("flex-1");
  });

  it("滚动体系单一归属：页面根不滚，对话流视图仅列表一个滚动容器", () => {
    useWorkspaceStore.setState({ workspaceType: "story" });

    renderCockpit();

    // 页面根 h-screen overflow-hidden：文档级不出现滚动条
    expect(screen.getByTestId("cockpit-page").className).toContain("overflow-hidden");

    // 对话流视图：ChatEntryList 容器是唯一原生滚动容器（其余区域不自滚）
    const conversation = screen.getByTestId("cockpit-conversation-flow");
    expect(conversation.querySelectorAll('[class*="overflow-auto"]').length).toBe(1);

    // 切到产物审核：面板内部不再出现原生滚动容器（滚动交给 Monaco 渲染区）
    fireEvent.click(screen.getByTestId("cockpit-artifact-review-tab"));
    const panel = screen.getByTestId("artifact-review-panel");
    expect(panel.querySelectorAll('[class*="overflow-auto"], [class*="overflow-scroll"]')).toHaveLength(0);
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
