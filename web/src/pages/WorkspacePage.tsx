import { ArrowLeft, Check, RotateCcw, Send, Settings, Square, Wifi, WifiOff } from "lucide-react";
import { useEffect, useRef, useState, type FormEvent } from "react";
import { useWorkspaceWs } from "../hooks/useWorkspaceWs";
import { useWorkspaceStore } from "../state/workspace-ws-store";

const STAGE_LABELS: Record<string, string> = {
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

export function WorkspacePage({
  sessionId,
  onBack,
}: {
  sessionId: string;
  onBack: () => void;
}) {
  const { sendMessage, rollback, confirm, abort, selectProvider, connectionStatus } =
    useWorkspaceWs(sessionId);
  const {
    stage,
    messages,
    streamingContent,
    artifact,
    checkpoints,
    error,
    workspaceType,
    providers,
  } = useWorkspaceStore();

  const [draft, setDraft] = useState("");
  const [showProviderPanel, setShowProviderPanel] = useState(false);
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

  return (
    <div className="flex h-screen flex-col bg-[var(--aria-bg)]">
      {/* Top Bar */}
      <header className="flex h-12 shrink-0 items-center justify-between border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4">
        <div className="flex items-center gap-3">
          <button
            type="button"
            onClick={onBack}
            className="inline-flex h-8 items-center gap-1 rounded-md px-2 text-sm text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]"
          >
            <ArrowLeft className="h-4 w-4" />
            返回
          </button>
          <span className="text-sm font-semibold text-[var(--aria-ink)]">
            {workspaceType === "story"
              ? "Story Spec"
              : workspaceType === "design"
                ? "Design Spec"
                : workspaceType === "work_item"
                  ? "Work Item"
                  : "Workspace"}
          </span>
        </div>
        <div className="flex items-center gap-3">
          <span className="rounded-full border border-[var(--aria-line)] px-2.5 py-0.5 text-xs font-medium text-[var(--aria-ink-muted)]">
            {STAGE_LABELS[stage] ?? stage}
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
      <div className="flex min-h-0 flex-1">
        {/* Chat Panel */}
        <div className="flex min-h-0 w-1/2 flex-col border-r border-[var(--aria-line)]">
          {/* Messages */}
          <div ref={scrollRef} className="min-h-0 flex-1 overflow-auto p-4 space-y-3">
            {messages.map((msg, idx) => (
              <div
                key={msg.id || idx}
                className={`flex ${msg.role === "user" ? "justify-end" : "justify-start"}`}
              >
                <div
                  className={`max-w-[80%] rounded-lg px-3 py-2 text-sm ${
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
                <div className="max-w-[80%] rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2 text-sm text-[var(--aria-ink)]">
                  <pre className="whitespace-pre-wrap break-words font-sans">
                    {streamingContent}
                    <span className="animate-pulse">▊</span>
                  </pre>
                </div>
              </div>
            ) : null}
            {error ? (
              <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
                {error}
              </div>
            ) : null}
          </div>

          {/* Input Area */}
          <div className="shrink-0 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] p-3">
            <div className="mb-2 flex gap-2">
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
              {streamingContent ? (
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

        {/* Artifact Panel */}
        <div className="flex min-h-0 w-1/2 flex-col">
          <div className="flex h-10 shrink-0 items-center justify-between border-b border-[var(--aria-line)] bg-[var(--aria-panel)] px-4">
            <span className="text-xs font-semibold uppercase text-[var(--aria-ink-muted)]">
              Artifact
            </span>
            {providers ? (
              <span className="text-xs text-[var(--aria-ink-muted)]">
                Author: {providers.author} | Reviewer: {providers.reviewer ?? "无"}
              </span>
            ) : null}
          </div>
          {showProviderPanel ? (
            <div className="shrink-0 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3 space-y-2">
              <div className="flex items-center gap-2">
                <label className="text-xs font-medium text-[var(--aria-ink-muted)] w-16">Author</label>
                <select
                  value={providers?.author ?? "claude_code"}
                  onChange={(e) => selectProvider("author", e.target.value)}
                  className="rounded-md border border-[var(--aria-line)] bg-white px-2 py-1 text-xs text-[var(--aria-ink)]"
                >
                  {PROVIDER_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                  ))}
                </select>
              </div>
              <div className="flex items-center gap-2">
                <label className="text-xs font-medium text-[var(--aria-ink-muted)] w-16">Reviewer</label>
                <select
                  value={providers?.reviewer ?? "codex"}
                  onChange={(e) => selectProvider("reviewer", e.target.value)}
                  className="rounded-md border border-[var(--aria-line)] bg-white px-2 py-1 text-xs text-[var(--aria-ink)]"
                >
                  {PROVIDER_OPTIONS.map((opt) => (
                    <option key={opt.value} value={opt.value}>{opt.label}</option>
                  ))}
                </select>
              </div>
            </div>
          ) : null}
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
        </div>
      </div>

      {/* Flow Rail (bottom) */}
      <footer className="flex h-10 shrink-0 items-center gap-2 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-4">
        {Object.entries(STAGE_LABELS).map(([key, label]) => (
          <span
            key={key}
            className={`rounded-full px-2.5 py-0.5 text-xs font-medium ${
              key === stage
                ? "bg-[var(--aria-primary)] text-white"
                : "bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]"
            }`}
          >
            {label}
          </span>
        ))}
      </footer>
    </div>
  );
}
