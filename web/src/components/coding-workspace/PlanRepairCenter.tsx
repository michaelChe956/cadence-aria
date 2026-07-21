import { useRef, useState } from "react";
import type {
  LinkedWorkspaceAmendmentTarget,
  LinkedWorkspaceSessionSnapshot,
} from "../../api/types";
import type { LinkedWorkspaceAmendmentStatus } from "../../state/linked-workspace-amendment-store";
import type { PlanRepairSessionState } from "../../state/plan-repair-session";
import { WorkItemPlanOverview } from "../workspace/WorkItemPlanOverview";
import { ImpactPreview } from "./ImpactPreview";
import {
  LinkedWorkspaceAmendmentSelector,
  type LinkedWorkspaceAmendmentTargets,
} from "./LinkedWorkspaceAmendmentSelector";
import { SemanticContractDiff } from "./SemanticContractDiff";

export type PlanRepairTab =
  | "summary"
  | "contract_diff"
  | "coder_diff"
  | "reviewer_diff"
  | "impact"
  | "evidence";

export type PlanRepairAction =
  | "confirm"
  | "regenerate"
  | "adjust_scope"
  | "cancel"
  | "open_workspace";

const TABS: Array<{ id: PlanRepairTab; label: string }> = [
  { id: "summary", label: "修订摘要" },
  { id: "contract_diff", label: "Contract Diff" },
  { id: "coder_diff", label: "Coder Diff" },
  { id: "reviewer_diff", label: "Reviewer Diff" },
  { id: "impact", label: "Impact" },
  { id: "evidence", label: "Evidence" },
];

