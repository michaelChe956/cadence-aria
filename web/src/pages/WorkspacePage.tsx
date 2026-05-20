import {
  Activity,
  ArrowLeft,
  Check,
  FileText,
  RotateCcw,
  Send,
  Settings,
  Square,
  Terminal,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { useWorkspaceStore, type ExecutionEvent } from "../state/workspace-ws-store";

const WORKSPACE_STAGE_LABELS: Record<string, string> = {
  prepare_context: "准备上下文",
  running: "运行中",
  cross_review: "交叉审查",
  review_decision: "等待返修决策",
  revision: "返修中",
  human_confirm: "人工确认",
  completed: "已完成",
};

const FLOW_STAGE_LABELS: Record<string, string> = {
  prepare_context: "准备上下文",
  running: "运行中",
  cross_review: "交叉审查",
  human_confirm: "人工确认",
  completed: "已完成",
};

const PROVIDER_OPTIONS = [
  { value: "claude_code", label: "Claude Code" },
  { value: "codex", label: "Codex" },
  { value: "fake", label: "Fake (测试)" },
];

const PROVIDER_STATUS_LABELS: Record<string, string> = {
  starting: "启动中",
  running: "运行中",
  waiting_approval: "等待权限",
  completed: "已完成",
  failed: "失败",
  aborted: "已中止",
};

const EXECUTION_STATUS_LABELS: Record<string, string> = {
  started: "开始",
  running: "运行中",
  waiting_approval: "等待权限",
  completed: "完成",
  failed: "失败",
  aborted: "中止",
};

const EXECUTION_KIND_LABELS: Record<string, string> = {
  provider: "Provider",
  turn: "Turn",
  command: "Command",
  output: "Output",
  artifact: "Artifact",
};

export function WorkspacePage({
  sessionId,
  onBack,
}: {
  sessionId: string;
  onBack: () => void;
}) {
  const {
    sendMessage,
    startGeneration,
    rollback,
	    confirm,
	    abort,
	    selectProvider,
	    sendReviewDecision,
	    respondPermission,
	    connectionStatus,
	  } = useWorkspaceWs(sessionId);
  const {
    stage,
    messages,
    streamingContent,
    artifact,
    checkpoints,
    error,
    workspaceType,
    providers,
    pendingPermissions,
    providerStatus,
    executionEvents,
    visitedStages,
	    timelineNodes,
	    selectedNodeId,
	    nodeDetails,
	    pendingDecision,
	    setSelectedNode,
	  } = useWorkspaceStore();

	  const [draft, setDraft] = useState("");
	  const [reviewContextDraft, setReviewContextDraft] = useState("");
	  const [showReviewContext, setShowReviewContext] = useState(false);
	  const [showProviderPanel, setShowProviderPanel] = useState(false);
	  const [activeRightTab, setActiveRightTab] = useState<"artifact" | "execution">("execution");
  const scrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (scrollRef.current) {
      scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
    }
  }, [messages.length, streamingContent]);

  function handleSubmit(e: FormEvent) {
    e.preventDefault();
    const content = draft.trim();
    if (!content) return;
    sendMessage(content);
    setDraft("");
  }

  const isConnected = connectionStatus === "connected";
	  const isCompleted = stage === "completed";
	  const canStartGeneration = stage === "prepare_context" && isConnected && !streamingContent;
	  const providerLocked = stage !== "prepare_context";
	  const currentFlowStage = flowStageFor(stage);
	  const canAbort = ["running", "cross_review", "revision"].includes(stage) || Boolean(streamingContent);
	  const selectedNode =
	    timelineNodes.find((node) => node.node_id === selectedNodeId) ?? timelineNodes.at(-1) ?? null;
	  const selectedDetail = selectedNode ? nodeDetails[selectedNode.node_id] : null;

  return (
    <div className="flex h-screen min-w-0 flex-col overflow-hidden bg-[var(--aria-bg)]">
      {/* Top Bar */}
      <header className="flex h-12 min-w-0 shrink-0 items-center justify-between gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 sm:px-4">
        <div className="flex min-w-0 items-center gap-2 sm:gap-3">
          <button
            type="button"
            onClick={onBack}
            className="inline-flex h-8 shrink-0 items-center gap-1 rounded-md px-2 text-sm text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]"
          >
            <ArrowLeft className="h-4 w-4" />
            <span className="hidden sm:inline">返回</span>
          </button>
          <span className="truncate text-sm font-semibold text-[var(--aria-ink)]">
            {workspaceType === "story"
              ? "Story Spec"
              : workspaceType === "design"
                ? "Design Spec"
                : workspaceType === "work_item"
                  ? "Work Item"
                  : "Workspace"}
          </span>
        </div>
        <div className="flex min-w-0 shrink-0 items-center gap-2 sm:gap-3">
	          <span className="max-w-24 truncate rounded-full border border-[var(--aria-line)] px-2 py-0.5 text-xs font-medium text-[var(--aria-ink-muted)] sm:max-w-none sm:px-2.5">
	            {WORKSPACE_STAGE_LABELS[stage] ?? stage}
	          </span>
          <button
            type="button"
            onClick={() => setShowProviderPanel((v) => !v)}
            className="inline-flex h-7 w-7 items-center justify-center rounded-md text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]"
            title="Provider 配置"
          >
            <Settings className="h-4 w-4" />
          </button>
          {isConnected ? (
            <Wifi className="h-4 w-4 text-green-500" />
          ) : (
            <WifiOff className="h-4 w-4 text-red-400" />
          )}
        </div>
      </header>

      {/* Main Content: Chat + Artifact */}
      <div className="flex min-h-0 min-w-0 flex-1 flex-col lg:flex-row">
        {/* Chat Panel */}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col border-b border-[var(--aria-line)] lg:w-1/2 lg:border-b-0 lg:border-r">
          {/* Timeline / Messages */}
          <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto p-4 space-y-3">
            {timelineNodes.length > 0 ? (
              <div className="space-y-2">
                {timelineNodes.map((node) => (
                  <button
                    key={node.node_id}
                    type="button"
                    onClick={() => setSelectedNode(node.node_id)}
                    className={`block w-full rounded-md border px-3 py-2 text-left ${
                      node.node_id === selectedNode?.node_id
                        ? "border-[var(--aria-primary)] bg-blue-50"
                        : "border-[var(--aria-line)] bg-white hover:bg-[var(--aria-panel-muted)]"
                    }`}
                  >
                    <div className="flex min-w-0 items-center justify-between gap-2">
                      <span className="truncate text-sm font-semibold text-[var(--aria-ink)]">
                        {node.title}
                      </span>
                      <span className={`shrink-0 rounded px-1.5 py-0.5 text-[11px] font-medium ${timelineStatusClass(node.status)}`}>
                        {timelineStatusLabel(node.status)}
                      </span>
                    </div>
                    <div className="mt-1 flex min-w-0 items-center gap-2">
                      <span className={`rounded px-1.5 py-0.5 text-[11px] font-semibold ${agentBadgeClass(node.agent)}`}>
                        {providerLabel(node.agent ?? "system")}
                      </span>
                      {node.summary ? (
                        <span className="truncate text-xs text-[var(--aria-ink-muted)]">
                          {node.summary}
                        </span>
                      ) : null}
                    </div>
                  </button>
                ))}
              </div>
            ) : messages.map((msg, idx) => (
              <div
                key={msg.id || idx}
                className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}
              >
                <div
                  className={`max-w-[min(100%,44rem)] rounded-lg px-3 py-2 text-sm ${
                    msg.role === "user"
                      ? "bg-[var(--aria-primary)] text-white"
                      : "border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink)]"
                  }`}
                >
                  <pre className="whitespace-pre-wrap break-words font-sans">{msg.content}</pre>
                  {msg.checkpoint_id ? (
                    <button
                      type="button"
                      onClick={() => rollback(msg.checkpoint_id!)}
                      className="mt-1 inline-flex items-center gap-1 text-xs opacity-60 hover:opacity-100"
                      title="回退到此消息"
                    >
                      <RotateCcw className="h-3 w-3" />
                      回退
                    </button>
                  ) : null}
                </div>
              </div>
            ))}
            {streamingContent ? (
              <div className="flex justify-start">
                <div className="max-w-[min(100%,44rem)] rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-sm text-[var(--aria-ink)]">
                  <pre className="whitespace-pre-wrap break-words font-sans">
                    {streamingContent}
                    <span className="animate-pulse">▊</span>
                  </pre>
                </div>
              </div>
            ) : null}
            {pendingPermissions.map((permission) => (
              <div
                key={permission.id}
                className="rounded-md border border-amber-300 bg-amber-50 p-3 text-sm"
              >
                <div className="font-semibold text-amber-900">{permission.tool_name}</div>
                <div className="mt-1 text-amber-800">{permission.description}</div>
                <div className="mt-2 flex items-center gap-2">
                  <span className="rounded border border-amber-300 px-2 py-0.5 text-xs text-amber-800">
                    {permission.risk_level}
                  </span>
                  <button
                    type="button"
                    onClick={() => respondPermission(permission.id, false, undefined)}
                    className="inline-flex h-7 items-center gap-1 rounded-md border border-red-300 bg-red-50 px-2 text-xs font-semibold text-red-700 hover:bg-red-100"
                  >
                    <X className="h-3.5 w-3.5" />
                    拒绝
                  </button>
                  <button
                    type="button"
                    onClick={() => respondPermission(permission.id, true, undefined)}
                    className="inline-flex h-7 items-center gap-1 rounded-md border border-green-500 bg-green-50 px-2 text-xs font-semibold text-green-700 hover:bg-green-100"
                  >
                    <Check className="h-3.5 w-3.5" />
                    允许
                  </button>
                </div>
              </div>
            ))}
            {error ? (
              <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
                {error}
              </div>
            ) : null}
          </div>

          {/* Input Area */}
	          <div className="shrink-0 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] p-3">
	            <div className="mb-2 flex flex-wrap gap-2">
	              {canStartGeneration ? (
	                <button
                  type="button"
                  onClick={startGeneration}
                  className="inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:opacity-50"
                >
                  <Send className="h-3.5 w-3.5" />
                  开始生成
                </button>
              ) : null}
              {stage === "human_confirm" ? (
                <button
                  type="button"
                  onClick={confirm}
                  disabled={!isConnected}
                  className="inline-flex h-8 items-center gap-1 rounded-md border border-green-500 bg-green-50 px-3 text-xs font-semibold text-green-700 hover:bg-green-100 disabled:opacity-50"
                >
                  <Check className="h-3.5 w-3.5" />
	                  确认通过
	                </button>
	              ) : null}
	              {stage === "review_decision" && pendingDecision ? (
	                <>
	                  <button
	                    type="button"
	                    onClick={() => sendReviewDecision("continue", undefined)}
	                    disabled={!isConnected}
	                    className="inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:opacity-50"
	                  >
	                    <RotateCcw className="h-3.5 w-3.5" />
	                    直接返修
	                  </button>
	                  <button
	                    type="button"
	                    onClick={() => setShowReviewContext((value) => !value)}
	                    disabled={!isConnected}
	                    className="inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
	                  >
	                    <FileText className="h-3.5 w-3.5" />
	                    补充信息后返修
	                  </button>
	                  <button
	                    type="button"
	                    onClick={() => sendReviewDecision("human_intervene", undefined)}
	                    disabled={!isConnected}
	                    className="inline-flex h-8 items-center gap-1 rounded-md border border-amber-300 bg-amber-50 px-3 text-xs font-semibold text-amber-700 hover:bg-amber-100 disabled:opacity-50"
	                  >
	                    <Check className="h-3.5 w-3.5" />
	                    人工介入
	                  </button>
	                </>
	              ) : null}
	              {canAbort ? (
	                <button
	                  type="button"
                  onClick={abort}
                  className="inline-flex h-8 items-center gap-1 rounded-md border border-red-300 bg-red-50 px-3 text-xs font-semibold text-red-600 hover:bg-red-100"
                >
                  <Square className="h-3.5 w-3.5" />
	                  中止
	                </button>
	              ) : null}
	            </div>
	            {stage === "review_decision" && pendingDecision && showReviewContext ? (
	              <div className="mb-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2">
	                <label
	                  htmlFor="workspace-review-context"
	                  className="mb-1 block text-xs font-medium text-[var(--aria-ink-muted)]"
	                >
	                  返修补充信息
	                </label>
	                <textarea
	                  id="workspace-review-context"
	                  value={reviewContextDraft}
	                  onChange={(event) => setReviewContextDraft(event.target.value)}
	                  className="min-h-20 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 text-sm text-[var(--aria-ink)]"
	                />
	                <div className="mt-2 flex justify-end">
	                  <button
	                    type="button"
	                    onClick={() => sendReviewDecision("continue_with_context", reviewContextDraft)}
	                    disabled={!isConnected || !reviewContextDraft.trim()}
	                    className="inline-flex h-8 items-center gap-1 rounded-md bg-[var(--aria-primary)] px-3 text-xs font-semibold text-white disabled:opacity-50"
	                  >
	                    <Send className="h-3.5 w-3.5" />
	                    提交返修
	                  </button>
	                </div>
	              </div>
	            ) : null}
	            <form onSubmit={handleSubmit} className="flex gap-2">
              <input
                type="text"
                value={draft}
                onChange={(e) => setDraft(e.target.value)}
                placeholder={isCompleted ? "会话已完成" : "输入消息..."}
                disabled={!isConnected || isCompleted}
                className="min-w-0 flex-1 rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)] disabled:opacity-50"
              />
              <button
                type="submit"
                disabled={!isConnected || !draft.trim() || isCompleted}
                className="inline-flex h-9 items-center gap-1 rounded-md bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white disabled:opacity-50"
              >
                <Send className="h-4 w-4" />
              </button>
            </form>
          </div>
        </div>

        {/* Artifact / Execution Panel */}
        <div className="flex min-h-0 min-w-0 flex-1 flex-col lg:w-1/2">
          <div className="flex h-10 min-w-0 shrink-0 items-center justify-between gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 sm:px-4">
            <div className="inline-flex shrink-0 items-center gap-1 rounded-md border border-[var(--aria-line)] bg-white p-0.5">
              <button
                type="button"
                onClick={() => setActiveRightTab("artifact")}
                aria-pressed={activeRightTab === "artifact"}
                className={`inline-flex h-7 items-center gap-1 rounded px-2 text-xs font-semibold ${
                  activeRightTab === "artifact"
                    ? "bg-[var(--aria-panel-muted)] text-[var(--aria-ink)]"
                    : "text-[var(--aria-ink-muted)] hover:text-[var(--aria-ink)]"
                }`}
              >
                <FileText className="h-3.5 w-3.5" />
                Artifact
              </button>
              <button
                type="button"
                onClick={() => setActiveRightTab("execution")}
                aria-pressed={activeRightTab === "execution"}
                className={`inline-flex h-7 items-center gap-1 rounded px-2 text-xs font-semibold ${
                  activeRightTab === "execution"
                    ? "bg-[var(--aria-panel-muted)] text-[var(--aria-ink)]"
                    : "text-[var(--aria-ink-muted)] hover:text-[var(--aria-ink)]"
                }`}
              >
                <Terminal className="h-3.5 w-3.5" />
                执行
              </button>
            </div>
            {providers ? (
              <span className="min-w-0 truncate text-xs text-[var(--aria-ink-muted)]">
                Author: {providers.author} | Reviewer: {providers.reviewer ?? "无"}
              </span>
            ) : null}
          </div>
          {showProviderPanel ? (
            <div className="shrink-0 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3 space-y-2">
              <div className="flex items-center gap-2">
                <label htmlFor="workspace-author-provider" className="text-xs font-medium text-[var(--aria-ink-muted)] w-16">Author</label>
                <select
                  id="workspace-author-provider"
                  value={providers?.author ?? "claude_code"}
                  onChange={(e) => selectProvider("author", e.target.value)}
                  disabled={providerLocked}
                  className="rounded-md border border-[var(--aria-line)] bg-white px-2 py-1 text-xs text-[var(--aria-ink)]"
                >
                  {PROVIDER_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                  ))}
                </select>
              </div>
              <div className="flex items-center gap-2">
                <label htmlFor="workspace-reviewer-provider" className="text-xs font-medium text-[var(--aria-ink-muted)] w-16">Reviewer</label>
                <select
                  id="workspace-reviewer-provider"
                  value={providers?.reviewer ?? "codex"}
                  onChange={(e) => selectProvider("reviewer", e.target.value)}
                  disabled={providerLocked}
                  className="rounded-md border border-[var(--aria-line)] bg-white px-2 py-1 text-xs text-[var(--aria-ink)]"
                >
                  {PROVIDER_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                  ))}
                </select>
              </div>
            </div>
          ) : null}
          {selectedNode ? (
            <TimelineDetailPanel
              node={selectedNode}
              detail={selectedDetail}
              artifact={artifact}
              providerStatus={providerStatus}
              pendingPermissions={pendingPermissions}
            />
          ) : activeRightTab === "artifact" ? (
            <div className="min-h-0 flex-1 overflow-auto p-4">
              {artifact ? (
                <pre className="whitespace-pre-wrap break-words font-mono text-sm text-[var(--aria-ink)]">
                  {artifact}
                </pre>
              ) : (
                <p className="text-sm text-[var(--aria-ink-muted)]">
                  等待生成...
                </p>
              )}
            </div>
          ) : (
            <ExecutionPanel
              providerStatus={providerStatus}
              executionEvents={executionEvents}
              pendingPermissions={pendingPermissions}
            />
          )}
        </div>
      </div>

	      {/* Flow Rail (bottom) */}
	      <footer className="flex h-10 min-w-0 shrink-0 items-center gap-2 overflow-x-auto border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 sm:px-4">
	        {Object.entries(FLOW_STAGE_LABELS).map(([key, label]) => {
	          const isCurrent = key === currentFlowStage;
          const visited = visitedStages.includes(key);
          return (
            <span
              key={key}
              aria-label={`${label} ${isCurrent ? "当前阶段" : visited ? "已经过" : "未开始"}`}
              className={`shrink-0 rounded-full px-2.5 py-0.5 text-xs font-medium ${
                isCurrent
                  ? "bg-[var(--aria-primary)] text-white"
                  : visited
                    ? "bg-green-50 text-green-700"
                    : "bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]"
              }`}
            >
              {label}
            </span>
          );
        })}
      </footer>
    </div>
  );
}

