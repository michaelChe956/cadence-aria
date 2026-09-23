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
import {
  fetchWorkspaceArtifactVersion,
  fetchWorkspaceNodeDetail,
} from "../api/workspace-content";
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
  useCockpitSettingsSlotRef: () => () => undefined,
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

    // F-38：plan 会话此前没有「产物审核」页签——Work Item Plan 全文（artifact_versions
    // 单串 markdown）在 cockpit 里不可达；PlanApprovalPanel 的数据源是 plan-repair
    // 链路，普通 SC 会话空转。产物页签扩到 work_item_plan（复用 ArtifactReviewPanel）。
    function planArtifactVersion(versionNo: number, isCurrent = true) {
      return {
        version: versionNo,
        generated_by: "pi" as const,
        reviewed_by: "kimi_code" as const,
        review_verdict: "pass" as const,
        confirmed_by: null,
        is_current: isCurrent,
        created_at: "2026-09-22T16:02:09Z",
        source_node_id: `timeline_node_00${versionNo}`,
      };
    }

    it("opens the artifact review tab for plan sessions", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      useWorkspaceStore.setState({
        artifactVersions: [planArtifactVersion(1)],
        artifact: "# Work Item Plan\n## WI-001 登录\n",
      });
      // 刷新/重建后的 artifact_version_summaries 不带 markdown（store 快照契约）——
      // 全文经版本端点取回，与 story/design 产物面板同一通路。
      vi.mocked(fetchWorkspaceArtifactVersion).mockReset();
      vi.mocked(fetchWorkspaceArtifactVersion).mockResolvedValue({
        version: 1,
        markdown: "# Work Item Plan\n## WI-001 登录\n",
      });

      renderCockpit();

      const tab = screen.getByTestId("cockpit-artifact-review-tab");
      expect(tab).toBeVisible();
      await user.click(tab);

      const panel = screen.getByTestId("artifact-review-panel");
      expect(panel).toBeVisible();
      // 产物全文（plan 是整块 markdown）经 ArtifactPane 渲染。
      expect(await within(panel).findByTestId("monaco-viewer")).toHaveTextContent(
        "Work Item Plan",
      );
      // F-38 fix2（可达性）：单版本会话 Diff 按钮 disabled、RevisionDiffView 不挂载——
      // 评审结论必须常显在产物面板头部（默认面）。
      expect(within(panel).getByTestId("artifact-version-verdict")).toHaveTextContent(
        "审批：通过",
      );
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("session_001", 1);
    });

    it("keeps the plan artifact panel free of story/design finalize actions", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      useWorkspaceStore.setState({
        artifactVersions: [planArtifactVersion(1)],
        artifact: "# Work Item Plan\n",
      });

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-artifact-review-tab"));

      // story/design author 门的定稿动作面不得泄入 plan 门（相位判据不是
      // author_confirm，而是 story/design author 门本身）。
      const panel = screen.getByTestId("artifact-review-panel");
      expect(within(panel).queryByRole("button", { name: "确认定稿" })).toBeNull();
      expect(within(panel).queryByRole("button", { name: "确认并评审" })).toBeNull();
      expect(within(panel).queryByRole("button", { name: "采纳 Review 意见" })).toBeNull();
    });

    it("shows the plan full text and review conclusion for a single artifact round", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      useWorkspaceStore.setState({
        artifactVersions: [planArtifactVersion(1)],
        chatEntries: [
          {
            id: "review:1",
            type: "review_verdict",
            role: "reviewer",
            content: "计划可确认",
            timestamp: "2026-09-22T16:03:00Z",
            node_id: "timeline_node_003",
            metadata: {
              verdict: "pass",
              summary: "计划可确认",
              findings: [
                { severity: "suggestion", message: "建议一" },
                { severity: "suggestion", message: "建议二" },
                { severity: "suggestion", message: "建议三" },
              ],
            },
          },
        ],
      });
      vi.mocked(fetchWorkspaceArtifactVersion).mockReset();
      vi.mocked(fetchWorkspaceArtifactVersion).mockResolvedValue({
        version: 1,
        markdown: "# Work Item Plan\n## WI-001 登录\n",
      });

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-plan-approval-tab"));

      // 单版本（无第二轮）此前只有「暂无对比对象」空话术——现在给全文+评审结论。
      expect(screen.getByTestId("revision-diff-single-version")).toBeVisible();
      expect(screen.getByTestId("revision-diff-verdict")).toHaveTextContent(
        "v1 审批：通过",
      );
      expect(screen.getByTestId("revision-diff-advisory-count")).toHaveTextContent(
        "可选建议 3 条",
      );
      expect(await screen.findByTestId("monaco-viewer")).toHaveTextContent(
        "WI-001 登录",
      );
      expect(vi.mocked(fetchWorkspaceArtifactVersion)).toHaveBeenCalledWith("session_001", 1);
    });

    it("switches to the artifact view from the pending gate card", async () => {
      const user = userEvent.setup();
      enterPlanSession();
      useWorkspaceStore.setState({
        artifactVersions: [planArtifactVersion(1)],
        artifact: "# Work Item Plan\n",
        chatEntries: [
          {
            id: "node_gate:gate-prompt",
            type: "gate_prompt",
            role: "system",
            content: "等待人工确认",
            timestamp: "2026-09-22T16:04:00Z",
            metadata: { action_facade: "legacy" },
          },
        ],
      });

      renderCockpit();

      const card = screen.getByTestId("gate-prompt-entry");
      expect(within(card).getByTestId("gate-artifact-context")).toHaveTextContent(
        "待确认产物：Work Item Plan v1",
      );
      await user.click(within(card).getByRole("button", { name: "查看产物" }));
      expect(screen.getByTestId("artifact-review-panel")).toBeVisible();
      expect(screen.queryByTestId("cockpit-conversation-flow-list")).toBeNull();
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
      // 该观测会话没有 artifact 轮次（artifact_versions: []）——单版本/零版本分支
      // 如实说明，不白屏（F-38 起零轮次与单轮次是两条不同话术）。
      expect(screen.getByTestId("revision-diff-view")).toHaveTextContent(
        "当前会话还没有 artifact 轮次",
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

  // REQ-PCG-01（plan-compile-gate-visibility）：work_item_plan 停在 batch_confirm 时，
  // Cockpit 的确认必须走 HTTP confirm 端点——SC AuthorConfirm 阶段矩阵不放行 WS
  // confirm 帧；同时不得误发任何 WS 三命令或 recovery 帧。
  describe("REQ-PCG-01 plan batch confirm wiring", () => {
    afterEach(() => {
      vi.unstubAllGlobals();
    });

    function batchConfirmSession() {
      useWorkspaceStore.setState({
        sessionId: "session_001",
        stage: "author_confirm",
        workspaceType: "work_item_plan",
        flowKind: "single_candidate",
        sessionStatus: "waiting_for_human",
        singleCandidatePhase: null,
        humanGateTurn: null,
        humanGateSnapshot: null,
        humanGateClosure: null,
        providers: { author: "pi", reviewer: null },
        artifact: null,
        chatEntries: [],
        timelineNodes: [
          timelineNode({
            node_id: "node_batch",
            node_type: "work_item_batch_confirm",
            stage: "author_confirm",
            status: "active",
            title: "Work Item Batch 确认",
          }),
        ],
      });
      useWorkspaceStore.getState().rebuildChatEntries();
    }

    it("confirms the whole batch over the HTTP endpoint and never the WS confirm frame", async () => {
      const user = userEvent.setup();
      const fetchMock = vi.fn().mockResolvedValue({
        ok: true,
        status: 200,
        text: async () =>
          JSON.stringify({
            workspace_session_id: "session_001",
            issue_id: "issue_0001",
            status: "confirmed",
          }),
      } as unknown as Response);
      vi.stubGlobal("fetch", fetchMock);
      batchConfirmSession();
      const workspaceWs = mockWorkspaceWs();
      renderCockpitWith(workspaceWs);

      const inbox = screen.getByTestId("cockpit-inbox");
      expect(within(inbox).getByText("确认整组 Work Item Draft")).toBeVisible();

      await user.click(within(inbox).getByRole("button", { name: "确认整组" }));

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
      expect(workspaceWs.sendAbandonGate).not.toHaveBeenCalled();
      expect(workspaceWs.sendHumanGateFeedback).not.toHaveBeenCalled();
      expect(workspaceWs.sendAdvance).not.toHaveBeenCalled();
      expect(workspaceWs.sendWorkItemPlanCompileRecoveryAction).not.toHaveBeenCalled();
    });
  });

  // REQ-PCG-02：recovery 门没有 confirm 语义——门开着时 confirm 热键/门面若落到
  // WS confirm 帧，SC HumanConfirm 的 Approval 臂会开启第二个 compile
  // （compile.rs enter_policy_valid_work_item_plan_compile）。确认入口必须 fail-closed。
  describe("REQ-PCG-02 recovery gate confirm routing", () => {
    function recoveryGateSession() {
      useWorkspaceStore.setState({
        sessionId: "session_001",
        stage: "human_confirm",
        workspaceType: "work_item_plan",
        flowKind: "single_candidate",
        sessionStatus: "waiting_for_human",
        singleCandidatePhase: null,
        humanGateTurn: null,
        humanGateSnapshot: null,
        humanGateClosure: null,
        providers: { author: "pi", reviewer: null },
        artifact: null,
        chatEntries: [],
        timelineNodes: [
          timelineNode({
            node_id: "node_recovery",
            node_type: "work_item_plan_compile_recovery",
            stage: "human_confirm",
            status: "active",
            title: "WorkItemPlan Compile Recovery",
            summary: "Final Compile 需要恢复：provider timeout",
          }),
        ],
      });
      useWorkspaceStore.getState().rebuildChatEntries();
    }

    it("keeps the confirm hotkey off the WS confirm frame while the recovery gate is open", () => {
      recoveryGateSession();
      const workspaceWs = mockWorkspaceWs();
      renderCockpitWith(workspaceWs);

      const inbox = screen.getByTestId("cockpit-inbox");
      expect(within(inbox).getByText("Final Compile 恢复")).toBeVisible();

      fireEvent.keyDown(document, { code: COCKPIT_HOTKEYS.confirm.code, ctrlKey: true });

      expect(workspaceWs.sendConfirmGate).not.toHaveBeenCalled();
      expect(workspaceWs.sendAbandonGate).not.toHaveBeenCalled();
      expect(workspaceWs.sendHumanGateFeedback).not.toHaveBeenCalled();
      expect(workspaceWs.sendAdvance).not.toHaveBeenCalled();
      expect(workspaceWs.sendWorkItemPlanCompileRecoveryAction).not.toHaveBeenCalled();
    });

    it("still submits the recovery action from the gate row (the confirm block is not a blanket disable)", async () => {
      const user = userEvent.setup();
      recoveryGateSession();
      const workspaceWs = mockWorkspaceWs();
      renderCockpitWith(workspaceWs);

      await user.click(
        within(screen.getByTestId("cockpit-inbox")).getByRole("button", { name: "继续" }),
      );

      expect(workspaceWs.sendWorkItemPlanCompileRecoveryAction).toHaveBeenCalledWith(
        "continue",
        undefined,
      );
      expect(workspaceWs.sendConfirmGate).not.toHaveBeenCalled();
    });
  });

  // F-39（执行流阶段卡定位）：非对话流页签（产物审核/计划审批）下 ChatEntryList 整块
  // 卸载——chatListRef.current === null，点击阶段卡此前只写 drilldownNodeId：视图不切回、
  // 滚动不发生、零反馈（静默 no-op）。滚动 effect 又只依赖 drilldownEntryId，视图切回
  // 之后也不再重滚。选中节点 = 切回对话流并定位到该节点的气泡；没有可定位目标时给出
  // 可见反馈而不是沉默。
  describe("F-39 timeline node drilldown locates the conversation bubble", () => {
    function planSessionWithReviewNode(summary: string | null) {
      useWorkspaceStore.setState({
        sessionId: "session_001",
        stage: "human_confirm",
        workspaceType: "work_item_plan",
        flowKind: "legacy",
        sessionStatus: "waiting_for_human",
        providers: { author: "pi", reviewer: "codex" },
        artifact: "# Work Item Plan\n",
        artifactVersions: [],
        chatEntries: [],
        timelineNodes: [
          timelineNode({
            node_id: "node_review_1",
            node_type: "reviewer_run",
            agent: "codex",
            stage: "cross_review",
            round: 1,
            status: "completed",
            title: "Review Round 1",
            summary,
            started_at: "2026-09-22T16:00:00Z",
            completed_at: "2026-09-22T16:01:00Z",
          }),
        ],
      });
    }

    it("switches back to the conversation view and scrolls to the node's bubble", async () => {
      const user = userEvent.setup();
      const scrollIntoView = vi.fn();
      Object.defineProperty(HTMLElement.prototype, "scrollIntoView", {
        configurable: true,
        value: scrollIntoView,
      });
      planSessionWithReviewNode("需要返修");
      useWorkspaceStore.setState({
        chatEntries: [
          {
            id: "node_review_1:stream",
            type: "provider_stream",
            role: "reviewer",
            content: "reviewing diff",
            timestamp: "2026-09-22T16:00:30Z",
            node_id: "node_review_1",
          },
        ],
      });
      // 水合缺位（detail 端点不可用）：本用例只依赖 store 里的既有条目。
      vi.mocked(fetchWorkspaceNodeDetail).mockRejectedValue(
        new Error("节点详情不可用"),
      );

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-artifact-review-tab"));
      // 前提：产物页签下对话流整块卸载，滚动入口（ref）为空。
      expect(screen.queryByTestId("cockpit-conversation-flow-list")).toBeNull();

      await user.click(screen.getByTestId("timeline-node-reviewer_run"));

      expect(screen.getByTestId("cockpit-conversation-flow-list")).toBeVisible();
      const scrolledEntryIds = scrollIntoView.mock.contexts
        .filter((context): context is HTMLElement => context instanceof HTMLElement)
        .map((context) => context.dataset.entryId);
      expect(scrolledEntryIds).toContain("node_review_1:stream");
    });

    it("surfaces a hint when the selected stage has no locatable bubble", async () => {
      const user = userEvent.setup();
      planSessionWithReviewNode("需要返修");
      vi.mocked(fetchWorkspaceNodeDetail).mockRejectedValue(
        new Error("节点详情不可用"),
      );

      renderCockpit();
      await user.click(screen.getByTestId("cockpit-artifact-review-tab"));
      await user.click(screen.getByTestId("timeline-node-reviewer_run"));

      expect(screen.getByTestId("cockpit-conversation-flow-list")).toBeVisible();
      // 无目标可辨反馈：不得静默。
      expect(screen.getByTestId("drilldown-no-target-hint")).toBeVisible();
    });
  });
});
