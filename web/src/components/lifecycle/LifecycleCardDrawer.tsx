import {
  Code,
  ExternalLink,
  FileText,
  GitBranch,
  Layers3,
  ListChecks,
  ScrollText,
  X,
} from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import type { ArtifactVersion, CodingAttempt, ProductIssueArtifact } from "../../api/types";
import { MonacoDiffViewer } from "../shared/MonacoDiffViewer";
import { MonacoViewer } from "../shared/MonacoViewer";

export type DrawerEntityKind = "issue" | "story_spec" | "design_spec" | "work_item";

export interface DrawerEntity {
  id: string;
  kind: DrawerEntityKind;
  title: string;
  status: string;
  version: number | null;
  artifactVersions?: ArtifactVersion[];
  description?: string;
  artifacts?: ProductIssueArtifact[];
  phase?: string;
  createdAt?: string;
  latestAttempt?: CodingAttempt | null;
}

interface LifecycleCardDrawerProps {
  entity: DrawerEntity;
  onClose: () => void;
  onOpenWorkspace: () => void;
  onOpenCodingWorkspace?: () => void;
  onGenerateNext?: () => void;
}

const KIND_LABELS: Record<DrawerEntityKind, string> = {
  issue: "Issue",
  story_spec: "Story Spec",
  design_spec: "Design Spec",
  work_item: "Work Item",
};

const STATUS_LABELS: Record<string, string> = {
  confirmed: "已确认",
  draft: "草稿",
  in_review: "审核中",
  change_requested: "要求修改",
  blocked: "阻塞",
  pending: "待处理",
  planning: "规划中",
  completed: "已完成",
};

const NEXT_ACTION_LABELS: Partial<Record<DrawerEntityKind, string>> = {
  story_spec: "生成 Design Spec",
  design_spec: "生成 Work Item",
};

