import { Check, Play, RefreshCcw, Send, X } from "lucide-react";
import { useState, type FormEvent } from "react";
import { useWorkspaceStore } from "../../state/workspace-ws-store";
import type { ChatEntry, ChatEntryType } from "../../state/chat-entries";

interface ChatInputBarProps {
  stage: string;
  onSendContextNote: (content: string) => void;
  onStartGeneration: () => void;
  onSendHumanDecision: (content: string) => void;
  onAuthorDecision?: (decision: "accept" | "reject") => void;
  onAbort: () => void;
  disabled?: boolean;
}

const BUSY_STAGES = new Set(["running", "cross_review", "revision"]);

export function ChatInputBar({
  stage,
  onSendContextNote,
  onStartGeneration,
  onSendHumanDecision,
  onAuthorDecision = () => undefined,
  onAbort,
  disabled = false,
}: ChatInputBarProps) {
  const [input, setInput] = useState("");
  const trimmedInput = input.trim();
  const isPrepareContext = stage === "prepare_context";
  const isAuthorConfirm = stage === "author_confirm";
  const isHumanConfirm = stage === "human_confirm";
  const isBusy = BUSY_STAGES.has(stage);
  const inputDisabled = disabled || isBusy || isAuthorConfirm || stage === "completed";
  const canSend = !inputDisabled && (isPrepareContext || isHumanConfirm) && trimmedInput.length > 0;
  const showSend = isPrepareContext || isHumanConfirm;

  function handleSubmit(event: FormEvent) {
    event.preventDefault();
    if (!canSend) {
      return;
    }

    if (isHumanConfirm) {
      useWorkspaceStore.getState().resolveGateEntry("request-change");
      appendOptimisticEntry("human_decision", trimmedInput);
      onSendHumanDecision(trimmedInput);
    } else {
      appendOptimisticEntry("context_note", trimmedInput);
      onSendContextNote(trimmedInput);
    }
    setInput("");
  }

  function handleStartGeneration() {
    if (disabled) {
      return;
    }
    appendOptimisticEntry("start_generation", "开始生成");
    onStartGeneration();
  }

  return (
    <form
      data-testid="chat-input-bar"
      onSubmit={handleSubmit}
      className="border-t border-[var(--aria-line)] bg-[var(--aria-panel)] p-3"
    >
      <div className="flex min-w-0 flex-col gap-2">
        <textarea
          data-testid="context-note-input"
          value={input}
          onChange={(event) => setInput(event.target.value)}
          disabled={inputDisabled}
          rows={3}
          placeholder={placeholderForStage(stage)}
          className="min-h-20 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm text-[var(--aria-ink)] placeholder:text-[var(--aria-ink-muted)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
        />
        <div className="flex flex-wrap justify-end gap-2">
          {isBusy ? (
            <button
              type="button"
              onClick={onAbort}
              disabled={disabled}
              className="inline-flex h-9 items-center gap-2 rounded-md border border-red-200 bg-red-50 px-3 text-sm font-semibold text-red-700 hover:bg-red-100 disabled:opacity-50"
            >
              <X className="h-4 w-4" />
              中止
            </button>
          ) : null}
          {showSend ? (
            <button
              data-testid={isHumanConfirm ? "send-human-decision" : "send-context-note"}
              type="submit"
              disabled={!canSend}
              className="inline-flex h-9 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-sm font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
            >
              <Send className="h-4 w-4" />
              {isHumanConfirm ? "发送修改意见" : "发送"}
            </button>
          ) : null}
          {isAuthorConfirm ? (
            <>
              <button
                type="button"
                onClick={() => onAuthorDecision("reject")}
                disabled={disabled}
                className="inline-flex h-9 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-sm font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
              >
                <RefreshCcw className="h-4 w-4" />
                重新编写
              </button>
              <button
                type="button"
                onClick={() => onAuthorDecision("accept")}
                disabled={disabled}
                className="inline-flex h-9 items-center gap-2 rounded-md bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white disabled:opacity-50"
              >
                <Check className="h-4 w-4" />
                进入 Review
              </button>
            </>
          ) : null}
          {isPrepareContext ? (
            <button
              data-testid="start-generation"
              type="button"
              onClick={handleStartGeneration}
              disabled={disabled}
              className="inline-flex h-9 items-center gap-2 rounded-md bg-[var(--aria-primary)] px-3 text-sm font-semibold text-white disabled:opacity-50"
            >
              <Play className="h-4 w-4" />
              开始生成
            </button>
          ) : null}
        </div>
      </div>
    </form>
  );
}

function placeholderForStage(stage: string) {
  if (stage === "human_confirm") {
    return "输入修改意见...";
  }
  if (stage === "author_confirm") {
    return "等待确认 Author 结果";
  }
  if (BUSY_STAGES.has(stage)) {
    return "Provider 运行中，暂不可输入";
  }
  if (stage === "completed") {
    return "流程已完成";
  }
  return "补充上下文";
}

function appendOptimisticEntry(type: ChatEntryType, content: string) {
  const entry: ChatEntry = {
    id: `${type}:optimistic:${Date.now()}`,
    type,
    role: type === "start_generation" ? "system" : "user",
    content,
    timestamp: new Date().toISOString(),
  };
  useWorkspaceStore.getState().appendChatEntry(entry);
}
