import { Check, ShieldAlert, X } from "lucide-react";
import { useState } from "react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function PermissionRequestEntry({
  entry,
  onRespond,
  embedded = false,
}: {
  entry: ChatEntry;
  onRespond?: (entry: ChatEntry, approved: boolean) => void;
  embedded?: boolean;
}) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const request = isRecord(metadata?.request) ? metadata.request : null;
  const toolName = stringField(request, "tool_name") ?? stringField(metadata, "tool_name") ?? "权限请求";
  const description =
    stringField(request, "description") ?? stringField(metadata, "description") ?? entry.content;
  const riskLevel = stringField(metadata, "risk_level") ?? stringField(request, "risk_level");
  const approved =
    metadata?.approved === true || (isRecord(metadata?.response) && metadata.response.approved === true);
  const rejected =
    metadata?.approved === false ||
    (isRecord(metadata?.response) && metadata.response.approved === false);

  const content = (
    <PermissionRequestContent
      entry={entry}
      toolName={toolName}
      description={description}
      riskLevel={riskLevel}
      resolved={entry.resolved === true}
      approved={approved}
      rejected={rejected}
      onRespond={onRespond}
    />
  );

  if (embedded) {
    return (
      <div
        data-testid="permission-request-entry"
        className="rounded-md border border-amber-200 bg-amber-50 px-3 py-2"
      >
        {content}
      </div>
    );
  }

  return (
    <ChatEntryContainer role="system" title="权限请求" testId="permission-request-entry">
      {content}
    </ChatEntryContainer>
  );
}

function PermissionRequestContent({
  entry,
  toolName,
  description,
  riskLevel,
  resolved,
  approved,
  rejected,
  onRespond,
}: {
  entry: ChatEntry;
  toolName: string;
  description: string;
  riskLevel: string | null;
  resolved: boolean;
  approved: boolean;
  rejected: boolean;
  onRespond?: (entry: ChatEntry, approved: boolean) => void;
}) {
  const [loading, setLoading] = useState(false);

  function respond(approved: boolean) {
    setLoading(true);
    onRespond?.(entry, approved);
  }

  return (
    <div className="space-y-3">
      <div className="flex items-start gap-2 text-sm text-[var(--aria-ink)]">
        <ShieldAlert className="mt-0.5 h-4 w-4 shrink-0 text-amber-600" />
        <div className="min-w-0">
          <div className="font-medium">{toolName}</div>
          <div className="mt-1 text-[var(--aria-ink-muted)]">{description}</div>
        </div>
      </div>
      <div className="flex flex-wrap items-center gap-2">
        {riskLevel ? (
          <span className="inline-flex items-center rounded-full border border-amber-300 bg-amber-100 px-2 py-0.5 text-xs font-medium text-amber-800">
            {riskLevel}
          </span>
        ) : null}
        {resolved ? (
          <span
            className={[
              "ml-auto inline-flex items-center rounded-md px-2 py-1 text-xs font-semibold ring-1",
              approved
                ? "bg-emerald-50 text-emerald-700 ring-emerald-200"
                : rejected
                  ? "bg-red-50 text-red-700 ring-red-200"
                  : "bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)] ring-[var(--aria-line)]",
            ].join(" ")}
          >
            {approved ? "已允许" : rejected ? "已拒绝" : "已响应"}
          </span>
        ) : onRespond ? (
          <div className="ml-auto flex items-center gap-2">
            <button
              type="button"
              onClick={() => respond(false)}
              disabled={loading}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-red-200 bg-white px-3 text-xs font-semibold text-red-700 hover:bg-red-50 disabled:cursor-not-allowed disabled:opacity-60"
            >
              <X className="h-3.5 w-3.5" />
              拒绝
            </button>
            <button
              type="button"
              onClick={() => respond(true)}
              disabled={loading}
              className="inline-flex h-8 items-center gap-1 rounded-md border border-emerald-200 bg-white px-3 text-xs font-semibold text-emerald-700 hover:bg-emerald-50 disabled:cursor-not-allowed disabled:opacity-60"
            >
              <Check className="h-3.5 w-3.5" />
              允许
            </button>
          </div>
        ) : null}
      </div>
    </div>
  );
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}

function stringField(value: unknown, key: string) {
  if (!isRecord(value)) {
    return null;
  }
  const field = value[key];
  return typeof field === "string" ? field : null;
}
