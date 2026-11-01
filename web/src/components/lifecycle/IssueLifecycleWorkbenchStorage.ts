import {
  defaultCollapsedGroups,
  ISSUE_QUEUE_GROUP_ORDER,
  type IssueQueueGroupKey,
} from "./issue-queue-derivation";

export function queueCollapsedStorageKey(projectId: string) {
  return `aria.workbench.queueCollapsed.${projectId}`;
}

export function queueGroupsStorageKey(projectId: string) {
  return `aria.workbench.groups.${projectId}`;
}

export function lcSummaryStorageKey(projectId: string) {
  return `aria.workbench.lcSummary.${projectId}`;
}

// localStorage 不可用（隐私模式/配额超限）时静默降级为仅内存记忆。
export function readStoredValue(key: string): string | null {
  try {
    return window.localStorage.getItem(key);
  } catch {
    return null;
  }
}

export function writeStoredValue(key: string, value: string) {
  try {
    window.localStorage.setItem(key, value);
  } catch {
    /* localStorage 不可用：静默降级 */
  }
}

export function readStoredQueueCollapsed(projectId: string): boolean {
  return readStoredValue(queueCollapsedStorageKey(projectId)) === "1";
}

export function readStoredLcSummaryExpanded(projectId: string): boolean {
  return readStoredValue(lcSummaryStorageKey(projectId)) === "1";
}

export function readStoredCollapsedGroups(projectId: string): IssueQueueGroupKey[] {
  const raw = readStoredValue(queueGroupsStorageKey(projectId));
  if (raw === null) {
    return defaultCollapsedGroups();
  }
  try {
    const parsed: unknown = JSON.parse(raw);
    if (!Array.isArray(parsed)) {
      return defaultCollapsedGroups();
    }
    return parsed.filter((value): value is IssueQueueGroupKey =>
      ISSUE_QUEUE_GROUP_ORDER.includes(value as IssueQueueGroupKey),
    );
  } catch {
    return defaultCollapsedGroups();
  }
}
