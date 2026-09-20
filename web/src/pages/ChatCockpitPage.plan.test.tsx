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

  describe("plan approval view (Phase 3)", () => {
    function planProjectionBundleFixture(): PlanProjectionBundle {
      return {
        id: "bundle_0001",
        plan_revision_id: "plan_rev_0002",
        dependency_graph_revision_id: "graph_rev_0001",
        work_item_projection_bundle_refs: [],
        human_group_projection: {
          plan_id: "plan_0001",
          goal: "g",
          split_reason: "s",
          work_items: [],
          contract_flow: [
            {
              from: "wi_backend",
              to: "wi_frontend",
              contract_id: "contract_metrics",
              required_capabilities: ["cap_export_csv", "cap_export_json"],
              provided_capabilities: ["cap_export_csv"],
              missing_capabilities: ["cap_export_json"],
            },
          ],
          risks: [],
          source_refs: [],
          normative: false,
          used_by_provider: false,
        },
        coder_group_context: {
          plan_id: "plan_0001",
          ordered_logical_work_item_ids: [],
          dependency_edges: [],
          group_write_scopes: {},
        },
        reviewer_group_matrix: {
          plan_id: "plan_0001",
          work_items: [],
          dependency_edges: [],
          design_traceability_refs: [],
        },
        human_group_projection_hash: "h1",
        coder_group_context_hash: "h2",
        reviewer_group_matrix_hash: "h3",
        compiler_version: "v1",
        created_at: "2026-09-14T08:00:00Z",
      };
    }

    function enterPlanSession() {
      useWorkspaceStore.getState().setSessionState({
        session_id: "session_001",
        workspace_type: "work_item_plan",
        stage: "human_confirm",
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
        messages: [],
        checkpoints: [],
        artifact: null,
        providers: { author: "fake", reviewer: null },
      });
    }

    it("shows the plan approval tab only for work_item_plan sessions", () => {
      renderCockpit();
      expect(screen.queryByTestId("cockpit-plan-approval-tab")).toBeNull();

      // store 更新发生在渲染之后，需要 act 刷新 zustand 驱动的重渲染。
      act(() => {
        enterPlanSession();
      });
      expect(screen.getByTestId("cockpit-plan-approval-tab")).toBeVisible();
    });

    it("swaps the drilldown zone to the plan approval panel and back", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      renderCockpit();

      await user.click(screen.getByTestId("cockpit-plan-approval-tab"));
      expect(screen.getByTestId("cockpit-plan-approval-panel")).toBeVisible();
      expect(screen.queryByTestId("cockpit-conversation-flow-list")).toBeNull();

      await user.click(screen.getByTestId("cockpit-conversation-tab"));
      expect(screen.getByTestId("cockpit-conversation-flow-list")).toBeVisible();
      expect(screen.queryByTestId("cockpit-plan-approval-panel")).toBeNull();
    });

    it("renders the revision diff summary after loading two artifact rounds", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      useWorkspaceStore.setState({
        artifactVersions: [
          {
            version: 1,
            markdown: undefined,
            generated_by: "fake",
            reviewed_by: null,
            review_verdict: null,
            confirmed_by: null,
            is_current: false,
            created_at: "2026-09-14T08:00:00Z",
            source_node_id: "node_1",
          },
          {
            version: 2,
            markdown: undefined,
            generated_by: "fake",
            reviewed_by: null,
            review_verdict: null,
            confirmed_by: null,
            is_current: true,
            created_at: "2026-09-14T09:00:00Z",
            source_node_id: "node_2",
          },
        ],
      });
      // beforeEach 只 clearAllMocks（不清 once 队列），这里显式重置避免吞前一测的残留实现。
      vi.mocked(fetchWorkspaceArtifactVersion).mockReset();
      vi.mocked(fetchWorkspaceArtifactVersion)
        .mockResolvedValueOnce({
          version: 1,
          markdown: "# 计划\n目标 A\n",
        })
        .mockResolvedValueOnce({
          version: 2,
          markdown: "# 计划\n目标 B\n",
        });

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-plan-approval-tab"));
      await user.click(screen.getByTestId("plan-approval-tab-diff"));

      const summary = await screen.findByTestId("revision-diff-summary");
      expect(summary).toHaveTextContent("~1");
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("session_001", 1);
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("session_001", 2);
    });

    it("lists checklist gaps and jumps back to the gate entry finding", async () => {
      const user = userEvent.setup();
      const scrollIntoView = vi.fn();
      Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
        configurable: true,
        value: scrollIntoView,
      });
      enterPlanSession();
      useWorkspaceStore.setState({
        workItemPlanProjectionArtifacts: {
          planProjection: planProjectionBundleFixture(),
          workItemProjections: [],
          history: null,
          validation: null,
          missingWorkItemProjectionRefs: [],
        },
        chatEntries: [
          {
            id: "node_gate:gate-prompt",
            type: "gate_prompt",
            role: "system",
            content: "等待人工确认",
            timestamp: "2026-09-14T08:00:00Z",
            metadata: {
              findings: [
                {
                  class: "repairable",
                  fingerprint: "fp",
                  category: null,
                  severity: "major",
                  message: "契约缺口",
                  evidence: null,
                  required_action: null,
                  contract_field: "contract_metrics",
                },
              ],
            },
          },
        ],
      });

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-plan-approval-tab"));
      await user.click(screen.getByTestId("plan-approval-tab-checklist"));

      expect(screen.getByTestId("contract-checklist-gap-count")).toHaveTextContent(
        "缺口 1 条 / 共 2 条",
      );
      await user.click(screen.getByRole("button", { name: "查看 finding" }));

      expect(screen.getByTestId("cockpit-conversation-flow-list")).toBeVisible();
      expect(screen.queryByTestId("cockpit-plan-approval-panel")).toBeNull();
      // 滚动断言（裁决补充）：跳转回对话流后落点滚动确实发生在 gate 条目上。
      const list = screen.getByTestId("cockpit-conversation-flow-list");
      expect(within(list).getByText("等待人工确认")).toBeVisible();
      expect(scrollIntoView).toHaveBeenCalled();
    });

    it("renders the read-only plan repair panel for the child session", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      useWorkspaceStore.setState({
        planRepair: planRepairSnapshotFixture("session_001"),
      });

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-plan-approval-tab"));
      await user.click(screen.getByTestId("plan-approval-tab-repair"));

      const panel = screen.getByTestId("plan-repair-read-only-panel");
      expect(panel).toHaveTextContent("修订编写中");
      expect(panel).toHaveTextContent("CONTRACT_CAPABILITY_MISSING");
      expect(
        within(panel).queryAllByRole("button"),
      ).toHaveLength(0);
    });

    it("renders the plan approval view for a takeover observed session without a white screen", async () => {
      const user = userEvent.setup();
      vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
        workspace_session_id: "child_plan_001",
      } as TakeoverResponse);
      cockpitInbox.push(stoppedItem("session_001"));
      cockpitObservedRecords.push({
        sessionId: "child_plan_001",
        state: observerStateFromSessionState({
          type: "session_state",
          session_id: "child_plan_001",
          workspace_type: "work_item_plan",
          stage: "human_confirm",
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

      renderCockpit();
      await user.click(screen.getByRole("button", { name: "接管" }));
      await user.click(screen.getByRole("button", { name: "确认接管" }));

      await user.click(await screen.findByTestId("cockpit-plan-approval-tab"));
      expect(screen.getByTestId("cockpit-plan-approval-panel")).toBeVisible();
      expect(screen.getByTestId("revision-diff-view")).toHaveTextContent(
        "当前会话只有一个 artifact 轮次，暂无对比对象。",
      );

      await user.click(screen.getByTestId("plan-approval-tab-checklist"));
      expect(screen.getByTestId("contract-checklist-view")).toHaveTextContent(
        "当前计划没有 capability / 契约条目",
      );
    });

    it("loads the observed child session's own artifact rounds instead of the parent cache", async () => {
      const user = userEvent.setup();
      vi.mocked(takeoverWorkspaceSession).mockResolvedValue({
        workspace_session_id: "child_plan_002",
      } as TakeoverResponse);
      cockpitInbox.push(stoppedItem("session_001"));
      // 操作者先在自己会话看过 v1/v2 轮次：主 store 缓存已持有主会话 markdown。
      const store = useWorkspaceStore.getState();
      store.setArtifactContentCacheEntry(1, "# 主会话\nAAA\n");
      store.setArtifactContentCacheEntry(2, "# 主会话\nBBB\n");
      cockpitObservedRecords.push({
        sessionId: "child_plan_002",
        state: observerStateFromSessionState({
          type: "session_state",
          session_id: "child_plan_002",
          workspace_type: "work_item_plan",
          stage: "human_confirm",
          superpowers_enabled: false,
          openspec_enabled: false,
          messages: [],
          checkpoints: [],
          artifact: null,
          providers: { author: "claude_code", reviewer: null },
          timeline_nodes: [],
          active_node_id: null,
          artifact_versions: [],
          artifact_version_summaries: [
            {
              version: 1,
              generated_by: "claude_code",
              created_at: "2026-09-14T08:00:00Z",
              source_node_id: "node_1",
            },
            {
              version: 2,
              generated_by: "claude_code",
              created_at: "2026-09-14T09:00:00Z",
              source_node_id: "node_2",
              is_current: true,
            },
          ],
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
      vi.mocked(fetchWorkspaceArtifactVersion).mockReset();
      vi.mocked(fetchWorkspaceArtifactVersion)
        .mockResolvedValueOnce({ version: 1, markdown: "# 子会话\nCCC\n" })
        .mockResolvedValueOnce({ version: 2, markdown: "# 子会话\nDDD\n" });

      renderCockpit();
      await user.click(screen.getByRole("button", { name: "接管" }));
      await user.click(screen.getByRole("button", { name: "确认接管" }));
      await user.click(await screen.findByTestId("cockpit-plan-approval-tab"));

      const summary = await screen.findByTestId("revision-diff-summary");
      expect(summary).toHaveTextContent("基准 v1 → 目标 v2");
      // 防串显：渲染内容来自子会话 fetch，主会话缓存不得泄入。
      expect(screen.getByTestId("revision-diff-hunk")).toHaveTextContent("DDD");
      expect(screen.queryAllByText(/主会话/)).toHaveLength(0);
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("child_plan_002", 1);
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("child_plan_002", 2);
    });
  });

  // F-21（v28 监控 0449，三次未修 v26/v27/v28）：plan 会话 human_confirm 门
  // 的终止接线。d2d67b39 只覆盖 story/design AuthorConfirm 面；plan 会话还会以
  // SC 流停在 human_confirm 的 context blocker（prepare 相位）/author 连续
  // validate 失败（generate 相位）/旧会话缺相位（null）形态——此前这些形态被
  // phase_mismatch 一刀切，门卡/收件箱零决策面，终止点击零 WS 出站且无可见反馈。
  // 矩阵（protocol.rs HumanConfirm SC 臂）与引擎 close_human_gate 均只校验
  // stage+flow_kind——终止接 sendAbandonGate（C3-T4 发送器）放行，二次确认惯例
  // 与 story/design 面一致；confirm/反馈维持相位纪律。
  describe("F-21 plan human_confirm gate terminate wiring", () => {
    function planGateSession(singleCandidatePhase: "prepare" | "generate" | null) {
      useWorkspaceStore.setState({
        sessionId: "session_001",
        stage: "human_confirm",
        workspaceType: "work_item_plan",
        flowKind: "single_candidate",
        singleCandidatePhase,
        sessionStatus: "waiting_for_human",
        humanGateTurn: null,
        humanGateSnapshot: null,
        humanGateClosure: null,
        providers: { author: "pi", reviewer: null },
        chatEntries: [],
        timelineNodes: [],
      });
      useWorkspaceStore.getState().rebuildChatEntries();
    }

    it.each([
      ["prepare", "context blocker 门"],
      ["generate", "author validate 失败门"],
      [null, "旧会话缺相位门"],
    ] as const)(
      "%s（%s）：收件箱门条与对话流门卡终止点击恰发一次 abandon_human_gate",
      async (singleCandidatePhase, _label) => {
        const user = userEvent.setup();
        planGateSession(singleCandidatePhase);
        const workspaceWs = mockWorkspaceWs();
        renderCockpitWith(workspaceWs);

        const inbox = screen.getByTestId("cockpit-inbox");
        expect(within(inbox).getByText("门禁等待")).toBeVisible();

        // 收件箱门条：终止二次确认后恰一次发送。
        await user.click(
          within(inbox).getByRole("button", { name: "终止" }),
        );
        expect(workspaceWs.sendAbandonGate).not.toHaveBeenCalled();
        await user.click(within(inbox).getByRole("button", { name: "确认终止" }));
        expect(workspaceWs.sendAbandonGate).toHaveBeenCalledTimes(1);
        expect(workspaceWs.sendAbandonGate).toHaveBeenCalledWith(expect.any(String));
        expect(workspaceWs.sendConfirmGate).not.toHaveBeenCalled();
        vi.mocked(workspaceWs.sendAbandonGate).mockClear();

        // 对话流门卡：同一发送器、同一惯例。
        await user.click(screen.getByTestId("cockpit-conversation-tab"));
        const gateCard = screen.getByTestId("gate-prompt-entry");
        await user.click(within(gateCard).getByRole("button", { name: "终止" }));
        expect(workspaceWs.sendAbandonGate).not.toHaveBeenCalled();
        await user.click(
          within(gateCard).getByRole("button", { name: "确认终止" }),
        );
        expect(workspaceWs.sendAbandonGate).toHaveBeenCalledTimes(1);
        expect(workspaceWs.sendAbandonGate).toHaveBeenCalledWith(expect.any(String));
      },
    );

    it("keeps the plan approval gate (evaluate phase) fully operable — confirm and terminate both wired", async () => {
      const user = userEvent.setup();
      planGateSession(null);
      useWorkspaceStore.setState({ singleCandidatePhase: "evaluate" });
      useWorkspaceStore.getState().rebuildChatEntries();
      const workspaceWs = mockWorkspaceWs();
      renderCockpitWith(workspaceWs);

      const inbox = screen.getByTestId("cockpit-inbox");
      await user.click(within(inbox).getByRole("button", { name: "确认" }));
      expect(workspaceWs.sendConfirmGate).toHaveBeenCalledTimes(1);

      await user.click(within(inbox).getByRole("button", { name: "终止" }));
      await user.click(within(inbox).getByRole("button", { name: "确认终止" }));
      expect(workspaceWs.sendAbandonGate).toHaveBeenCalledTimes(1);
    });
  });
});
