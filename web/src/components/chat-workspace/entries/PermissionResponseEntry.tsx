import { Check, X } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";

export function PermissionResponseEntry({ entry }: { entry: ChatEntry }) {
  const response = entry.metadata as Record<string, unknown> | undefined;
  const approved = response?.approved === true || entry.content.startsWith("已允许");
  const rejected = response?.approved === false || entry.content.startsWith("已拒绝");

  return (
    <div className="flex justify-start">
      <span
        className={[
          "inline-flex items-center gap-1 rounded-full border px-2 py-1 text-xs font-semibold",
          approved
            ? "border-emerald-200 bg-emerald-50 text-emerald-700"
            : rejected
              ? "border-red-200 bg-red-50 text-red-700"
              : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
        ].join(" ")}
      >
        {approved ? <Check className="h-3.5 w-3.5" /> : rejected ? <X className="h-3.5 w-3.5" /> : null}
        {entry.content}
      </span>
    </div>
  );
}
