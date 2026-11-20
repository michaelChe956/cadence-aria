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
import { useLayoutEffect, useRef, useState } from "react";
import type { FocusEvent, JSX } from "react";
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
  unloaded: "生命周期待查看",
};

// PR #92 三审 Important（并发删除）：行 busy 需覆盖并发在途的全部 Issue——
// deletingIssueIds 是在途 issueId 集合（受控，父层派生），标量 deletingIssueId
// 仅为单删除旧通道保留（直接渲染队列的既有测试在用），两者取并集。

// 迁组行的焦点还原目标：按 data-issue-id 定位行，再取标题 span 所在按钮。
function findRowTitleButton(
  root: HTMLElement,
  issueId: string,
): HTMLButtonElement | null {
  const rows = root.querySelectorAll<HTMLElement>(
    '[data-testid="issue-queue-row"]',
  );
  for (const row of rows) {
    if (row.getAttribute("data-issue-id") !== issueId) {
      continue;
    }
    const title = row.querySelector<HTMLElement>(
      '[data-testid="issue-queue-row-title"]',
    );
    const button = title?.closest("button");
    return button instanceof HTMLButtonElement ? button : null;
  }
  return null;
}

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
  // 并发删除在途的 issueId 集合：B 发起不得解除 A 行的在途禁用。
  deletingIssueIds?: ReadonlySet<string> | null;
  loading?: boolean;
  // 受控「显示更多」：父层收到组 key 后以更高 perGroupLimit 重派生该组 rows。
  onShowMoreGroup?: (key: IssueQueueGroupKey) => void;
  // 焦点可见性：行迁入折叠组/超出组渲染上限时请求父层让该 Issue 可见（展开所在
  // 组、提升该组渲染上限），随后队列照常还原焦点；父层无法使其可见时清除待还原
  // 记录，不抢焦点。
  onEnsureIssueVisible?: (issueId: string) => void;
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
    deletingIssueIds = null,
    loading = false,
    onShowMoreGroup,
    onEnsureIssueVisible,
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

  // 焦点保持（迁组场景）：行迁组（如 unloaded → 阶段组）时行换父容器，React 无法跨父
  // 复用 DOM 节点，被销毁的标题按钮会把键盘焦点丢给 <body>。行 key 已是 issueId（身份
  // 稳定），但 key 相同不足以跨父保住节点，因此显式还原：借 focus 冒泡记录最近在行内
  // 获得焦点的元素；每次提交后若该节点确已被本次渲染移除且行仍在（换了组），把焦点
  // 还原到该行标题按钮。未迁组 / 行被移除 / 焦点已被其他元素接管时均不动作。
  // 焦点还原意图：还原只修复「迁组本身造成的焦点丢失」。若用户已主动把焦点移出队列
  // （focusout 的下一落点不在队列区域内——含点击非聚焦空白区域后落回 <body>），迁组
  // 不得把焦点抢回，因此在 focusout 时即丢弃记录，而非等提交后再判断。
  const sectionRef = useRef<HTMLElement | null>(null);
  const lastRowFocusRef = useRef<{
    issueId: string;
    element: Element;
    visibilityRequested: boolean;
  } | null>(null);

  const handleSectionFocus = (event: FocusEvent<HTMLElement>) => {
    const row =
      event.target instanceof Element
        ? event.target.closest('[data-testid="issue-queue-row"]')
        : null;
    lastRowFocusRef.current =
      row === null
        ? null
        : {
            issueId: row.getAttribute("data-issue-id") ?? "",
            element: event.target,
            visibilityRequested: false,
          };
  };

  // focusout 冒泡：焦点离开行内元素时看下一落点。仍在队列区域内（行间移动、移向
  // 过滤输入框/组头）则不动记录，交给随后的 focus 冒泡刷新；落在队列外或 <body>
  // （relatedTarget 为空）说明用户主动离开，丢弃待还原记录。
  const handleSectionBlur = (event: FocusEvent<HTMLElement>) => {
    if (!(event.target instanceof Element)) {
      return;
    }
    if (
      event.target.closest('[data-testid="issue-queue-row"]') === null
    ) {
      return;
    }
    const next = event.relatedTarget;
    if (next instanceof Node && sectionRef.current?.contains(next)) {
      return;
    }
    lastRowFocusRef.current = null;
  };

  useLayoutEffect(() => {
    const prev = lastRowFocusRef.current;
    if (prev === null) {
      return;
    }
    // 原焦点节点仍在文档中：本次提交没有销毁它，焦点要么仍在原处，要么已由用户
    // 移走（focus 事件会刷新记录），交给默认行为。
    if (prev.element.isConnected) {
      return;
    }
    const active = document.activeElement;
    if (active !== null && active !== document.body && active.isConnected) {
      // 焦点已被其他元素接管（如父层焦点管理），不抢占。
      lastRowFocusRef.current = null;
      return;
    }
    const root = sectionRef.current;
    if (root === null) {
      lastRowFocusRef.current = null;
      return;
    }
    const titleButton = findRowTitleButton(root, prev.issueId);
    if (titleButton !== null) {
      lastRowFocusRef.current = null;
      titleButton.focus();
      return;
    }
    // 行未被渲染：迁入折叠组（折叠态不渲染 rows）或落在组渲染上限之外（父层截断）。
    // 请父层让该 Issue 可见——展开所在组、提升该组渲染上限；仅请求一次，父层处理后
    // 的下一轮提交内行已可见则照常还原焦点，仍不可见则清除待还原记录，不抢焦点。
    if (!prev.visibilityRequested && onEnsureIssueVisible != null) {
      lastRowFocusRef.current = { ...prev, visibilityRequested: true };
      onEnsureIssueVisible(prev.issueId);
      return;
    }
    lastRowFocusRef.current = null;
  });

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
      ref={sectionRef}
      onFocus={handleSectionFocus}
      onBlur={handleSectionBlur}
      role="region"
      aria-label="Issue 卡片列表"
      aria-busy={loading || undefined}
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
            {loading ? "正在加载议题…" : "没有匹配的 Issue。"}
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
                        // unloaded 行（详情未到）：删除不依赖生命周期详情，保持可用；
                        // 生成 Story 的幂等性依赖是否已有 Story，未知时不得放行
                        // （OpenSpec workbench-progressive-issue-loading）。
                        onGenerateStorySpec={row.group === "unloaded" ? undefined : () => onGenerateStorySpec(row.issueId)}
                        onDelete={() => onDeleteIssue(row.issueId)}
                        // 并发删除：busy 判定取「在途 issueId 集合 ∪ 标量
                        // deletingIssueId」，B 发起不得解除 A 的在途禁用。
                        deleting={
                          (deletingIssueIds?.has(row.issueId) ?? false) ||
                          row.issueId === deletingIssueId
                        }
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
