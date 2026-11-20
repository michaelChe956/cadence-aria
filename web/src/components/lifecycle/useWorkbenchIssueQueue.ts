// 从 IssueLifecycleWorkbench.tsx 拆出（large_file_guard 1200 行上限，纯移动零行为变化）：
// 双密度 Issue 队列状态——按 projectId 记忆的折叠/过滤/「已追加」组 Map（localStorage
// 回填）、队列分组派生值，以及折叠、过滤、显示更多与焦点可见性回调。
import { useMemo, useState } from "react";
import type { IssueLifecycleResponse, ProductIssue } from "../../api/types";
import {
  defaultCollapsedGroups,
  deriveIssueQueue,
  type IssueQueueGroupKey,
} from "./issue-queue-derivation";
import {
  queueCollapsedStorageKey,
  queueGroupsStorageKey,
  readStoredCollapsedGroups,
  readStoredQueueCollapsed,
  writeStoredValue,
} from "./IssueLifecycleWorkbenchStorage";

type Args = {
  selectedProjectId: string | null;
  lifecycles: IssueLifecycleResponse[];
  pendingIssues: ProductIssue[];
};

// 稳定空数组引用：避免每次 render 新建 [] 使 useMemo 依赖失效。
const EMPTY_GROUP_KEYS: IssueQueueGroupKey[] = [];

