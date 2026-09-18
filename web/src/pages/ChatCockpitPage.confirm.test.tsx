import { useProviderAvailabilityStore } from "../state/provider-availability-store";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import type * as ApiClient from "../api/client";
import type { TakeoverResponse } from "../api/types";
import { ApiRequestError, takeoverWorkspaceSession } from "../api/client";
import { act, fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  selectCockpitInbox,
  type CockpitInboxItem,
} from "../state/workspace-cockpit-projection";
import { useWorkspaceWs, type WorkspaceWsApi } from "../hooks/useWorkspaceWs";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import {
  useWorkspaceStore,
  type TimelineNode,
  type WorkspaceWsState,
} from "../state/workspace-ws-store";
import type { PlanProjectionBundle } from "../api/types";
import { fetchWorkspaceArtifactVersion } from "../api/workspace-content";
import { planRepairSnapshotFixture } from "../state/workspace-plan-repair-test-fixtures";
import { observerStateFromSessionState } from "../state/workspace-observer-store";
import { readCockpitSettings } from "../state/cockpit-settings";
import { ChatCockpitPage } from "./ChatCockpitPage";
import { COCKPIT_HOTKEYS } from "../state/cockpit-operation-semantics";
import { useOperationAuditStore } from "../state/operation-audit-store";
import {
  currentMockWorkspaceWs,
  mockWorkspaceWs,
} from "./ChatWorkspacePage.test-utils";
import {
  bulkConfirmStart,
  cockpitInbox,
  cockpitObservedRecords,
  gateItem,
  hardErrorItem,
  installCockpitPageTestHooks,
  renderCockpit,
  renderCockpitWith,
  stoppedItem,
  timelineNode,
  watchSession,
} from "./ChatCockpitPage.test-utils";

vi.mock("../state/bulk-confirm-store", () => ({
  useBulkConfirmStore: Object.assign(
    (selector: (state: { runs: readonly unknown[] }) => unknown) => selector({ runs: [] }),
    { getState: () => ({ start: bulkConfirmStart }) },
  ),
}));

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
  MonacoViewer: ({ value, height }: { value: string; height?: string }) => (
    <div data-testid="monaco-viewer" data-height={height}>
      {value}
    </div>
  ),
}));

vi.mock("../components/cockpit/CockpitShell", () => ({
  useCockpitShellInbox: () =>
    cockpitInbox.length > 0
      ? cockpitInbox
      : selectCockpitInbox(useWorkspaceStore.getState()).map((item) => ({
          ...item,
          id: `${useWorkspaceStore.getState().sessionId}:${item.id}`,
        })),
  useCockpitInboxPulse: () => false,
  useCockpitSessionWatch: () => watchSession,
  useCockpitSettings: () => readCockpitSettings(),
  useCockpitObservedRecords: () => cockpitObservedRecords,
}));

