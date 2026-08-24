// Task 4（容器组件）：workbench Issue 队列的分组容器。
// 消费 Task 1 的派生分组与 Task 3 的行组件：吸顶过滤条（受控输入，过滤本身在
// deriveIssueQueue 前置完成）、组折叠（collapsedGroups 受控，折叠态不渲染 rows）、
// 「显示更多（+N）」本地 state 记录已追加的组。零派生、零请求。
// 契约：外层必须是 <section role="region" aria-label="Issue 卡片列表">（既有 E2E/单测定位）。
import { useState } from "react";
import type { JSX } from "react";
import { ChevronDown, ChevronRight, Search } from "lucide-react";
import type {
  IssueQueueGroup,
  IssueQueueGroupKey,
} from "./issue-queue-derivation";
import { ISSUE_QUEUE_GROUP_ORDER } from "./issue-queue-derivation";
import { IssueQueueRow } from "./IssueQueueRow";

// 组名中文映射（与 Plan 逐字一致）。
const GROUP_LABEL: Record<IssueQueueGroupKey, string> = {
  needs_story: "待生成 Story",
  needs_design: "待生成 Design",
  needs_work_item: "待拆 Work Item",
  blocked: "阻塞",
  coding: "编码中",
  completed: "已完成",
};

export function IssueQueue(props: {
  groups: IssueQueueGroup[];
  focusedIssueId: string | null;
  collapsedGroups: IssueQueueGroupKey[];
  onToggleGroup: (key: IssueQueueGroupKey) => void;
  filterText: string;
  onFilterTextChange: (text: string) => void;
  onSelectIssue: (issueId: string) => void;
  onGenerateStorySpec: (issueId: string) => void;
  onDeleteIssue: (issueId: string) => void;
  deletingIssueId?: string | null;
}): JSX.Element {
  const {
    groups,
    focusedIssueId,
    collapsedGroups,
    onToggleGroup,
    filterText,
    onFilterTextChange,
    onSelectIssue,
    onGenerateStorySpec,
    onDeleteIssue,
    deletingIssueId = null,
  } = props;

  // 「显示更多」本地 state：记录已追加（点击过显示更多）的组，点击后该组入口消失。
  const [appendedGroups, setAppendedGroups] = useState<
    ReadonlySet<IssueQueueGroupKey>
  >(() => new Set());

  // 防御性排序：无论传入顺序如何，组一律按 ISSUE_QUEUE_GROUP_ORDER 渲染。
  const groupsByKey = new Map(groups.map((group) => [group.key, group]));
  const orderedGroups = ISSUE_QUEUE_GROUP_ORDER.flatMap((key) => {
    const group = groupsByKey.get(key);
    return group === undefined ? [] : [group];
  });

  // 总数 chip = 各组 total（过滤后的真实总数）之和。
  const totalCount = orderedGroups.reduce(
    (sum, group) => sum + group.total,
    0,
  );

  return (
    <section
      role="region"
      aria-label="Issue 卡片列表"
      className="flex min-h-0 flex-col overflow-hidden rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)]"
    >
      <div
        data-testid="issue-queue-header"
        className="sticky top-0 z-10 flex shrink-0 items-center gap-2 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-2"
      >
        <h2 className="shrink-0 text-sm font-semibold text-[var(--aria-ink)]">
          Issues
        </h2>
        <span
          data-testid="issue-queue-total-chip"
          className="shrink-0 rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]"
        >
          {totalCount}
        </span>
        <div className="relative ml-auto min-w-0 flex-1">
          <Search
            aria-hidden="true"
            className="pointer-events-none absolute top-1/2 left-2 h-3.5 w-3.5 -translate-y-1/2 text-[var(--aria-ink-muted)]"
          />
          <input
            type="text"
            value={filterText}
            aria-label="过滤 Issues"
            placeholder="过滤 Issues…"
            onChange={(event) => onFilterTextChange(event.target.value)}
            className="w-full cursor-text rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] py-1 pr-2 pl-7 text-xs text-[var(--aria-ink)] transition-colors duration-200 placeholder:text-[var(--aria-ink-muted)] focus-visible:border-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          />
        </div>
      </div>
      <div
        data-testid="issue-queue-group-list"
        className="min-h-0 flex-1 overflow-y-auto"
      >
        {orderedGroups.length === 0 ? (
          <p className="p-3 text-xs text-[var(--aria-ink-muted)]">
            没有匹配的 Issue。
          </p>
        ) : (
          orderedGroups.map((group) => {
            const collapsed = collapsedGroups.includes(group.key);
            const showMoreVisible =
              group.rows.length < group.total &&
              !appendedGroups.has(group.key);
            return (
              <div
                key={group.key}
                data-testid="issue-queue-group"
                data-group-key={group.key}
              >
                <button
                  type="button"
                  data-testid="issue-queue-group-header"
                  data-group-key={group.key}
                  aria-expanded={!collapsed}
                  onClick={() => onToggleGroup(group.key)}
                  className="flex w-full cursor-pointer items-center gap-1.5 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-1.5 text-left text-xs font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--aria-primary)]"
                >
                  {collapsed ? (
                    <ChevronRight
                      aria-hidden="true"
                      className="h-3.5 w-3.5 shrink-0 text-[var(--aria-ink-muted)]"
                    />
                  ) : (
                    <ChevronDown
                      aria-hidden="true"
                      className="h-3.5 w-3.5 shrink-0 text-[var(--aria-ink-muted)]"
                    />
                  )}
                  <span className="min-w-0 truncate">
                    {GROUP_LABEL[group.key]}
                  </span>
                  <span className="ml-auto shrink-0 font-mono text-[11px] font-normal text-[var(--aria-ink-muted)]">
                    {group.rows.length}/{group.total}
                  </span>
                </button>
                {collapsed ? null : (
                  <div
                    data-testid="issue-queue-group-rows"
                    data-group-key={group.key}
                  >
                    {group.rows.map((row) => (
                      <IssueQueueRow
                        key={row.issueId}
                        row={row}
                        focused={row.issueId === focusedIssueId}
                        onSelect={() => onSelectIssue(row.issueId)}
                        onGenerateStorySpec={() =>
                          onGenerateStorySpec(row.issueId)
                        }
                        onDelete={() => onDeleteIssue(row.issueId)}
                        deleting={row.issueId === deletingIssueId}
                      />
                    ))}
                    {showMoreVisible ? (
                      <button
                        type="button"
                        data-testid="issue-queue-show-more"
                        data-group-key={group.key}
                        onClick={() =>
                          setAppendedGroups((prev) => {
                            const next = new Set(prev);
                            next.add(group.key);
                            return next;
                          })
                        }
                        className="flex w-full cursor-pointer items-center justify-center border-b border-[var(--aria-line)] py-1.5 text-[11px] text-[var(--aria-ink-muted)] transition-colors duration-200 hover:bg-[var(--aria-panel)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-[var(--aria-primary)]"
                      >
                        显示更多（+{group.total - group.rows.length}）
                      </button>
                    ) : null}
                  </div>
                )}
              </div>
            );
          })
        )}
      </div>
    </section>
  );
}
