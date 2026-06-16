import {
  ArrowLeft,
  Check,
  Play,
  RotateCcw,
  Send,
  Trash2,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import {
  confirmWorkItemExecutionPlan,
  deleteCodingAttempt,
  getCodingAttemptDiff,
  requestWorkItemExecutionPlanChange,
} from "../api/client";
import type {
  AnalystDecisionRecord,
  CodeReviewReport,
  CodingExecutionStage,
  CodingGateRequired,
  CodingProviderRole,
  InternalPrReview,
  ReviewFinding,
  TestingStepResult,
  WorkItemExecutionPlan,
} from "../api/types";
import { CodingTimeline } from "../components/coding-workspace/CodingTimeline";
import { CodingProviderConfigPanel } from "../components/coding-workspace/CodingProviderConfigPanel";
import { RoleRunHistoryPanel } from "../components/coding-workspace/RoleRunHistoryPanel";
import { StageGateEntry } from "../components/coding-workspace/StageGateEntry";
import {
  ChatEntryList,
  type ChatEntryListHandle,
} from "../components/chat-workspace/ChatEntryList";
import { MonacoDiffViewer } from "../components/shared/MonacoDiffViewer";
import { MonacoViewer } from "../components/shared/MonacoViewer";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import {
  type CodingPendingGate,
  type CodingArtifactTab,
  useCodingWorkspaceStore,
} from "../state/coding-workspace-store";
import type { ChatEntry, ChoiceResponsePayload } from "../state/chat-entries";

const ACTIVE_ATTEMPT_STATUSES = new Set(["created", "running", "waiting_for_human", "blocked"]);

type CodingDiffState = {
  attemptId: string | null;
  status: "idle" | "loading" | "loaded" | "error";
  diff: string;
  error: string | null;
};

const TESTING_BLOCKED_REASON_LABELS: Record<string, string> = {
  test_plan_missing_json: "Tester 未返回测试计划 JSON",
  test_plan_invalid_json: "Tester 返回的 JSON 无法解析",
  test_plan_schema_invalid: "Tester 测试计划字段不完整",
  test_plan_repair_failed: "Tester 测试计划修复失败",
  missing_required_steps: "缺少 required 测试步骤证据",
  skipped_required_steps: "required 测试步骤被阻塞（无法执行）",
  testing_blocked: "测试被阻塞",
  high_risk_test_step_requires_permission: "高风险测试步骤需要人工确认",
};

const TESTING_RESULT_REVIEW_REASON_CODE = "testing_result_review_required";

function blockedGateDisplayTitle(gate: CodingPendingGate) {
  if (gate.reason_code === TESTING_RESULT_REVIEW_REASON_CODE) {
    return gate.title;
  }
  if (gate.stage === "testing" && gate.reason_code) {
    return TESTING_BLOCKED_REASON_LABELS[gate.reason_code] ?? gate.reason_code;
  }
  return gate.title;
}

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
  const chatListRef = useRef<ChatEntryListHandle | null>(null);

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
        <span className={deleteError ? "text-[var(--aria-danger)]" : undefined}>
          {deleteError ?? store.connectionStatus}
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

function CodingPanelTabs({
  activePanel,
  onSelectPanel,
}: {
  activePanel: "chat" | "results";
  onSelectPanel: (panel: "chat" | "results") => void;
}) {
  return (
    <div className="flex min-w-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2">
      <div className="flex min-w-0 items-center gap-1">
        <button
          type="button"
          onClick={() => onSelectPanel("chat")}
          className={codingPanelTabClass(activePanel === "chat")}
        >
          运行对话
        </button>
        <button
          type="button"
          onClick={() => onSelectPanel("results")}
          className={codingPanelTabClass(activePanel === "results")}
        >
          运行结果
        </button>
      </div>
    </div>
  );
}

function codingPanelTabClass(active: boolean) {
  return [
    "inline-flex h-8 items-center rounded-md px-3 text-xs font-semibold transition-colors",
    active
      ? "bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
      : "text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]",
  ].join(" ");
}

function errorMessage(reason: unknown, fallback: string) {
  return reason instanceof Error ? reason.message : fallback;
}

function requestIdFromEntry(entry: ChatEntry) {
  const requestId = entry.metadata?.request_id;
  return typeof requestId === "string" ? requestId : null;
}

function CodingComposer({
  api,
  stage,
  status,
  statusText,
}: {
  api: ReturnType<typeof useCodingWorkspaceWs>;
  stage: CodingExecutionStage | null;
  status: string | null;
  statusText: string;
}) {
  const [input, setInput] = useState("");
  const trimmedInput = input.trim();
  const inputDisabled = status === "completed" || status === "aborted";
  const canSend = !inputDisabled && trimmedInput.length > 0;

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!canSend) {
      return;
    }
    api.sendContextNote(trimmedInput);
    setInput("");
  }

  return (
    <form
      onSubmit={handleSubmit}
      className="grid gap-2 border-t border-[var(--aria-line)] bg-white px-3 py-2"
    >
      <textarea
        aria-label="补充 Coding 上下文"
        value={input}
        onChange={(event) => setInput(event.target.value)}
        disabled={inputDisabled}
        rows={2}
        placeholder="补充上下文"
        className="min-h-16 w-full resize-y rounded-md border border-[var(--aria-line)] px-3 py-2 text-sm text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
      />
      <div className="flex min-w-0 items-center justify-between gap-2">
        <div className="truncate text-xs text-[var(--aria-ink-muted)]">{statusText}</div>
        <div className="flex shrink-0 items-center gap-2">
          <button
            type="submit"
            disabled={!canSend}
            className="inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
          >
            <Send className="h-3.5 w-3.5" />
            发送上下文
          </button>
          <ActionButtons api={api} stage={stage} status={status} compact />
        </div>
      </div>
    </form>
  );
}