function flowStageFor(stage: string) {
  if (stage === "review_decision" || stage === "revision") return "cross_review";
  return stage;
}

function TimelineDetailPanel({
  node,
  detail,
  artifact,
  providerStatus,
  pendingPermissions,
}: {
  node: {
    node_id: string;
    node_type: string;
    title: string;
    agent?: string | null;
    summary?: string | null;
  };
  detail:
    | {
        messages: Array<{ id: string; content: string; role: string }>;
        streaming_content: string;
        execution_events: ExecutionEvent[];
        verdict?: { verdict: string; comments: string; summary: string } | null;
      }
    | undefined
    | null;
  artifact: string | null;
  providerStatus: string;
  pendingPermissions: Array<{
    id: string;
    tool_name: string;
    description: string;
    risk_level: "low" | "medium" | "high";
  }>;
}) {
  const content = detail?.streaming_content || detail?.messages.at(-1)?.content || "";
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <div className="border-b border-[var(--aria-line)] px-4 py-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className={`rounded px-1.5 py-0.5 text-[11px] font-semibold ${agentBadgeClass(node.agent)}`}>
            {providerLabel(node.agent ?? "system")}
          </span>
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">{node.title}</h2>
        </div>
        {node.summary ? (
          <p className="mt-1 text-xs text-[var(--aria-ink-muted)]">{node.summary}</p>
        ) : null}
      </div>
      {content ? (
        <pre className="whitespace-pre-wrap break-words border-b border-[var(--aria-line)] p-4 font-sans text-sm text-[var(--aria-ink)]">
          {content}
        </pre>
      ) : node.node_type === "human_confirm" || node.node_type === "completed" ? (
        <pre className="whitespace-pre-wrap break-words border-b border-[var(--aria-line)] p-4 font-mono text-sm text-[var(--aria-ink)]">
          {artifact ?? "等待生成..."}
        </pre>
      ) : (
        <div className="border-b border-[var(--aria-line)] p-4 text-sm text-[var(--aria-ink-muted)]">
          等待节点输出...
        </div>
      )}
      {detail?.verdict ? (
        <div className="border-b border-[var(--aria-line)] p-4 text-sm">
          <div className="font-semibold text-[var(--aria-ink)]">{detail.verdict.summary}</div>
          <pre className="mt-2 whitespace-pre-wrap break-words text-[var(--aria-ink-muted)]">
            {detail.verdict.comments}
          </pre>
        </div>
      ) : null}
      <ExecutionPanel
        providerStatus={providerStatus}
        executionEvents={detail?.execution_events ?? []}
        pendingPermissions={pendingPermissions}
      />
    </div>
  );
}

