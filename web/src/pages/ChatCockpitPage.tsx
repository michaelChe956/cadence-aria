import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft } from "lucide-react";
import { takeoverWorkspaceSession } from "../api/client";
import {
  ChatEntryList,
  type ChatEntryListHandle,
} from "../components/chat-workspace/ChatEntryList";
import { TimelineNodeList } from "../components/chat-workspace/TimelineNodeList";
import { CockpitInbox } from "../components/chat-workspace/cockpit/CockpitInbox";
import {
  useCockpitObservedRecords,
  useCockpitSessionWatch,
  useCockpitSettings,
  useCockpitShellInbox,
} from "../components/cockpit/CockpitShell";
import { useWorkspaceContentLoaders } from "../hooks/useWorkspaceContentLoaders";
import { useCockpitAutopilot } from "../hooks/useCockpitAutopilot";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { createCockpitActionFacade } from "../state/cockpit-action-routing";
import { workspaceContentCacheValues } from "../state/workspace-content-cache";
import { selectCockpitFlow } from "../state/workspace-cockpit-projection";
import { watchWindowCopy } from "../state/workspace-observer-store";
import { useWorkspaceStore } from "../state/workspace-ws-store";
import { scrollTargetEntryIdForNode } from "./ChatWorkspacePageParts";

function useNowTicker(intervalMs = 1000): number {
  const [now, setNow] = useState(() => Date.now());

  useEffect(() => {
    const timer = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(timer);
  }, [intervalMs]);

  return now;
}

export function ChatCockpitPage({
  sessionId,
  onBack,
}: {
  sessionId: string;
  onBack: () => void;
}) {
  const workspaceWs = useWorkspaceWs(sessionId);
  const state = useWorkspaceStore();
  const now = useNowTicker(1000);
  const cockpitSettings = useCockpitSettings();
  useCockpitAutopilot({
    sessionId,
    state,
    settings: cockpitSettings,
    sendAdvance: workspaceWs.sendAdvance,
  });
  const [takeoverSessionId, setTakeoverSessionId] = useState<string | null>(null);
  const observedInbox = useCockpitShellInbox();
  const observedRecords = useCockpitObservedRecords();
  const watchSession = useCockpitSessionWatch();
  const watchWindow = watchWindowCopy(
    cockpitSettings.watchLimit,
    cockpitSettings.observerRefreshIntervalMs,
  );
  const selectedState =
    takeoverSessionId === null
      ? state
      : observedRecords.find((record) => record.sessionId === takeoverSessionId)?.state ?? null;
  const selectedSessionId = takeoverSessionId ?? sessionId;
  const flowRows = useMemo(
    () => (selectedState ? selectCockpitFlow(selectedState, now) : []),
    [now, selectedState],
  );
  const contentCacheValues = useMemo(
    () => workspaceContentCacheValues(selectedState?.contentCache ?? state.contentCache),
    [selectedState?.contentCache, state.contentCache],
  );
  const { loadContent, cacheContent: cacheCurrentContent } = useWorkspaceContentLoaders(selectedSessionId);
  const cacheContent = takeoverSessionId === null ? cacheCurrentContent : undefined;
  const [drilldownNodeId, setDrilldownNodeId] = useState<string | null>(null);
  const actions = useMemo(
    () =>
      createCockpitActionFacade({
        flowKind: state.flowKind,
        commandId:
          typeof state.humanGateTurn?.command_id === "string"
            ? state.humanGateTurn.command_id
            : null,
        sendHumanConfirm: workspaceWs.sendHumanConfirm,
        sendHumanGateFeedback: workspaceWs.sendHumanGateFeedback,
      }),
    [
      state.flowKind,
      state.humanGateTurn?.command_id,
      workspaceWs.sendHumanConfirm,
      workspaceWs.sendHumanGateFeedback,
    ],
  );
  const handleTakeover = async (parentSessionId: string) => {
    const child = await takeoverWorkspaceSession(parentSessionId);
    watchSession(child.workspace_session_id);
    setTakeoverSessionId(child.workspace_session_id);
  };
  const handleRetry = (item: { id: string; source: string }) => {
    if (item.source !== "advance") {
      return;
    }
    const commandId = item.id.slice(item.id.lastIndexOf(":") + 1);
    workspaceWs.sendAdvance(commandId);
  };
  const chatListRef = useRef<ChatEntryListHandle | null>(null);
  const drilldownEntryId = useMemo(
    () =>
      drilldownNodeId && selectedState
        ? scrollTargetEntryIdForNode(selectedState.chatEntries, drilldownNodeId)
        : null,
    [drilldownNodeId, selectedState],
  );

  useEffect(() => {
    if (drilldownEntryId) {
      chatListRef.current?.scrollToEntry(drilldownEntryId);
    }
  }, [drilldownEntryId]);

  return (
    <div
      data-testid="cockpit-page"
      className="flex h-screen min-w-0 flex-col overflow-hidden bg-[var(--aria-bg)] text-[var(--aria-ink)]"
    >
      <header className="flex min-h-11 items-center gap-2 border-b border-[var(--aria-line)] px-3 py-1">
        <button
          type="button"
          onClick={onBack}
          className="btn-secondary h-11 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          <ArrowLeft className="h-4 w-4" aria-hidden="true" />
          返回
        </button>
        <span className="aria-mono text-xs text-[var(--aria-ink-muted)]">{sessionId}</span>
        <span className="text-xs text-[var(--aria-ink-muted)]">{watchWindow}</span>
      </header>

      <main className="grid min-h-0 flex-1 grid-cols-1 gap-2 p-2 lg:grid-cols-[20rem_minmax(0,1fr)]">
        <CockpitInbox
          items={observedInbox}
          actions={actions}
          onTakeover={handleTakeover}
          onRetry={handleRetry}
          actionableSessionId={sessionId}
        />
        <div className="grid min-h-0 grid-rows-[minmax(0,1.1fr)_minmax(0,1fr)] gap-2">
          <section
            data-testid="cockpit-execution-flow"
            aria-label="自动执行流"
            className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)]"
          >
            <div className="flex items-center justify-between gap-2 px-3 py-2">
              <h2 className="text-sm font-semibold text-[var(--aria-ink)]">自动执行流</h2>
              <button
                type="button"
                data-testid="cockpit-protocol-diagnostic-count"
                onClick={() => setDrilldownNodeId(selectedState?.activeNodeId ?? null)}
                className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[11px] text-[var(--aria-ink-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
              >
                诊断 {selectedState?.protocolDiagnostics.length ?? 0}
              </button>
            </div>
            <TimelineNodeList
              nodes={selectedState?.timelineNodes ?? []}
              // 下钻选中的行即 ② 区的「当前步」（aria-current="step"）；未下钻时退回 store 的进行中节点。
              activeNodeId={drilldownNodeId ?? selectedState?.activeNodeId ?? null}
              selectedNodeId={drilldownNodeId}
              onSelectNode={setDrilldownNodeId}
              variant="flow"
              flowRows={flowRows}
              className="border-0"
            />
          </section>

          <section
            data-testid="cockpit-conversation-flow"
            aria-label="下钻对话流"
            className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)]"
          >
            <h2 className="px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
              对话流
            </h2>
            <ChatEntryList
              ref={chatListRef}
              entries={selectedState?.chatEntries ?? []}
              actions={takeoverSessionId === null ? actions : undefined}
              contentCache={contentCacheValues}
              loadContent={loadContent}
              onCacheContent={cacheContent}
              sessionId={selectedSessionId}
              testId="cockpit-conversation-flow-list"
            />
          </section>
        </div>
      </main>
    </div>
  );
}
