import { CheckCircle2, HelpCircle, Wrench } from "lucide-react";
import type { ChatEntry } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

export function AnalystVerdictEntry({ entry }: { entry: ChatEntry }) {
  const verdict = analystVerdictFromEntry(entry);
  const style = styleForVerdict(verdict, entry);
  const Icon = style.icon;

  return (
    <ChatEntryContainer
      role="analyst"
      title={style.title}
      className={style.panel}
      testId="analyst-verdict-entry"
    >
      <div className="space-y-3">
        <div className="flex min-w-0 items-start gap-2">
          <Icon className={`mt-0.5 h-4 w-4 shrink-0 ${style.iconClass}`} />
          <div className="min-w-0 text-sm text-[var(--aria-ink)]">{entry.content}</div>
        </div>
        <AnalystDecisionDetails entry={entry} />
        {verdict === "needs_fix" && style.fixHints.length > 0 ? (
          <ul className="space-y-1 text-xs text-amber-900">
            {style.fixHints.map((hint) => (
              <li key={hint} className="rounded border border-amber-200 bg-white px-2 py-1">
                {hint}
              </li>
            ))}
          </ul>
        ) : null}
        {verdict === "needs_human_input" && style.questions.length > 0 ? (
          <div className="space-y-2">
            <ul className="space-y-1 text-xs text-blue-900">
              {style.questions.map((question) => (
                <li key={question} className="rounded border border-blue-200 bg-white px-2 py-1">
                  {question}
                </li>
              ))}
            </ul>
            <div className="text-xs font-medium text-blue-700">请在下方输入框补充上下文</div>
          </div>
        ) : null}
      </div>
    </ChatEntryContainer>
  );
}

function AnalystDecisionDetails({ entry }: { entry: ChatEntry }) {
  const structuredVerdict = metadataString(entry.metadata?.structured_verdict);
  const nextStage = metadataString(entry.metadata?.next_stage);
  const reason = metadataString(entry.metadata?.reason);
  const parseError = metadataString(entry.metadata?.parse_error);
  const evidenceRefs = stringList(entry.metadata?.evidence_refs);
  const rawProviderOutputRefs = stringList(entry.metadata?.raw_provider_output_refs);

  if (
    !structuredVerdict &&
    !nextStage &&
    !reason &&
    !parseError &&
    evidenceRefs.length === 0 &&
    rawProviderOutputRefs.length === 0
  ) {
    return null;
  }

  return (
    <div className="space-y-2 rounded-md border border-white/80 bg-white/70 px-2 py-2 text-xs">
      <div className="flex min-w-0 flex-wrap gap-1.5">
        {structuredVerdict ? <DecisionBadge label="verdict" value={structuredVerdict} /> : null}
        {nextStage ? <DecisionBadge label="next" value={nextStage} /> : null}
      </div>
      {reason ? <DecisionLine label="原因" value={reason} /> : null}
      {parseError ? (
        <DecisionLine label="解析错误" value={parseError} tone="danger" />
      ) : null}
      {evidenceRefs.length > 0 ? <DecisionRefs label="证据" refs={evidenceRefs} /> : null}
      {rawProviderOutputRefs.length > 0 ? (
        <DecisionRefs label="原始输出" refs={rawProviderOutputRefs} />
      ) : null}
    </div>
  );
}

function DecisionBadge({ label, value }: { label: string; value: string }) {
  return (
    <span className="inline-flex max-w-full items-center gap-1 rounded border border-[var(--aria-line)] bg-white px-1.5 py-0.5 font-mono text-[11px] text-[var(--aria-ink)]">
      <span className="text-[var(--aria-ink-muted)]">{label}</span>
      <span className="truncate">{value}</span>
    </span>
  );
}

function DecisionLine({
  label,
  value,
  tone = "normal",
}: {
  label: string;
  value: string;
  tone?: "normal" | "danger";
}) {
  return (
    <div className={tone === "danger" ? "text-red-700" : "text-[var(--aria-ink)]"}>
      <span className="font-semibold">{label}：</span>
      <span className="break-words">{value}</span>
    </div>
  );
}

function DecisionRefs({ label, refs }: { label: string; refs: string[] }) {
  return (
    <div>
      <div className="mb-1 font-semibold text-[var(--aria-ink-muted)]">{label}</div>
      <div className="flex min-w-0 flex-wrap gap-1">
        {refs.map((ref) => (
          <span
            key={ref}
            className="max-w-full truncate rounded border border-[var(--aria-line)] bg-white px-1.5 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]"
          >
            {ref}
          </span>
        ))}
      </div>
    </div>
  );
}

function analystVerdictFromEntry(entry: ChatEntry) {
  const verdict = entry.metadata?.verdict;
  return typeof verdict === "string" ? verdict : "no_issue";
}

function metadataString(value: unknown) {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function stringList(value: unknown) {
  return Array.isArray(value)
    ? value.filter((item): item is string => typeof item === "string" && item.length > 0)
    : [];
}

function styleForVerdict(verdict: string, entry: ChatEntry) {
  if (verdict === "needs_fix") {
    return {
      title: "需要修复",
      panel: "border-amber-200 bg-amber-50",
      icon: Wrench,
      iconClass: "text-amber-600",
      fixHints: stringList(entry.metadata?.fix_hints),
      questions: [],
    };
  }
  if (verdict === "needs_human_input") {
    return {
      title: "需要人工输入",
      panel: "border-blue-200 bg-blue-50",
      icon: HelpCircle,
      iconClass: "text-blue-600",
      fixHints: [],
      questions: stringList(entry.metadata?.questions),
    };
  }
  return {
    title: "未发现问题",
    panel: "border-green-200 bg-green-50",
    icon: CheckCircle2,
    iconClass: "text-green-600",
    fixHints: [],
    questions: [],
  };
}
