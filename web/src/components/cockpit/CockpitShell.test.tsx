import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { readCockpitSettings } from "../../state/cockpit-settings";
import { INFO_TTL_MS } from "../../state/recent-completion";
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
    choice: null,
  };
}
function infoItem(key: string, planId: string): CockpitInboxItem {
  return {
    id: `s1:info:${key}`,
    kind: "info",
    severity: 1,
    title: "Work Item Plan 已确认",
    summary: `plan ${planId} 已确认`,
    triage: false,
    source: "plan_confirmed_info",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: null,
    planInfo: {
      key,
      planId,
      sessionId: "s1",
      occurredAt: "2026-09-27T00:00:00Z",
      title: "Work Item Plan 已确认",
    },
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

function recentCompletionItem(key: string, title: string): CockpitInboxItem {
  return {
    ...infoItem(key, "plan_1"),
    id: `issue_1:info:${key}`,
    title,
    summary: `等待人工最终确认 · ${key}`,
    createdAt: new Date(Date.now() - 60_000).toISOString(),
    completionIdentity: JSON.stringify([
      "project_1",
      "issue_1",
      "coding_final_confirm",
      key,
    ]),
  };
}

function ShellWithInbox({
  inbox,
  countedInbox = inbox.filter((item) => item.kind !== "info"),
  notificationCandidates = [],
  onGoToInbox,
  onOpenAttentionTarget,
  onOpenInboxFocus,
}: {
  inbox: readonly CockpitInboxItem[];
  countedInbox?: readonly CockpitInboxItem[];
  notificationCandidates?: readonly CockpitInboxItem[];
  onGoToInbox?: (sessionId: string) => void;
  onOpenAttentionTarget?: (item: CodingAttentionLocationItem) => boolean;
  onOpenInboxFocus?: (target: WorkbenchInboxFocusTarget) => void;
}) {
  mockedUseWorkspaceSessionObservers.mockReturnValue({
    records: [],
    inbox,
    countedInbox,
    watchedSessionIds: ["s1", "s2", "s3", "s4"],
    watchSession: vi.fn(),
    codingAttemptForSession: () => null,
    displayItems: inbox,
    actionableCount: countedInbox.length,
    notificationCandidates,
  });

  return (
    <CockpitShell
      onGoToInbox={onGoToInbox}
      onOpenAttentionTarget={onOpenAttentionTarget}
      onOpenInboxFocus={onOpenInboxFocus}
    >
      <CockpitInbox items={inbox} />
    </CockpitShell>
  );
}

function renderShell({
  inbox,
  countedInbox,
  notificationCandidates,
  onGoToInbox,
  onOpenAttentionTarget,
  onOpenInboxFocus,
}: {
  inbox: readonly CockpitInboxItem[];
  countedInbox?: readonly CockpitInboxItem[];
  notificationCandidates?: readonly CockpitInboxItem[];
  onGoToInbox?: (sessionId: string) => void;
  onOpenAttentionTarget?: (item: CodingAttentionLocationItem) => boolean;
  onOpenInboxFocus?: (target: WorkbenchInboxFocusTarget) => void;
}) {
  return render(
    <ShellWithInbox
      inbox={inbox}
      countedInbox={countedInbox}
      notificationCandidates={notificationCandidates}
      onGoToInbox={onGoToInbox}
      onOpenAttentionTarget={onOpenAttentionTarget}
      onOpenInboxFocus={onOpenInboxFocus}
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
    codingAttemptForSession: () => null,
    displayItems: [],
    actionableCount: 0,
    notificationCandidates: [],
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

  // P1 WIGA Task 9（REQ-WIGA-07）：info 只读分区可读，顶部告警条与批量面
  // 只数 gate（info 不进待处理计数）。
  it("shows plan confirmation without increasing actionable count", () => {
    const info = infoItem("plan_confirmed:plan_1:compile_1", "plan_1");
    renderShell({ inbox: [info], onGoToInbox: () => {} });

    expect(screen.getByText("Work Item Plan 已确认")).toBeInTheDocument();
    expect(screen.getByText("进度信息")).toBeInTheDocument();
    expect(screen.queryByText(/待处理 1 项/)).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /批量确认 1 项/ }),
    ).not.toBeInTheDocument();
  });

  it("keeps a gate readable next to plan confirmation info", () => {
    renderShell({ inbox: [gateItem("s1"), infoItem("plan_confirmed:plan_1:compile_1", "plan_1")] });

    expect(screen.getByTestId("cockpit-inbox-item-gate")).toBeVisible();
    expect(screen.getByText("Work Item Plan 已确认")).toBeVisible();
    expect(screen.getByRole("alert")).toHaveTextContent("待处理 1 项");
  });

  it("hydrates existing info keys silently and toasts each new key once", async () => {
    const existing = infoItem("plan_confirmed:plan_1:compile_1", "plan_1");
    const view = renderShell({ inbox: [existing] });

    // 首次 hydration：只显示，不弹提示。
    expect(screen.getByText("Work Item Plan 已确认")).toBeInTheDocument();
    expect(screen.queryByRole("status")).toBeNull();

    // 同 key 重复刷新：仍不提示。
    view.rerender(
      <ShellWithInbox inbox={[infoItem("plan_confirmed:plan_1:compile_1", "plan_1")]} />,
    );
    expect(screen.queryByRole("status")).toBeNull();

    // 新 compile key：提示一次。
    view.rerender(
      <ShellWithInbox
        inbox={[
          infoItem("plan_confirmed:plan_1:compile_1", "plan_1"),
          infoItem("plan_confirmed:plan_1:compile_2", "plan_1"),
        ]}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent("进度信息：Work Item Plan 已确认");

    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    expect(screen.queryByRole("status")).toBeNull();

    // 同 key 再刷新不重复提示。
    view.rerender(
      <ShellWithInbox
        inbox={[
          infoItem("plan_confirmed:plan_1:compile_1", "plan_1"),
          infoItem("plan_confirmed:plan_1:compile_2", "plan_1"),
        ]}
      />,
    );
    expect(screen.queryByRole("status")).toBeNull();
  });

  // P3 WIGA Task 3（tasks.md §4.1 / REQ-WIGA-07）：候选驱动提示队列——
  // 每身份（completionIdentity）至多一次、按到达顺序逐条 5s；info 不进
  // 待处理计数/告警条/标题。
  it("prompts recent completion candidates one by one for five seconds each and never repeats an identity", async () => {
    const first = recentCompletionItem("k1", "完成事实一");
    const second = recentCompletionItem("k2", "完成事实二");
    const view = renderShell({
      inbox: [first, second],
      notificationCandidates: [first, second],
    });

    expect(screen.getByRole("status")).toHaveTextContent("进度信息：完成事实一");
    expect(screen.queryByRole("alert")).toBeNull();
    expect(document.title).toBe("Aria Web");

    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    expect(screen.getByRole("status")).toHaveTextContent("进度信息：完成事实二");

    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    expect(screen.queryByRole("status")).toBeNull();

    // 同身份候选再次到达：不重复提示。
    view.rerender(
      <ShellWithInbox
        inbox={[first, second]}
        notificationCandidates={[first, second]}
      />,
    );
    expect(screen.queryByRole("status")).toBeNull();
  });

  it("keeps recent completion candidates queued behind an active gate toast", async () => {
    const gate = gateItem("s1");
    const completion = recentCompletionItem("k1", "完成事实一");
    renderShell({
      inbox: [gate, completion],
      notificationCandidates: [completion],
    });

    // gate toast 先显示，info 留队待显示。
    expect(screen.getByRole("status")).toHaveTextContent("需要处理");

    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    expect(screen.getByRole("status")).toHaveTextContent("进度信息：完成事实一");
  });

  it("drops candidates whose TTL expired before display instead of prompting", async () => {
    const expired = {
      ...recentCompletionItem("k-old", "过期事实"),
      createdAt: new Date(Date.now() - INFO_TTL_MS - 1_000).toISOString(),
    };
    const fresh = recentCompletionItem("k-fresh", "新鲜事实");
    renderShell({
      inbox: [expired, fresh],
      notificationCandidates: [expired, fresh],
    });

    await act(async () => {
      await vi.advanceTimersByTimeAsync(0);
    });
    expect(screen.getByRole("status")).toHaveTextContent("进度信息：新鲜事实");

    await act(async () => {
      vi.advanceTimersByTime(5_000);
    });
    expect(screen.queryByRole("status")).toBeNull();
  });
});

