import { ChevronDown, ChevronRight, FileText, GitCompare } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type { ArtifactVersionSummary } from "../../state/workspace-ws-store";
import { MonacoDiffViewer } from "../shared/MonacoDiffViewer";
import { MonacoViewer } from "../shared/MonacoViewer";

interface ArtifactPaneProps {
  artifactVersions: ArtifactVersionSummary[];
  artifact: string | null;
  sessionId?: string | null;
  artifactContentCache?: Record<number, string>;
  loadArtifactVersion?: (version: number) => Promise<string>;
  onCacheArtifactContent?: (version: number, value: string) => void;
  className?: string;
}

export function ArtifactPane({
  artifactVersions,
  artifact,
  sessionId = null,
  artifactContentCache = {},
  loadArtifactVersion,
  onCacheArtifactContent,
  className = "",
}: ArtifactPaneProps) {
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
  const [loadedContent, setLoadedContent] = useState<Record<string, string>>({});
  const [loadingVersions, setLoadingVersions] = useState<Record<string, boolean>>({});
  const [loadErrors, setLoadErrors] = useState<Record<string, string>>({});
  const [retryNonce, setRetryNonce] = useState<Record<string, number>>({});
  const inFlightRef = useRef<Record<string, Promise<string>>>({});
  const mountedRef = useRef(false);
  const sessionRef = useRef<string | null>(sessionId);
  const ignoredCacheRef = useRef<Record<number, string> | null>(null);
  const currentSessionId = sessionId ?? null;
  if (sessionRef.current !== currentSessionId) {
    sessionRef.current = currentSessionId;
    ignoredCacheRef.current = artifactContentCache;
    inFlightRef.current = {};
  }
  const selected =
    sortedVersions.find((version) => version.version === selectedVersion) ?? latestVersion;
  const previous = previousVersion(sortedVersions, selected?.version ?? null);
  const selectedKey = selected ? versionContentKey(currentSessionId, selected.version) : null;
  const previousKey = previous ? versionContentKey(currentSessionId, previous.version) : null;
  const selectedMarkdown = selected
    ? contentForVersion(
        selected,
        currentSessionId,
        artifactContentCache,
        loadedContent,
        sessionRef.current,
        ignoredCacheRef.current,
      )
    : null;
  const previousMarkdown = previous
    ? contentForVersion(
        previous,
        currentSessionId,
        artifactContentCache,
        loadedContent,
        sessionRef.current,
        ignoredCacheRef.current,
      )
    : null;
  const markdown = selectedMarkdown ?? artifact ?? "等待 Artifact";
  const selectedError = selectedKey ? loadErrors[selectedKey] : undefined;
  const previousError = previousKey ? loadErrors[previousKey] : undefined;
  const selectedLoading = selectedKey ? loadingVersions[selectedKey] : false;
  const previousLoading = previousKey ? loadingVersions[previousKey] : false;
  const canLoadArtifact = Boolean(currentSessionId && loadArtifactVersion);

  useEffect(() => {
    mountedRef.current = true;
    return () => {
      mountedRef.current = false;
      inFlightRef.current = {};
    };
  }, []);

  useEffect(() => {
    inFlightRef.current = {};
    setLoadedContent({});
    setLoadingVersions({});
    setLoadErrors({});
    setRetryNonce({});
  }, [sessionId]);

  const ensureVersionLoaded = useCallback(
    (version: ArtifactVersionSummary | null) => {
      if (!version) {
        return;
      }
      const key = versionContentKey(currentSessionId, version.version);
      if (
        contentForVersion(
          version,
          currentSessionId,
          artifactContentCache,
          loadedContent,
          sessionRef.current,
          ignoredCacheRef.current,
        ) !== null
      ) {
        return;
      }
      if (!currentSessionId || !loadArtifactVersion || key in inFlightRef.current) {
        return;
      }
      const requestSessionId = currentSessionId;
      setLoadingVersions((current) => ({ ...current, [key]: true }));
      setLoadErrors((current) => {
        const { [key]: _removed, ...rest } = current;
        return rest;
      });
      const request = loadArtifactVersion(version.version)
        .then((value) => {
          if (
            !isCurrentRequest(requestSessionId, key, sessionRef.current, version.version) ||
            !mountedRef.current
          ) {
            return value;
          }
          setLoadedContent((current) => ({ ...current, [key]: value }));
          onCacheArtifactContent?.(version.version, value);
          return value;
        })
        .catch((error: unknown) => {
          if (
            !isCurrentRequest(requestSessionId, key, sessionRef.current, version.version) ||
            !mountedRef.current
          ) {
            throw error;
          }
          const message = error instanceof Error ? error.message : String(error);
          setLoadErrors((current) => ({ ...current, [key]: message }));
          throw error;
        })
        .finally(() => {
          if (inFlightRef.current[key] === request) {
            delete inFlightRef.current[key];
          }
          if (
            !isCurrentRequest(requestSessionId, key, sessionRef.current, version.version) ||
            !mountedRef.current
          ) {
            return;
          }
          setLoadingVersions((current) => ({ ...current, [key]: false }));
        });
      inFlightRef.current[key] = request;
      request.catch(() => undefined);
    },
    [artifactContentCache, currentSessionId, loadArtifactVersion, loadedContent, onCacheArtifactContent],
  );

  const retryVersion = useCallback((version: number) => {
    const key = versionContentKey(sessionRef.current, version);
    setLoadErrors((current) => {
      const { [key]: _removed, ...rest } = current;
      return rest;
    });
    setRetryNonce((current) => ({ ...current, [key]: (current[key] ?? 0) + 1 }));
  }, []);

  useEffect(() => {
    ensureVersionLoaded(selected);
    if (showDiff) {
      ensureVersionLoaded(previous);
    }
  }, [ensureVersionLoaded, previous, retryNonce, selected, showDiff]);

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
          <div data-testid="artifact-diff" className="h-full min-h-0">
            {selectedError || previousError ? (
              <ArtifactLoadError
                version={selectedError ? selected.version : previous.version}
                message={selectedError ?? previousError ?? "加载失败"}
                onRetry={retryVersion}
              />
            ) : !canLoadArtifact && (selectedMarkdown === null || previousMarkdown === null) ? (
              <ArtifactUnavailable />
            ) : selectedLoading || previousLoading || selectedMarkdown === null || previousMarkdown === null ? (
              <ArtifactLoading version={selectedLoading || selectedMarkdown === null ? selected.version : previous.version} />
            ) : (
              <MonacoDiffViewer
                original={previousMarkdown}
                modified={selectedMarkdown}
                language="markdown"
                height="100%"
              />
            )}
          </div>
        ) : selectedError ? (
          <ArtifactLoadError version={selected?.version ?? 0} message={selectedError} onRetry={retryVersion} />
        ) : selected && selectedMarkdown === null && !canLoadArtifact ? (
          <ArtifactUnavailable />
        ) : selected && (selectedLoading || selectedMarkdown === null) ? (
          <ArtifactLoading version={selected.version} />
        ) : (
          <MonacoViewer value={markdown} language="markdown" height="100%" />
        )}
      </div>
    </aside>
  );
}

