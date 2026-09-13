import { act, fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { CockpitInboxItem } from "../../state/workspace-cockpit-projection";
import { CockpitInbox } from "../chat-workspace/cockpit/CockpitInbox";
import { CockpitShell } from "./CockpitShell";
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

function ShellWithInbox({ inbox }: { inbox: readonly CockpitInboxItem[] }) {
  mockedUseWorkspaceSessionObservers.mockReturnValue({
    records: [],
    inbox,
    watchedSessionIds: [],
  });

  return (
    <CockpitShell>
      <CockpitInbox items={inbox} />
    </CockpitShell>
  );
}

function renderShell({ inbox }: { inbox: readonly CockpitInboxItem[] }) {
  return render(<ShellWithInbox inbox={inbox} />);
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
    window.addEventListener("aria:cockpit:go-to-inbox", onGoToInbox);
    const view = renderShell({ inbox: [] });

    view.rerender(<ShellWithInbox inbox={[gateItem("s1")]} />);

    expect(screen.getByRole("status")).toHaveTextContent("需要处理");
    fireEvent.click(screen.getByRole("button", { name: "去处理" }));

    expect(onGoToInbox).toHaveBeenCalledWith(
      expect.objectContaining({ detail: { sessionId: "s1" } }),
    );
    expect(screen.getByTestId("cockpit-inbox-item-gate")).toHaveAttribute(
      "data-pulse",
      "false",
    );
    window.removeEventListener("aria:cockpit:go-to-inbox", onGoToInbox);
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
});