// ── P4-e（REQ-UI37-19）：coding_action 新 key 系统提醒 ──
// 导航经注入的 onOpenAttentionTarget（生产由 router 挂载点提供统一 opener）；
// 测试以 mock 断言统一导航入参（契约 A1 target/focus），地址映射归
// router.test.tsx 的 codingAttentionLocation 用例。
import { codingAttentionItem } from "../../state/workspace-cockpit-projection";
import type { CodingAttentionItemDto } from "../../api/types/lifecycle";
import type { CodingAttentionLocationItem } from "../../router";
import type { WorkbenchInboxFocusTarget } from "../../state/workbench-inbox-focus";

function planACodingAttentionItem(): CockpitInboxItem {
  const dto: CodingAttentionItemDto = {
    key: "gate:attempt_a:gate_7",
    kind: "stalled",
    project_id: "project_0001",
    issue_id: "issue_0001",
    plan_id: null,
    attempt_id: "coding_attempt_a",
    logical_work_item_id: null,
    repository_name: "repo-a",
    reason_code: "coder_provider_stalled",
    title: "AI 挂起了",
    summary: "Coder 长时间没有输出",
    occurred_at: "2026-10-16T00:00:00Z",
    target: { type: "coding_attempt", attempt_id: "coding_attempt_a" },
    focus: { gate_id: "gate_7" },
  };
  return codingAttentionItem(dto);
}

