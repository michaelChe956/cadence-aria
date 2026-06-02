import { Check, ListChecks } from "lucide-react";
import { useState } from "react";
import type { ChatEntry, ChoiceResponsePayload } from "../../../state/chat-entries";
import { ChatEntryContainer } from "../ChatEntryContainer";

interface ChoiceRequestEntryProps {
  entry: ChatEntry;
  onRespond?: (entry: ChatEntry, response: ChoiceResponsePayload) => void;
  embedded?: boolean;
}

export function ChoiceRequestEntry({
  entry,
  onRespond,
  embedded = false,
}: ChoiceRequestEntryProps) {
  const metadata = entry.metadata as Record<string, unknown> | undefined;
  const options = choiceOptions(metadata?.options);
  const allowMultiple = metadata?.allow_multiple === true;
  const allowFreeText = metadata?.allow_free_text === true;
  const prompt = stringField(metadata, "prompt") ?? entry.content;
  const sourceLabel = choiceSourceLabel(stringField(metadata, "source"));
  const response = responseFromMetadata(metadata?.response);
  const isResolved = entry.resolved === true;
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [freeText, setFreeText] = useState("");
  const [submitting, setSubmitting] = useState(false);

  function toggleOption(optionId: string) {
    if (allowMultiple) {
      setSelectedIds((current) =>
        current.includes(optionId)
          ? current.filter((id) => id !== optionId)
          : [...current, optionId],
      );
      return;
    }
    setSelectedIds([optionId]);
  }

  function submitChoice() {
    const trimmedText = freeText.trim();
    const payload: ChoiceResponsePayload = {
      selected_option_ids: selectedIds,
      free_text: trimmedText ? trimmedText : null,
    };
    setSubmitting(true);
    onRespond?.(entry, payload);
  }

  const resolvedSummary = choiceSummary(response, options);
  const content = (
    <div className="space-y-3">
      <div className="flex items-start gap-2 text-sm text-[var(--aria-ink)]">
        <ListChecks className="mt-0.5 h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
        <div className="min-w-0">
          <div className="font-medium">选择请求</div>
          {sourceLabel ? (
            <span className="mt-1 inline-flex rounded-md border border-[var(--aria-line)] bg-white px-2 py-0.5 text-xs font-medium text-[var(--aria-ink-muted)]">
              {sourceLabel}
            </span>
          ) : null}
          <div className="mt-1 text-[var(--aria-ink-muted)]">{prompt}</div>
        </div>
      </div>

      {isResolved ? (
        <span className="inline-flex items-center gap-1 rounded-md border border-emerald-200 bg-emerald-50 px-2 py-1 text-xs font-semibold text-emerald-700">
          <Check className="h-3.5 w-3.5" />
          <span>已选择</span>
          {resolvedSummary ? <span>：{resolvedSummary}</span> : null}
        </span>
      ) : (
        <>
          {options.length > 0 ? (
            <div className="space-y-2">
              {options.map((option) => (
                <label
                  key={option.id}
                  className="flex min-h-8 cursor-pointer items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-2 text-sm text-[var(--aria-ink)]"
                >
                  <input
                    type={allowMultiple ? "checkbox" : "radio"}
                    name={`choice-${entry.id}`}
                    checked={selectedIds.includes(option.id)}
                    onChange={() => toggleOption(option.id)}
                    disabled={submitting}
                  />
                  <span className="min-w-0">
                    <span className="font-medium">{option.label}</span>
                    {option.description ? (
                      <span className="ml-2 text-xs text-[var(--aria-ink-muted)]">
                        {option.description}
                      </span>
                    ) : null}
                  </span>
                </label>
              ))}
            </div>
          ) : null}

          {allowFreeText ? (
            <label className="block text-xs font-medium text-[var(--aria-ink-muted)]">
              补充内容
              <textarea
                aria-label="补充内容"
                value={freeText}
                onChange={(event) => setFreeText(event.target.value)}
                disabled={submitting}
                className="mt-1 min-h-20 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-2 py-1 text-sm font-normal text-[var(--aria-ink)]"
              />
            </label>
          ) : null}

          {onRespond ? (
            <div className="flex justify-end">
              <button
                type="button"
                onClick={submitChoice}
                disabled={submitting || (selectedIds.length === 0 && freeText.trim().length === 0)}
                className="inline-flex h-8 items-center gap-1 rounded-md border border-[var(--aria-primary-soft)] bg-white px-3 text-xs font-semibold text-[var(--aria-primary)] hover:bg-[var(--aria-primary-soft)] disabled:cursor-not-allowed disabled:opacity-60"
              >
                <Check className="h-3.5 w-3.5" />
                提交选择
              </button>
            </div>
          ) : null}
        </>
      )}
    </div>
  );

  if (embedded) {
    return (
      <div
        data-testid="choice-request-entry"
        className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2"
      >
        {content}
      </div>
    );
  }

  return (
    <ChatEntryContainer role="system" title="选择请求" testId="choice-request-entry">
      {content}
    </ChatEntryContainer>
  );
}

interface ChoiceOption {
  id: string;
  label: string;
  description?: string | null;
}

function choiceOptions(value: unknown): ChoiceOption[] {
  if (!Array.isArray(value)) {
    return [];
  }
  return value.flatMap((item) => {
    if (!isRecord(item)) {
      return [];
    }
    const id = stringField(item, "id");
    const label = stringField(item, "label") ?? id;
    if (!id || !label) {
      return [];
    }
    return [{ id, label, description: stringField(item, "description") }];
  });
}

function responseFromMetadata(value: unknown): ChoiceResponsePayload | null {
  if (!isRecord(value)) {
    return null;
  }
  const selected = Array.isArray(value.selected_option_ids)
    ? value.selected_option_ids.filter((item): item is string => typeof item === "string")
    : [];
  const freeText = typeof value.free_text === "string" ? value.free_text : null;
  return { selected_option_ids: selected, free_text: freeText };
}

function choiceSummary(response: ChoiceResponsePayload | null, options: ChoiceOption[]) {
  if (!response) {
    return "";
  }
  const labels = response.selected_option_ids.map(
    (id) => options.find((option) => option.id === id)?.label ?? id,
  );
  if (response.free_text) {
    labels.push(response.free_text);
  }
  return labels.join("、");
}

function choiceSourceLabel(source: string | null) {
  switch (source) {
    case "ask_user_question":
      return "AskUserQuestion";
    case "request_user_input":
      return "requestUserInput";
    case "text_fallback":
      return "文本 fallback";
    case "provider_choice":
      return "provider choice";
    default:
      return null;
  }
}

function stringField(value: unknown, key: string) {
  if (!isRecord(value)) {
    return null;
  }
  const field = value[key];
  return typeof field === "string" ? field : null;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null;
}
