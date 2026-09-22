import { beforeEach, vi } from "vitest";
import { render } from "@testing-library/react";
import { takeoverWorkspaceSession } from "../api/client";
import type { WorkspaceWsApi } from "../hooks/useWorkspaceWs";
import { useOperationAuditStore } from "../state/operation-audit-store";
import type { CockpitInboxItem } from "../state/workspace-cockpit-projection";
import {
  useWorkspaceStore,
  type TimelineNode,
  type WorkspaceWsState,
} from "../state/workspace-ws-store";
import { ChatCockpitPage } from "./ChatCockpitPage";
import {
  currentMockWorkspaceWs,
  installChatWorkspacePageTestHooks,
  mockWorkspaceWs,
} from "./ChatWorkspacePage.test-utils";

export const cockpitInbox: CockpitInboxItem[] = [];
export const cockpitObservedRecords: Array<{ sessionId: string; state: WorkspaceWsState }> = [];
export const watchSession = vi.fn();
export const bulkConfirmStart = vi.fn();

export function installCockpitPageTestHooks() {
  installChatWorkspacePageTestHooks();
  beforeEach(() => {
    cockpitInbox.splice(0);
    cockpitObservedRecords.splice(0);
    watchSession.mockReset();
    vi.mocked(takeoverWorkspaceSession).mockReset();
    sessionStorage.clear();
    useOperationAuditStore.getState().reset();
    bulkConfirmStart.mockReset();

    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "approval",
    });
  });
}

export const renderCockpit = (
  sessionId = "session_001",
  mockWs = true,
  onOpenSession = vi.fn(),
) => {
  if (mockWs) {
    mockWorkspaceWs();
  }
  useWorkspaceStore.getState().setSessionIdForTest(sessionId);
  return render(
    <ChatCockpitPage
      sessionId={sessionId}
      onBack={vi.fn()}
      onOpenSession={onOpenSession}
      workspaceWs={currentMockWorkspaceWs()}
    />,
  );
};

export const renderCockpitWith = (
  workspaceWs: WorkspaceWsApi,
  sessionId = "session_001",
) => {
  useWorkspaceStore.getState().setSessionIdForTest(sessionId);
  return render(
    <ChatCockpitPage
      sessionId={sessionId}
      onBack={vi.fn()}
      onOpenSession={vi.fn()}
      workspaceWs={workspaceWs}
    />,
  );
};

export function timelineNode(overrides: Partial<TimelineNode> = {}): TimelineNode {
  return {
    node_id: "node-1",
    node_type: "author_run",
    agent: "claude_code",
    stage: "running",
    round: 1,
    status: "active",
    title: "Author 运行",
    summary: null,
    started_at: "2026-09-13T00:00:00Z",
    completed_at: null,
    duration_ms: null,
    artifact_ref: null,
    provider_config_snapshot: { author: "claude_code", reviewer: null, review_rounds: 1 },
    ...overrides,
  };
}

export function gateItem(sessionId: string, key: string): CockpitInboxItem {
  return {
    id: `${sessionId}:gate:${key}`,
    kind: "gate",
    severity: 1,
    title: "门禁等待",
    summary: "等待人工确认",
    triage: false,
    source: "gate",
    createdAt: null,
    gate: {
      key,
      kind: "human_gate",
      turn_id: key,
      stage: "human_confirm",
      flow_kind: "single_candidate",
      status: "open",
      trigger: null,
      remaining_budget: null,
      findings: [],
      resumable: false,
      triage: false,
      closed: null,
      closure_stage: null,
      opened_at: "2026-09-15T00:00:00.000Z",
      turn: null,
      action_block_reason: null,
      terminate_block_reason: null,
    },
    inlineError: null,
  };
}

export function stoppedItem(sessionId: string): CockpitInboxItem {
  return {
    id: `${sessionId}:stopped:${sessionId}`,
    kind: "stopped",
    severity: 2,
    title: "会话停在停点",
    summary: "等待人工接管后继续",
    triage: false,
    source: "session_status",
    createdAt: null,
    gate: null,
    inlineError: null,
  };
}

export function hardErrorItem(sessionId: string): CockpitInboxItem {
  return {
    id: `${sessionId}:hard_error:error`,
    kind: "hard_error",
    severity: 3,
    title: "引擎错误",
    summary: "错误",
    triage: false,
    source: "engine_error",
    createdAt: null,
    gate: null,
    inlineError: null,
  };
}