export function useWorkbenchIssueQueue({
  selectedProjectId,
  lifecycles,
  pendingIssues,
}: Args) {
  // Task 7：双密度外壳的队列状态。全部按 projectId 记忆，仅在首次遇到该 Project 时
  // 从 localStorage 回填；后续以内存 Map 为权威（2s 轮询不会重置这些 Map）。
  const [queueCollapsedByProject, setQueueCollapsedByProject] = useState<
    Record<string, boolean>
  >({});
  const [collapsedGroupsByProject, setCollapsedGroupsByProject] = useState<
    Record<string, IssueQueueGroupKey[]>
  >({});
  // 过滤文本与「已追加」组也按 projectId 隔离，避免切 Project 后沿用上一个的过滤态。
  const [queueFilterByProject, setQueueFilterByProject] = useState<
    Record<string, string>
  >({});
  const [showMoreGroupsByProject, setShowMoreGroupsByProject] = useState<
    Record<string, IssueQueueGroupKey[]>
  >({});

  // Task 7：队列派生与双密度状态的有效值。内存 Map 优先，缺失时从 localStorage 回填
  // （分组折叠缺省 defaultCollapsedGroups()）。轮询只写 lifecycles，不碰这些 Map。
  const queueCollapsed = selectedProjectId
    ? (queueCollapsedByProject[selectedProjectId] ??
      readStoredQueueCollapsed(selectedProjectId))
    : false;
  const collapsedQueueGroups = useMemo<IssueQueueGroupKey[]>(
    () =>
      selectedProjectId
        ? (collapsedGroupsByProject[selectedProjectId] ??
          readStoredCollapsedGroups(selectedProjectId))
        : defaultCollapsedGroups(),
    [collapsedGroupsByProject, selectedProjectId],
  );
  const queueFilterText = selectedProjectId
    ? (queueFilterByProject[selectedProjectId] ?? "")
    : "";
  const showMoreQueueGroups = selectedProjectId
    ? (showMoreGroupsByProject[selectedProjectId] ?? EMPTY_GROUP_KEYS)
    : EMPTY_GROUP_KEYS;
  // 「显示更多」受控实现（闭合 Task 4 评审 Important-1）：命中组用无上限 perGroupLimit
  // 重派生后替换该组 rows，于是 rows.length === total，入口自然消失。
  const issueQueueGroups = useMemo(() => {
    const baseGroups = deriveIssueQueue(lifecycles, {
      filterText: queueFilterText,
      pendingIssues,
    });
    if (showMoreQueueGroups.length === 0) {
      return baseGroups;
    }
    const expandedByKey = new Map(
      deriveIssueQueue(lifecycles, {
        filterText: queueFilterText,
        pendingIssues,
        perGroupLimit: Number.MAX_SAFE_INTEGER,
      }).map((group) => [group.key, group]),
    );
    return baseGroups.map((group) =>
      showMoreQueueGroups.includes(group.key)
        ? (expandedByKey.get(group.key) ?? group)
        : group,
    );
  }, [lifecycles, pendingIssues, queueFilterText, showMoreQueueGroups]);
  const queueTotalCount = issueQueueGroups.reduce(
    (sum, group) => sum + group.total,
    0,
  );

  // Task 7：折叠/展开仅切换密度——不动 focusedIssueId/selectedCardKey，不发请求。
  function handleToggleQueueCollapsed() {
    if (!selectedProjectId) {
      return;
    }
    const next = !queueCollapsed;
    setQueueCollapsedByProject((existing) => ({
      ...existing,
      [selectedProjectId]: next,
    }));
    writeStoredValue(
      queueCollapsedStorageKey(selectedProjectId),
      next ? "1" : "0",
    );
  }

  function handleToggleQueueGroup(key: IssueQueueGroupKey) {
    if (!selectedProjectId) {
      return;
    }
    const next = collapsedQueueGroups.includes(key)
      ? collapsedQueueGroups.filter((candidate) => candidate !== key)
      : [...collapsedQueueGroups, key];
    setCollapsedGroupsByProject((existing) => ({
      ...existing,
      [selectedProjectId]: next,
    }));
    writeStoredValue(
      queueGroupsStorageKey(selectedProjectId),
      JSON.stringify(next),
    );
  }

  function handleQueueFilterTextChange(text: string) {
    if (!selectedProjectId) {
      return;
    }
    setQueueFilterByProject((existing) => ({
      ...existing,
      [selectedProjectId]: text,
    }));
    // 过滤变化即重派生：「已追加」组复位（闭合 Task 4 评审 Important-2）。
    setShowMoreGroupsByProject((existing) =>
      (existing[selectedProjectId] ?? EMPTY_GROUP_KEYS).length === 0
        ? existing
        : { ...existing, [selectedProjectId]: [] },
    );
  }

  function handleShowMoreQueueGroup(key: IssueQueueGroupKey) {
    if (!selectedProjectId) {
      return;
    }
    setShowMoreGroupsByProject((existing) => {
      const current = existing[selectedProjectId] ?? EMPTY_GROUP_KEYS;
      if (current.includes(key)) {
        return existing;
      }
      return { ...existing, [selectedProjectId]: [...current, key] };
    });
  }

  // 焦点可见性（PR #92 三审 Important）：焦点行迁入折叠组或组渲染上限之外时，
  // 队列请求让该 Issue 可见。满足方式：行已在本组渲染 rows 内则只展开该组；不在
  // （被每组截断）则以无上限重派生定位其所在组，展开并把该组加入「显示更多」
  // 集合（无上限重派生覆盖任意下标）。行不在任何组（如被过滤移除）时无动作，
  // 队列自行清除待还原记录。
  function handleEnsureIssueVisible(issueId: string) {
    if (!selectedProjectId) {
      return;
    }
    const renderedGroup = issueQueueGroups.find((group) =>
      group.rows.some((row) => row.issueId === issueId),
    );
    const targetKey =
      renderedGroup?.key ??
      deriveIssueQueue(lifecycles, {
        filterText: queueFilterText,
        pendingIssues,
        perGroupLimit: Number.MAX_SAFE_INTEGER,
      }).find((group) => group.rows.some((row) => row.issueId === issueId))
        ?.key;
    if (targetKey === undefined) {
      return;
    }
    if (collapsedQueueGroups.includes(targetKey)) {
      // 折叠态下 toggle 即展开，与手动展开同一存储路径。
      handleToggleQueueGroup(targetKey);
    }
    if (renderedGroup === undefined) {
      handleShowMoreQueueGroup(targetKey);
    }
  }

  return {
    queueCollapsed,
    collapsedQueueGroups,
    queueFilterText,
    showMoreQueueGroups,
    issueQueueGroups,
    queueTotalCount,
    handleToggleQueueCollapsed,
    handleToggleQueueGroup,
    handleQueueFilterTextChange,
    handleShowMoreQueueGroup,
    handleEnsureIssueVisible,
  };
}