export function LifecycleCardDrawer({
  entity,
  onClose,
  onOpenWorkspace,
  onOpenCodingWorkspace,
  onGenerateNext,
}: LifecycleCardDrawerProps) {
  const [showAllVersions, setShowAllVersions] = useState(false);
  const [selectedVersionIndex, setSelectedVersionIndex] = useState(0);
  const [showVersionDiff, setShowVersionDiff] = useState(false);
  const [artifactPreviewOpen, setArtifactPreviewOpen] = useState(false);
  const [issuePreviewOpen, setIssuePreviewOpen] = useState(false);
  const versions = useMemo(
    () => [...(entity.artifactVersions ?? [])].sort((left, right) => right.version - left.version),
    [entity.artifactVersions],
  );
  const visibleVersions = showAllVersions ? versions : versions.slice(0, 3);
  const selectedArtifact = versions[selectedVersionIndex] ?? null;
  const latestVersion = versions[0] ?? null;
  const canShowDiff = selectedVersionIndex > 0 && selectedArtifact !== null && latestVersion !== null;
  const nextActionLabel = entity.status === "confirmed" ? NEXT_ACTION_LABELS[entity.kind] : null;
  const codingActionLabel = entity.latestAttempt ? "进入 Coding Workspace" : "开始 Coding";
  const Icon = iconForKind(entity.kind);

  useEffect(() => {
    setSelectedVersionIndex(0);
    setShowVersionDiff(false);
    setShowAllVersions(false);
    setArtifactPreviewOpen(false);
    setIssuePreviewOpen(false);
  }, [entity.id]);

  return (
    <aside
      data-testid="lifecycle-card-drawer"
      aria-label={`${KIND_LABELS[entity.kind]} 详情`}
      className="flex h-full min-h-0 flex-col border-l border-[var(--aria-line)] bg-[var(--aria-panel)]"
    >
      <header className="flex min-w-0 items-start justify-between gap-3 border-b border-[var(--aria-line)] px-4 py-3">
        <div className="min-w-0">
          <div className="mb-1 flex items-center gap-2 text-xs font-semibold uppercase text-[var(--aria-ink-muted)]">
            <Icon className="h-3.5 w-3.5 text-[var(--aria-primary)]" />
            {KIND_LABELS[entity.kind]}
          </div>
          <h2 className="truncate text-base font-semibold text-[var(--aria-ink)]">
            {entity.title}
          </h2>
          <div className="mt-2 flex flex-wrap gap-1.5 font-mono text-[11px] text-[var(--aria-ink-muted)]">
            <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
              {entity.id}
            </span>
            <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
              {STATUS_LABELS[entity.status] ?? entity.status}
            </span>
            {entity.version ? (
              <span className="rounded border border-[var(--aria-line)] px-1.5 py-0.5">
                v{entity.version}
              </span>
            ) : null}
          </div>
        </div>
        <button
          type="button"
          aria-label="关闭"
          onClick={onClose}
          className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)]"
        >
          <X className="h-4 w-4" />
        </button>
      </header>

      <div className="min-h-0 flex-1 overflow-auto">
        {entity.kind === "issue" ? (
          <IssueDetail
            entity={entity}
            onOpenPreview={() => setIssuePreviewOpen(true)}
          />
        ) : versions.length > 0 ? (
          <section className="border-b border-[var(--aria-line)] px-4 py-3">
            <h3 className="mb-2 text-sm font-semibold text-[var(--aria-ink)]">版本历史</h3>
            <div className="space-y-2">
              {visibleVersions.map((version, index) => (
                <button
                  type="button"
                  key={`${version.version}-${version.source_node_id}`}
                  onClick={() => {
                    setSelectedVersionIndex(index);
                    setShowVersionDiff(false);
                  }}
                  className={`w-full rounded-md border px-2 py-2 text-left text-xs transition-colors ${
                    index === selectedVersionIndex
                      ? "border-[var(--aria-primary)] bg-[var(--aria-primary)]/5"
                      : "border-[var(--aria-line)] bg-[var(--aria-panel-muted)] hover:border-[var(--aria-primary)]/50"
                  }`}
                >
                  <div className="flex min-w-0 items-center justify-between gap-2">
                    <span className="font-semibold text-[var(--aria-ink)]">v{version.version}</span>
                    <span className="shrink-0 text-[var(--aria-ink-muted)]">
                      {version.created_at.slice(0, 10)}
                    </span>
                  </div>
                  <div className="mt-1 text-[var(--aria-ink-muted)]">
                    作者: {providerLabel(version.generated_by)}
                    {version.reviewed_by ? ` · 审核: ${providerLabel(version.reviewed_by)}` : ""}
                    {version.confirmed_by ? ` · 确认: ${version.confirmed_by}` : ""}
                  </div>
                </button>
              ))}
            </div>
            {versions.length > 3 ? (
              <button
                type="button"
                onClick={() => setShowAllVersions((value) => !value)}
                className="mt-2 text-xs font-semibold text-[var(--aria-primary)] hover:underline"
              >
                {showAllVersions ? "收起" : `查看全部 ${versions.length} 个版本`}
              </button>
            ) : null}
          </section>
        ) : null}

        {selectedArtifact ? (
          <section className="px-4 py-3">
            <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
              <div className="flex min-w-0 items-center justify-between gap-3">
                <div className="min-w-0">
                  <div className="flex items-center gap-2 text-sm font-semibold text-[var(--aria-ink)]">
                    <FileText className="h-4 w-4 text-[var(--aria-primary)]" />
                    版本 v{selectedArtifact.version}
                  </div>
                  <p className="mt-1 text-xs leading-5 text-[var(--aria-ink-muted)]">
                    默认隐藏长内容，按需打开 Markdown 大预览。
                  </p>
                </div>
                <button
                  type="button"
                  onClick={() => setArtifactPreviewOpen(true)}
                  className="inline-flex h-8 shrink-0 items-center rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]"
                >
                  查看 Markdown 内容
                </button>
              </div>
            </div>
          </section>
        ) : null}
      </div>

      {artifactPreviewOpen && selectedArtifact ? (
        <div className="fixed inset-4 z-[70] flex min-h-0 flex-col rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-2xl">
          <div className="flex shrink-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] px-4 py-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-sm font-semibold text-[var(--aria-ink)]">
                <FileText className="h-4 w-4 text-[var(--aria-primary)]" />
                版本 v{selectedArtifact.version} 预览
              </div>
              <p className="mt-1 truncate font-mono text-xs text-[var(--aria-ink-muted)]">
                {entity.id}
              </p>
            </div>
            <button
              type="button"
              aria-label="关闭 Markdown 内容预览"
              onClick={() => {
                setArtifactPreviewOpen(false);
                setShowVersionDiff(false);
              }}
              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)]"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="min-h-0 flex-1 p-4">
            {showVersionDiff && canShowDiff ? (
              <MonacoDiffViewer
                original={selectedArtifact.markdown}
                modified={latestVersion.markdown}
                language="markdown"
                height="100%"
              />
            ) : (
              <MonacoViewer
                value={selectedArtifact.markdown}
                language="markdown"
                height="100%"
              />
            )}
          </div>
          {canShowDiff ? (
            <div className="flex shrink-0 justify-end border-t border-[var(--aria-line)] px-4 py-3">
              <button
                type="button"
                onClick={() => setShowVersionDiff((value) => !value)}
                className="inline-flex h-8 items-center rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]"
              >
                {showVersionDiff ? "隐藏对比" : "与最新版本对比"}
              </button>
            </div>
          ) : null}
        </div>
      ) : null}

      {issuePreviewOpen && entity.kind === "issue" && entity.description ? (
        <div className="fixed inset-4 z-[70] flex min-h-0 flex-col rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel)] shadow-2xl">
          <div className="flex shrink-0 items-center justify-between gap-3 border-b border-[var(--aria-line)] px-4 py-3">
            <div className="min-w-0">
              <div className="flex items-center gap-2 text-sm font-semibold text-[var(--aria-ink)]">
                <FileText className="h-4 w-4 text-[var(--aria-primary)]" />
                Issue 描述预览
              </div>
              <p className="mt-1 truncate font-mono text-xs text-[var(--aria-ink-muted)]">
                {entity.id}
              </p>
            </div>
            <button
              type="button"
              aria-label="关闭 Markdown 内容预览"
              onClick={() => setIssuePreviewOpen(false)}
              className="inline-flex h-8 w-8 shrink-0 items-center justify-center rounded-md border border-[var(--aria-line)] text-[var(--aria-ink-muted)] hover:border-[var(--aria-primary)] hover:text-[var(--aria-primary)]"
            >
              <X className="h-4 w-4" />
            </button>
          </div>
          <div className="min-h-0 flex-1 p-4">
            <MonacoViewer
              value={entity.description}
              language="markdown"
              height="100%"
            />
          </div>
        </div>
      ) : null}

      <footer className="space-y-2 border-t border-[var(--aria-line)] px-4 py-3">
        <button
          data-testid="drawer-open-workspace"
          type="button"
          onClick={onOpenWorkspace}
          className="inline-flex h-9 w-full items-center justify-center gap-2 rounded-md bg-[var(--aria-ink)] px-3 text-sm font-semibold text-white hover:opacity-90"
        >
          <ExternalLink className="h-4 w-4" />
          打开 Workspace
        </button>
        {nextActionLabel && onGenerateNext ? (
          <button
            data-testid="drawer-generate-next"
            type="button"
            onClick={onGenerateNext}
            className="inline-flex h-9 w-full items-center justify-center gap-2 rounded-md border border-[var(--aria-primary)] bg-white px-3 text-sm font-semibold text-[var(--aria-primary)] hover:bg-[var(--aria-panel-muted)]"
          >
            <GitBranch className="h-4 w-4" />
            {nextActionLabel}
          </button>
        ) : null}
        {entity.kind === "work_item" && onOpenCodingWorkspace ? (
          <button
            data-testid="drawer-open-coding-workspace"
            type="button"
            onClick={onOpenCodingWorkspace}
            className="inline-flex h-9 w-full items-center justify-center gap-2 rounded-md border border-[var(--aria-primary)] bg-white px-3 text-sm font-semibold text-[var(--aria-primary)] hover:bg-[var(--aria-panel-muted)]"
          >
            <Code className="h-4 w-4" />
            {codingActionLabel}
          </button>
        ) : null}
      </footer>
    </aside>
  );
}

