import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { readCockpitSettings } from "../../state/cockpit-settings";
import type { CockpitInboxItem } from "../../state/workspace-cockpit-projection";
import { CockpitInbox } from "../chat-workspace/cockpit/CockpitInbox";
import { CockpitShell, useCockpitSettingsSlotRef } from "./CockpitShell";
import { useWorkspaceSessionObservers } from "../../hooks/useWorkspaceSessionObservers";
import { useWorkspaceStore } from "../../state/workspace-ws-store";

vi.mock("../../hooks/useWorkspaceSessionObservers", () => ({
  useWorkspaceSessionObservers: vi.fn(),
}));

const mockedUseWorkspaceSessionObservers = vi.mocked(useWorkspaceSessionObservers);
const notificationTitles: string[] = [];

class NotificationMock {
  static requestPermission = vi.fn();

  constructor(title: string, _options?: NotificationOptions) {
    notificationTitles.push(title);
  }
}

function gateItem(sessionId: string): CockpitInboxItem {
  return {
    id: `${sessionId}:gate`,
    kind: "gate",
    severity: 3,
    title: "需要人工确认",
    summary: "等待处理",
    triage: false,
    source: "gate",
    createdAt: null,
    gate: null,
    inlineError: null,
  };
}
function stoppedItem(sessionId: string): CockpitInboxItem {
  return {
    ...gateItem(sessionId),
    id: `${sessionId}:stopped:${sessionId}`,
    kind: "stopped",
    title: "会话停在停点",
  };
}

function ShellWithInbox({
  inbox,
  countedInbox = inbox,
  onGoToInbox,
}: {
  inbox: readonly CockpitInboxItem[];
  countedInbox?: readonly CockpitInboxItem[];
  onGoToInbox?: (sessionId: string) => void;
}) {
  mockedUseWorkspaceSessionObservers.mockReturnValue({
    records: [],
    inbox,
    countedInbox,
    watchedSessionIds: ["s1", "s2", "s3", "s4"],
    watchSession: vi.fn(),
  });

  return (
    <CockpitShell onGoToInbox={onGoToInbox}>
      <CockpitInbox items={inbox} />
    </CockpitShell>
  );
}

function renderShell({
  inbox,
  countedInbox,
  onGoToInbox,
}: {
  inbox: readonly CockpitInboxItem[];
  countedInbox?: readonly CockpitInboxItem[];
  onGoToInbox?: (sessionId: string) => void;
}) {
  return render(
    <ShellWithInbox
      inbox={inbox}
      countedInbox={countedInbox}
      onGoToInbox={onGoToInbox}
    />,
  );
}

function stubEmptyObservers() {
  mockedUseWorkspaceSessionObservers.mockReturnValue({
    records: [],
    inbox: [],
    countedInbox: [],
    watchedSessionIds: [],
    watchSession: vi.fn(),
  });
}

function HeaderSlotProbe() {
  const slotRef = useCockpitSettingsSlotRef();
  return <div data-testid="page-header-slot" ref={slotRef} />;
}

