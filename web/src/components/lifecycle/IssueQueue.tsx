// Task 4（容器组件）：workbench Issue 队列的分组容器。
// 消费 Task 1 的派生分组与 Task 3 的行组件：吸顶过滤条（受控输入，过滤本身在
// deriveIssueQueue 前置完成）、组折叠（collapsedGroups 受控，折叠态不渲染 rows）、
// 「显示更多（+N）」。零派生、零请求。
// Task 7 修订（闭合 Task 4 评审 2 个 Important）：
// 1. 传入 onShowMoreGroup 时进入受控模式——点击只上报组 key，由父层用更高 perGroupLimit
//    重派生该组 rows；入口的显示/隐藏完全由 rows.length < total 决定，不再依赖本地 state。
// 2. 未传 onShowMoreGroup 的非受控回退模式下，本地「已追加」state 随 filterText 变化复位，
//    避免过滤重派生后入口被旧标记永久隐藏。
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
  // 受控「显示更多」：父层收到组 key 后以更高 perGroupLimit 重派生该组 rows。
  onShowMoreGroup?: (key: IssueQueueGroupKey) => void;
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
    onShowMoreGroup,
  } = props;

  // 非受控回退模式的「已追加」本地 state：与产生它的 filterText 绑定，
  // filterText 变化即视为复位（render 阶段调整，React 推荐写法）。
  const [appended, setAppended] = useState<{
    filterText: string;
    keys: ReadonlySet<IssueQueueGroupKey>;
  }>(() => ({ filterText, keys: new Set() }));
  if (appended.filterText !== filterText) {
    setAppended({ filterText, keys: new Set() });
  }
  const appendedGroups =
    appended.filterText === filterText
      ? appended.keys
      : (new Set<IssueQueueGroupKey>() as ReadonlySet<IssueQueueGroupKey>);

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
            // 受控模式：入口只看 rows 是否已补齐；非受控模式：额外看本地已追加标记。
            const showMoreVisible =
              group.rows.length < group.total &&
              (onShowMoreGroup !== undefined ||
                !appendedGroups.has(group.key));
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
                        onClick={() => {
                          if (onShowMoreGroup) {
                            onShowMoreGroup(group.key);
                            return;
                          }
                          setAppended((prev) => {
                            const next = new Set(prev.keys);
                            next.add(group.key);
                            return { filterText, keys: next };
                          });
                        }}
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
