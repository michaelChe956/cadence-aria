import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  notifyLifecycleInvalidated,
  subscribeToLifecycleInvalidation,
} from "../../state/lifecycle-workbench-store";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  lifecycleFetch,
  type LifecycleFetchMock,
} from "./IssueLifecycleWorkbench.test-utils";

vi.mock("../shared/MonacoViewer", () => ({
  MonacoViewer: ({ value, height }: { value: string; height?: number }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

// F-29 红/绿测试：确认成功后 lifecycle 卡片应实时刷新（同页 invalidation +
// 跨 tab BroadcastChannel），不经手动「刷新」按钮。
//
// BroadcastChannel 语义桩：同 channel 名的其它实例收到消息，发送方自身不回环
// （与标准一致，用于模拟“另一个 tab”）。
class FakeBroadcastChannel {
  readonly name: string;
  onmessage: ((event: MessageEvent) => void) | null = null;
  private static readonly byName = new Map<string, Set<FakeBroadcastChannel>>();

  constructor(name: string) {
    this.name = name;
    const peers = FakeBroadcastChannel.byName.get(name) ?? new Set();
    peers.add(this);
    FakeBroadcastChannel.byName.set(name, peers);
  }

  postMessage(message: unknown): void {
    queueMicrotask(() => {
      for (const peer of FakeBroadcastChannel.byName.get(this.name) ?? []) {
        if (peer === this || !peer.onmessage) {
          continue;
        }
        peer.onmessage(new MessageEvent("message", { data: message }));
      }
    });
  }

  close(): void {
    FakeBroadcastChannel.byName.get(this.name)?.delete(this);
  }
}

function countLifecycleFetches(fetchMock: LifecycleFetchMock) {
  return fetchMock.mock.calls.filter(([url]) =>
    String(url).includes("/lifecycle?project_id="),
  ).length;
}

async function confirmStoryOnServer(fetchMock: LifecycleFetchMock) {
  // 模拟 HTTP confirm 已在服务端落定 durable 状态（story draft→confirmed）。
  await fetchMock("/api/workspace-sessions/workspace_session_story_0001/confirm", {
    method: "POST",
  });
}

// 默认阶段是 work_item（fixture 同时有 story+design）——切到 Story 阶段查看
// story 卡（Task 6 单阶段面板）。
async function switchToStoryStage() {
  const user = userEvent.setup();
  await user.click(await screen.findByTestId("stage-tab-story"));
}

afterEach(() => {
  vi.unstubAllGlobals();
});

describe("IssueLifecycleWorkbench lifecycle invalidation (F-29)", () => {
  it("refreshes the story card in place after a same-page confirm invalidation, without manual refresh", async () => {
    vi.stubGlobal("BroadcastChannel", FakeBroadcastChannel);
    const fetchMock = lifecycleFetch({ storyDraftInitially: true });
    vi.stubGlobal("fetch", fetchMock);
    render(<IssueLifecycleWorkbench />);

    await switchToStoryStage();
    const storyCard = screen.getByTestId("lifecycle-card-story_spec");
    expect(within(storyCard).getByText("draft")).toBeInTheDocument();

    await confirmStoryOnServer(fetchMock);

    // confirm 成功的调用点发出 invalidation（同页 notify 总线）。
    act(() => {
      notifyLifecycleInvalidated("issue_0001");
    });

    await waitFor(() => {
      expect(
        within(screen.getByTestId("lifecycle-card-story_spec")).getByText(
          "confirmed",
        ),
      ).toBeInTheDocument();
    });
  });

  it("refreshes the story card when another tab broadcasts lifecycle-invalidated", async () => {
    vi.stubGlobal("BroadcastChannel", FakeBroadcastChannel);
    const fetchMock = lifecycleFetch({ storyDraftInitially: true });
    vi.stubGlobal("fetch", fetchMock);
    render(<IssueLifecycleWorkbench />);

    await switchToStoryStage();
    screen.getByTestId("lifecycle-card-story_spec");

    await confirmStoryOnServer(fetchMock);

    // 另一个 tab：同 channel 名的独立实例 postMessage（发送方≠接收方）。
    const otherTab = new FakeBroadcastChannel("lifecycle-invalidated");
    act(() => {
      otherTab.postMessage({
        type: "lifecycle-invalidated",
        issueId: "issue_0001",
      });
    });

    await waitFor(() => {
      expect(
        within(screen.getByTestId("lifecycle-card-story_spec")).getByText(
          "confirmed",
        ),
      ).toBeInTheDocument();
    });
  });

  it("ignores invalidations for issues the workbench is not showing", async () => {
    vi.stubGlobal("BroadcastChannel", FakeBroadcastChannel);
    const fetchMock = lifecycleFetch({ storyDraftInitially: true });
    vi.stubGlobal("fetch", fetchMock);
    render(<IssueLifecycleWorkbench />);

    await switchToStoryStage();
    screen.getByTestId("lifecycle-card-story_spec");
    const lifecycleFetchCount = countLifecycleFetches(fetchMock);

    act(() => {
      notifyLifecycleInvalidated("issue_not_loaded");
    });
    await act(async () => {
      await Promise.resolve();
    });

    expect(countLifecycleFetches(fetchMock)).toBe(lifecycleFetchCount);
    expect(
      within(screen.getByTestId("lifecycle-card-story_spec")).getByText("draft"),
    ).toBeInTheDocument();
  });

  it("delivers bus events to direct subscribers for wiring at confirm call sites", () => {
    vi.stubGlobal("BroadcastChannel", FakeBroadcastChannel);
    const events: string[] = [];
    const unsubscribe = subscribeToLifecycleInvalidation((event) => {
      events.push(event.issueId);
    });

    notifyLifecycleInvalidated("issue_0001");
    expect(events).toEqual(["issue_0001"]);

    unsubscribe();
    notifyLifecycleInvalidated("issue_0002");
    expect(events).toEqual(["issue_0001"]);
  });
});
