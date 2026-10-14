import { GitBranch } from "lucide-react";
import type { DeliveryEntryDto, IssueDeliverySummaryDto } from "../../api/types";
import { SectionCollapseToggle } from "./SectionCollapseToggle";

const OVERALL_LABELS: Record<IssueDeliverySummaryDto["overall"], string> = {
  all_pushed: "已全部交付",
  partial: "部分交付",
  none: "无 Work Item，不可交付",
};

const OVERALL_BADGE_CLASSES: Record<
  IssueDeliverySummaryDto["overall"],
  string
> = {
  all_pushed:
    "border-[var(--aria-success)] bg-[var(--aria-success-soft)] text-[var(--aria-success)]",
  partial:
    "border-[var(--aria-warning)] bg-[var(--aria-warning-soft)] text-[var(--aria-warning)]",
  none: "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] text-[var(--aria-ink-muted)]",
};

type DeliveryPushState = "pushed" | "pending" | "failed";

const PUSH_STATE_LABELS: Record<DeliveryPushState, string> = {
  pushed: "已推送",
  pending: "未推送",
  failed: "推送失败",
};

type DeliveryRepositoryCardData = {
  repositoryName: string;
  entries: DeliveryEntryDto[];
};

// owner 直批（2026-10-14）：后端 delivery_summary.entries 是 per-Work Item 投影，
// 同一仓库的多个 Work Item 会被渲染成多张同名仓卡（5 个 WI 时 frontend 重复两次）。
// 按仓库名去重为唯一仓卡，per-WI 事实下沉到副行（WI 数 + 推送状态分布）。
export function groupDeliveryEntriesByRepository(
  entries: DeliveryEntryDto[],
): DeliveryRepositoryCardData[] {
  const cards = new Map<string, DeliveryRepositoryCardData>();
  for (const entry of entries) {
    const existing = cards.get(entry.repository_name);
    if (existing) {
      existing.entries.push(entry);
    } else {
      cards.set(entry.repository_name, {
        repositoryName: entry.repository_name,
        entries: [entry],
      });
    }
  }
  return [...cards.values()];
}

function deliveryPushState(entry: DeliveryEntryDto): DeliveryPushState {
  if (entry.push_status === "pushed") {
    return "pushed";
  }
  return entry.push_status === "failed" ? "failed" : "pending";
}

// 仓卡状态取仓内最严重者：任一 WI 失败即失败；全部已推送才算已推送；其余未推送。
function repositoryPushState(entries: DeliveryEntryDto[]): DeliveryPushState {
  if (entries.some((entry) => deliveryPushState(entry) === "failed")) {
    return "failed";
  }
  return entries.every((entry) => deliveryPushState(entry) === "pushed")
    ? "pushed"
    : "pending";
}

function countByPushState(
  entries: DeliveryEntryDto[],
  state: DeliveryPushState,
): number {
  let count = 0;
  for (const entry of entries) {
    if (deliveryPushState(entry) === state) {
      count += 1;
    }
  }
  return count;
}

// 收起态的一眼可见摘要：仓数 + WI 数 + 未完成推送事实（不渲染仓卡也能判断进度）。
function collapsedSummaryText(cards: DeliveryRepositoryCardData[]): string {
  const entries = cards.flatMap((card) => card.entries);
  const segments = [
    `${cards.length} 个仓库`,
    `${entries.length} 个 Work Item`,
  ];
  const failed = countByPushState(entries, "failed");
  const pending = countByPushState(entries, "pending");
  if (failed > 0) {
    segments.push(`推送失败 ${failed}`);
  }
  if (pending > 0) {
    segments.push(`未推送 ${pending}`);
  }
  return segments.join(" · ");
}

