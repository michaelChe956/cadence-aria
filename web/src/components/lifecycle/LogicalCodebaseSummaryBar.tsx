// Task 8（展示组件）：逻辑代码库运维摘要条 —— 主工作区顶部的一行状态摘要。
// 纯展示：摘要字段（LC 名 / 索引状态 / 发布状态 / 是否异常）与展开态全部由父层派生，
// 本组件不做任何派生或请求，展开后的完整面板由父层渲染既有 LogicalCodebaseManagementPanel。
// 视觉规范：一行 flex、状态一律胶囊 chip、异常 amber（索引）/ rose（发布）警示、
// hover 仅改颜色/边框（不位移）、过渡 200ms + motion-reduce 降级、cursor-pointer、focus-visible 主色 ring。
import { ChevronDown, ChevronUp, TriangleAlert } from "lucide-react";
import type { JSX } from "react";

export type LogicalCodebaseSummary = {
  lcName: string | null;
  indexState: string | null;
  publicationStatus: string | null;
  hasWarning: boolean;
};

// 索引状态中文标签（与 AggregateIndexCard 语义一致，摘要条用更短的措辞）。
const INDEX_STATE_LABEL: Record<string, string> = {
  active: "可用",
  stale: "已过期",
  degraded: "降级",
  rebuilding: "重建中",
  missing: "未建立",
};

// 发布状态中文标签（与 PointerPublicationPanel 语义一致）。
const PUBLICATION_STATUS_LABEL: Record<string, string> = {
  in_progress: "发布中",
  completed_all: "全部完成",
  completed_partial: "部分失败",
  revoked: "已撤回",
};

const CHIP_BASE =
  "inline-flex h-6 shrink-0 items-center rounded-full border px-2 text-[11px] font-semibold";
const CHIP_NEUTRAL =
  "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]";

// 索引：仅 active 视为正常，其余（含未建立）用 amber 警示。
function indexChipClass(indexState: string | null): string {
  if (indexState === "active") {
    return "border-emerald-300 bg-emerald-50 text-emerald-700";
  }
  if (indexState === null) {
    return CHIP_NEUTRAL;
  }
  return "border-amber-300 bg-amber-50 text-amber-700";
}

// 发布：failed/partial 用 rose 强调，全部完成用 emerald，其余中性。
function publicationChipClass(publicationStatus: string | null): string {
  if (publicationStatus === null) {
    return CHIP_NEUTRAL;
  }
  if (
    publicationStatus.includes("failed") ||
    publicationStatus.includes("partial")
  ) {
    return "border-rose-300 bg-rose-50 text-rose-700";
  }
  if (publicationStatus === "completed_all") {
    return "border-emerald-300 bg-emerald-50 text-emerald-700";
  }
  return CHIP_NEUTRAL;
}

export function LogicalCodebaseSummaryBar(props: {
  summary: LogicalCodebaseSummary;
  expanded: boolean;
  onToggle: () => void;
}): JSX.Element {
  const { summary, expanded, onToggle } = props;
  const { lcName, indexState, publicationStatus, hasWarning } = summary;
  const ToggleIcon = expanded ? ChevronUp : ChevronDown;

  return (
    <div
      data-testid="lc-summary-bar"
      data-warning={hasWarning ? "true" : "false"}
      className={[
        "flex min-w-0 items-center gap-2 rounded-md border px-3 py-1.5",
        "transition-colors duration-200 motion-reduce:transition-none",
        hasWarning
          ? "border-amber-300 bg-amber-50"
          : "border-[var(--aria-line)] bg-[var(--aria-panel)]",
      ].join(" ")}
    >
      <span className="shrink-0 text-xs font-semibold text-[var(--aria-ink)]">
        逻辑代码库
      </span>
      <span
        data-testid="lc-summary-name"
        className={[
          CHIP_BASE,
          "max-w-40 truncate",
          lcName
            ? "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink)]"
            : CHIP_NEUTRAL,
        ].join(" ")}
      >
        {lcName ?? "未选择"}
      </span>
      <span
        data-testid="lc-summary-index"
        data-state={indexState ?? "unknown"}
        className={[CHIP_BASE, indexChipClass(indexState)].join(" ")}
      >
        索引 {indexState === null ? "未建立" : (INDEX_STATE_LABEL[indexState] ?? indexState)}
      </span>
      <span
        data-testid="lc-summary-publication"
        data-status={publicationStatus ?? "none"}
        className={[CHIP_BASE, publicationChipClass(publicationStatus)].join(
          " ",
        )}
      >
        发布{" "}
        {publicationStatus === null
          ? "未发布"
          : (PUBLICATION_STATUS_LABEL[publicationStatus] ?? publicationStatus)}
      </span>
      {hasWarning ? (
        <span
          data-testid="lc-summary-warning"
          className="inline-flex shrink-0 items-center gap-1 text-[11px] font-semibold text-amber-800"
        >
          <TriangleAlert className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
          需处理
        </span>
      ) : null}
      <button
        type="button"
        data-testid="lc-summary-toggle"
        aria-expanded={expanded}
        aria-label={expanded ? "折叠逻辑代码库管理" : "展开逻辑代码库管理"}
        onClick={onToggle}
        className={[
          "ml-auto inline-flex h-7 shrink-0 cursor-pointer items-center gap-1 rounded-md border px-2 text-[11px] font-semibold",
          "border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)]",
          "transition-colors duration-200 motion-reduce:transition-none",
          "hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)]",
          "focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]",
        ].join(" ")}
      >
        <ToggleIcon className="h-3.5 w-3.5 shrink-0" aria-hidden="true" />
        {expanded ? "收起" : "管理"}
      </button>
    </div>
  );
}
