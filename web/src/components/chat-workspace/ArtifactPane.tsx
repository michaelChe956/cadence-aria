import { ChevronDown, ChevronRight, FileText, GitCompare } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import type {
  ScEvidenceRef,
  ScFailureRepairPackage,
  WorkItemPlanArtifactVersion,
} from "../../api/types";
import type { ArtifactVersionSummary } from "../../state/workspace-ws-store";
import { MonacoDiffViewer } from "../shared/MonacoDiffViewer";
import { MonacoViewer } from "../shared/MonacoViewer";
import { workItemPlanArtifactLabel } from "../workspace/work-item-plan-projection-artifacts";
import { REVIEW_VERDICT_LABELS } from "./review-verdict-labels";

interface ArtifactPaneProps {
  artifactVersions: ArtifactVersionSummary[];
  /**
   * 已发布版本的结构化投影（driver/observer 同源）。只用于把 markdown rail 里
   * 缺失的 SC 失败修复包版本补齐——包恒 is_current=false，绝不替换普通产物。
   */
  structuredVersions?: WorkItemPlanArtifactVersion[];
  artifact: string | null;
  sessionId?: string | null;
  artifactContentCache?: Record<number, string>;
  loadArtifactVersion?: (version: number) => Promise<string>;
  onCacheArtifactContent?: (version: number, value: string) => void;
  className?: string;
}

