import {
  ArrowLeft,
  Check,
  Circle,
  Code,
  FileText,
  FlaskConical,
  GitBranch,
  GitPullRequest,
  Play,
  RefreshCw,
  SearchCode,
  Send,
  ShieldCheck,
  UserCheck,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useState, type FormEvent } from "react";
import type {
  CodeReviewReport,
  CodingExecutionStage,
  CodingGateRequired,
  CodingTimelineNode,
  InternalPrReview,
  ReviewFinding,
} from "../api/types";
import { ChatEntryList } from "../components/chat-workspace/ChatEntryList";
import { useCodingWorkspaceWs } from "../hooks/useCodingWorkspaceWs";
import { useUnloadGuard } from "../hooks/useUnloadGuard";
import {
  type CodingArtifactTab,
  useCodingWorkspaceStore,
} from "../state/coding-workspace-store";

const ACTIVE_ATTEMPT_STATUSES = new Set(["created", "running", "waiting_for_human", "blocked"]);

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

  useUnloadGuard({
    enabled: store.status === "running",
    message: "Coding attempt 运行中。刷新/关闭可能中断当前操作，是否继续？",
  });

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
          <StatusBadge value={store.status ?? "created"} />
          {connected ? (
            <Wifi aria-label="已连接" className="h-4 w-4 text-[var(--aria-success)]" />
          ) : (
            <WifiOff aria-label="未连接" className="h-4 w-4 text-[var(--aria-danger)]" />
          )}
        </div>
      </div>

      <header className="grid min-h-16 shrink-0 gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-3 md:grid-cols-[minmax(0,1fr)_auto]">
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
        <div className="flex items-center gap-2">
          <ActionButtons api={api} stage={store.stage} status={store.status} />
        </div>
      </header>

      <main className="grid min-h-0 flex-1 grid-cols-1 md:grid-cols-[16rem_minmax(0,1fr)]">
        <CodingTimeline
          nodes={store.timelineNodes}
          activeNodeId={store.activeNodeId}
          selectedNodeId={store.selectedNodeId}
          onSelectNode={(nodeId) => useCodingWorkspaceStore.getState().setSelectedNode(nodeId)}
        />
        <section className="grid min-h-0 grid-rows-[auto_minmax(0,1fr)] bg-[var(--aria-panel)]">
          <CodingPanelTabs activePanel={activePanel} onSelectPanel={setActivePanel} />
          {activePanel === "results" ? (
            <CodingArtifactTabs activeTab={activeTab} className="min-h-0" />
          ) : (
            <div className="grid min-h-0 grid-rows-[minmax(0,1fr)_auto_auto]">
              <ChatEntryList entries={store.chatEntries} />
              <PendingGatePanel
                gate={store.pendingGates.at(-1) ?? null}
                onRespond={api.respondGate}
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
        <span>{store.connectionStatus}</span>
        <span>rework {store.reworkCount}/{store.maxAutoRework}</span>
      </div>
    </div>
  );
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

function PendingGatePanel({
  gate,
  onRespond,
}: {
  gate: CodingGateRequired | null;
  onRespond: ReturnType<typeof useCodingWorkspaceWs>["respondGate"];
}) {
  if (!gate) {
    return null;
  }

  return (
    <div
      data-testid="coding-pending-gate"
      className="border-t border-amber-200 bg-amber-50 px-3 py-2"
    >
      <div className="flex min-w-0 flex-col gap-2 md:flex-row md:items-center md:justify-between">
        <div className="min-w-0">
          <div className="truncate text-sm font-semibold text-amber-900">{gate.title}</div>
          <div className="mt-0.5 line-clamp-2 text-xs text-amber-800">{gate.description}</div>
        </div>
        <div className="flex shrink-0 flex-wrap gap-2">
          {gate.available_actions.map((action) => (
            <button
              key={action.action_id}
              type="button"
              onClick={() => onRespond(gate.gate_id, action.action_id, undefined)}
              className="inline-flex h-8 items-center justify-center rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-900 hover:bg-amber-100"
            >
              {action.label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

function CodingTimeline({
  nodes,
  activeNodeId,
  selectedNodeId,
  onSelectNode,
}: {
  nodes: CodingTimelineNode[];
  activeNodeId: string | null;
  selectedNodeId: string | null;
  onSelectNode: (nodeId: string) => void;
}) {
  return (
    <nav
      aria-label="Coding Timeline"
      data-testid="coding-timeline"
      className="min-h-0 overflow-auto border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3 md:border-b-0 md:border-r"
    >
      {nodes.length === 0 ? (
        <div className="rounded-md border border-[var(--aria-line)] bg-white p-3 text-sm text-[var(--aria-ink-muted)]">
          暂无 Timeline 节点
        </div>
      ) : (
        <div className="space-y-2">
          {nodes.map((node) => {
            const Icon = iconForStage(node.stage);
            const active = node.id === activeNodeId;
            const selected = node.id === selectedNodeId;
            return (
              <button
                key={node.id}
                type="button"
                onClick={() => onSelectNode(node.id)}
                aria-current={active ? "step" : undefined}
                className={[
                  "block w-full rounded-md border bg-white px-3 py-2 text-left transition-colors",
                  active || selected
                    ? "border-[var(--aria-primary)] ring-1 ring-[var(--aria-primary)]"
                    : "border-[var(--aria-line)] hover:border-[var(--aria-primary)]",
                ].join(" ")}
              >
                <div className="flex min-w-0 items-start gap-2">
                  <Icon className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
                  <div className="min-w-0 flex-1">
                    <div className="flex min-w-0 items-center justify-between gap-2">
                      <span className="truncate text-sm font-semibold">{node.title}</span>
                      <span className="rounded bg-[var(--aria-panel-muted)] px-1.5 py-0.5 text-[11px] text-[var(--aria-ink-muted)]">
                        {node.status}
                      </span>
                    </div>
                    {node.summary ? (
                      <p className="mt-1 truncate text-xs text-[var(--aria-ink-muted)]">
                        {node.summary}
                      </p>
                    ) : null}
                  </div>
                </div>
              </button>
            );
          })}
        </div>
      )}
    </nav>
  );
}

function CodingArtifactTabs({
  activeTab,
  className = "",
}: {
  activeTab: CodingArtifactTab;
  className?: string;
}) {
  const store = useCodingWorkspaceStore();
  const tabs: CodingArtifactTab[] = ["diff", "tests", "review", "git", "logs"];

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
          <div className="text-[var(--aria-ink-muted)]">暂无代码变更摘要</div>
        )}
      </div>
    </aside>
  );
}

function TestsPanel() {
  const report = useCodingWorkspaceStore((state) => state.testingReport);
  if (!report) {
    return <div className="text-[var(--aria-ink-muted)]">暂无测试报告</div>;
  }
  return (
    <div className="space-y-3">
      <StatusBadge value={report.overall_status} />
      {report.commands.map((command, index) => (
        <div key={`${command.command.join(" ")}-${index}`} className="rounded-md border border-[var(--aria-line)] p-2">
          <div className="font-mono text-xs">{command.command.join(" ")}</div>
          <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
            {command.status} · exit {command.exit_code ?? "-"} · {command.duration_ms}ms
          </div>
        </div>
      ))}
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

function ReviewFindingItem({ finding }: { finding: ReviewFinding }) {
  return (
    <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-2 text-xs">
      <div className="flex min-w-0 items-center justify-between gap-2">
        <span className="rounded bg-white px-1.5 py-0.5 font-semibold text-[var(--aria-ink)]">
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

function iconForStage(stage: CodingExecutionStage): LucideIcon {
  switch (stage) {
    case "worktree_prepare":
      return GitBranch;
    case "coding":
      return Code;
    case "testing":
      return FlaskConical;
    case "code_review":
      return SearchCode;
    case "rework":
      return RefreshCw;
    case "review_request":
      return GitPullRequest;
    case "internal_pr_review":
      return ShieldCheck;
    case "final_confirm":
      return UserCheck;
    default:
      return Circle;
  }
}
