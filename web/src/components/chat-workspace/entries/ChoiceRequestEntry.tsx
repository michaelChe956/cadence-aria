import { Check, CircleAlert, ListChecks } from "lucide-react";
import { useState } from "react";
import type {
  ChatEntry,
  ChoiceAnswerPayload,
  ChoiceResponsePayload,
} from "../../../state/chat-entries";
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
  const allowMultiple = metadata?.allow_multiple === true;
  const allowFreeText = metadata?.allow_free_text === true;
  const prompt = stringField(metadata, "prompt") ?? entry.content;
  const options = choiceOptions(metadata?.options);
  const rawQuestions = metadata?.questions;
  const questions = choiceQuestions(
    rawQuestions,
    prompt,
    options,
    allowMultiple,
    allowFreeText,
  );
  const structuredQuestionCount = Array.isArray(rawQuestions) ? rawQuestions.length : 0;
  const usesStructuredQuestions = structuredQuestionCount > 0;
  const sourceLabel = choiceSourceLabel(stringField(metadata, "source"));
  const response = responseFromMetadata(metadata?.response);
  const isResolved = entry.resolved === true;
  const isRejected = metadata?.rejected === true;
  const rejectionReason = stringField(metadata, "rejection_reason");
  const [selectedByQuestion, setSelectedByQuestion] = useState<Record<string, string[]>>({});
  const [freeTextByQuestion, setFreeTextByQuestion] = useState<Record<string, string>>({});
  const [submitting, setSubmitting] = useState(false);

  function selectedIdsFor(questionId: string) {
    return selectedByQuestion[questionId] ?? [];
  }

  function freeTextFor(questionId: string) {
    return freeTextByQuestion[questionId] ?? "";
  }

  function toggleOption(question: ChoiceQuestion, optionId: string) {
    setSelectedByQuestion((current) => {
      const selectedIds = current[question.id] ?? [];
      const nextIds = question.allow_multiple
        ? selectedIds.includes(optionId)
          ? selectedIds.filter((id) => id !== optionId)
          : [...selectedIds, optionId]
        : [optionId];
      return {
        ...current,
        [question.id]: nextIds,
      };
    });
  }

  function updateFreeText(questionId: string, value: string) {
    setFreeTextByQuestion((current) => ({
      ...current,
      [questionId]: value,
    }));
  }

  function answerFor(question: ChoiceQuestion): ChoiceAnswerPayload {
    const trimmedText = freeTextFor(question.id).trim();
    return {
      question_id: question.id,
      selected_option_ids: selectedIdsFor(question.id),
      free_text: trimmedText ? trimmedText : null,
    };
  }

  function isQuestionAnswered(question: ChoiceQuestion) {
    const answer = answerFor(question);
    return answer.selected_option_ids.length > 0 || Boolean(answer.free_text);
  }

  function choiceAnswers() {
    return questions.map(answerFor);
  }

  function submitChoice() {
    const answers = choiceAnswers();
    const selectedOptionIds = answers.flatMap((answer) => answer.selected_option_ids);
    const freeTexts = answers
      .map((answer) => answer.free_text)
      .filter((text): text is string => Boolean(text));
    const payload: ChoiceResponsePayload = {
      selected_option_ids: selectedOptionIds,
      free_text: freeTexts.length > 0 ? freeTexts.join("\n\n") : null,
      ...(usesStructuredQuestions ? { answers } : {}),
    };
    setSubmitting(true);
    onRespond?.(entry, payload);
  }

  function renderQuestion(question: ChoiceQuestion, showPrompt: boolean) {
    const selectedIds = selectedIdsFor(question.id);
    const freeText = freeTextFor(question.id);
    const textareaLabel =
      usesStructuredQuestions || questions.length > 1
        ? `补充内容：${question.prompt}`
        : "补充内容";
    return (
      <div key={question.id} className="space-y-2">
        {showPrompt ? (
          <div className="text-sm font-medium text-[var(--aria-ink)]">{question.prompt}</div>
        ) : null}
        {question.options.length > 0 ? (
          <div className="space-y-2">
            {question.options.map((option) => (
              <label
                key={`${question.id}:${option.id}`}
                className="flex min-h-8 cursor-pointer items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-2 text-sm text-[var(--aria-ink)]"
              >
                <input
                  type={question.allow_multiple ? "checkbox" : "radio"}
                  name={`choice-${entry.id}-${question.id}`}
                  checked={selectedIds.includes(option.id)}
                  onChange={() => toggleOption(question, option.id)}
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

        {question.allow_free_text ? (
          <label className="block text-xs font-medium text-[var(--aria-ink-muted)]">
            补充内容
            <textarea
              aria-label={textareaLabel}
              value={freeText}
              onChange={(event) => updateFreeText(question.id, event.target.value)}
              disabled={submitting}
              className="mt-1 min-h-20 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-2 py-1 text-sm font-normal text-[var(--aria-ink)]"
            />
          </label>
        ) : null}
      </div>
    );
  }

  const canSubmit = questions.length > 0 && questions.every(isQuestionAnswered);
  const showQuestionPrompts = usesStructuredQuestions || questions.length > 1;

  const resolvedSummary = choiceSummary(response, questions, options);
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
        isRejected ? (
          <span className="inline-flex max-w-full items-start gap-1 rounded-md border border-amber-200 bg-amber-50 px-2 py-1 text-xs font-semibold text-amber-700">
            <CircleAlert className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            <span className="min-w-0">
              <span>选择已失效</span>
              {rejectionReason ? (
                <span className="ml-1 break-words font-medium">{rejectionReason}</span>
              ) : null}
            </span>
          </span>
        ) : (
          <span className="inline-flex items-center gap-1 rounded-md border border-emerald-200 bg-emerald-50 px-2 py-1 text-xs font-semibold text-emerald-700">
            <Check className="h-3.5 w-3.5" />
            <span>已选择</span>
            {resolvedSummary ? <span>：{resolvedSummary}</span> : null}
          </span>
        )
      ) : (
        <>
          <div className="space-y-3">
            {questions.map((question) => renderQuestion(question, showQuestionPrompts))}
          </div>

          {onRespond ? (
            <div className="flex justify-end">
              <button
                type="button"
                onClick={submitChoice}
                disabled={submitting || !canSubmit}
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

interface ChoiceQuestion {
  id: string;
  prompt: string;
  options: ChoiceOption[];
  allow_multiple: boolean;
  allow_free_text: boolean;
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

function choiceQuestions(
  value: unknown,
  fallbackPrompt: string,
  fallbackOptions: ChoiceOption[],
  fallbackAllowMultiple: boolean,
  fallbackAllowFreeText: boolean,
): ChoiceQuestion[] {
  if (Array.isArray(value) && value.length > 0) {
    const questions = value.flatMap((item) => {
      if (!isRecord(item)) {
        return [];
      }
      const id = stringField(item, "id");
      const prompt = stringField(item, "prompt") ?? stringField(item, "question");
      if (!id || !prompt) {
        return [];
      }
      return [
        {
          id,
          prompt,
          options: choiceOptions(item.options),
          allow_multiple: item.allow_multiple === true,
          allow_free_text: item.allow_free_text === true,
        },
      ];
    });
    if (questions.length > 0) {
      return questions;
    }
  }
  return [
    {
      id: "default",
      prompt: fallbackPrompt,
      options: fallbackOptions,
      allow_multiple: fallbackAllowMultiple,
      allow_free_text: fallbackAllowFreeText,
    },
  ];
}

function responseFromMetadata(value: unknown): ChoiceResponsePayload | null {
  if (!isRecord(value)) {
    return null;
  }
  const selected = Array.isArray(value.selected_option_ids)
    ? value.selected_option_ids.filter((item): item is string => typeof item === "string")
    : [];
  const freeText = typeof value.free_text === "string" ? value.free_text : null;
  const answers = Array.isArray(value.answers)
    ? value.answers.flatMap((item): ChoiceAnswerPayload[] => {
        if (!isRecord(item)) {
          return [];
        }
        const questionId = stringField(item, "question_id");
        if (!questionId) {
          return [];
        }
        const answerSelected = Array.isArray(item.selected_option_ids)
          ? item.selected_option_ids.filter((id): id is string => typeof id === "string")
          : [];
        const answerFreeText = typeof item.free_text === "string" ? item.free_text : null;
        return [
          {
            question_id: questionId,
            selected_option_ids: answerSelected,
            free_text: answerFreeText,
          },
        ];
      })
    : [];
  return {
    selected_option_ids: selected,
    free_text: freeText,
    ...(answers.length > 0 ? { answers } : {}),
  };
}

function choiceSummary(
  response: ChoiceResponsePayload | null,
  questions: ChoiceQuestion[],
  fallbackOptions: ChoiceOption[],
) {
  if (!response) {
    return "";
  }
  if (response.answers && response.answers.length > 0) {
    const labels = response.answers.flatMap((answer) => {
      const question = questions.find((item) => item.id === answer.question_id);
      const selected = answer.selected_option_ids.map(
        (id) =>
          question?.options.find((option) => option.id === id)?.label ??
          fallbackOptions.find((option) => option.id === id)?.label ??
          id,
      );
      if (answer.free_text) {
        selected.push(answer.free_text);
      }
      return selected;
    });
    return labels.join("、");
  }
  const labels = response.selected_option_ids.map(
    (id) => fallbackOptions.find((option) => option.id === id)?.label ?? id,
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