describe("ChatCockpitPage", () => {
  installCockpitPageTestHooks();

  describe("cockpit author confirm and review decision", () => {
    function artifactVersionSummaryFixture() {
      return {
        version: 1,
        markdown: "# 设计稿",
        generated_by: "claude_code" as const,
        reviewed_by: null,
        review_verdict: null,
        confirmed_by: null,
        is_current: true,
        created_at: "2026-09-17T10:00:00Z",
        source_node_id: "node-artifact",
      };
    }

    it("auto-switches a story session to the artifact tab at author confirm and finalizes", async () => {
      const user = userEvent.setup();
      const sendAuthorDecision = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendAuthorDecision });
      useWorkspaceStore.setState({
        stage: "author_confirm",
        workspaceType: "design",
        providers: { author: "pi", reviewer: "codex" },
        reviewerEnabled: true,
        artifactVersions: [artifactVersionSummaryFixture()],
        artifact: "# 设计稿",
      });

      renderCockpitWith(workspaceWs);

      const tab = screen.getByTestId("cockpit-artifact-review-tab");
      expect(tab).toHaveAttribute("aria-selected", "true");
      expect(screen.getByText("# 设计稿")).toBeVisible();

      await user.click(screen.getByRole("button", { name: "确认定稿" }));

      expect(sendAuthorDecision).toHaveBeenCalledWith("accept_finalize");
    });

    it("sends accept-with-review from the artifact tab", async () => {
      const user = userEvent.setup();
      const sendAuthorDecision = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendAuthorDecision });
      useWorkspaceStore.setState({
        stage: "author_confirm",
        workspaceType: "story",
        providers: { author: "pi", reviewer: "codex" },
        reviewerEnabled: true,
        artifactVersions: [artifactVersionSummaryFixture()],
        artifact: "# 用户故事",
      });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByRole("button", { name: "确认并送审" }));

      expect(sendAuthorDecision).toHaveBeenCalledWith("accept_with_review");
    });

    it("sends revision feedback from the input bar at author confirm", async () => {
      const user = userEvent.setup();
      const sendAuthorDecision = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendAuthorDecision });
      useWorkspaceStore.setState({ stage: "author_confirm", workspaceType: "story" });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByTestId("cockpit-conversation-tab"));
      await user.type(screen.getByTestId("context-note-input"), "第二段改成对话体");
      await user.click(screen.getByRole("button", { name: "发送反馈" }));

      expect(sendAuthorDecision).toHaveBeenCalledWith("revise", "第二段改成对话体");
    });

    it("prefills review findings into the input bar via the shared prefill handle", async () => {
      const user = userEvent.setup();
      const workspaceWs = mockWorkspaceWs();
      useWorkspaceStore.setState({
        stage: "author_confirm",
        workspaceType: "story",
        chatEntries: [
          {
            id: "review-1",
            type: "review_verdict",
            role: "reviewer",
            content: "第二段缺少冲突",
            timestamp: "2026-09-17T10:00:00Z",
          },
        ],
      });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByRole("button", { name: "采纳 Review 意见" }));

      expect(screen.getByTestId("cockpit-conversation-tab")).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(screen.getByTestId("context-note-input")).toHaveValue(
        "按以下 review 意见修订：\n\n第二段缺少冲突",
      );
    });

    it("keeps the conversation tab for non story/design sessions", () => {
      useWorkspaceStore.setState({ stage: "author_confirm", workspaceType: "work_item_plan" });

      renderCockpitWith(mockWorkspaceWs());

      expect(screen.queryByTestId("cockpit-artifact-review-tab")).toBeNull();
      expect(screen.getByTestId("cockpit-conversation-tab")).toBeVisible();
    });

    it("keeps finalize actions hidden while observing another session at author confirm", async () => {
      const user = userEvent.setup();
      vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
        workspace_session_id: "child_story_001",
      } as TakeoverResponse);
      cockpitInbox.push(stoppedItem("session_001"));
      cockpitObservedRecords.push({
        sessionId: "child_story_001",
        state: observerStateFromSessionState({
          type: "session_state",
          session_id: "child_story_001",
          workspace_type: "story",
          stage: "author_confirm",
          superpowers_enabled: false,
          openspec_enabled: false,
          messages: [],
          checkpoints: [],
          artifact: null,
          providers: { author: "claude_code", reviewer: null },
          timeline_nodes: [],
          active_node_id: null,
          artifact_versions: [],
          timeline_node_details: {},
          active_run_id: null,
          human_presentation_revisions: [],
          session_status: "waiting_for_human",
          flow_kind: "legacy",
          run_policy: "interactive",
          run_history: {
            seen_fingerprints: [],
            repairs_used: 0,
            manual_repairs_used: 0,
            transitions_used: 0,
            initial_review_count: 0,
            verification_review_count: 0,
          },
        }),
      });
      useWorkspaceStore.setState({
        stage: "author_confirm",
        workspaceType: "design",
        providers: { author: "pi", reviewer: "codex" },
        reviewerEnabled: true,
      });

      renderCockpit();
      await user.click(screen.getByRole("button", { name: "接管" }));
      await user.click(screen.getByRole("button", { name: "确认接管" }));

      expect(await screen.findByText(/Author：claude_code/)).toBeVisible();
      expect(screen.getByTestId("cockpit-artifact-review-tab")).toHaveAttribute(
        "aria-selected",
        "true",
      );
      expect(screen.queryByRole("button", { name: "确认定稿" })).toBeNull();
      expect(screen.queryByRole("button", { name: "确认并送审" })).toBeNull();
    });

    it("exposes review decision actions at review decision stage", async () => {
      const user = userEvent.setup();
      const sendSelectRevisionPath = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendSelectRevisionPath });
      useWorkspaceStore.setState({ stage: "review_decision", workspaceType: "story" });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByRole("button", { name: "接受修订建议" }));

      expect(sendSelectRevisionPath).toHaveBeenCalledWith("revise", undefined);
    });

    it("offers optional work item plan finding decisions through the shared options helper", async () => {
      const user = userEvent.setup();
      const sendReviewDecision = vi.fn();
      const workspaceWs = mockWorkspaceWs({ sendReviewDecision });
      useWorkspaceStore.setState({
        stage: "review_decision",
        workspaceType: "work_item_plan",
        chatEntries: [
          {
            id: "review-optional-1",
            type: "review_verdict",
            role: "reviewer",
            content: "仅有可选建议",
            timestamp: "2026-09-17T10:00:00Z",
            metadata: {
              verdict: "pass",
              review_gate: "user_confirm_allowed",
              findings: [
                {
                  severity: "suggestion",
                  message: "补充 handoff",
                  evidence: "当前 handoff 说明过短",
                  required_action: "补充上下游交接说明",
                },
              ],
            },
          },
        ],
      });

      renderCockpitWith(workspaceWs);
      await user.click(screen.getByRole("button", { name: "修复这些建议" }));

      expect(sendReviewDecision).toHaveBeenCalledWith("apply_optional_findings");
    });

    it("does not render ChatInputBar during human confirm", () => {
      useWorkspaceStore.setState({ stage: "human_confirm", workspaceType: "story" });

      renderCockpitWith(mockWorkspaceWs());

      expect(screen.queryByTestId("context-note-input")).toBeNull();
    });
  });
});
