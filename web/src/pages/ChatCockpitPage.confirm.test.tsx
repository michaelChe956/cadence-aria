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

    // 退役留档（T5/REQ-RET-02）：`auto-switches a story session to the artifact tab at author confirm and finalizes` 驱动已删除的 legacy 决策发送面，
    // 随消息族退役（wp5-attribution-table.md）；T1 矩阵 legacy 回归留档在案。

    // 退役留档（T5/REQ-RET-02）：`sends accept-with-review from the artifact tab` 驱动已删除的 legacy 决策发送面，
    // 随消息族退役（wp5-attribution-table.md）；T1 矩阵 legacy 回归留档在案。

    // 退役留档（T5/REQ-RET-02）：`sends revision feedback from the input bar at author confirm` 驱动已删除的 legacy 决策发送面，
    // 随消息族退役（wp5-attribution-table.md）；T1 矩阵 legacy 回归留档在案。

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

    // 退役留档（T5/REQ-RET-02）：`exposes review decision actions at review decision stage` 驱动已删除的 legacy 决策发送面，
    // 随消息族退役（wp5-attribution-table.md）；T1 矩阵 legacy 回归留档在案。

    // 退役留档（T5/REQ-RET-02）：`offers optional work item plan finding decisions through the shared options helper` 驱动已删除的 legacy 决策发送面，
    // 随消息族退役（wp5-attribution-table.md）；T1 矩阵 legacy 回归留档在案。

    it("does not render ChatInputBar during human confirm", () => {
      useWorkspaceStore.setState({ stage: "human_confirm", workspaceType: "story" });

      renderCockpitWith(mockWorkspaceWs());

      expect(screen.queryByTestId("context-note-input")).toBeNull();
    });
  });
});
