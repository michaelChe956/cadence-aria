import { useEffect, useMemo, useRef, useState } from "react";
import { ArrowLeft } from "lucide-react";
import {
  ChatEntryList,
  type ChatEntryListHandle,
} from "../components/chat-workspace/ChatEntryList";
import { TimelineNodeList } from "../components/chat-workspace/TimelineNodeList";
import { CockpitInbox } from "../components/chat-workspace/cockpit/CockpitInbox";
import { useWorkspaceContentLoaders } from "../hooks/useWorkspaceContentLoaders";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { workspaceContentCacheValues } from "../state/workspace-content-cache";
import {
  selectCockpitFlow,
  selectCockpitInbox,
} from "../state/workspace-cockpit-projection";
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
  useWorkspaceWs(sessionId);
  const state = useWorkspaceStore();
  const now = useNowTicker(1000);
  const inbox = useMemo(() => selectCockpitInbox(state), [state]);
  const flowRows = useMemo(() => selectCockpitFlow(state, now), [state, now]);
  const contentCacheValues = useMemo(
    () => workspaceContentCacheValues(state.contentCache),
    [state.contentCache],
  );
  const { loadContent, cacheContent } = useWorkspaceContentLoaders(sessionId);
  const [drilldownNodeId, setDrilldownNodeId] = useState<string | null>(null);
  const chatListRef = useRef<ChatEntryListHandle | null>(null);
  const drilldownEntryId = useMemo(
    () =>
      drilldownNodeId
        ? scrollTargetEntryIdForNode(state.chatEntries, drilldownNodeId)
        : null,
    [drilldownNodeId, state.chatEntries],
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
      </header>

      <main className="grid min-h-0 flex-1 grid-cols-1 gap-2 p-2 lg:grid-cols-[20rem_minmax(0,1fr)]">
        <CockpitInbox items={inbox} />

        <div className="grid min-h-0 grid-rows-[minmax(0,1.1fr)_minmax(0,1fr)] gap-2">
          <section
            data-testid="cockpit-execution-flow"
            aria-label="自动执行流"
            className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] rounded-xl border-2 border-[var(--aria-line-strong)] bg-[var(--aria-panel)]"
          >
            <h2 className="px-3 py-2 text-sm font-semibold text-[var(--aria-ink)]">
              自动执行流
            </h2>
            <TimelineNodeList
              nodes={state.timelineNodes}
              // 下钻选中的行即 ② 区的「当前步」（aria-current="step"）；未下钻时退回 store 的进行中节点。
              activeNodeId={drilldownNodeId ?? state.activeNodeId}
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
              entries={state.chatEntries}
              sessionId={sessionId}
              contentCache={contentCacheValues}
              loadContent={loadContent}
              onCacheContent={cacheContent}
              testId="cockpit-conversation-flow-list"
            />
          </section>
        </div>
      </main>
    </div>
  );
}
