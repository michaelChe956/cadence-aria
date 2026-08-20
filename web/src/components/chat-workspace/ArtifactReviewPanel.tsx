import { PanelRightClose } from "lucide-react";
import type { ReactNode } from "react";
import type { ArtifactVersionSummary } from "../../state/workspace-ws-store";
import { ArtifactPane } from "./ArtifactPane";

interface ArtifactReviewPanelProps {
  artifactVersions: ArtifactVersionSummary[];
  artifact: string | null;
  sessionId?: string | null;
  artifactContentCache?: Record<number, string>;
  loadArtifactVersion?: (version: number) => Promise<string>;
  onCacheArtifactContent?: (version: number, value: string) => void;
  /** 最近一次 completed revision 节点的 summary；为空时改动摘要条整条不渲染。 */
  changelogSummary?: string | null;
  onClose?: () => void;
  /** 审核操作插槽（Task 4 迁移三动作）。 */
  actions?: ReactNode;
  className?: string;
}

/**
 * Canvas 产物审核面板（spec-workbench-canvas-experience T3）。
 * 结构：工具条（标题 + 收起钮）→ 改动摘要折叠条（默认展开）→
 * ArtifactPane 渲染区（flex-1 min-h-0）→ 吸顶/吸底操作条（actions 插槽）。
 */
export function ArtifactReviewPanel({
  artifactVersions,
  artifact,
  sessionId = null,
  artifactContentCache = {},
  loadArtifactVersion,
  onCacheArtifactContent,
  changelogSummary,
  onClose,
  actions = null,
  className = "",
}: ArtifactReviewPanelProps) {
  const trimmedChangelog =
    typeof changelogSummary === "string" && changelogSummary.trim().length > 0
      ? changelogSummary.trim()
      : null;

  return (
    <section
      data-testid="artifact-review-panel"
      className={`aria-card-clay flex min-h-0 flex-col overflow-hidden transition-transform duration-300 ${className}`}
    >
      <div className="flex min-w-0 shrink-0 items-center justify-between gap-2 border-b border-[var(--aria-line)] px-3 py-2">
        <h2 className="truncate text-sm font-semibold text-[var(--aria-ink)]">
          Artifact 审核
        </h2>
        {onClose ? (
          <button
            type="button"
            onClick={onClose}
            className="inline-flex h-8 shrink-0 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]"
          >
            <PanelRightClose className="h-4 w-4" />
            收起面板
          </button>
        ) : null}
      </div>

      {trimmedChangelog ? (
        <details
          open
          data-testid="artifact-review-changelog"
          className="shrink-0 border-b border-[var(--aria-line)] bg-[var(--aria-cta-soft)] px-3 py-2"
        >
          <summary className="cursor-pointer text-xs font-semibold text-[var(--aria-ink)]">
            本轮改动
          </summary>
          <p className="mt-1 whitespace-pre-wrap break-words text-xs leading-5 text-[var(--aria-ink)]">
            {trimmedChangelog}
          </p>
        </details>
      ) : null}

      <div className="min-h-0 flex-1">
        <ArtifactPane
          artifactVersions={artifactVersions}
          artifact={artifact}
          sessionId={sessionId}
          artifactContentCache={artifactContentCache}
          loadArtifactVersion={loadArtifactVersion}
          onCacheArtifactContent={onCacheArtifactContent}
          className="h-full min-h-0 border-0"
        />
      </div>

      {actions ? (
        <div
          data-testid="artifact-review-actions"
          className="sticky bottom-0 z-10 flex shrink-0 flex-wrap items-center justify-end gap-2 border-t border-[var(--aria-line)] bg-[var(--aria-panel)] px-3 py-2"
        >
          {actions}
        </div>
      ) : null}
    </section>
  );
}
