import { ArrowLeft, Trash2, Wifi, WifiOff } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { deleteCodingAttempt } from "../api/client";
import { CodingTimeline } from "../components/coding-workspace/CodingTimeline";
import { CodingProviderConfigPanel } from "../components/coding-workspace/CodingProviderConfigPanel";
import { RoleRunHistoryPanel } from "../components/coding-workspace/RoleRunHistoryPanel";
import {
  ChatEntryList,
  type ChatEntryListHandle,
} from "../components/chat-workspace/ChatEntryList";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import type { ChatEntry, ChoiceResponsePayload } from "../state/chat-entries";
import { useCodingWorkspaceStore } from "../state/coding-workspace-store";
import { CodingArtifactTabs } from "./CodingWorkspaceArtifacts";
import {
  ACTIVE_ATTEMPT_STATUSES,
  ActionButtons,
  CodingComposer,
  CodingPanelTabs,
  GatePanel,
  errorMessage,
  lockedProviderRole,
  requestIdFromEntry,
} from "./CodingWorkspaceControls";
import { CodingWorkspaceGroupProgress } from "./CodingWorkspaceGroupProgress";
import { PrepareExecutionPlanPanel, StatusBadge } from "./CodingWorkspaceReports";

export function CodingWorkspacePage({
  attemptId,
  onBack,
}: {
  attemptId: string;
  onBack: () => void;
}) {
  const api = useCodingWorkspaceWs(attemptId);
  const store = useCodingWorkspaceStore();
  const connected = store.connectionStatus === "connected";
  const activeTab = store.activeTab;
  const [activePanel, setActivePanel] = useState<"chat" | "results">("chat");
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const chatListRef = useRef<ChatEntryListHandle | null>(null);
  const pageError = planError ?? deleteError;

  useUnloadGuard({
    enabled: store.status === "running",
    message: "Coding attempt 运行中。刷新/关闭可能中断当前操作，是否继续？",
  });

  async function handleDeleteCodingWorkspace() {
    const targetAttemptId = store.attemptId ?? attemptId;
    const active = ACTIVE_ATTEMPT_STATUSES.has(store.status ?? "created");
    const message = active
      ? "运行中的 Attempt 会被终止并删除。本操作会删除 Coding Workspace 的日志、测试输出和 worktree，且无法撤销。"
      : "本操作会删除 Coding Workspace 的日志、测试输出和 worktree，且无法撤销。";
    if (!window.confirm(message)) {
      return;
    }

    setDeleteBusy(true);
    setDeleteError(null);
    try {
      await deleteCodingAttempt(targetAttemptId);
      onBack();
    } catch (reason) {
      setDeleteError(errorMessage(reason, "删除 Coding Workspace 失败"));
    } finally {
      setDeleteBusy(false);
    }
  }

  return (
    <div className="flex h-screen min-w-0 flex-col overflow-hidden bg-[var(--aria-bg)] text-[var(--aria-ink)]">
      <div className="flex h-11 min-w-0 shrink-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3">
        <button
          type="button"
          onClick={onBack}
          className="inline-flex h-8 shrink-0 items-center gap-2 rounded-md px-2 text-sm text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]"
        >
          <ArrowLeft className="h-4 w-4" />
          返回
        </button>
        <div className="min-w-0 flex-1 truncate text-center text-sm font-semibold">
          Coding Attempt #{store.attemptId ?? attemptId}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            disabled={deleteBusy}
            onClick={() => void handleDeleteCodingWorkspace()}
            className="inline-flex h-8 shrink-0 items-center gap-1.5 rounded-md border border-[var(--aria-danger)] bg-white px-2 text-xs font-semibold text-[var(--aria-danger)] hover:bg-red-50 disabled:opacity-50"
          >
            <Trash2 className="h-3.5 w-3.5" />
            删除 Coding Workspace
          </button>
          <StatusBadge value={store.status ?? "created"} />
          {connected ? (
            <Wifi aria-label="已连接" className="h-4 w-4 text-[var(--aria-success)]" />
          ) : (
            <WifiOff aria-label="未连接" className="h-4 w-4 text-[var(--aria-danger)]" />
          )}
        </div>
      </div>

      <header className="grid min-h-16 min-w-0 shrink-0 gap-2 overflow-hidden border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-3 md:grid-cols-[minmax(0,1fr)_auto]">
        <div className="min-w-0">
          <div className="flex min-w-0 flex-wrap items-center gap-2">
            <span className="text-xs font-semibold uppercase text-[var(--aria-ink-muted)]">
              {store.stage ?? "prepare_context"}
            </span>
            <span className="text-xs text-[var(--aria-ink-muted)]">
              {store.baseBranch ?? "HEAD"} {"->"} {store.branchName ?? "未创建分支"}
            </span>
          </div>
          <div className="mt-1 truncate font-mono text-xs text-[var(--aria-ink-muted)]">
            {store.worktreePath ?? "worktree pending"}
          </div>
        </div>
        <div className="flex min-w-0 items-center justify-end gap-2">
          <ActionButtons api={api} stage={store.stage} status={store.status} />
        </div>
      </header>
      {store.attemptScope === "work_item_group" && store.units.length > 0 ? (
        <CodingWorkspaceGroupProgress
          planId={store.workItemGroupId}
          currentWorkItemId={store.currentWorkItemId}
          units={store.units}
        />
      ) : null}

      <main className="grid min-h-0 min-w-0 flex-1 grid-cols-1 overflow-hidden md:grid-cols-[16rem_minmax(0,1fr)]">
        <CodingTimeline
          nodes={store.timelineNodes}
          activeNodeId={store.activeNodeId}
          selectedNodeId={store.selectedNodeId}
          latestAnalystDecision={store.latestAnalystDecision}
          onSelectNode={(nodeId) => {
            useCodingWorkspaceStore.getState().setSelectedNode(nodeId);
            const targetEntry = useCodingWorkspaceStore
              .getState()
              .chatEntries.find((entry) => entry.node_id === nodeId);
            if (targetEntry) {
              chatListRef.current?.scrollToEntry(targetEntry.id);
            }
          }}
        />
        <section className="grid min-h-0 min-w-0 grid-rows-[auto_minmax(0,1fr)] overflow-hidden bg-[var(--aria-panel)]">
          <CodingPanelTabs activePanel={activePanel} onSelectPanel={setActivePanel} />
          {activePanel === "results" ? (
            <CodingArtifactTabs activeTab={activeTab} className="min-h-0" />
          ) : (
            <div
              className={[
                "grid min-h-0 min-w-0 overflow-hidden",
                store.stage === "prepare_context" && store.workItemExecutionPlan
                  ? "grid-rows-[auto_auto_auto_minmax(0,1fr)_auto_auto]"
                  : "grid-rows-[auto_auto_minmax(0,1fr)_auto_auto]",
              ].join(" ")}
            >
              {store.stage === "prepare_context" && store.workItemExecutionPlan ? (
                <PrepareExecutionPlanPanel
                  attemptId={attemptId}
                  plan={store.workItemExecutionPlan}
                  requireConfirm={store.requireExecutionPlanConfirm}
                  onError={setPlanError}
                />
              ) : null}
              <CodingProviderConfigPanel
                snapshot={store.roleProviderConfigSnapshot}
                lockedRole={lockedProviderRole(store.stage, store.status, store.pendingGates)}
                onSelect={api.sendProviderSelect}
                onPermissionModeSelect={api.sendPermissionModeSelect}
              />
              <RoleRunHistoryPanel
                roleRuns={store.roleRuns}
                timelineNodes={store.timelineNodes}
                selectedNodeId={store.selectedNodeId}
                onSelectNode={(nodeId) => {
                  useCodingWorkspaceStore.getState().setSelectedNode(nodeId);
                  const targetEntry = useCodingWorkspaceStore
                    .getState()
                    .chatEntries.find((entry) => entry.node_id === nodeId);
                  if (targetEntry) {
                    chatListRef.current?.scrollToEntry(targetEntry.id);
                  }
                }}
              />
              <ChatEntryList
                ref={chatListRef}
                entries={store.chatEntries}
                onPermissionResponse={handlePermissionResponse}
                onChoiceResponse={handleChoiceResponse}
              />
              <GatePanel
                gate={store.pendingGates.at(-1) ?? null}
                onRespond={api.respondGate}
                onConfirmStage={api.confirmStageGate}
                onAbort={api.abortAttempt}
              />
              <CodingComposer
                api={api}
                stage={store.stage}
                status={store.status}
                statusText={
                  store.protocolError
                    ? `${store.protocolError.code}: ${store.protocolError.message}`
                    : store.pendingGates.at(-1)?.title ?? "Coding Workspace"
                }
              />
            </div>
          )}
        </section>
      </main>

      <div
        data-testid="coding-status-bar"
        className="flex h-8 shrink-0 items-center justify-between gap-3 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-xs text-[var(--aria-ink-muted)]"
      >
        <span>{store.stage ?? "prepare_context"}</span>
        <span className={pageError ? "text-[var(--aria-danger)]" : undefined}>
          {pageError ?? store.connectionStatus}
        </span>
        <span>rework {store.reworkCount}/{store.maxAutoRework}</span>
      </div>
    </div>
  );

  function handlePermissionResponse(entry: ChatEntry, approved: boolean) {
    const requestId = requestIdFromEntry(entry);
    if (!requestId) return;
    api.respondPermission(requestId, approved);
  }

  function handleChoiceResponse(entry: ChatEntry, response: ChoiceResponsePayload) {
    const requestId = requestIdFromEntry(entry);
    if (!requestId) return;
    api.respondChoice(requestId, response.selected_option_ids, response.free_text);
  }
}
