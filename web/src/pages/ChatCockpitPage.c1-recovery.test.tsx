import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import { act, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { CockpitInboxItem } from "../state/workspace-cockpit-projection";
import { c1WaitingItem } from "../state/workspace-cockpit-projection";
import type { C1WaitingItem } from "../api/types/lifecycle";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { useOperationAuditStore } from "../state/operation-audit-store";
import { subscribeToLifecycleInvalidation } from "../state/lifecycle-workbench-store";
import { ChatCockpitPage } from "./ChatCockpitPage";
import { readCockpitSettings } from "../state/cockpit-settings";
import { currentMockWorkspaceWs, mockWorkspaceWs } from "./ChatWorkspacePage.test-utils";

// Task 9 页面级运行时断言：C1 恢复卡按钮 → ChatCockpitPage sendC1Action 的
// 真实 REST 接线。api/client 不打桩——真实 requestJson/fetch 发送器跑在
// stub 的全局 fetch 上，逐条断言 URL/method/body（不只测 mock 回调次数）。

vi.mock("../state/bulk-confirm-store", () => ({
  useBulkConfirmStore: Object.assign(
    (selector: (state: { runs: readonly unknown[] }) => unknown) => selector({ runs: [] }),
    { getState: () => ({ start: vi.fn() }) },
  ),
}));

vi.mock("../hooks/useWorkspaceWs", async (importOriginal) => ({
  ...(await importOriginal<typeof WorkspaceWsModule>()),
  useWorkspaceWs: vi.fn(),
}));
vi.mock("../hooks/useUnloadGuard", () => ({ useUnloadGuard: vi.fn() }));

vi.mock("../api/workspace-content", () => ({
  fetchWorkspaceArtifactVersion: vi.fn(),
  fetchWorkspaceEventOutput: vi.fn(),
  fetchWorkspaceNodeDetail: vi.fn(),
  fetchWorkspacePrompt: vi.fn(),
}));

vi.mock("../components/shared/MonacoViewer", () => ({
  MonacoViewer: ({ value }: { value: string; height?: number }) => (
    <div data-testid="monaco-viewer">{value}</div>
  ),
}));

const cockpitInbox: CockpitInboxItem[] = [];

vi.mock("../components/cockpit/CockpitShell", () => ({
  useCockpitShellInbox: () => cockpitInbox,
  useCockpitInboxPulse: () => false,
  useCockpitSessionWatch: () => vi.fn(),
  useCockpitSettings: () => readCockpitSettings(),
  useCockpitObservedRecords: () => [],
  useCockpitSettingsSlotRef: () => () => undefined,
  useCockpitCodingAttemptForSession: () => () => null,
}));

const PROJECT_ID = "project_0001";
const ISSUE_ID = "issue_0001";

const BINDING = {
  binding_version: 2,
  enrollment_id: "enrollment_0001",
  plan_id: "plan_0001",
  session_id: "wsp_0001",
  source: { stories: [], designs: [] },
  target: { kind: "single_repository", repository_id: "repo_physical_c1" },
  author_provider: "fake",
  reviewer_provider: "fake",
};

const TARGET = { kind: "single_repository", repository_id: "repo_physical_c1" } as const;

const retryWaiting: C1WaitingItem = {
  id: "c1:advance_retry_failed:advance_0001",
  kind: "advance_retry_failed",
  reason: "advance initialization failed; original record stays failed",
  completed_steps: ["record_persisted", "journal_prepared"],
  target: TARGET,
  plan_id: "plan_0001",
  session_id: "wsp_0001",
  attempt_id: "attempt_0001",
  gate_id: null,
  possible_side_effect: "provider start outcome unknown",
  actions: ["retry_initialization"],
  next_phase: "plan_binding_saved",
};

const takeoverWaiting: C1WaitingItem = {
  id: "c1:lease_takeover:issue_0001:lease_0001",
  kind: "lease_takeover",
  reason: "worktree lease owner reached a terminal state",
  completed_steps: ["lease_classified_dead"],
  target: TARGET,
  plan_id: "plan_0001",
  session_id: "wsp_0001",
  attempt_id: "attempt_0001",
  gate_id: null,
  possible_side_effect: null,
  actions: ["confirm_takeover"],
  next_phase: "lease_taken_over",
};

const candidateWaiting: C1WaitingItem = {
  id: "c1:candidate_recovery:gate_0001",
  kind: "candidate_recovery",
  reason: "candidate snapshot incomplete: source_revision_missing",
  completed_steps: ["candidate_source_persisted"],
  target: TARGET,
  plan_id: "plan_0001",
  session_id: "wsp_0001",
  attempt_id: null,
  gate_id: "gate_0001",
  possible_side_effect: null,
  actions: ["recover_candidate"],
  next_phase: "candidate_recovered",
};

const rebindWaiting: C1WaitingItem = {
  id: "c1:generation_history:1",
  kind: "generation_history",
  reason: "1 previous binding generation stays queryable",
  completed_steps: [],
  target: TARGET,
  plan_id: "plan_0001",
  session_id: "wsp_0001",
  attempt_id: null,
  gate_id: null,
  possible_side_effect: null,
  actions: ["rebind"],
  next_phase: null,
};

const repoInitFailedWaiting: C1WaitingItem = {
  id: "c1:project:project_0001:repository_init:op_init_0001",
  kind: "repository_initialization_failed",
  reason:
    "repository initialization failed at pre_check (provider_unavailable); awaiting gateway recovery",
  completed_steps: ["cadence_skills"],
  target: null,
  plan_id: null,
  session_id: null,
  attempt_id: null,
  gate_id: null,
  possible_side_effect: null,
  actions: ["resume_repository_initialization"],
  next_phase: "repository_registered",
  action_context: [],
  operation_id: "op_init_0001",
  diagnostics: {
    failed_step: "pre_check",
    reason_code: "provider_unavailable",
    provider: "claude_code",
    stderr_summary: "claude code gateway refused connection",
    changed_paths: ["repo-a/.claude/settings.json"],
    retryable: true,
  },
  project_id: PROJECT_ID,
};

type CapturedRequest = { url: string; method: string; body: unknown };
let captured: CapturedRequest[];

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "content-type": "application/json" },
  });
}

