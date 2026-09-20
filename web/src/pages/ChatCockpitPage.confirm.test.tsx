import { useProviderAvailabilityStore } from "../state/provider-availability-store";
import type * as WorkspaceWsModule from "../hooks/useWorkspaceWs";
import type * as ApiClient from "../api/client";
import type { TakeoverResponse } from "../api/types";
import { ApiRequestError, takeoverWorkspaceSession } from "../api/client";
import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
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

  // F-18/F-20（wave2-f18-report §5 遗留观察）：story/design AuthorConfirm 门 UI 决策面
  // ——确认接 HTTP confirm 端点（WS confirm 帧在该阶段被矩阵拒收），终止接 WS
  // abandon_human_gate（59d59760 矩阵放行）。
  describe("F-18/F-20 story/design author_confirm decision surface", () => {
    afterEach(() => {
      vi.unstubAllGlobals();
    });

    function authorConfirmSession(workspaceType: "story" | "design") {
      useWorkspaceStore.setState({
        sessionId: "session_001",
        stage: "author_confirm",
        workspaceType,
        flowKind: "legacy",
        sessionStatus: "waiting_for_human",
        singleCandidatePhase: null,
        humanGateTurn: null,
        humanGateSnapshot: null,
        humanGateClosure: null,
        providers: { author: "pi", reviewer: null },
        artifact: "# Story Spec",
        chatEntries: [],
        timelineNodes: [],
      });
      useWorkspaceStore.getState().rebuildChatEntries();
    }

    it.each(["story", "design"] as const)(
      "renders the author gate with clickable confirm and terminate buttons (%s)",
      async (workspaceType) => {
        const user = userEvent.setup();
        authorConfirmSession(workspaceType);
        renderCockpitWith(mockWorkspaceWs());

        // 收件箱门条（默认产物视图下仍可见）带 确认+终止 两按钮。
        const inbox = screen.getByTestId("cockpit-inbox");
        expect(within(inbox).getByText("门禁等待")).toBeVisible();
        expect(within(inbox).getByRole("button", { name: "确认" })).toBeEnabled();
        expect(within(inbox).getByRole("button", { name: "终止" })).toBeEnabled();

        // 产物审核页签（author_confirm 默认视图）动作位同样露出两按钮。
        expect(screen.getByTestId("cockpit-artifact-review-tab")).toHaveAttribute(
          "aria-selected",
          "true",
        );
        expect(screen.getByRole("button", { name: "确认产物" })).toBeEnabled();
        // 收件箱与产物面板各一枚终止（二次确认惯例），均可点。
        const terminateButtons = screen.getAllByRole("button", { name: "终止" });
        expect(terminateButtons.length).toBeGreaterThanOrEqual(2);
        for (const button of terminateButtons) {
          expect(button).toBeEnabled();
        }

        // 对话流门卡。
        await user.click(screen.getByTestId("cockpit-conversation-tab"));
        const gateCard = screen.getByTestId("gate-prompt-entry");
        expect(gateCard).toBeVisible();
        expect(within(gateCard).getByRole("button", { name: "确认产物" })).toBeEnabled();
        expect(within(gateCard).getByRole("button", { name: "终止" })).toBeEnabled();
      },
    );

    it("wires confirm to the HTTP confirm endpoint and never the WS confirm frame", async () => {
      const user = userEvent.setup();
      const fetchMock = vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        text: async () => JSON.stringify({ workspace_session_id: "session_001", status: "confirmed" }),
      } as unknown as Response);
      vi.stubGlobal("fetch", fetchMock);
      authorConfirmSession("story");
      const workspaceWs = mockWorkspaceWs();
      renderCockpitWith(workspaceWs);

      await user.click(
        within(screen.getByTestId("cockpit-inbox")).getByRole("button", { name: "确认" }),
      );

      await waitFor(() => {
        const confirmCalls = fetchMock.mock.calls.filter(
          (call) => call[0] === "/api/workspace-sessions/session_001/confirm",
        );
        expect(confirmCalls).toHaveLength(1);
        expect(confirmCalls[0]?.[1]).toMatchObject({
          method: "POST",
          body: JSON.stringify({ confirmed_by: "user" }),
        });
      });
      expect(workspaceWs.sendConfirmGate).not.toHaveBeenCalled();

      // 乐观收敛：sessionStatus confirmed → 决策面关闭。
      await waitFor(() => {
        expect(useWorkspaceStore.getState().sessionStatus).toBe("confirmed");
      });
      expect(
        screen.queryByRole("button", { name: "确认产物" }),
      ).toBeNull();
      expect(
        within(screen.getByTestId("cockpit-inbox")).queryByText("门禁等待"),
      ).toBeNull();
    });

    it("keeps the decision surface open when the HTTP confirm call fails", async () => {
      const user = userEvent.setup();
      const fetchMock = vi.fn().mockResolvedValue({
        ok: false,
        status: 422,
        statusText: "Unprocessable Entity",
        json: async () => ({ code: "confirm_gate_failed", message: "gate rejected", details: {} }),
      } as unknown as Response);
      vi.stubGlobal("fetch", fetchMock);
      authorConfirmSession("design");
      renderCockpitWith(mockWorkspaceWs());

      await user.click(
        within(screen.getByTestId("cockpit-inbox")).getByRole("button", { name: "确认" }),
      );

      await waitFor(() => {
        expect(useWorkspaceStore.getState().sessionStatus).toBe("waiting_for_human");
      });
      // 失败不收敛：门条仍在、按钮仍可点（用户可重试或终止）。
      expect(
        within(screen.getByTestId("cockpit-inbox")).getByRole("button", { name: "确认" }),
      ).toBeEnabled();
    });

    it("wires terminate through ConfirmTwice to the WS abandon_human_gate sender", async () => {
      const user = userEvent.setup();
      authorConfirmSession("story");
      const workspaceWs = mockWorkspaceWs();
      renderCockpitWith(workspaceWs);
      const inbox = screen.getByTestId("cockpit-inbox");

      await user.click(within(inbox).getByRole("button", { name: "终止" }));
      expect(workspaceWs.sendAbandonGate).not.toHaveBeenCalled();

      await user.click(within(inbox).getByRole("button", { name: "确认终止" }));

      expect(workspaceWs.sendAbandonGate).toHaveBeenCalledTimes(1);
      expect(workspaceWs.sendAbandonGate).toHaveBeenCalledWith(expect.any(String));
    });

    it("keeps bulk confirm away from author gates (the runner only serves WS confirm gates)", () => {
      authorConfirmSession("story");
      renderCockpit();

      expect(screen.queryByRole("checkbox")).toBeNull();
      expect(screen.queryByRole("button", { name: /批量确认/ })).toBeNull();
    });

    // k3 P2-1：门卡不随 stage_change 重建——修订反馈起 Author run（stage 离开
    // author_confirm 且无投影）后，旧门卡必须落「已离开人工确认门」且按钮不再发送。
    it("shows the left-stage fallback on the stale gate card and sends nothing", async () => {
      const user = userEvent.setup();
      const fetchMock = vi.fn();
      vi.stubGlobal("fetch", fetchMock);
      authorConfirmSession("story");
      const workspaceWs = mockWorkspaceWs();
      renderCockpitWith(workspaceWs);
      await user.click(screen.getByTestId("cockpit-conversation-tab"));
      const gateCard = screen.getByTestId("gate-prompt-entry");
      expect(
        within(gateCard).getByRole("button", { name: "确认产物" }),
      ).toBeEnabled();
      act(() => {
        // stage_change 现场只 setStage 不重建 chatEntries——旧门卡留在对话流
        // （修订反馈起 Author run=running 阶段）。
        useWorkspaceStore.getState().setStage("running");
      });

      expect(within(gateCard).getByText("已离开人工确认门")).toBeVisible();
      expect(
        within(gateCard).queryByRole("button", { name: "确认产物" }),
      ).toBeNull();
      // 即便遗留按钮被点到（防御），确认/终止零发送。
      expect(workspaceWs.sendConfirmGate).not.toHaveBeenCalled();
      expect(workspaceWs.sendAbandonGate).not.toHaveBeenCalled();
      expect(
        fetchMock.mock.calls.filter(([url]) =>
          typeof url === "string" && url.endsWith("/confirm"),
        ),
      ).toHaveLength(0);
    });

    // k3 P2-2：author_confirm 矩阵不放行 advance——HTTP confirm 乐观 confirmed 后
    // 也不露「手动推进」（门面 advance 同款早退，routing 级单测覆盖发送面）。
    it("hides manual advance at author_confirm even after an optimistic confirm", () => {
      authorConfirmSession("story");
      useWorkspaceStore.setState({ sessionStatus: "confirmed" });
      renderCockpitWith(mockWorkspaceWs());

      expect(screen.queryByRole("button", { name: "手动推进" })).toBeNull();
    });
  });
});
