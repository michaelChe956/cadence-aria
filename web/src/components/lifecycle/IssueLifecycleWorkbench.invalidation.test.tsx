import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, describe, expect, it, vi } from "vitest";
import {
  notifyLifecycleInvalidated,
  subscribeToLifecycleInvalidation,
} from "../../state/lifecycle-workbench-store";
import { IssueLifecycleWorkbench } from "./IssueLifecycleWorkbench";
import {
  deferred,
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
    expect(within(storyCard).getByText("草稿")).toBeInTheDocument();

    await confirmStoryOnServer(fetchMock);

    // confirm 成功的调用点发出 invalidation（同页 notify 总线）。
    act(() => {
      notifyLifecycleInvalidated("issue_0001");
    });

    await waitFor(() => {
      expect(
        within(screen.getByTestId("lifecycle-card-story_spec")).getByText(
          "已确认",
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
          "已确认",
        ),
      ).toBeInTheDocument();
    });
  });

  it("applies overlapping updates to two issues without re-reading the first issue", async () => {
    vi.stubGlobal("BroadcastChannel", FakeBroadcastChannel);
    const baseFetch = lifecycleFetch({
      storyDraftInitially: true,
      sharedLifecycleIdsAcrossIssues: true,
    });
    // Hold the first issue's update while allowing the other issue to refresh.
    let deferIssueOneLifecycle = false;
    const pendingIssueOneFetches: Array<{
      url: string;
      resolve: (response: Response) => void;
    }> = [];
    const fetchMock: LifecycleFetchMock = vi.fn(async (input, init) => {
      const url = String(input);
      if (
        deferIssueOneLifecycle &&
        url.includes("/api/issues/issue_0001/lifecycle")
      ) {
        const deferredResponse = deferred<Response>();
        pendingIssueOneFetches.push({
          url,
          resolve: deferredResponse.resolve,
        });
        return deferredResponse.promise;
      }
      return baseFetch(input, init);
    });
    vi.stubGlobal("fetch", fetchMock);
    render(<IssueLifecycleWorkbench />);

    await switchToStoryStage();
    expect(
      within(screen.getByTestId("lifecycle-card-story_spec")).getByText("草稿"),
    ).toBeInTheDocument();

    // 服务端已确认 issue_0001 的 story（durable 投影 draft→confirmed）。
    await baseFetch("/api/workspace-sessions/workspace_session_issue_0001_story/confirm", {
      method: "POST",
    });

    deferIssueOneLifecycle = true;
    act(() => {
      notifyLifecycleInvalidated("issue_0001");
    });
    // A concurrent update to another issue must not invalidate this request.
    act(() => {
      notifyLifecycleInvalidated("issue_0002");
    });
    expect(pendingIssueOneFetches.length).toBe(1);

    // The first response already contains the durable change; no retry is needed.
    const issueOneUrl = pendingIssueOneFetches[0]!.url;
    await act(async () => {
      pendingIssueOneFetches[0]!.resolve(await baseFetch(issueOneUrl));
    });

    await waitFor(() => {
      expect(
        within(screen.getByTestId("lifecycle-card-story_spec")).getByText(
          "已确认",
        ),
      ).toBeInTheDocument();
    });
    expect(pendingIssueOneFetches).toHaveLength(1);
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
      within(screen.getByTestId("lifecycle-card-story_spec")).getByText("草稿"),
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