export function PlanRepairCenter({
  state,
  onAction,
  actionsDisabled = false,
  actionPending = false,
  actionStatus = null,
  linkedAmendmentTargets = { story: [], design: [] },
  linkedAmendmentStatus = "idle",
  linkedAmendmentSnapshot = null,
  linkedAmendmentError = null,
  onStartLinkedAmendment,
}: {
  state: PlanRepairSessionState;
  onAction?: (action: PlanRepairAction) => void;
  actionsDisabled?: boolean;
  actionPending?: boolean;
  actionStatus?: string | null;
  linkedAmendmentTargets?: LinkedWorkspaceAmendmentTargets;
  linkedAmendmentStatus?: LinkedWorkspaceAmendmentStatus;
  linkedAmendmentSnapshot?: LinkedWorkspaceSessionSnapshot | null;
  linkedAmendmentError?: string | null;
  onStartLinkedAmendment?: (target: LinkedWorkspaceAmendmentTarget) => boolean;
}) {
  const [activeTab, setActiveTab] = useState<PlanRepairTab>("summary");
  const [scopeOpen, setScopeOpen] = useState(false);
  const tabRefs = useRef<Array<HTMLButtonElement | null>>([]);
  const canMutate =
    !actionsDisabled &&
    !actionPending &&
    onAction !== undefined &&
    state.stage === "awaiting_confirmation" &&
    state.amendment !== null;
  const canConfirm = canMutate && state.projection !== null;

  function selectTab(index: number) {
    const tab = TABS[index];
    if (!tab) return;
    setActiveTab(tab.id);
    tabRefs.current[index]?.focus();
  }

  return (
    <section
      data-testid="plan-repair-center"
      className="flex h-full min-h-0 min-w-0 flex-col bg-[var(--aria-panel)]"
      aria-labelledby="plan-repair-title"
    >
      <header className="shrink-0 border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-4 py-3">
        <div className="flex min-w-0 flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <p className="text-xs font-semibold uppercase tracking-wide text-[var(--aria-ink-muted)]">
              Coding Workspace 内嵌修订
            </p>
            <h2 id="plan-repair-title" className="mt-1 text-lg font-semibold text-[var(--aria-ink)]">
              Plan Repair
            </h2>
            <p className="mt-1 max-w-3xl text-sm leading-6 text-[var(--aria-ink-muted)]">
              {state.request.evidence.at(0)?.message ?? state.request.reason_code}
            </p>
          </div>
          <span className="rounded border border-[var(--aria-line)] bg-white px-2 py-1 font-mono text-xs text-[var(--aria-ink-muted)]">
            {state.stage}
          </span>
        </div>
      </header>

      <div className="shrink-0 border-b border-[var(--aria-line)] bg-white px-3 py-2">
        <div
          role="tablist"
          aria-label="Plan Repair 视图"
          className="flex min-w-0 gap-1 overflow-x-auto"
        >
          {TABS.map((tab, index) => (
            <button
              key={tab.id}
              ref={(node) => {
                tabRefs.current[index] = node;
              }}
              id={`plan-repair-tab-${tab.id}`}
              type="button"
              role="tab"
              aria-selected={activeTab === tab.id}
              aria-controls={
                activeTab === tab.id ? `plan-repair-panel-${activeTab}` : undefined
              }
              tabIndex={activeTab === tab.id ? 0 : -1}
              onClick={() => setActiveTab(tab.id)}
              onKeyDown={(event) => {
                if (event.key === "ArrowRight") {
                  event.preventDefault();
                  selectTab((index + 1) % TABS.length);
                } else if (event.key === "ArrowLeft") {
                  event.preventDefault();
                  selectTab((index - 1 + TABS.length) % TABS.length);
                } else if (event.key === "Home") {
                  event.preventDefault();
                  selectTab(0);
                } else if (event.key === "End") {
                  event.preventDefault();
                  selectTab(TABS.length - 1);
                }
              }}
              className={[
                "h-9 shrink-0 rounded px-3 text-xs font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]",
                activeTab === tab.id
                  ? "bg-[var(--aria-primary-soft)] text-[var(--aria-primary)]"
                  : "text-[var(--aria-ink-muted)] hover:bg-[var(--aria-panel-muted)] hover:text-[var(--aria-ink)]",
              ].join(" ")}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>

      <div
        id={`plan-repair-panel-${activeTab}`}
        role="tabpanel"
        aria-labelledby={`plan-repair-tab-${activeTab}`}
        className="min-h-0 flex-1 overflow-auto bg-[var(--aria-bg)] p-4"
      >
        {activeTab === "summary" && state.projection ? (
          <div className="space-y-4">
            <WorkItemPlanOverview
              projection={state.projection.human_group_projection}
              presentation={null}
              planProjectionBundleId={state.projection.id}
            />
            <ImpactPreview
              amendment={state.amendment}
              impact={state.impact}
              projection={state.projection}
            />
          </div>
        ) : null}
        {activeTab === "summary" && !state.projection ? (
          <p className="rounded-md border border-[var(--aria-line)] bg-white p-4 text-sm text-[var(--aria-ink-muted)]">
            正在生成 Human Projection。
          </p>
        ) : null}
        {activeTab === "contract_diff" ? (
          <SemanticContractDiff
            amendment={state.amendment}
            projection={state.projection}
            impact={state.impact}
            view="contract"
          />
        ) : null}
        {activeTab === "coder_diff" ? (
          <SemanticContractDiff
            amendment={state.amendment}
            projection={state.projection}
            impact={state.impact}
            view="coder"
          />
        ) : null}
        {activeTab === "reviewer_diff" ? (
          <SemanticContractDiff
            amendment={state.amendment}
            projection={state.projection}
            impact={state.impact}
            view="reviewer"
          />
        ) : null}
        {activeTab === "impact" ? (
          <ImpactPreview
            amendment={state.amendment}
            impact={state.impact}
            projection={state.projection}
          />
        ) : null}
        {activeTab === "evidence" ? (
          <div className="space-y-3">
            <section className="rounded-md border border-[var(--aria-line)] bg-white p-4">
              <h3 className="text-sm font-semibold text-[var(--aria-ink)]">触发证据</h3>
              <ul className="mt-2 space-y-2 text-sm text-[var(--aria-ink-muted)]">
                {state.request.evidence.map((evidence) => (
                  <li key={`${evidence.kind}-${evidence.source_ref}`}>
                    <div className="font-medium text-[var(--aria-ink)]">{evidence.message}</div>
                    <div className="mt-0.5 break-all font-mono text-xs">
                      {evidence.source_ref}
                    </div>
                  </li>
                ))}
              </ul>
            </section>
            <section className="rounded-md border border-[var(--aria-line)] bg-white p-4">
              <h3 className="text-sm font-semibold text-[var(--aria-ink)]">Revision History</h3>
              <ul className="mt-2 space-y-2 text-sm text-[var(--aria-ink-muted)]">
                {(state.history?.entries ?? []).map((entry) => (
                  <li key={`${entry.kind}-${entry.id}`}>{entry.summary}</li>
                ))}
              </ul>
            </section>
          </div>
        ) : null}
      </div>

      <footer className="shrink-0 border-t border-[var(--aria-line)] bg-white p-3">
        {scopeOpen ? (
          <LinkedWorkspaceAmendmentSelector
            parentSessionId={state.childSessionId}
            targets={linkedAmendmentTargets}
            status={linkedAmendmentStatus}
            snapshot={linkedAmendmentSnapshot}
            error={linkedAmendmentError}
            disabled={!canMutate}
            onStart={onStartLinkedAmendment}
          />
        ) : null}
        {actionStatus ? (
          <p role="status" className="mb-2 text-right text-xs text-[var(--aria-ink-muted)]">
            {actionStatus}
          </p>
        ) : null}
        <div
          role="group"
          aria-label="Plan Repair 操作"
          className="flex min-w-0 flex-wrap items-center justify-end gap-2"
        >
          <button
            type="button"
            disabled={!canMutate}
            onClick={() => onAction?.("regenerate")}
            className="h-9 rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            要求重新生成
          </button>
          <button
            type="button"
            disabled={!canMutate}
            onClick={() => {
              setScopeOpen((current) => !current);
              onAction?.("adjust_scope");
            }}
            className="h-9 rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            调整修订范围
          </button>
          <button
            type="button"
            disabled={!canMutate}
            onClick={() => onAction?.("cancel")}
            className="h-9 rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-danger)] transition-colors hover:bg-[var(--aria-danger-soft)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-danger)]"
          >
            取消修订
          </button>
          <a
            href={`/workbench/workspace/${encodeURIComponent(state.childSessionId)}`}
            target="_blank"
            rel="noreferrer"
            onClick={() => onAction?.("open_workspace")}
            className="inline-flex h-9 items-center rounded-md border border-[var(--aria-line)] px-3 text-xs font-semibold text-[var(--aria-ink)] transition-colors hover:bg-[var(--aria-panel-muted)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            在完整 Work Item Workspace 中打开
          </a>
          <button
            type="button"
            disabled={!canConfirm}
            onClick={() => onAction?.("confirm")}
            className="h-9 rounded-md bg-[var(--aria-primary)] px-4 text-xs font-semibold text-white transition-opacity hover:opacity-90 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] focus-visible:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50"
          >
            确认修订并恢复执行
          </button>
        </div>
      </footer>
    </section>
  );
}
