import { Bot, FileText } from "lucide-react";
import type { WebEvent } from "../../api/types";

export type ProviderExecutionPanelProps = {
  events: WebEvent[];
};

type ProviderEventRow = {
  id: string;
  nodeId: string;
  eventType: string;
  inputRef: string | null;
  summary: string | null;
};

export function ProviderExecutionPanel({ events }: ProviderExecutionPanelProps) {
  const rows = events.map(providerEventRow).filter((row): row is ProviderEventRow => Boolean(row));

  return (
    <section
      role="region"
      aria-label="Provider execution panel"
      className="rounded-lg border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4 shadow-sm"
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-sm font-semibold text-[var(--aria-ink)]">Provider 执行</h2>
          <p className="mt-0.5 text-xs font-medium text-[var(--aria-ink-muted)]">
            输入引用与摘要
          </p>
        </div>
        <span className="inline-flex h-7 items-center gap-1.5 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 font-mono text-xs font-semibold text-[var(--aria-ink-muted)]">
          <Bot className="h-4 w-4" />
          {rows.length}
        </span>
      </div>

      {rows.length > 0 ? (
        <ul aria-label="Provider events" className="space-y-3">
          {rows.map((row) => (
            <li
              key={row.id}
              className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-3"
            >
              <div className="flex flex-wrap items-center gap-2 font-mono text-[11px] font-medium text-[var(--aria-ink-muted)]">
                <span>{row.nodeId}</span>
                <span>{row.eventType}</span>
              </div>
              {row.inputRef ? (
                <p className="mt-2 flex min-w-0 items-center gap-2 text-xs font-medium text-[var(--aria-ink)]">
                  <FileText className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
                  <span className="break-all">{row.inputRef}</span>
                </p>
              ) : null}
              {row.summary ? (
                <p className="mt-2 text-sm font-medium leading-6 text-[var(--aria-ink-muted)]">
                  {row.summary}
                </p>
              ) : null}
            </li>
          ))}
        </ul>
      ) : (
        <div className="rounded-md border border-dashed border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-5 text-sm font-medium text-[var(--aria-ink-muted)]">
          暂无 provider 事件
        </div>
      )}
    </section>
  );
}

function providerEventRow(event: WebEvent): ProviderEventRow | null {
  if (!event.event_type.includes("provider")) {
    return null;
  }
  const payload = isRecord(event.payload) ? event.payload : {};
  const summary = summaryText(payload.input_summary);
  const inputRef =
    readString(payload.input_ref) ??
    readString(payload.inputRef) ??
    readFirstString(payload.input_refs) ??
    readFirstString(payload.canonical_input_refs);

  if (!summary && !inputRef) {
    return null;
  }

  return {
    id: `${event.cursor}-${event.event_type}`,
    nodeId: readString(payload.node_id) ?? "node",
    eventType: event.event_type,
    inputRef,
    summary,
  };
}

function summaryText(value: unknown): string | null {
  if (typeof value === "string") {
    return value;
  }
  if (isRecord(value)) {
    return JSON.stringify(omitSensitiveInputFields(value));
  }
  return null;
}

function omitSensitiveInputFields(value: Record<string, unknown>): Record<string, unknown> {
  return Object.fromEntries(
    Object.entries(value).filter(([key]) => key !== "prompt" && key !== "input_full"),
  );
}

function readString(value: unknown): string | null {
  return typeof value === "string" && value.trim() !== "" ? value : null;
}

function readFirstString(value: unknown): string | null {
  return Array.isArray(value) ? (value.find((item) => typeof item === "string") ?? null) : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}
