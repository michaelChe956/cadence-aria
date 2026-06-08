import { GitBranch } from "lucide-react";
import { useId, useState } from "react";
import type { RevisionPath } from "../../api/types";

export function ReviewDecisionActions({
  onSelectPath,
}: {
  onSelectPath: (path: RevisionPath, extraContext?: string) => void;
}) {
  const [isContextFormOpen, setIsContextFormOpen] = useState(false);
  const [contextDraft, setContextDraft] = useState("");
  const contextFieldId = useId();
  const trimmedContext = contextDraft.trim();

  return (
    <div className="space-y-2">
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
      {isContextFormOpen ? (
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
  );
}