function ActionButtons({
  api,
  stage,
  status,
  compact = false,
}: {
  api: ReturnType<typeof useCodingWorkspaceWs>;
  stage: CodingExecutionStage | null;
  status: string | null;
  compact?: boolean;
}) {
  const buttonClass = compact
    ? "inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold hover:bg-[var(--aria-panel-muted)]"
    : "inline-flex h-8 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold hover:bg-[var(--aria-panel-muted)]";

  if (stage === "prepare_context") {
    return (
      <button
        type="button"
        onClick={api.startCoding}
        className={buttonClass}
        aria-label={compact ? "底部开始 Coding" : undefined}
      >
        <Play className="h-3.5 w-3.5" />
        开始 Coding
      </button>
    );
  }

  if (stage === "final_confirm" && status === "waiting_for_human") {
    return (
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={api.finalConfirm}
          className={buttonClass}
          aria-label={compact ? "底部确认完成" : undefined}
        >
          <Check className="h-3.5 w-3.5" />
          确认完成
        </button>
        <button
          type="button"
          onClick={api.abortAttempt}
          className={buttonClass}
          aria-label={compact ? "底部中止" : undefined}
        >
          <X className="h-3.5 w-3.5" />
          中止
        </button>
      </div>
    );
  }

  if (stage === "rework" && status === "waiting_for_human") {
    return (
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={() => api.continueRework(null)}
          className={buttonClass}
          aria-label={compact ? "底部继续返修" : undefined}
        >
          <RotateCcw className="h-3.5 w-3.5" />
          继续返修
        </button>
        <button
          type="button"
          onClick={api.abortAttempt}
          className={buttonClass}
          aria-label={compact ? "底部中止" : undefined}
        >
          <X className="h-3.5 w-3.5" />
          中止
        </button>
      </div>
    );
  }

  if (status && ACTIVE_ATTEMPT_STATUSES.has(status)) {
    return (
      <button
        type="button"
        onClick={api.abortAttempt}
        className={buttonClass}
        aria-label={compact ? "底部中止" : undefined}
      >
        <X className="h-3.5 w-3.5" />
        中止
      </button>
    );
  }

  return null;
}

