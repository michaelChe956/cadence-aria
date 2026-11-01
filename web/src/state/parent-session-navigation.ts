import type { PlanRepairSessionSnapshot } from "../api/types";
import type { TakeoverLink } from "./operation-audit-store";

export function parentSessionIdFor(
  planRepair: PlanRepairSessionSnapshot | null,
  takeoverLinks: Readonly<Record<string, TakeoverLink>>,
  sessionId: string,
): string | null {
  if (planRepair?.link.child_session_id === sessionId) {
    return planRepair.link.parent_session_id;
  }

  return (
    takeoverLinks[sessionId]?.parentSessionId ??
    sessionStorage.getItem(`aria.takeover-parent:${sessionId}`)
  );
}