function ExecutionPanel({
  providerStatus,
  executionEvents,
  pendingPermissions,
}: {
  providerStatus: string;
  executionEvents: ExecutionEvent[];
  pendingPermissions: Array<{
    id: string;
    tool_name: string;
    description: string;
    risk_level: "low" | "medium" | "high";
  }>;
}) {
  return (
    <div className="min-h-0 flex-1 overflow-auto">
      <div className="flex h-11 items-center justify-between border-b border-[var(--aria-line)] px-4">
        <div className="inline-flex items-center gap-2 text-sm font-medium text-[var(--aria-ink)]">
          <Activity className="h-4 w-4 text-[var(--aria-primary)]" />
          Provider: {PROVIDER_STATUS_LABELS[providerStatus] ?? providerStatus}
        </div>
        {pendingPermissions.length > 0 ? (
          <span className="rounded border border-amber-300 bg-amber-50 px-2 py-0.5 text-xs font-medium text-amber-800">
            等待权限 {pendingPermissions.length}
          </span>
        ) : null}
      </div>
      {executionEvents.length === 0 ? (
        <div className="p-4 text-sm text-[var(--aria-ink-muted)]">
          等待 provider 执行事件...
        </div>
      ) : (
        <div>
          {executionEvents.map((event) => (
            <ExecutionEventRow key={event.event_id} event={event} />
          ))}
        </div>
      )}
    </div>
  );
}

