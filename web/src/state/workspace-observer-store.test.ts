import { describe, expect, it } from "vitest";
import type { WorkspaceSessionSummary } from "../api/types";
import type { WorkspaceWsState } from "./workspace-ws-store";
import {
  createObserverController,
  selectObservedInbox,
  selectWatchedSessionIds,
  watchWindowCopy,
  type WorkspaceObserverSocketFactory,
} from "./workspace-observer-store";

function summary(
  workspace_session_id: string,
  status: WorkspaceSessionSummary["status"] = "running",
): WorkspaceSessionSummary {
  return {
    workspace_session_id,
    issue_id: "issue_1",
    entity_id: `entity_${workspace_session_id}`,
    workspace_type: "work_item",
    status,
    author_provider: "claude_code",
    reviewer_provider: "codex",
    review_rounds: 1,
    superpowers_enabled: false,
    openspec_enabled: false,
  };
}

function observedState(
  sessionId: string,
  overrides: Partial<WorkspaceWsState> = {},
): WorkspaceWsState {
  return {
    sessionId,
    stage: "running",
    sessionStatus: "running",
    humanGateSnapshot: null,
    humanGateTurn: null,
    humanGateClosure: null,
    flowKind: null,
    pendingReviewerSummary: null,
    chatEntries: [],
    protocolError: null,
    error: null,
    advanceCommands: {},
    ...overrides,
  } as WorkspaceWsState;
}

describe("workspace observer store", () => {
  it("counts only watched records and tells the user K-external events are not real-time", () => {
    const ids = selectWatchedSessionIds(
      [summary("s1"), summary("s2"), summary("s3")],
      2,
    );
    const records = [
      { sessionId: "s1", state: observedState("s1", { sessionStatus: "stopped_needs_human" }) },
      { sessionId: "s3", state: observedState("s3", { error: "K 外事件" }) },
    ];

    expect(ids).toEqual(["s1", "s2"]);
    expect(
      selectObservedInbox(records.filter((record) => ids.includes(record.sessionId))),
    ).toHaveLength(1);
    expect(watchWindowCopy(2)).toBe("仅监视最近 2 个候选；集合外实时卡壳不计入计数");
  });

  it("keeps API order for active candidates and excludes terminal statuses", () => {
    expect(
      selectWatchedSessionIds(
        [
          summary("first", "open"),
          summary("terminal", "terminated"),
          summary("second", "waiting_for_human"),
          summary("blocked", "blocked_provider_unavailable"),
          summary("third", "failed"),
        ],
        3,
      ),
    ).toEqual(["first", "second", "third"]);
  });

  it("opens only entries newly admitted to K and closes entries leaving K", async () => {
    const opened: string[] = [];
    const closed: string[] = [];
    const fakeSocketFactory: WorkspaceObserverSocketFactory = (sessionId) => {
      opened.push(sessionId);
      return { close: () => closed.push(sessionId) };
    };
    const controller = createObserverController(fakeSocketFactory);

    await controller.replaceWatchedSessionIds(["a", "b"]);
    await controller.replaceWatchedSessionIds(["b", "c"]);

    expect(opened).toEqual(["a", "b", "c"]);
    expect(closed).toEqual(["a"]);
    expect(controller.records()).toEqual([]);
  });

  it("keys merged inbox entries by session and sorts severity before session id", () => {
    expect(
      selectObservedInbox([
        { sessionId: "z", state: observedState("z", { sessionStatus: "stopped_needs_human" }) },
        { sessionId: "a", state: observedState("a", { error: "连接失败" }) },
      ]).map((item) => item.id),
    ).toEqual(["a:hard_error:error", "z:stopped:z"]);
  });
});
