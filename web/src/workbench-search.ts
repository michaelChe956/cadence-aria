/**
 * UI-1/UI-3 共享抽屉定位契约（2026-10-24 冻结，勿改字段名/语义）：
 * `/workbench` 的 search 字段全部可选——
 *
 *   focus     现有实体键（IssueLifecycleWorkbench 实体抽屉），语义不变
 *   overlay   抽屉种类："plan-overview"（UI-1 Coding Plan 总览抽屉）|
 *             "inbox"（UI-3 待处理抽屉）
 *   planId / attemptId / workItemId / gateId / choiceId  抽屉定位字段
 *
 * 硬规则：任何写入 /workbench search 的调用方必须经 mergeWorkbenchSearch
 * 与现有 search 合并，不得整体覆盖（此前 router 的 syncDrawerFocus 只写
 * {focus}，会抹掉抽屉定位字段）。跨路由进入 /workbench 时不要用 functional
 * search 合并——from 路由（如 coding 路由）的同名字段会串味，直接传显式
 * search 对象。
 */
export type WorkbenchOverlayKind = "plan-overview" | "inbox";

export type WorkbenchSearch = {
  focus?: string;
  overlay?: WorkbenchOverlayKind;
  planId?: string;
  attemptId?: string;
  workItemId?: string;
  gateId?: string;
  choiceId?: string;
};

export const WORKBENCH_SEARCH_KEYS = [
  "focus",
  "overlay",
  "planId",
  "attemptId",
  "workItemId",
  "gateId",
  "choiceId",
] as const satisfies readonly (keyof WorkbenchSearch)[];

/** 关闭抽屉时应清空的全部 overlay 定位字段（focus 保留）。 */
export const WORKBENCH_OVERLAY_SEARCH_KEYS = [
  "overlay",
  "planId",
  "attemptId",
  "workItemId",
  "gateId",
  "choiceId",
] as const satisfies readonly (keyof WorkbenchSearch)[];

export function searchParamString(value: unknown): string | undefined {
  return typeof value === "string" && value.length > 0 ? value : undefined;
}

/** /workbench validateSearch 实现：未知 overlay 值按缺省处理（不透传脏值）。 */
export function parseWorkbenchSearch(
  raw: Record<string, unknown>,
): WorkbenchSearch {
  const overlay = raw.overlay;
  const parsed: WorkbenchSearch = {
    focus: searchParamString(raw.focus),
    overlay:
      overlay === "plan-overview" || overlay === "inbox" ? overlay : undefined,
    planId: searchParamString(raw.planId),
    attemptId: searchParamString(raw.attemptId),
    workItemId: searchParamString(raw.workItemId),
    gateId: searchParamString(raw.gateId),
    choiceId: searchParamString(raw.choiceId),
  };
  for (const key of WORKBENCH_SEARCH_KEYS) {
    if (parsed[key] === undefined) {
      delete parsed[key];
    }
  }
  return parsed;
}

/**
 * 合并写入：patch 覆盖同名字段，patch 中显式 undefined 表示删除该字段，
 * 未提及的契约字段保留；previous 中契约之外的字段一律丢弃（防跨路由串味）。
 */
export function mergeWorkbenchSearch(
  previous: Partial<WorkbenchSearch> | undefined,
  patch: Partial<WorkbenchSearch>,
): WorkbenchSearch {
  const merged: WorkbenchSearch = {
    focus: "focus" in patch ? patch.focus : previous?.focus,
    overlay: "overlay" in patch ? patch.overlay : previous?.overlay,
    planId: "planId" in patch ? patch.planId : previous?.planId,
    attemptId: "attemptId" in patch ? patch.attemptId : previous?.attemptId,
    workItemId: "workItemId" in patch ? patch.workItemId : previous?.workItemId,
    gateId: "gateId" in patch ? patch.gateId : previous?.gateId,
    choiceId: "choiceId" in patch ? patch.choiceId : previous?.choiceId,
  };
  for (const key of WORKBENCH_SEARCH_KEYS) {
    if (merged[key] === undefined) {
      delete merged[key];
    }
  }
  return merged;
}

/** 关闭抽屉的 patch：清空全部 overlay 定位字段、保留 focus。 */
export function closeWorkbenchOverlayPatch(): Partial<WorkbenchSearch> {
  const patch: Partial<WorkbenchSearch> = {};
  for (const key of WORKBENCH_OVERLAY_SEARCH_KEYS) {
    patch[key] = undefined;
  }
  return patch;
}