describe("CockpitShell", () => {
  beforeEach(() => {
    vi.useFakeTimers();
    vi.stubGlobal("Notification", NotificationMock);
    Object.defineProperty(NotificationMock, "permission", {
      configurable: true,
      get: () => "granted",
    });
    NotificationMock.requestPermission.mockClear();
    notificationTitles.length = 0;
    document.head.querySelector('link[rel~="icon"]')?.remove();
    const icon = document.createElement("link");
    icon.rel = "icon";
    icon.href = "data:image/svg+xml,original";
    document.head.append(icon);
    mockedUseWorkspaceSessionObservers.mockReset();
    useWorkspaceStore.getState().reset();
    document.title = "Aria Web";
    window.localStorage.clear();
  });

  it("persists changed K immediately without changing the URL", async () => {
    window.history.replaceState({}, "", "/chat/session-a");
    renderShell({ inbox: [] });
    fireEvent.click(screen.getByRole("button", { name: "驾驶舱设置" }));
    fireEvent.change(screen.getByLabelText("聚合窗口 K"), {
      target: { value: "4" },
    });

    expect(readCockpitSettings()).toMatchObject({ watchLimit: 4 });
    expect(mockedUseWorkspaceSessionObservers).toHaveBeenLastCalledWith(
      expect.objectContaining({ watchLimit: 4 }),
    );
    expect(window.location.pathname).toBe("/chat/session-a");
  });

  it("keeps an unclosable sticky banner until observed count reaches zero", () => {
    const view = renderShell({ inbox: [gateItem("s1")] });

    expect(screen.getByRole("alert")).toHaveTextContent("待处理 1 项");
    expect(screen.queryByRole("button", { name: "关闭" })).toBeNull();

    view.rerender(<ShellWithInbox inbox={[]} />);

    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("adds a toast only for a newly opened item and clears its pulse after visit", () => {
    const onGoToInbox = vi.fn();
    const view = renderShell({ inbox: [], onGoToInbox });

    view.rerender(<ShellWithInbox inbox={[gateItem("s1")]} onGoToInbox={onGoToInbox} />);

    expect(screen.getByRole("status")).toHaveTextContent("需要处理");
    fireEvent.click(screen.getByRole("button", { name: "去处理" }));

    expect(onGoToInbox).toHaveBeenCalledWith("s1");
    expect(screen.getByTestId("cockpit-inbox-item-gate")).toHaveAttribute(
      "data-pulse",
      "false",
    );
  });

  it("clears an item pulse when its session becomes current", () => {
    renderShell({ inbox: [gateItem("s1")] });

    expect(screen.getByTestId("cockpit-inbox-item-gate")).toHaveAttribute(
      "data-pulse",
      "true",
    );
    act(() => useWorkspaceStore.setState({ sessionId: "s1" }));

    expect(screen.getByTestId("cockpit-inbox-item-gate")).toHaveAttribute(
      "data-pulse",
      "false",
    );
  });

  it("keeps the current-session inbox visible without increasing global alert surfaces", () => {
    renderShell({ inbox: [gateItem("s1")], countedInbox: [] });

    expect(screen.getByTestId("cockpit-inbox-item-gate")).toBeVisible();
    expect(screen.queryByRole("alert")).toBeNull();
    expect(document.title).toBe("Aria Web");
  });

  it("alerts again when the same stopped item reopens after closing", () => {
    const view = renderShell({ inbox: [stoppedItem("s1")] });

    expect(screen.getByTestId("cockpit-inbox-item-stopped")).toHaveAttribute(
      "data-pulse",
      "true",
    );
    view.rerender(<ShellWithInbox inbox={[]} />);
    view.rerender(<ShellWithInbox inbox={[stoppedItem("s1")]} />);

    expect(screen.getByRole("status")).toHaveTextContent("会话停在停点");
    expect(screen.getByTestId("cockpit-inbox-item-stopped")).toHaveAttribute(
      "data-pulse",
      "true",
    );
  });

  it("adds a reduced-motion-aware pulse class to newly opened items", () => {
    renderShell({ inbox: [gateItem("s1")] });

    expect(screen.getByTestId("cockpit-inbox-item-gate").className).toContain(
      "motion-safe:animate-pulse",
    );
  });

  it("delays notification 30 seconds and keeps L1/L2 when permission is denied", async () => {
    vi.spyOn(Notification, "permission", "get").mockReturnValue("denied");
    renderShell({ inbox: [gateItem("s1")] });

    await vi.advanceTimersByTimeAsync(30_000);

    expect(Notification.requestPermission).not.toHaveBeenCalled();
    expect(screen.getByRole("alert")).toBeVisible();
  });

  it("updates the title and favicon badge independently", () => {
    renderShell({ inbox: [gateItem("s1")] });

    expect(document.title).toBe("🔴待处理×1 · aria");
    expect(
      document.querySelector<HTMLLinkElement>('link[rel~="icon"]')?.getAttribute("href") ?? "",
    ).toContain("%3E1%3C");
  });

  it("does not postpone notification when more items arrive before 30 seconds", async () => {
    const view = renderShell({ inbox: [gateItem("s1")] });

    await vi.advanceTimersByTimeAsync(10_000);
    view.rerender(<ShellWithInbox inbox={[gateItem("s1"), gateItem("s2")]} />);
    await vi.advanceTimersByTimeAsync(20_000);

    expect(notificationTitles).toEqual(["aria：需要处理"]);
  });

  it("does not notify twice when observer refreshes an unchanged inbox", async () => {
    const view = renderShell({ inbox: [gateItem("s1")] });

    await vi.advanceTimersByTimeAsync(30_000);
    view.rerender(<ShellWithInbox inbox={[gateItem("s1")]} />);
    await vi.advanceTimersByTimeAsync(0);

    expect(notificationTitles).toEqual(["aria：需要处理"]);
  });

  it("notifies once after an item remains unhandled for 30 seconds", async () => {
    renderShell({ inbox: [gateItem("s1")] });

    await vi.advanceTimersByTimeAsync(30_000);

    expect(notificationTitles).toEqual(["aria：需要处理"]);
  });

  it("aggregates one opened item across L1 toast, L2 sticky, L3 badge and L4 notification", async () => {
    renderShell({ inbox: [gateItem("s1")] });

    expect(screen.getByRole("status")).toHaveTextContent("需要处理");
    expect(screen.getByRole("alert")).toHaveTextContent("待处理 1 项");
    expect(document.title).toBe("🔴待处理×1 · aria");
    expect(
      document.querySelector<HTMLLinkElement>('link[rel~="icon"]')?.getAttribute("href") ?? "",
    ).toContain("%3E1%3C");

    await vi.advanceTimersByTimeAsync(30_000);

    expect(notificationTitles).toEqual(["aria：需要处理"]);
  });

  it("renders the settings trigger into the page-provided header slot instead of a fixed overlay", () => {
    stubEmptyObservers();

    render(
      <CockpitShell>
        <HeaderSlotProbe />
      </CockpitShell>,
    );

    const trigger = screen.getByTestId("cockpit-settings-trigger");
    expect(screen.getByTestId("page-header-slot")).toContainElement(trigger);
    expect(trigger.className).not.toContain("fixed");
    expect(screen.queryByTestId("cockpit-settings-fallback")).toBeNull();

    fireEvent.click(trigger);
    expect(screen.getByRole("dialog", { name: "驾驶舱设置" })).toBeInTheDocument();
  });

  // v40 复验 #2：review 反馈按钮在屏幕下方 + 主屏滚动条。实测根因是
  // 「待处理 N 项」告警条（52px，流内）叠在 h-screen 页面之上——文档
  // scrollHeight=100vh+52px，页面底部（输入条/发送反馈）被挤出视口。
  // 契约：外壳独占视口高度（唯一 h-screen），告警条与内容在同一列内
  // 分配高度；可增长页面在内滚容器里滚，文档级不再滚动。
  it("owns the viewport height when the pending alert bar is present (no document-level overflow)", () => {
    renderShell({ inbox: [gateItem("s1")] });

    const shell = screen.getByTestId("cockpit-shell");
    expect(shell.className).toContain("h-screen");
    expect(shell.className).toContain("flex-col");
    expect(shell.className).toContain("overflow-hidden");

    const alertBar = screen.getByRole("alert");
    expect(alertBar.className).toContain("shrink-0");
    expect(alertBar.parentElement).toBe(shell);

    const scroller = screen.getByTestId("cockpit-shell-scroll");
    expect(scroller.className).toContain("min-h-0");
    expect(scroller.className).toContain("flex-1");
    expect(scroller.className).toContain("overflow-y-auto");
    expect(scroller.parentElement).toBe(shell);
    expect(alertBar.compareDocumentPosition(scroller) & Node.DOCUMENT_POSITION_FOLLOWING).not.toBe(0);
  });

  it("keeps the settings trigger reachable through the fallback overlay without a page slot", () => {
    stubEmptyObservers();

    render(
      <CockpitShell>
        <span>页面内容</span>
      </CockpitShell>,
    );

    const fallback = screen.getByTestId("cockpit-settings-fallback");
    expect(fallback).toContainElement(screen.getByTestId("cockpit-settings-trigger"));
    // 兜底入口必须让位：落在页面顶栏之下，且 z 序低于抽屉/浮层（spec 抽屉 z-50）。
    expect(fallback.className).toContain("top-16");
    expect(fallback.className).toContain("z-40");
    expect(fallback.className).not.toContain("z-[100]");
  });
});
