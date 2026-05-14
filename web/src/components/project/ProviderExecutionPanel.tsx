import { Bot, FileText, TerminalSquare } from "lucide-react";
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
  outputText: string | null;
  stream: string | null;
};

export function ProviderExecutionPanel({ events }: ProviderExecutionPanelProps) {
  const rows = events.map(providerEventRow).filter((row): row is ProviderEventRow => Boolean(row));

  return (
    <section
      role="region"
      aria-label="Provider execution panel"
      className="rounded-lg border-2 border-cyan-200 bg-white p-4 shadow-[0_8px_0_rgba(6,182,212,0.10),0_18px_34px_rgba(15,118,110,0.12)]"
    >
      <div className="mb-3 flex flex-wrap items-center justify-between gap-3">
        <div>
          <h2 className="text-lg font-black text-[#241B2F]">Provider 执行</h2>
          <p className="mt-1 text-sm font-semibold text-[#5E516B]">输入摘要与输出事件</p>
        </div>
        <span className="inline-flex items-center gap-2 rounded-lg border-2 border-cyan-200 bg-cyan-50 px-3 py-1 text-xs font-black text-cyan-950">
          <Bot className="h-4 w-4" />
          {rows.length}
        </span>
      </div>

      {rows.length > 0 ? (
        <ul aria-label="Provider events" className="space-y-3">
          {rows.map((row) => (
            <li
              key={row.id}
              className="rounded-lg border-2 border-slate-200 bg-slate-50 px-3 py-3"
            >
              <div className="flex flex-wrap items-center gap-2 font-mono text-[11px] font-bold text-slate-600">
                <span>{row.nodeId}</span>
                <span>{row.eventType}</span>
                {row.stream ? <span>{row.stream}</span> : null}
              </div>
              {row.inputRef && !row.outputText ? (
                <p className="mt-2 flex min-w-0 items-center gap-2 text-xs font-bold text-slate-700">
                  <FileText className="h-4 w-4 shrink-0 text-cyan-700" />
                  <span className="break-all">{row.inputRef}</span>
                </p>
              ) : null}
              {row.summary ? (
                <p className="mt-2 text-sm font-semibold leading-6 text-[#5E516B]">
                  {row.summary}
                </p>
              ) : null}
              {row.outputText ? (
                <pre className="mt-2 whitespace-pre-wrap break-words rounded-lg border-2 border-cyan-100 bg-white px-3 py-2 font-mono text-xs leading-5 text-slate-900">
                  <TerminalSquare className="mr-1 inline h-4 w-4 align-[-0.2rem] text-cyan-700" />
                  {row.outputText}
                </pre>
              ) : null}
            </li>
          ))}
        </ul>
      ) : (
        <div className="rounded-lg border-2 border-dashed border-slate-200 bg-slate-50 px-3 py-5 text-sm font-semibold text-slate-500">
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
  const outputText = readString(payload.text) ?? readString(payload.output_chunk);
  const summary = summaryText(payload.input_summary);
  const inputRef =
    readString(payload.input_ref) ??
    readString(payload.inputRef) ??
    readFirstString(payload.input_refs) ??
    readFirstString(payload.canonical_input_refs);

  if (!outputText && !summary && !inputRef) {
    return null;
  }

  return {
    id: `${event.cursor}-${event.event_type}`,
    nodeId: readString(payload.node_id) ?? "node",
    eventType: event.event_type,
    inputRef,
    summary,
    outputText,
    stream: readString(payload.stream),
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