function requestBodyCommandId(request: CapturedRequest | undefined): string {
  if (
    request &&
    typeof request.body === "object" &&
    request.body !== null &&
    "command_id" in request.body &&
    typeof request.body.command_id === "string"
  ) {
    return request.body.command_id;
  }
  return "";
}

function installFetchRouter() {
  captured = [];
  const router = async (input: RequestInfo | URL, init?: RequestInit): Promise<Response> => {
    const url = typeof input === "string" ? input : input.toString();
    const method = (init?.method ?? "GET").toUpperCase();
    const rawBody = typeof init?.body === "string" ? init.body : null;
    const request: CapturedRequest = {
      url,
      method,
      body: rawBody === null ? null : JSON.parse(rawBody),
    };
    captured.push(request);
    if (
      method === "GET" &&
      url === `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/automation-enrollment`
    ) {
      return jsonResponse({ enrollment_id: "enrollment_0001", binding_history: { current: BINDING, previous: [] } });
    }
    if (
      method === "POST" &&
      url ===
        `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/work-item-plans/plan_0001/advance/retry-initialization`
    ) {
      return jsonResponse({
        command_id: requestBodyCommandId(request),
        state: "accepted",
        retry: {
          retry_id: "retry_0001",
          command_id: requestBodyCommandId(request),
          advance_id: "advance_0001",
          attempt_id: "attempt_0001",
          state: "accepted",
          checkpoint: "plan_binding_saved",
          created_at: "2026-09-29T00:00:00Z",
          updated_at: "2026-09-29T00:00:00Z",
        },
        outcome: null,
      });
    }
    if (
      method === "POST" &&
      url === `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/automation-enrollment/lease/takeover`
    ) {
      return jsonResponse({
        command_id: requestBodyCommandId(request),
        state: "accepted",
        lease: {
          disposition: "dead_needs_takeover",
          lease_id: "lease_0001",
          last_activity_at: null,
          evidence: ["takeover_confirmed: dead owner lease_0001 cleared under binding v2"],
        },
      });
    }
    if (method === "POST" && url === "/api/workspace-sessions/wsp_0001/human-actions") {
      return jsonResponse({
        command_id: requestBodyCommandId(request),
        state: "accepted",
        gate_id: "gate_0001",
      });
    }
    if (
      method === "POST" &&
      url === `/api/projects/${PROJECT_ID}/repository-initializations/op_init_0001/resume`
    ) {
      // C5 Task 6：resume 返回 RepositoryInitializationOperationSnapshot。
      return jsonResponse(
        {
          operation_id: "op_init_0002",
          status: "created",
          steps: [{ step_id: "cadence_skills", status: "pending" }],
          current_step: null,
          failed_step: null,
          result: null,
          error: null,
          created_at: "2026-09-30T00:00:00Z",
          updated_at: "2026-09-30T00:00:00Z",
          completed_at: null,
        },
        202,
      );
    }
    return jsonResponse({ code: "not_found", message: `no route for ${method} ${url}` }, 404);
  };
  vi.stubGlobal("fetch", vi.fn(router));
}

