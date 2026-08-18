import { ChevronDown, ChevronRight, FileText } from "lucide-react";
import { useState } from "react";
import type { ArtifactDraft } from "../../api/groupChat";
import { MarkdownContent } from "../chat-workspace/entries/ProviderStreamEntry";

interface DraftPreviewProps {
  slotLabel: string;
  draft: ArtifactDraft;
}

/** 折叠展示单个草稿槽的当前 Markdown 版本。 */
export function DraftPreview({ slotLabel, draft }: DraftPreviewProps) {
  const [expanded, setExpanded] = useState(false);

  return (
    <div className="rounded-md border border-[var(--aria-line)] bg-white">
      <button
        type="button"
        aria-expanded={expanded}
        aria-label={`${expanded ? "收起" : "预览"}${slotLabel}草稿`}
        onClick={() => setExpanded((current) => !current)}
        className="flex w-full items-center justify-between gap-2 px-2.5 py-2 text-left text-xs hover:bg-[var(--aria-panel-muted)]"
      >
        <span className="flex min-w-0 items-center gap-1.5">
          <FileText aria-hidden="true" className="h-3.5 w-3.5 shrink-0 text-[var(--aria-primary)]" />
          <span className="truncate font-medium text-[var(--aria-ink)]">{slotLabel}草稿</span>
          <span className="shrink-0 text-[var(--aria-ink-muted)]">v{draft.version}</span>
        </span>
        {expanded ? (
          <ChevronDown aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
        ) : (
          <ChevronRight aria-hidden="true" className="h-3.5 w-3.5 shrink-0" />
        )}
      </button>
      {expanded ? (
        <div className="border-t border-[var(--aria-line)] px-3 py-2 text-sm">
          <MarkdownContent content={draft.markdown} />
        </div>
      ) : null}
    </div>
  );
}