const attentionNotifications: {
  title: string;
  options?: NotificationOptions;
  onclick: ((event: Event) => void) | null;
}[] = [];

class AttentionNotificationMock {
  static requestPermission = vi.fn();
  static permission: NotificationPermission = "granted";
  onclick: ((event: Event) => void) | null = null;
  constructor(
    public title: string,
    public options?: NotificationOptions,
  ) {
    attentionNotifications.push(this);
  }
}

describe("CockpitShell coding_action 新 key 系统提醒（P4-e/REQ-UI37-19）", () => {
  beforeEach(() => {
    // 上一 describe 的 vi.useFakeTimers() 会残留，恢复真实计时器供
    // findBy/waitFor 轮询。
    vi.useRealTimers();
    attentionNotifications.length = 0;
    AttentionNotificationMock.permission = "granted";
    vi.stubGlobal("Notification", AttentionNotificationMock);
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

  afterEach(() => {
    vi.unstubAllGlobals();
  });
  it("plan B 收到 plan A 新 key：系统通知恰一次＋平台内消息，onclick 只导航到 attempt+focus", async () => {
    const item = planACodingAttentionItem();
    const onOpenAttentionTarget = vi.fn(() => true);
    renderShell({
      inbox: [item],
      countedInbox: [item],
      notificationCandidates: [item, item],
      onOpenAttentionTarget,
    });

    // 平台内消息：toast＋收件箱行（权限无关的退路）。
    expect(await screen.findByRole("status")).toHaveTextContent("需要处理：AI 挂起了");
    expect(await screen.findByTestId("cockpit-inbox-item-coding_action")).toBeInTheDocument();

    // 系统通知：同 key 候选重复到达也只发一次。
    await waitFor(() => expect(attentionNotifications).toHaveLength(1));
    expect(attentionNotifications[0].title).toContain("AI 挂起了");

    attentionNotifications[0].onclick?.(new Event("click"));
    expect(onOpenAttentionTarget).toHaveBeenCalledWith(
      expect.objectContaining({
        kind: "stalled",
        project_id: "project_0001",
        issue_id: "issue_0001",
        attempt_id: "coding_attempt_a",
        target: { type: "coding_attempt", attempt_id: "coding_attempt_a" },
        focus: { gate_id: "gate_7" },
      }),
    );
  });

  it("通知权限拒绝：不发系统通知，平台消息（toast＋收件箱行）仍在", async () => {
    AttentionNotificationMock.permission = "denied";
    const item = planACodingAttentionItem();
    renderShell({
      inbox: [item],
      countedInbox: [item],
      notificationCandidates: [item],
      onOpenAttentionTarget: vi.fn(() => true),
    });

    expect(await screen.findByRole("status")).toHaveTextContent("需要处理：AI 挂起了");
    expect(await screen.findByTestId("cockpit-inbox-item-coding_action")).toBeInTheDocument();
    expect(attentionNotifications).toHaveLength(0);
  });

  it("驾驶舱设置关闭系统通知时不发通知，平台消息仍在", async () => {
    window.localStorage.setItem(
      "aria.cockpit.settings.v1",
      JSON.stringify({ systemNotificationsEnabled: false }),
    );
    const item = planACodingAttentionItem();
    renderShell({
      inbox: [item],
      countedInbox: [item],
      notificationCandidates: [item],
      onOpenAttentionTarget: vi.fn(() => true),
    });

    expect(await screen.findByTestId("cockpit-inbox-item-coding_action")).toBeInTheDocument();
    expect(attentionNotifications).toHaveLength(0);
  });

  it("告警条「去处理」对 coding 首条走 attention 导航，不再把 coding_attention 前缀当 sessionId", async () => {
    const onGoToInbox = vi.fn();
    const onOpenAttentionTarget = vi.fn(() => true);
    const item = planACodingAttentionItem();
    renderShell({
      inbox: [item],
      countedInbox: [item],
      notificationCandidates: [item],
      onGoToInbox,
      onOpenAttentionTarget,
    });

    expect(await screen.findByRole("alert")).toHaveTextContent("待处理 1 项");
    // 同屏还有收件箱行内「去处理」——告警条按钮以 testid 定位。
    await userEvent.click(await screen.findByTestId("cockpit-attention-go-to"));

    expect(onOpenAttentionTarget).toHaveBeenCalledWith(
      expect.objectContaining({
        target: { type: "coding_attempt", attempt_id: "coding_attempt_a" },
        focus: { gate_id: "gate_7" },
      }),
    );
    expect(onGoToInbox).not.toHaveBeenCalledWith("coding_attention");
  });
});

// ── UI-3（待处理统一）：通知/告警/toast 点击只开共享待处理抽屉并聚焦 ──
// onOpenInboxFocus 注入替代内置路由实现；attempt 目标走抽屉聚焦，
// coding_plan/workspace_session 目标回落 onOpenAttentionTarget 统一导航。
describe("CockpitShell UI-3 待处理抽屉聚焦（通知同源）", () => {
  beforeEach(() => {
    vi.useRealTimers();
    attentionNotifications.length = 0;
    AttentionNotificationMock.permission = "granted";
    vi.stubGlobal("Notification", AttentionNotificationMock);
    mockedUseWorkspaceSessionObservers.mockReset();
    useWorkspaceStore.getState().reset();
    window.localStorage.clear();
  });

  afterEach(() => {
    vi.unstubAllGlobals();
  });

  it("系统通知 onclick 只开待处理抽屉并聚焦（attempt+gate 定位），不走统一导航", async () => {
    const item = planACodingAttentionItem();
    const onOpenInboxFocus = vi.fn();
    const onOpenAttentionTarget = vi.fn(() => true);
    renderShell({
      inbox: [item],
      countedInbox: [item],
      notificationCandidates: [item],
      onOpenInboxFocus,
      onOpenAttentionTarget,
    });

    await waitFor(() => expect(attentionNotifications).toHaveLength(1));
    attentionNotifications[0].onclick?.(new Event("click"));
    expect(onOpenInboxFocus).toHaveBeenCalledWith({
      planId: null,
      attemptId: "coding_attempt_a",
      workItemId: null,
      gateId: "gate_7",
      choiceId: null,
    });
    expect(onOpenAttentionTarget).not.toHaveBeenCalled();
  });

  it("告警条「去处理」开待处理抽屉聚焦；toast「去处理」同源", async () => {
    const item = planACodingAttentionItem();
    const onOpenInboxFocus = vi.fn();
    renderShell({
      inbox: [item],
      countedInbox: [item],
      notificationCandidates: [item],
      onOpenInboxFocus,
      onOpenAttentionTarget: vi.fn(() => true),
    });

    await screen.findByRole("alert");
    await userEvent.click(await screen.findByTestId("cockpit-attention-go-to"));
    expect(onOpenInboxFocus).toHaveBeenCalledWith(
      expect.objectContaining({ attemptId: "coding_attempt_a", gateId: "gate_7" }),
    );

    // toast（新增待处理的首条平台消息）同源聚焦。
    const toastOpen = await screen.findByTestId("cockpit-toast-open-inbox");
    await userEvent.click(toastOpen);
    expect(onOpenInboxFocus).toHaveBeenCalledTimes(2);
  });

  it("workspace_session 目标回落统一导航（不开抽屉）", async () => {
    const dto: CodingAttentionItemDto = {
      key: "recovery:ws_7",
      kind: "manual_recovery",
      project_id: "project_0001",
      issue_id: "issue_0001",
      plan_id: null,
      attempt_id: null,
      logical_work_item_id: null,
      repository_name: null,
      reason_code: null,
      title: "等待人工恢复",
      summary: "恢复候选待处理",
      occurred_at: "2026-10-16T00:00:00Z",
      target: { type: "workspace_session", session_id: "ws_7" },
      focus: null,
    };
    const item = codingAttentionItem(dto);
    const onOpenInboxFocus = vi.fn();
    const onOpenAttentionTarget = vi.fn(() => true);
    renderShell({
      inbox: [item],
      countedInbox: [item],
      notificationCandidates: [item],
      onOpenInboxFocus,
      onOpenAttentionTarget,
    });

    await screen.findByTestId("cockpit-attention-go-to");
    await userEvent.click(screen.getByTestId("cockpit-attention-go-to"));
    expect(onOpenInboxFocus).not.toHaveBeenCalled();
    expect(onOpenAttentionTarget).toHaveBeenCalledWith(
      expect.objectContaining({ target: { type: "workspace_session", session_id: "ws_7" } }),
    );
  });
});