function c1Requests(): CapturedRequest[] {
  return captured.filter(({ url }) =>
    [
      `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/automation-enrollment`,
      `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/work-item-plans/plan_0001/advance/retry-initialization`,
      `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/automation-enrollment/lease/takeover`,
      "/api/workspace-sessions/wsp_0001/human-actions",
      `/api/projects/${PROJECT_ID}/repository-initializations/op_init_0001/resume`,
    ].includes(url),
  );
}

describe("ChatCockpitPage C1 recovery REST wiring", () => {
  beforeEach(() => {
    mockWorkspaceWs();
    cockpitInbox.splice(0);
    captured = [];
    sessionStorage.clear();
    useOperationAuditStore.getState().reset();
    useWorkspaceStore.setState({
      stage: "human_confirm",
      flowKind: "single_candidate",
      singleCandidatePhase: "approval",
    });
    vi.spyOn(console, "info").mockImplementation(() => undefined);
    vi.spyOn(console, "error").mockImplementation(() => undefined);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    vi.restoreAllMocks();
  });

  it("dispatches retry-initialization with durable binding via the real REST route", async () => {
    installFetchRouter();
    cockpitInbox.push(c1WaitingItem(retryWaiting, PROJECT_ID, ISSUE_ID));
    useWorkspaceStore.getState().setSessionIdForTest("session_001");
    render(
      <ChatCockpitPage
        sessionId="session_001"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
        workspaceWs={currentMockWorkspaceWs()}
      />,
    );

    const card = screen.getByTestId("c1-waiting-advance_retry_failed");
    expect(within(card).getByText(/可能副作用：provider start outcome unknown/)).toBeVisible();
    await userEvent.click(within(card).getByTestId("c1-action-retry_initialization"));

    await waitForCalls(2);
    // 真实 URL/method/body：先补读 durable enrollment，再打 retry 路由。
    expect(c1Requests()).toEqual([
      {
        url: `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/automation-enrollment`,
        method: "GET",
        body: null,
      },
      {
        url: `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/work-item-plans/plan_0001/advance/retry-initialization`,
        method: "POST",
        body: {
          command_id: "cmd-c1-retry-c1:advance_retry_failed:advance_0001",
          expected_binding: BINDING,
          expected_attempt_id: "attempt_0001",
          expected_checkpoint: "plan_binding_saved",
          confirm_unknown_side_effect: false,
        },
      },
    ]);
  });

  it("sends the side-effect confirmation as an independent retry command", async () => {
    installFetchRouter();
    cockpitInbox.push(c1WaitingItem(retryWaiting, PROJECT_ID, ISSUE_ID));
    useWorkspaceStore.getState().setSessionIdForTest("session_001");
    render(
      <ChatCockpitPage
        sessionId="session_001"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
        workspaceWs={currentMockWorkspaceWs()}
      />,
    );

    const card = screen.getByTestId("c1-waiting-advance_retry_failed");
    await userEvent.click(
      within(card).getByTestId("c1-action-retry_initialization_confirm"),
    );

    await waitForCalls(2);
    const retryCall = c1Requests().at(-1);
    expect(retryCall).toEqual({
      url: `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/work-item-plans/plan_0001/advance/retry-initialization`,
      method: "POST",
      body: {
        command_id: "cmd-c1-retry-confirm-c1:advance_retry_failed:advance_0001",
        expected_binding: BINDING,
        expected_attempt_id: "attempt_0001",
        expected_checkpoint: "plan_binding_saved",
        confirm_unknown_side_effect: true,
      },
    });
  });

  it("dispatches lease takeover with the durable lease identity", async () => {
    installFetchRouter();
    cockpitInbox.push(c1WaitingItem(takeoverWaiting, PROJECT_ID, ISSUE_ID));
    useWorkspaceStore.getState().setSessionIdForTest("session_001");
    render(
      <ChatCockpitPage
        sessionId="session_001"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
        workspaceWs={currentMockWorkspaceWs()}
      />,
    );

    const card = screen.getByTestId("c1-waiting-lease_takeover");
    await userEvent.click(within(card).getByTestId("c1-action-confirm_takeover"));

    await waitForCalls(2);
    expect(c1Requests()).toEqual([
      {
        url: `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/automation-enrollment`,
        method: "GET",
        body: null,
      },
      {
        url: `/api/projects/${PROJECT_ID}/issues/${ISSUE_ID}/automation-enrollment/lease/takeover`,
        method: "POST",
        body: {
          command_id: "cmd-c1-takeover-c1:lease_takeover:issue_0001:lease_0001",
          expected_binding: BINDING,
          expected_lease_id: "lease_0001",
          expected_attempt_id: "attempt_0001",
        },
      },
    ]);
  });

  it("dispatches candidate recovery through the human-actions endpoint", async () => {
    installFetchRouter();
    cockpitInbox.push(c1WaitingItem(candidateWaiting, PROJECT_ID, ISSUE_ID));
    useWorkspaceStore.getState().setSessionIdForTest("session_001");
    render(
      <ChatCockpitPage
        sessionId="session_001"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
        workspaceWs={currentMockWorkspaceWs()}
      />,
    );

    const card = screen.getByTestId("c1-waiting-candidate_recovery");
    await userEvent.click(within(card).getByTestId("c1-action-recover_candidate"));

    await waitForCalls(1);
    expect(c1Requests()).toEqual([
      {
        url: "/api/workspace-sessions/wsp_0001/human-actions",
        method: "POST",
        body: {
          type: "candidate_recovery",
          command_id: "cmd-c1-recover-c1:candidate_recovery:gate_0001",
          expected_gate_id: "gate_0001",
          action: "recover",
        },
      },
    ]);
  });

  it("keeps rebind as a navigation hint without any outbound request", async () => {
    installFetchRouter();
    cockpitInbox.push(c1WaitingItem(rebindWaiting, PROJECT_ID, ISSUE_ID));
    useWorkspaceStore.getState().setSessionIdForTest("session_001");
    render(
      <ChatCockpitPage
        sessionId="session_001"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
        workspaceWs={currentMockWorkspaceWs()}
      />,
    );

    const card = screen.getByTestId("c1-waiting-generation_history");
    await userEvent.click(within(card).getByTestId("c1-action-rebind"));

    // 显式换代表单在 issue 生命周期工作台——驾驶舱零出站（fail-closed）。
    expect(c1Requests()).toEqual([]);
    expect(captured).toEqual([]);
  });

  // C5 Task 6/7：project 级 resume——真实 URL/body（cmd-repo-init-resume-
  // {operationId}），成功后按 project invalidation 唤醒观察器；无 issueId
  // 参与（project 等待项不要求 issue 归属）。
  it("dispatches repository initialization resume via the project REST route", async () => {
    const invalidations: string[] = [];
    const unsubscribe = subscribeToLifecycleInvalidation((event) =>
      invalidations.push(event.issueId),
    );
    installFetchRouter();
    cockpitInbox.push(c1WaitingItem(repoInitFailedWaiting, PROJECT_ID));
    useWorkspaceStore.getState().setSessionIdForTest("session_001");
    render(
      <ChatCockpitPage
        sessionId="session_001"
        onBack={vi.fn()}
        onOpenSession={vi.fn()}
        workspaceWs={currentMockWorkspaceWs()}
      />,
    );

    const card = screen.getByTestId("c1-waiting-repository_initialization_failed");
    expect(
      within(card).getByTestId("repo-init-resume-action"),
    ).toHaveTextContent("网关恢复后继续");
    await userEvent.click(
      within(card).getByTestId("repo-init-resume-action"),
    );

    await waitForCalls(1);
    expect(c1Requests()).toEqual([
      {
        url: `/api/projects/${PROJECT_ID}/repository-initializations/op_init_0001/resume`,
        method: "POST",
        body: { command_id: "cmd-repo-init-resume-op_init_0001" },
      },
    ]);
    await waitFor(() =>
      expect(invalidations).toContain(
        `repository_initialization:${PROJECT_ID}`,
      ),
    );
    unsubscribe();
  });
});


async function waitForCalls(count: number) {
  await act(async () => {
    for (let i = 0; i < 50 && c1Requests().length < count; i += 1) {

      // eslint-disable-next-line no-await-in-loop
      await new Promise<void>((resolve) => {
        setTimeout(resolve, 10);
      });
    }
  });
}