function ArtifactLoading({ version }: { version: number }) {
  return (
    <div data-testid="artifact-loading" className="text-sm text-[var(--aria-ink-muted)]">
      正在加载 v{version}
    </div>
  );
}

function ArtifactLoadError({
  version,
  message,
  onRetry,
}: {
  version: number;
  message: string;
  onRetry: (version: number) => void;
}) {
  return (
    <div role="alert" className="flex items-center gap-2 text-sm text-red-700">
      <span>加载 v{version} 失败：{message}</span>
      <button
        type="button"
        onClick={() => onRetry(version)}
        className="rounded border border-red-200 bg-white px-2 py-1 text-xs font-semibold text-red-700 hover:bg-red-50"
      >
        重试
      </button>
    </div>
  );
}

function ArtifactUnavailable() {
  return (
    <div data-testid="artifact-unavailable" className="text-sm text-[var(--aria-ink-muted)]">
      Artifact 内容未加载
    </div>
  );
}

function contentForVersion(
  version: ArtifactVersionSummary,
  sessionId: string | null,
  cache: Record<number, string>,
  loadedContent: Record<string, string>,
  committedSessionId: string | null,
  ignoredCache: Record<number, string> | null,
) {
  if ("markdown" in version && typeof version.markdown === "string") {
    return version.markdown;
  }
  const key = versionContentKey(sessionId, version.version);
  const localContent = loadedContent[key];
  if (localContent !== undefined) {
    return localContent;
  }
  if (sessionId && sessionId !== committedSessionId) {
    return null;
  }
  if (sessionId && cache === ignoredCache) {
    return null;
  }
  return cache[version.version] ?? null;
}

function versionContentKey(sessionId: string | null | undefined, version: number) {
  return `${sessionId ?? ""}:${version}`;
}

function isCurrentRequest(
  requestSessionId: string,
  requestKey: string,
  currentSessionId: string | null,
  version: number,
) {
  return currentSessionId === requestSessionId && requestKey === versionContentKey(currentSessionId, version);
}

function previousVersion(versions: ArtifactVersionSummary[], selectedVersion: number | null) {
  if (selectedVersion === null) {
    return null;
  }
  const index = versions.findIndex((version) => version.version === selectedVersion);
  return index > 0 ? versions[index - 1] : null;
}