function ExecutionEventRow({ event }: { event: ExecutionEvent }) {
  const isCommand = event.kind === "command";
  return (
    <div className="border-b border-[var(--aria-line)] px-4 py-3">
      <div className="flex items-start gap-3">
        <div
          className={`mt-0.5 flex h-7 w-7 shrink-0 items-center justify-center rounded-md border ${statusClass(event.status)}`}
        >
          {isCommand ? <Terminal className="h-3.5 w-3.5" /> : <Activity className="h-3.5 w-3.5" />}
        </div>
        <div className="min-w-0 flex-1">
          <div className="flex flex-wrap items-center gap-2">
            <span className="text-sm font-semibold text-[var(--aria-ink)]">
              {event.title}
            </span>
            <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5 text-[11px] font-medium text-[var(--aria-ink-muted)]">
              {EXECUTION_KIND_LABELS[event.kind] ?? event.kind}
            </span>
	            <span className={`rounded px-1.5 py-0.5 text-[11px] font-medium ${statusBadgeClass(event.status)}`}>
	              {EXECUTION_STATUS_LABELS[event.status] ?? event.status}
	            </span>
	            {event.agent ? (
	              <span className={`rounded px-1.5 py-0.5 text-[11px] font-semibold ${agentBadgeClass(event.agent)}`}>
	                {providerLabel(event.agent)}
	              </span>
	            ) : null}
	          </div>
          {event.detail ? (
            <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">{event.detail}</div>
          ) : null}
          {event.command ? (
            <pre className="mt-2 whitespace-pre-wrap break-words rounded-md bg-[var(--aria-panel-muted)] px-2 py-1.5 font-mono text-xs text-[var(--aria-ink)]">
              {event.command}
            </pre>
          ) : null}
          {event.cwd ? (
            <div className="mt-1 break-all font-mono text-[11px] text-[var(--aria-ink-muted)]">
              {event.cwd}
            </div>
          ) : null}
          {event.output ? (
            <pre className="mt-2 max-h-44 overflow-auto whitespace-pre-wrap break-words rounded-md border border-[var(--aria-line)] bg-white px-2 py-1.5 font-mono text-xs text-[var(--aria-ink)]">
              stdout: {event.output}
            </pre>
          ) : null}
        </div>
      </div>
    </div>
  );
}

