import { useEffect, useId, useRef, useState, type FormEvent, type KeyboardEvent } from "react";
import type {
  HumanPresentationRevision,
  HumanPresentationScope,
  SaveHumanPresentationRevisionMessage,
} from "../../api/types";

export type HumanPresentationEditorBase = {
  scope: HumanPresentationScope;
  source_projection_bundle_id: string;
  human_summary: string;
  why_split: string | null;
  dependency_explanation: string[];
  risk_explanation: string[];
  source_refs: string[];
  presentation: HumanPresentationRevision | null;
};

export function HumanPresentationEditor({
  base,
  onSave,
  saving = false,
  error = null,
}: {
  base: HumanPresentationEditorBase;
  onSave: (message: SaveHumanPresentationRevisionMessage) => void;
  saving?: boolean;
  error?: string | null;
}) {
  const id = useId();
  const errorRef = useRef<HTMLDivElement | null>(null);
  const [humanSummary, setHumanSummary] = useState("");
  const [whySplit, setWhySplit] = useState("");
  const [dependencyExplanation, setDependencyExplanation] = useState("");
  const [riskExplanation, setRiskExplanation] = useState("");
  const [sourceRefs, setSourceRefs] = useState("");

  useEffect(() => {
    const presentation = base.presentation;
    setHumanSummary(presentation?.human_summary ?? base.human_summary);
    setWhySplit(presentation?.why_split ?? base.why_split ?? "");
    setDependencyExplanation(
      (presentation?.dependency_explanation ?? base.dependency_explanation).join("\n"),
    );
    setRiskExplanation(
      (presentation?.risk_explanation ?? base.risk_explanation).join("\n"),
    );
    setSourceRefs((presentation?.source_refs ?? base.source_refs).join("\n"));
  }, [base.source_projection_bundle_id, base.presentation?.id]);

  useEffect(() => {
    if (error) {
      errorRef.current?.focus();
    }
  }, [error]);

  function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (saving || humanSummary.trim().length === 0) {
      return;
    }
    onSave({
      type: "save_human_presentation_revision",
      source_projection_bundle_id: base.source_projection_bundle_id,
      scope: base.scope,
      supersedes: base.presentation?.id ?? null,
      human_summary: humanSummary.trim(),
      why_split: nullableText(whySplit),
      dependency_explanation: lines(dependencyExplanation),
      risk_explanation: lines(riskExplanation),
      source_refs: lines(sourceRefs),
    });
  }

  function handleKeyboardSave(event: KeyboardEvent<HTMLFormElement>) {
    if ((event.ctrlKey || event.metaKey) && event.key === "Enter") {
      event.preventDefault();
      event.currentTarget.requestSubmit();
    }
  }

  return (
    <form
      aria-label="编辑人工说明"
      aria-busy={saving}
      className="mt-4 space-y-3 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-4"
      onSubmit={submit}
      onKeyDown={handleKeyboardSave}
    >
      <div>
        <label
          htmlFor={`${id}-summary`}
          className="text-xs font-semibold text-[var(--aria-ink)]"
        >
          拆分说明
        </label>
        <textarea
          id={`${id}-summary`}
          required
          rows={3}
          value={humanSummary}
          onChange={(event) => setHumanSummary(event.target.value)}
          className={editorClassName}
        />
      </div>
      <div>
        <label
          htmlFor={`${id}-why`}
          className="text-xs font-semibold text-[var(--aria-ink)]"
        >
          为什么这样拆分
        </label>
        <textarea
          id={`${id}-why`}
          rows={2}
          value={whySplit}
          onChange={(event) => setWhySplit(event.target.value)}
          className={editorClassName}
        />
      </div>
      <div className="grid gap-3 md:grid-cols-2">
        <LineEditor
          id={`${id}-dependencies`}
          label="依赖说明"
          value={dependencyExplanation}
          onChange={setDependencyExplanation}
        />
        <LineEditor
          id={`${id}-risks`}
          label="风险说明"
          value={riskExplanation}
          onChange={setRiskExplanation}
        />
      </div>
      <LineEditor
        id={`${id}-sources`}
        label="来源引用"
        value={sourceRefs}
        onChange={setSourceRefs}
        mono
      />
      {error ? (
        <div
          ref={errorRef}
          role="alert"
          tabIndex={-1}
          className="rounded border border-red-200 bg-red-50 px-3 py-2 text-xs text-red-800 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
        >
          {error}
        </div>
      ) : null}
      <div className="flex flex-wrap items-center justify-between gap-2">
        <p className="text-[11px] text-[var(--aria-ink-muted)]">
          Ctrl/⌘ + Enter 保存；说明不会改变 Coder 或 Reviewer 输入。
        </p>
        <button
          type="submit"
          disabled={saving || humanSummary.trim().length === 0}
          className="h-9 cursor-pointer rounded-md bg-[var(--aria-primary)] px-4 text-xs font-semibold text-white transition-colors hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
        >
          {saving ? "保存中…" : "保存说明"}
        </button>
      </div>
    </form>
  );
}

function LineEditor({
  id,
  label,
  value,
  onChange,
  mono = false,
}: {
  id: string;
  label: string;
  value: string;
  onChange: (value: string) => void;
  mono?: boolean;
}) {
  return (
    <div>
      <label htmlFor={id} className="text-xs font-semibold text-[var(--aria-ink)]">
        {label}
      </label>
      <textarea
        id={id}
        rows={3}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        className={`${editorClassName} ${mono ? "font-mono" : ""}`}
      />
      <p className="mt-1 text-[11px] text-[var(--aria-ink-muted)]">每行一项</p>
    </div>
  );
}

const editorClassName =
  "mt-1 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm leading-5 text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]";

function nullableText(value: string) {
  const trimmed = value.trim();
  return trimmed.length > 0 ? trimmed : null;
}

function lines(value: string) {
  return Array.from(
    new Set(
      value
        .split("\n")
        .map((line) => line.trim())
        .filter((line) => line.length > 0),
    ),
  );
}
