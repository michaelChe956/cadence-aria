import { useState } from "react";
import type {
  LinkedWorkspaceAmendmentTarget,
  LinkedWorkspaceSessionSnapshot,
} from "../../api/types";
import type { LinkedWorkspaceAmendmentStatus } from "../../state/linked-workspace-amendment-store";

export type LinkedWorkspaceAmendmentTargets = {
  story: string[];
  design: string[];
};

export function LinkedWorkspaceAmendmentSelector({
  parentSessionId,
  targets,
  status = "idle",
  snapshot = null,
  error = null,
  disabled = false,
  onStart,
}: {
  parentSessionId: string;
  targets: LinkedWorkspaceAmendmentTargets;
  status?: LinkedWorkspaceAmendmentStatus;
  snapshot?: LinkedWorkspaceSessionSnapshot | null;
  error?: string | null;
  disabled?: boolean;
  onStart?: (target: LinkedWorkspaceAmendmentTarget) => boolean;
}) {
  const initialType = targets.story.length > 0 ? "story" : "design";
  const [workspaceType, setWorkspaceType] = useState<"story" | "design">(
    initialType,
  );
  const [entityId, setEntityId] = useState("");
  const targetOptions = targets[workspaceType];
  const selectedEntityId = targetOptions.includes(entityId)
    ? entityId
    : targetOptions[0] ?? "";
  const pending = status === "pending";
  const canStart = !disabled && !pending && Boolean(selectedEntityId) && Boolean(onStart);
  const safeSnapshot = authoritativeReadySnapshot(
    parentSessionId,
    status,
    snapshot,
  );

  return (
    <fieldset className="mb-3 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
      <legend className="px-1 text-xs font-semibold text-[var(--aria-ink)]">
        Story/Design 修订目标
      </legend>
      <div className="grid gap-3 sm:grid-cols-2">
        <label className="grid gap-1 text-xs font-medium text-[var(--aria-ink-muted)]">
          修订类型
          <select
            value={workspaceType}
            disabled={disabled || pending}
            onChange={(event) => {
              const nextType = event.target.value as "story" | "design";
              setWorkspaceType(nextType);
              setEntityId(targets[nextType][0] ?? "");
            }}
            className="h-9 rounded-md border border-[var(--aria-line)] bg-white px-2 text-sm text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            <option value="story">Story</option>
            <option value="design">Design</option>
          </select>
        </label>
        <label className="grid gap-1 text-xs font-medium text-[var(--aria-ink-muted)]">
          修订目标
          <select
            value={selectedEntityId}
            disabled={disabled || pending || targetOptions.length === 0}
            onChange={(event) => setEntityId(event.target.value)}
            className="h-9 rounded-md border border-[var(--aria-line)] bg-white px-2 font-mono text-sm text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            {targetOptions.length === 0 ? (
              <option value="">当前执行计划没有可修订目标</option>
            ) : null}
            {targetOptions.map((target) => (
              <option key={target} value={target}>
                {target}
              </option>
            ))}
          </select>
        </label>
      </div>
      <div className="mt-3 flex min-w-0 flex-wrap items-center justify-between gap-2">
        <div className="min-w-0 text-xs text-[var(--aria-ink-muted)]">
          {pending ? <span role="status">正在创建关联 Child Workspace。</span> : null}
          {status === "error" && error ? (
            <span role="alert" className="text-[var(--aria-danger)]">
              {error}
            </span>
          ) : null}
          {safeSnapshot ? (
            <a
              href={`/workbench/workspace/${encodeURIComponent(
                safeSnapshot.link.child_session_id,
              )}`}
              target="_blank"
              rel="noreferrer"
              className="font-semibold text-[var(--aria-primary)] underline-offset-2 hover:underline focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
            >
              打开已创建的 {safeSnapshot.workspace_type === "story" ? "Story" : "Design"} Workspace
            </a>
          ) : null}
        </div>
        <button
          type="button"
          disabled={!canStart}
          onClick={() => {
            if (!selectedEntityId) return;
            onStart?.({
              entity_id: selectedEntityId,
              workspace_type: workspaceType,
              relation:
                workspaceType === "story"
                  ? "story_amendment"
                  : "design_amendment",
            });
          }}
          className="h-9 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:cursor-not-allowed disabled:opacity-50"
        >
          发起关联修订
        </button>
      </div>
    </fieldset>
  );
}

function authoritativeReadySnapshot(
  parentSessionId: string,
  status: LinkedWorkspaceAmendmentStatus,
  snapshot: LinkedWorkspaceSessionSnapshot | null,
): LinkedWorkspaceSessionSnapshot | null {
  if (
    status !== "ready" ||
    !snapshot ||
    !snapshot.link.id.trim() ||
    !snapshot.link.child_session_id.trim() ||
    snapshot.link.parent_session_id !== parentSessionId ||
    snapshot.link.return_context.original_route !==
      `/workbench/workspace/${parentSessionId}`
  ) {
    return null;
  }
  const relationMatches =
    (snapshot.workspace_type === "story" &&
      snapshot.link.relation === "story_amendment") ||
    (snapshot.workspace_type === "design" &&
      snapshot.link.relation === "design_amendment");
  return relationMatches ? snapshot : null;
}
