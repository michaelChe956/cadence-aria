import { PanelLeftClose, PanelLeftOpen } from "lucide-react";
import { useState, type ReactNode } from "react";
import type {
  CodebaseSummaryDto,
  CodingAttemptAddress,
  IssueDeliverySummaryDto,
  LogicalCodebaseBootstrapProjection,
  LogicalCodebaseDto,
  LogicalCodebaseListEntryDto,
  PointerPublicationDto,
  Project,
  Repository,
  RepositoryInitializationOperationSnapshot,
  WorkItemRepositoryGroup,
} from "../../api/types";
import type { LifecycleCard as LifecycleCardData } from "../../state/lifecycle-workbench-store";
import { WorkbenchSurface } from "../shell/WorkbenchSurface";
import { IssueLifecycleWorkbenchHeader } from "./IssueLifecycleWorkbenchHeader";
import { LogicalCodebaseManagementDrawer } from "./LogicalCodebaseManagementDrawer";
import { LogicalCodebaseManagementPanel } from "./LogicalCodebaseManagementPanel";
import { LogicalCodebaseSummaryBar } from "./LogicalCodebaseSummaryBar";
import { ProjectSidebar } from "./ProjectSidebar";
import { ProviderManagementDrawer } from "./ProviderManagementDrawer";
import { ProviderManagementPanel } from "./ProviderManagementPanel";
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
  bootstrapProjection: unknown;
  /** 冷启动补读；传入投影则直接采用（动作/推进响应已含最终投影，免一次 GET）。 */
  onBootstrapChanged: (projection?: LogicalCodebaseBootstrapProjection) => void;
  aggregateIndex: unknown; aggregateIndexRebuilding: boolean; latestPointerPublication: PointerPublicationDto | null;
  pointerPublicationBusy: boolean; showIncrementalHint: boolean; focusedEntity: LifecycleCardData | null;
  isDrawerOpen: boolean; drawerWorkItems: unknown[];
  codingAttempts: unknown[]; deliverySummary: IssueDeliverySummaryDto | undefined;
  selectedIssueDeliverySummary: IssueDeliverySummaryDto | undefined;
  pendingWorkItemPlanLaunch: boolean;
  onSelectProject: (id: string) => void; onCreateProject: () => void; onAddCodebase: () => void;
  onDeleteProject: (id: string) => void; onDeleteRepository: (id: string) => void;
  onDeleteLogicalCodebase: (id: string) => void;
  deletedLogicalCodebases: LogicalCodebaseListEntryDto[];
  onRestoreLogicalCodebase: (id: string) => void; onPurgeLogicalCodebase: (id: string) => void;
  onShowAll: () => void; onRefresh: () => void;
  onCreateIssue: () => void; onToggleLcSummary: () => void; onReopenOnboarding?: () => void;
  onSelectLogicalCodebase: (id: string) => void; onOpenRegistration: () => void;
  onStartAggregateInitialization: () => void; onCancelAggregateInitialization: () => void;
  onReinitializeAggregateInitialization: () => void;
  onRebuildAggregateIndex: () => void; onPublishFull: () => void; onPublishIncremental: () => void;
  onRetryRepo: (id: string) => void; onRevoke: () => void; onToggleQueueCollapsed: () => void;
  onToggleQueueGroup: (key: IssueQueueGroupKey) => void; onQueueFilterTextChange: (text: string) => void;
  onSelectIssue: (id: string) => void; onGenerateStorySpec: (id: string) => void;
  onDeleteIssue: (id: string) => void; onShowMoreQueueGroup: (key: IssueQueueGroupKey) => void;
  deletingIssueId: string | null; onSelectCard: (card: LifecycleCardData) => void;
  onOpenFullIssue: (card: LifecycleCardData) => void; onDeleteCard: (card: LifecycleCardData) => void;
  onGenerateForStage: (stage: WorkbenchStageKey) => void; onCloseDrawer: () => void;
  onOpenWorkspaceFromDrawer: () => void; onOpenCodingWorkspaceFromDrawer: () => void;
  onGenerateNext: () => void; onDeleteFromDrawer: () => void;
  onDeleteAllCodingWorkspacesFromDrawer: () => void; groupCodingDeleteBusy: boolean;
  dialogs: ReactNode;
};
/// REQ-MRE-01 / C5：issue 创建可用性门——**必须存在含 active 成员的逻辑代码库**。
/// 存量单仓项目（项目级 repositories 非空但没有任何 LC / 成员）不再放行：
/// 新建 Issue 的下拉只能选 LC，点开才发现无路可走是 UX 死路 ⇒ 明确禁用 + 原因提示。
/// member_count 语义=该 LC active 成员数（后端按 manifest active 计，缺 manifest→0）；
/// 纯空项目（无 LC 或无 active 成员）保持禁用。
function createIssueGate(
  selectedProjectId: string | null,
  logicalCodebases: CodebaseSummaryDto[],
): { canCreate: boolean; disabledReason: string | null } {
  if (!selectedProjectId) {
    return { canCreate: false, disabledReason: "先在左侧选择一个 Project" };
  }
  if (!logicalCodebases.some((codebase) => (codebase.member_count ?? 0) > 0)) {
    return {
      canCreate: false,
      disabledReason:
        "该项目还没有含 active 成员（member_count > 0）的逻辑代码库：请先「添加代码库」→ 登记成员仓 → 完成冷启动，再新建 Issue",
    };
  }
  return { canCreate: true, disabledReason: null };
}

