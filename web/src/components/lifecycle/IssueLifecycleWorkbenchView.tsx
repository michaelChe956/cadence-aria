import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import type { ReactNode } from "react";
import type {
  CodebaseSummaryDto,
  CodingAttemptAddress,
  LogicalCodebaseDto,
  PointerPublicationDto,
  Project,
  Repository,
  RepositoryInitializationOperationSnapshot,
  WorkItemRepositoryGroup,
} from "../../api/types";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import { WorkbenchSurface } from "../shell/WorkbenchSurface";
import { IssueLifecycleWorkbenchHeader } from "./IssueLifecycleWorkbenchHeader";
import { LogicalCodebaseManagementPanel } from "./LogicalCodebaseManagementPanel";
import { LogicalCodebaseSummaryBar } from "./LogicalCodebaseSummaryBar";
import { ProjectSidebar } from "./ProjectSidebar";
import { IssueQueue } from "./IssueQueue";
import type { IssueQueueGroup, IssueQueueGroupKey } from "./issue-queue-derivation";
import type { WorkbenchStageKey } from "./StageStepper";
import { IssueLifecycleDetail } from "./IssueLifecycleWorkbenchParts";
import { IssueLifecycleWorkbenchDrawer } from "./IssueLifecycleWorkbenchDrawer";

export type IssueLifecycleWorkbenchViewProps = {
  projects: Project[]; codebases: CodebaseSummaryDto[]; repositories: Repository[];
  selectedProjectId: string | null; issueCount: number; busy: boolean; error: string | null;
  selectedProject?: Project; focusedIssueId: string | null; selectedCardKey: string | null;
  deletingCardKey: string | null; selectedIssue: LifecycleCardData | null;
  storySpecs: LifecycleCardData[]; designSpecs: LifecycleCardData[]; workItems: LifecycleCardData[];
  workItemRepositoryGroups: WorkItemRepositoryGroup[]; issueQueueGroups: IssueQueueGroup[];
  queueCollapsed: boolean; queueTotalCount: number; collapsedQueueGroups: IssueQueueGroupKey[];
  queueFilterText: string; logicalCodebases: CodebaseSummaryDto[]; activeLogicalCodebaseId: string | null;
  activeLogicalCodebaseName: string | null; lcSummaryExpanded: boolean; lcSummaryHasWarning: boolean;
  logicalCodebaseMembers: unknown[]; aggregateInitialization: unknown; aggregateInitializationBusy: boolean;
  aggregateIndex: unknown; aggregateIndexRebuilding: boolean; latestPointerPublication: PointerPublicationDto | null;
  pointerPublicationBusy: boolean; showIncrementalHint: boolean; focusedEntity: LifecycleCardData | null;
  isDrawerOpen: boolean; drawerWorkItems: unknown[];
  codingAttempts: unknown[]; deliverySummary: unknown; pendingWorkItemPlanLaunch: boolean;
  onSelectProject: (id: string) => void; onCreateProject: () => void; onAddCodebase: () => void;
  onDeleteProject: (id: string) => void; onDeleteRepository: (id: string) => void;
  onDeleteLogicalCodebase: (id: string) => void; onShowAll: () => void; onRefresh: () => void;
  onCreateIssue: () => void; onToggleLcSummary: () => void;
  onSelectLogicalCodebase: (id: string) => void; onOpenRegistration: () => void;
  onStartAggregateInitialization: () => void; onCancelAggregateInitialization: () => void;
  onRebuildAggregateIndex: () => void; onPublishFull: () => void; onPublishIncremental: () => void;
  onRetryRepo: (id: string) => void; onRevoke: () => void; onToggleQueueCollapsed: () => void;
  onToggleQueueGroup: (key: IssueQueueGroupKey) => void; onQueueFilterTextChange: (text: string) => void;
  onSelectIssue: (id: string) => void; onGenerateStorySpec: (id: string) => void;
  onDeleteIssue: (id: string) => void; onShowMoreQueueGroup: (key: IssueQueueGroupKey) => void;
  deletingIssueId: string | null; onSelectCard: (card: LifecycleCardData) => void;
  onOpenFullIssue: (card: LifecycleCardData) => void; onDeleteCard: (card: LifecycleCardData) => void;
  onGenerateForStage: (stage: WorkbenchStageKey) => void; onCloseDrawer: () => void;
  onOpenWorkspaceFromDrawer: () => void; onOpenCodingWorkspaceFromDrawer: () => void;
  onGenerateNext: () => void; onDeleteFromDrawer: () => void; dialogs: ReactNode;
};

