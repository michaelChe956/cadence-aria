import { beforeEach, vi } from "vitest";
import { render } from "@testing-library/react";
import { takeoverWorkspaceSession } from "../api/client";
import type { WorkspaceWsApi } from "../hooks/useWorkspaceWs";
import { useOperationAuditStore } from "../state/operation-audit-store";
import type {
  CockpitInboxItem,
  LcResumeRecoveryInboxProjection,
} from "../state/workspace-cockpit-projection";
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
  onOpenInfoCoding?: (address: {
    projectId: string;
    issueId: string;
    attemptId: string;
  }) => void,
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
      onOpenInfoCoding={onOpenInfoCoding}
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
    title: "需要人工确认",
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
    choice: null,
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
    choice: null,
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
    choice: null,
  };
}

/** P2 WIGA Task 9：只读 coding FinalConfirm info 条目（kind="info"，下钻 Coding Workspace）。 */
export function codingInfoItem(
  attemptId = "coding_attempt_001",
  finalConfirmed = false,
): CockpitInboxItem {
  return {
    id: `issue_001:info:coding_final_confirm:${attemptId}:coding_node_0004`,
    kind: "info",
    severity: 1,
    title: finalConfirmed ? "已最终确认" : "编码执行完成，待最终确认",
    summary: finalConfirmed ? "人工已完成最终确认" : "等待人工最终确认",
    triage: false,
    source: "coding_final_confirm_info",
    createdAt: null,
    gate: null,
    inlineError: null,
    choice: null,
    codingInfo: {
      projectId: "project_001",
      issueId: "issue_001",
      planId: "work_item_plan_0001",
      attemptId,
      key: `coding_final_confirm:${attemptId}:coding_node_0004`,
      occurredAt: "2026-09-27T03:20:00Z",
      finalConfirmed,
    },
  };
}

/** P1 WIGA Task 9：只读 plan 确认 info 条目（kind="info"，不可选/不可批量）。 */
export function infoItem(
  key: string,
  planId: string,
  sessionId = "session_001",
): CockpitInboxItem {
  return {
    id: `${sessionId}:info:${key}`,
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
      sessionId,
      occurredAt: "2026-09-27T00:00:00Z",
      title: "Work Item Plan 已确认",
    },
  };
}

/** D（Task 7.1 / anchor_t21）：LC 恢复等待卡条目（C6.1 投影形状直投夹具；
 * id 前缀决定 actionable 归属——driver 会话用 store setState 走真实
 * selectCockpitInbox 投影，此夹具供 observed 会话卡直投 cockpitInbox）。 */
export function lcResumeRecoveryItem(
  overrides: Partial<LcResumeRecoveryInboxProjection> & { id?: string } = {},
): CockpitInboxItem {
  return {
    id: overrides.id ?? "session_001:lc_resume_recovery:rec-1",
    kind: "lc_resume_recovery",
    severity: 2,
    title: (overrides.state ?? "waiting") === "waiting"
      ? "旧原生会话已失效"
      : "正在启动新原生会话…",
    summary: `${overrides.reasonCode ?? "resume_fingerprint_mismatch"} · ${overrides.provider ?? "codex"} · ${overrides.runKind ?? "legacy_draft"}`,
    triage: false,
    source: "lc_resume_recovery",
    createdAt: overrides.detectedAt ?? "2026-10-11T00:02:00Z",
    gate: null,
    inlineError: null,
    choice: null,
    recovery: {
      recoveryId: "rec-1",
      state: "waiting",
      reasonCode: "resume_fingerprint_mismatch",
      provider: "codex",
      failedNodeId: "node-1",
      runKind: "legacy_draft",
      detectedAt: "2026-10-11T00:02:00Z",
      updatedAt: "2026-10-11T00:04:00Z",
      allowedActions: [],
      lastOperation: null,
      ...(overrides as Partial<LcResumeRecoveryInboxProjection>),
    },
  };
}
