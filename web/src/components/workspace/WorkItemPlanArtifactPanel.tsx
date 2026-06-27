import { useEffect, useState } from "react";
import type {
  WorkItemPlanArtifactPayload,
  WorkItemPlanArtifactVersion,
} from "../../api/types";
import {
  WorkItemPlanArtifactTabContent,
  groupWorkItemPlanArtifactVersions,
  type WorkItemPlanArtifactTab,
  workItemPlanArtifactLabel,
} from "./WorkItemPlanArtifactContent";

export interface WorkItemPlanArtifactPanelProps {
  artifact: WorkItemPlanArtifactPayload | null;
  versions?: WorkItemPlanArtifactVersion[];
  selectedVersion?: number | null;
  onSelectVersion?: (version: number | null) => void;
  activeNodeType?: string | null;
  readonly?: boolean;
  className?: string;
}

export function WorkItemPlanArtifactPanel({
  artifact,
  versions = [],
  selectedVersion = null,
  onSelectVersion,
  readonly = false,
  className = "",
}: WorkItemPlanArtifactPanelProps) {
  const [activeTab, setActiveTab] = useState<WorkItemPlanArtifactTab>(() =>
    artifact ? defaultArtifactTab(artifact) : "overview",
  );

  useEffect(() => {
    if (artifact) {
      setActiveTab(defaultArtifactTab(artifact));
    }
  }, [artifact?.type]);

  if (!artifact) {
    return (
      <div
        data-testid="work-item-plan-artifact-panel"
        className={`min-h-0 overflow-auto p-4 text-sm text-[var(--aria-ink-muted)] ${className}`}
      >
        尚未生成 staged artifact
      </div>
    );
  }

  return (
    <div
      data-testid="work-item-plan-artifact-panel"
      className={`min-h-0 overflow-auto p-4 ${className}`}
    >
      <section className="mb-4 space-y-3 border-b border-[var(--aria-line)] pb-4">
        <div className="flex min-w-0 flex-wrap items-start justify-between gap-3">
          <div className="min-w-0">
            <h2 className="text-sm font-semibold text-[var(--aria-ink)]">
              Work Item Plan 工作台
            </h2>
            <p
              aria-live="polite"
              className="mt-1 break-words text-xs leading-5 text-[var(--aria-ink-muted)]"
            >
              {artifactStatusMessage(artifact, readonly, selectedVersion)}
            </p>
          </div>
          <span className="rounded border border-[var(--aria-line)] px-2 py-0.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
            {selectedVersion ? `v${selectedVersion}` : "Current"}
          </span>
        </div>
        <WorkItemPlanArtifactVersionRail
          versions={versions}
          selectedVersion={selectedVersion}
          onSelectVersion={onSelectVersion}
        />
        <WorkItemPlanTabs activeTab={activeTab} onSelectTab={setActiveTab} />
      </section>

      {readonly ? (
        <div className="mb-3 rounded border border-amber-200 bg-amber-50 px-3 py-2 text-xs font-semibold text-amber-800">
          只读历史
        </div>
      ) : null}

      <WorkItemPlanArtifactTabContent
        artifact={artifact}
        activeTab={activeTab}
        versions={versions}
        selectedVersion={selectedVersion}
      />
    </div>
  );
}

function WorkItemPlanArtifactVersionRail({
  versions,
  selectedVersion,
  onSelectVersion,
}: {
  versions: WorkItemPlanArtifactVersion[];
  selectedVersion: number | null;
  onSelectVersion?: (version: number | null) => void;
}) {
  const groups = groupWorkItemPlanArtifactVersions(versions);
  const selectedPhaseKey = phaseKeyForSelectedVersion(groups, selectedVersion);
  const [phaseKey, setPhaseKey] = useState(selectedPhaseKey);

  useEffect(() => {
    setPhaseKey(selectedPhaseKey);
  }, [selectedPhaseKey]);

  const activeGroup =
    groups.find((group) => group.key === phaseKey) ?? groups[0] ?? null;
  const activeVersion =
    activeGroup?.versions.find((version) => version.version === selectedVersion) ??
    latestVersionInGroup(activeGroup);

  return (
    <div
      data-testid="work-item-plan-version-rail"
      className="grid min-w-0 gap-2 rounded-md border border-[var(--aria-line)] bg-white p-2 sm:grid-cols-[minmax(160px,220px)_minmax(220px,1fr)]"
    >
      {versions.length === 0 ? (
        <span className="text-xs text-[var(--aria-ink-muted)]">暂无版本</span>
      ) : (
        <>
          <select
            aria-label="Artifact phase"
            data-testid="work-item-plan-phase-select"
            value={activeGroup?.key ?? ""}
            onChange={(event) => {
              const nextGroup = groups.find((group) => group.key === event.target.value);
              setPhaseKey(event.target.value);
              const nextVersion = latestVersionInGroup(nextGroup);
              if (nextVersion) {
                onSelectVersion?.(nextVersion.version);
              }
            }}
            className="h-9 min-w-0 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs font-semibold text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            {groups.map((group) => (
              <option key={group.key} value={group.key}>
                {group.label}
              </option>
            ))}
          </select>
          <select
            aria-label="Artifact version"
            data-testid="work-item-plan-version-select"
            value={activeVersion?.version ?? ""}
            disabled={!activeGroup}
            onChange={(event) => {
              onSelectVersion?.(Number(event.target.value));
            }}
            className="h-9 min-w-0 rounded-md border border-[var(--aria-line)] bg-white px-2 font-mono text-xs text-[var(--aria-ink)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] disabled:bg-[var(--aria-panel-muted)] disabled:text-[var(--aria-ink-muted)]"
          >
            {(activeGroup?.versions ?? []).map((version) => {
              const label = version.artifact
                ? workItemPlanArtifactLabel(version.artifact)
                : "按需加载";
              return (
                <option key={version.version} value={version.version}>
                  v{version.version} · {label}
                  {version.is_current ? " · current" : ""}
                </option>
              );
            })}
          </select>
        </>
      )}
    </div>
  );
}

