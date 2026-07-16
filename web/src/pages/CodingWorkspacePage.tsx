import { ArrowLeft, History, Settings2, Trash2, Wifi, WifiOff, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { deleteCodingAttempt } from "../api/client";
import type { CodingAttemptAddress } from "../api/types";
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

type CodingWorkspaceDrawer = "providers" | "runs";

export function CodingWorkspacePage({
  address,
  onBack,
}: {
  address: CodingAttemptAddress;
  onBack: () => void;
}) {
  const api = useCodingWorkspaceWs(address);
  const store = useCodingWorkspaceStore();
  const connected = store.connectionStatus === "connected";
  const activeTab = store.activeTab;
  const [activePanel, setActivePanel] = useState<"chat" | "results">("chat");
  const [deleteBusy, setDeleteBusy] = useState(false);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [planError, setPlanError] = useState<string | null>(null);
  const [activeDrawer, setActiveDrawer] = useState<CodingWorkspaceDrawer | null>(null);
  const chatListRef = useRef<ChatEntryListHandle | null>(null);
  const addressKey = JSON.stringify([address.projectId, address.issueId, address.attemptId]);
  const addressKeyRef = useRef(addressKey);
  const deleteGenerationRef = useRef(0);
  const mountedRef = useRef(false);
  if (addressKeyRef.current !== addressKey) {
    addressKeyRef.current = addressKey;
    deleteGenerationRef.current += 1;
  }
  const pageError = planError ?? deleteError;
  const providerSummary = store.roleProviderConfigSnapshot
    ? `Coder ${store.roleProviderConfigSnapshot.coder} · Reviewer ${store.roleProviderConfigSnapshot.code_reviewer}`
    : "provider pending";
  const roleRunSummary =
    store.roleRuns.length === 0 ? "暂无运行记录" : `${store.roleRuns.length} 次角色运行`;
  const pendingGate = store.pendingGates.at(-1) ?? null;
  const storeMatchesAddress =
    store.projectId === address.projectId &&
    store.issueId === address.issueId &&
    store.attemptId === address.attemptId;

  useUnloadGuard({
    enabled: store.status === "running",
    message: "Coding attempt 运行中。刷新/关闭可能中断当前操作，是否继续？",
  });

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      deleteGenerationRef.current += 1;
    };
  }, []);

  useEffect(() => {
    setDeleteBusy(false);
    setDeleteError(null);
    setPlanError(null);
  }, [addressKey]);

  function isCurrentDeleteRequest(requestAddressKey: string, requestGeneration: number) {
    return (
      mountedRef.current &&
      addressKeyRef.current === requestAddressKey &&
      deleteGenerationRef.current === requestGeneration
    );
  }

  async function handleDeleteCodingWorkspace() {
    if (!storeMatchesAddress) {
      return;
    }
    const active = ACTIVE_ATTEMPT_STATUSES.has(store.status ?? "created");
    const message = active
      ? "运行中的 Attempt 会被终止并删除。本操作会删除 Coding Workspace 的日志、测试输出和 worktree，且无法撤销。"
      : "本操作会删除 Coding Workspace 的日志、测试输出和 worktree，且无法撤销。";
    if (!window.confirm(message)) {
      return;
    }

    const requestAddress = { ...address };
    const requestAddressKey = addressKey;
    const requestGeneration = deleteGenerationRef.current + 1;
    deleteGenerationRef.current = requestGeneration;
    setDeleteBusy(true);
    setDeleteError(null);
    try {
      await deleteCodingAttempt(requestAddress);
      if (isCurrentDeleteRequest(requestAddressKey, requestGeneration)) {
        onBack();
      }
    } catch (reason) {
      if (isCurrentDeleteRequest(requestAddressKey, requestGeneration)) {
        setDeleteError(errorMessage(reason, "删除 Coding Workspace 失败"));
      }
    } finally {
      if (isCurrentDeleteRequest(requestAddressKey, requestGeneration)) {
        setDeleteBusy(false);
      }
    }
  }

  function handleSelectTimelineNode(nodeId: string) {
    useCodingWorkspaceStore.getState().setSelectedNode(nodeId);
    const targetEntry = useCodingWorkspaceStore
      .getState()
      .chatEntries.find((entry) => entry.node_id === nodeId);
    if (targetEntry) {
      chatListRef.current?.scrollToEntry(targetEntry.id);
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
          Coding Attempt #{store.attemptId ?? address.attemptId}
        </div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="button"
            disabled={deleteBusy || !storeMatchesAddress}
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
              {displayCodingStage(store.stage ?? "prepare_context")}
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
          {pendingGate?.kind === "blocked" ? null : (
            <ActionButtons api={api} stage={store.stage} status={store.status} />
          )}
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
          onSelectNode={handleSelectTimelineNode}
        />
        <section className="grid min-h-0 min-w-0 grid-rows-[auto_minmax(0,1fr)] overflow-hidden bg-[var(--aria-panel)]">
          <CodingPanelTabs activePanel={activePanel} onSelectPanel={setActivePanel} />
          {activePanel === "results" ? (
            <CodingArtifactTabs
              address={address}
              activeTab={activeTab}
              className="min-h-0"
            />
          ) : (
            <div
              className={[
                "grid min-h-0 min-w-0 overflow-hidden",
                store.stage === "prepare_context" && store.workItemExecutionPlan
                  ? "grid-rows-[auto_auto_minmax(0,1fr)_auto_auto]"
                  : "grid-rows-[auto_minmax(0,1fr)_auto_auto]",
              ].join(" ")}
            >
              {store.stage === "prepare_context" && store.workItemExecutionPlan ? (
                <PrepareExecutionPlanPanel
                  address={address}
                  plan={store.workItemExecutionPlan}
                  requireConfirm={store.requireExecutionPlanConfirm}
                  onError={setPlanError}
                />
              ) : null}
              <div
                data-testid="coding-chat-toolbar"
                className="flex h-11 min-w-0 items-center justify-between gap-2 border-b border-[var(--aria-line)] bg-white px-3"
              >
                <div className="min-w-0 truncate text-xs text-[var(--aria-ink-muted)]">
                  <span className="font-semibold text-[var(--aria-ink)]">运行对话</span>
                  <span className="ml-2">{store.chatEntries.length} 条消息</span>
                </div>
                <div className="flex min-w-0 shrink-0 items-center gap-1.5">
                  <button
                    type="button"
                    aria-label="Provider 设置"
                    onClick={() => setActiveDrawer("providers")}
                    className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)]"
                  >
                    <Settings2 className="h-3.5 w-3.5" />
                    <span>Provider 设置</span>
                    <span
                      aria-hidden="true"
                      className="hidden max-w-[14rem] truncate font-mono text-[11px] font-normal text-[var(--aria-ink-muted)] lg:inline"
                    >
                      {providerSummary}
                    </span>
                  </button>
                  <button
                    type="button"
                    aria-label="角色运行历史"
                    onClick={() => setActiveDrawer("runs")}
                    className="inline-flex h-8 cursor-pointer items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)]"
                  >
                    <History className="h-3.5 w-3.5" />
                    <span>角色运行历史</span>
                    <span
                      aria-hidden="true"
                      className="hidden max-w-[8rem] truncate text-[11px] font-normal text-[var(--aria-ink-muted)] md:inline"
                    >
                      {roleRunSummary}
                    </span>
                  </button>
                </div>
              </div>
              <div
                data-testid="coding-chat-entry-list"
                className="min-h-0 min-w-0 overflow-hidden"
              >
                <ChatEntryList
                  ref={chatListRef}
                  entries={store.chatEntries}
                  onPermissionResponse={handlePermissionResponse}
                  onChoiceResponse={handleChoiceResponse}
                  className="h-full"
                />
              </div>
              <GatePanel
                gate={pendingGate}
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
                    : pendingGate?.title ?? "Coding Workspace"
                }
                pendingGate={pendingGate}
              />
            </div>
          )}
        </section>
      </main>

      {activeDrawer ? (
        <div className="fixed inset-0 z-40 flex justify-end bg-black/20">
          <button
            type="button"
            aria-label="关闭辅助面板"
            className="absolute inset-0 cursor-default"
            onClick={() => setActiveDrawer(null)}
          />
          <aside
            role="dialog"
            aria-modal="true"
            aria-label={activeDrawer === "providers" ? "Provider 设置" : "角色运行历史"}
            className="relative grid h-full w-full max-w-[42rem] grid-rows-[auto_minmax(0,1fr)] overflow-hidden border-l border-[var(--aria-line)] bg-white shadow-xl"
          >
            <div className="flex h-12 min-w-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] px-4">
              <div className="min-w-0">
                <div className="truncate text-sm font-semibold text-[var(--aria-ink)]">
                  {activeDrawer === "providers" ? "Provider 设置" : "角色运行历史"}
                </div>
                <div className="truncate text-xs text-[var(--aria-ink-muted)]">
                  {activeDrawer === "providers" ? providerSummary : roleRunSummary}
                </div>
              </div>
              <button
                type="button"
                aria-label="关闭辅助面板"
                onClick={() => setActiveDrawer(null)}
                className="inline-flex h-8 w-8 cursor-pointer items-center justify-center rounded-md text-[var(--aria-ink-muted)] transition-colors hover:bg-[var(--aria-panel-muted)]"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div className="min-h-0 overflow-auto p-4">
              {activeDrawer === "providers" ? (
                <CodingProviderConfigPanel
                  snapshot={store.roleProviderConfigSnapshot}
                  attemptScope={store.attemptScope}
                  lockedRole={lockedProviderRole(store.stage, store.status, store.pendingGates)}
                  configLocked={
                    store.status !== "created" || store.stage !== "prepare_context"
                  }
                  maxAutoRework={store.maxAutoRework}
                  onSelect={api.sendProviderSelect}
                  onPermissionModeSelect={api.sendPermissionModeSelect}
                  onMaxAutoReworkSelect={api.sendMaxAutoReworkSelect}
                />
              ) : (
                <RoleRunHistoryPanel
                  roleRuns={store.roleRuns}
                  timelineNodes={store.timelineNodes}
                  selectedNodeId={store.selectedNodeId}
                  onSelectNode={handleSelectTimelineNode}
                />
              )}
            </div>
          </aside>
        </div>
      ) : null}

      <div
        data-testid="coding-status-bar"
        className="flex h-8 shrink-0 items-center justify-between gap-3 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 text-xs text-[var(--aria-ink-muted)]"
      >
        <span>{displayCodingStage(store.stage ?? "prepare_context")}</span>
        <span className={pageError ? "text-[var(--aria-danger)]" : undefined}>
          {pageError ?? store.connectionStatus}
        </span>
        <span>Coder 修复次数 {store.reworkCount}/{store.maxAutoRework}</span>
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

function displayCodingStage(stage: string) {
  const labels: Record<string, string> = {
    prepare_context: "准备上下文",
    worktree_prepare: "准备 Worktree",
    coding: "Coder",
    testing: "Tester",
    code_review: "Code Reviewer",
    review_request: "准备 PR",
    internal_pr_review: "GroupFinalReview",
    final_confirm: "最终确认",
  };
  return labels[stage] ?? stage;
}
