import { GitBranch, MessageSquareText } from "lucide-react";
import { useId, useState } from "react";
import type { RevisionPath } from "../../../api/types";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function ReviewVerdictEntry({
  entry,
  onSelectPath,
}: {
  entry: ChatEntry;
  onSelectPath?: (path: RevisionPath, extraContext?: string) => void;
}) {
  const verdict = verdictFromEntry(entry);
  const [isContextFormOpen, setIsContextFormOpen] = useState(false);
  const [contextDraft, setContextDraft] = useState("");
  const contextFieldId = useId();
  const trimmedContext = contextDraft.trim();

  return (
    <ChatEntryContainer
      role="reviewer"
      title={verdictLabel(verdict?.verdict ?? null)}
      className="border-amber-200 bg-amber-50"
      testId="review-verdict-entry"
    >
      <div className="space-y-3">
        <div className="flex items-start gap-2">
          <MessageSquareText className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
          <div className="min-w-0">
            <div className="text-sm font-medium text-[var(--aria-ink)]">{entry.content}</div>
            {verdict?.comments ? (
              <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">{verdict.comments}</div>
            ) : null}
          </div>
        </div>
        {onSelectPath ? (
          <div className="flex flex-wrap justify-end gap-2">
            <button
              type="button"
              onClick={() => onSelectPath("revise")}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-800 hover:bg-amber-100"
            >
              <GitBranch className="h-3.5 w-3.5" />
              接受修订建议
            </button>
            <button
              type="button"
              onClick={() => setIsContextFormOpen(true)}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-800 hover:bg-amber-100"
            >
              <GitBranch className="h-3.5 w-3.5" />
              补充上下文后修订
            </button>
            <button
              type="button"
              onClick={() => onSelectPath("skip-to-human")}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-amber-300 bg-white px-3 text-xs font-semibold text-amber-800 hover:bg-amber-100"
            >
              <GitBranch className="h-3.5 w-3.5" />
              跳过，人工处理
            </button>
          </div>
        ) : null}
        {onSelectPath && isContextFormOpen ? (
          <div className="space-y-2 rounded-md border border-amber-200 bg-white p-2">
            <label className="block text-xs font-medium text-amber-900" htmlFor={contextFieldId}>
              补充返修上下文
            </label>
            <textarea
              id={contextFieldId}
              aria-label="补充返修上下文"
              value={contextDraft}
              onChange={(event) => setContextDraft(event.target.value)}
              rows={3}
              className="min-h-20 w-full resize-y rounded-md border border-amber-200 bg-white px-2 py-1.5 text-sm text-[var(--aria-ink)] outline-none focus:border-amber-400"
            />
            <div className="flex flex-wrap justify-end gap-2">
              <button
                type="button"
                onClick={() => {
                  setContextDraft("");
                  setIsContextFormOpen(false);
                }}
                className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)]"
              >
                取消
              </button>
              <button
                type="button"
                disabled={!trimmedContext}
                onClick={() => onSelectPath("revise-with-context", trimmedContext)}
                className="inline-flex h-8 items-center rounded-md border border-amber-400 bg-amber-100 px-3 text-xs font-semibold text-amber-900 hover:bg-amber-200 disabled:cursor-not-allowed disabled:border-amber-200 disabled:bg-amber-50 disabled:text-amber-300"
              >
                提交补充并修订
              </button>
            </div>
          </div>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}

function verdictFromEntry(entry: ChatEntry) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const verdict = typeof metadata?.verdict === "string" ? metadata.verdict : null;
  const comments = typeof metadata?.comments === "string" ? metadata.comments : null;
  const summary = typeof metadata?.summary === "string" ? metadata.summary : null;
  return { verdict, comments, summary };
}

function verdictLabel(verdict: string | null) {
  if (verdict === "pass") {
    return "通过";
  }
  if (verdict === "revise") {
    return "建议返修";
  }
  if (verdict === "needs_human") {
    return "需要人工确认";
  }
  return "审核结论";
}
