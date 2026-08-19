import { AggregateIndexCard } from "./AggregateIndexCard";
import { AggregateInitializationCard } from "./AggregateInitializationCard";
import { PointerPublicationPanel } from "./PointerPublicationPanel";
import type {
  AggregateIndexActiveResponse,
  AggregateInitializationOperationSnapshot,
  CodebaseSummaryDto,
  LogicalCodebaseMemberDto,
  PointerPublicationDto,
} from "../../api/types";

type LogicalCodebaseManagementPanelProps = {
  logicalCodebases: CodebaseSummaryDto[];
  activeLogicalCodebaseId: string | null;
  onSelectLogicalCodebase: (logicalCodebaseId: string) => void;
  onOpenRegistration: () => void;
  logicalCodebaseMembers: LogicalCodebaseMemberDto[];
  aggregateInitialization: AggregateInitializationOperationSnapshot | null;
  aggregateInitializationBusy: boolean;
  onStartAggregateInitialization: () => void;
  onCancelAggregateInitialization: () => void;
  aggregateIndex: AggregateIndexActiveResponse | null;
  aggregateIndexRebuilding: boolean;
  onRebuildAggregateIndex: () => void;
  latestPointerPublication: PointerPublicationDto | null;
  pointerPublicationBusy: boolean;
  showIncrementalHint: boolean;
  onPublishFull: () => void;
  onPublishIncremental: () => void;
  onRetryRepo: (memberRepoId: string) => void;
  onRevoke: () => void;
};

/**
 * 逻辑代码库管理面板（纯搬运自 IssueLifecycleWorkbench，无行为改动）：
 * LC 切换 tab + 聚合初始化/索引卡 + 指针发布面板。
 */
export function LogicalCodebaseManagementPanel({
  logicalCodebases,
  activeLogicalCodebaseId,
  onSelectLogicalCodebase,
  onOpenRegistration,
  logicalCodebaseMembers,
  aggregateInitialization,
  aggregateInitializationBusy,
  onStartAggregateInitialization,
  onCancelAggregateInitialization,
  aggregateIndex,
  aggregateIndexRebuilding,
  onRebuildAggregateIndex,
  latestPointerPublication,
  pointerPublicationBusy,
  showIncrementalHint,
  onPublishFull,
  onPublishIncremental,
  onRetryRepo,
  onRevoke,
}: LogicalCodebaseManagementPanelProps) {
  return (
    <div className="overflow-hidden rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)]">
      <div className="flex items-center justify-between gap-3 border-b border-[var(--aria-line)] px-3 py-2">
        <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
          逻辑代码库
        </h2>
        <button
          type="button"
          disabled={logicalCodebases.length === 0}
          onClick={onOpenRegistration}
          className="rounded-md border border-[var(--aria-primary)] bg-[var(--aria-primary)] px-3 py-1.5 text-xs font-semibold text-white disabled:opacity-60"
        >
          登记成员
        </button>
      </div>
      {logicalCodebases.length > 0 ? (
        <div
          role="tablist"
          aria-label="逻辑代码库切换"
          className="flex flex-wrap gap-2 border-b border-[var(--aria-line)] px-3 py-2"
        >
          {logicalCodebases.map((codebase) => {
            const lcId = codebase.logical_codebase_id ?? codebase.id;
            const selected = lcId === activeLogicalCodebaseId;
            return (
              <button
                key={codebase.id}
                type="button"
                role="tab"
                aria-selected={selected}
                data-testid={`lc-selector-${codebase.name}`}
                onClick={() => onSelectLogicalCodebase(lcId)}
                className={
                  selected
                    ? "rounded-md border border-[var(--aria-primary)] bg-[var(--aria-panel-muted)] px-3 py-1 text-xs font-semibold text-[var(--aria-primary)] ring-2 ring-[var(--aria-primary)]"
                    : "rounded-md border border-[var(--aria-line)] px-3 py-1 text-xs font-semibold text-[var(--aria-ink-muted)]"
                }
              >
                {codebase.name}
              </button>
            );
          })}
        </div>
      ) : null}
      {logicalCodebaseMembers.length > 0 ? (
        <AggregateInitializationCard
          operation={aggregateInitialization}
          busy={aggregateInitializationBusy}
          onStart={onStartAggregateInitialization}
          onCancel={onCancelAggregateInitialization}
        />
      ) : null}
      {aggregateIndex ? (
        <AggregateIndexCard
          index={aggregateIndex}
          rebuilding={aggregateIndexRebuilding}
          onRebuild={onRebuildAggregateIndex}
        />
      ) : null}
      <PointerPublicationPanel
        publication={latestPointerPublication}
        busy={pointerPublicationBusy}
        showIncrementalHint={showIncrementalHint}
        onPublishFull={onPublishFull}
        onPublishIncremental={onPublishIncremental}
        onRetryRepo={onRetryRepo}
        onRevoke={onRevoke}
      />
    </div>
  );
}
