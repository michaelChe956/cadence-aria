import { AggregateIndexCard } from "./AggregateIndexCard";
import { AggregateInitializationCard } from "./AggregateInitializationCard";
import { LogicalCodebaseBootstrapCard } from "./LogicalCodebaseBootstrapCard";
import { PointerPublicationPanel } from "./PointerPublicationPanel";
import type {
  AggregateIndexActiveResponse,
  AggregateInitializationOperationSnapshot,
  CodebaseSummaryDto,
  LogicalCodebaseBootstrapProjection,
  LogicalCodebaseMemberDto,
  PointerPublicationDto,
} from "../../api/types";

type LogicalCodebaseManagementPanelProps = {
  logicalCodebases: CodebaseSummaryDto[];
  projectId: string | null;
  bootstrapProjection: LogicalCodebaseBootstrapProjection | null;
  /** 冷启动补读；传入投影则直接采用（动作/推进响应已含最终投影，免一次 GET）。 */
  onBootstrapChanged: (projection?: LogicalCodebaseBootstrapProjection) => void;
  activeLogicalCodebaseId: string | null;
  onSelectLogicalCodebase: (logicalCodebaseId: string) => void;
  onOpenRegistration: () => void;
  /** 打开独立的「Provider 管理」面板（provider 能力核验已搬出本面板）。 */
  onOpenProviderManagement: () => void;
  logicalCodebaseMembers: LogicalCodebaseMemberDto[];
  aggregateInitialization: AggregateInitializationOperationSnapshot | null;
  aggregateInitializationBusy: boolean;
  onStartAggregateInitialization: () => void;
  onCancelAggregateInitialization: () => void;
  onReinitializeAggregateInitialization: () => void;
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
 * 逻辑代码库管理面板：LC 切换 tab + 冷启动 + 聚合初始化/索引卡 + 指针发布面板。
 * 抽屉化后由 LogicalCodebaseManagementDrawer 承载（右侧滑入），本组件只管内容。
 * 2026-11-04：provider 能力核验区已搬去独立的「Provider 管理」面板（本面板只留
 * 一行只读说明 + 「去 Provider 管理」入口）。
 */
export function LogicalCodebaseManagementPanel({
  projectId,
  bootstrapProjection,
  onBootstrapChanged,
  logicalCodebases,
  activeLogicalCodebaseId,
  onSelectLogicalCodebase,
  onOpenRegistration,
  onOpenProviderManagement,
  logicalCodebaseMembers,
  aggregateInitialization,
  aggregateInitializationBusy,
  onStartAggregateInitialization,
  onCancelAggregateInitialization,
  onReinitializeAggregateInitialization,
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
    <div className="bg-[var(--aria-panel)]">
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
      {logicalCodebases.length > 0 ? (
        <LogicalCodebaseBootstrapCard
          projectId={projectId}
          projection={bootstrapProjection}
          onChanged={onBootstrapChanged}
        />
      ) : null}
      {logicalCodebases.length > 0 ? (
        /* 2026-11-04：provider 能力核验区整体搬去独立的「Provider 管理」面板
           （owner 裁定：能力核验已全局化，provider 管理不该寄居在 LC 仓抽屉里）。
           原位置保留一行只读说明 + 「去 Provider 管理」入口（可发现性不断），
           不在这里派生任何新的统计口径（数字口径待能力全局化落地后再定）。 */
        <div
          data-testid="lc-provider-management-summary"
          className="border-b border-[var(--aria-line)] px-3 py-2"
        >
          <p className="text-xs text-[var(--aria-ink-muted)]">
            Provider 能力核验、安装与版本管理已移至「Provider 管理」入口。
          </p>
          <button
            type="button"
            data-testid="provider-management-link"
            onClick={onOpenProviderManagement}
            className="mt-1 inline-flex h-7 items-center rounded-md border border-[var(--aria-line)] px-2 text-[11px] font-semibold text-[var(--aria-ink)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            去 Provider 管理
          </button>
        </div>
      ) : null}
      {logicalCodebaseMembers.length > 0 ? (
        <AggregateInitializationCard
          operation={aggregateInitialization}
          busy={aggregateInitializationBusy}
          onStart={onStartAggregateInitialization}
          onCancel={onCancelAggregateInitialization}
          onReinitialize={onReinitializeAggregateInitialization}
          advanceStop={bootstrapProjection?.advance_stop ?? undefined}
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