function providerLabel(provider: string) {
  if (provider === "claude_code") return "Claude Code";
  if (provider === "codex") return "Codex";
  if (provider === "fake") return "Fake";
  return "System";
}

function agentBadgeClass(agent?: string | null) {
  if (agent === "claude_code") return "bg-blue-50 text-blue-700";
  if (agent === "codex") return "bg-violet-50 text-violet-700";
  if (agent === "fake") return "bg-slate-100 text-slate-600";
  return "bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]";
}

function timelineStatusLabel(status: string) {
  if (status === "active") return "运行中";
  if (status === "paused") return "等待决策";
  if (status === "completed") return "完成";
  if (status === "failed") return "失败";
  if (status === "skipped") return "跳过";
  return status;
}

function timelineStatusClass(status: string) {
  if (status === "completed") return "bg-green-50 text-green-700";
  if (status === "failed") return "bg-red-50 text-red-700";
  if (status === "paused") return "bg-amber-50 text-amber-700";
  if (status === "skipped") return "bg-slate-100 text-slate-600";
  return "bg-blue-50 text-blue-700";
}

function statusClass(status: string) {
  if (status === "completed") return "border-green-200 bg-green-50 text-green-700";
  if (status === "failed" || status === "aborted") return "border-red-200 bg-red-50 text-red-700";
  if (status === "waiting_approval") return "border-amber-300 bg-amber-50 text-amber-700";
  return "border-blue-200 bg-blue-50 text-blue-700";
}

function statusBadgeClass(status: string) {
  if (status === "completed") return "bg-green-50 text-green-700";
  if (status === "failed" || status === "aborted") return "bg-red-50 text-red-700";
  if (status === "waiting_approval") return "bg-amber-50 text-amber-700";
  return "bg-blue-50 text-blue-700";
}