export function ArtifactPane({
  artifactVersions,
  structuredVersions,
  artifact,
  sessionId = null,
  artifactContentCache = {},
  loadArtifactVersion,
  onCacheArtifactContent,
  className = "",
}: ArtifactPaneProps) {
  const sortedVersions = useMemo(
    () => mergeRailVersions(artifactVersions, structuredVersions),
    [artifactVersions, structuredVersions],
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
  const selectedPackage = scFailurePackageOf(selected);
  const previousPackage = scFailurePackageOf(previous);
  const diffAvailable = Boolean(
    selected && previous && !selectedPackage && !previousPackage,
  );
  const selectedKey = selected ? versionContentKey(currentSessionId, selected.version) : null;
  const previousKey = previous ? versionContentKey(currentSessionId, previous.version) : null;
  const selectedMarkdown =
    selected && !selectedPackage
      ? contentForVersion(
          selected,
          currentSessionId,
          artifactContentCache,
          loadedContent,
          sessionRef.current,
          ignoredCacheRef.current,
        )
      : null;
  const previousMarkdown =
    previous && !previousPackage
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
    (version: WorkItemPlanArtifactVersion | null) => {
      if (!version) {
        return;
      }
      // SC 失败修复包没有 markdown 正文：只读呈现包本身，绝不为它发全文请求。
      if (scFailurePackageOf(version)) {
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
      className={`aria-card-clay flex min-h-0 flex-col ${className}`}
    >
      <div className="flex min-w-0 flex-wrap items-center justify-between gap-2 border-b border-[var(--aria-line)] px-3 py-2">
        <div className="flex min-w-0 items-center gap-2">
          <FileText className="h-4 w-4 shrink-0 text-[var(--aria-primary)]" />
          <h2 className="truncate text-sm font-semibold text-[var(--aria-ink)]">Artifact</h2>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <label className="aria-chip text-[var(--aria-ink-muted)]">
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
                    {railVersionLabel(version)}
                  </option>
                ))
              ) : (
                <option value={0}>v0</option>
              )}
            </select>
          </label>
          {/* F-38 fix2（controller 二次实测反例，可达性）：评审结论徽章常显在头部——
              单版本会话「显示 Diff」disabled（无 previous）时 RevisionDiffView 不挂载，
              徽章若只在那里就不可达；数据源=所选版本的 review_verdict（多版本时跟随
              版本下拉），无结论不渲染。 */}
          {selected?.review_verdict != null ? (
            <span
              data-testid="artifact-version-verdict"
              className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
            >
              审批：{REVIEW_VERDICT_LABELS[selected.review_verdict]}
            </span>
          ) : null}
          <button
            type="button"
            onClick={() => setShowDiff((value) => !value)}
            disabled={!diffAvailable}
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
        {selectedPackage ? (
          <ScFailureRepairPackageView payload={selectedPackage} />
        ) : showDiff && selected && previous && !previousPackage ? (
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

/**
 * SC 失败修复包只读视图（A4 payload）：先给真实的完整/不完整判定与关键字段，
 * 再给包全文 JSON。没有任何动作位——不下载、不重试、不发请求。
 */
function ScFailureRepairPackageView({ payload }: { payload: ScFailureRepairPackage }) {
  const complete = payload.completeness === "complete";
  return (
    <div
      data-testid="sc-failure-repair-package-view"
      className="flex h-full min-h-0 flex-col gap-3 overflow-auto"
    >
      <div className="aria-card-clay flex flex-col gap-2 p-3">
        <div className="flex flex-wrap items-center gap-2">
          <span className="text-sm font-semibold text-[var(--aria-ink)]">
            {workItemPlanArtifactLabel({ type: "sc_failure_repair_package", payload })}
          </span>
          <span className="aria-chip border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]">
            {complete ? "证据完整" : "证据不完整"}
          </span>
        </div>
        <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1 text-xs text-[var(--aria-ink-muted)]">
          <dt>终止类别</dt>
          <dd className="text-[var(--aria-ink)]">{payload.terminal_class}</dd>
          <dt>终止原因</dt>
          <dd className="text-[var(--aria-ink)]">{payload.terminal_reason}</dd>
          <dt>失败链</dt>
          <dd className="aria-mono break-all">{payload.chain_id}</dd>
          <dt>轮次数</dt>
          <dd className="text-[var(--aria-ink)]">{payload.rounds.length}</dd>
          <dt>缺失证据</dt>
          <dd className="text-[var(--aria-ink)]">{payload.missing_evidence.length}</dd>
          <dt>末次提供者输出</dt>
          <dd className="aria-mono break-all">{evidenceRefText(payload.last_provider_output)}</dd>
          <dt>末次失败源</dt>
          <dd className="aria-mono break-all">
            {payload.last_failure_source.source_round_id} ·{" "}
            {evidenceRefText(payload.last_failure_source.source)}
          </dd>
          <dt>证据日志</dt>
          <dd className="aria-mono break-all">{payload.evidence_log_ref}</dd>
          <dt>证据摘要</dt>
          <dd className="aria-mono break-all">{payload.evidence_digest}</dd>
        </dl>
        {payload.missing_evidence.length > 0 ? (
          <ul className="flex flex-col gap-1 text-xs text-[var(--aria-ink-muted)]">
            {payload.missing_evidence.map((missing) => (
              <li key={`${missing.round_id}:${missing.kind}:${missing.code}`}>
                {missing.round_id} · {missing.kind} · {missing.code} · {missing.detail}
              </li>
            ))}
          </ul>
        ) : null}
        <ul className="flex flex-col gap-1 text-xs text-[var(--aria-ink-muted)]">
          {payload.rounds.map((round) => (
            <li key={round.identity.round_id}>
              {round.identity.round_id} · {round.failure_class ?? "未分类失败"} ·{" "}
              {round.blocking_reasons.length > 0
                ? round.blocking_reasons.join("；")
                : "无阻断原因"}
            </li>
          ))}
        </ul>
      </div>
      <pre
        data-testid="sc-failure-repair-package-json"
        className="aria-mono overflow-auto whitespace-pre-wrap break-words rounded-md border border-[var(--aria-line)] bg-[var(--aria-panel-muted)] p-3 text-xs text-[var(--aria-ink)]"
      >
        {JSON.stringify(payload, null, 2)}
      </pre>
    </div>
  );
}

function evidenceRefText(reference: ScEvidenceRef): string {
  return reference.status === "available"
    ? `${reference.status} · ${reference.locator} · ${reference.bytes}B`
    : `${reference.status} · ${reference.code} · ${reference.detail}`;
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

/**
 * 版本轨（rail）= 既有 markdown 版本摘要 ∪ 结构化投影里缺失的 SC 失败修复包版本。
 * 只补包版本：其它结构化变体仍由既有产物面板呈现，不挤进 markdown 轨。
 */
function mergeRailVersions(
  artifactVersions: ArtifactVersionSummary[],
  structuredVersions: WorkItemPlanArtifactVersion[] | undefined,
): WorkItemPlanArtifactVersion[] {
  const merged = new Map<number, WorkItemPlanArtifactVersion>();
  for (const version of artifactVersions) {
    merged.set(version.version, { ...version });
  }
  for (const version of structuredVersions ?? []) {
    if (!scFailurePackageOf(version)) {
      continue;
    }
    const existing = merged.get(version.version);
    merged.set(version.version, {
      ...version,
      ...existing,
      markdown: existing?.markdown ?? version.markdown,
      artifact: version.artifact,
    });
  }
  return [...merged.values()].sort((left, right) => left.version - right.version);
}

function scFailurePackageOf(
  version: WorkItemPlanArtifactVersion | null | undefined,
): ScFailureRepairPackage | null {
  const artifact = version?.artifact;
  if (!artifact || artifact.type !== "sc_failure_repair_package") {
    return null;
  }
  return artifact.payload;
}

function railVersionLabel(version: WorkItemPlanArtifactVersion): string {
  const artifact = version.artifact;
  if (artifact?.type === "sc_failure_repair_package") {
    return `v${version.version} · ${workItemPlanArtifactLabel(artifact)}`;
  }
  return `v${version.version}`;
}
