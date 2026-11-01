import {
  GitBranch,
  Layers3,
  ListChecks,
  Sparkles,
  ScrollText,
  Trash2,
} from "lucide-react";
import type { LifecycleWorkItem } from "../../api/types";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import {
  workItemKindLabel,
  workItemWaitingReason,
} from "../../state/lifecycle-workbench-store";

// —— 实体视觉语言共享层（卡片与抽屉同构）——
// 状态 chips 只跟「状态」走：同一状态在卡片与抽屉两个表面使用相同文案、
// 语义色调与样式类；kind 色仅属于边框/图标/标签等结构元素。
export const LIFECYCLE_STATUS_LABELS: Record<string, string> = {
  confirmed: "已确认",
  draft: "草稿",
  in_review: "审核中",
  change_requested: "要求修改",
  blocked: "阻塞",
  pending: "待处理",
  planning: "规划中",
  completed: "已完成",
};

export type LifecycleStatusTone = "confirmed" | "draft" | "neutral";

export function lifecycleStatusTone(status: string): LifecycleStatusTone {
  if (status === "confirmed" || status === "completed") {
    return "confirmed";
  }
  if (status === "draft") {
    return "draft";
  }
  return "neutral";
}

export function lifecycleStatusChipClass(status: string): string {
  const tone = lifecycleStatusTone(status);
  if (tone === "confirmed") {
    return "rounded border px-1.5 py-0.5 border-emerald-300 bg-white/70 text-emerald-800";
  }
  if (tone === "draft") {
    return "rounded border px-1.5 py-0.5 border-amber-300 bg-white/70 text-amber-800";
  }
  return "rounded border px-1.5 py-0.5 border-[var(--aria-line)] bg-white/70 text-[var(--aria-ink-muted)]";
}

export const LIFECYCLE_META_CHIP_CLASS =
  "rounded border px-1.5 py-0.5 border-[var(--aria-line)] bg-white/70 text-[var(--aria-ink-muted)]";

export function LifecycleCard({
  card,
  selected,
  deleting = false,
  onSelect,
  onGenerateStorySpec,
  onDelete,
  allWorkItems,
}: {
  card: LifecycleCardData;
  selected: boolean;
  deleting?: boolean;
  onSelect: () => void;
  onGenerateStorySpec?: () => void;
  onDelete?: () => void;
  allWorkItems?: LifecycleWorkItem[];
}) {
  const workItemStatusLabel = workItemStatusBadge(card, allWorkItems);
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
      aria-current={selected ? "true" : undefined}
      aria-busy={deleting || undefined}
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
              <span
                data-testid="lifecycle-card-id-chip"
                className={LIFECYCLE_META_CHIP_CLASS}
              >
                {card.id}
              </span>
              <span
                data-testid="lifecycle-card-status-chip"
                data-tone={lifecycleStatusTone(card.status)}
                className={lifecycleStatusChipClass(card.status)}
              >
                {LIFECYCLE_STATUS_LABELS[card.status] ?? card.status}
              </span>
              {card.version ? (
                <span
                  data-testid="lifecycle-card-version-chip"
                  className={LIFECYCLE_META_CHIP_CLASS}
                >
                  v{card.version}
                </span>
              ) : null}
              {card.kind === "work_item" ? (
                <span className={LIFECYCLE_META_CHIP_CLASS}>
                  {workItemKindLabel(card.raw.kind)}
                </span>
              ) : null}
              {workItemReviewStatusLabel(card) ? (
                <span
                  data-testid="lifecycle-card-review-status"
                  className={[
                    "rounded border px-1.5 py-0.5",
                    workItemReviewStatusLabel(card) === "Review 进行中"
                      ? "border-sky-300 bg-sky-50 text-sky-800"
                      : "border-[var(--aria-primary)] bg-white/70 text-[var(--aria-primary)]",
                  ].join(" ")}
                >
                  {workItemReviewStatusLabel(card)}
                </span>
              ) : null}
              {workItemStatusLabel ? (
                <span
                  className={[
                    "rounded border px-1.5 py-0.5",
                    workItemStatusLabel.waiting
                      ? "border-amber-300 bg-amber-50 text-amber-800"
                      : "border-[var(--aria-primary)] text-[var(--aria-primary)]",
                  ].join(" ")}
                >
                  {workItemStatusLabel.text}
                </span>
              ) : null}
              {card.kind === "work_item" &&
              card.raw.validator_findings?.some(
                (finding) => finding.code === "integration_or_e2e_skipped_risk",
              ) ? (
                <span className="rounded border border-rose-200 bg-rose-50 px-1.5 py-0.5 text-rose-700">
                  跳过贯通测试
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
  if (kind === "work_item_group") {
    return "Work Item Group";
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
    },
    story_spec: {
      label: "Story",
      token: "emerald",
      cardClassName: "border-emerald-200 border-l-emerald-500 bg-emerald-50/70",
      hoverClassName: "hover:border-emerald-300 hover:bg-emerald-50",
      iconClassName: "text-emerald-700",
      labelClassName: "border-emerald-200 bg-emerald-100 text-emerald-800",
    },
    design_spec: {
      label: "Design",
      token: "violet",
      cardClassName: "border-violet-200 border-l-violet-500 bg-violet-50/70",
      hoverClassName: "hover:border-violet-300 hover:bg-violet-50",
      iconClassName: "text-violet-700",
      labelClassName: "border-violet-200 bg-violet-100 text-violet-800",
    },
    work_item: {
      label: "Work Item",
      token: "amber",
      cardClassName: "border-amber-200 border-l-amber-500 bg-amber-50/70",
      hoverClassName: "hover:border-amber-300 hover:bg-amber-50",
      iconClassName: "text-amber-700",
      labelClassName: "border-amber-200 bg-amber-100 text-amber-900",
    },
    work_item_group: {
      label: "Work Item Group",
      token: "amber",
      cardClassName: "border-amber-200 border-l-amber-500 bg-amber-50/70",
      hoverClassName: "hover:border-amber-300 hover:bg-amber-50",
      iconClassName: "text-amber-700",
      labelClassName: "border-amber-200 bg-amber-100 text-amber-900",
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
    }
  >;

  return visuals[kind];
}

function workItemStatusBadge(
  card: LifecycleCardData,
  allWorkItems?: LifecycleWorkItem[],
): { text: string; waiting: boolean } | null {
  if (card.kind !== "work_item") {
    return null;
  }
  const waitingReason = allWorkItems
    ? workItemWaitingReason(card.raw, allWorkItems)
    : null;
  if (waitingReason) {
    return { text: waitingReason, waiting: true };
  }
  const latestAttempt = card.raw.latest_attempt;
  if (latestAttempt) {
    return { text: `${latestAttempt.status} · ${latestAttempt.stage}`, waiting: false };
  }
  if (card.raw.plan_status === "confirmed") {
    return { text: "可编码", waiting: false };
  }
  return null;
}

// F-26b：spec/plan 卡片的 workspace review 证据投影——timeline reviewer 节点
// 状态经后端 review_status 传入（running/completed），issue 工作台不进会话页
// 即可看到「review 已做/进行中」。issue/work_item 卡无此投影（零证据零展示）。
function workItemReviewStatusLabel(card: LifecycleCardData): string | null {
  if (
    card.kind !== "story_spec" &&
    card.kind !== "design_spec" &&
    card.kind !== "work_item_group"
  ) {
    return null;
  }
  if (card.raw.review_status === "running") {
    return "Review 进行中";
  }
  if (card.raw.review_status === "completed") {
    return "Review 已完成";
  }
  return null;
}