/// 服务端失败面是三段成品（问题 / 原因 / 怎么做，换行分隔）；告警条默认按
/// 单段渲染会把三段合并成一行 ⇒ 本地只保留换行，不引入第二份文案。
function AlertLines({ text }: { text: string }) {
  return <span className="block whitespace-pre-line leading-snug">{text}</span>;
}

export function IssueLifecycleWorkbenchView({
  projects, codebases, repositories, selectedProjectId, issueCount, busy, error, selectedProject,
  focusedIssueId, selectedCardKey, deletingCardKey, selectedIssue, storySpecs, designSpecs, workItems,
  workItemRepositoryGroups, issueQueueGroups, queueCollapsed, queueTotalCount, collapsedQueueGroups,
  queueFilterText, logicalCodebases, activeLogicalCodebaseId, activeLogicalCodebaseName,
  lcSummaryExpanded, lcSummaryHasWarning, logicalCodebaseMembers, aggregateInitialization,
  bootstrapProjection, onBootstrapChanged,
  aggregateInitializationBusy, aggregateIndex, aggregateIndexRebuilding, latestPointerPublication,
  pointerPublicationBusy, showIncrementalHint, focusedEntity, isDrawerOpen, drawerWorkItems,
  codingAttempts, deliverySummary, selectedIssueDeliverySummary, onSelectProject, onCreateProject, onAddCodebase, onDeleteProject,
  onDeleteRepository, onDeleteLogicalCodebase, deletedLogicalCodebases,
  onRestoreLogicalCodebase, onPurgeLogicalCodebase, onShowAll, onRefresh, onCreateIssue, onToggleLcSummary, onReopenOnboarding,
  onSelectLogicalCodebase, onOpenRegistration, onStartAggregateInitialization,
  onCancelAggregateInitialization, onReinitializeAggregateInitialization,
  onRebuildAggregateIndex, onPublishFull, onPublishIncremental,
  onRetryRepo, onRevoke, onToggleQueueCollapsed, onToggleQueueGroup, onQueueFilterTextChange,
  onSelectIssue, onGenerateStorySpec, onDeleteIssue, onShowMoreQueueGroup, deletingIssueId, onSelectCard,
  onOpenFullIssue, onDeleteCard, onGenerateForStage, onCloseDrawer, onOpenWorkspaceFromDrawer,
  onOpenCodingWorkspaceFromDrawer, onGenerateNext, onDeleteFromDrawer,
  onDeleteAllCodingWorkspacesFromDrawer, groupCodingDeleteBusy, dialogs,
}: IssueLifecycleWorkbenchViewProps) {
  const issueGate = createIssueGate(selectedProjectId, logicalCodebases);
  // 2026-11-04：Provider 管理是独立于 LC 管理抽屉的第二个管理浮层（能力核验
  // 已全局化，不再寄居在 LC 仓抽屉里）⇒ 开关独立，与 LC 抽屉互斥不叠加。
  const [providerManagementOpen, setProviderManagementOpen] = useState(false);
  // 能力面端点仍按逻辑代码库定位（承载 404 守卫），内容与逻辑代码库无关
  // （项目级全局）⇒ 取当前选中 LC，没有就取项目下任意一个已存在的 LC。
  const providerCapabilityScopeId =
    activeLogicalCodebaseId ??
    logicalCodebases[0]?.logical_codebase_id ??
    null;

  function handleOpenProviderManagement() {
    // 两个右侧管理抽屉互斥：先收起 LC 管理抽屉再开 Provider 管理（否则两层
    // 同位叠加）。
    if (lcSummaryExpanded) {
      onToggleLcSummary();
    }
    setProviderManagementOpen(true);
  }

  return <>
    <div data-testid="workbench-shell" className={`grid h-[100dvh] min-h-0 bg-[var(--aria-bg)] text-[var(--aria-ink)] lg:grid-cols-[17rem_minmax(0,1fr)]${isDrawerOpen && focusedEntity ? " lg:pr-[calc(480px+1rem)]" : ""}`}>
      <ProjectSidebar projects={projects} codebases={codebases} deletedLogicalCodebases={deletedLogicalCodebases} repositories={repositories} selectedProjectId={selectedProjectId} issueCount={issueCount} busy={busy} onSelectProject={onSelectProject} onCreateProject={onCreateProject} onAddCodebase={onAddCodebase} onDeleteProject={onDeleteProject} onDeleteRepository={onDeleteRepository} onDeleteLogicalCodebase={onDeleteLogicalCodebase} onRestoreLogicalCodebase={onRestoreLogicalCodebase} onPurgeLogicalCodebase={onPurgeLogicalCodebase} onOpenProviderManagement={handleOpenProviderManagement} />
      <WorkbenchSurface mainLabel="Issue 生命周期工作台" statusBar={busy ? <span className="text-xs font-semibold text-[var(--aria-ink-muted)]">加载中</span> : null} alert={error ? <AlertLines text={error} /> : null} header={<><IssueLifecycleWorkbenchHeader projectName={selectedProject?.name} focusedIssueId={focusedIssueId} canCreateIssue={issueGate.canCreate} onShowAll={onShowAll} onRefresh={onRefresh} onCreateIssue={onCreateIssue} onReopenOnboarding={onReopenOnboarding} />{issueGate.disabledReason ? (
        <p data-testid="issue-create-disabled-reason" className="mt-1 text-xs text-[var(--aria-ink-muted)]">
          {issueGate.disabledReason}
        </p>
      ) : null}</>} main={<div className="space-y-3">
        {selectedProjectId && logicalCodebases.length > 0 ? <>
          <LogicalCodebaseSummaryBar summary={{ lcName: activeLogicalCodebaseName, indexState: (aggregateIndex as { state?: string } | null)?.state ?? null, publicationStatus: latestPointerPublication?.status ?? null, hasWarning: lcSummaryHasWarning }} expanded={lcSummaryExpanded} onToggle={onToggleLcSummary} />
        </> : null}
        <div className="flex h-[calc(100dvh-6rem)] min-h-0 gap-3">
          {queueCollapsed ? <div data-testid="issue-queue-collapsed-rail" className="flex w-10 shrink-0 flex-col items-center gap-2 rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] py-2"><button type="button" aria-label="展开 Issue 队列" aria-expanded={false} onClick={onToggleQueueCollapsed} className="inline-flex h-7 w-7 shrink-0 cursor-pointer items-center justify-center rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] text-[var(--aria-ink-muted)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"><PanelLeftOpen className="h-4 w-4" /></button><span data-testid="issue-queue-rail-count" className="shrink-0 rounded border border-[var(--aria-line)] bg-[var(--aria-panel)] px-1 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">{queueTotalCount}</span></div> : <div data-testid="issue-queue-column" className="grid w-72 min-h-0 shrink-0 grid-rows-[auto_minmax(0,1fr)] gap-2"><button type="button" aria-label="折叠 Issue 队列" aria-expanded onClick={onToggleQueueCollapsed} className="inline-flex h-7 shrink-0 cursor-pointer items-center justify-center gap-1.5 self-start rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] px-2 text-[11px] font-semibold text-[var(--aria-ink-muted)] transition-colors duration-200 hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"><PanelLeftClose className="h-3.5 w-3.5" />折叠队列</button><IssueQueue groups={issueQueueGroups} focusedIssueId={focusedIssueId} collapsedGroups={collapsedQueueGroups} onToggleGroup={onToggleQueueGroup} filterText={queueFilterText} onFilterTextChange={onQueueFilterTextChange} onSelectIssue={onSelectIssue} onGenerateStorySpec={onGenerateStorySpec} onDeleteIssue={onDeleteIssue} onShowMoreGroup={onShowMoreQueueGroup} deletingIssueId={deletingIssueId} /></div>}
          <div className="flex min-h-0 min-w-0 flex-1"><IssueLifecycleDetail issue={selectedIssue} storySpecs={storySpecs} designSpecs={designSpecs} workItems={workItems} workItemRepositoryGroups={workItemRepositoryGroups} selectedKey={selectedCardKey} deliverySummary={selectedIssueDeliverySummary} onSelect={onSelectCard} onOpenFullIssue={onOpenFullIssue} onDelete={onDeleteCard} onGenerateForStage={onGenerateForStage} deletingKey={deletingCardKey} /></div>
        </div>
      </div>} />
    </div>
    {isDrawerOpen && focusedEntity ? <IssueLifecycleWorkbenchDrawer focusedEntity={focusedEntity} workItems={drawerWorkItems as never} codingAttempts={codingAttempts as never} deliverySummary={deliverySummary} onClose={onCloseDrawer} onOpenWorkspace={onOpenWorkspaceFromDrawer} onOpenCodingWorkspace={onOpenCodingWorkspaceFromDrawer} onGenerateNext={onGenerateNext} onDelete={onDeleteFromDrawer} onDeleteAllCodingWorkspaces={onDeleteAllCodingWorkspacesFromDrawer} groupCodingDeleteBusy={groupCodingDeleteBusy} /> : null}
    {selectedProjectId && logicalCodebases.length > 0 ? (
      <LogicalCodebaseManagementDrawer open={lcSummaryExpanded} onClose={onToggleLcSummary}>
        <LogicalCodebaseManagementPanel projectId={selectedProjectId} bootstrapProjection={bootstrapProjection as never} onBootstrapChanged={onBootstrapChanged} logicalCodebases={logicalCodebases} activeLogicalCodebaseId={activeLogicalCodebaseId} onSelectLogicalCodebase={onSelectLogicalCodebase} onOpenRegistration={onOpenRegistration} onOpenProviderManagement={handleOpenProviderManagement} logicalCodebaseMembers={logicalCodebaseMembers as never} aggregateInitialization={aggregateInitialization as never} aggregateInitializationBusy={aggregateInitializationBusy} onStartAggregateInitialization={onStartAggregateInitialization} onCancelAggregateInitialization={onCancelAggregateInitialization} onReinitializeAggregateInitialization={onReinitializeAggregateInitialization} aggregateIndex={aggregateIndex as never} aggregateIndexRebuilding={aggregateIndexRebuilding} onRebuildAggregateIndex={onRebuildAggregateIndex} latestPointerPublication={latestPointerPublication} pointerPublicationBusy={pointerPublicationBusy} showIncrementalHint={showIncrementalHint} onPublishFull={onPublishFull} onPublishIncremental={onPublishIncremental} onRetryRepo={onRetryRepo} onRevoke={onRevoke} />
      </LogicalCodebaseManagementDrawer>
    ) : null}
    {/* Provider 管理（独立浮层，与 LC 管理抽屉互斥）：能力核验已全局化，
        provider 的探针/核验/更新/安装/版本统一在这里管理。 */}
    {selectedProjectId ? (
      <ProviderManagementDrawer
        open={providerManagementOpen}
        onClose={() => setProviderManagementOpen(false)}
      >
        <ProviderManagementPanel
          projectId={selectedProjectId}
          logicalCodebaseId={providerCapabilityScopeId}
        />
      </ProviderManagementDrawer>
    ) : null}
    {dialogs}
  </>;
}
