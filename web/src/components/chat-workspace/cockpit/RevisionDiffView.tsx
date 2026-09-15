import { useEffect, useMemo, useRef, useState } from "react";
import type { ArtifactVersionSummary, ReviewVerdictType } from "../../../api/types";
import {
  REVISION_DIFF_MAX_LINES,
  computeRevisionDiff,
  toUnifiedHunks,
} from "../../../state/revision-diff";

export interface RevisionDiffViewProps {
  sessionId: string | null;
  versions: readonly ArtifactVersionSummary[];
  contentCache: Readonly<Record<number, string>>; // 版本号 -> markdown（含零个条目）
  loadVersionMarkdown: ((version: number) => Promise<string>) | null;
  onCacheVersionMarkdown: ((version: number, markdown: string) => void) | null;
}

// 「为什么改」：与 ReviewVerdictEntry 的 verdictLabel 同一套中文文案。
const REVIEW_VERDICT_LABELS: Readonly<Record<ReviewVerdictType, string>> = {
  pass: "通过",
  revise: "建议返修",
  needs_human: "需要人工确认",
};

export function RevisionDiffView({
  sessionId,
  versions,
  contentCache,
  loadVersionMarkdown,
  onCacheVersionMarkdown,
}: RevisionDiffViewProps) {
  const [baseVersion, setBaseVersion] = useState<number | null>(null);
  const [targetVersion, setTargetVersion] = useState<number | null>(null);
  const [localCache, setLocalCache] = useState<Record<number, string>>({});
  const [loadError, setLoadError] = useState<string | null>(null);

  const inFlightVersionsRef = useRef(new Set<number>());
  const loadedVersionsRef = useRef(new Set<number>());

  const sortedVersions = useMemo(
    () => [...versions].sort((left, right) => right.version - left.version),
    [versions],
  );
  // 默认取最近两轮（目标=最新、基准=次新）；两侧 select 互斥排除对方当前值
  // （baseOptions / targetOptions），版本号唯一 ⇒ 基准与目标结构性不可能相同。
  const effectiveTarget = targetVersion ?? sortedVersions[0]?.version ?? null;
  const effectiveBase = baseVersion ?? sortedVersions[1]?.version ?? null;

  const markdownFor = (versionNo: number): string | undefined =>
    contentCache[versionNo] ?? localCache[versionNo];

  const missingKey = [effectiveBase, effectiveTarget]
    .filter(
      (versionNo): versionNo is number =>
        versionNo !== null && markdownFor(versionNo) === undefined,
    )
    .join(",");

  useEffect(() => {
    setLoadError(null);
    if (sortedVersions.length < 2 || !loadVersionMarkdown || !missingKey) return;
    for (const versionNo of missingKey.split(",").map(Number)) {
      if (
        inFlightVersionsRef.current.has(versionNo) ||
        loadedVersionsRef.current.has(versionNo)
      ) continue;
      inFlightVersionsRef.current.add(versionNo);
      void loadVersionMarkdown(versionNo)
        .then((markdown) => {
          loadedVersionsRef.current.add(versionNo);
          setLocalCache((previous) => ({ ...previous, [versionNo]: markdown }));
          onCacheVersionMarkdown?.(versionNo, markdown);
        })
        .catch(() => setLoadError(`轮次 v${versionNo} 加载失败`))
        .finally(() => inFlightVersionsRef.current.delete(versionNo));
    }
  }, [missingKey, loadVersionMarkdown, onCacheVersionMarkdown, sortedVersions.length]);

  const diff = useMemo(() => {
    if (effectiveBase === null || effectiveTarget === null) {
      return null;
    }
    const oldMarkdown = markdownFor(effectiveBase);
    const newMarkdown = markdownFor(effectiveTarget);
    if (oldMarkdown === undefined || newMarkdown === undefined) {
      return null;
    }
    return computeRevisionDiff(oldMarkdown, newMarkdown);
  }, [contentCache, localCache, effectiveBase, effectiveTarget]);

  // 摘要直接消费 T1 导出的 summary / truncated，不从 lines 重算。
  const hunks = useMemo(
    () => (diff ? toUnifiedHunks(diff.lines) : []),
    [diff],
  );

  const baseOptions = sortedVersions.filter(
    (item) => item.version !== effectiveTarget,
  );
  const targetOptions = sortedVersions.filter(
    (item) => item.version !== effectiveBase,
  );

  const verdictChips = [effectiveBase, effectiveTarget]
    .filter(
      (versionNo): versionNo is number =>
        versionNo !== null &&
        versions.some(
          (item) => item.version === versionNo && item.review_verdict != null,
        ),
    )
    .map((versionNo) => ({
      version: versionNo,
      verdict: versions.find((item) => item.version === versionNo)!
        .review_verdict as ReviewVerdictType,
    }));

  return (
    <div data-testid="revision-diff-view" className="flex min-h-0 flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2">
        <label className="flex items-center gap-1 text-xs text-[var(--aria-ink-muted)]">
          基准轮次
          <select
            data-testid="revision-diff-base-select"
            value={effectiveBase ?? ""}
            onChange={(event) =>
              setBaseVersion(event.target.value === "" ? null : Number(event.target.value))
            }
            disabled={baseOptions.length === 0}
            className="aria-mono aria-num min-h-11 rounded-md border border-[var(--aria-line-strong)] bg-white px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            {baseOptions.map((item) => (
              <option key={item.version} value={item.version}>
                v{item.version}
              </option>
            ))}
          </select>
        </label>
        <span className="text-xs text-[var(--aria-ink-muted)]">对比</span>
        <label className="flex items-center gap-1 text-xs text-[var(--aria-ink-muted)]">
          目标轮次
          <select
            data-testid="revision-diff-target-select"
            value={effectiveTarget ?? ""}
            onChange={(event) =>
              setTargetVersion(event.target.value === "" ? null : Number(event.target.value))
            }
            disabled={targetOptions.length === 0}
            className="aria-mono aria-num min-h-11 rounded-md border border-[var(--aria-line-strong)] bg-white px-2 text-sm focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[var(--aria-primary)]"
          >
            {targetOptions.map((item) => (
              <option key={item.version} value={item.version}>
                v{item.version}
                {item.is_current ? "（当前）" : ""}
              </option>
            ))}
          </select>
        </label>
        {verdictChips.map((chip) => (
          <span
            key={chip.version}
            data-testid="revision-diff-verdict"
            className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]"
          >
            v{chip.version} 审批：{REVIEW_VERDICT_LABELS[chip.verdict]}
          </span>
        ))}
        {sessionId ? (
          <span className="aria-mono text-[11px] text-[var(--aria-ink-muted)]">
            {sessionId}
          </span>
        ) : null}
      </div>

      {sortedVersions.length < 2 ? (
        <p className="text-sm text-[var(--aria-ink-muted)]">
          当前会话只有一个 artifact 轮次，暂无对比对象。
        </p>
      ) : loadError ? (
        <p role="alert" className="text-sm text-[var(--aria-danger)]">
          {loadError}
        </p>
      ) : !diff ? (
        <p className="text-sm text-[var(--aria-ink-muted)]">正在加载轮次内容…</p>
      ) : (
        <>
          <div
            data-testid="revision-diff-summary"
            className="flex flex-wrap items-center gap-2 text-xs"
          >
            <span className="aria-chip aria-mono aria-num border-[var(--aria-line-strong)] text-[var(--aria-ink-muted)]">
              基准 v{effectiveBase} → 目标 v{effectiveTarget}
            </span>
            <span className="aria-chip aria-mono aria-num border-[var(--aria-topo-node-done-border)] bg-[var(--aria-topo-node-done-bg)] text-[var(--aria-topo-node-done-fg)]">
              +{diff.summary.added} 新增
            </span>
            <span className="aria-chip aria-mono aria-num border-[var(--aria-topo-node-failed-border)] bg-[var(--aria-topo-node-failed-bg)] text-[var(--aria-topo-node-failed-fg)]">
              -{diff.summary.removed} 删除
            </span>
            <span className="aria-chip aria-mono aria-num border-[var(--aria-gate-open-border)] bg-[var(--aria-gate-open-bg)] text-[var(--aria-gate-open-fg)]">
              ~{diff.summary.changed} 修改
            </span>
            {diff.truncated ? (
              <span className="aria-chip border-[var(--aria-topo-node-failed-border)] bg-[var(--aria-topo-node-failed-bg)] text-[var(--aria-topo-node-failed-fg)]">
                超过 {REVISION_DIFF_MAX_LINES} 行，按整档替换呈现
              </span>
            ) : null}
          </div>
          {hunks.length === 0 ? (
            <p className="text-sm text-[var(--aria-ink-muted)]">两轮内容一致，无差异。</p>
          ) : (
            <div className="min-h-0 space-y-3 overflow-auto">
              {hunks.map((hunk, index) => (
                <section
                  key={`${hunk.header}-${index}`}
                  data-testid="revision-diff-hunk"
                  className="overflow-hidden rounded-md border border-[var(--aria-line)]"
                >
                  <div className="aria-mono aria-num border-b border-[var(--aria-line)] bg-[var(--aria-panel-muted)] px-3 py-1 text-xs text-[var(--aria-ink-muted)]">
                    {hunk.header}
                  </div>
                  <pre className="aria-mono overflow-x-auto text-xs leading-5">
                    {hunk.lines.map((line) => (
                      <div
                        key={`${line.kind}-${line.oldLineNo}-${line.newLineNo}-${line.text}`}
                        data-testid={
                          line.kind === "added"
                            ? "revision-diff-line-added"
                            : line.kind === "removed"
                              ? "revision-diff-line-removed"
                              : "revision-diff-line-same"
                        }
                        className={
                          line.kind === "added"
                            ? "bg-[var(--aria-topo-node-done-bg)] px-3"
                            : line.kind === "removed"
                              ? "bg-[var(--aria-topo-node-failed-bg)] px-3"
                              : "px-3"
                        }
                      >
                        <span className="aria-num mr-2 inline-block w-8 text-right text-[var(--aria-ink-muted)]">
                          {line.oldLineNo ?? ""}
                        </span>
                        <span className="aria-num mr-2 inline-block w-8 text-right text-[var(--aria-ink-muted)]">
                          {line.newLineNo ?? ""}
                        </span>
                        <span className="mr-2">
                          {line.kind === "added" ? "+" : line.kind === "removed" ? "-" : " "}
                        </span>
                        {line.text}
                      </div>
                    ))}
                  </pre>
                </section>
              ))}
            </div>
          )}
        </>
      )}
    </div>
  );
}
