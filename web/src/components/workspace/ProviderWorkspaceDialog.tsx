import { X } from "lucide-react";
import type { WorkspaceSession } from "../../api/types";
import { WorkspaceArtifactPane } from "./WorkspaceArtifactPane";
import { WorkspaceConversation } from "./WorkspaceConversation";
import { WorkspaceFlowRail } from "./WorkspaceFlowRail";

export function ProviderWorkspaceDialog({
  open,
  title,
  session,
  onClose,
  onMessage,
  onRunNext,
  onConfirm,
  onRequestChange,
}: {
  open: boolean;
  title: string;
  session: WorkspaceSession;
  onClose: () => void;
  onMessage: (content: string) => void | Promise<void>;
  onRunNext: () => void | Promise<void>;
  onConfirm: () => void | Promise<void>;
  onRequestChange: () => void | Promise<void>;
}) {
  if (!open) {
    return null;
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-slate-950/35 px-4 py-6">
      <section
        role="dialog"
        aria-modal="true"
        aria-label={title}
        className="grid max-h-[90vh] w-full max-w-6xl grid-rows-[auto_minmax(0,1fr)] overflow-hidden rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-xl"
      >
        <header className="flex flex-wrap items-center justify-between gap-3 border-b border-[var(--aria-line)] px-4 py-3">
          <div className="min-w-0">
            <h2 className="truncate text-base font-semibold text-[var(--aria-ink)]">{title}</h2>
            <div className="mt-1 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
              <span>{session.workspace_session_id}</span>
              <span>review {session.review_rounds}</span>
            </div>
          </div>
          <button
            type="button"
            aria-label="关闭"
            onClick={onClose}
            className="inline-flex h-8 w-8 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)]"
          >
            <X className="h-4 w-4" />
          </button>
        </header>
        <div className="grid min-h-0 overflow-hidden lg:grid-cols-[15rem_minmax(0,1fr)_20rem]">
          <WorkspaceFlowRail workspaceType={session.workspace_type} status={session.status} />
          <WorkspaceConversation
            messages={session.messages}
            onMessage={onMessage}
            onRunNext={onRunNext}
            onConfirm={onConfirm}
            onRequestChange={onRequestChange}
          />
          <WorkspaceArtifactPane session={session} />
        </div>
      </section>
    </div>
  );
}