function IssueDetail({
  entity,
  onOpenPreview,
}: {
  entity: DrawerEntity;
  onOpenPreview: () => void;
}) {
  const artifacts = entity.artifacts ?? [];
  return (
    <>
      {entity.description ? (
        <section className="border-b border-[var(--aria-line)] px-4 py-3">
          <div className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3">
            <div className="flex min-w-0 items-center justify-between gap-3">
              <div className="min-w-0">
                <div className="flex items-center gap-2 text-sm font-semibold text-[var(--aria-ink)]">
                  <FileText className="h-4 w-4 text-[var(--aria-primary)]" />
                  Issue 描述
                </div>
                <p className="mt-1 text-xs leading-5 text-[var(--aria-ink-muted)]">
                  默认隐藏长内容，按需打开 Markdown 大预览。
                </p>
              </div>
              <button
                type="button"
                onClick={onOpenPreview}
                className="inline-flex h-8 shrink-0 items-center rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]"
              >
                查看 Markdown 内容
              </button>
            </div>
          </div>
        </section>
      ) : null}
      {artifacts.length > 0 ? (
        <section className="border-b border-[var(--aria-line)] px-4 py-3">
          <h3 className="mb-2 text-sm font-semibold text-[var(--aria-ink)]">关联产物</h3>
          <div className="space-y-2">
            {artifacts.map((artifact) => (
              <div
                key={artifact.artifact_ref}
                className="rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-2 py-2 text-xs"
              >
                <div className="flex items-center justify-between gap-2">
                  <span className="font-semibold text-[var(--aria-ink)]">
                    {artifact.artifact_kind}
                  </span>
                  <span className="text-[var(--aria-ink-muted)]">{artifact.stage}</span>
                </div>
                <div className="mt-1 text-[var(--aria-ink-muted)]">{artifact.summary}</div>
              </div>
            ))}
          </div>
        </section>
      ) : null}
      {entity.phase || entity.createdAt ? (
        <section className="px-4 py-3">
          <h3 className="mb-2 text-sm font-semibold text-[var(--aria-ink)]">元信息</h3>
          <div className="space-y-1 text-xs text-[var(--aria-ink-muted)]">
            {entity.phase ? <div>阶段: {entity.phase}</div> : null}
            {entity.createdAt ? <div>创建时间: {entity.createdAt.slice(0, 10)}</div> : null}
          </div>
        </section>
      ) : null}
    </>
  );
}

function iconForKind(kind: DrawerEntityKind) {
  if (kind === "issue") return ListChecks;
  if (kind === "story_spec") return ScrollText;
  if (kind === "design_spec") return Layers3;
  return GitBranch;
}

function providerLabel(provider: string) {
  if (provider === "claude_code") return "Claude Code";
  if (provider === "codex") return "Codex";
  if (provider === "fake") return "Fake";
  return provider;
}
