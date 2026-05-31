import {
  GitBranch,
  Layers3,
  ListChecks,
  Sparkles,
  ScrollText,
  Trash2,
} from "lucide-react";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";

export function LifecycleCard({
  card,
  selected,
  deleting = false,
  onSelect,
  onGenerateStorySpec,
  onDelete,
}: {
  card: LifecycleCardData;
  selected: boolean;
  deleting?: boolean;
  onSelect: () => void;
  onGenerateStorySpec?: () => void;
  onDelete?: () => void;
}) {
  const codingStatusLabel = workItemCodingStatusLabel(card);
  const visual = lifecycleCardVisual(card.kind);
  const Icon =
    card.kind === "issue"
      ? ListChecks
      : card.kind === "story_spec"
        ? ScrollText
        : card.kind === "design_spec"
          ? Layers3
          : GitBranch;

  return (
    <div
      data-testid={`lifecycle-card-${card.kind}`}
      data-color-token={visual.token}
      data-delete-state={deleting ? "deleting" : "idle"}
      aria-busy={deleting}
      className={[
        "flex w-full items-start gap-2 rounded-md border border-l-4 p-3 text-left transition-colors focus-within:ring-2 focus-within:ring-[var(--aria-primary)]",
        visual.cardClassName,
        deleting
          ? "aria-lifecycle-card--deleting"
          : selected
            ? "shadow-sm ring-2 ring-[var(--aria-primary)]"
            : visual.hoverClassName,
      ].join(" ")}
    >
      <button
        type="button"
        aria-label={card.title}
        aria-pressed={selected}
        disabled={deleting}
        onClick={onSelect}
        className="min-w-0 flex-1 cursor-pointer text-left focus-visible:outline-none"
      >
        <span className="flex min-w-0 items-start gap-2">
          <Icon className={`mt-0.5 h-4 w-4 shrink-0 ${visual.iconClassName}`} />
          <span className="min-w-0 flex-1">
            <span
              className={`mb-1 inline-flex items-center rounded-full border px-1.5 py-0.5 text-[10px] font-semibold ${visual.labelClassName}`}
            >
              {visual.label}
            </span>
            <span
              data-testid="lifecycle-card-title"
              className="line-clamp-2 block whitespace-normal break-words text-sm font-semibold leading-5 text-[var(--aria-ink)]"
            >
              {card.title}
            </span>
            {card.preview ? (
              <span className="mt-1 line-clamp-2 block max-h-10 overflow-hidden whitespace-pre-wrap break-words text-xs leading-5 text-[var(--aria-ink-muted)]">
                {card.preview}
              </span>
            ) : null}
            <span className="mt-1 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
              <span>{card.id}</span>
              <span
                className={`rounded border px-1.5 py-0.5 ${visual.metaClassName}`}
              >
                {card.status}
              </span>
              {card.version ? (
                <span
                  className={`rounded border px-1.5 py-0.5 ${visual.metaClassName}`}
                >
                  v{card.version}
                </span>
              ) : null}
              {codingStatusLabel ? (
                <span className="rounded border border-[var(--aria-primary)] px-1.5 py-0.5 text-[var(--aria-primary)]">
                  {codingStatusLabel}
                </span>
              ) : null}
            </span>
          </span>
        </span>
      </button>
      {card.kind === "issue" && onGenerateStorySpec ? (
        <button
          type="button"
          disabled={deleting}
          onClick={onGenerateStorySpec}
          className="inline-flex h-7 shrink-0 cursor-pointer items-center gap-1 rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-2 text-xs font-semibold text-white hover:opacity-90"
        >
          <Sparkles className="h-3.5 w-3.5" />
          生成 Story Spec
        </button>
      ) : null}
      {onDelete ? (
        <button
          type="button"
          aria-label={`删除 ${lifecycleCardDeleteLabel(card.kind)} ${card.title}`}
          disabled={deleting}
          onClick={(event) => {
            event.stopPropagation();
            onDelete();
          }}
          className="inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:border-[var(--aria-danger)] hover:text-[var(--aria-danger)]"
        >
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      ) : null}
    </div>
  );
}

function lifecycleCardDeleteLabel(kind: LifecycleCardData["kind"]) {
  if (kind === "issue") {
    return "Issue";
  }
  if (kind === "story_spec") {
    return "Story Spec";
  }
  if (kind === "design_spec") {
    return "Design Spec";
  }
  return "Work Item";
}

function lifecycleCardVisual(kind: LifecycleCardData["kind"]) {
  const visuals = {
    issue: {
      label: "Issue",
      token: "sky",
      cardClassName: "border-sky-200 border-l-sky-500 bg-sky-50/70",
      hoverClassName: "hover:border-sky-300 hover:bg-sky-50",
      iconClassName: "text-sky-700",
      labelClassName: "border-sky-200 bg-sky-100 text-sky-800",
      metaClassName: "border-sky-200 bg-white/70 text-sky-900",
    },
    story_spec: {
      label: "Story",
      token: "emerald",
      cardClassName: "border-emerald-200 border-l-emerald-500 bg-emerald-50/70",
      hoverClassName: "hover:border-emerald-300 hover:bg-emerald-50",
      iconClassName: "text-emerald-700",
      labelClassName: "border-emerald-200 bg-emerald-100 text-emerald-800",
      metaClassName: "border-emerald-200 bg-white/70 text-emerald-900",
    },
    design_spec: {
      label: "Design",
      token: "violet",
      cardClassName: "border-violet-200 border-l-violet-500 bg-violet-50/70",
      hoverClassName: "hover:border-violet-300 hover:bg-violet-50",
      iconClassName: "text-violet-700",
      labelClassName: "border-violet-200 bg-violet-100 text-violet-800",
      metaClassName: "border-violet-200 bg-white/70 text-violet-900",
    },
    work_item: {
      label: "Work Item",
      token: "amber",
      cardClassName: "border-amber-200 border-l-amber-500 bg-amber-50/70",
      hoverClassName: "hover:border-amber-300 hover:bg-amber-50",
      iconClassName: "text-amber-700",
      labelClassName: "border-amber-200 bg-amber-100 text-amber-900",
      metaClassName: "border-amber-200 bg-white/70 text-amber-900",
    },
  } satisfies Record<
    LifecycleCardData["kind"],
    {
      label: string;
      token: string;
      cardClassName: string;
      hoverClassName: string;
      iconClassName: string;
      labelClassName: string;
      metaClassName: string;
    }
  >;

  return visuals[kind];
}

function workItemCodingStatusLabel(card: LifecycleCardData) {
  if (card.kind !== "work_item") {
    return null;
  }
  const latestAttempt = card.raw.latest_attempt;
  if (latestAttempt) {
    return `${latestAttempt.status} · ${latestAttempt.stage}`;
  }
  return card.raw.plan_status === "confirmed" ? "可编码" : null;
}