function phaseKeyForSelectedVersion(
  groups: ReturnType<typeof groupWorkItemPlanArtifactVersions>,
  selectedVersion: number | null,
) {
  if (selectedVersion !== null) {
    const selectedGroup = groups.find((group) =>
      group.versions.some((version) => version.version === selectedVersion),
    );
    if (selectedGroup) {
      return selectedGroup.key;
    }
  }
  const currentGroup = groups.find((group) =>
    group.versions.some((version) => version.is_current),
  );
  return currentGroup?.key ?? groups[0]?.key ?? "";
}

function latestVersionInGroup(
  group: ReturnType<typeof groupWorkItemPlanArtifactVersions>[number] | null | undefined,
) {
  if (!group || group.versions.length === 0) {
    return null;
  }
  return group.versions.reduce((latest, version) =>
    version.version > latest.version ? version : latest,
  );
}

function WorkItemPlanTabs({
  activeTab,
  onSelectTab,
}: {
  activeTab: WorkItemPlanArtifactTab;
  onSelectTab: (tab: WorkItemPlanArtifactTab) => void;
}) {
  const tabs: Array<[WorkItemPlanArtifactTab, string]> = [
    ["overview", "Overview"],
    ["outline", "Outline"],
    ["drafts", "Drafts"],
    ["diff", "Diff"],
    ["review", "Review"],
    ["json", "JSON"],
  ];
  return (
    <div
      aria-label="Work Item Plan artifact views"
      className="flex min-w-0 gap-1 overflow-x-auto rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-1 text-xs"
    >
      {tabs.map(([tab, label]) => (
        <button
          key={tab}
          type="button"
          aria-pressed={activeTab === tab}
          onClick={() => onSelectTab(tab)}
          className={`h-8 shrink-0 rounded px-3 font-semibold transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)] ${
            activeTab === tab
              ? "bg-white text-[var(--aria-ink)] shadow-sm"
              : "text-[var(--aria-ink-muted)] hover:bg-white hover:text-[var(--aria-ink)]"
          }`}
        >
          {label}
        </button>
      ))}
    </div>
  );
}

function artifactStatusMessage(
  artifact: WorkItemPlanArtifactPayload,
  readonly: boolean,
  selectedVersion?: number | null,
) {
  if (readonly && selectedVersion) {
    return `正在查看历史版本 v${selectedVersion}，不影响当前流程。`;
  }
  switch (artifact.type) {
    case "outline_candidate":
      return "Outline 已生成，等待确认。Work Item 尚未生成。";
    case "draft_candidate":
      return "当前仅展示单个 Draft，不代表整组 Work Item 完成。";
    case "batch_state":
      return `已生成 ${artifact.payload.draft_records.length} 个 Draft，等待接受全部或返修。`;
    case "compile_report":
      if (artifact.payload.status === "committed") {
        return `Compile 已提交，生成 ${artifact.payload.work_item_ids.length} 个 Work Item、${artifact.payload.verification_plan_ids.length} 个 Verification Plan、${artifact.payload.child_session_ids.length} 个 child session。`;
      }
      return `Compile ${artifact.payload.status}，Work Item 尚未确认完成。`;
    case "context_blocker":
      return "缺少上下文，Work Item Plan 暂时无法继续。";
    default:
      return "Work Item Plan artifact 已更新。";
  }
}

function defaultArtifactTab(artifact: WorkItemPlanArtifactPayload): WorkItemPlanArtifactTab {
  switch (artifact.type) {
    case "outline_candidate":
      return "outline";
    case "draft_candidate":
    case "batch_state":
      return "drafts";
    case "context_blocker":
      return "review";
    case "compile_report":
      return "overview";
    default:
      return "overview";
  }
}