export function IssueLifecycleWorkbenchView({
  projects, codebases, repositories, selectedProjectId, issueCount, busy, error, selectedProject,
  focusedIssueId, selectedCardKey, deletingCardKey, selectedIssue, storySpecs, designSpecs, workItems,
  workItemRepositoryGroups, issueQueueGroups, queueCollapsed, queueTotalCount, collapsedQueueGroups,
  queueFilterText, logicalCodebases, activeLogicalCodebaseId, activeLogicalCodebaseName,
  lcSummaryExpanded, lcSummaryHasWarning, logicalCodebaseMembers, aggregateInitialization,
  aggregateInitializationBusy, aggregateIndex, aggregateIndexRebuilding, latestPointerPublication,
  pointerPublicationBusy, showIncrementalHint, focusedEntity, isDrawerOpen, drawerWorkItems,
  codingAttempts, deliverySummary, onSelectProject, onCreateProject, onAddCodebase, onDeleteProject,
  onDeleteRepository, onDeleteLogicalCodebase, onShowAll, onRefresh, onCreateIssue, onToggleLcSummary,
  onSelectLogicalCodebase, onOpenRegistration, onStartAggregateInitialization,
  onCancelAggregateInitialization, onRebuildAggregateIndex, onPublishFull, onPublishIncremental,
  onRetryRepo, onRevoke, onToggleQueueCollapsed, onToggleQueueGroup, onQueueFilterTextChange,
  onSelectIssue, onGenerateStorySpec, onDeleteIssue, onShowMoreQueueGroup, deletingIssueId, onSelectCard,
  onOpenFullIssue, onDeleteCard, onGenerateForStage, onCloseDrawer, onOpenWorkspaceFromDrawer,
  onOpenCodingWorkspaceFromDrawer, onGenerateNext, onDeleteFromDrawer, dialogs,
}: IssueLifecycleWorkbenchViewProps) {
  return <>
    <div data-testid="workbench-shell" className="grid h-[100dvh] min-h-0 bg-[var(--aria-bg)] text-[var(--aria-ink)] lg:grid-cols-[17rem_minmax(0,1fr)]">
      <ProjectSidebar projects={projects} codebases={codebases} repositories={repositories} selectedProjectId={selectedProjectId} issueCount={issueCount} busy={busy} onSelectProject={onSelectProject} onCreateProject={onCreateProject} onAddCodebase={onAddCodebase} onDeleteProject={onDeleteProject} onDeleteRepository={onDeleteRepository} onDeleteLogicalCodebase={onDeleteLogicalCodebase} />
      <WorkbenchSurface mainLabel="Issue 生命周期工作台" statusBar={busy ? <span className="text-xs font-semibold text-[var(--aria-ink-muted)]">加载中</span> : null} alert={error} header={<IssueLifecycleWorkbenchHeader projectName={selectedProject?.name} focusedIssueId={focusedIssueId} canCreateIssue={Boolean(selectedProjectId) && repositories.length > 0} onShowAll={onShowAll} onRefresh={onRefresh} onCreateIssue={onCreateIssue} />} main={<div className="space-y-3">
        {selectedProjectId && logicalCodebases.length > 0 ? <div className="space-y-2">
          <LogicalCodebaseSummaryBar summary={{ lcName: activeLogicalCodebaseName, indexState: (aggregateIndex as { state?: string } | null)?.state ?? null, publicationStatus: latestPointerPublication?.status ?? null, hasWarning: lcSummaryHasWarning }} expanded={lcSummaryExpanded} onToggle={onToggleLcSummary} />
          {lcSummaryExpanded ? <LogicalCodebaseManagementPanel logicalCodebases={logicalCodebases} activeLogicalCodebaseId={activeLogicalCodebaseId} onSelectLogicalCodebase={onSelectLogicalCodebase} onOpenRegistration={onOpenRegistration} logicalCodebaseMembers={logicalCodebaseMembers as never} aggregateInitialization={aggregateInitialization as never} aggregateInitializationBusy={aggregateInitializationBusy} onStartAggregateInitialization={onStartAggregateInitialization} onCancelAggregateInitialization={onCancelAggregateInitialization} aggregateIndex={aggregateIndex as never} aggregateIndexRebuilding={aggregateIndexRebuilding} onRebuildAggregateIndex={onRebuildAggregateIndex} latestPointerPublication={latestPointerPublication} pointerPublicationBusy={pointerPublicationBusy} showIncrementalHint={showIncrementalHint} onPublishFull={onPublishFull} onPublishIncremental={onPublishIncremental} onRetryRepo={onRetryRepo} onRevoke={onRevoke} /> : null}
        </div> : null}
        <div className="flex h-[calc(100dvh-6rem)] min-h-0 gap-3">
          {queueCollapsed ? <div data-testid="issue-queue-collapsed-rail" className="flex w-10 shrink-0 flex-col items-center gap-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] py-2"><button type="button" aria-label="展开 Issue 队列" aria-expanded={false} onClick={onToggleQueueCollapsed} className="inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"><PanelLeftOpen className="h-4 w-4" /></button><span data-testid="issue-queue-rail-count" className="shrink-0 rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-1 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">{queueTotalCount}</span></div> : <div data-testid="issue-queue-column" className="grid w-72 min-h-0 shrink-0 grid-rows-[auto_minmax(0,1fr)] gap-2"><button type="button" aria-label="折叠 Issue 队列" aria-expanded onClick={onToggleQueueCollapsed} className="inline-flex h-7 shrink-0 cursor-pointer items-center justify-center gap-1.5 self-start rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 text-[11px] font-semibold text-[var(--aria-ink-muted)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"><PanelLeftClose className="h-3.5 w-3.5" />折叠队列</button><IssueQueue groups={issueQueueGroups} focusedIssueId={focusedIssueId} collapsedGroups={collapsedQueueGroups} onToggleGroup={onToggleQueueGroup} filterText={queueFilterText} onFilterTextChange={onQueueFilterTextChange} onSelectIssue={onSelectIssue} onGenerateStorySpec={onGenerateStorySpec} onDeleteIssue={onDeleteIssue} onShowMoreGroup={onShowMoreQueueGroup} deletingIssueId={deletingIssueId} /></div>}
          <div className="flex min-h-0 min-w-0 flex-1"><IssueLifecycleDetail issue={selectedIssue} storySpecs={storySpecs} designSpecs={designSpecs} workItems={workItems} workItemRepositoryGroups={workItemRepositoryGroups} selectedKey={selectedCardKey} onSelect={onSelectCard} onOpenFullIssue={onOpenFullIssue} onDelete={onDeleteCard} onGenerateForStage={onGenerateForStage} deletingKey={deletingCardKey} /></div>
        </div>
      </div>} />
    </div>
    {isDrawerOpen && focusedEntity ? <IssueLifecycleWorkbenchDrawer focusedEntity={focusedEntity} workItems={drawerWorkItems as never} codingAttempts={codingAttempts as never} deliverySummary={deliverySummary as never} onClose={onCloseDrawer} onOpenWorkspace={onOpenWorkspaceFromDrawer} onOpenCodingWorkspace={onOpenCodingWorkspaceFromDrawer} onGenerateNext={onGenerateNext} onDelete={onDeleteFromDrawer} /> : null}
    {dialogs}
  </>;
}
