import { GitBranch } from "lucide-react";
import { useState } from "react";
import type { RevisionPath } from "../../../api/types";

interface ReviewDecisionStagePanelProps {
  reviewer: string;
  verdict: string;
  summary: string;
  onSelectPath: (path: RevisionPath, extraContext?: string) => void;
}

const PATHS: Array<{
  key: RevisionPath;
  label: string;
  description: string;
}> = [
  {
    key: "revise",
    label: "直接返修",
    description: "Author 基于审核意见自动修订",
  },
  {
    key: "revise-with-context",
    label: "补充上下文后返修",
    description: "追加上下文后再进入修订",
  },
  {
    key: "skip-to-human",
    label: "跳过审核结论，进入人工确认",
    description: "不自动返修，直接交给人工确认",
  },
];

export function ReviewDecisionStagePanel({
  reviewer,
  verdict,
  summary,
  onSelectPath,
}: ReviewDecisionStagePanelProps) {
  const [selectedPath, setSelectedPath] = useState<RevisionPath>("revise");
  const [extraContext, setExtraContext] = useState("");

  return (
    <section
      data-testid="review-decision-panel"
      className="space-y-4 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] p-4"
    >
      <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
        <div className="flex items-center gap-2 text-sm font-semibold text-[var(--aria-ink)]">
          <GitBranch className="h-4 w-4 text-[var(--aria-primary)]" />
          审核结论：{verdictLabel(verdict)}
        </div>
        <div className="mt-1 text-xs text-[var(--aria-ink-muted)]">
          Reviewer: {providerLabel(reviewer)} · Verdict: {verdict}
        </div>
        <p className="mt-2 text-sm text-[var(--aria-ink)]">{summary}</p>
      </div>

      <div className="space-y-2">
        {PATHS.map((path) => (
          <label
            key={path.key}
            className={
              selectedPath === path.key
                ? "flex cursor-pointer items-start gap-2 rounded-md border border-[var(--aria-primary)] bg-white p-3 ring-1 ring-[var(--aria-primary)]"
                : "flex cursor-pointer items-start gap-2 rounded-md border border-[var(--aria-line)] bg-white p-3 hover:border-[var(--aria-primary)]"
            }
          >
            <input
              type="radio"
              name="revision-path"
              aria-label={path.label}
              value={path.key}
              checked={selectedPath === path.key}
              onChange={() => setSelectedPath(path.key)}
              className="mt-1"
            />
            <span className="min-w-0">
              <span className="block text-sm font-semibold text-[var(--aria-ink)]">{path.label}</span>
              <span className="mt-1 block text-xs text-[var(--aria-ink-muted)]">
                {path.description}
              </span>
            </span>
          </label>
        ))}
      </div>

      {selectedPath === "revise-with-context" ? (
        <label className="block text-sm font-semibold text-[var(--aria-ink)]">
          补充上下文
          <textarea
            value={extraContext}
            onChange={(event) => setExtraContext(event.target.value)}
            rows={3}
            className="mt-2 w-full resize-y rounded-md border border-[var(--aria-line)] bg-white px-3 py-2 text-sm font-normal text-[var(--aria-ink)]"
          />
        </label>
      ) : null}

      <div className="flex justify-end">
        <button
          type="button"
          onClick={() =>
            onSelectPath(
              selectedPath,
              selectedPath === "revise-with-context" ? extraContext : undefined,
            )
          }
          className="inline-flex h-9 items-center rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-4 text-sm font-semibold text-white"
        >
          确定路径
        </button>
      </div>
    </section>
  );
}

function verdictLabel(verdict: string) {
  if (verdict === "revise") return "建议返修";
  if (verdict === "pass") return "通过";
  if (verdict === "needs_human") return "需要人工确认";
  return verdict;
}

function providerLabel(provider: string) {
  if (provider === "claude_code") return "Claude Code";
  if (provider === "codex") return "Codex";
  if (provider === "fake") return "Fake";
  return provider;
}