export function DeliveryStatusPanel({
  summary,
  collapsed = false,
  onToggleCollapsed,
}: {
  summary: IssueDeliverySummaryDto;
  // 折叠受控值：仅工作台 issue 详情页传入（owner 要求默认收起）。不传
  // onToggleCollapsed 时不渲染折叠控件、常驻展开——「查看完整 Issue」抽屉等
  // 显式详情面保持原样。
  collapsed?: boolean;
  onToggleCollapsed?: () => void;
}) {
  const cards = groupDeliveryEntriesByRepository(summary.entries);
  const toggle = onToggleCollapsed;
  const foldable = toggle !== undefined && cards.length > 0;

  return (
    <section
      data-testid="delivery-status-panel"
      className="border-b border-[var(--aria-line)] px-4 py-3"
    >
      <div className="mb-2 flex flex-wrap items-center gap-2">
        <span
          data-testid="delivery-status-badge"
          data-status={summary.overall}
          className={`inline-flex items-center rounded border px-2 py-1 text-xs font-semibold ${OVERALL_BADGE_CLASSES[summary.overall]}`}
        >
          {OVERALL_LABELS[summary.overall]}
        </span>
        {foldable && toggle ? (
          <SectionCollapseToggle
            expanded={!collapsed}
            expandLabel={`展开交付明细（${cards.length} 个仓库）`}
            collapseLabel="收起交付明细"
            onToggle={toggle}
          />
        ) : null}
        {foldable && collapsed ? (
          <span
            data-testid="delivery-status-collapsed-summary"
            className="text-xs text-[var(--aria-ink-muted)]"
          >
            {collapsedSummaryText(cards)}
          </span>
        ) : null}
      </div>
      {summary.overall !== "none" && cards.length > 0 && (!foldable || !collapsed) ? (
        <div className="space-y-2">
          {cards.map((card) => (
            <DeliveryRepositoryCard
              key={card.repositoryName}
              card={card}
            />
          ))}
        </div>
      ) : null}
    </section>
  );
}

function DeliveryRepositoryCard({
  card,
}: {
  card: DeliveryRepositoryCardData;
}) {
  const state = repositoryPushState(card.entries);
  const failed = state === "failed";
  const pushed = state === "pushed";

  return (
    <div
      data-testid="delivery-entry-row"
      data-status={state}
      data-work-item-count={card.entries.length}
      className={`rounded-md border px-2 py-2 text-xs ${
        failed
          ? "border-[var(--aria-danger)] bg-[var(--aria-danger-soft)]"
          : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
      }`}
    >
      <div className="flex items-center justify-between gap-2">
        <span className="font-semibold text-[var(--aria-ink)]">
          {card.repositoryName}
        </span>
        <span
          className={
            failed
              ? "font-semibold text-[var(--aria-danger)]"
              : pushed
                ? "font-semibold text-[var(--aria-success)]"
                : "text-[var(--aria-ink-muted)]"
          }
        >
          {PUSH_STATE_LABELS[state]}
        </span>
      </div>
      {card.entries.length > 1 ? (
        <div
          data-testid="delivery-entry-work-item-count"
          className="mt-0.5 text-[11px] text-[var(--aria-ink-muted)]"
        >
          {card.entries.length} 个 Work Item
          {" · "}
          {pushStateBreakdown(card.entries)}
        </div>
      ) : null}
      {card.entries.map((entry) => (
        <div
          key={entry.work_item_id}
          data-testid="delivery-entry-branch"
          className="mt-1 flex items-center gap-2 text-[var(--aria-ink-muted)]"
        >
          <GitBranch className="h-3.5 w-3.5 shrink-0" />
          <span className="truncate">{entry.branch_name ?? "—"}</span>
          {entry.commit_sha ? (
            <span className="shrink-0 font-mono text-[11px]">
              {entry.commit_sha.slice(0, 7)}
            </span>
          ) : null}
        </div>
      ))}
      {card.entries
        .filter((entry) => entry.push_status === "failed" && entry.push_error)
        .map((entry) => (
          <div
            key={entry.work_item_id}
            data-testid="delivery-entry-error"
            className="mt-1 text-[var(--aria-danger)]"
          >
            {entry.push_error}
          </div>
        ))}
    </div>
  );
}

// 多 WI 仓卡的副行明细：只列出实际出现的推送状态与计数（保持副行短小）。
function pushStateBreakdown(entries: DeliveryEntryDto[]): string {
  const order: DeliveryPushState[] = ["pushed", "pending", "failed"];
  return order
    .map((state) => ({ state, count: countByPushState(entries, state) }))
    .filter((item) => item.count > 0)
    .map((item) => `${PUSH_STATE_LABELS[item.state]} ${item.count}`)
    .join(" · ");
}
