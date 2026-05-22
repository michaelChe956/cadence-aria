import { ChevronDown, ChevronRight, FileText, GitCompare } from "lucide-react";
import { useMemo, useState } from "react";
import type { ArtifactVersion } from "../../state/workspace-ws-store";
import { MonacoDiffViewer } from "../shared/MonacoDiffViewer";
import { MonacoViewer } from "../shared/MonacoViewer";

interface ArtifactPaneProps {
  artifactVersions: ArtifactVersion[];
  artifact: string | null;
  className?: string;
}

export function ArtifactPane({ artifactVersions, artifact, className = "" }: ArtifactPaneProps) {
  const sortedVersions = useMemo(
    () => [...artifactVersions].sort((left, right) => left.version - right.version),
    [artifactVersions],
  );
  const latestVersion = sortedVersions.at(-1) ?? null;
  const [selectedVersion, setSelectedVersion] = useState<number | null>(
    latestVersion?.version ?? null,
  );
  const [collapsed, setCollapsed] = useState(false);
  const [showDiff, setShowDiff] = useState(false);
  const selected =
    sortedVersions.find((version) => version.version === selectedVersion) ?? latestVersion;
  const previous = previousVersion(sortedVersions, selected?.version ?? null);
  const markdown = selected?.markdown ?? artifact ?? "等待 Artifact";

  if (collapsed) {
    return (
      <aside
        data-testid="artifact-pane"
        className={`border-l border-[var(--aria-line)] bg-[var(--aria-panel)] p-3 ${className}`}
      >
        <button
          type="button"
          onClick={() => setCollapsed(false)}
          className="inline-flex h-8 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]"
        >
          <ChevronRight className="h-4 w-4" />
          展开 Artifact
        </button>
      </aside>
    );
  }

  return (
    <aside
      data-testid="artifact-pane"
      className={`flex min-h-0 flex-col border-l border-[var(--aria-line)] bg-[var(--aria-panel)] ${className}`}
    >
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-2 border-b border-[var(--aria-line)] px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <FileText className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
          <h2 className="truncate text-sm font-semibold text-[var(--aria-ink)]">Artifact</h2>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <label className="inline-flex items-center gap-2 text-xs text-[var(--aria-ink-muted)]">
            版本
            <select
              aria-label="Artifact 版本"
              value={selected?.version ?? 0}
              onChange={(event) => setSelectedVersion(Number(event.target.value))}
              className="h-8 rounded-md border border-[var(--aria-line)] bg-white px-2 text-xs text-[var(--aria-ink)]"
            >
              {sortedVersions.length > 0 ? (
                sortedVersions.map((version) => (
                  <option key={version.version} value={version.version}>
                    v{version.version}
                  </option>
                ))
              ) : (
                <option value={0}>v0</option>
              )}
            </select>
          </label>
          <button
            type="button"
            onClick={() => setShowDiff((value) => !value)}
            disabled={!previous || !selected}
            className="inline-flex h-8 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)] disabled:opacity-50"
          >
            <GitCompare className="h-3.5 w-3.5" />
            {showDiff ? "隐藏 Diff" : "显示 Diff"}
          </button>
          <button
            type="button"
            onClick={() => setCollapsed(true)}
            className="inline-flex h-8 items-center gap-2 rounded-md border border-[var(--aria-line)] bg-white px-3 text-xs font-semibold text-[var(--aria-ink)] hover:bg-[var(--aria-panel-muted)]"
          >
            <ChevronDown className="h-3.5 w-3.5" />
            折叠 Artifact
          </button>
        </div>
      </div>

      <div className="min-h-0 flex-1 p-3">
        {showDiff && selected && previous ? (
          <MonacoDiffViewer
            original={previous.markdown}
            modified={selected.markdown}
            language="markdown"
            height="100%"
          />
        ) : (
          <MonacoViewer value={markdown} language="markdown" height="100%" />
        )}
      </div>
    </aside>
  );
}

function previousVersion(versions: ArtifactVersion[], selectedVersion: number | null) {
  if (selectedVersion === null) {
    return null;
  }
  const index = versions.findIndex((version) => version.version === selectedVersion);
  return index > 0 ? versions[index - 1] : null;
}