function GatePanel({
  gate,
  onRespond,
  onConfirmStage,
  onAbort,
}: {
  gate: CodingPendingGate | null;
  onRespond: ReturnType<typeof useCodingWorkspaceWs>["respondGate"];
  onConfirmStage: ReturnType<typeof useCodingWorkspaceWs>["confirmStageGate"];
  onAbort: ReturnType<typeof useCodingWorkspaceWs>["abortAttempt"];
}) {
  const [reason, setReason] = useState("");
  const [localError, setLocalError] = useState<string | null>(null);

  if (!gate) {
    return null;
  }

  if (gate.kind === "stage_gate") {
    return (
      <StageGateEntry gate={gate} onConfirmStage={onConfirmStage} onAbort={onAbort} />
    );
  }

  const activeGate = gate;
  const submitting = activeGate.submitting === true;
  const gateErrorCode = activeGate.errorCode ?? null;
  const needsReason = activeGate.available_actions.some(actionRequiresReason);
  const trimmedReason = reason.trim();
  const reasonTooLong = reason.length > 2000;
  const displayedError = reasonTooLong ? "原因不能超过 2000 字" : localError;
  const displayTitle = blockedGateDisplayTitle(activeGate);
  const testingResultReview = activeGate.reason_code === TESTING_RESULT_REVIEW_REASON_CODE;
  const testingBlocked = activeGate.stage === "testing" && !testingResultReview;
  const analystGate = activeGate.role === "analyst";
  const hasQualityBypassAction = activeGate.available_actions.some(actionRequiresReason);

  function handleAction(action: CodingGateRequired["available_actions"][number]) {
    if (action.action_type === "confirm_stage" && activeGate.stage) {
      onConfirmStage(activeGate.stage);
      return;
    }
    if (actionRequiresReason(action)) {
      if (!trimmedReason) {
        setLocalError("需要填写原因");
        return;
      }
      if (reasonTooLong) {
        return;
      }
      setLocalError(null);
      onRespond(activeGate.gate_id, action.action_id, trimmedReason);
      return;
    }
    setLocalError(null);
    onRespond(activeGate.gate_id, action.action_id, undefined);
  }

  return (
    <div
      data-testid="coding-pending-gate"
      className="border-t border-amber-200 bg-amber-50 px-3 py-2"
    >
      <div className="flex min-w-0 flex-col gap-2 md:flex-row md:items-center md:justify-between">
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-amber-900">{displayTitle}</div>
          {testingBlocked ? (
            <div className="mt-0.5 text-xs font-semibold text-amber-900">测试被阻塞</div>
          ) : null}
          {testingResultReview ? (
            <div className="mt-0.5 text-xs font-semibold text-amber-900">
              等待确认 Tester 结果
            </div>
          ) : null}
          {analystGate ? (
            <div className="mt-0.5 text-xs font-semibold text-amber-900">
              Analyst 建议人工决策
            </div>
          ) : null}
          <div className="mt-0.5 line-clamp-2 text-xs text-amber-800">
            {activeGate.description}
          </div>
          {hasQualityBypassAction ? (
            <div className="mt-1 text-xs font-semibold text-amber-900">
              人工放行会记录质量豁免；请说明跳过该门禁的原因和后续风险处理
            </div>
          ) : null}
          <GateMetadata gate={activeGate} />
          {needsReason ? (
            <div className="mt-2 grid gap-1">
              <textarea
                aria-label="门禁跳过原因"
                value={reason}
                onChange={(event) => {
                  setReason(event.target.value);
                  setLocalError(null);
                }}
                rows={2}
                maxLength={2100}
                placeholder="说明跳过该门禁的原因和后续风险处理"
                className="min-h-14 w-full resize-y rounded-md border border-amber-300 bg-white px-2 py-1.5 text-xs text-amber-950 placeholder:text-amber-700"
              />
              {displayedError ? (
                <div className="text-xs font-semibold text-[var(--aria-danger)]">
                  {displayedError}
                </div>
              ) : null}
            </div>
          ) : null}
          {gateErrorCode ? (
            <div className="mt-1 text-xs font-semibold text-[var(--aria-danger)]">
              {gateErrorCode}
            </div>
          ) : null}
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          {activeGate.available_actions.map((action) => (
            <button
              key={action.action_id}
              type="button"
              disabled={submitting || (actionRequiresReason(action) && reasonTooLong)}
              onClick={() => handleAction(action)}
              className="inline-flex h-8 items-center justify-center rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-900 hover:bg-amber-100"
            >
              {submitting ? "处理中" : action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function actionRequiresReason(action: CodingGateRequired["available_actions"][number]) {
  return action.action_type === "manual_continue" || action.action_type === "accept_risk";
}

function GateMetadata({ gate }: { gate: CodingPendingGate }) {
  const rows = [
    gate.reason_code ? ["reason", gate.reason_code] : null,
    gate.raw_provider_output_ref ? ["raw", gate.raw_provider_output_ref] : null,
    gate.evidence_refs?.length ? ["evidence", gate.evidence_refs.join(", ")] : null,
  ].filter(Boolean) as [string, string][];
  if (rows.length === 0) {
    return null;
  }
  return (
    <dl className="mt-1 grid gap-0.5 text-xs text-amber-800">
      {rows.map(([label, value]) => (
        <div key={label} className="grid min-w-0 grid-cols-[4.5rem_minmax(0,1fr)] gap-2">
          <dt className="font-semibold">{label}</dt>
          <dd className="min-w-0 break-words font-mono">{value}</dd>
        </div>
      ))}
    </dl>
  );
}

function lockedProviderRole(
  stage: CodingExecutionStage | null,
  status: string | null,
  pendingGates: CodingGateRequired[],
): CodingProviderRole | null {
  if (pendingGates.some((gate) => gate.kind === "stage_gate")) {
    return null;
  }
  if (status !== "running" || !stage) {
    return null;
  }
  return providerRoleForStage(stage);
}

function providerRoleForStage(stage: CodingExecutionStage): CodingProviderRole | null {
  switch (stage) {
    case "coding":
      return "coder";
    case "testing":
      return "tester";
    case "rework":
      return "analyst";
    case "code_review":
      return "code_reviewer";
    case "internal_pr_review":
      return "internal_reviewer";
    default:
      return null;
  }
}

function CodingArtifactTabs({
  activeTab,
  className = "",
}: {
  activeTab: CodingArtifactTab;
  className?: string;
}) {
  const attemptId = useCodingWorkspaceStore((state) => state.attemptId);
  const [diffState, setDiffState] = useState<CodingDiffState>({
    attemptId: null,
    status: "idle",
    diff: "",
    error: null,
  });
  const tabs: CodingArtifactTab[] = ["diff", "tests", "review", "git", "logs"];

  useEffect(() => {
    if (activeTab !== "diff" || !attemptId) {
      return;
    }
    if (diffState.attemptId === attemptId && diffState.status === "loaded") {
      return;
    }

    let cancelled = false;
    setDiffState({
      attemptId,
      status: "loading",
      diff: "",
      error: null,
    });
    getCodingAttemptDiff(attemptId)
      .then((response) => {
        if (cancelled) return;
        setDiffState({
          attemptId,
          status: "loaded",
          diff: response.diff,
          error: null,
        });
      })
      .catch((reason) => {
        if (cancelled) return;
        setDiffState({
          attemptId,
          status: "error",
          diff: "",
          error: errorMessage(reason, "加载代码变更失败"),
        });
      });

    return () => {
      cancelled = true;
    };
  }, [activeTab, attemptId]);

  return (
    <aside
      data-testid="coding-artifact-tabs"
      className={`flex min-h-0 flex-col bg-[var(--aria-panel)] ${className}`}
    >
      <div className="flex shrink-0 border-b border-[var(--aria-line)] px-2 py-2">
        {tabs.map((tab) => (
          <button
            key={tab}
            type="button"
            onClick={() => useCodingWorkspaceStore.getState().setActiveTab(tab)}
            className={[
              "h-8 rounded-md px-2 text-xs font-semibold",
              activeTab === tab
                ? "bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
                : "text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]",
            ].join(" ")}
          >
            {tab}
          </button>
        ))}
      </div>
      <div className="min-h-0 flex-1 overflow-auto p-3 text-sm">
        {activeTab === "tests" ? (
          <TestsPanel />
        ) : activeTab === "review" ? (
          <ReviewPanel />
        ) : activeTab === "git" ? (
          <GitPanel />
        ) : activeTab === "logs" ? (
          <LogsPanel />
        ) : (
          <DiffPanel diffState={diffState} />
        )}
      </div>
    </aside>
  );
}

function DiffPanel({ diffState }: { diffState: CodingDiffState }) {
  if (diffState.status === "loading") {
    return <div className="text-[var(--aria-ink-muted)]">正在加载代码变更...</div>;
  }
  if (diffState.status === "error") {
    return <div className="text-[var(--aria-danger)]">{diffState.error}</div>;
  }
  if (!diffState.diff.trim()) {
    return <div className="text-[var(--aria-ink-muted)]">暂无代码变更</div>;
  }
  const files = parseUnifiedDiff(diffState.diff);
  if (files.length > 0) {
    return (
      <div className="space-y-3">
        {files.map((file) => (
          <div
            key={file.id}
            data-testid="coding-diff-file"
            className="overflow-hidden rounded-md border border-[var(--aria-line)] bg-white"
          >
            <div className="flex h-9 min-w-0 items-center gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3">
              <span className="shrink-0 rounded bg-[var(--aria-panel-subtle)] px-1.5 py-0.5 text-[10px] font-semibold uppercase text-[var(--aria-ink-muted)]">
                {diffStatusLabel(file.status)}
              </span>
              <span className="truncate font-mono text-xs text-[var(--aria-ink)]">
                {file.path}
              </span>
            </div>
            {file.binary ? (
              <div className="px-3 py-2 text-xs text-[var(--aria-ink-muted)]">
                二进制文件变更未展示内容
              </div>
            ) : (
              <MonacoDiffViewer
                original={file.original}
                modified={file.modified}
                language={languageForPath(file.path)}
                height="min(62vh, 620px)"
              />
            )}
          </div>
        ))}
      </div>
    );
  }
  return (
    <div className="min-h-[420px] overflow-hidden rounded-md border border-[var(--aria-line)]">
      <MonacoViewer value={diffState.diff} language="diff" height="min(70vh, 720px)" />
    </div>
  );
}

type ParsedDiffFile = {
  id: string;
  path: string;
  oldPath: string;
  newPath: string;
  original: string;
  modified: string;
  status: "added" | "deleted" | "modified" | "renamed" | "binary";
  binary: boolean;
};

type MutableParsedDiffFile = Omit<ParsedDiffFile, "original" | "modified"> & {
  originalLines: string[];
  modifiedLines: string[];
  inHunk: boolean;
  hunkCount: number;
};

function parseUnifiedDiff(diff: string): ParsedDiffFile[] {
  const files: MutableParsedDiffFile[] = [];
  let current: MutableParsedDiffFile | null = null;
  const lines = diff.replace(/\r\n?/g, "\n").split("\n");

  function pushCurrent() {
    if (current) {
      files.push(current);
      current = null;
    }
  }

  for (const line of lines) {
    const header = parseDiffHeader(line);
    if (header) {
      pushCurrent();
      current = {
        id: `${files.length}:${header.oldPath}:${header.newPath}`,
        oldPath: header.oldPath,
        newPath: header.newPath,
        path: header.newPath !== "/dev/null" ? header.newPath : header.oldPath,
        status: "modified",
        binary: false,
        originalLines: [],
        modifiedLines: [],
        inHunk: false,
        hunkCount: 0,
      };
      continue;
    }
    if (!current) {
      continue;
    }
    if (line.startsWith("new file mode")) {
      current.status = "added";
      continue;
    }
    if (line.startsWith("deleted file mode")) {
      current.status = "deleted";
      continue;
    }
    if (line.startsWith("rename to ")) {
      current.status = "renamed";
      current.newPath = line.slice("rename to ".length).trim();
      current.path = current.newPath;
      continue;
    }
    if (line.startsWith("Binary files ")) {
      current.status = "binary";
      current.binary = true;
      continue;
    }
    if (line.startsWith("@@")) {
      if (current.hunkCount > 0) {
        current.originalLines.push("");
        current.modifiedLines.push("");
      }
      current.inHunk = true;
      current.hunkCount += 1;
      continue;
    }
    if (!current.inHunk || line.startsWith("\\ No newline")) {
      continue;
    }
    const marker = line[0];
    const text = line.slice(1);
    if (marker === " ") {
      current.originalLines.push(text);
      current.modifiedLines.push(text);
    } else if (marker === "-") {
      current.originalLines.push(text);
    } else if (marker === "+") {
      current.modifiedLines.push(text);
    }
  }
  pushCurrent();

  return files.map(({ originalLines, modifiedLines, inHunk, hunkCount, ...file }) => ({
    ...file,
    original: originalLines.join("\n"),
    modified: modifiedLines.join("\n"),
  }));
}

function parseDiffHeader(line: string) {
  const match = /^diff --git a\/(.+) b\/(.+)$/.exec(line);
  if (!match) {
    return null;
  }
  return {
    oldPath: match[1],
    newPath: match[2],
  };
}

function diffStatusLabel(status: ParsedDiffFile["status"]) {
  switch (status) {
    case "added":
      return "新增";
    case "deleted":
      return "删除";
    case "renamed":
      return "重命名";
    case "binary":
      return "二进制";
    default:
      return "修改";
  }
}

function languageForPath(path: string) {
  const extension = path.split(".").pop()?.toLowerCase();
  switch (extension) {
    case "py":
      return "python";
    case "ts":
    case "tsx":
      return "typescript";
    case "js":
    case "jsx":
      return "javascript";
    case "json":
      return "json";
    case "md":
      return "markdown";
    case "rs":
      return "rust";
    case "toml":
      return "toml";
    case "yml":
    case "yaml":
      return "yaml";
    case "sh":
      return "shell";
    case "css":
      return "css";
    case "html":
      return "html";
    default:
      return "plaintext";
  }
}

function TestsPanel() {
  const report = useCodingWorkspaceStore((state) => state.testingReport);
  const stage = useCodingWorkspaceStore((state) => state.stage);
  const latestAnalystDecision = useCodingWorkspaceStore(
    (state) => state.latestAnalystDecision,
  );
  if (!report) {
    return <div className="text-[var(--aria-ink-muted)]">暂无测试报告</div>;
  }
  const steps = report.steps ?? [];
  const missingRequiredSteps = report.missing_required_steps ?? [];
  const skippedRequiredSteps = report.skipped_required_steps ?? [];
  const contextWarnings = report.context_warnings ?? [];
  const unplannedCommands = report.unplanned_commands ?? [];
  const unplannedEvidence = report.unplanned_evidence ?? [];
  const hasPlanDetails =
    Boolean(report.plan_summary) ||
    steps.length > 0 ||
    missingRequiredSteps.length > 0 ||
    skippedRequiredSteps.length > 0 ||
    contextWarnings.length > 0 ||
    Boolean(report.raw_provider_output_ref);

  return (
    <div className="space-y-3">
      <StatusBadge value={report.overall_status} />
      <AnalystDecisionStatus
        decision={latestAnalystDecision}
        waiting={stage === "rework" && !latestAnalystDecision}
      />
      {hasPlanDetails ? (
        <div data-testid="coding-test-plan-report" className="space-y-2">
          {report.plan_summary ? (
            <div>
              <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">
                Test Plan
              </div>
              <div className="mt-0.5 break-words text-sm font-semibold">
                {report.plan_summary}
              </div>
            </div>
          ) : null}
          {steps.length > 0 ? (
            <div className="space-y-2">
              {steps.map((step) => (
                <TestingStepResultRow key={step.step_id} step={step} />
              ))}
            </div>
          ) : null}
          <TestingList label="missing required" values={missingRequiredSteps} />
          <TestingList label="skipped required" values={skippedRequiredSteps} />
          <TestingList label="context warning" values={contextWarnings} />
          {report.raw_provider_output_ref ? (
            <EvidencePath label="raw output" value={report.raw_provider_output_ref} />
          ) : null}
          {unplannedEvidence.length > 0 ? (
            <div className="space-y-1">
              <div className="text-xs font-semibold text-[var(--aria-ink-muted)]">
                unplanned evidence
              </div>
              {unplannedEvidence.map((evidence) => (
                <div
                  key={evidence.tool_use_id}
                  className="rounded-md border border-[var(--aria-line)] p-2 text-xs"
                >
                  <div className="font-mono">{evidence.tool_name}</div>
                  <div className="text-[var(--aria-ink-muted)]">{evidence.status}</div>
                </div>
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
      {[...report.commands, ...unplannedCommands].map((command, index) => (
        <div
          key={`${command.command.join(" ")}-${index}`}
          className="rounded-md border border-[var(--aria-line)] p-2"
        >
          <div className="break-words font-mono text-xs">{command.command.join(" ")}</div>
          <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
            {command.status} · exit {command.exit_code ?? "-"} · {command.duration_ms}ms
          </div>
        </div>
      ))}
    </div>
  );
}

function AnalystDecisionStatus({
  decision,
  waiting,
}: {
  decision: AnalystDecisionRecord | null;
  waiting: boolean;
}) {
  if (!decision) {
    return waiting ? (
      <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
        等待 Analyst 决策
      </div>
    ) : null;
  }

  return (
    <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs">
      <div className="font-semibold text-[var(--aria-ink)]">Analyst 已决策</div>
      <div className="mt-1 font-mono text-[var(--aria-ink-muted)]">
        {decision.verdict} {"->"} {decision.next_stage}
      </div>
      <div className="mt-1 break-words text-[var(--aria-ink)]">{decision.reason}</div>
      <TestingList label="analyst evidence" values={decision.evidence_refs} />
      {decision.raw_provider_output_refs.length > 0 ? (
        <TestingList label="analyst raw" values={decision.raw_provider_output_refs} />
      ) : null}
    </div>
  );
}

function TestingStepResultRow({ step }: { step: TestingStepResult }) {
  return (
    <div className="rounded-md border border-[var(--aria-line)] p-2">
      <div className="flex min-w-0 flex-wrap items-center gap-2">
        <StatusBadge value={step.status} />
        <span className="min-w-0 break-words font-mono text-xs">{step.step_id}</span>
      </div>
      {step.command?.length ? (
        <div className="mt-1 break-words font-mono text-xs text-[var(--aria-ink-muted)]">
          {step.command.join(" ")}
        </div>
      ) : null}
      {step.evidence_refs?.length ? (
        <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
          evidence: {step.evidence_refs.join(", ")}
        </div>
      ) : null}
      {step.provider_analysis ? (
        <div className="mt-1 break-words text-xs">{step.provider_analysis}</div>
      ) : null}
    </div>
  );
}

function TestingList({ label, values }: { label: string; values: string[] }) {
  if (values.length === 0) {
    return null;
  }
  return (
    <div className="space-y-1 text-xs">
      {values.map((value) => (
        <div key={`${label}:${value}`} className="break-words">
          {label}: {value}
        </div>
      ))}
    </div>
  );
}

function EvidencePath({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid min-w-0 grid-cols-[5rem_minmax(0,1fr)] gap-2 text-xs text-[var(--aria-ink-muted)]">
      <span className="font-semibold">{label}</span>
      <span className="min-w-0 break-words font-mono">{value}</span>
    </div>
  );
}

function ReviewPanel() {
  const codeReviews = useCodingWorkspaceStore((state) => state.codeReviewReports);
  const internalReview = useCodingWorkspaceStore((state) => state.internalPrReview);
  if (codeReviews.length === 0 && !internalReview) {
    return <div className="text-[var(--aria-ink-muted)]">暂无审查报告</div>;
  }
  return (
    <div className="space-y-3">
      {codeReviews.map((report) => (
        <ReviewReportCard
          key={report.id}
          title={`Code Review #${report.round}`}
          report={report}
        />
      ))}
      {internalReview ? (
        <ReviewReportCard title="Internal PR Review" report={internalReview} />
      ) : null}
    </div>
  );
}

function ReviewReportCard({
  title,
  report,
}: {
  title: string;
  report: CodeReviewReport | InternalPrReview;
}) {
  return (
    <div className="rounded-md border border-[var(--aria-line)] p-2">
      <div className="flex items-center justify-between gap-2">
        <div className="text-xs font-semibold">{title}</div>
        <StatusBadge value={report.verdict} />
      </div>
      <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">{report.summary}</div>
      {isInternalPrReview(report) ? (
        <InternalPrReviewDetails review={report} />
      ) : null}
      {report.findings.length > 0 ? (
        <div className="mt-2 space-y-2">
          {report.findings.map((finding, index) => (
            <ReviewFindingItem key={`${finding.file_path ?? "global"}-${finding.line ?? index}-${index}`} finding={finding} />
          ))}
        </div>
      ) : null}
    </div>
  );
}

function InternalPrReviewDetails({ review }: { review: InternalPrReview }) {
  return (
    <div className="mt-2 space-y-2 text-xs">
      {review.impact_scope.length > 0 ? (
        <div>
          <div className="font-semibold text-[var(--aria-ink)]">影响范围</div>
          <div className="mt-1 flex flex-wrap gap-1">
            {review.impact_scope.map((scope) => (
              <span
                key={scope}
                className="rounded border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-1.5 py-0.5 font-mono text-[var(--aria-ink-muted)]"
              >
                {scope}
              </span>
            ))}
          </div>
        </div>
      ) : null}
      {review.pr_description ? (
        <div>
          <div className="font-semibold text-[var(--aria-ink)]">PR description</div>
          <div className="mt-1 whitespace-pre-wrap text-[var(--aria-ink-muted)]">
            {review.pr_description}
          </div>
        </div>
      ) : null}
      {review.commit_message_suggestion ? (
        <div>
          <div className="font-semibold text-[var(--aria-ink)]">commit message</div>
          <div className="mt-1 font-mono text-[var(--aria-ink-muted)]">
            {review.commit_message_suggestion}
          </div>
        </div>
      ) : null}
    </div>
  );
}

function ReviewFindingItem({ finding }: { finding: ReviewFinding }) {
  return (
    <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <span className={["rounded border px-1.5 py-0.5 font-semibold", severityClass(finding.severity)].join(" ")}>
          {finding.severity}
        </span>
        <span className="truncate font-mono text-[var(--aria-ink-muted)]">
          {findingLocation(finding)}
        </span>
      </div>
      <div className="mt-1 text-[var(--aria-ink)]">{finding.message}</div>
      {finding.required_action ? (
        <div className="mt-1 text-[var(--aria-ink-muted)]">{finding.required_action}</div>
      ) : null}
    </div>
  );
}

function isInternalPrReview(report: CodeReviewReport | InternalPrReview): report is InternalPrReview {
  return "review_request_id" in report;
}

function severityClass(severity: ReviewFinding["severity"]) {
  switch (severity) {
    case "error":
      return "border-red-200 bg-red-50 text-red-700";
    case "warning":
      return "border-amber-200 bg-amber-50 text-amber-700";
    case "info":
      return "border-sky-200 bg-sky-50 text-sky-700";
  }
}

function findingLocation(finding: ReviewFinding) {
  if (!finding.file_path) return "global";
  return finding.line ? `${finding.file_path}:${finding.line}` : finding.file_path;
}

function GitPanel() {
  const store = useCodingWorkspaceStore();
  const request = store.reviewRequest;
  return (
    <div className="space-y-3 text-xs">
      <dl className="space-y-2">
        <InfoRow label="base" value={store.baseBranch ?? request?.base_branch ?? "-"} />
        <InfoRow label="branch" value={store.branchName ?? request?.branch_name ?? "-"} />
        <InfoRow label="commit" value={store.headCommit ?? request?.commit_sha ?? "-"} />
        <InfoRow label="remote" value={store.pushedRemote ?? request?.remote ?? "-"} />
        <InfoRow label="push" value={request?.push_status ?? "-"} />
        <InfoRow label="request" value={request?.id ?? "-"} />
      </dl>
      {request?.external_url ? (
        <a
          href={request.external_url}
          target="_blank"
          rel="noreferrer"
          className="block truncate font-mono text-[var(--aria-primary)] hover:underline"
        >
          {request.external_url}
        </a>
      ) : null}
      {request?.manual_instructions.length ? (
        <ol className="space-y-1 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2">
          {request.manual_instructions.map((instruction, index) => (
            <li key={`${instruction}-${index}`} className="text-[var(--aria-ink-muted)]">
              {instruction}
            </li>
          ))}
        </ol>
      ) : null}
    </div>
  );
}

function LogsPanel() {
  const logs = useCodingWorkspaceStore((state) => state.logs);
  if (logs.length === 0) return <div className="text-[var(--aria-ink-muted)]">暂无日志</div>;
  return (
    <div className="space-y-2">
      {logs.map((log) => (
        <div key={log.id} className="text-xs">
          {log.timestamp} · {log.message}
        </div>
      ))}
    </div>
  );
}

function PrepareExecutionPlanPanel({
  attemptId,
  plan,
  requireConfirm,
}: {
  attemptId: string;
  plan: WorkItemExecutionPlan;
  requireConfirm: boolean;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [changeNote, setChangeNote] = useState("");
  const showActions = requireConfirm && plan.status !== "confirmed";

  async function handleConfirm() {
    setBusy(true);
    setError(null);
    try {
      const updated = await confirmWorkItemExecutionPlan(attemptId);
      useCodingWorkspaceStore.setState({ workItemExecutionPlan: updated });
    } catch (reason) {
      setError(errorMessage(reason, "确认执行计划失败"));
    } finally {
      setBusy(false);
    }
  }

  async function handleRequestChange() {
    const note = changeNote.trim();
    if (!note) {
      setError("请填写修改说明");
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const updated = await requestWorkItemExecutionPlanChange(attemptId, { note });
      useCodingWorkspaceStore.setState({ workItemExecutionPlan: updated });
      setChangeNote("");
    } catch (reason) {
      setError(errorMessage(reason, "请求修改执行计划失败"));
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2">
      <div className="flex items-center justify-between">
        <h3 className="text-xs font-semibold uppercase text-[var(--aria-ink)]">执行计划</h3>
        <span className="text-xs text-[var(--aria-ink-muted)]">{plan.status}</span>
      </div>
      <div className="mt-1 text-sm font-semibold text-[var(--aria-ink)]">{plan.goal}</div>
      <dl className="mt-2 grid gap-1 text-xs">
        <InfoRow label="允许写入" value={plan.allowed_write_scopes.join(", ")} />
        {plan.forbidden_write_scopes.length > 0 ? (
          <InfoRow label="禁止写入" value={plan.forbidden_write_scopes.join(", ")} />
        ) : null}
        {plan.dependency_handoffs.length > 0 ? (
          <InfoRow
            label="依赖交接"
            value={plan.dependency_handoffs.map((handoff) => handoff.work_item_id).join(", ")}
          />
        ) : null}
        <InfoRow label="验证计划" value={plan.verification_plan_ref} />
        <InfoRow label="验证摘要" value={plan.verification_summary} />
        {plan.risk_notes.length > 0 ? (
          <InfoRow label="风险说明" value={plan.risk_notes.join("; ")} />
        ) : null}
      </dl>
      {showActions ? (
        <div className="mt-3 grid gap-2">
          <textarea
            aria-label="修改说明"
            value={changeNote}
            onChange={(event) => {
              setChangeNote(event.target.value);
              setError(null);
            }}
            rows={2}
            placeholder="请求修改的原因或补充说明"
            className="min-h-14 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-xs text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)]"
          />
          <div className="flex flex-wrap gap-2">
            <button
              type="button"
              disabled={busy}
              onClick={() => void handleConfirm()}
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
            >
              <Check className="h-3.5 w-3.5" />
              确认执行计划
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => void handleRequestChange()}
              className="inline-flex h-8 items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
            >
              请求修改
            </button>
          </div>
          {error ? (
            <div className="text-xs font-semibold text-[var(--aria-danger)]">{error}</div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}

function InfoRow({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[4rem_minmax(0,1fr)] gap-2">
      <dt className="text-[var(--aria-ink-muted)]">{label}</dt>
      <dd className="truncate font-mono">{value}</dd>
    </div>
  );
}

function StatusBadge({ value }: { value: string }) {
  return (
    <span className="inline-flex h-6 items-center rounded bg-[var(--aria-panel-subtle)] px-2 text-xs font-semibold text-[var(--aria-ink-muted)]">
      {value}
    </span>
  );
}
